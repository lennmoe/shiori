use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::Serialize;
use serde_json::json;
use tauri::{
    AppHandle, Emitter, Manager, PhysicalPosition, WebviewUrl, WebviewWindow,
    WebviewWindowBuilder,
};

use crate::capture::ScreenRect;
use crate::state::Shared;
use crate::winapi;

pub const BROWSER_ARGS: &str =
    "--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection --autoplay-policy=no-user-gesture-required --disable-background-timer-throttling --disable-renderer-backgrounding --disable-backgrounding-occluded-windows";

static CREATING: Mutex<()> = Mutex::new(());

fn focus_main(window: &WebviewWindow) {
    let _ = window.unminimize();
    let _ = window.show();
    let _ = window.set_focus();
}

pub fn show_main(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        focus_main(&window);
        return;
    }

    let app = app.clone();
    std::thread::spawn(move || {
        let _creating = CREATING.lock().unwrap();
        if let Some(window) = app.get_webview_window("main") {
            focus_main(&window);
            return;
        }
        let Some(conf) = app
            .config()
            .app
            .windows
            .iter()
            .find(|w| w.label == "main")
            .cloned()
        else {
            return;
        };
        match WebviewWindowBuilder::from_config(&app, &conf).and_then(|b| b.build()) {
            Ok(window) => {
                let _ = window.show();
                let _ = window.set_focus();
            }
            Err(e) => eprintln!("couldn't open the window: {e}"),
        }
    });
}

static PENDING_PAGE: Mutex<Option<String>> = Mutex::new(None);

pub fn open_page(app: &AppHandle, page: &str) {
    if app.get_webview_window("main").is_some() {
        let _ = app.emit_to("main", "navigate", page);
    } else {
        *PENDING_PAGE.lock().unwrap() = Some(page.to_string());
    }
    show_main(app);
}

pub fn take_pending_page() -> Option<String> {
    PENDING_PAGE.lock().unwrap().take()
}

fn popup(
    app: &AppHandle,
    label: &str,
    page: &str,
    width: f64,
    height: f64,
) -> Option<WebviewWindow> {
    let _creating = CREATING.lock().unwrap();
    if let Some(existing) = app.get_webview_window(label) {
        return Some(existing);
    }
    crate::crash::note(format!("create {label}"));

    let window = WebviewWindowBuilder::new(app, label, WebviewUrl::App(page.into()))
        .additional_browser_args(BROWSER_ARGS)
        .title("Shiori RPC")
        .inner_size(width, height)
        .decorations(false)
        .transparent(true)
        .shadow(false)
        .resizable(false)
        .skip_taskbar(true)
        .always_on_top(true)
        .focused(false)
        .focusable(false)
        .visible(false)
        .build()
        .ok()?;

    if let Ok(hwnd) = window.hwnd() {
        winapi::make_no_activate(hwnd.0 as isize);
    }
    Some(window)
}

const GUIDE: &str = "walkthrough";
const GUIDE_SIZE: (f64, f64) = (380.0, 440.0);

fn guide_position(shared: &Shared) -> Option<(i32, i32)> {
    let cfg = shared.config.lock().unwrap();
    let at = cfg.get("walkthrough_overlay_at")?;
    Some((at.get(0)?.as_i64()? as i32, at.get(1)?.as_i64()? as i32))
}

pub fn hide_walkthrough(shared: &Shared) {
    let Some(window) = shared.app.get_webview_window(GUIDE) else {
        return;
    };
    if !window.is_visible().unwrap_or(false) {
        return;
    }
    if let Ok(pos) = window.outer_position() {
        let changes = serde_json::Map::from_iter([("walkthrough_overlay_at".to_string(), json!([pos.x, pos.y]))]);
        let _ = shared.config.lock().unwrap().update(changes);
    }
    let _ = window.hide();
}

pub fn toggle_walkthrough(shared: &Arc<Shared>) {
    let enabled = shared
        .config
        .lock()
        .unwrap()
        .get("walkthrough_enabled")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(true);
    if !enabled {
        hide_walkthrough(shared);
        return;
    }
    let shown = shared
        .app
        .get_webview_window(GUIDE)
        .is_some_and(|w| w.is_visible().unwrap_or(false));
    if shown {
        hide_walkthrough(shared);
        return;
    }
    let shared = Arc::clone(shared);
    std::thread::spawn(move || {
        let snap = shared.snapshot();
        let area = (snap.detected && snap.hwnd != 0)
            .then(|| crate::capture::client_rect_on_screen(winapi::content_window(snap.hwnd)));
        let Some(window) = popup(&shared.app, GUIDE, "walkthrough.html", GUIDE_SIZE.0, GUIDE_SIZE.1) else {
            return;
        };
        let scale = window.scale_factor().unwrap_or(1.0);
        let gap = (16.0 * scale) as i32;
        let width = (GUIDE_SIZE.0 * scale) as i32;
        let (x, y) = match (guide_position(&shared), area) {
            (Some(at), _) => at,
            (None, Some(area)) => (area.right - width - gap, area.top + gap),
            (None, None) => (gap * 6, gap * 6),
        };
        let _ = window.set_position(PhysicalPosition::new(x, y));
        let _ = window.show();
        let _ = window.emit("walkthrough-changed", json!({ "key": snap.key }));
    });
}

fn close_later(app: &AppHandle, label: &'static str, generation: &'static AtomicU64, seconds: f64) {
    let mine = generation.fetch_add(1, Ordering::SeqCst) + 1;
    let app = app.clone();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_secs_f64(seconds));
        if generation.load(Ordering::SeqCst) == mine {
            if let Some(window) = app.get_webview_window(label) {
                crate::crash::note(format!("hide {label}"));
                let _ = window.hide();
            }
        }
    });
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct Toast {
    pub title: String,
    pub detail: String,
    pub image: Option<String>,
    pub ok: bool,
    pub action: String,
}

static TOAST: Mutex<Option<Toast>> = Mutex::new(None);
static TOAST_GENERATION: AtomicU64 = AtomicU64::new(0);

pub fn show_toast(app: &AppHandle, toast: Toast, area: Option<ScreenRect>) {
    *TOAST.lock().unwrap() = Some(toast.clone());

    let app = app.clone();
    std::thread::spawn(move || {
        let app = &app;
        let Some(window) = popup(app, "toast", "toast.html", 340.0, 76.0) else {
            return;
        };

        let (x, y) = match area {
            Some(area) => (area.left + 16, area.top + 16),
            None => {
                let (cx, cy) = winapi::cursor_pos().unwrap_or((0, 0));
                let (left, top, _, _) = winapi::work_area_at(cx, cy);
                (left + 16, top + 16)
            }
        };
        let _ = window.set_position(PhysicalPosition::new(x, y));
        let _ = window.emit("toast", &toast);
        let _ = window.show();

        let seconds = if toast.action.is_empty() { 2.8 } else { 7.0 };
        close_later(app, "toast", &TOAST_GENERATION, seconds);
    });
}

pub fn current_toast() -> Option<Toast> {
    TOAST.lock().unwrap().clone()
}

static BALLOON_TEXT: Mutex<(String, String)> = Mutex::new((String::new(), String::new()));
static BALLOON_GENERATION: AtomicU64 = AtomicU64::new(0);
static BUBBLE: Mutex<Option<(i32, i32, i32, i32)>> = Mutex::new(None);
static BUBBLE_WATCH: AtomicBool = AtomicBool::new(false);

pub fn show_balloon(app: &AppHandle, mascot: &str, text: &str, seconds: f64) {
    *BALLOON_TEXT.lock().unwrap() = (text.to_string(), mascot.to_string());

    let app = app.clone();
    let said = balloon_text();
    std::thread::spawn(move || {
        let app = &app;
        let Some(window) = popup(app, "balloon", "balloon.html", 300.0, 220.0) else {
            return;
        };
        let _ = window.emit("say", &said);

        let mine = BALLOON_GENERATION.fetch_add(1, Ordering::SeqCst) + 1;
        std::thread::sleep(Duration::from_secs_f64(seconds));
        if BALLOON_GENERATION.load(Ordering::SeqCst) == mine {
            hide_balloon(app);
        }
    });
}

pub fn balloon_text() -> serde_json::Value {
    let (text, mascot) = BALLOON_TEXT.lock().unwrap().clone();
    serde_json::json!({ "text": text, "mascot": mascot })
}

pub fn place_balloon(app: &AppHandle, width: u32, height: u32, anchor: Option<Vec<f64>>) -> &'static str {
    let Some(speaker) = crate::mascot::speaker() else {
        return "left";
    };
    let (Some(balloon), Some(mascot)) = (
        app.get_webview_window("balloon"),
        app.get_webview_window(&speaker),
    ) else {
        return "left";
    };
    let (Ok(pos), Ok(size), Ok(frame)) = (
        mascot.outer_position(),
        mascot.outer_size(),
        balloon.outer_size(),
    ) else {
        return "left";
    };

    let (bw, bh) = (width as i32, height as i32);
    let (cw, ch) = (size.width as i32, size.height as i32);
    let (left, top, right, bottom) = winapi::work_area_at(pos.x + cw / 2, pos.y + ch / 2);
    let (pad, share) = crate::mascot::content_share(&speaker).unwrap_or((0.0, 1.0));
    let (px, cw) = (pos.x + (pad * f64::from(cw)) as i32, (share * f64::from(cw)) as i32);

    let anchor = anchor.filter(|a| (2..=3).contains(&a.len()) && a.iter().all(|n| n.is_finite()));
    let (x, y, before) = match anchor.as_deref() {
        Some([ax, ay, rest @ ..]) => {
            let (ax, ay) = (ax.clamp(-3.0, 4.0), ay.clamp(-3.0, 4.0));
            let at = px + (ax * cw as f64) as i32;
            let (x, before) = match rest.first() {
                Some(side) if *side > 0.0 => (at - bw, true),
                Some(_) => (at, false),
                None => (at - bw / 2, ax < 0.5),
            };
            let y = pos.y + (ay * ch as f64) as i32 - bh / 2;
            (x, y, before)
        }
        _ => {
            let mut x = px - bw + cw / 6;
            let before = x >= left;
            if !before {
                x = px + cw - cw / 6;
            }
            (x, top.max(pos.y + ch / 10), before)
        }
    };
    let x = x.clamp(left, (right - bw).max(left));
    let y = y.clamp(top, (bottom - bh).max(top));

    let target = if before {
        PhysicalPosition::new(x + bw - frame.width as i32, y)
    } else {
        PhysicalPosition::new(x, y)
    };
    if balloon.outer_position().ok() != Some(target) {
        let _ = balloon.set_position(target);
    }
    if !balloon.is_visible().unwrap_or(false) {
        let _ = balloon.set_ignore_cursor_events(true);
        let _ = balloon.show();
    }
    raise(&balloon, &mascot);
    *BUBBLE.lock().unwrap() = Some((x, y, bw, bh));
    watch_bubble(app);
    if before {
        "right"
    } else {
        "left"
    }
}

fn raise(window: &WebviewWindow, mascot: &WebviewWindow) {
    if let (Ok(hwnd), Ok(under)) = (window.hwnd(), mascot.hwnd()) {
        winapi::stack_above(hwnd.0 as isize, under.0 as isize);
    }
}

fn watch_bubble(app: &AppHandle) {
    if BUBBLE_WATCH.swap(true, Ordering::SeqCst) {
        return;
    }
    let app = app.clone();
    std::thread::spawn(move || {
        let mut ignoring = true;
        let mut ticks = 0u32;
        while let Some(window) = app.get_webview_window("balloon") {
            std::thread::sleep(Duration::from_millis(60));
            ticks = ticks.wrapping_add(1);
            if ticks % 8 == 0 && window.is_visible().unwrap_or(false) {
                if let Some(mascot) = crate::mascot::speaker().and_then(|label| app.get_webview_window(&label)) {
                    raise(&window, &mascot);
                }
            }
            let over = match (*BUBBLE.lock().unwrap(), winapi::cursor_pos()) {
                (Some((x, y, w, h)), Some((cx, cy))) => cx >= x && cx < x + w && cy >= y && cy < y + h,
                _ => false,
            };
            if ignoring == over {
                ignoring = !over;
                let _ = window.set_ignore_cursor_events(ignoring);
            }
        }
        BUBBLE_WATCH.store(false, Ordering::SeqCst);
    });
}

pub fn hide_balloon(app: &AppHandle) {
    BALLOON_GENERATION.fetch_add(1, Ordering::SeqCst);
    *BUBBLE.lock().unwrap() = None;
    if app.get_webview_window("balloon").is_some() {
        crate::crash::note("hide balloon");
        let _ = app.emit_to("balloon", "hush", ());
    }
}
