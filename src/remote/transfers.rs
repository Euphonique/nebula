//! List of uploads, downloads, copies and saves on servers, with their state,
//! for the transfer list in the status bar.

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

use super::Request;

/// Finished entries kept at most.
const MAX_FINISHED: usize = 100;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TransferState {
    Queued,
    /// Files done so far, and the total if known.
    Running { done: usize, total: Option<usize> },
    Done(String),
    Failed(String),
}

impl TransferState {
    pub fn finished(&self) -> bool {
        matches!(self, TransferState::Done(_) | TransferState::Failed(_))
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Up,
    Down,
    /// Copy within one server.
    Within,
}

pub struct Transfer {
    pub tag: u64,
    pub site_id: String,
    pub site_name: String,
    pub request: Request,
    pub state: TransferState,
    pub started: Instant,
    /// When it finished (for "fertig vor …").
    pub finished: Option<Instant>,
    pub cancel: Arc<AtomicBool>,
}

impl Transfer {
    pub fn direction(&self) -> Direction {
        match self.request {
            Request::Download { .. } => Direction::Down,
            Request::Copy { .. } => Direction::Within,
            _ => Direction::Up,
        }
    }

    /// File or folder name shown in the list.
    pub fn name(&self) -> String {
        let last = |p: &str| p.trim_end_matches('/').rsplit('/').next().unwrap_or(p).to_string();
        match &self.request {
            Request::Upload { local, .. } => local.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
            Request::Download { remote, .. } => last(remote),
            Request::Copy { from, .. } => last(from),
            Request::Write { path, .. } => last(path),
            _ => String::new(),
        }
    }

    /// Where it goes, e.g. "→ Server:/var/www".
    pub fn detail(&self) -> String {
        match &self.request {
            Request::Upload { remote, .. } => format!("nach {}:{}", self.site_name, super::parent(remote)),
            Request::Download { local, .. } => format!("von {} nach {}", self.site_name, local.parent().map(|p| p.display().to_string()).unwrap_or_default()),
            Request::Copy { to, .. } => format!("{}:{}", self.site_name, to),
            Request::Write { .. } => format!("Speichern auf {}", self.site_name),
            _ => String::new(),
        }
    }

    /// Saves are retried by saving again (the tab must learn about it).
    pub fn can_retry(&self) -> bool {
        matches!(self.state, TransferState::Failed(_)) && !matches!(self.request, Request::Write { .. })
    }
}

/// Requests that move file contents and belong in the list.
pub fn is_transfer(request: &Request) -> bool {
    matches!(request, Request::Upload { .. } | Request::Download { .. } | Request::Copy { .. } | Request::Write { .. })
}

/// Number of files below a local path (for upload progress).
pub fn count_local(path: &Path) -> usize {
    if path.is_dir() {
        std::fs::read_dir(path).map(|rd| rd.flatten().map(|e| count_local(&e.path())).sum()).unwrap_or(0)
    } else {
        1
    }
}

#[derive(Default)]
pub struct Transfers {
    pub items: Vec<Transfer>,
}

impl Transfers {
    pub fn add(&mut self, t: Transfer) {
        self.items.insert(0, t);
        // Drop the oldest finished entries.
        let mut finished = 0;
        self.items.retain(|t| {
            if t.state.finished() {
                finished += 1;
                finished <= MAX_FINISHED
            } else {
                true
            }
        });
    }

    pub fn get_mut(&mut self, tag: u64) -> Option<&mut Transfer> {
        self.items.iter_mut().find(|t| t.tag == tag)
    }

    pub fn running(&self) -> usize {
        self.items.iter().filter(|t| !t.state.finished()).count()
    }

    pub fn failed(&self) -> usize {
        self.items.iter().filter(|t| matches!(t.state, TransferState::Failed(_))).count()
    }

    pub fn cancel(&mut self, tag: u64) {
        if let Some(t) = self.get_mut(tag) {
            t.cancel.store(true, Ordering::Relaxed);
        }
    }

    pub fn clear_finished(&mut self) {
        self.items.retain(|t| !t.state.finished());
    }

    pub fn remove(&mut self, tag: u64) {
        self.items.retain(|t| t.tag != tag);
    }
}
