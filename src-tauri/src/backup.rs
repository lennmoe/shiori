use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use serde_json::{json, Map, Value};
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipArchive, ZipWriter};

use crate::config::{safe_filename, Config, Entry};

const FORMAT: i64 = 1;
const MANIFEST: &str = "vnrpc-backup.json";
const THIS_PC_ONLY: [&str; 5] = ["sync_token", "sync_user", "sync_auto_save", "sync_vndb_token", "profile_public"];
const CLOUD_SECRETS: [&str; 1] = ["vndb_token"];

pub fn default_name() -> String {
    chrono::Local::now()
        .format("ShioriRPC-backup-%Y-%m-%d.zip")
        .to_string()
}

pub fn backup_dir(cfg: &Config) -> PathBuf {
    cfg.dir.join("backups")
}

fn yaml(value: &Map<String, Value>) -> Result<String, String> {
    serde_yaml::to_string(value).map_err(|e| e.to_string())
}

fn local_cover(entry: &Entry) -> Option<PathBuf> {
    if entry.get("cover_source").and_then(Value::as_str) != Some("local") {
        return None;
    }
    let path = PathBuf::from(entry.get("cover_value").and_then(Value::as_str)?);
    path.is_file().then_some(path)
}

pub fn export(cfg: &Config, dest: &Path) -> Result<usize, String> {
    export_without(cfg, dest, &[])
}

pub fn export_for_cloud(cfg: &Config, dest: &Path) -> Result<usize, String> {
    if cfg.bool("sync_vndb_token") {
        export_without(cfg, dest, &[])
    } else {
        export_without(cfg, dest, &CLOUD_SECRETS)
    }
}

fn export_without(cfg: &Config, dest: &Path, secrets: &[&str]) -> Result<usize, String> {
    let part = dest.with_extension("zip.part");
    let result = write_zip(cfg, &part, secrets);

    match result {
        Ok(count) => {
            fs::rename(&part, dest).map_err(|e| e.to_string())?;
            Ok(count)
        }
        Err(e) => {
            let _ = fs::remove_file(&part);
            Err(e)
        }
    }
}

fn write_zip(cfg: &Config, path: &Path, secrets: &[&str]) -> Result<usize, String> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let file = File::create(path).map_err(|e| e.to_string())?;
    let mut zip = ZipWriter::new(file);
    let options = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);

    let add = |zip: &mut ZipWriter<File>, name: &str, bytes: &[u8]| -> Result<(), String> {
        zip.start_file(name, options).map_err(|e| e.to_string())?;
        zip.write_all(bytes).map_err(|e| e.to_string())
    };

    let mut settings = cfg.data().clone();
    for key in THIS_PC_ONLY.iter().chain(secrets) {
        settings.remove(*key);
    }
    add(&mut zip, "config.yaml", yaml(&settings)?.as_bytes())?;

    let mut covers = Map::new();
    let games = cfg.all_games();
    for (n, (key, entry)) in games.iter().enumerate() {
        let n = n + 1;
        let mut with_key = entry.clone();
        with_key.insert("_key".into(), json!(key));
        add(
            &mut zip,
            &format!("games/{n:04}.yaml"),
            yaml(&with_key)?.as_bytes(),
        )?;

        if let Some(cover) = local_cover(entry) {
            let name = cover
                .file_name()
                .map(|f| f.to_string_lossy().to_string())
                .unwrap_or_default();
            let arc = format!("covers/{n:04}_{}", safe_filename(&name));
            let bytes = fs::read(&cover).map_err(|e| e.to_string())?;
            add(&mut zip, &arc, &bytes)?;
            covers.insert(key.clone(), json!(arc));
        }
    }

    let manifest = json!({
        "format": FORMAT,
        "app_version": env!("CARGO_PKG_VERSION"),
        "exported_at": chrono::Utc::now().timestamp(),
        "games": games.len(),
        "covers": covers,
    });
    add(
        &mut zip,
        MANIFEST,
        serde_json::to_string_pretty(&manifest).unwrap().as_bytes(),
    )?;

    zip.finish().map_err(|e| e.to_string())?;
    Ok(games.len())
}

fn read_entry(zip: &mut ZipArchive<File>, name: &str) -> Result<Vec<u8>, String> {
    let mut file = zip
        .by_name(name)
        .map_err(|_| format!("{name} is missing"))?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes).map_err(|e| e.to_string())?;
    Ok(bytes)
}

fn read_yaml(zip: &mut ZipArchive<File>, name: &str) -> Result<Map<String, Value>, String> {
    let bytes = read_entry(zip, name)?;
    match serde_yaml::from_slice::<Value>(&bytes) {
        Ok(Value::Object(map)) => Ok(map),
        Ok(_) => Ok(Map::new()),
        Err(e) => Err(format!("{name} is damaged ({e})")),
    }
}

pub fn import(cfg: &mut Config, src: &Path) -> Result<usize, String> {
    import_keeping(cfg, src, &[])
}

pub fn import_from_cloud(cfg: &mut Config, src: &Path) -> Result<usize, String> {
    import_keeping(cfg, src, &CLOUD_SECRETS)
}

fn open_backup(src: &Path) -> Result<(ZipArchive<File>, Value), String> {
    let file = File::open(src).map_err(|e| e.to_string())?;
    let mut zip = ZipArchive::new(file).map_err(|_| "not a Shiori RPC backup".to_string())?;

    let manifest: Value = read_entry(&mut zip, MANIFEST)
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .ok_or("not a Shiori RPC backup")?;
    if manifest["format"].as_i64().unwrap_or(0) > FORMAT {
        return Err("this backup was made by a newer version of the app: update it first".into());
    }
    Ok((zip, manifest))
}

fn backup_games(zip: &mut ZipArchive<File>) -> Result<Vec<(String, Entry)>, String> {
    let mut games: Vec<(String, Entry)> = Vec::new();
    let names: Vec<String> = zip.file_names().map(str::to_string).collect();
    for name in names
        .iter()
        .filter(|n| n.starts_with("games/") && n.ends_with(".yaml"))
    {
        let mut entry = read_yaml(zip, name)?;
        if let Some(Value::String(key)) = entry.remove("_key") {
            if !key.is_empty() {
                games.push((key, entry));
            }
        }
    }
    Ok(games)
}

fn seconds(entry: &Entry) -> i64 {
    entry.get("playtime_seconds").and_then(Value::as_i64).unwrap_or(0)
}

fn status(entry: &Entry) -> &str {
    entry.get("status").and_then(Value::as_str).unwrap_or("")
}

fn totals<'a>(games: impl Iterator<Item = &'a Entry>) -> Value {
    let (mut vns, mut time, mut finished) = (0, 0, 0);
    for entry in games {
        vns += 1;
        time += seconds(entry);
        finished += i64::from(status(entry) == "finished");
    }
    json!({ "vns": vns, "seconds": time, "finished": finished })
}

pub fn preview(cfg: &Config, src: &Path, shown: impl Fn(&str, &Entry) -> bool) -> Result<Value, String> {
    let (mut zip, manifest) = open_backup(src)?;
    let settings = read_yaml(&mut zip, "config.yaml").map_err(|_| "the backup is missing its settings")?;
    let cloud: Vec<(String, Entry)> = backup_games(&mut zip)?
        .into_iter()
        .filter(|(key, entry)| shown(key, entry))
        .collect();
    let here: Vec<(&String, &Entry)> = cfg.all_games().iter().filter(|(key, entry)| shown(key, entry)).collect();

    let skip: Vec<&str> = THIS_PC_ONLY.iter().chain(CLOUD_SECRETS.iter()).copied().collect();
    let settings_changed = settings
        .keys()
        .chain(cfg.data().keys())
        .filter(|key| !skip.contains(&key.as_str()))
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .filter(|key| settings.get(*key) != cfg.data().get(*key))
        .count();

    let side = |entry: &Entry| json!({ "seconds": seconds(entry), "status": status(entry) });
    let cover = |entry: &Entry| cfg.cached_cover(entry).map(|p| p.to_string_lossy().to_string());
    let nsfw = |entry: &Entry| entry.get("cover_nsfw").and_then(Value::as_bool).unwrap_or(false);

    let mut items = Vec::new();
    for (key, entry) in &cloud {
        let mine = cfg.all_games().get(key).filter(|mine| shown(key, mine));
        let state = match mine {
            None => "cloud",
            Some(mine) if (seconds(mine) - seconds(entry)).abs() >= 60 || status(mine) != status(entry) => "changed",
            Some(_) => "same",
        };
        items.push(json!({
            "key": key,
            "name": crate::screenshots::display_name(key, entry),
            "cover": cover(entry).or_else(|| mine.and_then(cover)),
            "nsfw": nsfw(entry),
            "cloud": side(entry),
            "here": mine.map(side),
            "state": state,
        }));
    }
    for (key, entry) in &here {
        if cloud.iter().any(|(k, _)| k == *key) {
            continue;
        }
        items.push(json!({
            "key": key,
            "name": crate::screenshots::display_name(key, entry),
            "cover": cover(entry),
            "nsfw": nsfw(entry),
            "cloud": null,
            "here": side(entry),
            "state": "here",
        }));
    }

    Ok(json!({
        "exported_at": manifest["exported_at"],
        "app_version": manifest["app_version"],
        "cloud": totals(cloud.iter().map(|(_, e)| e)),
        "here": totals(here.iter().map(|(_, e)| *e)),
        "settings_changed": settings_changed,
        "items": items,
    }))
}

fn import_keeping(cfg: &mut Config, src: &Path, secrets: &[&str]) -> Result<usize, String> {
    let (mut zip, manifest) = open_backup(src)?;

    let mut data =
        read_yaml(&mut zip, "config.yaml").map_err(|_| "the backup is missing its settings")?;
    for key in THIS_PC_ONLY.iter().chain(secrets) {
        let brought = data.get(*key).and_then(Value::as_str).is_some_and(|v| !v.is_empty());
        if secrets.contains(key) && brought {
            continue;
        }
        match cfg.get(key) {
            Some(value) => data.insert(key.to_string(), value.clone()),
            None => data.remove(*key),
        };
    }

    let mut games = backup_games(&mut zip)?;

    let stamp = chrono::Local::now().format("before-import-%Y-%m-%d_%H-%M-%S.zip");
    export(cfg, &backup_dir(cfg).join(stamp.to_string()))?;

    let covers_dir = cfg.covers_dir().join("local");
    fs::create_dir_all(&covers_dir).map_err(|e| e.to_string())?;
    let covers = manifest["covers"].as_object().cloned().unwrap_or_default();
    for (key, entry) in games.iter_mut() {
        let Some(arc) = covers.get(key.as_str()).and_then(Value::as_str) else {
            continue;
        };
        let Ok(bytes) = read_entry(&mut zip, arc) else {
            continue;
        };
        let file_name = arc.rsplit('/').next().unwrap_or(arc);
        let dest = covers_dir.join(safe_filename(file_name));
        fs::write(&dest, bytes).map_err(|e| e.to_string())?;
        entry.insert("cover_value".into(), json!(dest.to_string_lossy()));
    }

    let count = games.len();
    cfg.replace_all(data, games).map_err(|e| e.to_string())?;
    Ok(count)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fresh(name: &str) -> Config {
        let dir = std::env::temp_dir().join(format!("shiori-backup-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        Config::load(&dir)
    }

    fn settings_in(zip: &Path) -> String {
        let mut archive = ZipArchive::new(File::open(zip).unwrap()).unwrap();
        String::from_utf8(read_entry(&mut archive, "config.yaml").unwrap()).unwrap()
    }

    fn set(cfg: &mut Config, pairs: &[(&str, Value)]) {
        cfg.update(pairs.iter().map(|(k, v)| (k.to_string(), v.clone())).collect()).unwrap();
    }

    #[test]
    fn the_vndb_token_goes_to_the_cloud_only_when_asked() {
        let mut cfg = fresh("cloud");
        set(&mut cfg, &[("vndb_token", json!("tok-123")), ("sync_token", json!("sess"))]);
        let zip = cfg.dir.join("cloud.zip");

        export_for_cloud(&cfg, &zip).unwrap();
        let plain = settings_in(&zip);
        assert!(!plain.contains("tok-123") && !plain.contains("sess"));

        set(&mut cfg, &[("sync_vndb_token", json!(true))]);
        export_for_cloud(&cfg, &zip).unwrap();
        let with_token = settings_in(&zip);
        assert!(with_token.contains("tok-123") && !with_token.contains("sess"));

        export(&cfg, &zip).unwrap();
        assert!(!settings_in(&zip).contains("sess"));
    }

    #[test]
    fn loading_from_the_cloud_keeps_or_brings_the_vndb_token() {
        let mut cfg = fresh("restore");
        set(&mut cfg, &[("vndb_token", json!("from-cloud")), ("sync_vndb_token", json!(true))]);
        let with_token = cfg.dir.join("with.zip");
        export_for_cloud(&cfg, &with_token).unwrap();
        set(&mut cfg, &[("sync_vndb_token", json!(false))]);
        let without = cfg.dir.join("without.zip");
        export_for_cloud(&cfg, &without).unwrap();

        set(&mut cfg, &[("vndb_token", json!("local")), ("sync_token", json!("sess"))]);
        import_from_cloud(&mut cfg, &without).unwrap();
        assert_eq!(cfg.str("vndb_token"), "local");
        assert_eq!(cfg.str("sync_token"), "sess");

        import_from_cloud(&mut cfg, &with_token).unwrap();
        assert_eq!(cfg.str("vndb_token"), "from-cloud");
        assert_eq!(cfg.str("sync_token"), "sess");
    }

    #[test]
    fn the_cloud_preview_shows_what_loading_changes() {
        let mut cfg = fresh("preview");
        let game = |seconds: i64, status: &str| {
            Map::from_iter([("playtime_seconds".to_string(), json!(seconds)), ("status".to_string(), json!(status))])
        };
        cfg.set_game("same.exe", game(3600, "playing"));
        cfg.set_game("more.exe", game(600, "playing"));
        cfg.set_game("gone.exe", game(1200, "finished"));
        let zip = cfg.dir.join("preview.zip");
        export_for_cloud(&cfg, &zip).unwrap();

        cfg.set_game("more.exe", game(7200, "finished"));
        cfg.set_game("new.exe", game(300, ""));
        cfg.remove_game("gone.exe");

        let preview = preview(&cfg, &zip, |_, _| true).unwrap();
        let state = |key: &str| {
            preview["items"].as_array().unwrap().iter().find(|i| i["key"] == key).unwrap()["state"].clone()
        };
        assert_eq!(state("same.exe"), "same");
        assert_eq!(state("more.exe"), "changed");
        assert_eq!(state("gone.exe"), "cloud");
        assert_eq!(state("new.exe"), "here");
        assert_eq!(preview["cloud"]["vns"], 3);
        assert_eq!(preview["here"]["seconds"], 3600 + 7200 + 300);
    }
}
