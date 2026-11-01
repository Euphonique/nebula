//! Upload history: before Nebula overwrites, deletes or renames anything on a
//! server, the previous state is kept locally, so a whole operation (a save,
//! an upload, a publish, a sync) can be undone with one click.
//!
//! Kept per server below `journal\` in the settings folder: `index.json` lists
//! the operations, `blobs\` holds the earlier file contents (named by their
//! SHA-256, so the same content is stored once).

use std::path::PathBuf;
use std::sync::Mutex;

use anyhow::Result;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::{Backend, RemoteEntry, Stamp};

/// Larger files are not kept (the operation is then only partly undoable).
pub const MAX_FILE: u64 = 10 * 1024 * 1024;
/// The oldest operations of a server go when its copies get larger than this.
const MAX_SITE: u64 = 200 * 1024 * 1024;
/// Operations kept per server at most.
const MAX_ENTRIES: usize = 200;
/// Operations older than this are forgotten.
const MAX_AGE_MS: u64 = 30 * 24 * 3600 * 1000;
/// Saves of the same file within this time count as one operation.
const MERGE_SAVES_MS: u64 = 10 * 60 * 1000;

/// One process writes the index of a server at a time.
static LOCK: Mutex<()> = Mutex::new(());

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Kind {
    Save,
    Upload,
    Publish,
    Sync,
    Copy,
    Delete,
    Rename,
    Create,
    Undo,
}

/// What a file looked like before the operation.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Prior {
    /// It did not exist.
    Absent,
    /// Its content, by blob name.
    Kept(String),
    /// Too large to keep (size in bytes).
    TooLarge(u64),
    /// It existed but could not be read.
    Unreadable,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Change {
    Wrote { prior: Prior, after: Option<Stamp> },
    Removed { prior: Prior },
    Mkdir,
    Rmdir,
    Renamed { to: String },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Action {
    pub path: String,
    pub change: Change,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Entry {
    pub id: u64,
    /// Further ids that were merged into this entry (repeated saves).
    #[serde(default)]
    pub also: Vec<u64>,
    /// Milliseconds since 1970 of the last change.
    pub time: u64,
    pub kind: Kind,
    /// Main server path (file or folder), empty for a sync.
    pub name: String,
    pub actions: Vec<Action>,
    /// For [`Kind::Undo`]: the entry that was undone and its kind.
    #[serde(default)]
    pub reverts: Option<(u64, Kind)>,
    #[serde(default)]
    pub undone: bool,
}

impl Entry {
    pub fn has_id(&self, id: u64) -> bool {
        self.id == id || self.also.contains(&id)
    }

    /// Server files touched (not folders).
    pub fn file_count(&self) -> usize {
        let mut paths: Vec<&str> = self
            .actions
            .iter()
            .filter(|a| matches!(a.change, Change::Wrote { .. } | Change::Removed { .. }))
            .map(|a| a.path.as_str())
            .collect();
        paths.sort_unstable();
        paths.dedup();
        paths.len()
    }

    /// Files whose earlier state was not kept.
    pub fn incomplete(&self) -> usize {
        self.actions
            .iter()
            .filter(|a| matches!(&a.change, Change::Wrote { prior, .. } | Change::Removed { prior } if matches!(prior, Prior::TooLarge(_) | Prior::Unreadable)))
            .count()
    }
}

/// The kind of entry a request makes, `None` for requests that change nothing
/// on the server (or only permissions).
pub fn kind_of(request: &super::Request) -> Option<Kind> {
    use super::Request as R;
    Some(match request {
        R::Write { .. } => Kind::Save,
        R::Upload { .. } => Kind::Upload,
        R::Copy { .. } => Kind::Copy,
        R::Delete { .. } => Kind::Delete,
        R::Rename { .. } => Kind::Rename,
        R::CreateFile(_) | R::Mkdir(_) => Kind::Create,
        R::Sync(_) => Kind::Sync,
        R::Undo { .. } => Kind::Undo,
        R::List(_) | R::Read(_) | R::Download { .. } | R::Chmod { .. } | R::Scan { .. } | R::Compare(_) => return None,
    })
}

/// The server path an entry is named after.
pub fn name_of(request: &super::Request) -> String {
    use super::Request as R;
    match request {
        R::Write { path, .. } | R::Delete { path, .. } | R::CreateFile(path) | R::Mkdir(path) => path.clone(),
        R::Upload { remote, .. } => remote.clone(),
        R::Copy { to, .. } => to.clone(),
        R::Rename { from, .. } => from.clone(),
        _ => String::new(),
    }
}

fn now_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

fn root() -> PathBuf {
    crate::settings::config_dir().join("journal")
}

fn site_dir(site_id: &str) -> PathBuf {
    // Site ids are generated (letters, digits, dashes); keep anything else out.
    let safe: String = site_id.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_').collect();
    root().join(safe)
}

fn blob_path(site_id: &str, name: &str) -> PathBuf {
    site_dir(site_id).join("blobs").join(name)
}

fn load(site_id: &str) -> Vec<Entry> {
    std::fs::read(site_dir(site_id).join("index.json"))
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

fn save(site_id: &str, entries: &[Entry]) -> std::io::Result<()> {
    let dir = site_dir(site_id);
    std::fs::create_dir_all(&dir)?;
    let json = serde_json::to_vec(entries).map_err(std::io::Error::other)?;
    crate::safe_write::write(&dir.join("index.json"), &json)
}

/// Operations of a server, newest first.
pub fn list(site_id: &str) -> Vec<Entry> {
    let mut entries = load(site_id);
    entries.reverse();
    entries
}

pub fn get(site_id: &str, id: u64) -> Option<Entry> {
    load(site_id).into_iter().find(|e| e.has_id(id))
}

/// The kept content of a file.
pub fn read_blob(site_id: &str, name: &str) -> std::io::Result<Vec<u8>> {
    std::fs::read(blob_path(site_id, name))
}

fn store_blob(site_id: &str, data: &[u8]) -> std::io::Result<String> {
    let name = format!("{:x}", Sha256::digest(data));
    let path = blob_path(site_id, &name);
    if !path.exists() {
        std::fs::create_dir_all(path.parent().unwrap())?;
        crate::safe_write::write(&path, data)?;
    }
    Ok(name)
}

/// Adds the actions of an operation. Actions of an id that exists already
/// (a retried request, further files of a publish) are added to it; repeated
/// saves of one file shortly after each other become one entry.
pub fn commit(site_id: &str, id: u64, kind: Kind, name: String, actions: Vec<Action>, reverts: Option<(u64, Kind)>) {
    if actions.is_empty() {
        return;
    }
    let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut entries = load(site_id);
    let now = now_ms();
    let single_save = |e: &Entry, path: &str| e.kind == Kind::Save && e.actions.len() == 1 && e.actions[0].path == path;
    if let Some(e) = entries.iter_mut().rev().find(|e| e.has_id(id)) {
        merge_actions(&mut e.actions, actions);
        e.time = now;
    } else if kind == Kind::Save
        && actions.len() == 1
        && let Some(last) = entries.last_mut()
        && single_save(last, &actions[0].path)
        && !last.undone
        && now.saturating_sub(last.time) < MERGE_SAVES_MS
    {
        merge_actions(&mut last.actions, actions);
        last.also.push(id);
        last.time = now;
    } else {
        if let Some((undone, _)) = reverts {
            // Undoing an undo brings the original back.
            let again = entries.iter().find(|e| e.id == undone).and_then(|e| e.reverts).map(|(id, _)| id);
            for e in entries.iter_mut() {
                if e.id == undone {
                    e.undone = true;
                }
                if Some(e.id) == again {
                    e.undone = false;
                }
            }
        }
        entries.push(Entry { id, also: Vec::new(), time: now, kind, name, actions, reverts, undone: false });
    }
    prune(site_id, &mut entries, now);
    let _ = save(site_id, &entries);
}

/// A file written twice keeps its first earlier state and its last result.
fn merge_actions(into: &mut Vec<Action>, new: Vec<Action>) {
    for a in new {
        if let Change::Wrote { after, .. } = &a.change
            && let Some(old) = into.iter_mut().rev().find(|o| o.path == a.path)
            && let Change::Wrote { after: old_after, .. } = &mut old.change
        {
            *old_after = *after;
            continue;
        }
        into.push(a);
    }
}

/// Forgets old operations and removes copies nothing refers to any more.
fn prune(site_id: &str, entries: &mut Vec<Entry>, now: u64) {
    entries.retain(|e| now.saturating_sub(e.time) < MAX_AGE_MS);
    if entries.len() > MAX_ENTRIES {
        entries.drain(..entries.len() - MAX_ENTRIES);
    }
    let blobs = |entries: &[Entry]| -> std::collections::HashSet<String> {
        entries
            .iter()
            .flat_map(|e| &e.actions)
            .filter_map(|a| match &a.change {
                Change::Wrote { prior: Prior::Kept(b), .. } | Change::Removed { prior: Prior::Kept(b) } => Some(b.clone()),
                _ => None,
            })
            .collect()
    };
    let dir = site_dir(site_id).join("blobs");
    let sizes: std::collections::HashMap<String, u64> = std::fs::read_dir(&dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|f| Some((f.file_name().into_string().ok()?, f.metadata().ok()?.len())))
        .collect();
    // The oldest go until the rest fits (the newest always stays).
    loop {
        let used = blobs(entries);
        let total: u64 = used.iter().filter_map(|b| sizes.get(b)).sum();
        if total <= MAX_SITE || entries.len() <= 1 {
            break;
        }
        entries.remove(0);
    }
    let used = blobs(entries);
    for name in sizes.keys() {
        if !used.contains(name) {
            let _ = std::fs::remove_file(dir.join(name));
        }
    }
}

/// Size of everything kept, for the settings page.
pub fn total_size() -> u64 {
    fn size(dir: &std::path::Path) -> u64 {
        std::fs::read_dir(dir)
            .into_iter()
            .flatten()
            .flatten()
            .map(|e| match e.metadata() {
                Ok(m) if m.is_dir() => size(&e.path()),
                Ok(m) => m.len(),
                Err(_) => 0,
            })
            .sum()
    }
    size(&root())
}

pub fn clear() -> std::io::Result<()> {
    let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    match std::fs::remove_dir_all(root()) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
        _ => Ok(()),
    }
}

/// Earlier state read before a change, not yet stored.
enum Before {
    Absent,
    Data(Vec<u8>),
    TooLarge(u64),
    Unreadable,
}

/// A backend that notes the earlier state of everything it changes.
pub struct Recorder<'a> {
    inner: &'a mut dyn Backend,
    site_id: String,
    pub actions: Vec<Action>,
    /// The file read last (backups read the file just before overwriting it).
    last_read: Option<(String, Vec<u8>)>,
}

impl<'a> Recorder<'a> {
    pub fn new(inner: &'a mut dyn Backend, site_id: &str) -> Self {
        Self { inner, site_id: site_id.to_string(), actions: Vec::new(), last_read: None }
    }

    fn seen(&self, path: &str) -> bool {
        self.actions.iter().any(|a| a.path == path && matches!(a.change, Change::Wrote { .. } | Change::Removed { .. }))
    }

    fn before(&mut self, path: &str) -> Result<Before> {
        if let Some((p, data)) = &self.last_read
            && p == path
        {
            return Ok(Before::Data(data.clone()));
        }
        Ok(match self.inner.stat(path) {
            Ok(Some(s)) if s.size > MAX_FILE => Before::TooLarge(s.size),
            Ok(Some(_)) => match self.inner.read(path) {
                Ok(data) => Before::Data(data),
                Err(e) if super::is_connection_error(&e) => return Err(e),
                Err(_) => Before::Unreadable,
            },
            _ => Before::Absent,
        })
    }

    fn keep(&self, before: Before) -> Prior {
        match before {
            Before::Absent => Prior::Absent,
            Before::Data(data) => store_blob(&self.site_id, &data).map(Prior::Kept).unwrap_or(Prior::Unreadable),
            Before::TooLarge(n) => Prior::TooLarge(n),
            Before::Unreadable => Prior::Unreadable,
        }
    }

    fn forget_read(&mut self, path: &str) {
        if self.last_read.as_ref().is_some_and(|(p, _)| p == path) {
            self.last_read = None;
        }
    }
}

impl Backend for Recorder<'_> {
    fn home(&mut self) -> Result<String> {
        self.inner.home()
    }

    fn list(&mut self, path: &str) -> Result<Vec<RemoteEntry>> {
        self.inner.list(path)
    }

    fn read(&mut self, path: &str) -> Result<Vec<u8>> {
        let data = self.inner.read(path)?;
        if data.len() as u64 <= MAX_FILE {
            self.last_read = Some((path.to_string(), data.clone()));
        }
        Ok(data)
    }

    fn write(&mut self, path: &str, data: &[u8]) -> Result<()> {
        // Server backups (`.bak`) are not worth undoing.
        if path.ends_with(".bak") {
            return self.inner.write(path, data);
        }
        if self.seen(path) {
            self.forget_read(path);
            self.inner.write(path, data)?;
            let after = self.inner.stat(path).ok().flatten();
            merge_actions(&mut self.actions, vec![Action { path: path.to_string(), change: Change::Wrote { prior: Prior::Absent, after } }]);
            return Ok(());
        }
        let before = self.before(path)?;
        self.forget_read(path);
        self.inner.write(path, data)?;
        // Nothing changed: nothing to undo.
        if matches!(&before, Before::Data(old) if old == data) {
            return Ok(());
        }
        let after = self.inner.stat(path).ok().flatten();
        let prior = self.keep(before);
        self.actions.push(Action { path: path.to_string(), change: Change::Wrote { prior, after } });
        Ok(())
    }

    fn mkdir(&mut self, path: &str) -> Result<()> {
        self.inner.mkdir(path)?;
        self.actions.push(Action { path: path.to_string(), change: Change::Mkdir });
        Ok(())
    }

    fn remove_file(&mut self, path: &str) -> Result<()> {
        let before = if self.seen(path) { Before::Absent } else { self.before(path)? };
        self.forget_read(path);
        self.inner.remove_file(path)?;
        let prior = self.keep(before);
        self.actions.push(Action { path: path.to_string(), change: Change::Removed { prior } });
        Ok(())
    }

    fn remove_dir(&mut self, path: &str) -> Result<()> {
        self.inner.remove_dir(path)?;
        self.actions.push(Action { path: path.to_string(), change: Change::Rmdir });
        Ok(())
    }

    fn rename(&mut self, from: &str, to: &str) -> Result<()> {
        self.inner.rename(from, to)?;
        self.last_read = None;
        self.actions.push(Action { path: from.to_string(), change: Change::Renamed { to: to.to_string() } });
        Ok(())
    }

    fn chmod(&mut self, path: &str, mode: u32) -> Result<()> {
        self.inner.chmod(path, mode)
    }

    fn ping(&mut self) -> Result<()> {
        self.inner.ping()
    }

    fn stat(&mut self, path: &str) -> Result<Option<Stamp>> {
        self.inner.stat(path)
    }
}

/// Files changed on the server since the operation (by someone else, or by a
/// later operation): undoing would replace those changes too.
pub fn changed_since(b: &mut dyn Backend, entry: &Entry) -> Result<Vec<String>> {
    let mut out = Vec::new();
    let mut checked = std::collections::HashSet::new();
    for a in entry.actions.iter().rev() {
        if !checked.insert(a.path.as_str()) {
            continue;
        }
        let changed = match &a.change {
            Change::Wrote { after: Some(after), .. } => !b.stat(&a.path)?.is_some_and(|now| now.matches(after)),
            Change::Removed { .. } => b.stat(&a.path)?.is_some(),
            _ => false,
        };
        if changed {
            out.push(a.path.clone());
        }
    }
    Ok(out)
}

/// What undoing an operation did.
#[derive(Debug, Default)]
pub struct Undone {
    pub restored: usize,
    /// Files whose earlier state was not kept.
    pub skipped: Vec<String>,
    pub failed: Vec<String>,
    /// Every server path that changed (files and folders).
    pub paths: Vec<String>,
}

/// Restores the state before `entry`, last change first. Goes on after a
/// failed step, so as much as possible is restored.
pub fn undo(b: &mut dyn Backend, site_id: &str, entry: &Entry) -> Undone {
    let mut out = Undone::default();
    let mut done = std::collections::HashSet::new();
    // The first earlier state of a file is the one to go back to.
    let mut firsts: std::collections::HashMap<&str, &Prior> = std::collections::HashMap::new();
    for a in &entry.actions {
        if let Change::Wrote { prior, .. } | Change::Removed { prior } = &a.change {
            firsts.entry(a.path.as_str()).or_insert(prior);
        }
    }
    for a in entry.actions.iter().rev() {
        let path = a.path.as_str();
        let result = match &a.change {
            Change::Wrote { .. } | Change::Removed { .. } => {
                if !done.insert(path) {
                    continue;
                }
                match firsts[path] {
                    Prior::Absent => match b.remove_file(path) {
                        // Already gone is fine.
                        Err(_) if matches!(b.stat(path), Ok(None)) => Ok(()),
                        r => r,
                    },
                    Prior::Kept(blob) => match read_blob(site_id, blob) {
                        Ok(data) => super::write_creating(b, path, &data),
                        Err(e) => Err(anyhow::anyhow!(t!("Die gesicherte Fassung fehlt: {e}", e = e))),
                    },
                    Prior::TooLarge(_) | Prior::Unreadable => {
                        out.skipped.push(path.to_string());
                        continue;
                    }
                }
            }
            // A folder that still has files (from elsewhere) stays.
            Change::Mkdir => {
                let _ = b.remove_dir(path);
                out.paths.push(path.to_string());
                continue;
            }
            Change::Rmdir => match b.mkdir(path) {
                Err(_) if b.list(path).is_ok() => Ok(()),
                r => r,
            },
            Change::Renamed { to } => {
                out.paths.push(to.clone());
                b.rename(to, path)
            }
        };
        match result {
            Ok(()) => {
                if matches!(a.change, Change::Wrote { .. } | Change::Removed { .. }) {
                    out.restored += 1;
                }
                out.paths.push(path.to_string());
            }
            Err(e) => out.failed.push(format!("{path}: {e:#}")),
        }
    }
    out
}

#[cfg(test)]
mod tests;
