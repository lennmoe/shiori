use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

pub const REPO: &str = "lennmoe/vnrpc";
pub const CURRENT: &str = env!("CARGO_PKG_VERSION");

fn http() -> reqwest::blocking::Client {
    reqwest::blocking::Client::builder()
        .user_agent(format!("VisualNovelRPC/{CURRENT}"))
        .timeout(Duration::from_secs(10))
        .build()
        .expect("http client")
}

fn get_json(url: &str) -> Result<Value, String> {
    http()
        .get(url)
        .header("Accept", "application/vnd.github+json")
        .send()
        .and_then(|r| r.error_for_status())
        .and_then(|r| r.json())
        .map_err(|e| e.to_string())
}

pub fn parse_version(tag: &str) -> Vec<u64> {
    let digits = tag.trim().trim_start_matches(['v', 'V']);
    let end = digits
        .find(|c: char| !(c.is_ascii_digit() || c == '.'))
        .unwrap_or(digits.len());
    let mut parts = Vec::new();
    for piece in digits[..end].split('.') {
        match piece.parse() {
            Ok(n) => parts.push(n),
            Err(_) => break,
        }
    }
    parts
}

pub fn is_newer(tag: &str, current: &str) -> bool {
    let mut new = parse_version(tag);
    let mut cur = parse_version(current);
    if new.is_empty() {
        return false;
    }
    let width = new.len().max(cur.len());
    new.resize(width, 0);
    cur.resize(width, 0);
    new > cur
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Release {
    pub version: String,
    pub url: String,
    pub size: u64,
    pub sha256: String,
    pub notes: String,
    pub page: String,
}

pub fn find_update() -> Result<Option<Release>, String> {
    let data = get_json(&format!(
        "https://api.github.com/repos/{REPO}/releases/latest"
    ))?;

    let tag = data["tag_name"].as_str().unwrap_or("");
    let draft = data["draft"].as_bool().unwrap_or(false);
    let prerelease = data["prerelease"].as_bool().unwrap_or(false);
    if draft || prerelease || !is_newer(tag, CURRENT) {
        return Ok(None);
    }

    let assets = data["assets"].as_array().cloned().unwrap_or_default();
    let names: Vec<&str> = assets
        .iter()
        .map(|asset| asset["name"].as_str().unwrap_or(""))
        .collect();
    let Some(asset) = pick_asset(&names).map(|i| &assets[i]) else {
        return Ok(None);
    };

    let digest = asset["digest"].as_str().unwrap_or("");
    Ok(Some(Release {
        version: tag.trim_start_matches(['v', 'V']).to_string(),
        url: asset["browser_download_url"]
            .as_str()
            .unwrap_or("")
            .to_string(),
        size: asset["size"].as_u64().unwrap_or(0),
        sha256: digest.strip_prefix("sha256:").unwrap_or("").to_lowercase(),
        notes: data["body"].as_str().unwrap_or("").trim().to_string(),
        page: data["html_url"].as_str().unwrap_or("").to_string(),
    }))
}

fn pick_asset(names: &[&str]) -> Option<usize> {
    let lower: Vec<String> = names.iter().map(|name| name.to_lowercase()).collect();
    lower
        .iter()
        .position(|name| name == "vnrpc.exe")
        .or_else(|| {
            lower
                .iter()
                .position(|name| name.ends_with(".exe") && !name.contains("setup"))
        })
}

fn base_stem(stem: &str) -> &str {
    let mut base = stem;
    while let Some(rest) = base
        .strip_suffix(".new")
        .or_else(|| base.strip_suffix(".old"))
    {
        base = rest;
    }
    base
}

fn stem_of(path: &Path) -> String {
    path.file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default()
}

pub fn staging_path(target: &Path) -> PathBuf {
    target.with_file_name(format!("{}.new.exe", stem_of(target)))
}

fn retired_path(target: &Path) -> PathBuf {
    target.with_file_name(format!("{}.old.exe", stem_of(target)))
}

fn swap_in(new_exe: &Path, target: &Path) -> Result<(), String> {
    let retired = retired_path(target);
    let _ = fs::remove_file(&retired);
    if target.exists() {
        fs::rename(target, &retired).map_err(|e| e.to_string())?;
    }
    if let Err(e) = fs::rename(new_exe, target) {
        let _ = fs::rename(&retired, target);
        return Err(e.to_string());
    }
    Ok(())
}

fn is_leftover(name: &str, base: &str) -> bool {
    let name = name.to_lowercase();
    let Some(rest) = name.strip_prefix(&format!("{}.", base.to_lowercase())) else {
        return false;
    };
    let Some(marks) = rest
        .strip_suffix(".exe.part")
        .or_else(|| rest.strip_suffix(".exe"))
    else {
        return false;
    };
    marks.split('.').all(|mark| mark == "new" || mark == "old")
}

pub fn clean_leftovers() {
    let Ok(exe) = std::env::current_exe() else {
        return;
    };
    let base = stem_of(&exe);
    let Some(dir) = exe.parent().map(Path::to_path_buf) else {
        return;
    };
    std::thread::spawn(move || {
        for _ in 0..5 {
            let mut left = false;
            for file in fs::read_dir(&dir).into_iter().flatten().flatten() {
                if is_leftover(&file.file_name().to_string_lossy(), &base) {
                    left |= fs::remove_file(file.path()).is_err();
                }
            }
            if !left {
                return;
            }
            std::thread::sleep(Duration::from_secs(3));
        }
    });
}

const AFTER_UPDATE: &str = "--after-update";

pub fn wait_for_previous() {
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::System::Threading::{
        OpenProcess, WaitForSingleObject, PROCESS_SYNCHRONIZE,
    };

    let args: Vec<String> = std::env::args().collect();
    let pid = args
        .iter()
        .position(|arg| arg == AFTER_UPDATE)
        .and_then(|i| args.get(i + 1))
        .and_then(|pid| pid.parse::<u32>().ok());
    let Some(pid) = pid else {
        return;
    };
    unsafe {
        if let Ok(process) = OpenProcess(PROCESS_SYNCHRONIZE, false, pid) {
            WaitForSingleObject(process, 20_000);
            let _ = CloseHandle(process);
        }
    }
}

pub fn heal_name() -> bool {
    let Ok(exe) = std::env::current_exe() else {
        return false;
    };
    let stem = stem_of(&exe);
    let base = base_stem(&stem);
    if base == stem || base.is_empty() {
        return false;
    }
    let target = exe.with_file_name(format!("{base}.exe"));
    if swap_in(&exe, &target).is_err() {
        return false;
    }
    let args = std::env::args().skip(1);
    std::process::Command::new(&target)
        .args(args)
        .spawn()
        .is_ok()
}

pub fn sync_installed_version() {
    use winreg::enums::{HKEY_CURRENT_USER, KEY_READ, KEY_SET_VALUE};

    let Ok(key) = winreg::RegKey::predef(HKEY_CURRENT_USER).open_subkey_with_flags(
        r"Software\Microsoft\Windows\CurrentVersion\Uninstall\VisualNovelRPC",
        KEY_READ | KEY_SET_VALUE,
    ) else {
        return;
    };
    let folder: String = key.get_value("InstallLocation").unwrap_or_default();
    let folder = PathBuf::from(folder.trim_matches('"'));
    let installed = std::env::current_exe().is_ok_and(|exe| exe.parent() == Some(folder.as_path()));
    if installed {
        let _ = key.set_value("DisplayVersion", &CURRENT);
    }
}

pub fn download(release: &Release, dest: &Path) -> Result<PathBuf, String> {
    let bytes = reqwest::blocking::Client::builder()
        .user_agent(format!("VisualNovelRPC/{CURRENT}"))
        .timeout(Duration::from_secs(120))
        .build()
        .map_err(|e| e.to_string())?
        .get(&release.url)
        .send()
        .and_then(|r| r.error_for_status())
        .and_then(|r| r.bytes())
        .map_err(|e| e.to_string())?;

    let wrong_size = release.size > 0 && bytes.len() as u64 != release.size;
    let hash: String = Sha256::digest(&bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    let wrong_hash = !release.sha256.is_empty() && hash != release.sha256;
    if wrong_size || wrong_hash {
        return Err("the downloaded file is incomplete or corrupted".into());
    }

    let part = dest.with_extension("exe.part");
    let mut file = fs::File::create(&part).map_err(|e| e.to_string())?;
    file.write_all(&bytes).map_err(|e| e.to_string())?;
    drop(file);
    fs::rename(&part, dest).map_err(|e| e.to_string())?;
    Ok(dest.to_path_buf())
}

pub fn install(new_exe: &Path) -> Result<(), String> {
    let target = std::env::current_exe().map_err(|e| e.to_string())?;
    swap_in(new_exe, &target)?;

    let started = std::process::Command::new(&target)
        .arg(AFTER_UPDATE)
        .arg(std::process::id().to_string())
        .spawn();
    if let Err(e) = started {
        let _ = fs::rename(&target, new_exe);
        let _ = fs::rename(retired_path(&target), &target);
        return Err(e.to_string());
    }
    Ok(())
}

#[derive(Debug, Serialize)]
pub struct Notes {
    pub version: String,
    pub date: String,
    pub body: String,
}

pub fn release_notes() -> Result<Vec<Notes>, String> {
    let data = get_json(&format!(
        "https://api.github.com/repos/{REPO}/releases?per_page=30"
    ))?;

    let mut notes: Vec<Notes> = data
        .as_array()
        .into_iter()
        .flatten()
        .filter(|rel| !rel["draft"].as_bool().unwrap_or(false))
        .filter(|rel| !rel["prerelease"].as_bool().unwrap_or(false))
        .filter(|rel| !parse_version(rel["tag_name"].as_str().unwrap_or("")).is_empty())
        .map(|rel| Notes {
            version: rel["tag_name"]
                .as_str()
                .unwrap_or("")
                .trim_start_matches(['v', 'V'])
                .to_string(),
            date: rel["published_at"]
                .as_str()
                .unwrap_or("")
                .chars()
                .take(10)
                .collect(),
            body: rel["body"].as_str().unwrap_or("").to_string(),
        })
        .collect();

    notes.sort_by(|a, b| parse_version(&b.version).cmp(&parse_version(&a.version)));
    Ok(notes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_compare_like_python() {
        assert!(is_newer("v1.2.1", "1.2.0"));
        assert!(!is_newer("1.2.1", "1.2.1"));
        assert!(is_newer("v1.10.0", "1.9.3"));
        assert!(!is_newer("v1.2", "1.2.0"));
        assert!(is_newer("1.2.0.1", "1.2"));
        assert!(!is_newer("v1.1.0", "1.2.0"));
        assert!(!is_newer("vn", "1.2.0"));
    }

    #[test]
    fn never_updates_from_the_installer() {
        let setup = "vnrpc_setup.exe";
        assert_eq!(pick_asset(&[setup, "vnrpc.exe"]), Some(1));
        assert_eq!(pick_asset(&[setup, "other.exe"]), Some(1));
        assert_eq!(pick_asset(&[setup]), None);
    }

    #[test]
    fn recognises_what_a_failed_update_left_behind() {
        assert_eq!(base_stem("vnrpc.new.new"), "vnrpc");
        assert_eq!(base_stem("vnrpc"), "vnrpc");
        assert!(is_leftover("vnrpc.new.exe", "vnrpc"));
        assert!(is_leftover("VNRPC.new.new.exe", "vnrpc"));
        assert!(is_leftover("vnrpc.old.exe", "vnrpc"));
        assert!(is_leftover("vnrpc.new.exe.part", "vnrpc"));
        assert!(!is_leftover("vnrpc.exe", "vnrpc"));
        assert!(!is_leftover("vnrpc.backup.exe", "vnrpc"));
        assert!(!is_leftover("other.new.exe", "vnrpc"));
    }

    #[test]
    fn swaps_the_exe_even_in_a_non_ascii_folder() {
        let dir = std::env::temp_dir().join(format!("vnrpc-swap-é更新-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let target = dir.join("vnrpc.exe");
        let staged = staging_path(&target);
        fs::write(&target, "old").unwrap();
        fs::write(&staged, "new").unwrap();

        swap_in(&staged, &target).unwrap();

        assert_eq!(fs::read_to_string(&target).unwrap(), "new");
        assert_eq!(fs::read_to_string(retired_path(&target)).unwrap(), "old");
        assert!(!staged.exists());
        let _ = fs::remove_dir_all(&dir);
    }
}
