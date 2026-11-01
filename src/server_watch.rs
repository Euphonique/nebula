//! Change watcher for servers: the state of a server folder (size and time of
//! every file) is taken once; later scans report what changed without Nebula –
//! a colleague with FileZilla, a plugin update, or injected malicious code.
//! Nebula's own changes update the state as they happen.
//!
//! Kept per server as `watch\<site>.json` in the settings folder.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

use crate::remote::ScannedFile;
use crate::remote::journal::{Action, Change};

/// Names skipped by default: folders that change on their own all the time.
pub const DEFAULT_EXCLUDE: &str = ".git, node_modules, cache, tmp, temp, sessions, logs, *.log, error_log, .well-known";

static LOCK: Mutex<()> = Mutex::new(());

/// Size and time of a file; `mtime: None` after Nebula's own change (the next
/// scan takes the time from the server's listing, whose precision differs).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Known {
    pub size: u64,
    pub mtime: Option<i64>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Baseline {
    /// Server folder that is watched.
    pub root: String,
    /// Names (or `*.ext` / `prefix*` patterns) left out, comma separated.
    pub exclude: String,
    /// Relative path → state.
    pub files: BTreeMap<String, Known>,
    /// When the state was taken and last checked (ms since 1970).
    pub taken: u64,
    pub checked: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Added,
    Modified,
    Removed,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Found {
    pub rel: String,
    pub kind: Kind,
    /// Time on the server (seconds since 1970), if known.
    pub mtime: Option<i64>,
}

impl Found {
    /// Code that runs on the server or in browsers: worth a closer look.
    pub fn is_code(&self) -> bool {
        let lower = self.rel.to_lowercase();
        let name = lower.rsplit('/').next().unwrap_or(&lower);
        [".php", ".phtml", ".phar", ".js", ".cgi", ".pl", ".py", ".sh", ".asp", ".aspx", ".jsp"].iter().any(|e| name.ends_with(e))
            || name == ".htaccess"
            || name == ".user.ini"
            || name == "php.ini"
    }
}

fn now_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

fn path(site_id: &str) -> PathBuf {
    let safe: String = site_id.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_').collect();
    crate::settings::config_dir().join("watch").join(format!("{safe}.json"))
}

pub fn load(site_id: &str) -> Option<Baseline> {
    std::fs::read(path(site_id)).ok().and_then(|b| serde_json::from_slice(&b).ok())
}

fn save(site_id: &str, b: &Baseline) -> std::io::Result<()> {
    let p = path(site_id);
    std::fs::create_dir_all(p.parent().unwrap())?;
    crate::safe_write::write(&p, &serde_json::to_vec(b).map_err(std::io::Error::other)?)
}

/// Switches the watcher off for a server.
pub fn remove(site_id: &str) {
    let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _ = std::fs::remove_file(path(site_id));
}

/// Whether a relative path is left out by the exclusion list.
pub fn excluded(exclude: &str, rel: &str) -> bool {
    let patterns: Vec<&str> = exclude.split(',').map(str::trim).filter(|p| !p.is_empty()).collect();
    rel.split('/').any(|part| {
        patterns.iter().any(|p| match (p.strip_prefix('*'), p.strip_suffix('*')) {
            (Some(suffix), _) => part.ends_with(suffix),
            (_, Some(prefix)) => part.starts_with(prefix),
            _ => part == *p,
        })
    })
}

fn known_of(files: &[ScannedFile], exclude: &str) -> BTreeMap<String, Known> {
    files.iter().filter(|f| !excluded(exclude, &f.rel)).map(|f| (f.rel.clone(), Known { size: f.size, mtime: f.mtime })).collect()
}

/// Takes the state of a scanned folder as the new reference.
pub fn take(site_id: &str, root: &str, exclude: &str, files: &[ScannedFile]) -> std::io::Result<Baseline> {
    let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let now = now_ms();
    let b = Baseline { root: root.to_string(), exclude: exclude.to_string(), files: known_of(files, exclude), taken: now, checked: now };
    save(site_id, &b)?;
    Ok(b)
}

/// Compares a scan with the reference. Files whose time Nebula left open
/// (own changes) take the time of this scan, as long as the size matches.
pub fn check(site_id: &str, files: &[ScannedFile]) -> Option<Vec<Found>> {
    let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut b = load(site_id)?;
    let now = known_of(files, &b.exclude);
    let mut found = Vec::new();
    for (rel, cur) in &now {
        match b.files.get_mut(rel) {
            None => found.push(Found { rel: rel.clone(), kind: Kind::Added, mtime: cur.mtime }),
            Some(old) if old.mtime.is_none() && old.size == cur.size => old.mtime = cur.mtime,
            Some(old) if old.size != cur.size || old.mtime != cur.mtime => found.push(Found { rel: rel.clone(), kind: Kind::Modified, mtime: cur.mtime }),
            Some(_) => {}
        }
    }
    for rel in b.files.keys() {
        if !now.contains_key(rel) {
            found.push(Found { rel: rel.clone(), kind: Kind::Removed, mtime: None });
        }
    }
    b.checked = now_ms();
    let _ = save(site_id, &b);
    Some(found)
}

/// Takes reported changes into the reference (the user knows them).
pub fn accept(site_id: &str, rels: &[String], files: &[ScannedFile]) {
    let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let Some(mut b) = load(site_id) else { return };
    let now = known_of(files, &b.exclude);
    for rel in rels {
        match now.get(rel) {
            Some(k) => b.files.insert(rel.clone(), *k),
            None => b.files.remove(rel),
        };
    }
    let _ = save(site_id, &b);
}

/// Notes Nebula's own changes on a server (from the worker), so they are
/// not reported.
pub fn note_own(site_id: &str, actions: &[Action]) {
    if actions.is_empty() {
        return;
    }
    let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let Some(mut b) = load(site_id) else { return };
    let base = format!("{}/", b.root.trim_end_matches('/'));
    let rel = |p: &str| p.strip_prefix(base.as_str()).map(str::to_string);
    for a in actions {
        match &a.change {
            Change::Wrote { after, .. } => {
                if let Some(r) = rel(&a.path) {
                    match after {
                        Some(s) => b.files.insert(r, Known { size: s.size, mtime: None }),
                        None => b.files.remove(&r),
                    };
                }
            }
            Change::Removed { .. } => {
                if let Some(r) = rel(&a.path) {
                    b.files.remove(&r);
                }
            }
            Change::Rmdir => {
                if let Some(r) = rel(&a.path) {
                    let prefix = format!("{r}/");
                    b.files.retain(|k, _| !k.starts_with(&prefix));
                }
            }
            Change::Renamed { to } => {
                let (Some(from), Some(to)) = (rel(&a.path), rel(to)) else { continue };
                // A file, or a folder with everything inside.
                let prefix = format!("{from}/");
                let moved: Vec<(String, Known)> = b.files.iter().filter(|(k, _)| **k == from || k.starts_with(&prefix)).map(|(k, v)| (k.clone(), *v)).collect();
                for (k, v) in moved {
                    b.files.remove(&k);
                    b.files.insert(format!("{to}{}", &k[from.len()..]), Known { mtime: None, ..v });
                }
            }
            Change::Mkdir => {}
        }
    }
    let _ = save(site_id, &b);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::remote::Stamp;

    fn f(rel: &str, size: u64, mtime: i64) -> ScannedFile {
        ScannedFile { rel: rel.into(), size, mtime: Some(mtime) }
    }

    #[test]
    fn reports_only_foreign_changes() {
        crate::remote::integration_tests::isolated_config();
        let site = "watch-test";
        let start = [f("index.php", 10, 100), f("a.css", 5, 100), f("cache/x", 1, 100), f("old.txt", 1, 100)];
        take(site, "/www", DEFAULT_EXCLUDE, &start).unwrap();
        // Nebula saves index.php (the listing later shows another time format).
        note_own(site, &[Action { path: "/www/index.php".into(), change: Change::Wrote { prior: crate::remote::journal::Prior::Absent, after: Some(Stamp { size: 12, mtime: Some(555) }) } }]);
        let later = [f("index.php", 12, 200), f("a.css", 6, 300), f("cache/x", 9, 300), f("shell.php", 3, 300)];
        let mut found = check(site, &later).unwrap();
        found.sort_by(|a, b| a.rel.cmp(&b.rel));
        let kinds: Vec<(&str, Kind)> = found.iter().map(|x| (x.rel.as_str(), x.kind)).collect();
        assert_eq!(kinds, [("a.css", Kind::Modified), ("old.txt", Kind::Removed), ("shell.php", Kind::Added)]);
        assert!(found.iter().find(|x| x.rel == "shell.php").unwrap().is_code());
        // Accepted changes are not reported again; index.php took the new time.
        let rels: Vec<String> = found.iter().map(|x| x.rel.clone()).collect();
        accept(site, &rels, &later);
        assert!(check(site, &later).unwrap().is_empty());
        let b = load(site).unwrap();
        assert_eq!(b.files["index.php"], Known { size: 12, mtime: Some(200) });
    }

    #[test]
    fn exclusion_patterns() {
        assert!(excluded(DEFAULT_EXCLUDE, "wp-content/cache/page.html"));
        assert!(excluded(DEFAULT_EXCLUDE, "logs/access"));
        assert!(excluded(DEFAULT_EXCLUDE, "app/debug.log"));
        assert!(excluded("tmp*", "tmp_upload/x"));
        assert!(!excluded(DEFAULT_EXCLUDE, "index.php"));
    }

    #[test]
    fn own_renames_and_deletes() {
        crate::remote::integration_tests::isolated_config();
        let site = "watch-rename";
        take(site, "/", "", &[f("d/a.txt", 1, 1), f("d/b.txt", 2, 1), f("x.txt", 3, 1)]).unwrap();
        note_own(site, &[
            Action { path: "/d".into(), change: Change::Renamed { to: "/e".into() } },
            Action { path: "/x.txt".into(), change: Change::Removed { prior: crate::remote::journal::Prior::Absent } },
        ]);
        assert!(check(site, &[f("e/a.txt", 1, 7), f("e/b.txt", 2, 7)]).unwrap().is_empty());
    }
}
