use std::collections::VecDeque;
use std::io::{BufRead, BufReader};
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::Deserialize;
use serde_json::json;

use crate::recorder;
use crate::state::Shared;

const NO_WINDOW: u32 = 0x0800_0000;
const MAX_ANIMATED_SECONDS: f64 = 60.0;
const LOG_LINES: usize = 12;

static BUSY: AtomicBool = AtomicBool::new(false);
static CANCEL: AtomicBool = AtomicBool::new(false);

#[derive(Deserialize, Clone, Copy)]
pub struct Crop {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

#[derive(Deserialize)]
pub struct Edit {
    pub start: f64,
    pub end: f64,
    pub crop: Option<Crop>,
    pub format: String,
    pub width: u32,
    pub fps: u32,
    pub audio: bool,
}

#[derive(Clone, Copy, PartialEq)]
enum Format {
    Mp4,
    Gif,
    Webp,
}

impl Format {
    fn parse(name: &str) -> Result<Self, String> {
        match name {
            "mp4" => Ok(Self::Mp4),
            "gif" => Ok(Self::Gif),
            "webp" => Ok(Self::Webp),
            _ => Err(format!("unknown format {name}")),
        }
    }

    fn ext(self) -> &'static str {
        match self {
            Self::Mp4 => "mp4",
            Self::Gif => "gif",
            Self::Webp => "webp",
        }
    }
}

pub fn cancel() {
    CANCEL.store(true, Ordering::SeqCst);
}

fn even(n: u32) -> u32 {
    n & !1
}

fn output_for(input: &Path, format: Format) -> PathBuf {
    let folder = input.parent().map(Path::to_path_buf).unwrap_or_default();
    let stem = input.file_stem().and_then(|s| s.to_str()).unwrap_or("clip");
    let stem = stem.split("-edit").next().unwrap_or(stem);
    (1..)
        .map(|n| folder.join(format!("{stem}-edit{n}.{}", format.ext())))
        .find(|path| !path.exists())
        .unwrap()
}

fn filters(edit: &Edit, format: Format) -> Vec<String> {
    let mut chain = Vec::new();
    if let Some(crop) = edit.crop {
        let (width, height) = match format {
            Format::Mp4 => (even(crop.width), even(crop.height)),
            _ => (crop.width, crop.height),
        };
        chain.push(format!("crop={width}:{height}:{}:{}", crop.x, crop.y));
    }
    if format != Format::Mp4 {
        chain.push(format!("fps={}", edit.fps.clamp(5, 50)));
    }
    if edit.width > 0 {
        let height = if format == Format::Mp4 { -2 } else { -1 };
        chain.push(format!("scale='min({},iw)':{height}:flags=lanczos", edit.width.min(3840)));
    } else if format == Format::Mp4 {
        chain.push("scale=trunc(iw/2)*2:trunc(ih/2)*2".into());
    }
    chain
}

fn arguments(input: &Path, output: &Path, edit: &Edit, format: Format) -> Vec<String> {
    let length = edit.end - edit.start;
    let mut args: Vec<String> = vec![
        "-hide_banner".into(),
        "-y".into(),
        "-ss".into(),
        format!("{:.3}", edit.start),
        "-i".into(),
        input.to_string_lossy().into_owned(),
        "-t".into(),
        format!("{length:.3}"),
    ];
    let chain = filters(edit, format).join(",");
    match format {
        Format::Mp4 => {
            if !chain.is_empty() {
                args.extend(["-vf".into(), chain]);
            }
            args.extend(
                ["-c:v", "libx264", "-preset", "veryfast", "-crf", "20", "-pix_fmt", "yuv420p", "-movflags", "+faststart"]
                    .map(String::from),
            );
            if edit.audio {
                args.extend(["-c:a", "aac", "-b:a", "160k"].map(String::from));
            } else {
                args.push("-an".into());
            }
        }
        Format::Gif => {
            let graph = format!(
                "[0:v]{chain},split[a][b];[a]palettegen=max_colors=256:stats_mode=diff[p];[b][p]paletteuse=dither=bayer:bayer_scale=4:diff_mode=rectangle"
            );
            args.extend(["-filter_complex".into(), graph, "-loop".into(), "0".into(), "-an".into()]);
        }
        Format::Webp => {
            args.extend(["-vf".into(), chain]);
            args.extend(["-c:v", "libwebp_anim", "-quality", "75", "-compression_level", "4", "-loop", "0", "-an"].map(String::from));
        }
    }
    args.extend(["-progress", "pipe:1", "-nostats"].map(String::from));
    args.push(output.to_string_lossy().into_owned());
    args
}

fn check(edit: &Edit, format: Format) -> Result<(), String> {
    if !edit.start.is_finite() || !edit.end.is_finite() || edit.start < 0.0 || edit.end - edit.start < 0.2 {
        return Err("the clip is too short".into());
    }
    if format != Format::Mp4 && edit.end - edit.start > MAX_ANIMATED_SECONDS {
        return Err("GIF and WebP can be 60 seconds at most".into());
    }
    if let Some(crop) = edit.crop {
        if crop.width < 16 || crop.height < 16 {
            return Err("the crop is too small".into());
        }
    }
    Ok(())
}

pub fn export(shared: &Arc<Shared>, input: &Path, edit: &Edit) -> Result<PathBuf, String> {
    let format = Format::parse(&edit.format)?;
    check(edit, format)?;
    let exe = recorder::exe(shared).ok_or("the clip editor needs the HD recorder: download it in Settings → Screenshots")?;
    if !input.is_file() {
        return Err("that clip doesn't exist anymore".into());
    }
    if BUSY.swap(true, Ordering::SeqCst) {
        return Err("a clip is already being exported".into());
    }
    CANCEL.store(false, Ordering::SeqCst);
    let output = output_for(input, format);
    let result = run(shared, &exe, input, &output, edit, format);
    BUSY.store(false, Ordering::SeqCst);
    if result.is_err() {
        let _ = std::fs::remove_file(&output);
    }
    result.map(|()| output)
}

fn run(shared: &Arc<Shared>, exe: &Path, input: &Path, output: &Path, edit: &Edit, format: Format) -> Result<(), String> {
    let mut child = Command::new(exe)
        .args(arguments(input, output, edit, format))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .creation_flags(NO_WINDOW)
        .spawn()
        .map_err(|e| format!("couldn't start ffmpeg: {e}"))?;

    let length = (edit.end - edit.start).max(0.001);
    let stdout = child.stdout.take();
    let progress_shared = Arc::clone(shared);
    let reader = std::thread::spawn(move || {
        let Some(stdout) = stdout else { return };
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            if let Some(micros) = line.strip_prefix("out_time_us=").and_then(|v| v.trim().parse::<f64>().ok()) {
                let percent = (micros / 1_000_000.0 / length * 100.0).clamp(0.0, 99.0);
                progress_shared.emit("clip-export-progress", json!({ "percent": percent.round() }));
            }
        }
    });
    let log = Arc::new(Mutex::new(VecDeque::new()));
    let stderr = child.stderr.take();
    let log_writer = Arc::clone(&log);
    let errors = std::thread::spawn(move || {
        let Some(stderr) = stderr else { return };
        for line in BufReader::new(stderr).lines().map_while(Result::ok) {
            let mut log = log_writer.lock().unwrap();
            log.push_back(line);
            if log.len() > LOG_LINES {
                log.pop_front();
            }
        }
    });

    let status = loop {
        if CANCEL.load(Ordering::SeqCst) {
            let _ = child.kill();
            let _ = child.wait();
            let _ = reader.join();
            let _ = errors.join();
            return Err("export cancelled".into());
        }
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => std::thread::sleep(Duration::from_millis(100)),
            Err(e) => return Err(e.to_string()),
        }
    };
    let _ = reader.join();
    let _ = errors.join();
    if !status.success() {
        let last = log.lock().unwrap().iter().rev().find(|line| !line.trim().is_empty()).cloned().unwrap_or_default();
        return Err(format!("ffmpeg failed: {last}"));
    }
    shared.emit("clip-export-progress", json!({ "percent": 100 }));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn edit(format: &str, crop: Option<Crop>, width: u32) -> Edit {
        Edit { start: 1.0, end: 4.5, crop, format: format.into(), width, fps: 15, audio: true }
    }

    #[test]
    fn mp4_crops_to_even_sizes() {
        let crop = Some(Crop { x: 10, y: 20, width: 301, height: 199 });
        let chain = filters(&edit("mp4", crop, 0), Format::Mp4);
        assert_eq!(chain[0], "crop=300:198:10:20");
        assert_eq!(chain[1], "scale=trunc(iw/2)*2:trunc(ih/2)*2");
    }

    #[test]
    fn gif_uses_a_palette_and_fps() {
        let args = arguments(Path::new("a.mp4"), Path::new("a-edit1.gif"), &edit("gif", None, 480), Format::Gif);
        let graph = &args[args.iter().position(|a| a == "-filter_complex").unwrap() + 1];
        assert!(graph.starts_with("[0:v]fps=15,scale='min(480,iw)':-1:flags=lanczos,split"));
        assert!(graph.contains("paletteuse"));
        assert!(args.contains(&"-an".to_string()));
    }

    #[test]
    fn long_animations_are_refused() {
        let mut long = edit("webp", None, 0);
        long.end = 90.0;
        assert!(check(&long, Format::Webp).is_err());
        assert!(check(&edit("webp", None, 0), Format::Webp).is_ok());
        let mut short = edit("mp4", None, 0);
        short.end = 1.1;
        assert!(check(&short, Format::Mp4).is_err());
    }
}
