use std::path::{Path, PathBuf};

use serde::Serialize;
use serde_json::{Map, Value};

use crate::config::Config;
use crate::shell;
use crate::steam;

pub const NORMAL: &str = "";
pub const LOCALE_EMULATOR: &str = "le";
pub const NTLEA: &str = "ntlea";

pub fn tool_setting(launcher: &str) -> &'static str {
    match launcher {
        LOCALE_EMULATOR => "locale_emulator_path",
        NTLEA => "ntlea_path",
        _ => "",
    }
}

pub fn tool_path(cfg: &Config, launcher: &str) -> Option<PathBuf> {
    let path = cfg.str(tool_setting(launcher));
    let path = PathBuf::from(path.trim());
    path.is_file().then_some(path)
}

#[derive(Debug, Serialize)]
#[serde(tag = "kind", content = "detail")]
pub enum LaunchError {
    GameMissing,
    ToolMissing(String),
    Failed(String),
}

pub fn launch_exe(entry: &Map<String, Value>) -> String {
    let text = |key: &str| entry.get(key).and_then(Value::as_str).unwrap_or("").to_string();
    let picked = text("launch_path");
    if !picked.is_empty() && Path::new(&picked).is_file() {
        picked
    } else {
        text("path")
    }
}

pub fn launch(cfg: &Config, entry: &Map<String, Value>) -> Result<(), LaunchError> {
    let exe = launch_exe(entry);
    let exe = Path::new(&exe);
    if exe.as_os_str().is_empty() || !exe.is_file() {
        return Err(LaunchError::GameMissing);
    }
    let folder = exe.parent().unwrap_or(Path::new("."));

    let launcher = entry
        .get("launcher")
        .and_then(Value::as_str)
        .unwrap_or(NORMAL);
    if launcher != LOCALE_EMULATOR && launcher != NTLEA {
        if let Some(id) = steam::launch_id_for_exe(&exe.to_string_lossy()) {
            return shell::open(&format!("steam://rungameid/{id}")).map_err(LaunchError::Failed);
        }
        return shell::start_program(exe, "", folder).map_err(LaunchError::Failed);
    }

    let Some(tool) = tool_path(cfg, launcher) else {
        return Err(LaunchError::ToolMissing(launcher.to_string()));
    };

    let params = if launcher == LOCALE_EMULATOR {
        format!("\"{}\"", exe.display())
    } else {
        format!("\"{}\" C932 L1041", exe.display())
    };
    shell::start_program(&tool, &params, folder).map_err(LaunchError::Failed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_picked_launcher_wins_over_the_detected_exe() {
        let launcher = std::env::current_exe().unwrap().to_string_lossy().to_string();
        let entry = |picked: &str| {
            Map::from_iter([
                ("path".into(), Value::from(r"C:\Games\Iroseka\engine\engine.exe")),
                ("launch_path".into(), Value::from(picked)),
            ])
        };
        assert_eq!(launch_exe(&entry(&launcher)), launcher);
        assert_eq!(launch_exe(&entry("")), r"C:\Games\Iroseka\engine\engine.exe");
        assert_eq!(launch_exe(&entry(r"C:\gone\launcher.exe")), r"C:\Games\Iroseka\engine\engine.exe");
    }
}
