//! Copies of server files and folder listings, so they stay readable when
//! the server cannot be reached. Kept per server below `offline\` in the
//! settings folder; names are hashes of the server path.

use std::path::PathBuf;
use std::time::SystemTime;

use crate::remote::RemoteEntry;

/// Larger files are not kept.
const MAX_FILE: usize = 10 * 1024 * 1024;
/// The oldest copies go when all together get larger than this.
const MAX_TOTAL: u64 = 500 * 1024 * 1024;

fn root() -> PathBuf {
    crate::settings::config_dir().join("offline")
}

fn site_dir(site_id: &str) -> PathBuf {
    // Site ids are generated (letters, digits, dashes); keep anything else out.
    let safe: String = site_id.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_').collect();
    root().join(safe)
}

/// FNV-1a of the path: stable across builds (unlike the std hasher).
fn key(path: &str) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in path.bytes() {
        h = (h ^ b as u64).wrapping_mul(0x0100_0000_01b3);
    }
    format!("{h:016x}")
}

fn file_path(site_id: &str, path: &str) -> PathBuf {
    site_dir(site_id).join("f").join(key(path))
}

fn listing_path(site_id: &str, dir: &str) -> PathBuf {
    site_dir(site_id).join("l").join(format!("{}.json", key(dir)))
}

fn write(target: PathBuf, data: Vec<u8>) {
    // In the background: the UI never waits for the disk.
    std::thread::spawn(move || {
        if let Some(dir) = target.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let _ = std::fs::write(&target, data);
    });
}

/// Keeps a copy of a server file (opened or saved).
pub fn store_file(site_id: &str, path: &str, data: &[u8]) {
    if data.len() <= MAX_FILE {
        write(file_path(site_id, path), data.to_vec());
    }
}

/// The kept copy of a server file and when it was stored.
pub fn load_file(site_id: &str, path: &str) -> Option<(Vec<u8>, SystemTime)> {
    let p = file_path(site_id, path);
    let when = std::fs::metadata(&p).and_then(|m| m.modified()).ok()?;
    Some((std::fs::read(&p).ok()?, when))
}

#[derive(serde::Serialize, serde::Deserialize)]
struct Listing {
    entries: Vec<RemoteEntry>,
}

pub fn store_listing(site_id: &str, dir: &str, entries: &[RemoteEntry]) {
    if let Ok(json) = serde_json::to_vec(&Listing { entries: entries.to_vec() }) {
        write(listing_path(site_id, dir), json);
    }
}

pub fn load_listing(site_id: &str, dir: &str) -> Option<Vec<RemoteEntry>> {
    let bytes = std::fs::read(listing_path(site_id, dir)).ok()?;
    serde_json::from_slice::<Listing>(&bytes).ok().map(|l| l.entries)
}

/// The start folder the server reported last.
pub fn store_home(site_id: &str, home: &str) {
    write(site_dir(site_id).join("home.txt"), home.as_bytes().to_vec());
}

/// The kept start folder and when it was stored (the age of the offline view).
pub fn load_home(site_id: &str) -> Option<(String, SystemTime)> {
    let p = site_dir(site_id).join("home.txt");
    let when = std::fs::metadata(&p).and_then(|m| m.modified()).ok()?;
    Some((std::fs::read_to_string(&p).ok()?, when))
}

/// "23.10.2026 14:05" in local time.
pub fn format_time(t: SystemTime) -> String {
    let ms = t.duration_since(SystemTime::UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64);
    let l = crate::win32::local_time(ms);
    format!("{:02}.{:02}.{} {:02}:{:02}", l.day, l.month, l.year, l.hour, l.minute)
}

/// Removes everything kept for a server (when it is removed).
pub fn forget_site(site_id: &str) {
    let _ = std::fs::remove_dir_all(site_dir(site_id));
}

/// Size of everything kept, in bytes.
pub fn total_size() -> u64 {
    files_under(&root()).iter().map(|(_, size, _)| size).sum()
}

pub fn clear() -> std::io::Result<()> {
    match std::fs::remove_dir_all(root()) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
        _ => Ok(()),
    }
}

/// Drops the oldest copies while everything together is too large.
pub fn prune() {
    prune_to(&root(), MAX_TOTAL);
}

fn prune_to(dir: &std::path::Path, max: u64) {
    let mut files = files_under(dir);
    let mut total: u64 = files.iter().map(|(_, size, _)| size).sum();
    files.sort_by_key(|(_, _, when)| *when);
    for (path, size, _) in files {
        if total <= max {
            break;
        }
        if std::fs::remove_file(&path).is_ok() {
            total -= size;
        }
    }
}

fn files_under(dir: &std::path::Path) -> Vec<(PathBuf, u64, SystemTime)> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&d) else { continue };
        for e in rd.flatten() {
            let Ok(meta) = e.metadata() else { continue };
            if meta.is_dir() {
                stack.push(e.path());
            } else {
                out.push((e.path(), meta.len(), meta.modified().unwrap_or(SystemTime::UNIX_EPOCH)));
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prune_drops_oldest_first() {
        let dir = tempfile::tempdir().unwrap();
        for (n, name) in ["alt", "mittel", "neu"].iter().enumerate() {
            let p = dir.path().join(name);
            std::fs::write(&p, vec![0u8; 100]).unwrap();
            let t = SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_000_000 + n as u64 * 1000);
            std::fs::File::options().write(true).open(&p).unwrap().set_modified(t).unwrap();
        }
        prune_to(dir.path(), 250);
        assert!(!dir.path().join("alt").exists());
        assert!(dir.path().join("mittel").exists() && dir.path().join("neu").exists());
    }
}
