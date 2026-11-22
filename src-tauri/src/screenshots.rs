use std::fs::{self, File};
use std::io::{BufReader, BufWriter};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use image::RgbImage;
use serde::Serialize;
use serde_json::{json, Map, Value};
use tauri::Manager;

use crate::capture;
use crate::config::Config;
use crate::mascot;
use crate::shell;
use crate::sound;
use crate::state::Shared;
use crate::ui_windows::{self, Toast};

const IMAGE_EXTS: [&str; 6] = ["png", "jpg", "jpeg", "webp", "bmp", "mp4"];
const RESERVED: [&str; 4] = ["CON", "PRN", "AUX", "NUL"];

pub fn default_root() -> PathBuf {
    let pictures = shell::pictures_folder();
    let root = pictures.join("Shiori RPC");
    let before_rename = pictures.join("Visual Novel RPC");
    if !root.exists() && before_rename.exists() {
        before_rename
    } else {
        root
    }
}

pub fn root(cfg: &Config) -> PathBuf {
    let custom = cfg.str("screenshot_dir");
    if custom.trim().is_empty() {
        default_root()
    } else {
        PathBuf::from(custom.trim())
    }
}

pub fn folder_name(title: &str, taken: &[String]) -> String {
    let cleaned: String = title
        .chars()
        .filter(|c| !"<>:\"/\\|?*".contains(*c) && (*c as u32) >= 0x20)
        .collect();
    let cleaned = cleaned.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut name: String = cleaned.chars().take(80).collect();
    name = name.trim_end_matches(['.', ' ']).to_string();

    let stem = name.split('.').next().unwrap_or("").to_uppercase();
    let numbered_device = (stem.starts_with("COM") || stem.starts_with("LPT"))
        && stem.len() == 4
        && stem.ends_with(|c: char| ('1'..='9').contains(&c));
    let reserved = RESERVED.contains(&stem.as_str()) || numbered_device;
    if name.is_empty() {
        name = "Game".into();
    } else if reserved {
        name.push('_');
    }

    let taken: Vec<String> = taken.iter().map(|t| t.to_lowercase()).collect();
    let mut candidate = name.clone();
    let mut n = 2;
    while taken.contains(&candidate.to_lowercase()) {
        candidate = format!("{name} ({n})");
        n += 1;
    }
    candidate
}

pub fn own_folder(entry: &Map<String, Value>) -> Option<PathBuf> {
    absolute(&text(entry, "screenshot_path"))
}

pub fn collection_list(cfg: &Config) -> Vec<Map<String, Value>> {
    cfg.get("collections")
        .and_then(Value::as_array)
        .map(|list| {
            list.iter()
                .filter_map(Value::as_object)
                .filter(|c| !text(c, "id").is_empty())
                .cloned()
                .collect()
        })
        .unwrap_or_default()
}

pub fn collections_of(entry: &Map<String, Value>) -> Vec<String> {
    entry
        .get("collections")
        .and_then(Value::as_array)
        .map(|ids| ids.iter().filter_map(Value::as_str).map(str::to_string).collect())
        .unwrap_or_default()
}

fn absolute(path: &str) -> Option<PathBuf> {
    let path = path.trim();
    (!path.is_empty() && Path::new(path).is_absolute()).then(|| PathBuf::from(path))
}

fn collection_folder(cfg: &Config, entry: &Map<String, Value>) -> Option<PathBuf> {
    let mine = collections_of(entry);
    collection_list(cfg)
        .iter()
        .filter(|c| mine.contains(&text(c, "id")))
        .find_map(|c| absolute(&text(c, "folder")))
}

pub fn other_roots(cfg: &Config) -> Vec<PathBuf> {
    let mut roots: Vec<PathBuf> = collection_list(cfg)
        .iter()
        .filter_map(|c| absolute(&text(c, "folder")))
        .chain(cfg.all_games().values().filter_map(own_folder))
        .collect();
    roots.sort();
    roots.dedup();
    roots
}

pub fn folder_for(cfg: &mut Config, key: &str, create: bool) -> Option<PathBuf> {
    let entry = cfg.game(key);
    if let Some(own) = own_folder(&entry) {
        if !create || fs::create_dir_all(&own).is_ok() {
            return Some(own);
        }
    }
    let mut name = text(&entry, "screenshot_folder");

    if name.is_empty() {
        if !create {
            return None;
        }
        let taken: Vec<String> = cfg
            .all_games()
            .iter()
            .filter(|(k, _)| k.as_str() != key)
            .map(|(_, e)| text(e, "screenshot_folder"))
            .collect();
        let title = [text(&entry, "title"), text(&entry, "name"), key.to_string()]
            .into_iter()
            .find(|t| !t.is_empty())
            .unwrap_or_default();

        name = folder_name(&title, &taken);
        cfg.set_game(
            key,
            Map::from_iter([("screenshot_folder".into(), json!(name))]),
        );
    }

    let base = collection_folder(cfg, &entry).unwrap_or_else(|| root(cfg));
    let mut path = base.join(&name);
    if create && fs::create_dir_all(&path).is_err() {
        path = root(cfg).join(&name);
        let _ = fs::create_dir_all(&path);
    }
    Some(path)
}

fn text(entry: &Map<String, Value>, field: &str) -> String {
    entry
        .get(field)
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string()
}

fn modified(path: &Path) -> std::time::SystemTime {
    fs::metadata(path)
        .and_then(|m| m.modified())
        .unwrap_or(std::time::UNIX_EPOCH)
}

fn shots_by_date(folder: Option<&Path>) -> Vec<(PathBuf, std::time::SystemTime)> {
    let Some(folder) = folder else {
        return vec![];
    };

    let mut shots: Vec<(PathBuf, std::time::SystemTime)> = fs::read_dir(folder)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|entry| {
            let path = entry.path();
            let ext = path.extension().map(|e| e.to_string_lossy().to_lowercase());
            ext.is_some_and(|ext| IMAGE_EXTS.contains(&ext.as_str()))
        })
        .filter_map(|entry| {
            let meta = entry.metadata().ok().filter(|meta| meta.is_file())?;
            Some((entry.path(), meta.modified().unwrap_or(std::time::UNIX_EPOCH)))
        })
        .collect();

    shots.sort_by_key(|(_, time)| std::cmp::Reverse(*time));
    shots
}

pub fn list_shots(folder: Option<&Path>) -> Vec<PathBuf> {
    shots_by_date(folder).into_iter().map(|(path, _)| path).collect()
}

pub fn move_shots(from: &Path, to: &Path) -> Result<(), String> {
    if from == to || !from.is_dir() {
        return Ok(());
    }
    fs::create_dir_all(to).map_err(|e| e.to_string())?;
    for shot in list_shots(Some(from)) {
        let stem = shot.file_stem().unwrap_or_default().to_string_lossy().to_string();
        let ext = shot.extension().unwrap_or_default().to_string_lossy().to_string();
        let mut dest = to.join(format!("{stem}.{ext}"));
        let mut n = 2;
        while dest.exists() {
            dest = to.join(format!("{stem} ({n}).{ext}"));
            n += 1;
        }
        if fs::rename(&shot, &dest).is_err() {
            fs::copy(&shot, &dest).map_err(|e| e.to_string())?;
            fs::remove_file(&shot).map_err(|e| e.to_string())?;
        }
    }
    let _ = fs::remove_dir(from);
    Ok(())
}

#[derive(Serialize)]
pub struct GameShots {
    pub key: String,
    pub name: String,
    pub shots: Vec<String>,
    pub times: Vec<u64>,
}

pub fn games_with_shots(cfg: &mut Config) -> Vec<GameShots> {
    let games: Vec<(String, String)> = cfg
        .all_games()
        .iter()
        .map(|(key, e)| (key.clone(), display_name(key, e)))
        .collect();

    let mut out = Vec::new();
    for (key, name) in games {
        let folder = folder_for(cfg, &key, false);
        let found = shots_by_date(folder.as_deref());
        if !found.is_empty() {
            let times = found
                .iter()
                .map(|(_, time)| {
                    time.duration_since(std::time::UNIX_EPOCH)
                        .map_or(0, |d| d.as_secs())
                })
                .collect();
            let shots = found
                .iter()
                .map(|(path, _)| path.to_string_lossy().to_string())
                .collect();
            out.push(GameShots { key, name, shots, times });
        }
    }

    out.sort_by_key(|game: &GameShots| std::cmp::Reverse(game.times[0]));
    out
}

pub fn display_name(key: &str, entry: &Map<String, Value>) -> String {
    if let Some(title) = crate::config::selected_release_title(entry) {
        return title.to_string();
    }
    [text(entry, "title"), text(entry, "name")]
        .into_iter()
        .find(|t| !t.is_empty())
        .unwrap_or_else(|| key.to_string())
}

pub fn save(img: &RgbImage, folder: &Path, game: &str, section: &str) -> Result<PathBuf, String> {
    let now = chrono::Local::now();
    fs::create_dir_all(folder).map_err(|e| e.to_string())?;

    let stem = now.format("%Y-%m-%d %H-%M-%S").to_string();
    let mut path = folder.join(format!("{stem}.png"));
    let mut n = 2;
    while path.exists() {
        path = folder.join(format!("{stem} ({n}).png"));
        n += 1;
    }

    let part = path.with_extension("png.part");
    write_png(
        img,
        &part,
        &now.format("%Y-%m-%dT%H:%M:%S").to_string(),
        game,
        section,
    )
    .map_err(|e| e.to_string())?;
    fs::rename(&part, &path).map_err(|e| e.to_string())?;
    Ok(path)
}

fn write_png(
    img: &RgbImage,
    path: &Path,
    taken: &str,
    game: &str,
    section: &str,
) -> Result<(), png::EncodingError> {
    let file = BufWriter::new(File::create(path)?);
    let mut encoder = png::Encoder::new(file, img.width(), img.height());
    encoder.set_color(png::ColorType::Rgb);
    encoder.set_depth(png::BitDepth::Eight);

    encoder.add_text_chunk("Creation Time".into(), taken.into())?;
    encoder.add_text_chunk("Software".into(), "Shiori RPC".into())?;
    if !game.is_empty() {
        encoder.add_itxt_chunk("Title".into(), game.into())?;
    }
    if !section.is_empty() {
        encoder.add_itxt_chunk("Section".into(), section.into())?;
    }

    let mut writer = encoder.write_header()?;
    writer.write_image_data(img.as_raw())?;
    writer.finish()
}

#[derive(Serialize, Default)]
pub struct ShotInfo {
    pub taken: String,
    pub width: u32,
    pub height: u32,
    pub section: String,
    pub file_name: String,
}

pub fn info(path: &Path) -> ShotInfo {
    let taken: chrono::DateTime<chrono::Local> = modified(path).into();
    let mut info = ShotInfo {
        taken: taken.format("%Y-%m-%dT%H:%M:%S").to_string(),
        file_name: path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default(),
        ..Default::default()
    };

    if let Ok((w, h)) = image::image_dimensions(path) {
        info.width = w;
        info.height = h;
    }

    let Ok(file) = File::open(path) else {
        return info;
    };
    let Ok(reader) = png::Decoder::new(BufReader::new(file)).read_info() else {
        return info;
    };

    let png_info = reader.info();
    for chunk in &png_info.uncompressed_latin1_text {
        if chunk.keyword == "Creation Time" {
            info.taken = chunk.text.clone();
        }
    }
    for chunk in &png_info.utf8_text {
        if chunk.keyword == "Section" {
            info.section = chunk.get_text().unwrap_or_default();
        }
    }
    info
}

pub fn take(shared: &Arc<Shared>) {
    crate::crash::note("screenshot");
    match capture_and_save(shared) {
        Ok((path, key, area)) => {
            shared.emit("screenshots-changed", json!({ "key": key }));
            ui_windows::show_toast(
                &shared.app,
                Toast {
                    title: "Screenshot saved".into(),
                    detail: path
                        .file_name()
                        .map(|n| n.to_string_lossy().to_string())
                        .unwrap_or_default(),
                    image: Some(path.to_string_lossy().to_string()),
                    ok: true,
                    ..Default::default()
                },
                Some(area),
            );
            mascot::screenshot_saved(&shared.app);
        }
        Err(message) => {
            let toast = Toast {
                title: "No screenshot".into(),
                detail: message,
                image: None,
                ok: false,
                ..Default::default()
            };
            ui_windows::show_toast(&shared.app, toast, None);
        }
    }
}

fn capture_and_save(
    shared: &Arc<Shared>,
) -> Result<(PathBuf, String, capture::ScreenRect), String> {
    let snap = shared.snapshot();
    if !snap.detected || snap.hwnd == 0 {
        return Err("No visual novel is running.".into());
    }

    let window = crate::winapi::content_window(snap.hwnd);
    let img = capture::capture_window(window)?;
    let area = capture::client_rect_on_screen(window);

    let (folder, volume) = {
        let mut cfg = shared.config.lock().unwrap();
        let folder = folder_for(&mut cfg, &snap.key, true).ok_or("no screenshot folder")?;
        let volume = cfg
            .get("screenshot_volume")
            .and_then(Value::as_i64)
            .unwrap_or(30);
        (folder, volume)
    };

    sound::play_shutter(&shared.data_dir.join("cache"), volume);

    let path = save(&img, &folder, &snap.game_name, &snap.section_label)?;
    let _ = shared
        .app
        .asset_protocol_scope()
        .allow_directory(&folder, false);
    Ok((path, snap.key, area))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_name_uses_selected_release_and_falls_back_when_cleared() {
        let mut entry = Map::from_iter([
            ("title".into(), json!("VN title")),
            ("vndb_release_id".into(), json!("r1")),
            ("vndb_release_title".into(), json!("Release title")),
        ]);
        assert_eq!(display_name("game", &entry), "Release title");
        entry.insert("vndb_release_id".into(), json!(""));
        assert_eq!(display_name("game", &entry), "VN title");
        entry.insert("vndb_release_id".into(), json!("r1"));
        entry.insert("vndb_release_title".into(), json!("  "));
        assert_eq!(display_name("game", &entry), "VN title");
        entry.remove("title");
        entry.insert("name".into(), json!("Detected name"));
        assert_eq!(display_name("game", &entry), "Detected name");
    }

    #[test]
    fn folder_names_are_windows_safe() {
        assert_eq!(folder_name("Fate/stay night", &[]), "Fatestay night");
        assert_eq!(folder_name("CON", &[]), "CON_");
        assert_eq!(folder_name("", &[]), "Game");
        assert_eq!(
            folder_name("Amatsutsumi", &["amatsutsumi".into()]),
            "Amatsutsumi (2)"
        );
        assert_eq!(folder_name("Dots...", &[]), "Dots");
    }

    #[test]
    fn the_game_folder_beats_collections_and_collections_beat_the_root() {
        let base = std::env::temp_dir().join(format!("shiori-shots-{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        let mut cfg = Config::load(&base.join("data"));
        let root = base.join("root");
        let yuzu = base.join("yuzu");
        let mine = base.join("mine");
        cfg.update(Map::from_iter([
            ("screenshot_dir".into(), json!(root.to_string_lossy())),
            (
                "collections".into(),
                json!([
                    {"id": "a", "name": "Long story", "folder": ""},
                    {"id": "b", "name": "Yuzusoft", "folder": yuzu.to_string_lossy()},
                    {"id": "c", "name": "Elsewhere", "folder": "relative/path"},
                ]),
            ),
        ]))
        .unwrap();
        cfg.set_game("senren", Map::from_iter([("title".into(), json!("Senren Banka"))]));

        assert_eq!(folder_for(&mut cfg, "senren", true), Some(root.join("Senren Banka")));

        cfg.set_game("senren", Map::from_iter([("collections".into(), json!(["a", "c", "b"]))]));
        assert_eq!(folder_for(&mut cfg, "senren", false), Some(yuzu.join("Senren Banka")));

        cfg.set_game("senren", Map::from_iter([("screenshot_path".into(), json!(mine.to_string_lossy()))]));
        assert_eq!(folder_for(&mut cfg, "senren", true), Some(mine.clone()));
        assert!(mine.is_dir());
        assert_eq!(other_roots(&cfg), {
            let mut both = vec![mine.clone(), yuzu.clone()];
            both.sort();
            both
        });

        cfg.set_game("senren", Map::from_iter([("screenshot_path".into(), json!(""))]));
        assert_eq!(folder_for(&mut cfg, "senren", false), Some(yuzu.join("Senren Banka")));
        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn moving_shots_keeps_every_file() {
        let base = std::env::temp_dir().join(format!("shiori-move-{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        let (from, to) = (base.join("from"), base.join("to"));
        fs::create_dir_all(&from).unwrap();
        fs::create_dir_all(&to).unwrap();
        fs::write(from.join("a.png"), b"new").unwrap();
        fs::write(to.join("a.png"), b"old").unwrap();
        move_shots(&from, &to).unwrap();
        assert!(!from.exists());
        assert_eq!(fs::read(to.join("a.png")).unwrap(), b"old");
        assert_eq!(fs::read(to.join("a (2).png")).unwrap(), b"new");
        let _ = fs::remove_dir_all(&base);
    }
}
