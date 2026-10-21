use std::sync::Arc;

use windows::Win32::UI::Input::KeyboardAndMouse::{
    RegisterHotKey, UnregisterHotKey, HOT_KEY_MODIFIERS, MOD_NOREPEAT,
};
use windows::Win32::UI::WindowsAndMessaging::{
    MsgWaitForMultipleObjects, PeekMessageW, MSG, PM_REMOVE, QS_ALLINPUT, WM_HOTKEY,
};

use crate::state::Shared;
use crate::winapi;

const MOD_ALT: u32 = 0x1;
const MOD_CONTROL: u32 = 0x2;
const MOD_SHIFT: u32 = 0x4;
const MOD_WIN: u32 = 0x8;

const MODIFIERS: [(&str, u32); 4] = [
    ("Ctrl", MOD_CONTROL),
    ("Alt", MOD_ALT),
    ("Shift", MOD_SHIFT),
    ("Win", MOD_WIN),
];

const NAMED_KEYS: [(&str, u32); 31] = [
    ("PrintScreen", 0x2C),
    ("Pause", 0x13),
    ("Insert", 0x2D),
    ("Delete", 0x2E),
    ("Home", 0x24),
    ("End", 0x23),
    ("PageUp", 0x21),
    ("PageDown", 0x22),
    ("ScrollLock", 0x91),
    ("Space", 0x20),
    ("Tab", 0x09),
    ("Left", 0x25),
    ("Up", 0x26),
    ("Right", 0x27),
    ("Down", 0x28),
    ("Numpad0", 0x60),
    ("Numpad1", 0x61),
    ("Numpad2", 0x62),
    ("Numpad3", 0x63),
    ("Numpad4", 0x64),
    ("Numpad5", 0x65),
    ("Numpad6", 0x66),
    ("Numpad7", 0x67),
    ("Numpad8", 0x68),
    ("Numpad9", 0x69),
    ("NumpadMultiply", 0x6A),
    ("NumpadAdd", 0x6B),
    ("NumpadSubtract", 0x6D),
    ("NumpadDecimal", 0x6E),
    ("NumpadDivide", 0x6F),
    ("Escape", 0x1B),
];

fn vk_for(name: &str) -> Option<u32> {
    let upper = name.to_uppercase();

    if name.len() == 1 && name.chars().all(|c| c.is_ascii_alphanumeric()) {
        return Some(upper.chars().next()? as u32);
    }

    if let Some(n) = upper.strip_prefix('F').and_then(|n| n.parse::<u32>().ok()) {
        if (1..=24).contains(&n) {
            return Some(0x70 + n - 1);
        }
    }

    NAMED_KEYS
        .iter()
        .find(|(key, _)| key.to_uppercase() == upper)
        .map(|(_, vk)| *vk)
}

pub fn parse(text: &str) -> Option<(u32, u32)> {
    let parts: Vec<&str> = text
        .split('+')
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .collect();
    let (key, mods) = parts.split_last()?;

    let mut flags = 0;
    for part in mods {
        let (_, flag) = MODIFIERS
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case(part))?;
        flags |= flag;
    }

    Some((flags, vk_for(key)?))
}

pub fn normalize(text: &str) -> String {
    let Some((mods, vk)) = parse(text) else {
        return String::new();
    };

    let key = if (0x70..=0x87).contains(&vk) {
        format!("F{}", vk - 0x6F)
    } else if let Some((name, _)) = NAMED_KEYS.iter().find(|(_, code)| *code == vk) {
        name.to_string()
    } else {
        char::from_u32(vk).map(String::from).unwrap_or_default()
    };

    let mut parts: Vec<String> = MODIFIERS
        .iter()
        .filter(|(_, flag)| mods & flag != 0)
        .map(|(name, _)| name.to_string())
        .collect();
    parts.push(key);
    parts.join("+")
}

const HOTKEY_ID: i32 = 1;
const MASCOT_HOTKEY_ID: i32 = 2;

fn game_in_front(pid: u32) -> bool {
    pid != 0 && winapi::window_pid(winapi::foreground_window()) == pid
}

fn register(id: i32, (mods, vk): (u32, u32)) -> bool {
    let flags = HOT_KEY_MODIFIERS(mods) | MOD_NOREPEAT;
    unsafe { RegisterHotKey(None, id, flags, vk).is_ok() }
}

fn unregister(id: i32) {
    unsafe {
        let _ = UnregisterHotKey(None, id);
    }
}

fn keep(id: i32, active: &mut Option<(u32, u32)>, wanted: Option<(u32, u32)>) -> bool {
    if wanted == *active {
        return true;
    }
    if active.is_some() {
        unregister(id);
    }
    *active = wanted;
    wanted.is_none_or(|combo| register(id, combo))
}

pub fn run(shared: Arc<Shared>) {
    let mut active: Option<(u32, u32)> = None;
    let mut mascot_active: Option<(u32, u32)> = None;
    let mut reported = false;
    let mut mascot_reported = false;
    let mut msg = MSG::default();

    loop {
        let pid = shared.snapshot().pid;
        unsafe {
            while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                if msg.message != WM_HOTKEY {
                    continue;
                }
                if msg.wParam.0 == MASCOT_HOTKEY_ID as usize {
                    let key = shared.config.lock().unwrap().str("mascot_click_hotkey");
                    crate::mascot::toggle_view_only(&shared.app, &key);
                } else if game_in_front(pid) {
                    let shared = Arc::clone(&shared);
                    std::thread::spawn(move || crate::screenshots::take(&shared));
                }
            }
        }

        let (hotkey, mascot_hotkey) = {
            let cfg = shared.config.lock().unwrap();
            (cfg.str("screenshot_hotkey"), cfg.str("mascot_click_hotkey"))
        };
        let wanted = if game_in_front(pid) {
            parse(&hotkey)
        } else {
            None
        };
        let mascot_wanted = parse(&mascot_hotkey).filter(|combo| Some(*combo) != wanted);

        if !keep(HOTKEY_ID, &mut active, wanted) && !reported {
            reported = true;
            shared.emit(
                "toast",
                serde_json::json!({
                    "title": "Couldn't watch the screenshot key: another app already uses it",
                    "ok": false,
                }),
            );
        }
        if !keep(MASCOT_HOTKEY_ID, &mut mascot_active, mascot_wanted) && !mascot_reported {
            mascot_reported = true;
            shared.emit(
                "toast",
                serde_json::json!({
                    "title": "Couldn't watch the mascot key: another app already uses it",
                    "ok": false,
                }),
            );
        }

        unsafe {
            MsgWaitForMultipleObjects(None, false, 100, QS_ALLINPUT);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hotkeys_are_normalized_like_python() {
        assert_eq!(normalize("shift+ctrl+f12"), "Ctrl+Shift+F12");
        assert_eq!(normalize("PrintScreen"), "PrintScreen");
        assert_eq!(normalize("alt+s"), "Alt+S");
        assert_eq!(normalize("Ctrl+"), "");
        assert_eq!(normalize("Hyper+S"), "");
        assert_eq!(normalize(""), "");
    }
}
