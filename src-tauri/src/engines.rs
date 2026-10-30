use std::collections::HashSet;
use std::path::Path;
use std::sync::LazyLock;

use fancy_regex::Regex;

use crate::winapi::WindowInfo;

pub const BUILTIN_BLACKLIST: &[&str] = &[
    "explorer.exe",
    "chrome.exe",
    "firefox.exe",
    "msedge.exe",
    "brave.exe",
    "opera.exe",
    "discord.exe",
    "spotify.exe",
    "code.exe",
    "devenv.exe",
    "steam.exe",
    "steamwebhelper.exe",
    "obs64.exe",
    "obs32.exe",
    "notepad.exe",
    "notepad++.exe",
    "python.exe",
    "pythonw.exe",
    "cmd.exe",
    "powershell.exe",
    "windowsterminal.exe",
    "textinputhost.exe",
    "searchhost.exe",
    "applicationframehost.exe",
    "systemsettings.exe",
    "taskmgr.exe",
    "shiori.exe",
    "vnrpc.exe",
    "visualnovelrpc.exe",
    "msedgewebview2.exe",
    "lockapp.exe",
    "shellexperiencehost.exe",
    "startmenuexperiencehost.exe",
    "sechealthui.exe",
    "searchapp.exe",
    "searchui.exe",
    "peopleexperiencehost.exe",
    "widgets.exe",
    "gamebar.exe",
    "phoneexperiencehost.exe",
];

static WINDOWS_DIR: LazyLock<String> = LazyLock::new(|| {
    let root = std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".into());
    format!("{}\\", root.trim_end_matches('\\').to_lowercase())
});

pub fn is_windows_app(exe_path: &str) -> bool {
    exe_path
        .replace('/', "\\")
        .to_lowercase()
        .starts_with(WINDOWS_DIR.as_str())
}

static HELPER_EXE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?i)^(?:launcher|config|configtool|setup|settings|unins\d*|uninstall|update|updater|crashreport|crashhandler|language)(?:[_-]?(?:x64|x86|64|32))?\.exe$",
    )
    .unwrap()
});

pub fn is_helper_exe(exe: &str) -> bool {
    HELPER_EXE.is_match(exe).unwrap_or(false)
}

fn file_matches(rule: &str, name: &str) -> bool {
    if rule.starts_with('.') {
        name.ends_with(rule)
    } else {
        name == rule
    }
}

pub fn normalize_exe(name: &str) -> String {
    let base = name
        .trim()
        .rsplit(['\\', '/'])
        .next()
        .unwrap_or("")
        .to_lowercase();
    if !base.is_empty() && !base.ends_with(".exe") {
        format!("{base}.exe")
    } else {
        base
    }
}

pub fn blacklist_set<'a>(user: impl IntoIterator<Item = &'a str>) -> HashSet<String> {
    let mut set: HashSet<String> = BUILTIN_BLACKLIST.iter().map(|s| s.to_string()).collect();
    set.extend(
        user.into_iter()
            .map(normalize_exe)
            .filter(|s| !s.is_empty()),
    );
    set
}

pub struct Engine {
    pub name: &'static str,
    exe_patterns: Vec<Regex>,
    class_patterns: Vec<Regex>,
    dir_files: &'static [&'static str],
}

fn rx(patterns: &[&str]) -> Vec<Regex> {
    patterns
        .iter()
        .map(|p| Regex::new(p).expect("engine pattern"))
        .collect()
}

impl Engine {
    fn new(
        name: &'static str,
        exe: &[&str],
        class: &[&str],
        dir_files: &'static [&'static str],
    ) -> Self {
        Engine {
            name,
            exe_patterns: rx(exe),
            class_patterns: rx(class),
            dir_files,
        }
    }

    fn score(&self, win: &WindowInfo) -> u32 {
        let exe = win.exe().to_lowercase();
        let cls = win.class_name.to_lowercase();
        let mut pts = 0;
        if self
            .exe_patterns
            .iter()
            .any(|r| r.is_match(&exe).unwrap_or(false))
        {
            pts += 40;
        }
        if !self.dir_files.is_empty() && !win.exe_path.is_empty() {
            let listing: Vec<String> = Path::new(&win.exe_path)
                .parent()
                .and_then(|d| std::fs::read_dir(d).ok())
                .map(|it| {
                    it.flatten()
                        .map(|e| e.file_name().to_string_lossy().to_lowercase())
                        .collect()
                })
                .unwrap_or_default();
            if self
                .dir_files
                .iter()
                .any(|f| listing.iter().any(|n| file_matches(f, n)))
            {
                pts += 20;
            }
        }
        if pts > 0
            && self
                .class_patterns
                .iter()
                .any(|r| r.is_match(&cls).unwrap_or(false))
        {
            pts += 25;
        }
        pts
    }
}

static ENGINES: LazyLock<Vec<Engine>> = LazyLock::new(|| {
    vec![
        Engine::new(
            "Ren'Py",
            &[],
            &[r"^sdl_app$", r"pygame", r"renpy"],
            &[".rpa", ".rpyc", "renpy", "game/script"],
        ),
        Engine::new(
            "KiriKiri",
            &[r"^krkr", r"kirikiri"],
            &[r"tvp", r"kirikiri"],
            &[".xp3", "data.xp3", "startup.tjs"],
        ),
        Engine::new(
            "Buriko General Interpreter",
            &[r"^bgi\.exe$", r"ethornell"],
            &[],
            &["sysgrp.arc", "sysprg.arc", "data01000.arc", "bgi.gdb"],
        ),
        Engine::new("AdvHD", &[r"^advhd"], &[], &["rio.arc", ".ws2"]),
        Engine::new(
            "CatSystem2",
            &[r"^cs2\.exe$", r"catsystem"],
            &[],
            &["scene.int", "config.int", "cs2.conf"],
        ),
        Engine::new("KaGuYa", &[r"kaguya"], &[], &["message.dat"]),
        Engine::new("Majiro", &[r"majiro"], &[], &["scenario.arc", ".mjo"]),
        Engine::new("Musica", &[r"musica"], &[], &["scr.paz", "sys.paz"]),
        Engine::new("Propeller", &[], &[], &[".mpk"]),
        Engine::new("ShSystem", &[], &[], &[".hxp", ".hst"]),
        Engine::new("AI6WIN", &[r"^ai[56]win"], &[], &["mes.arc"]),
        Engine::new("Qlie", &[r"qlie"], &[], &[".pack"]),
        Engine::new("Softpal", &[], &[], &["pal.dll", "script.src"]),
        Engine::new("SystemNNN", &[r"systemnnn"], &[], &[".spt", ".nnn"]),
        Engine::new("TmrHiroAdvSystem", &[], &[], &[".srp"]),
        Engine::new(
            "TyranoScript",
            &[r"nw\.exe$", r"tyrano"],
            &[r"^nw_", r"chrome_widgetwin"],
            &["tyrano", "data/scenario"],
        ),
        Engine::new(
            "SiglusEngine",
            &[r"siglus", r"^gameexe"],
            &[],
            &["scene.pck", "gameexe.dat"],
        ),
        Engine::new(
            "RealLive",
            &[r"reallive", r"^rlvm"],
            &[],
            &["seen.txt", "gameexe.ini"],
        ),
        Engine::new("Artemis", &[r"artemis"], &[], &[".pfs", "root.pfs"]),
        Engine::new("YU-RIS", &[r"yuris", r"^ys_"], &[], &["ysbin", ".ypf"]),
        Engine::new(
            "NScripter",
            &[r"nscr", r"onscripter", r"ons\.exe$"],
            &[],
            &["nscript.dat", "0.txt", "arc.nsa"],
        ),
        Engine::new(
            "Unity",
            &[],
            &[r"^unitywndclass$"],
            &["_data/globalgamemanagers", "unityplayer.dll"],
        ),
    ]
});

pub fn detect_engine(win: &WindowInfo) -> Option<(&'static Engine, u32)> {
    if BUILTIN_BLACKLIST.contains(&win.exe().to_lowercase().as_str()) || is_helper_exe(win.exe()) {
        return None;
    }
    let mut best: Option<(&'static Engine, u32)> = None;
    for eng in ENGINES.iter() {
        let s = eng.score(win);
        if s > best.map_or(0, |(_, b)| b) {
            best = Some((eng, s));
        }
    }
    best.filter(|(_, s)| *s >= 20)
}

static COMMON_CLEANERS: LazyLock<Vec<Regex>> = LazyLock::new(|| {
    [
        r"\s*[-–—]\s*steam\s*$",
        r"[\(\[【]?R[\-\s]?18\+?[\)\]】]?(?:版|edition)?[@＠]*",
        r"\s*\[?\d{3,4}\s*[xX]\s*\d{3,4}\]?",
        r"\s*\bver(?:sion)?\.?\s*\d+(?:[.;]\d+)*[a-z]?\b",
        r"\s*\[ver\.[^\]]*\]",
        r"\s*[\[(]\s*v?\d+(?:[.;]\d+)+[a-z]?\s*[\])]",
        r"\s*\bv\d+(?:[.;]\d+)+[a-z]?\b",
        r"\s*(?<![\w/])\d+[.;]\d+(?:[.;]\d+)*[a-z]?\b",
        r"\s*\bfps[:=]?\s*\d+\b",
        r"\s*\bDirect3D\b|\s*\bOpenGL\b",
        r"\s*[\[(（【][^\])）】]*\b(?:edition|hd|remaster(?:ed)?|english|steam|uncensored|full\s*voice|patch(?:ed)?|translat\w*)\b[^\])）】]*[\])）】]",
        r"\s*[\[(（【]\s*[\])）】]",
    ]
    .iter()
    .map(|p| Regex::new(&format!("(?i){p}")).expect("cleaner"))
    .collect()
});

static MULTI_SPACE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\s{2,}").unwrap());

pub fn tidy(text: &str) -> String {
    let text = MULTI_SPACE.replace_all(text, " ");
    text.trim()
        .trim_matches(|c: char| " -–—|:·•".contains(c))
        .trim()
        .to_string()
}

pub fn clean_title(title: &str) -> String {
    let mut out = title.replace("\u{81}\u{40}", " ");
    for r in COMMON_CLEANERS.iter() {
        out = r.replace_all(&out, "").into_owned();
    }
    tidy(&out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_rules_match_whole_names() {
        assert!(file_matches(".pack", "data0.pack"));
        assert!(!file_matches(".pack", "microsoft.system.package.metadata"));
        assert!(file_matches("pal.dll", "pal.dll"));
        assert!(!file_matches("pal.dll", "opal.dll"));
    }

    #[test]
    fn launchers_are_not_games() {
        assert!(is_helper_exe("LAUNCHER.exe"));
        assert!(is_helper_exe("LAUNCHER_x64.exe"));
        assert!(is_helper_exe("unins000.exe"));
        assert!(!is_helper_exe("nine_kokoiro.exe"));
        assert!(!is_helper_exe("launcher_of_dreams.exe"));
    }

    #[test]
    fn windows_apps_are_recognized() {
        assert!(is_windows_app(
            r"C:\Windows\SystemApps\Microsoft.LockApp_cw5n1h2txyewy\LockApp.exe"
        ));
        assert!(is_windows_app("c:/windows/explorer.exe"));
        assert!(!is_windows_app(r"C:\Games\Windows Shell\game.exe"));
    }
}
