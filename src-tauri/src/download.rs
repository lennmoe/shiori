use std::fs::File;
use std::io::{Read, Write};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use sha2::{Digest, Sha256};

const USER_AGENT: &str = concat!("ShioriRPC/", env!("CARGO_PKG_VERSION"));
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(3600);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
const CHUNK: usize = 64 * 1024;
pub const CANCELLED: &str = "cancelled";
pub const DAMAGED: &str = "The downloaded file is incomplete or corrupted.";

pub struct Download {
    pub url: &'static str,
    pub sha256: &'static str,
    pub size: u64,
}

pub fn client() -> Result<reqwest::blocking::Client, String> {
    reqwest::blocking::Client::builder()
        .user_agent(USER_AGENT)
        .connect_timeout(CONNECT_TIMEOUT)
        .timeout(DOWNLOAD_TIMEOUT)
        .build()
        .map_err(|e| e.to_string())
}

pub fn fetch(
    client: &reqwest::blocking::Client,
    item: &Download,
    part: &Path,
    stop: &AtomicBool,
    progress: impl FnMut(u64),
) -> Result<(), String> {
    let response = client
        .get(item.url)
        .send()
        .and_then(|r| r.error_for_status())
        .map_err(|e| e.to_string())?;
    let file = File::create(part).map_err(|e| e.to_string())?;
    let copied = copy_checked(response, file, item.sha256, stop, progress);
    if copied.is_err() {
        let _ = std::fs::remove_file(part);
    }
    copied
}

pub fn copy_checked(
    mut from: impl Read,
    mut to: impl Write,
    sha256: &str,
    stop: &AtomicBool,
    mut progress: impl FnMut(u64),
) -> Result<(), String> {
    let mut hash = Sha256::new();
    let mut buffer = vec![0; CHUNK];
    loop {
        if stop.load(Ordering::SeqCst) {
            return Err(CANCELLED.into());
        }
        let read = from.read(&mut buffer).map_err(|e| e.to_string())?;
        if read == 0 {
            break;
        }
        hash.update(&buffer[..read]);
        to.write_all(&buffer[..read]).map_err(|e| e.to_string())?;
        progress(read as u64);
    }
    let found: String = hash.finalize().iter().map(|b| format!("{b:02x}")).collect();
    if found != sha256 {
        return Err(DAMAGED.into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_wrong_hash_is_refused() {
        let mut out = Vec::new();
        let result = copy_checked(&b"hello"[..], &mut out, "00", &AtomicBool::new(false), |_| {});
        assert_eq!(result, Err(DAMAGED.to_string()));
    }

    #[test]
    fn the_right_hash_is_accepted() {
        let mut out = Vec::new();
        let mut seen = 0;
        let hello = "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824";
        let result = copy_checked(&b"hello"[..], &mut out, hello, &AtomicBool::new(false), |n| seen += n);
        assert_eq!(result, Ok(()));
        assert_eq!(out, b"hello");
        assert_eq!(seen, 5);
    }

    #[test]
    fn a_cancelled_download_stops() {
        let result = copy_checked(&b"hello"[..], Vec::new(), "00", &AtomicBool::new(true), |_| {});
        assert_eq!(result, Err(CANCELLED.to_string()));
    }
}
