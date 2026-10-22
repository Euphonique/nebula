//! Project search on a server: runs `rg`, `git grep` or `grep` over SSH
//! (whichever the server has) and streams the matches back.

use std::sync::Arc;

use crossbeam_channel::{Receiver, Sender};
use eframe::egui;
use russh::ChannelMsg;

use super::HostKeyPrompt;
use super::sftp::{PromptFn, RUNTIME, open_session};
use crate::settings::Site;

/// Matching lines read at most.
pub const MAX_LINES: usize = 5000;
/// Longest line kept per match; protects against minified files.
const MAX_LINE_BYTES: usize = 600;
/// Prefix of the line carrying the searched folder's absolute path.
const ROOT_MARK: &str = "NEBULA-ROOT:";

#[derive(Clone, Default)]
pub struct Query {
    pub pattern: String,
    pub regex: bool,
    pub case_sensitive: bool,
    pub whole_word: bool,
    /// Patterns like in `.gitignore`: `*.php` limits, `!vendor/` excludes.
    pub files: Vec<String>,
    /// Also search ignored files and folders like `node_modules`.
    pub include_ignored: bool,
    /// Folders skipped unless `include_ignored`.
    pub skip_dirs: Vec<String>,
}

/// One matching line: path relative to the search folder, zero-based line.
pub struct Hit {
    pub path: String,
    pub line: usize,
    pub text: String,
}

pub enum SearchEvent {
    HostKey(HostKeyPrompt),
    /// Absolute path of the searched folder (comes before the hits).
    Root(String),
    Hits(Vec<Hit>),
    /// Finished; with an error message if the search could not run.
    Done(Option<String>),
}

/// A running search. Dropping it stops the search on the server.
pub struct RemoteSearch {
    pub events: Receiver<SearchEvent>,
    cancel: Arc<tokio::sync::Notify>,
}

impl Drop for RemoteSearch {
    fn drop(&mut self) {
        self.cancel.notify_one();
    }
}

/// Starts a search in `root` on `site`.
pub fn start(mut site: Site, root: String, query: Query, ctx: egui::Context) -> RemoteSearch {
    // FTP servers usually offer SSH on the standard port (like the terminal).
    if site.protocol != crate::settings::Protocol::Sftp {
        site.port = 22;
    }
    let (tx, events) = crossbeam_channel::unbounded();
    let cancel = Arc::new(tokio::sync::Notify::new());
    let stop = cancel.clone();
    RUNTIME.spawn(async move {
        let result = run(site, root, query, tx.clone(), ctx.clone(), stop).await;
        let _ = tx.send(SearchEvent::Done(result.err().map(|e| format!("{e:#}"))));
        ctx.request_repaint();
    });
    RemoteSearch { events, cancel }
}

async fn run(
    site: Site,
    root: String,
    query: Query,
    tx: Sender<SearchEvent>,
    ctx: egui::Context,
    cancel: Arc<tokio::sync::Notify>,
) -> anyhow::Result<()> {
    let password = {
        let site = site.clone();
        tokio::task::spawn_blocking(move || site.password()).await?.unwrap_or_default()
    };
    let prompt = {
        let (tx, ctx) = (tx.clone(), ctx.clone());
        move |p: HostKeyPrompt| {
            let _ = tx.send(SearchEvent::HostKey(p));
            ctx.request_repaint();
        }
    };
    let session = tokio::select! {
        s = open_session(&site, &password, Arc::new(Box::new(prompt) as PromptFn)) => s?,
        _ = cancel.notified() => return Ok(()),
    };
    let mut channel = session.channel_open_session().await?;
    channel.exec(true, command(&root, &query)).await?;

    let mut buf = Vec::new();
    let mut stderr = Vec::new();
    let mut lines = 0;
    let mut status = None;
    loop {
        let msg = tokio::select! {
            msg = channel.wait() => msg,
            _ = cancel.notified() => {
                // Stop the command; closing the connection alone would let it run on.
                let _ = channel.signal(russh::Sig::TERM).await;
                return Ok(());
            }
        };
        let Some(msg) = msg else { break };
        match msg {
            ChannelMsg::Data { data } => {
                buf.extend_from_slice(&data);
                // Hand over complete lines as they arrive.
                if let Some(end) = buf.iter().rposition(|&b| b == b'\n') {
                    let chunk: Vec<u8> = buf.drain(..=end).collect();
                    let mut hits = Vec::new();
                    for raw in chunk.split(|&b| b == b'\n') {
                        if let Some(root) = raw.strip_prefix(ROOT_MARK.as_bytes()) {
                            let root = String::from_utf8_lossy(root).trim_end().to_string();
                            let _ = tx.send(SearchEvent::Root(root));
                        } else {
                            hits.extend(parse_line(raw));
                        }
                    }
                    lines += hits.len();
                    if !hits.is_empty() {
                        if tx.send(SearchEvent::Hits(hits)).is_err() {
                            break;
                        }
                        ctx.request_repaint();
                    }
                    // Enough; closing the session stops the command.
                    if lines >= MAX_LINES {
                        buf.clear();
                        break;
                    }
                }
            }
            ChannelMsg::ExtendedData { data, .. } => stderr.extend_from_slice(&data),
            ChannelMsg::ExitStatus { exit_status } => status = Some(exit_status),
            ChannelMsg::Eof | ChannelMsg::Close => {}
            _ => {}
        }
    }
    if let Some(hit) = parse_line(&buf) {
        lines += 1;
        let _ = tx.send(SearchEvent::Hits(vec![hit]));
    }
    drop(session);
    // grep/rg exit with 1 for "nothing found"; 2 and up are real errors.
    if lines == 0 && status.is_some_and(|s| s >= 2) {
        let msg = String::from_utf8_lossy(&stderr).trim().lines().last().unwrap_or("").to_string();
        anyhow::bail!(if msg.is_empty() { t!("Suche auf dem Server fehlgeschlagen (Status {unwrap})", unwrap = status.unwrap()) } else { msg });
    }
    Ok(())
}

/// `'it'\''s'` – safe in any POSIX shell.
fn quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}

/// Shell command that uses the best search tool available on the server.
/// Every tool prints `path NUL line (: or NUL) text`.
fn command(root: &str, q: &Query) -> String {
    let pattern = quote(&q.pattern);
    let mut common = String::new();
    if !q.case_sensitive {
        common.push_str(" -i");
    }
    if q.whole_word {
        common.push_str(" -w");
    }
    let (includes, excludes): (Vec<&str>, Vec<&str>) = {
        let (ex, inc): (Vec<&String>, Vec<&String>) = q.files.iter().partition(|p| p.starts_with('!'));
        (inc.into_iter().map(|s| s.as_str()).collect(), ex.into_iter().map(|s| &s[1..]).collect())
    };
    let skip: Vec<&str> = if q.include_ignored { Vec::new() } else { q.skip_dirs.iter().map(|s| s.as_str()).collect() };

    // ripgrep: same regex syntax as Nebula, honours .gitignore itself.
    let mut rg = t!("rg --no-heading --line-number --null --color never --no-messages --max-filesize 4M{common}", common = common);
    rg.push_str(if q.regex { "" } else { " -F" });
    rg.push_str(if q.include_ignored { " -uu" } else { " --hidden -g '!.git/'" });
    for p in &includes {
        rg.push_str(&format!(" -g {}", quote(p)));
    }
    for p in &excludes {
        rg.push_str(&format!(" -g {}", quote(&format!("!{p}"))));
    }
    for d in &skip {
        rg.push_str(&format!(" -g {}", quote(&format!("!{d}/"))));
    }
    rg.push_str(&format!(" -e {pattern} ."));

    // git grep: tracked and untracked files, without the ignored ones.
    let mut git = t!("git grep -n -z -I --untracked --no-color{common}", common = common);
    git.push_str(if q.regex { " -E" } else { " -F" });
    git.push_str(&format!(" -e {pattern} --"));
    if includes.is_empty() {
        git.push_str(" .");
    }
    for p in &includes {
        git.push_str(&format!(" {}", quote(p)));
    }
    for p in &excludes {
        git.push_str(&format!(" {}", quote(&git_exclude(p))));
    }
    for d in &skip {
        git.push_str(&format!(" {}", quote(&git_exclude(&format!("{d}/")))));
    }

    // grep: always there; folder excludes approximate .gitignore.
    let mut grep = format!("grep -rsnIHZ --color=never{common}");
    // Older greps crash on -i -F; an escaped -E pattern does the same.
    let grep_pattern = if !q.regex && !q.case_sensitive { quote(&ere_escape(&q.pattern)) } else { pattern.clone() };
    grep.push_str(if q.regex || !q.case_sensitive { " -E" } else { " -F" });
    for d in [".git", ".svn", ".hg"].iter().chain(&skip) {
        grep.push_str(&format!(" --exclude-dir={}", quote(d)));
    }
    for p in &includes {
        grep.push_str(&format!(" --include={}", quote(p.trim_start_matches("**/"))));
    }
    for p in &excludes {
        if let Some(dir) = p.strip_suffix('/') {
            grep.push_str(&format!(" --exclude-dir={}", quote(dir)));
        } else {
            grep.push_str(&format!(" --exclude={}", quote(p)));
        }
    }
    grep.push_str(&format!(" -e {grep_pattern} ."));

    let use_git = if q.include_ignored { "false" } else { "git rev-parse --is-inside-work-tree >/dev/null 2>&1" };
    let dir = if root.trim().is_empty() { ".".to_string() } else { quote(root) };
    // First line: the absolute folder, so results can be opened.
    format!(
        "cd {dir} || exit 2; printf '{ROOT_MARK}%s\\n' \"$(pwd)\"; if command -v rg >/dev/null 2>&1; then exec {rg}; elif {use_git}; then exec {git}; else exec {grep}; fi"
    )
}

/// Literal text as a POSIX extended regex.
fn ere_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        if r"\.[]()*+?{}|^$".contains(c) {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

/// A `.gitignore`-style exclude as git pathspec: without a slash it applies
/// at any depth, a trailing slash means the folder and everything inside.
fn git_exclude(p: &str) -> String {
    let (p, dir) = match p.strip_suffix('/') {
        Some(d) => (d, true),
        None => (p, false),
    };
    let anchored = p.strip_prefix('/');
    let p = match anchored {
        Some(a) => a.to_string(),
        None if p.contains('/') => p.to_string(),
        None => format!("**/{p}"),
    };
    format!(":(exclude,glob){p}{}", if dir { "/**" } else { "" })
}

fn parse_line(raw: &[u8]) -> Option<Hit> {
    let raw = raw.strip_suffix(b"\r").unwrap_or(raw);
    let nul = raw.iter().position(|&b| b == 0)?;
    let path = String::from_utf8_lossy(&raw[..nul]);
    let rest = &raw[nul + 1..];
    let digits = rest.iter().take_while(|b| b.is_ascii_digit()).count();
    if digits == 0 || !matches!(rest.get(digits), Some(b':' | 0)) {
        return None;
    }
    let line: usize = std::str::from_utf8(&rest[..digits]).ok()?.parse().ok()?;
    let mut text = &rest[digits + 1..];
    if text.len() > MAX_LINE_BYTES {
        text = &text[..MAX_LINE_BYTES];
    }
    Some(Hit {
        path: path.trim_start_matches("./").to_string(),
        line: line.saturating_sub(1),
        text: String::from_utf8_lossy(text).into_owned(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_all_tool_formats() {
        let grep = parse_line(b"./src/a.php\x0012:  echo $x;").unwrap();
        assert_eq!((grep.path.as_str(), grep.line, grep.text.as_str()), ("src/a.php", 11, "  echo $x;"));
        let git = parse_line(b"src/a.php\x003\x00a:b").unwrap();
        assert_eq!((git.path.as_str(), git.line, git.text.as_str()), ("src/a.php", 2, "a:b"));
        assert!(parse_line(b"").is_none());
        assert!(parse_line(b"Binary file matches").is_none());
    }

    #[test]
    fn command_quotes_and_filters() {
        let q = Query {
            pattern: "it's".into(),
            files: vec!["*.php".into(), "!vendor/".into()],
            skip_dirs: vec!["node_modules".into()],
            ..Default::default()
        };
        let c = command("/var/www/my site", &q);
        assert!(c.starts_with("cd '/var/www/my site' || exit 2; printf 'NEBULA-ROOT:%s\\n' \"$(pwd)\";"));
        assert!(command("", &q).starts_with("cd . ||"));
        assert!(c.contains(r"-e 'it'\''s'"));
        assert!(c.contains("-g '*.php' -g '!vendor/' -g '!node_modules/'"));
        assert!(c.contains("-- '*.php' ':(exclude,glob)**/vendor/**' ':(exclude,glob)**/node_modules/**'"));
        assert_eq!(git_exclude("app.rs"), ":(exclude,glob)**/app.rs");
        assert_eq!(git_exclude("/build/x.js"), ":(exclude,glob)build/x.js");
        assert!(c.contains("--exclude-dir='node_modules' --include='*.php' --exclude-dir='vendor'"));
    }
}
