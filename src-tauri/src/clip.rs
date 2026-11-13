use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use serde_json::{json, Value};
use tauri::Manager;
use windows::core::HSTRING;
use windows::Foundation::TimeSpan;
use windows::Media::Editing::{MediaClip, MediaComposition, MediaTrimmingPreference};
use windows::Media::MediaProperties::MediaEncodingProfile;
use windows::Storage::Streams::{DataReader, IRandomAccessStream, InMemoryRandomAccessStream};
use windows::Storage::{CreationCollisionOption, StorageFile, StorageFolder};
use windows::Win32::Foundation::{HWND, POINT, RECT};
use windows::Win32::Graphics::Dwm::{DwmGetWindowAttribute, DWMWA_EXTENDED_FRAME_BOUNDS};
use windows::Win32::Graphics::Gdi::ClientToScreen;
use windows::Win32::System::Performance::{QueryPerformanceCounter, QueryPerformanceFrequency};
use windows::Win32::UI::WindowsAndMessaging::GetClientRect;
use windows_capture::capture::{CaptureControl, Context, GraphicsCaptureApiHandler};
use windows_capture::encoder::{
    AudioSettingsBuilder, ContainerSettingsBuilder, VideoEncoder, VideoSettingsBuilder, VideoSettingsSubType,
};
use windows_capture::frame::Frame;
use windows_capture::graphics_capture_api::{GraphicsCaptureApi, InternalCaptureControl};
use windows_capture::settings::{
    ColorFormat, CursorCaptureSettings, DirtyRegionSettings, DrawBorderSettings, MinimumUpdateIntervalSettings,
    SecondaryWindowSettings, Settings,
};
use windows_capture::window::Window;

use crate::screenshots;
use crate::state::Shared;
use crate::ui_windows::{self, Toast};

const SAMPLE_RATE: u32 = 48_000;
const CHANNELS: u32 = 2;
const BLOCK: usize = 4;
const TICKS: i64 = 10_000_000;
const SEGMENT: Duration = Duration::from_secs(10);

enum Output {
    File(PathBuf),
    Memory(InMemoryRandomAccessStream),
}

struct Active {
    encoder: VideoEncoder,
    output: Output,
    started: Instant,
    audio_sent: u64,
    last_tick: i64,
    width: u32,
    height: u32,
    flipped: Vec<u8>,
}

#[derive(Default)]
struct Picture {
    data: Vec<u8>,
    width: u32,
    height: u32,
}

struct Pipe {
    slot: Mutex<Option<Active>>,
    picture: Mutex<Picture>,
    target: Mutex<Option<Output>>,
    hwnd: isize,
    fps: u32,
    audio: bool,
    quality: f64,
    closed: AtomicBool,
}

fn now_ticks() -> i64 {
    let (mut count, mut freq) = (0i64, 0i64);
    unsafe {
        let _ = QueryPerformanceCounter(&mut count);
        let _ = QueryPerformanceFrequency(&mut freq);
    }
    (i128::from(count) * i128::from(TICKS) / i128::from(freq.max(1))) as i64
}

fn even(n: u32) -> u32 {
    (n & !1).max(2)
}

fn bitrate(width: u32, height: u32, fps: u32, quality: f64) -> u32 {
    let bits = f64::from(width) * f64::from(height) * f64::from(fps) * quality;
    bits.clamp(4_000_000.0, 60_000_000.0) as u32
}

fn quality_of(name: &str) -> f64 {
    match name {
        "normal" => 0.15,
        "best" => 0.5,
        _ => 0.3,
    }
}

fn client_area(hwnd: isize, frame_width: u32, frame_height: u32) -> Option<(u32, u32, u32, u32)> {
    let window = HWND(hwnd as *mut std::ffi::c_void);
    let mut bounds = RECT::default();
    let mut client = RECT::default();
    let mut origin = POINT::default();
    unsafe {
        DwmGetWindowAttribute(
            window,
            DWMWA_EXTENDED_FRAME_BOUNDS,
            &mut bounds as *mut RECT as *mut std::ffi::c_void,
            std::mem::size_of::<RECT>() as u32,
        )
        .ok()?;
        GetClientRect(window, &mut client).ok()?;
        if !ClientToScreen(window, &mut origin).as_bool() {
            return None;
        }
    }
    let left = (origin.x - bounds.left).clamp(0, frame_width as i32) as u32;
    let top = (origin.y - bounds.top).clamp(0, frame_height as i32) as u32;
    let right = (left + client.right.max(0) as u32).min(frame_width);
    let bottom = (top + client.bottom.max(0) as u32).min(frame_height);
    (right > left + 16 && bottom > top + 16).then_some((left, top, right, bottom))
}

fn flip_into(picture: &Picture, width: u32, height: u32, out: &mut Vec<u8>) {
    let row = width as usize * 4;
    out.clear();
    out.resize(row * height as usize, 0);
    let copy = row.min(picture.width as usize * 4);
    let rows = height.min(picture.height) as usize;
    let source_row = picture.width as usize * 4;
    for y in 0..rows {
        let from = &picture.data[y * source_row..y * source_row + copy];
        let to = (height as usize - 1 - y) * row;
        out[to..to + copy].copy_from_slice(from);
    }
}

impl Pipe {
    fn new_encoder(&self, width: u32, height: u32) -> Result<Active, String> {
        let (width, height) = (even(width), even(height));
        let video = VideoSettingsBuilder::new(width, height)
            .sub_type(VideoSettingsSubType::H264)
            .frame_rate(self.fps)
            .bitrate(bitrate(width, height, self.fps, self.quality));
        let audio = AudioSettingsBuilder::new()
            .sample_rate(SAMPLE_RATE)
            .channel_count(CHANNELS)
            .bit_per_sample(16)
            .disabled(!self.audio);
        let target = self.target.lock().unwrap().take();
        let (encoder, output) = match target {
            Some(Output::File(path)) => (
                VideoEncoder::new(video, audio, ContainerSettingsBuilder::new(), &path).map_err(|e| e.to_string())?,
                Output::File(path),
            ),
            _ => {
                let stream = InMemoryRandomAccessStream::new().map_err(|e| e.to_string())?;
                let raw: IRandomAccessStream = stream.clone().into();
                (
                    VideoEncoder::new_from_stream(video, audio, ContainerSettingsBuilder::new(), raw)
                        .map_err(|e| e.to_string())?,
                    Output::Memory(stream),
                )
            }
        };
        Ok(Active {
            encoder,
            output,
            started: Instant::now(),
            audio_sent: 0,
            last_tick: 0,
            width,
            height,
            flipped: Vec::new(),
        })
    }

    fn send(&self, active: &mut Active, tick: i64) -> Result<(), String> {
        let tick = tick.max(active.last_tick + 1);
        active.last_tick = tick;
        {
            let picture = self.picture.lock().unwrap();
            if picture.data.is_empty() {
                return Ok(());
            }
            flip_into(&picture, active.width, active.height, &mut active.flipped);
        }
        active.encoder.send_frame_buffer(&active.flipped, tick).map_err(|e| e.to_string())
    }

    fn push(&self, tick: i64) -> Result<(), String> {
        let mut slot = self.slot.lock().unwrap();
        if slot.is_none() {
            let (width, height) = {
                let picture = self.picture.lock().unwrap();
                (picture.width, picture.height)
            };
            *slot = Some(self.new_encoder(width, height)?);
        }
        let active = slot.as_mut().unwrap();
        self.send(active, tick)
    }

    fn rotate(&self, next: Option<Output>) -> Result<Option<Active>, String> {
        let mut slot = self.slot.lock().unwrap();
        let Some(mut old) = slot.take() else {
            return Ok(None);
        };
        let tick = now_ticks();
        let _ = self.send(&mut old, tick);
        if let Some(next) = next {
            *self.target.lock().unwrap() = Some(next);
            let mut fresh = self.new_encoder(old.width, old.height)?;
            self.send(&mut fresh, tick)?;
            *slot = Some(fresh);
        }
        Ok(Some(old))
    }
}

struct Recorder {
    pipe: Arc<Pipe>,
    gap: i64,
    last: Option<i64>,
    scratch: Vec<u8>,
}

impl GraphicsCaptureApiHandler for Recorder {
    type Flags = Arc<Pipe>;
    type Error = String;

    fn new(ctx: Context<Arc<Pipe>>) -> Result<Self, String> {
        let gap = TICKS / i64::from(ctx.flags.fps.max(1)) - 20_000;
        Ok(Self { pipe: ctx.flags, gap, last: None, scratch: Vec::new() })
    }

    fn on_frame_arrived(&mut self, frame: &mut Frame, _control: InternalCaptureControl) -> Result<(), String> {
        let tick = frame.timestamp().map_err(|e| e.to_string())?.Duration;
        if self.last.is_some_and(|last| tick - last < self.gap) {
            return Ok(());
        }
        self.last = Some(tick);

        let (width, height) = (frame.width(), frame.height());
        let (left, top, right, bottom) = client_area(self.pipe.hwnd, width, height).unwrap_or((0, 0, width, height));
        let buffer = frame.buffer_crop(left, top, right, bottom).map_err(|e| e.to_string())?;
        let pixels = buffer.as_nopadding_buffer(&mut self.scratch);
        {
            let mut picture = self.pipe.picture.lock().unwrap();
            picture.data.clear();
            picture.data.extend_from_slice(pixels);
            picture.width = right - left;
            picture.height = bottom - top;
        }
        self.pipe.push(tick)
    }

    fn on_closed(&mut self) -> Result<(), String> {
        self.pipe.closed.store(true, Ordering::SeqCst);
        Ok(())
    }
}

impl Pipe {
    fn hear(&self, fresh: &[u8]) -> Result<(), String> {
        let mut slot = self.slot.lock().unwrap();
        let Some(active) = slot.as_mut() else {
            return Ok(());
        };
        let expected = (active.started.elapsed().as_secs_f64() * f64::from(SAMPLE_RATE)) as u64;
        let have = fresh.len() / BLOCK;
        let behind = expected.saturating_sub(active.audio_sent + have as u64);
        let mut chunk = Vec::new();
        if behind > u64::from(SAMPLE_RATE) / 10 {
            chunk.resize(behind as usize * BLOCK, 0);
        }
        chunk.extend_from_slice(&fresh[..have * BLOCK]);
        if !chunk.is_empty() {
            active.encoder.send_audio_buffer(&chunk, 0).map_err(|e| e.to_string())?;
            active.audio_sent += (chunk.len() / BLOCK) as u64;
        }
        Ok(())
    }
}

struct Tap {
    pid: u32,
    stop: Arc<AtomicBool>,
    thread: JoinHandle<()>,
}

static LISTENERS: Mutex<Vec<Arc<Pipe>>> = Mutex::new(Vec::new());
static TAP: Mutex<Option<Tap>> = Mutex::new(None);

fn listen(pid: u32, pipe: &Arc<Pipe>) {
    LISTENERS.lock().unwrap().push(Arc::clone(pipe));
    let mut tap = TAP.lock().unwrap();
    if tap.as_ref().is_some_and(|t| t.pid == pid && !t.thread.is_finished()) {
        return;
    }
    if let Some(old) = tap.take() {
        old.stop.store(true, Ordering::SeqCst);
        let _ = old.thread.join();
    }
    let stop = Arc::new(AtomicBool::new(false));
    let flag = Arc::clone(&stop);
    let thread = std::thread::spawn(move || {
        if let Err(error) = record_audio(pid, &flag) {
            crate::crash::note(format!("clip audio: {error}"));
        }
    });
    *tap = Some(Tap { pid, stop, thread });
}

fn unlisten(pipe: &Arc<Pipe>) {
    let empty = {
        let mut listeners = LISTENERS.lock().unwrap();
        listeners.retain(|p| !Arc::ptr_eq(p, pipe));
        listeners.is_empty()
    };
    if empty {
        if let Some(old) = TAP.lock().unwrap().take() {
            old.stop.store(true, Ordering::SeqCst);
            let _ = old.thread.join();
        }
    }
}

fn record_audio(pid: u32, stop: &AtomicBool) -> Result<(), String> {
    let _ = wasapi::initialize_mta();
    let format = wasapi::WaveFormat::new(16, 16, wasapi::SampleType::Int, SAMPLE_RATE as usize, CHANNELS as usize, None);
    let mut client = wasapi::AudioClient::new_application_loopback_client(pid, true).map_err(|e| e.to_string())?;
    let mode = wasapi::StreamMode::EventsShared { autoconvert: true, buffer_duration_hns: 0 };
    client
        .initialize_client(&format, wasapi::Direction::Capture, mode, None)
        .map_err(|e| e.to_string())?;
    let event = client.set_get_eventhandle().map_err(|e| e.to_string())?;
    let capture = client.get_audiocaptureclient().map_err(|e| e.to_string())?;
    client.start_stream().map_err(|e| e.to_string())?;

    let mut queue: VecDeque<u8> = VecDeque::new();
    while !stop.load(Ordering::SeqCst) {
        let _ = event.wait_for_event(40);
        while let Ok(Some(frames)) = capture.get_next_packet_size() {
            if frames == 0 || capture.read_from_device_to_deque(&mut queue).is_err() {
                break;
            }
        }
        let whole = queue.len() / BLOCK * BLOCK;
        let fresh: Vec<u8> = queue.drain(..whole).collect();
        for pipe in LISTENERS.lock().unwrap().iter() {
            if let Err(error) = pipe.hear(&fresh) {
                crate::crash::note(format!("clip audio: {error}"));
            }
        }
    }
    let _ = client.stop_stream();
    Ok(())
}

struct Session {
    control: CaptureControl<Recorder, String>,
    pipe: Arc<Pipe>,
    started: Instant,
    key: String,
}

pub fn key_on(cfg: &crate::config::Config) -> bool {
    cfg.get("clip_enabled").and_then(Value::as_bool).unwrap_or(true)
        && cfg.get("clip_key_enabled").and_then(Value::as_bool).unwrap_or(true)
}

pub fn replay_length(cfg: &crate::config::Config) -> u64 {
    let seconds = cfg.get("clip_replay_seconds").and_then(Value::as_u64).unwrap_or(0).min(600);
    let on = cfg.get("clip_enabled").and_then(Value::as_bool).unwrap_or(true)
        && cfg.get("clip_replay_enabled").and_then(Value::as_bool).unwrap_or(seconds > 0);
    match (on, seconds) {
        (false, _) => 0,
        (true, 0) => 60,
        (true, seconds) => seconds,
    }
}

struct ClipSettings {
    enabled: bool,
    fps: u32,
    audio: bool,
    quality: f64,
    max: u64,
    replay: u64,
}

fn settings_of(shared: &Shared) -> ClipSettings {
    let cfg = shared.config.lock().unwrap();
    let number = |name: &str, fallback: u64| cfg.get(name).and_then(Value::as_u64).unwrap_or(fallback);
    ClipSettings {
        enabled: key_on(&cfg),
        fps: if number("clip_fps", 30) == 60 { 60 } else { 30 },
        audio: cfg.get("clip_audio").and_then(Value::as_bool).unwrap_or(true),
        quality: quality_of(&cfg.str("clip_quality")),
        max: match number("clip_max_seconds", 60) {
            0 => u64::MAX,
            seconds => seconds.clamp(10, 3600),
        },
        replay: replay_length(&cfg),
    }
}

fn open_session(shared: &Shared, target: Option<Output>, clip: &ClipSettings) -> Result<Session, String> {
    let snap = shared.snapshot();
    if !snap.detected || snap.hwnd == 0 {
        return Err("No visual novel is running.".into());
    }
    if !GraphicsCaptureApi::is_supported().unwrap_or(false) {
        return Err("Clips need Windows 10 version 1903 or newer.".into());
    }
    let pipe = Arc::new(Pipe {
        slot: Mutex::new(None),
        picture: Mutex::new(Picture::default()),
        target: Mutex::new(target),
        hwnd: snap.hwnd,
        fps: clip.fps,
        audio: clip.audio,
        quality: clip.quality,
        closed: AtomicBool::new(false),
    });
    let border = if GraphicsCaptureApi::is_border_settings_supported().unwrap_or(false) {
        DrawBorderSettings::WithoutBorder
    } else {
        DrawBorderSettings::Default
    };
    let settings = Settings::new(
        Window::from_raw_hwnd(snap.hwnd as *mut std::ffi::c_void),
        CursorCaptureSettings::WithoutCursor,
        border,
        SecondaryWindowSettings::Default,
        MinimumUpdateIntervalSettings::Default,
        DirtyRegionSettings::Default,
        ColorFormat::Bgra8,
        Arc::clone(&pipe),
    );
    let control = Recorder::start_free_threaded(settings).map_err(|e| format!("couldn't film the window ({e})"))?;
    if clip.audio {
        listen(snap.pid, &pipe);
    }
    Ok(Session { control, pipe, started: Instant::now(), key: snap.key })
}

fn close_session(session: Session) -> Option<Active> {
    unlisten(&session.pipe);
    let _ = session.control.stop();
    session.pipe.rotate(None).ok().flatten()
}

fn memory_bytes(stream: &InMemoryRandomAccessStream) -> Result<Vec<u8>, String> {
    let size = stream.Size().map_err(|e| e.to_string())? as u32;
    let input = stream.GetInputStreamAt(0).map_err(|e| e.to_string())?;
    let reader = DataReader::CreateDataReader(&input).map_err(|e| e.to_string())?;
    reader.LoadAsync(size).map_err(|e| e.to_string())?.join().map_err(|e| e.to_string())?;
    let mut bytes = vec![0u8; size as usize];
    reader.ReadBytes(&mut bytes).map_err(|e| e.to_string())?;
    Ok(bytes)
}

fn toast(shared: &Shared, title: &str, detail: String, ok: bool) {
    ui_windows::show_toast(
        &shared.app,
        Toast { title: title.into(), detail, image: None, ok, ..Default::default() },
        None,
    );
}

fn clip_folder(shared: &Shared, key: &str) -> Result<PathBuf, String> {
    let folder = {
        let mut cfg = shared.config.lock().unwrap();
        screenshots::folder_for(&mut cfg, key, true).ok_or("no screenshot folder")?
    };
    std::fs::create_dir_all(&folder).map_err(|e| e.to_string())?;
    let _ = shared.app.asset_protocol_scope().allow_directory(&folder, false);
    Ok(folder)
}

fn clip_name(folder: &Path) -> PathBuf {
    let stem = chrono::Local::now().format("%Y-%m-%d %H-%M-%S").to_string();
    let mut path = folder.join(format!("{stem}.mp4"));
    let mut n = 2;
    while path.exists() {
        path = folder.join(format!("{stem} ({n}).mp4"));
        n += 1;
    }
    path
}

fn length_text(seconds: u64) -> String {
    format!("{}:{:02}", seconds / 60, seconds % 60)
}

static RECORDING: Mutex<Option<(Session, PathBuf, PathBuf)>> = Mutex::new(None);
static BUSY: AtomicBool = AtomicBool::new(false);

pub fn is_recording() -> bool {
    RECORDING.lock().unwrap().is_some()
}

pub fn state() -> Value {
    let recording = RECORDING.lock().unwrap();
    let replay = REPLAY.lock().unwrap().is_some();
    match recording.as_ref() {
        Some((session, _, _)) => {
            json!({ "recording": true, "seconds": session.started.elapsed().as_secs(), "replay": replay })
        }
        None => json!({ "recording": false, "replay": replay }),
    }
}

fn announce(shared: &Shared) {
    shared.emit("clip-state", state());
}

pub fn toggle(shared: &Arc<Shared>) {
    if BUSY.swap(true, Ordering::SeqCst) {
        return;
    }
    let result = if is_recording() { stop(shared) } else { start(shared) };
    BUSY.store(false, Ordering::SeqCst);
    if let Err(message) = result {
        toast(shared, "No clip", message, false);
    }
    announce(shared);
}

fn start(shared: &Arc<Shared>) -> Result<(), String> {
    let clip = settings_of(shared);
    if !clip.enabled {
        return Err("Clips are turned off in the settings.".into());
    }
    let key = shared.snapshot().key;
    let folder = clip_folder(shared, &key)?;
    let path = clip_name(&folder);
    let part = path.with_extension("recording.mp4");
    let session = open_session(shared, Some(Output::File(part.clone())), &clip)?;
    crate::crash::note("clip started");
    *RECORDING.lock().unwrap() = Some((session, part, path));

    let (hotkey, language) = {
        let cfg = shared.config.lock().unwrap();
        (cfg.str("clip_hotkey"), crate::lang::resolve(&cfg.str("language")))
    };
    let detail = crate::lang::tr("Press {} again to stop.", language).replace("{}", &hotkey);
    toast(shared, "Recording a clip", detail, true);
    watch(Arc::clone(shared), clip.max);
    Ok(())
}

fn watch(shared: Arc<Shared>, max: u64) {
    let mut shown = 0;
    std::thread::spawn(move || loop {
        std::thread::sleep(Duration::from_millis(250));
        let (seconds, done) = {
            let recording = RECORDING.lock().unwrap();
            let Some((session, _, _)) = recording.as_ref() else {
                return;
            };
            let snap = shared.snapshot();
            let seconds = session.started.elapsed().as_secs();
            let gone = session.pipe.closed.load(Ordering::SeqCst) || !snap.detected || snap.hwnd != session.pipe.hwnd;
            (seconds, seconds >= max || gone)
        };
        if done {
            toggle(&shared);
            return;
        }
        if seconds != shown {
            shown = seconds;
            announce(&shared);
        }
    });
}

fn stop(shared: &Arc<Shared>) -> Result<(), String> {
    let Some((session, part, path)) = RECORDING.lock().unwrap().take() else {
        return Ok(());
    };
    announce(shared);
    let (key, seconds) = (session.key.clone(), session.started.elapsed().as_secs());
    let Some(active) = close_session(session) else {
        let _ = std::fs::remove_file(&part);
        return Err("Nothing moved on screen, so there's nothing to save.".into());
    };
    if let Err(error) = active.encoder.finish() {
        let _ = std::fs::remove_file(&part);
        return Err(format!("couldn't save the clip ({error})"));
    }
    std::fs::rename(&part, &path).map_err(|e| e.to_string())?;
    crate::crash::note("clip saved");
    shared.emit("screenshots-changed", json!({ "key": key }));
    let name = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    toast(shared, "Clip saved", format!("{name} · {}", length_text(seconds)), true);
    Ok(())
}

struct Piece {
    bytes: Vec<u8>,
    seconds: f64,
}

struct Replay {
    session: Session,
    pieces: VecDeque<Piece>,
    keep: u64,
}

static REPLAY: Mutex<Option<Replay>> = Mutex::new(None);
static SAVING: AtomicBool = AtomicBool::new(false);

fn finish_piece(active: Active) -> Option<Piece> {
    let seconds = active.started.elapsed().as_secs_f64();
    let Output::Memory(stream) = &active.output else {
        return None;
    };
    let stream = stream.clone();
    active.encoder.finish().ok()?;
    let bytes = memory_bytes(&stream).ok()?;
    (!bytes.is_empty()).then_some(Piece { bytes, seconds })
}

fn trim(pieces: &mut VecDeque<Piece>, keep: u64) {
    let wanted = keep as f64 + SEGMENT.as_secs_f64();
    while pieces.len() > 1 && pieces.iter().skip(1).map(|p| p.seconds).sum::<f64>() >= wanted {
        pieces.pop_front();
    }
}

fn replay_tick(shared: &Arc<Shared>) {
    let clip = settings_of(shared);
    let snap = shared.snapshot();
    if !clip.enabled && is_recording() {
        toggle(shared);
    }
    let wanted = clip.replay > 0 && snap.detected && snap.hwnd != 0;
    let mut replay = REPLAY.lock().unwrap();

    let stale = replay.as_ref().is_some_and(|r| {
        !wanted
            || r.session.pipe.hwnd != snap.hwnd
            || r.session.pipe.closed.load(Ordering::SeqCst)
            || r.session.pipe.fps != clip.fps
            || r.session.pipe.audio != clip.audio
            || (r.session.pipe.quality - clip.quality).abs() > f64::EPSILON
    });
    if stale {
        if let Some(old) = replay.take() {
            drop(replay);
            if let Some(active) = close_session(old.session) {
                let _ = active.encoder.finish();
            }
            announce(shared);
            return;
        }
    }
    if wanted && replay.is_none() {
        match open_session(shared, None, &clip) {
            Ok(session) => {
                crate::crash::note("replay started");
                *replay = Some(Replay { session, pieces: VecDeque::new(), keep: clip.replay });
                drop(replay);
                announce(shared);
            }
            Err(error) => crate::crash::note(format!("replay: {error}")),
        }
        return;
    }
    let Some(r) = replay.as_mut() else {
        return;
    };
    r.keep = clip.replay;
    let due = r.session.pipe.slot.lock().unwrap().as_ref().is_some_and(|a| a.started.elapsed() >= SEGMENT);
    if due && !SAVING.load(Ordering::SeqCst) {
        let Ok(next) = InMemoryRandomAccessStream::new() else {
            return;
        };
        if let Ok(Some(old)) = r.session.pipe.rotate(Some(Output::Memory(next))) {
            if let Some(piece) = finish_piece(old) {
                r.pieces.push_back(piece);
            }
            let keep = r.keep;
            trim(&mut r.pieces, keep);
        }
    }
}

pub fn start_replay_watch(shared: &Arc<Shared>) {
    let shared = Arc::clone(shared);
    std::thread::spawn(move || loop {
        std::thread::sleep(Duration::from_secs(1));
        replay_tick(&shared);
    });
}

fn windows_path(path: &Path) -> HSTRING {
    HSTRING::from(path.to_string_lossy().replace('/', "\\"))
}

fn compose(pieces: &[(PathBuf, f64)], keep: f64, output: &Path, fps: u32, quality: f64) -> Result<(), String> {
    let _ = wasapi::initialize_mta();
    let e = |e: windows::core::Error| e.to_string();
    let composition = MediaComposition::new().map_err(e)?;
    let total: f64 = pieces.iter().map(|(_, s)| s).sum();
    let mut cut = (total - keep).max(0.0);
    let mut first_file = None;
    for (path, seconds) in pieces {
        if cut >= *seconds {
            cut -= seconds;
            continue;
        }
        let file = StorageFile::GetFileFromPathAsync(&windows_path(path)).map_err(e)?.join().map_err(e)?;
        let clip = MediaClip::CreateFromFileAsync(&file).map_err(e)?.join().map_err(e)?;
        if cut > 0.0 {
            clip.SetTrimTimeFromStart(TimeSpan { Duration: (cut * TICKS as f64) as i64 }).map_err(e)?;
            cut = 0.0;
        }
        composition.Clips().map_err(e)?.Append(&clip).map_err(e)?;
        first_file.get_or_insert(file);
    }
    let first = first_file.ok_or("nothing recorded yet")?;
    let profile = MediaEncodingProfile::CreateFromFileAsync(&first).map_err(e)?.join().map_err(e)?;
    if let Ok(video) = profile.Video() {
        let (width, height) = (video.Width().map_err(e)?, video.Height().map_err(e)?);
        video.SetBitrate(bitrate(width, height, fps, quality)).map_err(e)?;
    }
    let folder_path = output.parent().ok_or("bad folder")?;
    let name = output.file_name().ok_or("bad name")?;
    let folder = StorageFolder::GetFolderFromPathAsync(&windows_path(folder_path))
        .map_err(e)?
        .join()
        .map_err(e)?;
    let target = folder
        .CreateFileAsync(&HSTRING::from(name), CreationCollisionOption::ReplaceExisting)
        .map_err(e)?
        .join()
        .map_err(e)?;
    composition
        .RenderToFileWithProfileAsync(&target, MediaTrimmingPreference::Precise, &profile)
        .map_err(e)?
        .join()
        .map_err(e)?;
    Ok(())
}

pub fn save_replay(shared: &Arc<Shared>) {
    if SAVING.swap(true, Ordering::SeqCst) {
        return;
    }
    let result = save_replay_now(shared);
    SAVING.store(false, Ordering::SeqCst);
    match result {
        Ok((path, key, seconds)) => {
            shared.emit("screenshots-changed", json!({ "key": key }));
            let name = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
            toast(shared, "Clip saved", format!("{name} · {}", length_text(seconds)), true);
        }
        Err(message) => toast(shared, "No clip", message, false),
    }
}

fn save_replay_now(shared: &Arc<Shared>) -> Result<(PathBuf, String, u64), String> {
    let (pieces, keep, key, fps, quality) = {
        let mut replay = REPLAY.lock().unwrap();
        let Some(r) = replay.as_mut() else {
            return Err("Turn on “Instant replay” in Settings → Clips first.".into());
        };
        let next = InMemoryRandomAccessStream::new().map_err(|e| e.to_string())?;
        if let Ok(Some(old)) = r.session.pipe.rotate(Some(Output::Memory(next))) {
            if let Some(piece) = finish_piece(old) {
                r.pieces.push_back(piece);
            }
        }
        let keep = r.keep;
        trim(&mut r.pieces, keep);
        let pieces: Vec<(Vec<u8>, f64)> = r.pieces.iter().map(|p| (p.bytes.clone(), p.seconds)).collect();
        (pieces, keep, r.session.key.clone(), r.session.pipe.fps, r.session.pipe.quality)
    };
    if pieces.is_empty() {
        return Err("Nothing moved on screen, so there's nothing to save.".into());
    }
    toast(shared, "Saving the clip…", length_text(keep), true);

    let temp = shared.data_dir.join("cache").join("replay");
    let _ = std::fs::remove_dir_all(&temp);
    std::fs::create_dir_all(&temp).map_err(|e| e.to_string())?;
    let mut files = Vec::new();
    for (i, (bytes, seconds)) in pieces.iter().enumerate() {
        let path = temp.join(format!("{i:03}.mp4"));
        std::fs::write(&path, bytes).map_err(|e| e.to_string())?;
        files.push((path, *seconds));
    }
    let rendered = temp.join("clip.mp4");
    let result = compose(&files, keep as f64, &rendered, fps, quality).and_then(|()| {
        let folder = clip_folder(shared, &key)?;
        let path = clip_name(&folder);
        if std::fs::rename(&rendered, &path).is_err() {
            std::fs::copy(&rendered, &path).map_err(|e| e.to_string())?;
        }
        Ok(path)
    });
    let _ = std::fs::remove_dir_all(&temp);
    let path = result.map_err(|e| format!("couldn't save the clip ({e})"))?;
    let seconds = files.iter().map(|(_, s)| s).sum::<f64>().min(keep as f64).round() as u64;
    Ok((path, key, seconds))
}

pub fn stop_on_exit() {
    if let Some((session, part, path)) = RECORDING.lock().unwrap().take() {
        if let Some(active) = close_session(session) {
            if active.encoder.finish().is_ok() {
                let _ = std::fs::rename(&part, &path);
            }
        }
    }
    if let Some(replay) = REPLAY.lock().unwrap().take() {
        if let Some(active) = close_session(replay.session) {
            let _ = active.encoder.finish();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_are_even_and_bitrate_stays_sane() {
        assert_eq!(even(1281), 1280);
        assert_eq!(even(1), 2);
        assert_eq!(bitrate(320, 240, 30, quality_of("high")), 4_000_000);
        assert_eq!(bitrate(3840, 2160, 60, quality_of("best")), 60_000_000);
        let hd = bitrate(1920, 1080, 30, quality_of("high"));
        assert!(hd > 15_000_000 && hd < 21_000_000);
    }

    #[test]
    fn frames_are_flipped_and_padded() {
        let picture = Picture { data: vec![1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3], width: 1, height: 3 };
        let mut out = Vec::new();
        flip_into(&picture, 2, 2, &mut out);
        assert_eq!(out, vec![2, 2, 2, 2, 0, 0, 0, 0, 1, 1, 1, 1, 0, 0, 0, 0]);
    }

    #[test]
    fn replay_keeps_just_enough_pieces() {
        let mut pieces: VecDeque<Piece> = (0..10).map(|_| Piece { bytes: vec![1], seconds: 10.0 }).collect();
        trim(&mut pieces, 30);
        assert_eq!(pieces.len(), 4);
    }
}
