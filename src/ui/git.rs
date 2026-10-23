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
    changes: Vec<Change>,
    branches: Vec<String>,
    /// (behind, ahead) of the upstream branch; `None` without upstream.
    sync: Option<(usize, usize)>,
}

/// A git command running in the background (pull, push, commit, …).
pub struct GitOp {
    pub label: String,
    rx: Receiver<Result<String, String>>,
}

#[derive(Default)]
pub struct GitStatus {
    root: Option<PathBuf>,
    pub branch: Option<String>,
    /// Top folder of the repository and the current commit.
    pub top: Option<PathBuf>,
    pub head: Option<String>,
    pub changes: Vec<Change>,
    pub branches: Vec<String>,
    pub sync: Option<(usize, usize)>,
    files: HashMap<PathBuf, FileState>,
    dirs: HashSet<PathBuf>,
    pending: Option<Receiver<Option<Snapshot>>>,
    last: Option<Instant>,
    pub op: Option<GitOp>,
    /// Outcome of the last operation, for a message: (label, result).
    pub op_done: Option<(String, Result<String, String>)>,
}

/// Runs git for an operation; Ok(stdout) or Err(the useful part of stderr).
fn git_op(root: &Path, args: &[&str]) -> Result<String, String> {
    let mut cmd = std::process::Command::new("git");
    cmd.args(args)
        .current_dir(root)
        // Never wait for typed input that nobody can see; the credential
        // manager still shows its own login window.
        .env("GIT_TERMINAL_PROMPT", "0")
        .stdin(std::process::Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    let out = cmd.output().map_err(|e| t!("git konnte nicht gestartet werden: {e}", e = e))?;
    let stdout = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if out.status.success() {
        return Ok(stdout);
    }
    let stderr = String::from_utf8_lossy(&out.stderr);
    let msg = stderr
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with("hint:"))
        .collect::<Vec<_>>()
        .join("\n");
    Err(if msg.is_empty() { stdout } else { msg })
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

/// What a commit would contain, for writing its message.
pub struct CommitContext {
    /// Unified diff (cut if very long).
    pub diff: String,
    /// New files that would be committed too.
    pub new_files: Vec<String>,
    /// Summary lines of the last commits (for language and style).
    pub recent: Vec<String>,
}

/// Longest diff given to a model.
const MAX_DIFF: usize = 40_000;

/// The changes of the next commit: the staged ones, or everything when
/// nothing is staged (the commit button then adds all).
pub fn commit_context(top: &Path, staged_only: bool) -> Result<CommitContext, String> {
    let mut diff = if staged_only { git_op(top, &["diff", "--cached", "--no-color"])? } else { git_op(top, &["diff", "HEAD", "--no-color"]).or_else(|_| git_op(top, &["diff", "--no-color"]))? };
    if diff.len() > MAX_DIFF {
        let cut = diff.char_indices().nth(MAX_DIFF).map_or(diff.len(), |(i, _)| i);
        diff.truncate(cut);
        diff.push_str("\n… (diff cut)");
    }
    let new_files = if staged_only {
        Vec::new()
    } else {
        git_op(top, &["ls-files", "--others", "--exclude-standard"])?.lines().map(str::to_string).collect()
    };
    let recent = git_op(top, &["log", "-8", "--format=%s"]).map(|s| s.lines().map(str::to_string).collect()).unwrap_or_default();
    Ok(CommitContext { diff, new_files, recent })
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

/// `path` relative to the repository at `top`, with `/`.
fn repo_rel(top: &Path, path: &Path) -> Option<String> {
    // Windows paths compare without regard to case (git may report "a:/" for "A:\").
    let (p, t) = (path.to_string_lossy(), top.to_string_lossy());
    let t = t.trim_end_matches('\\');
    if !p.get(..t.len()).is_some_and(|prefix| prefix.eq_ignore_ascii_case(t)) || p.len() <= t.len() {
        return None;
    }
    Some(p[t.len()..].trim_start_matches('\\').replace('\\', "/"))
}

/// Runs git in `top` and returns its raw output if it succeeded.
fn git_bytes(top: &Path, args: &[&str]) -> Option<Vec<u8>> {
    let mut cmd = std::process::Command::new("git");
    cmd.args(args).current_dir(top);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    let out = cmd.output().ok()?;
    out.status.success().then_some(out.stdout)
}

/// Contents of `path` in the current commit (`None` if it is not tracked).
pub fn head_file(top: &Path, path: &Path) -> Option<Vec<u8>> {
    let rel = repo_rel(top, path)?;
    git_bytes(top, &["show", &format!("HEAD:{rel}")])
}

/// A commit that last changed some lines of a file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BlameCommit {
    pub hash: String,
    pub author: String,
    /// Seconds since 1970.
    pub time: i64,
    pub summary: String,
}

/// Who last changed each line of a file as of HEAD.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Blame {
    pub commits: Vec<BlameCommit>,
    /// Index into `commits` per line of the HEAD version.
    pub lines: Vec<u32>,
}

impl Blame {
    pub fn line(&self, line: usize) -> Option<&BlameCommit> {
        self.commits.get(*self.lines.get(line)? as usize)
    }
}

impl BlameCommit {
    /// "vor 3 Tagen" and the like, relative to `now` (seconds since 1970).
    pub fn ago(&self, now: i64) -> String {
        let s = (now - self.time).max(0);
        let (min, hour, day) = (60, 3600, 86_400);
        match s {
            _ if s < min => t!("gerade eben").into(),
            _ if s < hour => t!("vor {value} Min.", value = s / min),
            _ if s < day => t!("vor {value} Std.", value = s / hour),
            _ if s < 2 * day => t!("gestern").into(),
            _ if s < 30 * day => t!("vor {value} Tagen", value = s / day),
            _ if s < 60 * day => t!("vor einem Monat").into(),
            _ if s < 365 * day => t!("vor {value} Monaten", value = s / (30 * day)),
            _ if s < 2 * 365 * day => t!("vor einem Jahr").into(),
            _ => t!("vor {value} Jahren", value = s / (365 * day)),
        }
    }

    /// Date and time of the commit in local time.
    pub fn date(&self) -> String {
        let t = crate::win32::local_time(self.time.max(0) as u64 * 1000);
        format!("{:02}.{:02}.{} {:02}:{:02}", t.day, t.month, t.year, t.hour, t.minute)
    }
}

/// Blame of `path` as of HEAD (`None` if it is not tracked).
pub fn blame(top: &Path, path: &Path) -> Option<Blame> {
    let rel = repo_rel(top, path)?;
    let out = git_bytes(top, &["blame", "--porcelain", "HEAD", "--", &rel])?;
    Some(parse_blame(&out))
}

/// Parses `git blame --porcelain`: a header line per line of the file
/// ("<hash> <orig> <final> [<count>]"), commit details the first time a
/// commit appears, then the line itself after a tab.
fn parse_blame(out: &[u8]) -> Blame {
    let mut blame = Blame::default();
    let mut index: HashMap<String, u32> = HashMap::new();
    let mut current: Option<u32> = None;
    for raw in out.split(|&b| b == b'\n') {
        if raw.first() == Some(&b'\t') {
            if let Some(c) = current.take() {
                blame.lines.push(c);
            }
            continue;
        }
        let line = String::from_utf8_lossy(raw);
        let (key, value) = line.split_once(' ').unwrap_or((&line, ""));
        if current.is_none() {
            if key.len() == 40 && key.bytes().all(|b| b.is_ascii_hexdigit()) {
                let next = blame.commits.len() as u32;
                let i = *index.entry(key.to_string()).or_insert_with(|| {
                    blame.commits.push(BlameCommit { hash: key.to_string(), author: String::new(), time: 0, summary: String::new() });
                    next
                });
                current = Some(i);
            }
            continue;
        }
        let commit = &mut blame.commits[current.unwrap() as usize];
        match key {
            "author" => commit.author = value.to_string(),
            "author-time" => commit.time = value.parse().unwrap_or(0),
            "summary" => commit.summary = value.to_string(),
            _ => {}
        }
    }
    blame
}

/// One changed file from `git status`: index (staged) and worktree letters.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Change {
    pub path: PathBuf,
    /// Path relative to the repository, with `/`.
    pub rel: String,
    pub index: u8,
    pub worktree: u8,
}

impl Change {
    pub fn state(&self) -> FileState {
        match (self.index, self.worktree) {
            (b'?', b'?') => FileState::Untracked,
            (b'U', _) | (_, b'U') | (b'A', b'A') | (b'D', b'D') => FileState::Conflict,
            (b'R', _) => FileState::Renamed,
            (b'A', _) => FileState::Added,
            (b'D', _) | (_, b'D') => FileState::Deleted,
            _ => FileState::Modified,
        }
    }

    /// Has staged changes (shown under "Vorgemerkt").
    pub fn staged(&self) -> bool {
        !matches!(self.index, b' ' | b'?') && self.state() != FileState::Conflict
    }

    /// Has changes not staged yet (shown under "Änderungen").
    pub fn unstaged(&self) -> bool {
        self.worktree != b' ' || self.state() == FileState::Conflict
    }
}

fn parse_changes(top: &Path, text: &str) -> Vec<Change> {
    let mut out = Vec::new();
    let mut entries = text.split('\0');
    while let Some(e) = entries.next() {
        if e.len() < 4 {
            continue;
        }
        let (index, worktree) = (e.as_bytes()[0], e.as_bytes()[1]);
        if index == b'R' || index == b'C' {
            entries.next(); // original name
        }
        let rel = e[3..].trim_end_matches('/').to_string();
        out.push(Change { path: top.join(rel.replace('/', "\\")), rel, index, worktree });
    }
    out
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
                let changes = parse_changes(&top, &status);
                let branches = git(&root, &["branch", "--format=%(refname:short)"])
                    .map(|b| b.lines().map(|l| l.trim().to_string()).filter(|l| !l.is_empty()).collect())
                    .unwrap_or_default();
                let sync = git(&root, &["rev-list", "--left-right", "--count", "@{upstream}...HEAD"]).and_then(|s| {
                    let mut n = s.split_whitespace().map(|x| x.parse::<usize>().ok());
                    Some((n.next()??, n.next()??))
                });
                Some(Snapshot { root, top, head, branch, changes, branches, sync })
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
                    let files: HashMap<PathBuf, FileState> = s.changes.iter().map(|c| (c.path.clone(), c.state())).collect();
                    for p in files.keys() {
                        let mut d = p.parent();
                        while let Some(dir) = d {
                            if !self.dirs.insert(dir.to_path_buf()) {
                                break;
                            }
                            d = dir.parent();
                        }
                    }
                    self.files = files;
                    self.changes = s.changes;
                    self.branches = s.branches;
                    self.sync = s.sync;
                    self.branch = s.branch;
                    self.top = Some(s.top);
                    self.head = s.head;
                }
                _ => {
                    self.files.clear();
                    self.dirs.clear();
                    self.changes.clear();
                    self.branches.clear();
                    self.sync = None;
                    self.branch = None;
                    self.top = None;
                    self.head = None;
                }
            }
        }
        // A finished operation: report it and show the new state at once.
        if let Some(op) = &self.op
            && let Ok(result) = op.rx.try_recv()
        {
            let op = self.op.take().unwrap();
            self.op_done = Some((op.label, result));
            self.last = None;
        }
        let Some(root) = root else { return };
        let due = self.last.is_none_or(|t| t.elapsed() > Duration::from_secs(5)) || self.root.as_deref() != Some(root);
        if due && ctx.input(|i| i.focused) {
            self.refresh(root, ctx);
        }
        ctx.request_repaint_after(Duration::from_secs(5));
    }

    /// Runs git commands one after another in the background (stops at the
    /// first error). Ignored while another operation runs.
    pub fn run(&mut self, label: &str, steps: Vec<Vec<String>>, ctx: &egui::Context) {
        let Some(top) = self.top.clone() else { return };
        if self.op.is_some() {
            return;
        }
        let (tx, rx) = crossbeam_channel::bounded(1);
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let mut result = Ok(String::new());
            for args in &steps {
                let args: Vec<&str> = args.iter().map(String::as_str).collect();
                result = git_op(&top, &args);
                if result.is_err() {
                    break;
                }
            }
            let _ = tx.send(result);
            ctx.request_repaint();
        });
        self.op = Some(GitOp { label: label.to_string(), rx });
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
        let files: HashMap<PathBuf, FileState> = parse_changes(top, " M src/a.css\0?? new.html\0R  b.js\0old.js\0A  c.php\0")
            .into_iter()
            .map(|c| (c.path.clone(), c.state()))
            .collect();
        assert_eq!(files[&PathBuf::from(r"C:\p\src\a.css")], FileState::Modified);
        assert_eq!(files[&PathBuf::from(r"C:\p\new.html")], FileState::Untracked);
        assert_eq!(files[&PathBuf::from(r"C:\p\b.js")], FileState::Renamed);
        assert_eq!(files[&PathBuf::from(r"C:\p\c.php")], FileState::Added);
        assert_eq!(files.len(), 4);
    }

    #[test]
    fn parses_blame_porcelain() {
        let a = "a".repeat(40);
        let b = "b".repeat(40);
        let out = format!(
            "{a} 1 1 2\nauthor Anna\nauthor-mail <a@x>\nauthor-time 1700000000\nsummary Erster Stand\nfilename f\n\tzeile 1\n\
             {a} 2 2\n\tsummary looks like a header\n\
             {b} 3 3 1\nauthor Bert\nauthor-time 1710000000\nsummary Fix\nprevious {a} f\nfilename f\n\t{a} 9 9\n"
        );
        let blame = parse_blame(out.as_bytes());
        assert_eq!(blame.lines, [0, 0, 1]);
        assert_eq!(blame.commits[0].author, "Anna");
        assert_eq!(blame.commits[0].summary, "Erster Stand");
        assert_eq!(blame.line(2).unwrap().author, "Bert");
        assert_eq!(blame.line(2).unwrap().time, 1_710_000_000);
        assert!(blame.line(3).is_none());
    }

    #[test]
    fn staged_and_unstaged() {
        let changes = parse_changes(Path::new(r"C:\p"), "M  a\0 M b\0MM c\0?? d\0A  e\0UU f\0");
        let staged: Vec<&str> = changes.iter().filter(|c| c.staged()).map(|c| c.rel.as_str()).collect();
        let unstaged: Vec<&str> = changes.iter().filter(|c| c.unstaged()).map(|c| c.rel.as_str()).collect();
        assert_eq!(staged, ["a", "c", "e"]);
        assert_eq!(unstaged, ["b", "c", "d", "f"]);
    }

    #[test]
    fn repo_top_keeps_drive_letter() {
        assert_eq!(repo_top(Path::new(r"A:\Dev\proj"), "\n"), PathBuf::from(r"A:\Dev\proj"));
        assert_eq!(repo_top(Path::new(r"A:\Dev\proj\web"), "../\n"), PathBuf::from(r"A:\Dev\proj"));
        assert_eq!(repo_top(Path::new(r"A:\Dev\proj\a\b"), "../../"), PathBuf::from(r"A:\Dev\proj"));
    }
}
