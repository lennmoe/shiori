use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime};

use reqwest::Method;
use serde_json::{json, Value};

const API: &str = "https://api.vndb.org/kana";
const USER_AGENT: &str = concat!("ShioriRPC/", env!("CARGO_PKG_VERSION"));
const MIN_INTERVAL: Duration = Duration::from_millis(500);
const FIELDS: &str =
    "id,title,alttitle,titles.title,titles.latin,titles.lang,titles.official,aliases,released,rating,length_minutes,image.url,image.dims,image.sexual,image.violence";

pub const LIST_LABELS: [(&str, i64); 4] = [
    ("playing", 1),
    ("finished", 2),
    ("stalled", 3),
    ("dropped", 4),
];
const WISHLIST_LABEL: i64 = 5;

#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct VnResult {
    pub id: String,
    pub title: String,
    pub alt_title: String,
    pub year: String,
    pub image_url: String,
    pub nsfw: bool,
    pub rating: f64,
    pub length_minutes: i64,
    pub other_titles: Vec<String>,
    pub english_title: String,
}

pub(crate) fn is_nsfw(image: &Value) -> bool {
    let sexual = image["sexual"].as_f64().unwrap_or(0.0);
    let violence = image["violence"].as_f64().unwrap_or(0.0);
    sexual >= 1.0 || violence >= 1.0
}

fn text(value: &Value) -> String {
    value.as_str().unwrap_or("").to_string()
}

fn other_titles(o: &Value) -> Vec<String> {
    let titles = o["titles"].as_array().into_iter().flatten();
    let names = titles
        .flat_map(|t| [&t["title"], &t["latin"]])
        .chain(o["aliases"].as_array().into_iter().flatten())
        .chain([&o["alttitle"]])
        .map(text);

    let mut out: Vec<String> = Vec::new();
    for name in names {
        if !name.is_empty() && name != text(&o["title"]) && !out.contains(&name) {
            out.push(name);
        }
    }
    out
}

fn english_title(o: &Value) -> String {
    o["titles"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|t| t["lang"] == "en" && t["official"] == true)
        .map(|t| text(&t["title"]).trim().to_string())
        .unwrap_or_default()
}

impl VnResult {
    fn from_api(o: &Value) -> VnResult {
        let released = text(&o["released"]);

        VnResult {
            id: text(&o["id"]),
            title: text(&o["title"]),
            alt_title: text(&o["alttitle"]),
            year: released.chars().take(4).collect(),
            image_url: text(&o["image"]["url"]),
            nsfw: is_nsfw(&o["image"]),
            rating: o["rating"].as_f64().unwrap_or(0.0),
            length_minutes: o["length_minutes"].as_i64().unwrap_or(0),
            other_titles: other_titles(o),
            english_title: english_title(o),
        }
    }

    pub fn url(&self) -> String {
        format!("https://vndb.org/{}", self.id)
    }
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct ReleaseCover {
    pub url: String,
    pub kind: String,
    pub release_title: String,
    pub nsfw: bool,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ReleaseImage {
    pub url: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub sexual: Option<f64>,
    pub violence: Option<f64>,
}

impl ReleaseImage {
    pub fn nsfw(&self) -> bool {
        self.sexual.unwrap_or(0.0) >= 1.0 || self.violence.unwrap_or(0.0) >= 1.0
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ReleaseLanguage {
    pub lang: String,
    pub title: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ReleaseMedium {
    pub medium: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ReleaseVn {
    pub id: String,
    pub rtype: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Release {
    pub id: String,
    pub title: String,
    pub alttitle: Option<String>,
    pub released: String,
    pub official: bool,
    pub patch: bool,
    pub platforms: Vec<String>,
    pub languages: Vec<ReleaseLanguage>,
    pub media: Vec<ReleaseMedium>,
    pub vns: Vec<ReleaseVn>,
    pub images: Vec<ReleaseImage>,
}

impl Release {
    pub fn alternate_title(&self, original_language: Option<&str>) -> &str {
        self.languages
            .iter()
            .filter(|language| Some(language.lang.as_str()) == original_language)
            .filter_map(|language| language.title.as_deref())
            .chain(self.alttitle.as_deref())
            .map(str::trim)
            .find(|title| !title.is_empty() && *title != self.title.trim())
            .unwrap_or("")
    }

    pub fn cover(&self) -> Option<&ReleaseImage> {
        ["pkgfront", "dig"].iter().find_map(|kind| {
            self.images
                .iter()
                .find(|image| image.kind == *kind && !image.url.is_empty())
        })
    }
}

#[derive(serde::Serialize)]
pub struct ReleaseList {
    pub original_language: Option<String>,
    pub releases: Vec<Release>,
}

#[derive(serde::Deserialize)]
struct OriginalLanguage {
    olang: Option<String>,
}

#[derive(serde::Deserialize)]
struct OriginalLanguageResponse {
    results: Vec<OriginalLanguage>,
}

fn original_language(data: Value) -> Result<Option<String>, String> {
    let response: OriginalLanguageResponse = serde_json::from_value(data)
        .map_err(|error| format!("couldn't read the VN's original language: {error}"))?;
    let vn = response.results.into_iter().next()
        .ok_or_else(|| "Couldn't find the selected VN on VNDB.".to_string())?;
    Ok(vn.olang.filter(|language| !language.trim().is_empty()))
}

#[derive(serde::Deserialize)]
struct ReleasePage {
    results: Vec<Release>,
    more: bool,
}

fn collect_releases(
    id: &str,
    mut fetch: impl FnMut(usize) -> Result<ReleasePage, String>,
) -> Result<Vec<Release>, String> {
    let mut releases = Vec::new();
    let mut page = 1;
    loop {
        let response = fetch(page)?;
        if response.more && response.results.is_empty() {
            return Err("VNDB returned an empty release page with more results".into());
        }
        if response.results.iter().any(|release| !release.vns.iter().any(|vn| vn.id == id)) {
            return Err("VNDB returned a release that does not belong to the selected VN.".into());
        }
        releases.extend(response.results);
        if !response.more {
            return Ok(releases);
        }
        page += 1;
    }
}

fn collect_selected_release_dates(
    ids: &[String],
    mut fetch: impl FnMut(&Value) -> Result<Value, String>,
) -> Result<HashMap<String, String>, String> {
    let mut dates = HashMap::new();
    for chunk in ids.chunks(100) {
        let mut filters = vec![json!("or")];
        filters.extend(chunk.iter().map(|id| json!(["id", "=", id])));
        let filters = if chunk.len() == 1 {
            filters[1].clone()
        } else {
            Value::Array(filters)
        };
        let body = json!({"filters": filters, "fields": "released", "results": 100});
        let data = fetch(&body)?;
        let results = data["results"]
            .as_array()
            .ok_or_else(|| "Couldn't read release dates from VNDB.".to_string())?;
        for release in results {
            let id = release["id"]
                .as_str()
                .ok_or_else(|| "VNDB returned a release without an ID.".to_string())?;
            let date = release["released"]
                .as_str()
                .ok_or_else(|| format!("VNDB returned an invalid release date for {id}."))?;
            dates.insert(id.to_string(), date.to_string());
        }
    }
    Ok(dates)
}

pub struct Vndb {
    http: reqwest::blocking::Client,
    last_call: Mutex<Instant>,
    cache_dir: PathBuf,
    covers_dir: PathBuf,
    token_users: Mutex<HashMap<String, Value>>,
}

fn hash16(text: &str) -> String {
    sha1_smol::Sha1::from(text).digest().to_string()[..16].to_string()
}

fn fresh(path: &Path, days: u64) -> bool {
    let age = fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| SystemTime::now().duration_since(t).ok());

    age.is_some_and(|age| age < Duration::from_secs(days * 86400))
}

pub(crate) fn read_cached(path: &Path, days: u64) -> Option<Value> {
    if !fresh(path, days) {
        return None;
    }
    let text = fs::read_to_string(path).ok()?;
    serde_json::from_str(&text).ok()
}

fn has_length(raw: &Value) -> bool {
    raw.get("length_minutes").is_some()
        && raw["titles"]
            .as_array()
            .is_some_and(|titles| titles.first().is_none_or(|title| title.get("lang").is_some()))
}

fn vn_id(id: &str) -> String {
    let id = id.trim();
    if id.starts_with('v') {
        id.to_string()
    } else {
        format!("v{id}")
    }
}

impl Vndb {
    pub fn new(data_dir: &Path) -> Vndb {
        let http = reqwest::blocking::Client::builder()
            .user_agent(USER_AGENT)
            .timeout(Duration::from_secs(15))
            .build()
            .expect("http client");

        Vndb {
            http,
            last_call: Mutex::new(Instant::now() - MIN_INTERVAL),
            cache_dir: data_dir.join("cache").join("vndb"),
            covers_dir: data_dir.join("cache").join("covers"),
            token_users: Mutex::new(HashMap::new()),
        }
    }

    fn throttle(&self) {
        let mut last = self.last_call.lock().unwrap();
        let since = last.elapsed();
        if since < MIN_INTERVAL {
            std::thread::sleep(MIN_INTERVAL - since);
        }
        *last = Instant::now();
    }

    fn request(
        &self,
        method: Method,
        endpoint: &str,
        body: Option<&Value>,
        token: &str,
    ) -> Result<Value, String> {
        for attempt in 0..4u64 {
            self.throttle();

            let mut req = self
                .http
                .request(method.clone(), format!("{API}{endpoint}"));
            if let Some(body) = body {
                req = req.json(body);
            }
            if !token.is_empty() {
                req = req.header("Authorization", format!("Token {token}"));
            }

            let resp = match req.send() {
                Ok(resp) => resp,
                Err(e) if attempt == 3 => return Err(format!("network error: {e}")),
                Err(_) => {
                    std::thread::sleep(Duration::from_millis(1500 * (attempt + 1)));
                    continue;
                }
            };

            match resp.status().as_u16() {
                429 => std::thread::sleep(Duration::from_secs(2 * (attempt + 1))),
                401 => return Err("the VNDB token is invalid or was revoked".into()),
                code if code >= 400 => {
                    let detail: String =
                        resp.text().unwrap_or_default().chars().take(200).collect();
                    return Err(format!("HTTP {code}: {detail}"));
                }
                _ => {
                    let bytes = resp.bytes().map_err(|e| e.to_string())?;
                    if bytes.is_empty() {
                        return Ok(Value::Null);
                    }
                    return serde_json::from_slice(&bytes).map_err(|e| e.to_string());
                }
            }
        }
        Err("rate limited, giving up".into())
    }

    pub(crate) fn post(&self, endpoint: &str, body: &Value) -> Result<Value, String> {
        self.request(Method::POST, endpoint, Some(body), "")
    }

    pub fn search(&self, query: &str, limit: u32) -> Result<Vec<VnResult>, String> {
        let query = query.trim();
        if query.is_empty() {
            return Ok(vec![]);
        }

        let cache = self
            .cache_dir
            .join(format!("{}.json", hash16(&query.to_lowercase())));
        if let Some(Value::Array(raw)) = read_cached(&cache, 7) {
            if raw.iter().all(has_length) {
                return Ok(raw.iter().map(VnResult::from_api).collect());
            }
        }

        let body = json!({
            "filters": ["search", "=", query],
            "fields": FIELDS,
            "results": limit.clamp(1, 25),
            "sort": "searchrank",
        });
        let data = self.post("/vn", &body)?;
        let results = data["results"].as_array().cloned().unwrap_or_default();

        let _ = fs::create_dir_all(&self.cache_dir);
        let _ = fs::write(&cache, serde_json::to_string(&results).unwrap_or_default());

        Ok(results.iter().map(VnResult::from_api).collect())
    }

    pub fn cached_raw(&self, id: &str) -> Option<Value> {
        let cache = self.cache_dir.join(format!("id_{}.json", vn_id(id)));
        serde_json::from_str(&fs::read_to_string(cache).ok()?).ok()
    }

    pub fn get(&self, id: &str) -> Result<Option<VnResult>, String> {
        let id = vn_id(id);
        let cache = self.cache_dir.join(format!("id_{id}.json"));
        if let Some(raw) = read_cached(&cache, 7).filter(has_length) {
            return Ok(Some(VnResult::from_api(&raw)));
        }

        let body = json!({"filters": ["id", "=", id], "fields": FIELDS, "results": 1});
        let data = self.post("/vn", &body)?;
        let Some(first) = data["results"].get(0).cloned() else {
            return Ok(None);
        };

        let _ = fs::create_dir_all(&self.cache_dir);
        let _ = fs::write(&cache, first.to_string());
        Ok(Some(VnResult::from_api(&first)))
    }

    pub fn release_dates(&self, ids: &[String]) -> Result<Vec<(String, String)>, String> {
        let mut out = Vec::new();
        for chunk in ids.chunks(100) {
            let mut filters = vec![json!("or")];
            filters.extend(chunk.iter().map(|id| json!(["id", "=", vn_id(id)])));
            let filters = if chunk.len() == 1 {
                filters[1].clone()
            } else {
                Value::Array(filters)
            };
            let body = json!({"filters": filters, "fields": "released", "results": 100});
            let data = self.post("/vn", &body)?;
            for vn in data["results"].as_array().into_iter().flatten() {
                out.push((text(&vn["id"]), text(&vn["released"])));
            }
        }
        Ok(out)
    }

    pub fn selected_release_dates(&self, ids: &[String]) -> Result<HashMap<String, String>, String> {
        collect_selected_release_dates(ids, |body| self.post("/release", body))
    }

    pub fn description(&self, id: &str) -> Result<String, String> {
        let id = vn_id(id);
        let cache = self.cache_dir.join(format!("desc_{id}.json"));
        if let Some(raw) = read_cached(&cache, 30) {
            return Ok(text(&raw["description"]));
        }

        let body = json!({"filters": ["id", "=", id], "fields": "description", "results": 1});
        let data = self.post("/vn", &body)?;
        let first = data["results"].get(0).cloned().unwrap_or_else(|| json!({}));

        let _ = fs::create_dir_all(&self.cache_dir);
        let _ = fs::write(&cache, first.to_string());
        Ok(text(&first["description"]))
    }

    pub fn characters(&self, id: &str) -> Result<Value, String> {
        let id = vn_id(id);
        let cache = self.cache_dir.join(format!("chars_{id}.json"));
        if let Some(raw) = read_cached(&cache, 30) {
            return Ok(raw);
        }

        let body = json!({
            "filters": ["vn", "=", ["id", "=", id]],
            "fields": "id,name,original,description,image.url,image.sexual,image.violence,blood_type,height,weight,bust,waist,hips,age,birthday,sex,vns.id,vns.role,vns.spoiler",
            "results": 100,
            "sort": "name",
        });
        let data = self.post("/character", &body)?;
        let results = data["results"].clone();

        let _ = fs::create_dir_all(&self.cache_dir);
        let _ = fs::write(&cache, results.to_string());
        Ok(results)
    }

    pub fn releases(&self, id: &str) -> Result<ReleaseList, String> {
        let id = vn_id(id);
        let body = json!({
            "filters": ["id", "=", id],
            "fields": "olang",
            "results": 1,
        });
        let original_language = original_language(self.post("/vn", &body)?)?;
        let releases = collect_releases(&id, |page| {
            let body = json!({
                "filters": ["vn", "=", ["id", "=", id]],
                "fields": "title,alttitle,released,official,patch,platforms,languages.lang,languages.title,media.medium,vns.id,vns.rtype,images.url,images.type,images.sexual,images.violence",
                "results": 100,
                "page": page,
                "sort": "released",
            });
            serde_json::from_value(self.post("/release", &body)?)
                .map_err(|error| format!("couldn't read VNDB releases: {error}"))
        })?;
        Ok(ReleaseList { original_language, releases })
    }

    pub fn release_covers(&self, id: &str) -> Result<Vec<ReleaseCover>, String> {
        let id = vn_id(id);
        let cache = self.cache_dir.join(format!("releases_{id}.json"));

        let releases = match read_cached(&cache, 7) {
            Some(Value::Array(raw)) => raw,
            _ => {
                let body = json!({
                    "filters": ["vn", "=", ["id", "=", id]],
                    "fields": "id,title,released,images.url,images.type,images.sexual,images.violence",
                    "results": 100,
                    "sort": "released",
                });
                let data = self.post("/release", &body)?;
                let results = data["results"].as_array().cloned().unwrap_or_default();

                let _ = fs::create_dir_all(&self.cache_dir);
                let _ = fs::write(&cache, serde_json::to_string(&results).unwrap_or_default());
                results
            }
        };

        let mut covers = Vec::new();
        for release in &releases {
            for image in release["images"].as_array().into_iter().flatten() {
                let url = text(&image["url"]);
                if url.is_empty() {
                    continue;
                }
                covers.push(ReleaseCover {
                    url,
                    kind: text(&image["type"]),
                    release_title: text(&release["title"]),
                    nsfw: is_nsfw(image),
                });
            }
        }
        Ok(covers)
    }

    pub fn cover_path(&self, stem: &str, image_url: &str) -> Option<PathBuf> {
        if image_url.is_empty() {
            return None;
        }

        let file_name = image_url.rsplit('/').next().unwrap_or("");
        let ext = file_name
            .rsplit_once('.')
            .map(|(_, ext)| ext.split('?').next().unwrap_or("jpg"))
            .unwrap_or("jpg");
        let ext: String = ext.chars().take(4).collect();

        let dest = self.covers_dir.join(format!("{stem}.{ext}"));
        if fs::metadata(&dest).map(|m| m.len() > 0).unwrap_or(false) {
            return Some(dest);
        }

        let bytes = self.download(image_url).ok()?;
        let _ = fs::create_dir_all(&self.covers_dir);
        fs::write(&dest, &bytes).ok()?;
        Some(dest)
    }

    pub fn download(&self, url: &str) -> Result<Vec<u8>, String> {
        self.throttle();
        let resp = self
            .http
            .get(url)
            .timeout(Duration::from_secs(20))
            .send()
            .and_then(|r| r.error_for_status())
            .map_err(|e| e.to_string())?;
        resp.bytes().map(|b| b.to_vec()).map_err(|e| e.to_string())
    }

    pub fn token_user(&self, token: &str) -> Result<Value, String> {
        if let Some(user) = self.token_users.lock().unwrap().get(token) {
            return Ok(user.clone());
        }
        let user = self.request(Method::GET, "/authinfo", None, token)?;
        self.token_users
            .lock()
            .unwrap()
            .insert(token.to_string(), user.clone());
        Ok(user)
    }

    pub fn list_user(&self, token: &str) -> Result<Value, String> {
        let user = self.token_user(token)?;
        let can_write = user["permissions"]
            .as_array()
            .is_some_and(|perms| perms.iter().any(|p| p == "listwrite"));

        if can_write {
            Ok(user)
        } else {
            Err("this token can't edit your list (it needs list write access)".into())
        }
    }

    pub fn list_entry(&self, token: &str, id: &str) -> Result<Option<Value>, String> {
        let user = self.list_user(token)?;
        let body = json!({
            "user": user["id"],
            "filters": ["id", "=", vn_id(id)],
            "fields": "labels.id,started,finished,vote",
        });
        let data = self.request(Method::POST, "/ulist", Some(&body), token)?;
        Ok(data["results"].get(0).cloned())
    }

    pub fn update_list_entry(&self, token: &str, id: &str, changes: &Value) -> Result<(), String> {
        let endpoint = format!("/ulist/{}", vn_id(id));
        self.request(Method::PATCH, &endpoint, Some(changes), token)?;
        Ok(())
    }

    pub fn list_votes(&self, token: &str) -> Result<Vec<(String, i64)>, String> {
        let user = self.token_user(token)?;
        let mut out = Vec::new();
        for page in 1..=50 {
            let body = json!({
                "user": user["id"],
                "fields": "vote",
                "sort": "id",
                "results": 100,
                "page": page,
            });
            let data = self.request(Method::POST, "/ulist", Some(&body), token)?;
            for entry in data["results"].as_array().into_iter().flatten() {
                out.push((text(&entry["id"]), entry["vote"].as_i64().unwrap_or(0)));
            }
            if data["more"].as_bool() != Some(true) {
                break;
            }
        }
        Ok(out)
    }

    pub fn wishlist(&self, token: &str) -> Result<Vec<VnResult>, String> {
        self.label_list(token, WISHLIST_LABEL)
    }

    pub fn label_list(&self, token: &str, label: i64) -> Result<Vec<VnResult>, String> {
        let list = self.labelled(token, label, "")?;
        Ok(list.into_iter().map(|(vn, _)| vn).collect())
    }

    pub fn labels(&self, token: &str) -> Result<Vec<Value>, String> {
        let user = self.token_user(token)?;
        let endpoint = format!("/ulist_labels?user={}&fields=count", text(&user["id"]));
        let data = self.request(Method::GET, &endpoint, None, token)?;
        Ok(data["labels"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|label| label["id"].as_i64().is_some())
            .map(|label| {
                json!({
                    "id": label["id"],
                    "label": text(&label["label"]),
                    "private": label["private"].as_bool().unwrap_or(false),
                    "count": label["count"].as_i64().unwrap_or(0),
                })
            })
            .collect())
    }

    pub fn whole_list(&self, token: &str) -> Result<Vec<(VnResult, Value)>, String> {
        self.labelled(token, 0, "labels.id,started,finished,vote")
    }

    pub fn length_votes(&self, uid: &str) -> Result<HashMap<String, i64>, String> {
        if !uid.starts_with('u') || !uid[1..].chars().all(|c| c.is_ascii_digit()) {
            return Err("unknown VNDB user".into());
        }
        let mut out = HashMap::new();
        for page in 1..=40 {
            let bytes = match self.download(&format!("https://vndb.org/{uid}/lengthvotes?p={page}")) {
                Ok(bytes) => bytes,
                Err(e) if page == 1 => return Err(e),
                Err(_) => break,
            };
            let html = String::from_utf8_lossy(&bytes);
            let votes = parse_length_votes(&html);
            let more = ["\"", ";", "&"].iter().any(|end| html.contains(&format!("p={}{end}", page + 1)));
            let empty = votes.is_empty();
            for (id, minutes) in votes {
                out.entry(id).or_insert(minutes);
            }
            if empty || !more {
                break;
            }
        }
        Ok(out)
    }

    fn labelled(
        &self,
        token: &str,
        label: i64,
        extra: &str,
    ) -> Result<Vec<(VnResult, Value)>, String> {
        let user = self.token_user(token)?;
        let mut fields: Vec<String> = FIELDS.split(',').map(|f| format!("vn.{f}")).collect();
        if !extra.is_empty() {
            fields.push(extra.to_string());
        }
        let fields = fields.join(",");

        let mut out = Vec::new();
        for page in 1..=50 {
            let mut body = json!({
                "user": user["id"],
                "fields": fields,
                "sort": "id",
                "results": 100,
                "page": page,
            });
            if label > 0 {
                body["filters"] = json!(["label", "=", label]);
            }
            let data = self.request(Method::POST, "/ulist", Some(&body), token)?;

            for entry in data["results"].as_array().into_iter().flatten() {
                let mut result = VnResult::from_api(&entry["vn"]);
                result.id = text(&entry["id"]);
                out.push((result, entry.clone()));
            }

            if data["more"].as_bool() != Some(true) {
                break;
            }
        }
        Ok(out)
    }
}

pub fn parse_length_votes(html: &str) -> Vec<(String, i64)> {
    let Some(table) = html.split("<table class=\"stripe\">").nth(1) else {
        return Vec::new();
    };
    let table = table.split("</table>").next().unwrap_or("");
    let mut out = Vec::new();
    for row in table.split("<tr>").skip(1) {
        let Some(start) = row.find("<a href=\"/v") else { continue };
        let digits: String = row[start + 11..].chars().take_while(char::is_ascii_digit).collect();
        if digits.is_empty() {
            continue;
        }
        let Some(cell) = row.split("<td class=\"tc3\">").nth(1) else { continue };
        let cell = cell.split("</td>").next().unwrap_or("");
        let plain: String = {
            let mut text = String::new();
            let mut tag = false;
            for c in cell.chars() {
                match c {
                    '<' => tag = true,
                    '>' => tag = false,
                    _ if !tag => text.push(c),
                    _ => {}
                }
            }
            text
        };
        let mut minutes = 0;
        let mut number = String::new();
        for c in plain.chars() {
            if c.is_ascii_digit() {
                number.push(c);
            } else if c == 'h' || c == 'm' {
                let value: i64 = number.parse().unwrap_or(0);
                minutes += if c == 'h' { value * 60 } else { value };
                number.clear();
            } else {
                number.clear();
            }
        }
        if minutes > 0 {
            out.push((format!("v{digits}"), minutes));
        }
    }
    out
}

#[derive(Default)]
pub struct ListDates {
    pub started: Option<String>,
    pub finished: Option<String>,
}

pub fn push_list_status(
    vndb: &Vndb,
    token: &str,
    id: &str,
    status: &str,
    keep_existing: bool,
    dates: &ListDates,
) -> Result<String, String> {
    let entry = vndb.list_entry(token, id)?.unwrap_or(Value::Null);
    let label_ids: Vec<i64> = entry["labels"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|label| label["id"].as_i64())
        .collect();

    let current = LIST_LABELS
        .iter()
        .find(|(_, label)| label_ids.contains(label))
        .map(|(name, _)| name.to_string())
        .unwrap_or_default();

    if current == status || (!current.is_empty() && keep_existing) {
        return Ok(current);
    }

    let all_labels: Vec<i64> = LIST_LABELS.iter().map(|(_, label)| *label).collect();
    if status.is_empty() {
        vndb.update_list_entry(token, id, &json!({"labels_unset": all_labels}))?;
        return Ok(String::new());
    }

    let Some((_, label)) = LIST_LABELS.iter().find(|(name, _)| *name == status) else {
        return Err(format!("unknown status {status}"));
    };
    let mut others: Vec<i64> = all_labels.iter().copied().filter(|l| l != label).collect();
    others.push(WISHLIST_LABEL);

    let mut changes = json!({"labels_set": [label], "labels_unset": others});
    let today = chrono::Local::now().format("%Y-%m-%d").to_string();
    if entry["started"].is_null() {
        changes["started"] = json!(dates.started.clone().unwrap_or_else(|| today.clone()));
    }
    if status == "finished" && entry["finished"].is_null() {
        changes["finished"] = json!(dates.finished.clone().unwrap_or(today));
    }

    vndb.update_list_entry(token, id, &changes)?;
    Ok(status.to_string())
}

pub fn similarity(a: &str, b: &str) -> f64 {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }
    2.0 * matching_chars(&a, &b) as f64 / (a.len() + b.len()) as f64
}

fn matching_chars(a: &[char], b: &[char]) -> usize {
    let (mut best, mut a_start, mut b_start) = (0, 0, 0);
    let mut previous = vec![0usize; b.len() + 1];

    for i in 0..a.len() {
        let mut current = vec![0usize; b.len() + 1];
        for j in 0..b.len() {
            if a[i] == b[j] {
                current[j + 1] = previous[j] + 1;
                if current[j + 1] > best {
                    best = current[j + 1];
                    a_start = i + 1 - best;
                    b_start = j + 1 - best;
                }
            }
        }
        previous = current;
    }

    if best == 0 {
        return 0;
    }

    let left = matching_chars(&a[..a_start], &b[..b_start]);
    let right = matching_chars(&a[a_start + best..], &b[b_start + best..]);
    best + left + right
}

pub fn best_match(results: Vec<VnResult>, query: &str) -> Option<VnResult> {
    use crate::title_parser::loose;

    let q = loose(query);
    if q.is_empty() {
        return None;
    }

    let mut best: Option<(f64, &VnResult)> = None;
    for vn in &results {
        let score = [&vn.title, &vn.alt_title]
            .into_iter()
            .chain(&vn.other_titles)
            .map(|title| similarity(&q, &loose(title)))
            .fold(0.0, f64::max);
        if score > best.map_or(0.0, |(s, _)| s) {
            best = Some((score, vn));
        }
    }

    best.filter(|(score, _)| *score >= 0.6).map(|(_, vn)| vn.clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_length_votes_from_the_vndb_page() {
        let html = r#"<table class="stripe"><thead><tr><td>Date</td></tr></thead><tr><td class="tc1">2025-06-03</td><td class="tc2"><a href="/v47172" lang="zh-Latn">Epiao</a></td><td class="tc3">9h<span class="small">14m</span></td></tr><tr><td class="tc1">2025-01-01</td><td class="tc2"><a href="/v4">Clannad</a></td><td class="tc3"><span class="small">45m</span></td></tr></table>"#;
        assert_eq!(parse_length_votes(html), vec![("v47172".to_string(), 554), ("v4".to_string(), 45)]);
    }

    fn vn(id: &str, title: &str, rating: f64) -> VnResult {
        VnResult { id: id.into(), title: title.into(), rating, ..Default::default() }
    }

    #[test]
    fn no_guess_without_a_close_title() {
        let results = vec![vn("v1", "Eternum", 9.0), vn("v2", "D.S. -Dal Segno-", 7.0)];
        assert!(best_match(results.clone(), "Some Unknown Game").is_none());
        assert_eq!(best_match(results, "Eternum").map(|v| v.id), Some("v1".into()));
    }

    #[test]
    fn english_title_is_the_official_english_one() {
        let vn = json!({"titles": [
            {"lang": "ja", "title": "抜きゲーみたいな島", "official": true},
            {"lang": "en", "title": "Fan TL", "official": false},
            {"lang": "en", "title": "NUKITASHI", "official": true}
        ]});
        assert_eq!(english_title(&vn), "NUKITASHI");
        assert_eq!(english_title(&json!({"titles": [{"lang": "ja", "title": "x", "official": true}]})), "");
        assert!(!has_length(&json!({"length_minutes": 1, "titles": [{"title": "x"}]})));
        assert!(has_length(&json!({"length_minutes": 1, "titles": [{"title": "x", "lang": "ja"}]})));
    }

    #[test]
    fn other_titles_count() {
        let mut entry = vn("v3", "Romanized Name", 8.0);
        entry.other_titles = vec!["素晴らしき日々".into()];
        assert_eq!(best_match(vec![entry], "素晴らしき日々").map(|v| v.id), Some("v3".into()));
    }

    fn release(id: &str, rtype: &str) -> Release {
        serde_json::from_value(json!({
            "id": id, "title": "Title", "alttitle": null, "released": "2022-10-28",
            "official": true, "patch": false, "platforms": ["win"],
            "languages": [{"lang": "ja", "title": "Japanese title"}],
            "media": [{"medium": "dvd"}],
            "vns": [{"id": "v1", "rtype": rtype}], "images": []
        })).unwrap()
    }

    #[test]
    fn original_language_handles_unknown_and_invalid_responses() {
        assert_eq!(original_language(json!({"results": [{"olang": "en"}]})).unwrap(), Some("en".into()));
        assert!(original_language(json!({"results": []})).is_err());
        assert_eq!(original_language(json!({"results": [{"olang": ""}]})).unwrap(), None);
        assert_eq!(original_language(json!({"results": [{"olang": null}]})).unwrap(), None);
        assert!(original_language(json!({"results": [{"olang": 1}]})).is_err());
    }

    #[test]
    fn selected_release_dates_skip_missing_ids_and_fetch_later_chunks() {
        let ids: Vec<_> = (1..=101).map(|id| format!("r{id}")).collect();
        let mut requests = 0;
        let dates = collect_selected_release_dates(&ids, |_| {
            requests += 1;
            Ok(if requests == 1 {
                json!({"results": [{"id": "r1", "released": "2022-10-28"}]})
            } else {
                json!({"results": [{"id": "r101", "released": "TBA"}]})
            })
        }).unwrap();
        assert_eq!(requests, 2);
        assert_eq!(dates.get("r1").map(String::as_str), Some("2022-10-28"));
        assert_eq!(dates.get("r101").map(String::as_str), Some("TBA"));
        assert!(!dates.contains_key("r2"));
    }

    #[test]
    fn selected_release_dates_report_invalid_responses_and_request_errors() {
        let ids = vec!["r1".to_string()];
        assert!(collect_selected_release_dates(&ids, |_| Err("network error".into())).is_err());
        assert!(collect_selected_release_dates(&ids, |_| Ok(json!({}))).is_err());
        assert!(collect_selected_release_dates(&ids, |_| Ok(json!({"results": [{"released": "TBA"}]}))).is_err());
        assert!(collect_selected_release_dates(&ids, |_| Ok(json!({"results": [{"id": "r1", "released": null}]}))).is_err());
    }

    #[test]
    fn release_pages_are_all_collected_and_failures_are_reported() {
        let mut pages = Vec::new();
        let results = collect_releases("v1", |page| {
            pages.push(page);
            Ok(ReleasePage {
                results: vec![release(if page == 1 { "r1" } else { "r2" }, if page == 1 { "partial" } else { "trial" })],
                more: page == 1,
            })
        }).unwrap();
        assert_eq!(pages, [1, 2]);
        assert_eq!(results.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(), ["r1", "r2"]);
        assert!(collect_releases("v1", |page| {
            if page == 1 {
                Ok(ReleasePage { results: vec![release("r1", "complete")], more: true })
            } else {
                Err("page failed".into())
            }
        }).is_err());
        assert!(collect_releases("v1", |_| {
            Ok(ReleasePage { results: vec![], more: true })
        }).is_err());
        assert!(collect_releases("v2", |_| {
            Ok(ReleasePage { results: vec![release("r1", "complete")], more: false })
        }).is_err());
    }

    #[test]
    fn release_prefers_original_name_and_front_cover() {
        let mut item = release("r1", "partial");
        item.images = serde_json::from_value(json!([
            {"url": "back", "type": "pkgback", "sexual": 0, "violence": 0},
            {"url": "digital", "type": "dig", "sexual": 0, "violence": 0},
            {"url": "front", "type": "pkgfront", "sexual": 1, "violence": 0}
        ])).unwrap();
        assert_eq!(item.alternate_title(Some("ja")), "Japanese title");
        assert_eq!(item.alternate_title(Some("en")), "");
        item.languages.push(ReleaseLanguage { lang: "en".into(), title: Some("English title".into()) });
        assert_eq!(item.alternate_title(Some("en")), "English title");
        item.languages.pop();
        item.alttitle = Some(" Alternative ".into());
        assert_eq!(item.alternate_title(Some("en")), "Alternative");
        item.alttitle = Some(" Title ".into());
        assert_eq!(item.alternate_title(None), "");
        assert_eq!(item.cover().unwrap().url, "front");
        assert!(item.cover().unwrap().nsfw());
        item.images.retain(|image| image.kind == "pkgback");
        assert!(item.cover().is_none());
    }

    #[test]
    #[ignore = "requires access to the VNDB API"]
    fn live_releases_include_selected_release_and_cover() {
        let client = Vndb::new(&std::env::temp_dir());
        let response = client.post("/release", &json!({
            "filters": ["id", ">=", "r1"],
            "fields": "vns.id,images.url,images.type",
            "results": 100
        })).unwrap();
        let candidate = response["results"].as_array().unwrap().iter().find(|release| {
            release["vns"].as_array().is_some_and(|vns| !vns.is_empty())
                && release["images"].as_array().is_some_and(|images| images.iter().any(|image| {
                    matches!(image["type"].as_str(), Some("pkgfront" | "dig"))
                        && image["url"].as_str().is_some_and(|url| !url.is_empty())
                }))
        }).expect("VNDB returned no release with a VN and cover");
        let vn_id = candidate["vns"][0]["id"].as_str().unwrap();
        let release_id = candidate["id"].as_str().unwrap();
        let result = client.releases(vn_id).unwrap();
        let selected = result.releases.iter().find(|item| item.id == release_id).unwrap();
        assert!(selected.cover().is_some());
        let dates = client.selected_release_dates(&[release_id.to_string()]).unwrap();
        assert_eq!(dates[release_id], selected.released);
    }
}
