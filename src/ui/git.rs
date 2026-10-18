//! Git status of the local project, via the `git` command line.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use crossbeam_channel::Receiver;
use eframe::egui::{self, Color32};

use crate::theme::Theme;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FileState {
    Modified,
    Added,
    Untracked,
    Deleted,
    Renamed,
    Conflict,
}

impl FileState {
    pub fn letter(self) -> &'static str {
        match self {
            FileState::Modified => "M",
            FileState::Added => "A",
            FileState::Untracked => "U",
            FileState::Deleted => "D",
            FileState::Renamed => "R",
            FileState::Conflict => "!",
        }
    }

    pub fn color(self, theme: &Theme) -> Color32 {
        match self {
            FileState::Modified | FileState::Renamed => theme.modified,
            FileState::Added | FileState::Untracked => theme.success,
            FileState::Deleted | FileState::Conflict => theme.danger,
        }
    }
}

struct Snapshot {
    root: PathBuf,
    top: PathBuf,
    head: Option<String>,
    branch: Option<String>,
    files: HashMap<PathBuf, FileState>,
}

#[derive(Default)]
pub struct GitStatus {
    root: Option<PathBuf>,
    pub branch: Option<String>,
    /// Top folder of the repository and the current commit.
    pub top: Option<PathBuf>,
    pub head: Option<String>,
    files: HashMap<PathBuf, FileState>,
    dirs: HashSet<PathBuf>,
    pending: Option<Receiver<Option<Snapshot>>>,
    last: Option<Instant>,
}

fn git(root: &Path, args: &[&str]) -> Option<String> {
    let mut cmd = std::process::Command::new("git");
    cmd.args(args).current_dir(root);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    let out = cmd.output().ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

/// The repository folder from the project folder and `git rev-parse --show-cdup` ("../../").
fn repo_top(root: &Path, cdup: &str) -> PathBuf {
    let mut top = root.to_path_buf();
    for part in cdup.trim().split(['/', '\\']).filter(|p| !p.is_empty()) {
        if part == ".." {
            top.pop();
        }
    }
    top
}

/// Contents of `path` in the current commit (`None` if it is not tracked).
pub fn head_file(top: &Path, path: &Path) -> Option<Vec<u8>> {
    // Windows paths compare without regard to case (git may report "a:/" for "A:\").
    let (p, t) = (path.to_string_lossy(), top.to_string_lossy());
    let t = t.trim_end_matches('\\');
    if !p.get(..t.len()).is_some_and(|prefix| prefix.eq_ignore_ascii_case(t)) || p.len() <= t.len() {
        return None;
    }
    let rel = p[t.len()..].trim_start_matches('\\').replace('\\', "/");
    let mut cmd = std::process::Command::new("git");
    cmd.args(["show", &format!("HEAD:{rel}")]).current_dir(top);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    let out = cmd.output().ok()?;
    out.status.success().then_some(out.stdout)
}

fn parse_porcelain(top: &Path, text: &str) -> HashMap<PathBuf, FileState> {
    let mut files = HashMap::new();
    let mut entries = text.split('\0');
    while let Some(e) = entries.next() {
        if e.len() < 4 {
            continue;
        }
        let (x, y) = (e.as_bytes()[0], e.as_bytes()[1]);
        let path = &e[3..];
        let state = match (x, y) {
            (b'?', b'?') => FileState::Untracked,
            (b'U', _) | (_, b'U') | (b'A', b'A') | (b'D', b'D') => FileState::Conflict,
            (b'R', _) => {
                entries.next(); // original name
                FileState::Renamed
            }
            (b'A', _) => FileState::Added,
            (b'D', _) | (_, b'D') => FileState::Deleted,
            _ => FileState::Modified,
        };
        let p: PathBuf = top.join(path.trim_end_matches('/').replace('/', "\\"));
        files.insert(p, state);
    }
    files
}

impl GitStatus {
    /// Starts a background refresh for `root` (no-op if one is running).
    pub fn refresh(&mut self, root: &Path, ctx: &egui::Context) {
        if self.pending.is_some() {
            return;
        }
        if self.root.as_deref() != Some(root) {
            self.files.clear();
            self.dirs.clear();
            self.branch = None;
        }
        self.root = Some(root.to_path_buf());
        self.last = Some(Instant::now());
        let (tx, rx) = crossbeam_channel::bounded(1);
        let root = root.to_path_buf();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let snapshot = (|| {
                // Relative to the project folder rather than `--show-toplevel`:
                // on a mapped network drive git answers with the UNC path
                // (//server/share/…), which would not match our `A:\…` paths.
                let cdup = git(&root, &["rev-parse", "--show-cdup"])?;
                let top = repo_top(&root, &cdup);
                let branch = git(&root, &["rev-parse", "--abbrev-ref", "HEAD"]).map(|b| b.trim().to_string());
                let head = git(&root, &["rev-parse", "HEAD"]).map(|b| b.trim().to_string());
                let status = git(&root, &["status", "--porcelain=v1", "-z", "--untracked-files=all"])?;
                let files = parse_porcelain(&top, &status);
                Some(Snapshot { root, top, head, branch, files })
            })();
            let _ = tx.send(snapshot);
            ctx.request_repaint();
        });
        self.pending = Some(rx);
    }

    /// Picks up finished refreshes and refreshes periodically.
    pub fn poll(&mut self, root: Option<&Path>, ctx: &egui::Context) {
        if let Some(rx) = &self.pending
            && let Ok(result) = rx.try_recv()
        {
            self.pending = None;
            match result {
                Some(s) if Some(s.root.as_path()) == root => {
                    self.dirs.clear();
                    for p in s.files.keys() {
                        let mut d = p.parent();
                        while let Some(dir) = d {
                            if !self.dirs.insert(dir.to_path_buf()) {
                                break;
                            }
                            d = dir.parent();
                        }
                    }
                    self.files = s.files;
                    self.branch = s.branch;
                    self.top = Some(s.top);
                    self.head = s.head;
                }
                _ => {
                    self.files.clear();
                    self.dirs.clear();
                    self.branch = None;
                    self.top = None;
                    self.head = None;
                }
            }
        }
        let Some(root) = root else { return };
        let due = self.last.is_none_or(|t| t.elapsed() > Duration::from_secs(5)) || self.root.as_deref() != Some(root);
        if due && ctx.input(|i| i.focused) {
            self.refresh(root, ctx);
        }
        ctx.request_repaint_after(Duration::from_secs(5));
    }

    pub fn file(&self, path: &Path) -> Option<FileState> {
        self.files.get(path).copied()
    }

    pub fn dir_changed(&self, path: &Path) -> bool {
        self.dirs.contains(path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_porcelain_output() {
        let top = Path::new(r"C:\p");
        let files = parse_porcelain(top, " M src/a.css\0?? new.html\0R  b.js\0old.js\0A  c.php\0");
        assert_eq!(files[&PathBuf::from(r"C:\p\src\a.css")], FileState::Modified);
        assert_eq!(files[&PathBuf::from(r"C:\p\new.html")], FileState::Untracked);
        assert_eq!(files[&PathBuf::from(r"C:\p\b.js")], FileState::Renamed);
        assert_eq!(files[&PathBuf::from(r"C:\p\c.php")], FileState::Added);
        assert_eq!(files.len(), 4);
    }

    #[test]
    fn repo_top_keeps_drive_letter() {
        assert_eq!(repo_top(Path::new(r"A:\Dev\proj"), "\n"), PathBuf::from(r"A:\Dev\proj"));
        assert_eq!(repo_top(Path::new(r"A:\Dev\proj\web"), "../\n"), PathBuf::from(r"A:\Dev\proj"));
        assert_eq!(repo_top(Path::new(r"A:\Dev\proj\a\b"), "../../"), PathBuf::from(r"A:\Dev\proj"));
    }
}
