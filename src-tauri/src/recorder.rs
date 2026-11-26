use std::collections::VecDeque;
use std::fs::{self, File};
use std::io::{self, BufRead, BufReader, Write};
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStderr, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{sync_channel, Receiver, RecvTimeoutError, SyncSender};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant, SystemTime};

use serde_json::json;
use windows::core::HSTRING;
use windows::Win32::Foundation::{CloseHandle, ERROR_PIPE_CONNECTED, HANDLE};
use windows::Win32::Storage::FileSystem::{WriteFile, PIPE_ACCESS_OUTBOUND};
use windows::Win32::System::Pipes::{ConnectNamedPipe, CreateNamedPipeW, PIPE_TYPE_BYTE, PIPE_WAIT};
use zip::ZipArchive;

use crate::download::{self, Download, CANCELLED};
use crate::state::Shared;

const NO_WINDOW: u32 = 0x0800_0000;
const EXE: &str = "ffmpeg.exe";
const ENTRY: &str = "ffmpeg-9.0.2-essentials_build/bin/ffmpeg.exe";
const ARCHIVE: Download = Download {
    url: "https://github.com/GyanD/codexffmpeg/releases/download/9.0.2/ffmpeg-9.0.2-essentials_build.zip",
    sha256: "60f467265b1e312373dbcd92200c2618a74850f98d3d078e94296bb3fa2047ba",
    size: 114_768_076,
};
const COULD_NOT_DOWNLOAD: &str = "Couldn't download the HD recorder. Clips keep using the basic one.";
const SAMPLE_RATE: u64 = 48_000;
const BLOCK: usize = 4;
const SEGMENT_SECONDS: u64 = 10;
const START_TIMEOUT: Duration = Duration::from_secs(10);
const STOP_TIMEOUT: Duration = Duration::from_secs(20);
const LOG_LINES: usize = 40;

static DOWNLOADING: AtomicBool = AtomicBool::new(false);
static STOP: AtomicBool = AtomicBool::new(false);
static WORKING: Mutex<Option<usize>> = Mutex::new(None);
static PIPES: AtomicU64 = AtomicU64::new(0);

struct Candidate {
    codec: &'static str,
    label: &'static str,
    gpu_frames: bool,
}

const CANDIDATES: [Candidate; 6] = [
    Candidate { codec: "h264_nvenc", label: "NVIDIA", gpu_frames: true },
    Candidate { codec: "h264_amf", label: "AMD", gpu_frames: true },
    Candidate { codec: "h264_nvenc", label: "NVIDIA", gpu_frames: false },
    Candidate { codec: "h264_amf", label: "AMD", gpu_frames: false },
    Candidate { codec: "h264_qsv", label: "Intel", gpu_frames: false },
    Candidate { codec: "libx264", label: "CPU", gpu_frames: false },
];

fn folder(shared: &Shared) -> PathBuf {
    shared.data_dir.join("recorder")
}

pub fn replay_root(shared: &Shared) -> PathBuf {
    shared.data_dir.join("cache").join("replay-clips")
}

pub fn exe(shared: &Shared) -> Option<PathBuf> {
    let path = folder(shared).join(EXE);
    path.is_file().then_some(path)
}

pub fn status(shared: &Shared) -> serde_json::Value {
    let encoder = WORKING.lock().unwrap().map(|index| CANDIDATES[index].label);
    json!({ "ready": exe(shared).is_some(), "downloading": DOWNLOADING.load(Ordering::SeqCst), "encoder": encoder })
}

pub fn cancel() {
    STOP.store(true, Ordering::SeqCst);
}

pub fn remove(shared: &Shared) -> Result<(), String> {
    *WORKING.lock().unwrap() = None;
    fs::remove_file(folder(shared).join(EXE)).map_err(|e| e.to_string())
}

pub fn clean_unfinished(shared: &Shared) {
    for entry in fs::read_dir(folder(shared)).into_iter().flatten().flatten() {
        if entry.path().extension().is_some_and(|ext| ext == "part") {
            let _ = fs::remove_file(entry.path());
        }
    }
    let _ = fs::remove_dir_all(replay_root(shared));
}

pub fn download(shared: &Shared) -> Result<bool, String> {
    if DOWNLOADING.swap(true, Ordering::SeqCst) {
        return Err(COULD_NOT_DOWNLOAD.into());
    }
    STOP.store(false, Ordering::SeqCst);
    let result = download_now(shared);
    DOWNLOADING.store(false, Ordering::SeqCst);
    match result {
        Err(error) if error == CANCELLED => Ok(false),
        Err(error) => {
            crate::crash::note(format!("recorder download: {error}"));
            Err(COULD_NOT_DOWNLOAD.into())
        }
        Ok(()) => Ok(true),
    }
}

fn download_now(shared: &Shared) -> Result<(), String> {
    let dir = folder(shared);
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let archive = dir.join("ffmpeg.zip.part");
    let client = download::client()?;
    let (mut done, mut shown) = (0u64, u64::MAX);
    download::fetch(&client, &ARCHIVE, &archive, &STOP, |chunk| {
        done += chunk;
        let percent = done * 100 / ARCHIVE.size;
        if percent != shown {
            shown = percent;
            shared.emit(
                "recorder-progress",
                json!({ "percent": percent, "done": done / 1_000_000, "total": ARCHIVE.size / 1_000_000 }),
            );
        }
    })?;
    let unpacked = unpack(&archive, &dir.join(EXE));
    let _ = fs::remove_file(&archive);
    unpacked
}

fn unpack(archive: &Path, exe: &Path) -> Result<(), String> {
    let file = File::open(archive).map_err(|e| e.to_string())?;
    let mut archive = ZipArchive::new(file).map_err(|e| e.to_string())?;
    let mut entry = archive.by_name(ENTRY).map_err(|e| e.to_string())?;
    let part = exe.with_extension("exe.part");
    let mut out = File::create(&part).map_err(|e| e.to_string())?;
    io::copy(&mut entry, &mut out).map_err(|e| e.to_string())?;
    drop(out);
    fs::rename(&part, exe).map_err(|e| e.to_string())
}

pub struct Size {
    pub width: u32,
    pub height: u32,
}

pub enum Target {
    File(PathBuf),
    Segments { folder: PathBuf, keep: u64 },
}

pub struct Options<'a> {
    pub exe: &'a Path,
    pub view: isize,
    pub size: Size,
    pub fps: u32,
    pub level: u8,
    pub audio: bool,
    pub target: Target,
}

fn video_filter(options: &Options, candidate: &Candidate) -> String {
    let mut filter = format!(
        "gfxcapture=hwnd={}:capture_cursor=0:max_framerate={}:width=-2:height=-2,fps={}",
        options.view, options.fps, options.fps
    );
    if !candidate.gpu_frames {
        let format = if candidate.codec == "libx264" { "yuv420p" } else { "nv12" };
        filter.push_str(&format!(
            ",hwdownload,format=bgra,scale=out_color_matrix=bt601:out_range=tv,format={format}"
        ));
    }
    filter.push_str(",setparams=range=tv:colorspace=bt470bg:color_primaries=bt709:color_trc=bt709[v]");
    filter
}

fn max_rate(size: &Size, fps: u32) -> u64 {
    let bits = u64::from(size.width) * u64::from(size.height) * u64::from(fps) / 2;
    bits.clamp(8_000_000, 80_000_000)
}

fn encoder_args(candidate: &Candidate, level: u8, max_rate: u64) -> Vec<String> {
    let q = [28, 23, 19][usize::from(level.min(2))];
    let args: Vec<String> = match candidate.codec {
        "h264_nvenc" => vec![
            "-preset".into(),
            "p5".into(),
            "-tune".into(),
            "hq".into(),
            "-rc".into(),
            "vbr".into(),
            "-cq".into(),
            q.to_string(),
            "-b:v".into(),
            "0".into(),
            "-maxrate".into(),
            max_rate.to_string(),
            "-bufsize".into(),
            (max_rate * 2).to_string(),
            "-spatial-aq".into(),
            "1".into(),
        ],
        "h264_amf" => vec![
            "-quality".into(),
            "quality".into(),
            "-rc".into(),
            "cqp".into(),
            "-qp_i".into(),
            q.to_string(),
            "-qp_p".into(),
            (q + 2).to_string(),
        ],
        "h264_qsv" => vec!["-preset".into(), "medium".into(), "-global_quality".into(), q.to_string()],
        _ => vec!["-preset".into(), "veryfast".into(), "-crf".into(), q.to_string()],
    };
    let mut all = vec!["-c:v".into(), candidate.codec.into()];
    all.extend(args);
    all.extend(["-profile:v".into(), "high".into()]);
    all
}

fn wrap(keep: u64) -> u64 {
    keep / SEGMENT_SECONDS + 3
}

#[derive(Clone, Copy, PartialEq, PartialOrd, Debug)]
enum Phase {
    Starting,
    Capturing,
    Ready,
    Ended,
}

struct Watch {
    phase: Mutex<Phase>,
    changed: Condvar,
    log: Mutex<VecDeque<String>>,
}

impl Watch {
    fn new() -> Self {
        Self { phase: Mutex::new(Phase::Starting), changed: Condvar::new(), log: Mutex::new(VecDeque::new()) }
    }

    fn reach(&self, phase: Phase) {
        let mut current = self.phase.lock().unwrap();
        if phase > *current {
            *current = phase;
            self.changed.notify_all();
        }
    }

    fn wait_for(&self, phase: Phase, timeout: Duration) -> Phase {
        let current = self.phase.lock().unwrap();
        let (current, _) = self.changed.wait_timeout_while(current, timeout, |now| *now < phase).unwrap();
        *current
    }

    fn now(&self) -> Phase {
        *self.phase.lock().unwrap()
    }

    fn remember(&self, line: String) {
        let mut log = self.log.lock().unwrap();
        log.push_back(line);
        while log.len() > LOG_LINES {
            log.pop_front();
        }
    }

    fn tail(&self) -> String {
        let log = self.log.lock().unwrap();
        let errors: Vec<&String> =
            log.iter().filter(|line| line.to_lowercase().contains("error") || line.contains("failed")).collect();
        let picked: Vec<&String> = if errors.is_empty() { log.iter().rev().take(3).collect() } else { errors };
        picked.iter().map(|line| line.as_str()).collect::<Vec<_>>().join(" | ")
    }
}

fn read_log(stderr: ChildStderr, watch: Arc<Watch>) {
    let mut reader = BufReader::new(stderr);
    let mut line = Vec::new();
    loop {
        line.clear();
        match reader.read_until(b'\n', &mut line) {
            Ok(0) | Err(_) => break,
            Ok(_) => {}
        }
        let text = String::from_utf8_lossy(&line).trim().to_string();
        if text.contains("Capture size changed") {
            watch.reach(Phase::Capturing);
        } else if text.starts_with("Output #0") {
            watch.reach(Phase::Ready);
        }
        watch.remember(text);
    }
    watch.reach(Phase::Ended);
}

struct AudioPipe {
    name: String,
    handle: isize,
}

impl AudioPipe {
    fn create() -> Result<Self, String> {
        let name = format!(r"\\.\pipe\shiori-clip-{}-{}", std::process::id(), PIPES.fetch_add(1, Ordering::SeqCst));
        let handle = unsafe {
            CreateNamedPipeW(&HSTRING::from(&name), PIPE_ACCESS_OUTBOUND, PIPE_TYPE_BYTE | PIPE_WAIT, 1, 1 << 20, 0, 0, None)
        };
        if handle.is_invalid() {
            return Err("couldn't open the sound pipe".into());
        }
        Ok(Self { name, handle: handle.0 as isize })
    }

    fn raw(&self) -> HANDLE {
        HANDLE(self.handle as *mut std::ffi::c_void)
    }

    fn connect(&self) -> bool {
        match unsafe { ConnectNamedPipe(self.raw(), None) } {
            Ok(()) => true,
            Err(error) => error.code() == ERROR_PIPE_CONNECTED.to_hresult(),
        }
    }

    fn write(&self, bytes: &[u8]) -> bool {
        let mut written = 0;
        unsafe { WriteFile(self.raw(), Some(bytes), Some(&mut written), None) }.is_ok()
    }
}

impl Drop for AudioPipe {
    fn drop(&mut self) {
        let _ = unsafe { CloseHandle(self.raw()) };
    }
}

pub struct Feed {
    chunks: Mutex<Option<SyncSender<Vec<u8>>>>,
}

impl Feed {
    pub fn hear(&self, fresh: &[u8]) {
        if let Some(chunks) = self.chunks.lock().unwrap().as_ref() {
            let _ = chunks.try_send(fresh.to_vec());
        }
    }

    fn close(&self) {
        self.chunks.lock().unwrap().take();
    }
}

fn pump(pipe: AudioPipe, chunks: Receiver<Vec<u8>>, watch: Arc<Watch>, connected: Arc<AtomicBool>) {
    if !pipe.connect() {
        return;
    }
    connected.store(true, Ordering::SeqCst);
    if watch.wait_for(Phase::Capturing, START_TIMEOUT) == Phase::Ended {
        return;
    }
    while chunks.try_recv().is_ok() {}
    let start = Instant::now();
    let mut written = 0u64;
    loop {
        let chunk = match chunks.recv_timeout(Duration::from_millis(20)) {
            Ok(chunk) => Some(chunk),
            Err(RecvTimeoutError::Timeout) => None,
            Err(RecvTimeoutError::Disconnected) => break,
        };
        let expected = (start.elapsed().as_secs_f64() * SAMPLE_RATE as f64) as u64;
        let have = chunk.as_ref().map_or(0, |chunk| (chunk.len() / BLOCK) as u64);
        let behind = expected.saturating_sub(written + have);
        if behind > SAMPLE_RATE / 10 {
            if !pipe.write(&vec![0; behind as usize * BLOCK]) {
                break;
            }
            written += behind;
        }
        if let Some(chunk) = chunk {
            if !chunk.is_empty() && !pipe.write(&chunk[..have as usize * BLOCK]) {
                break;
            }
            written += have;
        }
    }
}

pub struct Capture {
    child: Child,
    stdin: Option<ChildStdin>,
    feed: Option<Arc<Feed>>,
    watch: Arc<Watch>,
    pipe_name: Option<String>,
    connected: Arc<AtomicBool>,
}

impl Capture {
    pub fn start(options: &Options) -> Result<Capture, String> {
        let first = WORKING.lock().unwrap().unwrap_or(0);
        let order = std::iter::once(first).chain((0..CANDIDATES.len()).filter(move |&index| index != first));
        let mut last = String::from("no encoder worked");
        for index in order {
            let candidate = &CANDIDATES[index];
            match Self::launch(options, candidate) {
                Ok(capture) => {
                    *WORKING.lock().unwrap() = Some(index);
                    return Ok(capture);
                }
                Err(error) => {
                    crate::crash::note(format!("recorder {} gpu={}: {error}", candidate.codec, candidate.gpu_frames));
                    last = error;
                }
            }
            if let Target::Segments { folder, .. } = &options.target {
                let _ = fs::remove_dir_all(folder);
                let _ = fs::create_dir_all(folder);
            }
        }
        Err(last)
    }

    fn launch(options: &Options, candidate: &Candidate) -> Result<Capture, String> {
        let pipe = if options.audio { Some(AudioPipe::create()?) } else { None };
        let mut command = Command::new(options.exe);
        command.args(["-hide_banner", "-nostats", "-loglevel", "verbose", "-y"]);
        if let Some(pipe) = &pipe {
            command.args(["-thread_queue_size", "4096", "-probesize", "32", "-analyzeduration", "0"]);
            command.args(["-f", "s16le", "-ar", "48000", "-ac", "2", "-i", &pipe.name]);
        }
        command.args(["-filter_complex", &video_filter(options, candidate), "-map", "[v]"]);
        if pipe.is_some() {
            command.args(["-map", "0:a", "-c:a", "aac", "-b:a", "192k"]);
        }
        command.args(encoder_args(candidate, options.level, max_rate(&options.size, options.fps)));
        command.args(["-g", &(options.fps * 2).to_string()]);
        match &options.target {
            Target::File(path) => {
                command.args(["-movflags", "+faststart"]).arg(path);
            }
            Target::Segments { folder, keep } => {
                command.args(["-f", "segment", "-segment_format", "mpegts", "-reset_timestamps", "1"]);
                command.args(["-segment_time", &SEGMENT_SECONDS.to_string(), "-segment_wrap", &wrap(*keep).to_string()]);
                command.arg(folder.join("piece%03d.ts"));
            }
        }
        command.stdin(Stdio::piped()).stdout(Stdio::null()).stderr(Stdio::piped()).creation_flags(NO_WINDOW);
        let mut child = command.spawn().map_err(|e| e.to_string())?;

        let watch = Arc::new(Watch::new());
        let stderr = child.stderr.take();
        let log_watch = Arc::clone(&watch);
        if let Some(stderr) = stderr {
            std::thread::spawn(move || read_log(stderr, log_watch));
        }
        let connected = Arc::new(AtomicBool::new(false));
        let pipe_name = pipe.as_ref().map(|pipe| pipe.name.clone());
        let feed = pipe.map(|pipe| {
            let (sender, receiver) = sync_channel(512);
            let (pump_watch, pump_connected) = (Arc::clone(&watch), Arc::clone(&connected));
            std::thread::spawn(move || pump(pipe, receiver, pump_watch, pump_connected));
            Arc::new(Feed { chunks: Mutex::new(Some(sender)) })
        });
        let stdin = child.stdin.take();
        let capture = Capture { child, stdin, feed, watch, pipe_name, connected };
        match capture.watch.wait_for(Phase::Ready, START_TIMEOUT) {
            Phase::Ready => Ok(capture),
            Phase::Ended => Err(capture.watch.tail()),
            _ => Err(format!("the recorder didn't start ({})", capture.watch.tail())),
        }
    }

    pub fn feed(&self) -> Option<Arc<Feed>> {
        self.feed.clone()
    }

    pub fn closed(&self) -> bool {
        self.watch.now() == Phase::Ended
    }

    pub fn finish(mut self) -> Result<(), String> {
        if let Some(mut stdin) = self.stdin.take() {
            let _ = stdin.write_all(b"q");
            let _ = stdin.flush();
        }
        if let Some(feed) = &self.feed {
            feed.close();
        }
        let deadline = Instant::now() + STOP_TIMEOUT;
        loop {
            match self.child.try_wait() {
                Ok(Some(status)) if status.success() => return Ok(()),
                Ok(Some(_)) => return Err(self.watch.tail()),
                Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(50)),
                _ => return Err("the recorder didn't stop in time".into()),
            }
        }
    }
}

impl Drop for Capture {
    fn drop(&mut self) {
        if let Some(feed) = &self.feed {
            feed.close();
        }
        if !matches!(self.child.try_wait(), Ok(Some(_))) {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
        if let Some(name) = &self.pipe_name {
            if !self.connected.load(Ordering::SeqCst) {
                let _ = fs::OpenOptions::new().read(true).open(name);
            }
        }
    }
}

fn run(exe: &Path, args: &[&std::ffi::OsStr]) -> Result<String, String> {
    let output = Command::new(exe)
        .args(["-hide_banner", "-nostats", "-y"])
        .args(args)
        .stdin(Stdio::null())
        .creation_flags(NO_WINDOW)
        .output()
        .map_err(|e| e.to_string())?;
    let log = String::from_utf8_lossy(&output.stderr).to_string();
    if output.status.success() {
        Ok(log)
    } else {
        Err(log.lines().rev().take(2).collect::<Vec<_>>().join(" | "))
    }
}

fn duration_in(log: &str) -> Option<f64> {
    let at = log.find("Duration: ")? + "Duration: ".len();
    let text = log.get(at..at + 11)?;
    let mut parts = text.split(':');
    let hours: f64 = parts.next()?.parse().ok()?;
    let minutes: f64 = parts.next()?.parse().ok()?;
    let seconds: f64 = parts.next()?.parse().ok()?;
    Some(hours * 3600.0 + minutes * 60.0 + seconds)
}

fn newest_pieces(folder: &Path, keep: u64) -> Vec<PathBuf> {
    let mut pieces: Vec<(SystemTime, PathBuf)> = fs::read_dir(folder)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|entry| entry.path().extension().is_some_and(|ext| ext == "ts"))
        .filter_map(|entry| Some((entry.metadata().ok()?.modified().ok()?, entry.path())))
        .filter(|(_, path)| fs::metadata(path).is_ok_and(|meta| meta.len() > 0))
        .collect();
    pieces.sort();
    let wanted = (keep / SEGMENT_SECONDS + 2) as usize;
    let skip = pieces.len().saturating_sub(wanted);
    pieces.into_iter().skip(skip).map(|(_, path)| path).collect()
}

fn join_list(pieces: &[PathBuf]) -> String {
    pieces
        .iter()
        .map(|path| format!("file '{}'\n", path.to_string_lossy().replace('\\', "/").replace('\'', "'\\''")))
        .collect()
}

pub fn save_replay(exe: &Path, folder: &Path, keep: u64, work: &Path, output: &Path) -> Result<u64, String> {
    let pieces = newest_pieces(folder, keep);
    if pieces.is_empty() {
        return Err("Nothing was recorded yet.".into());
    }
    fs::create_dir_all(work).map_err(|e| e.to_string())?;
    let list = work.join("join.txt");
    fs::write(&list, join_list(&pieces)).map_err(|e| e.to_string())?;
    let joined = work.join("joined.mp4");
    run(
        exe,
        &[
            "-f".as_ref(),
            "concat".as_ref(),
            "-safe".as_ref(),
            "0".as_ref(),
            "-i".as_ref(),
            list.as_os_str(),
            "-c".as_ref(),
            "copy".as_ref(),
            "-bsf:a".as_ref(),
            "aac_adtstoasc".as_ref(),
            joined.as_os_str(),
        ],
    )?;
    let back = format!("-{keep}");
    let log = run(
        exe,
        &[
            "-sseof".as_ref(),
            back.as_ref(),
            "-i".as_ref(),
            joined.as_os_str(),
            "-c".as_ref(),
            "copy".as_ref(),
            "-movflags".as_ref(),
            "+faststart".as_ref(),
            output.as_os_str(),
        ],
    )?;
    let total = duration_in(&log).unwrap_or(keep as f64);
    Ok(total.min(keep as f64).round() as u64)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn size() -> Size {
        Size { width: 1280, height: 720 }
    }

    #[test]
    fn the_filter_films_the_window_and_tags_the_colors() {
        let options = Options {
            exe: Path::new("ffmpeg.exe"),
            view: 1234,
            size: size(),
            fps: 60,
            level: 1,
            audio: true,
            target: Target::File(PathBuf::from("out.mp4")),
        };
        let gpu = video_filter(&options, &CANDIDATES[0]);
        assert!(gpu.starts_with("gfxcapture=hwnd=1234:"));
        assert!(!gpu.contains("crop_"));
        assert!(gpu.contains(",fps=60,setparams=range=tv:colorspace=bt470bg"));
        assert!(!gpu.contains("hwdownload"));
        let cpu = video_filter(&options, &CANDIDATES[5]);
        assert!(cpu.contains("hwdownload,format=bgra,scale=out_color_matrix=bt601:out_range=tv,format=yuv420p"));
        assert!(cpu.ends_with("[v]"));
    }

    #[test]
    fn better_quality_asks_for_a_lower_quantizer() {
        let normal = encoder_args(&CANDIDATES[0], 0, 8_000_000);
        let best = encoder_args(&CANDIDATES[0], 2, 8_000_000);
        assert!(normal.windows(2).any(|pair| pair[0] == "-cq" && pair[1] == "28"));
        assert!(best.windows(2).any(|pair| pair[0] == "-cq" && pair[1] == "19"));
        assert_eq!(max_rate(&size(), 60), 27_648_000);
    }

    #[test]
    fn the_duration_is_read_from_the_log() {
        let log = "Input #0, mov\n  Duration: 00:01:05.50, start: 0.000000, bitrate: 900 kb/s\n";
        assert_eq!(duration_in(log), Some(65.5));
        assert_eq!(duration_in("nothing"), None);
    }

    #[test]
    fn the_join_list_quotes_paths() {
        let list = join_list(&[PathBuf::from(r"C:\data\it's\piece001.ts")]);
        assert_eq!(list, "file 'C:/data/it'\\''s/piece001.ts'\n");
    }

    #[test]
    fn replay_keeps_enough_pieces() {
        assert_eq!(wrap(120), 15);
        let dir = std::env::temp_dir().join(format!("shiori-replay-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        for n in 0..6 {
            fs::write(dir.join(format!("piece{n:03}.ts")), b"x").unwrap();
            std::thread::sleep(Duration::from_millis(20));
        }
        fs::write(dir.join("empty.ts"), b"").unwrap();
        let picked = newest_pieces(&dir, 20);
        let names: Vec<String> = picked.iter().map(|p| p.file_name().unwrap().to_string_lossy().to_string()).collect();
        assert_eq!(names, ["piece002.ts", "piece003.ts", "piece004.ts", "piece005.ts"]);
        let _ = fs::remove_dir_all(&dir);
    }
}
