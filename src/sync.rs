//! Two-way synchronization between a local project folder and a server
//! folder: compares both sides with the state of the last synchronization
//! and proposes what to upload, download, delete or decide.

use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::remote::ScannedFile;

/// Names skipped unless the project says otherwise.
pub const DEFAULT_EXCLUDE: &[&str] = &[".git", "node_modules", ".env", ".DS_Store", ".idea", ".vscode", "Thumbs.db"];

/// A file of the local project, relative path with `/`.
#[derive(Clone, Debug)]
pub struct LocalFile {
    pub rel: String,
    pub size: u64,
    /// Seconds since the Unix epoch.
    pub mtime: u64,
}

/// Both sides of a file as they were after the last synchronization.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct SyncedState {
    pub local_size: u64,
    pub local_mtime: u64,
    pub remote_size: u64,
    pub remote_mtime: Option<i64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Upload,
    Download,
    DeleteRemote,
    DeleteLocal,
    /// Changed on both sides (or different on the first sync): the user decides.
    Conflict,
}

/// What a conflict is resolved with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Resolution {
    Skip,
    Upload,
    Download,
}

#[derive(Clone, Debug)]
pub struct Item {
    pub rel: String,
    pub kind: Kind,
    /// Carried out (for conflicts: `resolution` is not `Skip`).
    pub selected: bool,
    pub resolution: Resolution,
    /// The side the item suggests for a conflict (newer file).
    pub suggestion: Resolution,
    pub note: String,
}

impl Item {
    fn new(rel: &str, kind: Kind, selected: bool, note: &str) -> Self {
        Item { rel: rel.to_string(), kind, selected, resolution: Resolution::Skip, suggestion: Resolution::Skip, note: note.to_string() }
    }

    /// The step to carry out, if any.
    pub fn action(&self) -> Option<Kind> {
        match self.kind {
            Kind::Conflict => match self.resolution {
                Resolution::Skip => None,
                Resolution::Upload => Some(Kind::Upload),
                Resolution::Download => Some(Kind::Download),
            },
            k => self.selected.then_some(k),
        }
    }
}

/// Files of a local folder, skipping excluded names (folders and files).
/// Fails if something can't be read or there are more than `limit` files:
/// a missing file would look deleted (or new on the other side) and lead
/// to wrong deletions or overwrites.
pub fn scan_local(root: &Path, exclude: &[String], limit: usize) -> Result<Vec<LocalFile>, String> {
    let mut out = Vec::new();
    let mut stack = vec![(root.to_path_buf(), String::new())];
    while let Some((dir, rel)) = stack.pop() {
        let unreadable = |e: std::io::Error| t!("{path} konnte nicht gelesen werden: {e}", path = dir.display(), e = e);
        for e in std::fs::read_dir(&dir).map_err(unreadable)? {
            let e = e.map_err(unreadable)?;
            let name = e.file_name().to_string_lossy().into_owned();
            if exclude.iter().any(|x| *x == name) {
                continue;
            }
            let rel = if rel.is_empty() { name } else { format!("{rel}/{name}") };
            let meta = e.metadata().map_err(unreadable)?;
            if meta.is_dir() {
                stack.push((e.path(), rel));
            } else {
                let mtime = meta.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map_or(0, |d| d.as_secs());
                out.push(LocalFile { rel, size: meta.len(), mtime });
                if out.len() > limit {
                    return Err(t!("Das Projekt enthält mehr als {limit} Dateien – große Ordner (z. B. vendor) in die Ausschlussliste aufnehmen.", limit = limit));
                }
            }
        }
    }
    Ok(out)
}

/// Files on both sides that were never synchronized and have the same size:
/// only their contents tell whether they are the same file.
pub fn compare_candidates(local: &[LocalFile], remote: &[ScannedFile], last: &BTreeMap<String, SyncedState>) -> Vec<String> {
    let r: BTreeMap<&str, &ScannedFile> = remote.iter().map(|f| (f.rel.as_str(), f)).collect();
    local
        .iter()
        .filter(|lf| !last.contains_key(&lf.rel) && r.get(lf.rel.as_str()).is_some_and(|rf| rf.size == lf.size))
        .map(|lf| lf.rel.clone())
        .collect()
}

/// Compares both sides with the last synchronization and lists what to do.
/// `differing`: never synchronized files of equal size whose contents differ.
pub fn plan(local: &[LocalFile], remote: &[ScannedFile], last: &BTreeMap<String, SyncedState>, differing: &HashSet<String>) -> Vec<Item> {
    let l: BTreeMap<&str, &LocalFile> = local.iter().map(|f| (f.rel.as_str(), f)).collect();
    let r: BTreeMap<&str, &ScannedFile> = remote.iter().map(|f| (f.rel.as_str(), f)).collect();
    let all: BTreeSet<&str> = l.keys().chain(r.keys()).copied().chain(last.keys().map(String::as_str)).collect();
    let mut items = Vec::new();
    for rel in all {
        let (lf, rf, snap) = (l.get(rel), r.get(rel), last.get(rel));
        let local_changed = |f: &LocalFile, s: &SyncedState| f.size != s.local_size || f.mtime != s.local_mtime;
        let remote_changed = |f: &ScannedFile, s: &SyncedState| f.size != s.remote_size || f.mtime != s.remote_mtime;
        match (lf, rf, snap) {
            (Some(lf), Some(rf), Some(s)) => match (local_changed(lf, s), remote_changed(rf, s)) {
                (false, false) => {}
                (true, false) => items.push(Item::new(rel, Kind::Upload, true, t!("lokal geändert"))),
                (false, true) => items.push(Item::new(rel, Kind::Download, true, t!("auf dem Server geändert"))),
                (true, true) => items.push(conflict(rel, lf, rf, t!("auf beiden Seiten geändert"))),
            },
            (Some(lf), Some(rf), None) => {
                // First sync: equal size counts as the same file unless the
                // contents were compared and differ.
                if lf.size != rf.size || differing.contains(rel) {
                    items.push(conflict(rel, lf, rf, t!("unterschiedlich, noch nie abgeglichen")));
                }
            }
            (Some(lf), None, Some(s)) => {
                if local_changed(lf, s) {
                    items.push(Item::new(rel, Kind::Upload, false, t!("auf dem Server gelöscht, lokal geändert")));
                } else {
                    items.push(Item::new(rel, Kind::DeleteLocal, false, t!("auf dem Server gelöscht")));
                }
            }
            (None, Some(rf), Some(s)) => {
                if remote_changed(rf, s) {
                    items.push(Item::new(rel, Kind::Download, false, t!("lokal gelöscht, auf dem Server geändert")));
                } else {
                    items.push(Item::new(rel, Kind::DeleteRemote, false, t!("lokal gelöscht")));
                }
            }
            (Some(_), None, None) => items.push(Item::new(rel, Kind::Upload, true, "neu")),
            (None, Some(_), None) => items.push(Item::new(rel, Kind::Download, true, t!("neu auf dem Server"))),
            (None, None, _) => {}
        }
    }
    items
}

fn conflict(rel: &str, lf: &LocalFile, rf: &ScannedFile, note: &str) -> Item {
    let remote_newer = rf.mtime.is_some_and(|m| m > lf.mtime as i64);
    let mut item = Item::new(rel, Kind::Conflict, false, note);
    item.suggestion = if remote_newer { Resolution::Download } else { Resolution::Upload };
    let side = if remote_newer { t!("Server") } else { "lokal" };
    item.note = t!("{note} – neuer: {side}", note = note, side = side);
    item
}

/// The state to remember after a synchronization, from fresh scans of both
/// sides: files present on both sides that are in sync now or were just transferred.
pub fn snapshot(
    local: &[LocalFile],
    remote: &[ScannedFile],
    last: &BTreeMap<String, SyncedState>,
    transferred: &HashSet<String>,
    differing: &HashSet<String>,
) -> BTreeMap<String, SyncedState> {
    let open: HashSet<String> = plan(local, remote, last, differing).into_iter().map(|i| i.rel).collect();
    let r: BTreeMap<&str, &ScannedFile> = remote.iter().map(|f| (f.rel.as_str(), f)).collect();
    local
        .iter()
        .filter_map(|lf| {
            let rf = r.get(lf.rel.as_str())?;
            (transferred.contains(&lf.rel) || !open.contains(&lf.rel)).then(|| {
                (lf.rel.clone(), SyncedState { local_size: lf.size, local_mtime: lf.mtime, remote_size: rf.size, remote_mtime: rf.mtime })
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn l(rel: &str, size: u64, mtime: u64) -> LocalFile {
        LocalFile { rel: rel.into(), size, mtime }
    }
    fn r(rel: &str, size: u64, mtime: i64) -> ScannedFile {
        ScannedFile { rel: rel.into(), size, mtime: Some(mtime) }
    }
    fn kinds(items: &[Item]) -> Vec<(&str, Kind, bool)> {
        items.iter().map(|i| (i.rel.as_str(), i.kind, i.selected)).collect()
    }

    #[test]
    fn first_sync() {
        let local = [l("same.css", 10, 100), l("new.php", 5, 100), l("diff.js", 3, 100)];
        let remote = [r("same.css", 10, 50), r("server.txt", 7, 50), r("diff.js", 4, 200)];
        let none = HashSet::new();
        // Only same.css has the same size on both sides: its contents decide.
        assert_eq!(compare_candidates(&local, &remote, &BTreeMap::new()), ["same.css"]);
        let items = plan(&local, &remote, &BTreeMap::new(), &none);
        assert_eq!(
            kinds(&items),
            [("diff.js", Kind::Conflict, false), ("new.php", Kind::Upload, true), ("server.txt", Kind::Download, true)]
        );
        // The server copy is newer.
        assert_eq!(items[0].suggestion, Resolution::Download);
        // Same size, different contents: a conflict, and not remembered as in sync.
        let differing = HashSet::from(["same.css".to_string()]);
        let items = plan(&local, &remote, &BTreeMap::new(), &differing);
        assert!(items.iter().any(|i| i.rel == "same.css" && i.kind == Kind::Conflict));
        assert!(!snapshot(&local, &remote, &BTreeMap::new(), &none, &differing).contains_key("same.css"));
        assert!(snapshot(&local, &remote, &BTreeMap::new(), &none, &none).contains_key("same.css"));
    }

    #[test]
    fn changes_and_deletions_since_last_sync() {
        let st = |ls, lm, rs, rm| SyncedState { local_size: ls, local_mtime: lm, remote_size: rs, remote_mtime: Some(rm) };
        let mut last = BTreeMap::new();
        for name in ["a", "b", "c", "d", "e", "f"] {
            last.insert(name.to_string(), st(1, 10, 1, 20));
        }
        let local = [l("a", 1, 10), l("b", 2, 11), l("c", 1, 10), l("e", 1, 10), l("f", 3, 12)];
        let remote = [r("a", 1, 20), r("b", 1, 20), r("c", 5, 30), r("d", 1, 20), r("f", 4, 40)];
        let items = plan(&local, &remote, &last, &HashSet::new());
        assert_eq!(
            kinds(&items),
            [
                ("b", Kind::Upload, true),
                ("c", Kind::Download, true),
                ("d", Kind::DeleteRemote, false),
                ("e", Kind::DeleteLocal, false),
                ("f", Kind::Conflict, false),
            ]
        );
        // Nothing happens with a conflict until it is decided.
        assert_eq!(items[4].action(), None);
    }

    #[test]
    fn snapshot_after_sync() {
        // b was uploaded (so its server time changed); c is still undecided.
        let local = [l("a", 1, 10), l("b", 2, 11), l("c", 9, 12)];
        let remote = [r("a", 1, 20), r("b", 2, 99), r("c", 8, 13)];
        let snap = snapshot(&local, &remote, &BTreeMap::new(), &HashSet::from(["b".to_string()]), &HashSet::new());
        assert_eq!(snap.keys().collect::<Vec<_>>(), ["a", "b"]);
        assert_eq!(snap["b"].remote_mtime, Some(99));
        // Next time nothing is left to do for a and b.
        assert_eq!(kinds(&plan(&local, &remote, &snap, &HashSet::new())), [("c", Kind::Conflict, false)]);
    }

    #[test]
    fn local_scan_skips_excluded() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(tmp.path().join("node_modules/x")).unwrap();
        std::fs::create_dir_all(tmp.path().join("css")).unwrap();
        std::fs::write(tmp.path().join("node_modules/x/a.js"), "x").unwrap();
        std::fs::write(tmp.path().join("css/s.css"), "abc").unwrap();
        std::fs::write(tmp.path().join(".env"), "SECRET=1").unwrap();
        let exclude: Vec<String> = DEFAULT_EXCLUDE.iter().map(|s| s.to_string()).collect();
        let files = scan_local(tmp.path(), &exclude, 100).unwrap();
        assert_eq!(files.iter().map(|f| (f.rel.as_str(), f.size)).collect::<Vec<_>>(), [("css/s.css", 3)]);
        // Too many files: refused instead of a partial list.
        assert!(scan_local(tmp.path(), &[], 1).is_err());
    }
}
