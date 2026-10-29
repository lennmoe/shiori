use serde_json::{Map, Value};

use crate::config::Config;
use crate::discord::Activity;
use crate::engine::format_playtime;
use crate::lang;
use crate::state::Snapshot;

pub const APP_ICON: &str = "https://raw.githubusercontent.com/lennmoe/vnrpc/main/ui/assets/app_icon.png";

pub type Vars = Vec<(&'static str, String)>;

pub fn language(cfg: &Config) -> &'static str {
    match cfg.str("presence_language").as_str() {
        "" | "app" => lang::resolve(&cfg.str("language")),
        fixed => lang::resolve(fixed),
    }
}

fn text(entry: &Map<String, Value>, field: &str) -> String {
    entry
        .get(field)
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string()
}

fn capitalize(word: &str) -> String {
    let mut chars = word.chars();
    chars
        .next()
        .map(|first| first.to_uppercase().chain(chars).collect())
        .unwrap_or_default()
}

pub fn vars(snap: &Snapshot, entry: &Map<String, Value>, original_first: bool, language: &str) -> Vars {
    let romanized = [&snap.game_name, &snap.raw_title]
        .into_iter()
        .find(|n| !n.is_empty())
        .cloned()
        .unwrap_or_else(|| "Visual Novel".into());
    let original = snap
        .vn
        .as_ref()
        .map(|vn| vn.alt_title.trim().to_string())
        .unwrap_or_default();
    let title = if original_first && !original.is_empty() {
        original.clone()
    } else {
        romanized.clone()
    };
    let length = snap.vn.as_ref().map_or(0, |vn| vn.length_minutes);
    let progress = (length > 0)
        .then(|| snap.playtime_seconds / 60 * 100 / length)
        .filter(|p| *p < 100)
        .map(|p| format!("{p}%"))
        .unwrap_or_default();
    let today = entry
        .get("daily")
        .and_then(|d| d.get(crate::config::today()))
        .and_then(Value::as_i64)
        .unwrap_or(0);

    vec![
        ("title", title),
        ("romanized", romanized),
        ("original", original),
        ("reading", lang::tr("Reading", language).to_string()),
        ("section", snap.section_label.clone()),
        ("total", (snap.playtime_seconds > 0).then(|| format_playtime(snap.playtime_seconds)).unwrap_or_default()),
        ("today", (today > 0).then(|| format_playtime(today)).unwrap_or_default()),
        ("progress", progress),
        ("length", (length > 0).then(|| format!("~{}h", (length + 30) / 60)).unwrap_or_default()),
        ("status", capitalize(&text(entry, "status"))),
        ("rating", snap.vn.as_ref().filter(|vn| vn.rating > 0.0).map(|vn| format!("{:.1}", vn.rating / 10.0)).unwrap_or_default()),
        ("year", snap.vn.as_ref().map(|vn| vn.year.chars().take(4).collect()).unwrap_or_default()),
        ("engine", snap.engine_name.clone()),
        ("vndb", snap.vn.as_ref().map(|vn| vn.url()).unwrap_or_default()),
    ]
}

pub fn sample_vars(language: &str) -> Vars {
    vec![
        ("title", "Senren＊Banka".into()),
        ("romanized", "Senren＊Banka".into()),
        ("original", "千恋＊万花".into()),
        ("reading", lang::tr("Reading", language).to_string()),
        ("section", "Murasame Route".into()),
        ("total", "35h 11m".into()),
        ("today", "1h 20m".into()),
        ("progress", "78%".into()),
        ("length", "~45h".into()),
        ("status", "Playing".into()),
        ("rating", "8.3".into()),
        ("year", "2016".into()),
        ("engine", "SiglusEngine".into()),
        ("vndb", "https://vndb.org/v19073".into()),
    ]
}

const SEPARATORS: [&str; 5] = [" - ", " — ", "·", "|", "・"];

fn segments(template: &str) -> Vec<(&str, &str)> {
    let mut out = vec![];
    let mut sep = "";
    let mut rest = template;
    loop {
        let next = SEPARATORS
            .iter()
            .filter_map(|s| rest.find(s).map(|i| (i, *s)))
            .min_by_key(|(i, _)| *i);
        let Some((i, s)) = next else {
            out.push((sep, rest));
            return out;
        };
        out.push((sep, &rest[..i]));
        sep = s;
        rest = &rest[i + s.len()..];
    }
}

fn fill_segment(segment: &str, vars: &Vars) -> Option<String> {
    let mut out = String::new();
    let mut known = 0;
    let mut filled = 0;
    let mut rest = segment;
    while let Some(open) = rest.find('{') {
        out.push_str(&rest[..open]);
        let after = &rest[open + 1..];
        let Some(close) = after.find('}') else {
            out.push_str(&rest[open..]);
            rest = "";
            break;
        };
        match vars.iter().find(|(key, _)| *key == &after[..close]) {
            Some((_, value)) => {
                known += 1;
                filled += usize::from(!value.is_empty());
                out.push_str(value);
            }
            None => out.push_str(&rest[open..open + close + 2]),
        }
        rest = &after[close + 1..];
    }
    out.push_str(rest);
    let out = out.split_whitespace().collect::<Vec<_>>().join(" ");
    (!out.is_empty() && (known == 0 || filled > 0)).then_some(out)
}

pub fn fill(template: &str, vars: &Vars) -> String {
    let mut out = String::new();
    for (sep, segment) in segments(template) {
        let Some(text) = fill_segment(segment, vars) else {
            continue;
        };
        if !out.is_empty() {
            match sep {
                "・" => out.push('・'),
                _ => out.push_str(&format!(" {} ", sep.trim())),
            }
        }
        out.push_str(&text);
    }
    if out.chars().count() < 2 {
        String::new()
    } else {
        out
    }
}

fn field<'a>(custom: &'a Value, key: &str) -> &'a str {
    custom.get(key).and_then(Value::as_str).unwrap_or("")
}

pub fn enabled(custom: Option<&Value>) -> bool {
    custom
        .and_then(|c| c.get("enabled"))
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

pub fn apply(activity: &mut Activity, custom: &Value, vars: &Vars, session_start: i64, playtime: i64, now: i64) {
    let name = fill(field(custom, "name"), vars);
    if !name.is_empty() {
        activity.name = name;
    }
    activity.details = fill(field(custom, "details"), vars);
    activity.state = fill(field(custom, "state"), vars);
    activity.large_text = fill(field(custom, "large_text"), vars);

    activity.small_image = match field(custom, "small_icon") {
        "app" => APP_ICON.to_string(),
        url if url.starts_with("https://") || url.starts_with("http://") => url.to_string(),
        _ => String::new(),
    };
    activity.small_text = if activity.small_image.is_empty() {
        String::new()
    } else {
        fill(field(custom, "small_text"), vars)
    };

    activity.start = match field(custom, "timer") {
        "off" => None,
        "total" => Some(now - playtime.max(0)),
        _ => Some(session_start).filter(|s| *s > 0),
    };

    activity.buttons = custom
        .get("buttons")
        .and_then(Value::as_array)
        .map(|list| {
            list.iter()
                .filter_map(|b| {
                    let label: String = fill(b.get("label")?.as_str()?, vars).chars().take(32).collect();
                    let url = fill(b.get("url")?.as_str()?, vars).replace(' ', "");
                    let web = url.starts_with("https://") || url.starts_with("http://");
                    (!label.is_empty() && web).then_some((label, url))
                })
                .take(2)
                .collect()
        })
        .unwrap_or_default();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Vars {
        let mut vars = sample_vars("en");
        vars.retain(|(k, _)| *k != "progress");
        vars.push(("progress", String::new()));
        vars
    }

    #[test]
    fn empty_variables_leave_no_dangling_separators() {
        let vars = sample();
        assert_eq!(fill("{total} · {progress}", &vars), "35h 11m");
        assert_eq!(fill("{progress} · {total}", &vars), "35h 11m");
        assert_eq!(fill("{reading}・{section}", &vars), "Reading・Murasame Route");
        assert_eq!(fill("{progress}", &vars), "");
        assert_eq!(fill("Read: {progress}", &vars), "");
        assert_eq!(fill("{title} - {progress}", &vars), "Senren＊Banka");
        assert_eq!(fill("Sakura no Uta -Sakura-", &vars), "Sakura no Uta -Sakura-");
        assert_eq!(fill("{unknown} here", &vars), "{unknown} here");
    }

    #[test]
    fn custom_layout_replaces_the_lines() {
        let custom = serde_json::json!({
            "enabled": true,
            "name": "{original}",
            "details": "{section}",
            "state": "{total} today {today}",
            "large_text": "{title} ({year})",
            "small_icon": "app",
            "small_text": "{engine}",
            "timer": "total",
            "buttons": [{ "label": "VNDB", "url": "{vndb}" }, { "label": "Bad", "url": "nope" }],
        });
        let mut activity = Activity::default();
        apply(&mut activity, &custom, &sample(), 0, 3600, 10_000);
        assert_eq!(activity.name, "千恋＊万花");
        assert_eq!(activity.details, "Murasame Route");
        assert_eq!(activity.state, "35h 11m today 1h 20m");
        assert_eq!(activity.large_text, "Senren＊Banka (2016)");
        assert_eq!(activity.small_image, APP_ICON);
        assert_eq!(activity.small_text, "SiglusEngine");
        assert_eq!(activity.start, Some(6_400));
        assert_eq!(activity.buttons, vec![("VNDB".to_string(), "https://vndb.org/v19073".to_string())]);
    }
}
