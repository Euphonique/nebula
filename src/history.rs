//! Local history: earlier versions of a file, kept next to the settings.
//! A version is stored when a file is opened (the state before editing) and
//! on every save, so changes on a server can be undone even without Git.

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::editor::DocSource;
use crate::settings::config_dir;

/// Larger files are not kept.
pub const MAX_BYTES: usize = 5 * 1024 * 1024;
/// Saves this close together replace each other (autosave would flood the list).
const MERGE_MS: u64 = 30_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// Content when the file was opened or reloaded.
    Opened,
    Saved,
    /// Unsaved changes thrown away by reloading the file.
    Discarded,
}

impl Kind {
    fn tag(self) -> &'static str {
        match self {
            Kind::Opened => "o",
            Kind::Saved => "s",
            Kind::Discarded => "d",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Kind::Opened => "geladen",
            Kind::Saved => "gespeichert",
            Kind::Discarded => "verworfene Änderungen",
        }
    }
}

#[derive(Clone, Debug)]
pub struct Entry {
    /// Unix time in milliseconds.
    pub time: u64,
    pub kind: Kind,
    pub size: u64,
    file: PathBuf,
}

impl Entry {
    pub fn read(&self) -> std::io::Result<Vec<u8>> {
        std::fs::read(&self.file)
    }
}

fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or_default()
}

/// Stable across program versions (unlike `DefaultHasher`).
fn fnv1a(s: &str) -> u64 {
    s.bytes().fold(0xcbf2_9ce4_8422_2325, |h, b| (h ^ b as u64).wrapping_mul(0x0100_0000_01b3))
}

/// Readable identity of a file; `None` for unsaved documents.
fn key_of(source: &DocSource) -> Option<String> {
    match source {
        // Windows paths are case-insensitive.
        DocSource::Local(p) => Some(format!("local:{}", p.to_string_lossy().to_lowercase())),
        DocSource::Remote { site_id, path } => Some(format!("remote:{site_id}:{path}")),
        DocSource::Untitled(_) => None,
    }
}

fn base() -> PathBuf {
    config_dir().join("history")
}

fn dir_in(base: &Path, key: &str) -> PathBuf {
    base.join(format!("{:016x}", fnv1a(key)))
}

/// Versions of a file, newest first.
pub fn list(source: &DocSource) -> Vec<Entry> {
    key_of(source).map(|k| list_in(&dir_in(&base(), &k))).unwrap_or_default()
}

fn list_in(dir: &Path) -> Vec<Entry> {
    let Ok(rd) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut v: Vec<Entry> = rd
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            let (time, kind) = name.strip_suffix(".bin")?.split_once('-')?;
            let kind = [Kind::Opened, Kind::Saved, Kind::Discarded].into_iter().find(|k| k.tag() == kind)?;
            Some(Entry { time: time.parse().ok()?, kind, size: e.metadata().map(|m| m.len()).unwrap_or(0), file: e.path() })
        })
        .collect();
    v.sort_by(|a, b| b.time.cmp(&a.time));
    v
}

/// Stores `bytes` as a new version unless it equals the newest one; keeps at
/// most `max` versions.
pub fn record(source: &DocSource, bytes: &[u8], kind: Kind, max: usize) {
    if let Some(key) = key_of(source)
        && let Err(e) = record_in(&base(), &key, bytes, kind, max, now_ms())
    {
        eprintln!("Lokaler Verlauf: {e}");
    }
}

fn record_in(base: &Path, key: &str, bytes: &[u8], kind: Kind, max: usize, now: u64) -> std::io::Result<()> {
    if bytes.len() > MAX_BYTES || max == 0 {
        return Ok(());
    }
    let dir = dir_in(base, key);
    let entries = list_in(&dir);
    if let Some(newest) = entries.first() {
        if newest.size == bytes.len() as u64 && newest.read().is_ok_and(|b| b == bytes) {
            return Ok(());
        }
        // A quick follow-up save replaces the previous save (not the opened state).
        if kind == Kind::Saved && newest.kind == Kind::Saved && now.saturating_sub(newest.time) < MERGE_MS {
            let _ = std::fs::remove_file(&newest.file);
        }
    }
    std::fs::create_dir_all(&dir)?;
    // Which file this is, for people looking into the folder.
    let label = dir.join("source.txt");
    if !label.exists() {
        std::fs::write(&label, key)?;
    }
    // Never overwrite: two versions within the same millisecond get distinct names.
    let mut time = now;
    while list_in(&dir).iter().any(|e| e.time == time) {
        time += 1;
    }
    std::fs::write(dir.join(format!("{time}-{}.bin", kind.tag())), bytes)?;
    for old in list_in(&dir).into_iter().skip(max) {
        let _ = std::fs::remove_file(old.file);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn records_dedupes_merges_and_prunes() {
        let base = std::env::temp_dir().join(format!("nebula-history-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let key = "remote:s:/var/www/index.php";
        let dir = dir_in(&base, key);

        record_in(&base, key, b"v0", Kind::Opened, 3, 1_000).unwrap();
        record_in(&base, key, b"v0", Kind::Saved, 3, 2_000).unwrap(); // unchanged: skipped
        assert_eq!(list_in(&dir).len(), 1);

        record_in(&base, key, b"v1", Kind::Saved, 3, 3_000).unwrap();
        record_in(&base, key, b"v2", Kind::Saved, 3, 4_000).unwrap(); // within 30 s: replaces v1
        let v = list_in(&dir);
        assert_eq!(v.len(), 2);
        assert_eq!(v[0].read().unwrap(), b"v2");
        assert_eq!(v[1].kind, Kind::Opened);

        record_in(&base, key, b"v3", Kind::Saved, 3, 100_000).unwrap();
        record_in(&base, key, b"v4", Kind::Saved, 3, 200_000).unwrap();
        let v = list_in(&dir);
        assert_eq!(v.iter().map(|e| e.read().unwrap()).collect::<Vec<_>>(), [b"v4", b"v3", b"v2"]);
        assert_eq!(std::fs::read_to_string(dir.join("source.txt")).unwrap(), key);
        let _ = std::fs::remove_dir_all(&base);
    }
}
