use winreg::enums::{HKEY_CURRENT_USER, KEY_READ, KEY_SET_VALUE};
use winreg::RegKey;

const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const VALUE_NAME: &str = "Shiori RPC";
const VALUE_NAME_BEFORE_RENAME: &str = "VisualNovelRPC";

pub fn is_enabled() -> bool {
    RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey_with_flags(RUN_KEY, KEY_READ)
        .and_then(|key| key.get_value::<String, _>(VALUE_NAME))
        .is_ok()
}

fn wanted_command() -> Option<String> {
    let exe = std::env::current_exe().ok()?;
    Some(format!("\"{}\" --minimized", exe.display()))
}

fn carry_over_old_value() {
    let Ok(key) = RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey_with_flags(RUN_KEY, KEY_READ | KEY_SET_VALUE)
    else {
        return;
    };
    if key.get_value::<String, _>(VALUE_NAME_BEFORE_RENAME).is_ok() {
        let _ = key.delete_value(VALUE_NAME_BEFORE_RENAME);
        let _ = set_enabled(true);
    }
}

pub fn refresh() {
    carry_over_old_value();
    let saved = RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey_with_flags(RUN_KEY, KEY_READ)
        .and_then(|key| key.get_value::<String, _>(VALUE_NAME));
    if let (Ok(saved), Some(wanted)) = (saved, wanted_command()) {
        if saved != wanted {
            let _ = set_enabled(true);
        }
    }
}

pub fn set_enabled(enabled: bool) -> Result<(), String> {
    let key = RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey_with_flags(RUN_KEY, KEY_SET_VALUE)
        .map_err(|e| e.to_string())?;

    if enabled {
        let command = wanted_command().ok_or("can't find the app's location")?;
        return key
            .set_value(VALUE_NAME, &command)
            .map_err(|e| e.to_string());
    }

    match key.delete_value(VALUE_NAME) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.to_string()),
        _ => Ok(()),
    }
}
