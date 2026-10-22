//! Comparisons as read-only tabs in unified diff format: the working state
//! against the last commit, or any two files (local or on a server).

use std::collections::HashMap;
use std::path::Path;

use super::NebulaApp;
use crate::editor::{DocSource, Document};
use crate::lang::Language;
use crate::remote::Request;
use crate::ui::sidebar::FileRef;

/// Lines of context around each change.
const CONTEXT: usize = 3;

/// A comparison waiting for server files.
pub(super) struct DiffJob {
    title: String,
    /// (label, text) of the old and the new side once known.
    sides: [Option<(String, String)>; 2],
    /// Request tag -> side.
    pending: HashMap<u64, usize>,
}

#[derive(Default)]
pub(super) struct DiffJobs(Vec<DiffJob>);

fn name_of(file: &FileRef) -> String {
    match file {
        FileRef::Local(p) => p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
        FileRef::Remote { path, .. } => path.rsplit('/').next().unwrap_or(path).to_string(),
    }
}

impl NebulaApp {
    /// Opens the differences as a new read-only tab.
    fn open_diff_tab(&mut self, title: String, old: (&str, &str), new: (&str, &str)) {
        if old.1 == new.1 {
            self.info(t!("{title}: keine Unterschiede", title = title));
            return;
        }
        let text = crate::diff::patch(old.1, new.1, old.0, new.0, CONTEXT);
        self.untitled += 1;
        let mut doc = Document::new(DocSource::Untitled(self.untitled), &text);
        doc.language = Language::Diff;
        doc.suggested_name = Some(format!("{title}.diff"));
        doc.read_only = true;
        let at = self.active.map_or(self.docs.len(), |i| i + 1);
        self.docs.insert(at, doc);
        self.activate(at);
    }

    /// How a file is named in the diff header.
    fn diff_label(&self, file: &FileRef) -> String {
        match file {
            FileRef::Local(p) => self
                .sidebar
                .root
                .as_ref()
                .and_then(|root| p.strip_prefix(root).ok())
                .unwrap_or(p)
                .to_string_lossy()
                .replace('\\', "/"),
            FileRef::Remote { site_id, path } => {
                let site = self.sites.get(site_id).map(|s| s.display_name()).unwrap_or_default();
                format!("{site}:{path}")
            }
        }
    }

    /// The text of an open tab for `file`, including unsaved changes.
    fn open_text(&mut self, file: &FileRef) -> Option<String> {
        let source = match file {
            FileRef::Local(p) => DocSource::Local(p.clone()),
            FileRef::Remote { site_id, path } => DocSource::Remote { site_id: site_id.clone(), path: path.clone() },
        };
        let i = self.docs.iter().position(|d| d.source == source)?;
        Some(self.docs[i].text().to_string())
    }

    /// Working state of tab `i` against the version in the last Git commit.
    pub(super) fn compare_with_head(&mut self, i: usize) {
        let DocSource::Local(path) = self.docs[i].source.clone() else {
            self.info(t!("Nur lokale Dateien in einem Git-Projekt lassen sich mit dem letzten Commit vergleichen."));
            return;
        };
        self.compare_path_with_head(&path);
    }

    /// A local file (its open tab if any, else the disk) against the last commit.
    pub(super) fn compare_path_with_head(&mut self, path: &Path) {
        let Some(top) = self.sidebar.git.top.clone() else {
            self.info(t!("Das Projekt ist kein Git-Repository."));
            return;
        };
        let file = FileRef::Local(path.to_path_buf());
        let name = name_of(&file);
        let Some(bytes) = crate::ui::git::head_file(&top, path) else {
            self.info(t!("„{name}“ ist im letzten Commit nicht enthalten.", name = name));
            return;
        };
        let head = decode(&bytes);
        // A deleted file compares with nothing.
        let mine = match self.open_text(&file) {
            Some(text) => text,
            None if !path.exists() => String::new(),
            None => match read_local(path) {
                Ok(text) => text,
                Err(e) => {
                    self.error(e);
                    return;
                }
            },
        };
        let label = self.diff_label(&file);
        self.open_diff_tab(t!("{name} (Änderungen)", name = name), (&t!("a/{label} (letzter Commit)", label = label), &head), (&t!("b/{label} (Arbeitsstand)", label = label), &mine));
    }

    /// Tab `i` against a local file picked in a dialog.
    pub(super) fn compare_with_file_dialog(&mut self, i: usize) {
        let mut dialog = rfd::FileDialog::new();
        if let DocSource::Local(p) = &self.docs[i].source
            && let Some(dir) = p.parent()
        {
            dialog = dialog.set_directory(dir);
        } else if let Some(root) = &self.sidebar.root {
            dialog = dialog.set_directory(root);
        }
        let Some(other) = dialog.pick_file() else { return };
        let current = match &self.docs[i].source {
            DocSource::Local(p) => FileRef::Local(p.clone()),
            DocSource::Remote { site_id, path } => FileRef::Remote { site_id: site_id.clone(), path: path.clone() },
            DocSource::Untitled(_) => {
                // Unsaved tab: compare its text directly.
                let mine = self.docs[i].text().to_string();
                let title = self.docs[i].title();
                match read_local(&other) {
                    Ok(theirs) => {
                        let label = self.diff_label(&FileRef::Local(other.clone()));
                        self.open_diff_tab(format!("{} ↔ {title}", name_of(&FileRef::Local(other))), (&label, &theirs), (&title, &mine));
                    }
                    Err(e) => self.error(e),
                }
                return;
            }
        };
        self.compare_files(FileRef::Local(other), current);
    }

    /// Compares two files; `old` is shown with `-`, `new` with `+`.
    pub(super) fn compare_files(&mut self, old: FileRef, new: FileRef) {
        let title = format!("{} ↔ {}", name_of(&old), name_of(&new));
        let mut job = DiffJob { title, sides: [None, None], pending: HashMap::new() };
        for (side, file) in [old, new].into_iter().enumerate() {
            let label = self.diff_label(&file);
            if let Some(text) = self.open_text(&file) {
                job.sides[side] = Some((label, text));
                continue;
            }
            match &file {
                FileRef::Local(p) => match read_local(p) {
                    Ok(text) => job.sides[side] = Some((label, text)),
                    Err(e) => {
                        self.error(e);
                        return;
                    }
                },
                FileRef::Remote { site_id, path } => {
                    let Some(site) = self.sites.get(site_id).cloned() else {
                        self.error(t!("Der Server dieser Datei existiert nicht mehr."));
                        return;
                    };
                    self.ensure_tree(&site);
                    let tag = self.remote.send(&site, Request::Read(path.clone()));
                    job.sides[side] = Some((label, String::new()));
                    job.pending.insert(tag, side);
                }
            }
        }
        if job.pending.is_empty() {
            self.finish_diff(job);
        } else {
            self.diff_jobs.0.push(job);
        }
    }

    fn finish_diff(&mut self, job: DiffJob) {
        let [Some(old), Some(new)] = job.sides else { return };
        self.open_diff_tab(job.title, (&old.0, &old.1), (&new.0, &new.1));
    }

    /// A server read finished; returns true if a comparison was waiting for it.
    pub(super) fn diff_read_done(&mut self, tag: u64, data: Result<&[u8], String>) -> bool {
        let Some(n) = self.diff_jobs.0.iter().position(|j| j.pending.contains_key(&tag)) else { return false };
        match data {
            Ok(bytes) => {
                let job = &mut self.diff_jobs.0[n];
                let side = job.pending.remove(&tag).unwrap_or(0);
                if let Some((_, text)) = &mut job.sides[side] {
                    *text = decode(bytes);
                }
                if job.pending.is_empty() {
                    let job = self.diff_jobs.0.remove(n);
                    self.finish_diff(job);
                }
            }
            Err(e) => {
                let job = self.diff_jobs.0.remove(n);
                self.error(t!("Vergleich „{title}“ nicht möglich: {e}", title = job.title, e = e));
            }
        }
        true
    }
}

fn decode(bytes: &[u8]) -> String {
    crate::editor::document::Encoding::detect(bytes).decode(bytes).replace("\r\n", "\n")
}

fn read_local(path: &Path) -> Result<String, String> {
    let bytes = std::fs::read(path).map_err(|e| t!("{path} konnte nicht gelesen werden: {e}", path = path.display(), e = e))?;
    if bytes[..bytes.len().min(8192)].contains(&0) {
        return Err(t!("{path} ist keine Textdatei.", path = path.display()));
    }
    Ok(decode(&bytes))
}
