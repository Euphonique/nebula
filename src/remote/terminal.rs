//! Interactive shell on a server over SSH. The output is fed into a VT100
//! emulator that the terminal panel draws.

use std::sync::{Arc, Mutex};

use crossbeam_channel::{Receiver, Sender};
use eframe::egui;
use russh::ChannelMsg;

use super::HostKeyPrompt;
use super::sftp::{PromptFn, RUNTIME, open_session};
use crate::settings::Site;

/// A shell installed on this computer.
#[derive(Clone, Debug, PartialEq)]
pub struct LocalShell {
    pub name: String,
    pub program: String,
    pub args: Vec<String>,
}

/// Shells found on this computer, the usual default first.
pub fn local_shells() -> Vec<LocalShell> {
    let shell = |name: &str, program: String, args: &[&str]| LocalShell {
        name: name.to_string(),
        program,
        args: args.iter().map(|a| a.to_string()).collect(),
    };
    let in_path = |exe: &str| {
        std::env::var_os("PATH").and_then(|paths| std::env::split_paths(&paths).map(|p| p.join(exe)).find(|p| p.is_file()))
    };
    let mut out = Vec::new();
    if let Some(p) = in_path("pwsh.exe") {
        out.push(shell("PowerShell", p.to_string_lossy().into_owned(), &["-NoLogo"]));
    }
    let windows = std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".into());
    let ps = format!(r"{windows}\System32\WindowsPowerShell\v1.0\powershell.exe");
    if std::path::Path::new(&ps).is_file() {
        let name = if out.is_empty() { "PowerShell" } else { "Windows PowerShell" };
        out.push(shell(name, ps, &["-NoLogo"]));
    }
    // Git Bash: next to git.exe (…\Git\cmd\git.exe -> …\Git\bin\bash.exe) or in the default folder.
    let git_bash = in_path("git.exe")
        .and_then(|git| git.parent()?.parent().map(|g| g.join(r"bin\bash.exe")))
        .filter(|p| p.is_file())
        .or_else(|| {
            let p = std::path::PathBuf::from(r"C:\Program Files\Git\bin\bash.exe");
            p.is_file().then_some(p)
        });
    if let Some(bash) = git_bash {
        out.push(shell("Git Bash", bash.to_string_lossy().into_owned(), &["--login", "-i"]));
    }
    out.push(shell("Eingabeaufforderung", format!(r"{windows}\System32\cmd.exe"), &[]));
    out
}

/// Lines kept above the screen.
const SCROLLBACK: usize = 5000;

pub enum TermEvent {
    /// The server's host key is not trusted yet; the UI asks.
    HostKey(HostKeyPrompt),
    Connected,
    /// The shell ended; with an error message if it failed.
    Closed(Option<String>),
}

enum Input {
    Data(Vec<u8>),
    Resize(u16, u16),
}

#[derive(Clone, PartialEq)]
pub enum TermStatus {
    Connecting,
    Connected,
    Closed(Option<String>),
}

pub struct Terminal {
    pub id: u64,
    pub site_id: String,
    pub status: TermStatus,
    pub parser: Arc<Mutex<vt100::Parser>>,
    input: tokio::sync::mpsc::UnboundedSender<Input>,
    events: Receiver<TermEvent>,
    size: (u16, u16),
}

static NEXT_ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

impl Terminal {
    /// Connects to `site` and starts a login shell of `rows` x `cols`.
    pub fn open(site: &Site, ctx: &egui::Context, rows: u16, cols: u16) -> Self {
        Self::open_with(site, ctx, rows, cols, None)
    }

    /// Like [`Self::open`]; `password` replaces the one stored for the site.
    pub fn open_with(site: &Site, ctx: &egui::Context, rows: u16, cols: u16, password: Option<String>) -> Self {
        let (rows, cols) = (rows.max(2), cols.max(10));
        let parser = Arc::new(Mutex::new(vt100::Parser::new(rows, cols, SCROLLBACK)));
        let (input, input_rx) = tokio::sync::mpsc::unbounded_channel();
        let (events_tx, events) = crossbeam_channel::unbounded();
        let site = site.clone();
        let ctx = ctx.clone();
        let out = parser.clone();
        RUNTIME.spawn(async move {
            let result = run(site, password, ctx.clone(), out, input_rx, events_tx.clone(), rows, cols).await;
            let _ = events_tx.send(TermEvent::Closed(result.err().map(|e| format!("{e:#}"))));
            ctx.request_repaint();
        });
        Self {
            id: NEXT_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
            site_id: String::new(),
            status: TermStatus::Connecting,
            parser,
            input,
            events,
            size: (rows, cols),
        }
    }

    /// Starts a local shell in a Windows pseudo console (ConPTY) in `cwd`.
    pub fn open_local(shell: &LocalShell, cwd: Option<&std::path::Path>, ctx: &egui::Context, rows: u16, cols: u16) -> Self {
        use portable_pty::{CommandBuilder, PtySize, native_pty_system};
        let (rows, cols) = (rows.max(2), cols.max(10));
        let parser = Arc::new(Mutex::new(vt100::Parser::new(rows, cols, SCROLLBACK)));
        let (input, mut input_rx) = tokio::sync::mpsc::unbounded_channel::<Input>();
        let (events_tx, events) = crossbeam_channel::unbounded();
        let mut term = Self {
            id: NEXT_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
            site_id: String::new(),
            status: TermStatus::Connected,
            parser: parser.clone(),
            input,
            events,
            size: (rows, cols),
        };
        let started = (|| -> anyhow::Result<()> {
            let pair = native_pty_system().openpty(PtySize { rows, cols, pixel_width: 0, pixel_height: 0 })?;
            let mut cmd = CommandBuilder::new(&shell.program);
            cmd.args(&shell.args);
            if let Some(dir) = cwd {
                cmd.cwd(dir);
            }
            cmd.env("TERM", "xterm-256color");
            cmd.env("COLORTERM", "truecolor");
            let mut child = pair.slave.spawn_command(cmd)?;
            drop(pair.slave);
            let mut reader = pair.master.try_clone_reader()?;
            let mut writer = pair.master.take_writer()?;
            let mut killer = child.clone_killer();
            let master = Arc::new(Mutex::new(Some(pair.master)));

            // Output into the emulator until the pseudo console closes.
            let ctx2 = ctx.clone();
            let answer = term.input.clone();
            std::thread::spawn(move || {
                let mut buf = [0u8; 8192];
                while let Ok(n) = std::io::Read::read(&mut reader, &mut buf) {
                    if n == 0 {
                        break;
                    }
                    let data = &buf[..n];
                    let mut p = parser.lock().unwrap();
                    p.process(data);
                    // ConPTY asks for the cursor position (ESC[6n) and waits for the answer.
                    if data.windows(4).any(|w| w == b"\x1b[6n") {
                        let (row, col) = p.screen().cursor_position();
                        let _ = answer.send(Input::Data(format!("\x1b[{};{}R", row + 1, col + 1).into_bytes()));
                    }
                    drop(p);
                    ctx2.request_repaint();
                }
            });
            // The shell exited: close the pseudo console so the reader ends too.
            let (master2, ctx2, events_tx2) = (master.clone(), ctx.clone(), events_tx.clone());
            std::thread::spawn(move || {
                let status = child.wait();
                master2.lock().unwrap().take();
                let error = match status {
                    Ok(s) if !s.success() => Some(format!("Beendet mit Code {}", s.exit_code())),
                    Err(e) => Some(e.to_string()),
                    _ => None,
                };
                let _ = events_tx2.send(TermEvent::Closed(error));
                ctx2.request_repaint();
            });
            // Keys and size changes; ends (and stops the shell) when the panel drops the terminal.
            std::thread::spawn(move || {
                while let Some(i) = input_rx.blocking_recv() {
                    match i {
                        Input::Data(bytes) => {
                            if std::io::Write::write_all(&mut writer, &bytes).is_err() {
                                break;
                            }
                        }
                        Input::Resize(rows, cols) => {
                            if let Some(m) = master.lock().unwrap().as_ref() {
                                let _ = m.resize(PtySize { rows, cols, pixel_width: 0, pixel_height: 0 });
                            }
                        }
                    }
                }
                let _ = killer.kill();
            });
            Ok(())
        })();
        if let Err(e) = started {
            term.status = TermStatus::Closed(Some(format!("{} konnte nicht gestartet werden: {e:#}", shell.name)));
        }
        term
    }

    pub fn send(&self, bytes: impl Into<Vec<u8>>) {
        let _ = self.input.send(Input::Data(bytes.into()));
    }

    /// Adapts the screen and the server's pty to a new size.
    pub fn resize(&mut self, rows: u16, cols: u16) {
        let (rows, cols) = (rows.max(2), cols.max(10));
        if self.size == (rows, cols) {
            return;
        }
        self.size = (rows, cols);
        self.parser.lock().unwrap().screen_mut().set_size(rows, cols);
        let _ = self.input.send(Input::Resize(rows, cols));
    }

    /// Applies events from the connection; returns host key questions for the UI.
    pub fn poll(&mut self) -> Vec<HostKeyPrompt> {
        let mut prompts = Vec::new();
        for ev in self.events.try_iter() {
            match ev {
                TermEvent::HostKey(p) => prompts.push(p),
                TermEvent::Connected => self.status = TermStatus::Connected,
                TermEvent::Closed(err) => self.status = TermStatus::Closed(err),
            }
        }
        prompts
    }
}

async fn run(
    site: Site,
    password: Option<String>,
    ctx: egui::Context,
    parser: Arc<Mutex<vt100::Parser>>,
    mut input: tokio::sync::mpsc::UnboundedReceiver<Input>,
    events: Sender<TermEvent>,
    rows: u16,
    cols: u16,
) -> anyhow::Result<()> {
    // The keyring lookup blocks; keep it off the async workers.
    let password = match password {
        Some(p) => p,
        None => {
            let site = site.clone();
            tokio::task::spawn_blocking(move || site.password()).await?.unwrap_or_default()
        }
    };
    let prompt = {
        let (events, ctx) = (events.clone(), ctx.clone());
        move |p: HostKeyPrompt| {
            let _ = events.send(TermEvent::HostKey(p));
            ctx.request_repaint();
        }
    };
    let session = open_session(&site, &password, Arc::new(Box::new(prompt) as PromptFn)).await?;
    let channel = session.channel_open_session().await?;
    channel.request_pty(false, "xterm-256color", cols as u32, rows as u32, 0, 0, &[]).await?;
    channel.request_shell(false).await?;
    let _ = events.send(TermEvent::Connected);
    ctx.request_repaint();

    let (mut read, write) = channel.split();
    let reader = {
        let ctx = ctx.clone();
        async move {
            while let Some(msg) = read.wait().await {
                match msg {
                    ChannelMsg::Data { data } | ChannelMsg::ExtendedData { data, .. } => {
                        parser.lock().unwrap().process(&data);
                        ctx.request_repaint();
                    }
                    ChannelMsg::Eof | ChannelMsg::Close => break,
                    _ => {}
                }
            }
        }
    };
    let writer = async move {
        while let Some(i) = input.recv().await {
            let sent = match i {
                Input::Data(bytes) => write.data(&bytes[..]).await,
                Input::Resize(rows, cols) => write.window_change(cols as u32, rows as u32, 0, 0).await,
            };
            if sent.is_err() {
                break;
            }
        }
        let _ = write.close().await;
    };
    // Ends when the shell exits or the panel drops the terminal.
    tokio::select! {
        _ = reader => {}
        _ = writer => {}
    }
    drop(session);
    Ok(())
}

/// Bytes a key press sends to the shell, if any.
pub fn key_bytes(key: egui::Key, mods: egui::Modifiers, app_cursor: bool) -> Option<Vec<u8>> {
    use egui::Key;
    let csi = |s: &str| Some(format!("\x1b[{s}").into_bytes());
    let cursor = |c: char| {
        // Arrows with modifiers: CSI 1;m X (xterm).
        let m = 1 + u8::from(mods.shift) + 2 * u8::from(mods.alt) + 4 * u8::from(mods.ctrl);
        if m > 1 {
            Some(format!("\x1b[1;{m}{c}").into_bytes())
        } else if app_cursor {
            Some(format!("\x1bO{c}").into_bytes())
        } else {
            Some(format!("\x1b[{c}").into_bytes())
        }
    };
    if mods.ctrl && !mods.alt {
        // Ctrl+letter: control characters (^A = 1 …).
        let name = key.name();
        if name.len() == 1 && name.as_bytes()[0].is_ascii_alphabetic() {
            return Some(vec![name.as_bytes()[0].to_ascii_uppercase() - b'@']);
        }
    }
    match key {
        Key::Enter => Some(b"\r".to_vec()),
        Key::Backspace => Some(if mods.ctrl { vec![0x08] } else { vec![0x7f] }),
        Key::Tab if mods.shift => csi("Z"),
        Key::Tab => Some(b"\t".to_vec()),
        Key::Escape => Some(vec![0x1b]),
        Key::ArrowUp => cursor('A'),
        Key::ArrowDown => cursor('B'),
        Key::ArrowRight => cursor('C'),
        Key::ArrowLeft => cursor('D'),
        Key::Home => cursor('H'),
        Key::End => cursor('F'),
        Key::Insert => csi("2~"),
        Key::Delete => csi("3~"),
        Key::PageUp => csi("5~"),
        Key::PageDown => csi("6~"),
        Key::F1 => Some(b"\x1bOP".to_vec()),
        Key::F2 => Some(b"\x1bOQ".to_vec()),
        Key::F3 => Some(b"\x1bOR".to_vec()),
        Key::F4 => Some(b"\x1bOS".to_vec()),
        Key::F5 => csi("15~"),
        Key::F6 => csi("17~"),
        Key::F7 => csi("18~"),
        Key::F8 => csi("19~"),
        Key::F9 => csi("20~"),
        Key::F10 => csi("21~"),
        Key::F11 => csi("23~"),
        Key::F12 => csi("24~"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use eframe::egui::{Key, Modifiers};

    #[test]
    fn keys_map_to_terminal_sequences() {
        assert_eq!(key_bytes(Key::C, Modifiers::CTRL, false), Some(vec![3]));
        assert_eq!(key_bytes(Key::ArrowUp, Modifiers::NONE, false), Some(b"\x1b[A".to_vec()));
        assert_eq!(key_bytes(Key::ArrowUp, Modifiers::NONE, true), Some(b"\x1bOA".to_vec()));
        assert_eq!(key_bytes(Key::ArrowRight, Modifiers::CTRL, false), Some(b"\x1b[1;5C".to_vec()));
        assert_eq!(key_bytes(Key::Enter, Modifiers::NONE, false), Some(b"\r".to_vec()));
        assert_eq!(key_bytes(Key::A, Modifiers::NONE, false), None);
    }
}
