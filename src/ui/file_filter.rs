//! The "Filtern" field below the file tree: finds matching file names in the
//! background (respecting `.gitignore`), so typing never blocks the window –
//! the project may be large or sit on a network drive.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crossbeam_channel::Receiver;
use eframe::egui;
use ignore::WalkBuilder;

/// Pause after the last keystroke before searching.
const DEBOUNCE: Duration = Duration::from_millis(150);
/// More matches than this are not shown (the tree would be unusable anyway).
const MAX_MATCHES: usize = 2000;
/// Never descended into, even without a `.gitignore`.
const SKIP_DIRS: &[&str] = &[".git", ".svn", ".hg", "node_modules", "target"];

/// Files and folders whose names contain the filter text.
pub struct Matches {
    /// Folders to show: those with matches inside and those matching themselves.
    pub dirs: HashSet<PathBuf>,
    pub files: HashSet<PathBuf>,
    pub truncated: bool,
}

#[derive(Default)]
pub struct LocalFilter {
    /// Query (lowercase) and root the current or pending result is for.
    wanted: Option<(String, PathBuf)>,
    changed_at: Option<Instant>,
    job: Option<Receiver<Matches>>,
    result: Option<Arc<Matches>>,
    /// The result belongs to `wanted` (otherwise it is from an earlier query).
    fresh: bool,
}

impl LocalFilter {
    /// The matches to show for `query` (lowercase), searching when it changed.
    /// Returns the last result while a new search runs; `None` before the first.
    pub fn update(&mut self, ctx: &egui::Context, root: &Path, query: &str) -> Option<Arc<Matches>> {
        if query.is_empty() {
            *self = Self::default();
            return None;
        }
        let key = (query.to_string(), root.to_path_buf());
        if self.wanted.as_ref() != Some(&key) {
            self.wanted = Some(key);
            self.changed_at = Some(Instant::now());
            self.job = None;
            self.fresh = false;
        }
        if let Some(rx) = &self.job {
            match rx.try_recv() {
                Ok(m) => {
                    self.result = Some(Arc::new(m));
                    self.fresh = true;
                    self.job = None;
                }
                Err(crossbeam_channel::TryRecvError::Empty) => ctx.request_repaint_after(Duration::from_millis(50)),
                Err(crossbeam_channel::TryRecvError::Disconnected) => self.job = None,
            }
        } else if !self.fresh
            && let Some(at) = self.changed_at
        {
            let wait = DEBOUNCE.saturating_sub(at.elapsed());
            if wait.is_zero() {
                let (tx, rx) = crossbeam_channel::bounded(1);
                let (query, root) = (query.to_string(), root.to_path_buf());
                let repaint = ctx.clone();
                std::thread::spawn(move || {
                    let _ = tx.send(find(&root, &query));
                    repaint.request_repaint();
                });
                self.job = Some(rx);
                self.changed_at = None;
            } else {
                ctx.request_repaint_after(wait);
            }
        }
        self.result.clone()
    }

    /// Searching for the current text has not finished yet.
    pub fn busy(&self) -> bool {
        self.wanted.is_some() && !self.fresh
    }

    /// Files changed on disk: search again.
    pub fn invalidate(&mut self) {
        if self.wanted.is_some() {
            self.fresh = false;
            self.job = None;
            self.changed_at = Some(Instant::now() - DEBOUNCE);
        }
    }
}

/// Walks `root` for names containing `query` (lowercase).
pub fn find(root: &Path, query: &str) -> Matches {
    let mut m = Matches { dirs: HashSet::new(), files: HashSet::new(), truncated: false };
    let walk = WalkBuilder::new(root)
        .hidden(false)
        .require_git(false)
        .filter_entry(|e| !(e.depth() > 0 && e.file_type().is_some_and(|t| t.is_dir()) && SKIP_DIRS.iter().any(|s| e.file_name() == *s)))
        .build();
    for entry in walk.flatten() {
        if entry.depth() == 0 || !entry.file_name().to_string_lossy().to_lowercase().contains(query) {
            continue;
        }
        if m.files.len() + m.dirs.len() >= MAX_MATCHES {
            m.truncated = true;
            break;
        }
        let path = entry.path().to_path_buf();
        // Every folder on the way down stays visible.
        for parent in path.ancestors().skip(1) {
            if parent == root || !m.dirs.insert(parent.to_path_buf()) {
                break;
            }
        }
        if entry.file_type().is_some_and(|t| t.is_dir()) {
            m.dirs.insert(path);
        } else {
            m.files.insert(path);
        }
    }
    m
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_names_and_keeps_parents() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        std::fs::create_dir_all(root.join("src/ui")).unwrap();
        std::fs::create_dir_all(root.join("target/debug")).unwrap();
        std::fs::write(root.join("src/ui/Sidebar.rs"), "").unwrap();
        std::fs::write(root.join("src/main.rs"), "").unwrap();
        std::fs::write(root.join("target/debug/sidebar.d"), "").unwrap();
        let m = find(root, "sidebar");
        assert!(m.files.contains(&root.join("src/ui/Sidebar.rs")));
        assert!(m.dirs.contains(&root.join("src")) && m.dirs.contains(&root.join("src/ui")));
        assert!(!m.files.contains(&root.join("src/main.rs")));
        // Build folders are skipped.
        assert!(!m.dirs.contains(&root.join("target")));
    }
}
