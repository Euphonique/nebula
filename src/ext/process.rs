//! Subprocesses for extensions (the Process API). Nova extensions are
//! written for macOS, so Unix-style launches like `/usr/bin/env node` are
//! mapped to their Windows equivalents.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};

use crossbeam_channel::Sender;

pub enum ProcEvent {
    /// A line of output, including its line break.
    Line { id: u64, stderr: bool, line: String },
    /// A JSON-RPC message body.
    Json { id: u64, text: String },
    Exit { id: u64, code: i32 },
}

pub struct Spawn {
    pub command: String,
    pub args: Vec<String>,
    pub env: HashMap<String, String>,
    pub cwd: Option<String>,
    /// "pipe", "ignore" or "jsonrpc" per channel (stdin, stdout, stderr).
    pub stdio: [String; 3],
    pub jsonrpc: bool,
    pub shell: Option<String>,
}

pub struct Running {
    pub pid: u32,
    pub stdin: Option<ChildStdin>,
}

const CREATE_NO_WINDOW: u32 = 0x0800_0000;

fn path_dirs(env: &HashMap<String, String>) -> Vec<PathBuf> {
    let path = env
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case("PATH"))
        .map(|(_, v)| v.clone())
        .or_else(|| std::env::var("PATH").ok())
        .unwrap_or_default();
    let mut dirs: Vec<PathBuf> = std::env::split_paths(&path).collect();
    // Node installed per user (nvm, volta) or globally.
    for extra in [r"C:\Program Files\nodejs", r"C:\Program Files\Git\bin", r"C:\Program Files\Git\usr\bin"] {
        dirs.push(PathBuf::from(extra));
    }
    if let Some(appdata) = std::env::var_os("APPDATA") {
        dirs.push(PathBuf::from(appdata).join("npm"));
    }
    dirs
}

const EXTS: &[&str] = &["exe", "cmd", "bat", "com"];

/// Finds an executable: absolute paths get Windows extensions tried,
/// bare names are searched in PATH.
pub fn which(name: &str, env: &HashMap<String, String>) -> Option<PathBuf> {
    let p = Path::new(name);
    let candidates = |base: &Path| -> Option<PathBuf> {
        if base.extension().is_some() && base.is_file() {
            return Some(base.to_path_buf());
        }
        for ext in EXTS {
            let c = base.with_extension(ext);
            if c.is_file() {
                return Some(c);
            }
            // "name.js" should still find "name.js.cmd"-style launchers.
            let mut s = base.as_os_str().to_owned();
            s.push(format!(".{ext}"));
            let c = PathBuf::from(s);
            if c.is_file() {
                return Some(c);
            }
        }
        base.is_file().then(|| base.to_path_buf())
    };
    if p.is_absolute() || name.contains('/') || name.contains('\\') {
        return candidates(p);
    }
    path_dirs(env).iter().find_map(|d| candidates(&d.join(name)))
}

fn bash(env: &HashMap<String, String>) -> Option<PathBuf> {
    [r"C:\Program Files\Git\bin\bash.exe", r"C:\Program Files (x86)\Git\bin\bash.exe"]
        .iter()
        .map(PathBuf::from)
        .find(|p| p.is_file())
        .or_else(|| which("bash", env).filter(|p| !p.to_string_lossy().to_lowercase().contains("system32")))
}

/// Turns a macOS-style launch into (program, args).
pub fn resolve(command: &str, args: &[String], env: &mut HashMap<String, String>) -> Result<(PathBuf, Vec<String>), String> {
    let mut command = command.to_string();
    let mut args = args.to_vec();
    if matches!(command.trim_start_matches('/'), "usr/bin/env" | "bin/env" | "env") {
        // Leading VAR=value arguments set environment variables.
        while let Some(a) = args.first() {
            match a.split_once('=') {
                Some((k, v)) if !k.is_empty() && !k.contains('/') => {
                    env.insert(k.to_string(), v.to_string());
                    args.remove(0);
                }
                _ => break,
            }
        }
        if args.is_empty() {
            return Err(t!("/usr/bin/env ohne Programm").into());
        }
        command = args.remove(0);
    }
    // Unix shells: Git Bash if installed, otherwise cmd for "-c" scripts.
    let base = command.rsplit('/').next().unwrap_or(&command).to_string();
    if matches!(base.as_str(), "sh" | "bash" | "zsh") && command.starts_with('/') || matches!(command.as_str(), "sh" | "bash" | "zsh") {
        if let Some(b) = bash(env) {
            bash_env(env);
            return Ok((b, args));
        }
        if args.first().map(String::as_str) == Some("-c") && args.len() > 1 {
            return Ok((PathBuf::from("cmd.exe"), vec!["/D".into(), "/C".into(), args[1].clone()]));
        }
        return Err(t!("Für Shell-Skripte wird Git Bash benötigt.").into());
    }
    // Homebrew and system locations: look the tool up in PATH instead.
    for prefix in ["/usr/local/bin/", "/opt/homebrew/bin/", "/usr/bin/", "/bin/", "/opt/local/bin/"] {
        if let Some(rest) = command.strip_prefix(prefix) {
            command = rest.to_string();
        }
    }
    // MSYS paths from Git Bash: /c/Users/... and /mingw64/bin/git.
    if command.starts_with('/') && !Path::new(&command).exists() {
        let b = command.as_bytes();
        if b.len() > 2 && b[2] == b'/' && b[1].is_ascii_alphabetic() {
            command = format!("{}:{}", (b[1] as char).to_ascii_uppercase(), &command[2..]);
        } else if let Some(name) = command.rsplit('/').next().filter(|n| !n.is_empty()) {
            if which(name, env).is_some() {
                command = name.to_string();
            }
        }
    }
    // macOS `open` opens files and URLs with their default application.
    if command == "open" && which("open", env).is_none() {
        let mut a = vec!["/D".to_string(), "/C".into(), "start".into(), String::new()];
        a.extend(args.into_iter().filter(|a| !a.starts_with('-')));
        return Ok((PathBuf::from("cmd.exe"), a));
    }
    let lower = command.to_lowercase();
    if lower.ends_with(".js") || lower.ends_with(".mjs") || lower.ends_with(".cjs") {
        let node = which("node", env).ok_or(t!("Node.js wurde nicht gefunden."))?;
        let mut a = vec![command.clone()];
        a.extend(args);
        return Ok((node, a));
    }
    let exe = which(&command, env).ok_or_else(|| t!("Programm nicht gefunden: {command}", command = command))?;
    // `node <npm shim>`: run the shim's JavaScript target instead.
    let mut args = args;
    if is_node(&exe)
        && let Some(target) = args.first().and_then(|a| unshim(Path::new(a)))
    {
        args[0] = target.to_string_lossy().into_owned();
    }
    // Scripts with a shebang (shell scripts, npm shims without extension).
    let native = exe.extension().is_some_and(|e| EXTS.iter().any(|x| e.eq_ignore_ascii_case(x)));
    if !native {
        let mut first = String::new();
        if let Ok(f) = std::fs::File::open(&exe) {
            let _ = BufReader::new(f).take(200).read_line(&mut first);
        }
        if first.starts_with("#!") {
            if let Some(target) = unshim(&exe) {
                let node = which("node", env).ok_or(t!("Node.js wurde nicht gefunden."))?;
                let mut a = vec![target.to_string_lossy().into_owned()];
                a.extend(args);
                return Ok((node, a));
            }
            if first.contains("node") {
                let node = which("node", env).ok_or(t!("Node.js wurde nicht gefunden."))?;
                let mut a = vec![exe.to_string_lossy().into_owned()];
                a.extend(args);
                return Ok((node, a));
            }
            if let Some(b) = bash(env) {
                bash_env(env);
                let mut a = vec![exe.to_string_lossy().into_owned()];
                a.extend(args);
                return Ok((b, a));
            }
            return Err(t!("Für Shell-Skripte wird Git Bash benötigt (git-scm.com).").into());
        }
    }
    Ok((exe, args))
}

fn is_node(p: &Path) -> bool {
    p.file_stem().is_some_and(|s| s.eq_ignore_ascii_case("node"))
}

/// npm's `node_modules/.bin` entries on Windows are shell shims instead of
/// symlinks; returns the JavaScript file a shim runs.
pub fn unshim(path: &Path) -> Option<PathBuf> {
    if path.extension().is_some() {
        return None;
    }
    let text = std::fs::read_to_string(path).ok()?;
    if !text.starts_with("#!") || !text.contains("basedir") {
        return None;
    }
    // The shim mentions "$basedir/node" first; the target is the script.
    text.match_indices("\"$basedir/").find_map(|(i, m)| {
        let start = i + m.len();
        let rel = &text[start..start + text[start..].find('"')?];
        if !(rel.ends_with(".js") || rel.ends_with(".mjs") || rel.ends_with(".cjs")) {
            return None;
        }
        let target = path.parent()?.join(rel.replace('/', "\\"));
        target.is_file().then_some(target)
    })
}

const NODE_WRAPPER: &str = r#"#!/bin/bash
# Nebula: npm .bin entries are shell shims on Windows; run their JavaScript target.
p="${1//\\//}"
if [ -f "$p" ] && head -n 1 "$p" | grep -q '^#!' && grep -q 'basedir' "$p"; then
  target=$(grep -o '"$basedir/[^"]*\.[cm]\{0,1\}js"' "$p" | head -n 1 | sed 's/^"$basedir\///; s/"$//')
  if [ -n "$target" ] && [ -f "$(dirname "$p")/$target" ]; then
    first="$(dirname "$p")/$target"
    shift
    exec "$NEBULA_REAL_NODE" "$first" "$@"
  fi
fi
exec "$NEBULA_REAL_NODE" "$@"
"#;

/// Shell scripts call `node` with npm shims too: put a wrapper in front of
/// the real node that resolves them.
fn bash_env(env: &mut HashMap<String, String>) {
    let Some(node) = which("node", env) else { return };
    let dir = std::env::temp_dir().join("nebula-node-shim");
    let script = dir.join("node");
    if std::fs::read_to_string(&script).ok().as_deref() != Some(NODE_WRAPPER) {
        let _ = std::fs::create_dir_all(&dir);
        let _ = std::fs::write(&script, NODE_WRAPPER);
    }
    let path = env
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case("PATH"))
        .map(|(_, v)| v.clone())
        .or_else(|| std::env::var("PATH").ok())
        .unwrap_or_default();
    env.retain(|k, _| !k.eq_ignore_ascii_case("PATH"));
    env.insert("PATH".into(), format!("{};{path}", dir.display()));
    env.insert("NEBULA_REAL_NODE".into(), node.to_string_lossy().replace('\\', "/"));
}

pub fn spawn(id: u64, spec: Spawn, events: Sender<ProcEvent>, wake: impl Fn() + Send + Clone + 'static) -> Result<Running, String> {
    let mut env = spec.env.clone();
    let (program, args) = match &spec.shell {
        Some(shell) => {
            let line = std::iter::once(spec.command.clone()).chain(spec.args.iter().map(|a| shell_quote(a))).collect::<Vec<_>>().join(" ");
            match bash(&env).filter(|_| shell != "cmd") {
                Some(b) => (b, vec!["-lc".into(), line]),
                None => (PathBuf::from("cmd.exe"), vec!["/D".into(), "/C".into(), line]),
            }
        }
        None => resolve(&spec.command, &spec.args, &mut env)?,
    };
    let mut cmd = Command::new(&program);
    cmd.args(&args).envs(&env);
    if let Some(cwd) = spec.cwd.as_ref().filter(|c| Path::new(c).is_dir()) {
        cmd.current_dir(cwd);
    }
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    let pipe = |s: &str| if s == "ignore" { Stdio::null() } else { Stdio::piped() };
    cmd.stdin(pipe(&spec.stdio[0])).stdout(pipe(&spec.stdio[1])).stderr(pipe(&spec.stdio[2]));
    let mut child: Child = cmd.spawn().map_err(|e| format!("{}: {e}", program.display()))?;
    let pid = child.id();
    // Readers report when they hit EOF so the exit event comes after all output.
    let (done_tx, done_rx) = crossbeam_channel::unbounded::<()>();
    let mut readers = 0;

    if let Some(out) = child.stdout.take() {
        let tx = events.clone();
        let wake = wake.clone();
        let done = done_tx.clone();
        readers += 1;
        let jsonrpc = spec.jsonrpc;
        std::thread::spawn(move || {
            if jsonrpc {
                read_jsonrpc(id, out, &tx, &wake);
            } else {
                read_lines(id, false, out, &tx, &wake);
            }
            let _ = done.send(());
        });
    }
    if let Some(err) = child.stderr.take() {
        let tx = events.clone();
        let wake = wake.clone();
        let done = done_tx.clone();
        readers += 1;
        std::thread::spawn(move || {
            read_lines(id, true, err, &tx, &wake);
            let _ = done.send(());
        });
    }
    let stdin = child.stdin.take();
    std::thread::spawn(move || {
        let code = child.wait().ok().and_then(|s| s.code()).unwrap_or(-1);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        for _ in 0..readers {
            if done_rx.recv_deadline(deadline).is_err() {
                break;
            }
        }
        let _ = events.send(ProcEvent::Exit { id, code });
        wake();
    });
    Ok(Running { pid, stdin })
}

fn shell_quote(s: &str) -> String {
    if !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric() || "-_./=:@,+".contains(c)) {
        s.to_string()
    } else {
        format!("\"{}\"", s.replace('"', "\\\""))
    }
}

fn read_lines(id: u64, stderr: bool, r: impl Read, tx: &Sender<ProcEvent>, wake: &impl Fn()) {
    let mut reader = BufReader::new(r);
    let mut buf = Vec::new();
    loop {
        buf.clear();
        match reader.read_until(b'\n', &mut buf) {
            Ok(0) | Err(_) => break,
            Ok(_) => {
                let line = String::from_utf8_lossy(&buf).replace("\r\n", "\n");
                if tx.send(ProcEvent::Line { id, stderr, line }).is_err() {
                    break;
                }
                wake();
            }
        }
    }
}

/// Reads `Content-Length` framed JSON-RPC messages.
fn read_jsonrpc(id: u64, r: impl Read, tx: &Sender<ProcEvent>, wake: &impl Fn()) {
    let mut reader = BufReader::new(r);
    loop {
        let mut len = None;
        loop {
            let mut header = String::new();
            match reader.read_line(&mut header) {
                Ok(0) | Err(_) => return,
                Ok(_) => {}
            }
            let h = header.trim();
            if h.is_empty() {
                if len.is_some() {
                    break;
                }
                continue;
            }
            if let Some((k, v)) = h.split_once(':')
                && k.trim().eq_ignore_ascii_case("content-length")
            {
                len = v.trim().parse::<usize>().ok();
            }
        }
        let mut body = vec![0u8; len.unwrap_or(0)];
        if reader.read_exact(&mut body).is_err() {
            return;
        }
        if tx.send(ProcEvent::Json { id, text: String::from_utf8_lossy(&body).into_owned() }).is_err() {
            return;
        }
        wake();
    }
}

pub fn write_jsonrpc(stdin: &mut ChildStdin, body: &str) -> std::io::Result<()> {
    write!(stdin, "Content-Length: {}\r\n\r\n", body.len())?;
    stdin.write_all(body.as_bytes())?;
    stdin.flush()
}

/// Ends a process and its children.
pub fn kill(pid: u32) {
    use std::os::windows::process::CommandExt;
    let _ = Command::new("taskkill")
        .args(["/PID", &pid.to_string(), "/T", "/F"])
        .creation_flags(CREATE_NO_WINDOW)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn env_launch_resolves_through_path() {
        let mut env = HashMap::new();
        let (prog, args) = resolve("/usr/bin/env", &["FOO=1".into(), "cmd".into(), "/C".into(), "echo".into()], &mut env).unwrap();
        assert!(prog.to_string_lossy().to_lowercase().ends_with("cmd.exe"));
        assert_eq!(args, vec!["/C", "echo"]);
        assert_eq!(env["FOO"], "1");
    }

    #[test]
    fn npm_shims_resolve_to_scripts() {
        let tmp = tempfile::tempdir().unwrap();
        let bin = tmp.path().join("node_modules").join(".bin");
        std::fs::create_dir_all(&bin).unwrap();
        std::fs::create_dir_all(tmp.path().join("node_modules").join("pkg")).unwrap();
        std::fs::write(tmp.path().join("node_modules/pkg/cli.js"), "").unwrap();
        std::fs::write(bin.join("pkg"), "#!/bin/sh
basedir=x
if [ -x \"$basedir/node\" ]; then
  exec \"$basedir/node\"  \"$basedir/../pkg/cli.js\" \"$@\"
fi
").unwrap();
        let target = unshim(&bin.join("pkg")).unwrap();
        assert!(target.ends_with("cli.js"));
    }

    #[test]
    fn runs_and_reads_lines() {
        let (tx, rx) = crossbeam_channel::unbounded();
        let spec = Spawn {
            command: "cmd".into(),
            args: vec!["/C".into(), "echo hallo".into()],
            env: HashMap::new(),
            cwd: None,
            stdio: ["pipe".into(), "pipe".into(), "pipe".into()],
            jsonrpc: false,
            shell: None,
        };
        spawn(1, spec, tx, || {}).unwrap();
        let mut out = String::new();
        let mut exited = false;
        while let Ok(ev) = rx.recv_timeout(std::time::Duration::from_secs(10)) {
            match ev {
                ProcEvent::Line { line, .. } => out.push_str(&line),
                ProcEvent::Exit { code, .. } => {
                    assert_eq!(code, 0);
                    exited = true;
                    break;
                }
                _ => {}
            }
        }
        assert!(exited);
        assert_eq!(out.trim(), "hallo");
    }
}
