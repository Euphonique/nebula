//! File operations from the sidebar: clipboard, copy/move between local
//! folders and servers, duplicate, delete and drops from Explorer.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};

use eframe::egui;

use super::{Dialog, FollowUp, NebulaApp};
use crate::editor::DocSource;
use crate::remote::{self, Request};
use crate::ui::sidebar::{DirRef, FileRef, Item, RemoteStatus};
use crate::win32;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ConflictPolicy {
    Replace,
    KeepBoth,
    Skip,
}

/// Pending copy/move operations; paused while a name conflict is asked about.
pub struct TransferQueue {
    pub items: VecDeque<(Item, DirRef)>,
    pub copy: bool,
    pub policy: Option<ConflictPolicy>,
}

impl NebulaApp {
    pub(super) fn set_clipboard(&mut self, items: Vec<Item>, cut: bool) {
        // Local files also go to the system clipboard so Explorer can paste them.
        let locals: Vec<PathBuf> = items
            .iter()
            .filter_map(|i| match &i.file {
                FileRef::Local(p) => Some(p.clone()),
                FileRef::Remote { .. } => None,
            })
            .collect();
        if !locals.is_empty() && locals.len() == items.len() {
            win32::set_clipboard_files(&locals);
        }
        let n = items.len();
        self.sidebar.clipboard = Some((items, cut, win32::clipboard_sequence()));
        self.info(format!(
            "{n} {} {}",
            if n == 1 { t!("Objekt") } else { t!("Objekte") },
            if cut { "ausgeschnitten" } else { "kopiert" }
        ));
    }

    pub(super) fn paste(&mut self, target: DirRef) {
        // Files copied in Explorer after our own copy take precedence.
        let seq = win32::clipboard_sequence();
        let external = win32::clipboard_files();
        let ours_current = self.sidebar.clipboard.as_ref().is_some_and(|(_, _, s)| *s == seq);
        if !external.is_empty() && !ours_current {
            let items = external.into_iter().map(|p| {
                let dir = p.is_dir();
                Item::local(p, dir)
            });
            self.transfer(items.collect(), target, true);
            return;
        }
        let Some((items, cut, _)) = self.sidebar.clipboard.clone() else { return };
        if cut {
            self.sidebar.clipboard = None;
        }
        self.transfer(items, target, !cut);
    }

    pub(super) fn duplicate(&mut self, items: Vec<Item>) {
        let queue = items
            .into_iter()
            .filter_map(|i| {
                let parent = i.parent()?;
                Some((i, parent))
            })
            .collect();
        self.run_transfers(TransferQueue { items: queue, copy: true, policy: Some(ConflictPolicy::KeepBoth) });
    }

    /// Copies or moves items into `target`, asking about name conflicts.
    pub(super) fn transfer(&mut self, items: Vec<Item>, target: DirRef, copy: bool) {
        let before = items.len();
        let items: VecDeque<(Item, DirRef)> = items
            .into_iter()
            .filter(|i| !target.is_inside(i))
            .map(|i| (i, target.clone()))
            .collect();
        if items.len() < before {
            self.error(t!("Ein Ordner kann nicht in sich selbst kopiert oder verschoben werden."));
        }
        self.sidebar.selection.clear();
        self.run_transfers(TransferQueue { items, copy, policy: None });
    }

    pub(super) fn run_transfers(&mut self, mut queue: TransferQueue) {
        while let Some((item, target)) = queue.items.pop_front() {
            let name = item.name();
            let same_dir = item.parent().as_ref() == Some(&target);
            if same_dir && !queue.copy {
                continue; // moving into its own folder changes nothing
            }
            // A server folder not listed yet: read it first, so existing
            // names are asked about instead of silently overwritten.
            if let DirRef::Remote { site_id, path } = &target
                && self.sidebar.remote_names(site_id, path).is_none()
            {
                let (site_id, path) = (site_id.clone(), path.clone());
                queue.items.push_front((item, target));
                if !self.sidebar.remote.get(&site_id).is_some_and(|t| t.loading.contains(&path)) {
                    self.remote_request(&site_id, Request::List(path));
                }
                self.waiting_transfers.push(queue);
                return;
            }
            let mut dest = name.clone();
            if same_dir {
                dest = self.unique_name(&target, &name);
            } else if self.target_has(&target, &name) {
                match queue.policy {
                    None => {
                        queue.items.push_front((item, target));
                        self.dialog = Some(Dialog::TransferConflict { name, queue, apply_all: false });
                        return;
                    }
                    Some(ConflictPolicy::Skip) => continue,
                    Some(ConflictPolicy::KeepBoth) => dest = self.unique_name(&target, &name),
                    Some(ConflictPolicy::Replace) => {}
                }
            }
            self.transfer_one(&item, &target, &dest, queue.copy);
        }
    }

    /// Continues queues that waited for a server folder listing.
    pub(super) fn resume_waiting_transfers(&mut self) {
        if self.waiting_transfers.is_empty() || self.dialog.is_some() {
            return;
        }
        for queue in std::mem::take(&mut self.waiting_transfers) {
            let Some((_, DirRef::Remote { site_id, path })) = queue.items.front() else { continue };
            let tree = self.sidebar.remote.get(site_id);
            if tree.is_none_or(|t| matches!(t.status, RemoteStatus::Failed(_))) {
                // The connection error was reported already.
                continue;
            }
            if self.sidebar.remote_names(site_id, path).is_some() && self.dialog.is_none() {
                self.run_transfers(queue);
            } else {
                self.waiting_transfers.push(queue);
            }
        }
    }

    /// Continues a paused queue after the user answered a conflict.
    pub(super) fn resolve_conflict(&mut self, mut queue: TransferQueue, policy: ConflictPolicy, all: bool) {
        if all {
            queue.policy = Some(policy);
            self.run_transfers(queue);
            return;
        }
        if let Some(first) = queue.items.pop_front() {
            self.run_transfers(TransferQueue {
                items: VecDeque::from([first]),
                copy: queue.copy,
                policy: Some(policy),
            });
        }
        self.run_transfers(queue);
    }

    /// Whether a listed server folder already has `name` (unknown: no).
    pub(super) fn remote_name_taken(&self, site_id: &str, dir: &str, name: &str) -> bool {
        self.sidebar.remote_names(site_id, dir).is_some_and(|names| names.iter().any(|n| n == name))
    }

    fn target_has(&self, target: &DirRef, name: &str) -> bool {
        match target {
            DirRef::Local(dir) => dir.join(name).exists(),
            DirRef::Remote { site_id, path } => self
                .sidebar
                .remote_names(site_id, path)
                .is_some_and(|names| names.iter().any(|n| n == name)),
        }
    }

    /// "name Kopie.ext", "name Kopie 2.ext", … that doesn't exist in `target`.
    fn unique_name(&self, target: &DirRef, name: &str) -> String {
        let (stem, ext) = match name.rfind('.') {
            Some(i) if i > 0 => (&name[..i], &name[i..]),
            _ => (name, ""),
        };
        let mut n = 1;
        loop {
            let candidate = if n == 1 { t!("{stem} Kopie{ext}", stem = stem, ext = ext) } else { t!("{stem} Kopie {n}{ext}", stem = stem, n = n, ext = ext) };
            if !self.target_has(target, &candidate) {
                return candidate;
            }
            n += 1;
        }
    }

    fn transfer_one(&mut self, item: &Item, target: &DirRef, dest_name: &str, copy: bool) {
        match (&item.file, target) {
            (FileRef::Local(src), DirRef::Local(dir)) => {
                let dest = dir.join(dest_name);
                let result = if copy {
                    copy_local(src, &dest)
                } else {
                    std::fs::rename(src, &dest).or_else(|_| {
                        copy_local(src, &dest)?;
                        remove_local(src)
                    })
                };
                match result {
                    Ok(()) => {
                        if !copy {
                            self.retarget_local_docs(src, &dest);
                        }
                        self.sidebar.refresh_local(Some(dir));
                        self.sidebar.refresh_local(src.parent());
                    }
                    Err(e) => self.error(t!("{src} konnte nicht übertragen werden: {e}", src = src.display(), e = e)),
                }
            }
            (FileRef::Local(src), DirRef::Remote { site_id, path }) => {
                let remote = remote::join(path, dest_name);
                let request = Request::Upload { local: src.clone(), remote };
                if let Some(tag) = self.send_remote(site_id, request)
                    && !copy
                {
                    self.pending_followups.insert(tag, FollowUp::DeleteLocal(src.clone()));
                }
            }
            (FileRef::Remote { site_id, path }, DirRef::Local(dir)) => {
                let request = Request::Download { remote: path.clone(), local: dir.join(dest_name), is_dir: item.is_dir };
                if let Some(tag) = self.send_remote(site_id, request)
                    && !copy
                {
                    self.pending_followups.insert(
                        tag,
                        FollowUp::DeleteRemote { site_id: site_id.clone(), path: path.clone(), is_dir: item.is_dir },
                    );
                }
            }
            (FileRef::Remote { site_id: from_site, path }, DirRef::Remote { site_id: to_site, path: dir })
                if from_site == to_site =>
            {
                let to = remote::join(dir, dest_name);
                if copy {
                    self.send_remote(from_site, Request::Copy { from: path.clone(), to, is_dir: item.is_dir });
                } else if self.target_has(target, dest_name) {
                    // Replacing: renaming refuses existing targets, so copy
                    // over them and remove the source afterwards.
                    let request = Request::Copy { from: path.clone(), to: to.clone(), is_dir: item.is_dir };
                    if let Some(tag) = self.send_remote(from_site, request) {
                        let follow = FollowUp::MovedRemote { site_id: from_site.clone(), from: path.clone(), to, is_dir: item.is_dir };
                        self.pending_followups.insert(tag, follow);
                    }
                } else {
                    self.retarget_remote_docs(from_site, path, &to);
                    self.send_remote(from_site, Request::Rename { from: path.clone(), to });
                    self.send_remote(from_site, Request::List(remote::parent(path)));
                }
            }
            (FileRef::Remote { site_id: from_site, path }, DirRef::Remote { site_id: to_site, path: dir }) => {
                // Between servers: download to a temporary folder, then upload.
                let temp = std::env::temp_dir()
                    .join("nebula-transfer")
                    .join(format!("{:x}", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_nanos()))
                    .join(dest_name);
                if let Some(parent) = temp.parent() {
                    let _ = std::fs::create_dir_all(parent);
                }
                let then = (!copy).then(|| {
                    Box::new(FollowUp::DeleteRemote { site_id: from_site.clone(), path: path.clone(), is_dir: item.is_dir })
                });
                let request = Request::Download { remote: path.clone(), local: temp.clone(), is_dir: item.is_dir };
                if let Some(tag) = self.send_remote(from_site, request) {
                    self.pending_followups.insert(
                        tag,
                        FollowUp::Upload { site_id: to_site.clone(), local: temp, remote: remote::join(dir, dest_name), then },
                    );
                }
            }
        }
    }

    /// Sends a request to a configured site; returns its tag.
    pub(super) fn send_remote(&mut self, site_id: &str, request: Request) -> Option<u64> {
        let site = self.sites.get(site_id).cloned()?;
        self.ensure_tree(&site);
        Some(self.remote.send(&site, request))
    }

    pub(super) fn delete_items(&mut self, items: Vec<Item>) {
        for item in items {
            match &item.file {
                // Into the recycle bin, like Explorer (network drives have none).
                FileRef::Local(p) => {
                    if win32::move_to_recycle_bin(p) {
                        self.sidebar.refresh_local(p.parent());
                    } else {
                        self.error(t!("{p} konnte nicht gelöscht werden: {e}", p = p.display(), e = t!("Windows hat das Löschen abgelehnt (Datei in Benutzung?)")));
                    }
                }
                FileRef::Remote { site_id, path } => {
                    self.send_remote(site_id, Request::Delete { path: path.clone(), is_dir: item.is_dir });
                }
            }
        }
        self.sidebar.selection.clear();
    }

    pub(super) fn upload_files_dialog(&mut self, site_id: String, dir: String) {
        let Some(files) = rfd::FileDialog::new().set_title(t!("Dateien hochladen")).pick_files() else { return };
        let items = files.into_iter().map(|p| Item::local(p, false)).collect();
        self.transfer(items, DirRef::Remote { site_id, path: dir }, true);
    }

    pub(super) fn download_dialog(&mut self, item: Item) {
        let Some(dir) = rfd::FileDialog::new().set_title(t!("Herunterladen nach")).pick_folder() else { return };
        self.transfer(vec![item], DirRef::Local(dir), true);
    }

    /// Handles files dropped from Explorer: into a sidebar folder, or open them.
    pub(super) fn handle_external_drop(&mut self, ctx: &egui::Context, paths: Vec<PathBuf>) {
        let pos = win32::cursor_pos(ctx.pixels_per_point());
        if self.show_sidebar
            && let Some(pos) = pos
            && self.sidebar.contains(pos)
        {
            if let Some(target) = self.sidebar.drop_target_at(pos) {
                let items = paths
                    .into_iter()
                    .map(|p| {
                        let dir = p.is_dir();
                        Item::local(p, dir)
                    })
                    .collect();
                self.transfer(items, target, true);
            }
            return;
        }
        for p in paths {
            if p.is_dir() {
                self.open_folder(p);
            } else {
                self.open_local(&p);
            }
        }
    }

    pub(super) fn retarget_local_docs(&mut self, from: &Path, to: &Path) {
        for d in &mut self.docs {
            if let DocSource::Local(p) = &mut d.source
                && let Ok(rest) = p.strip_prefix(from)
            {
                *p = if rest.as_os_str().is_empty() { to.to_path_buf() } else { to.join(rest) };
            }
        }
    }

    pub(super) fn retarget_remote_docs(&mut self, site: &str, from: &str, to: &str) {
        for d in &mut self.docs {
            if let DocSource::Remote { site_id, path } = &mut d.source
                && site_id == site
            {
                if path == from {
                    *path = to.to_string();
                } else if let Some(rest) = path.strip_prefix(&format!("{from}/")) {
                    *path = format!("{to}/{rest}");
                }
            }
        }
    }
}

fn copy_local(src: &Path, dest: &Path) -> std::io::Result<()> {
    if src.is_dir() {
        std::fs::create_dir_all(dest)?;
        for entry in std::fs::read_dir(src)? {
            let entry = entry?;
            copy_local(&entry.path(), &dest.join(entry.file_name()))?;
        }
        Ok(())
    } else {
        std::fs::copy(src, dest).map(|_| ())
    }
}

fn remove_local(path: &Path) -> std::io::Result<()> {
    if path.is_dir() {
        std::fs::remove_dir_all(path)
    } else {
        std::fs::remove_file(path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn copies_folders_recursively() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("a");
        std::fs::create_dir_all(src.join("b")).unwrap();
        std::fs::write(src.join("b/c.txt"), "x").unwrap();
        copy_local(&src, &dir.path().join("a2")).unwrap();
        assert_eq!(std::fs::read_to_string(dir.path().join("a2/b/c.txt")).unwrap(), "x");
        remove_local(&src).unwrap();
        assert!(!src.exists());
    }
}

impl NebulaApp {
    /// Project-wide replace. Open documents are edited in the editor (undoable),
    /// other files are rewritten on disk.
    pub(super) fn replace_in_files(&mut self, files: Vec<PathBuf>, regex: regex::Regex, replacement: String, ctx: &egui::Context) {
        let mut count = 0usize;
        let mut changed_files = 0usize;
        for path in files {
            let canon = path.canonicalize().unwrap_or_else(|_| path.clone());
            let open = self.docs.iter().position(|d| match &d.source {
                DocSource::Local(p) => p == &path || p == &canon || p.canonicalize().ok().as_ref() == Some(&canon),
                _ => false,
            });
            if let Some(i) = open {
                let doc = &mut self.docs[i];
                let text = doc.text();
                let mut changes = Vec::new();
                let mut chars = 0usize;
                let mut last = 0usize;
                for m in regex.find_iter(&text) {
                    if m.is_empty() {
                        continue;
                    }
                    chars += text[last..m.start()].chars().count();
                    let start = chars;
                    chars += m.as_str().chars().count();
                    last = m.end();
                    let with = regex.replace(m.as_str(), replacement.as_str()).into_owned();
                    changes.push(crate::editor::document::Change::new(start..chars, with));
                }
                if !changes.is_empty() {
                    count += changes.len();
                    changed_files += 1;
                    doc.apply(changes, None, crate::editor::document::EditKind::Other);
                }
                continue;
            }
            let Ok(bytes) = std::fs::read(&path) else { continue };
            let Ok(text) = String::from_utf8(bytes) else { continue };
            let n = regex.find_iter(&text).filter(|m| !m.is_empty()).count();
            if n == 0 {
                continue;
            }
            let new = regex.replace_all(&text, replacement.as_str());
            match std::fs::write(&path, new.as_bytes()) {
                Ok(()) => {
                    count += n;
                    changed_files += 1;
                }
                Err(e) => self.error(t!("{path} konnte nicht geschrieben werden: {e}", path = path.display(), e = e)),
            }
        }
        self.info(t!("{count} Ersetzungen in {changed_files} Dateien (geöffnete Dateien noch speichern)", count = count, changed_files = changed_files));
        {
            let root = self.sidebar.root.clone();
            self.sidebar.search.start(root.as_deref(), &self.sites, ctx);
        }
    }
}
