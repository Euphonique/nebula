//! Interactive shell on a server over SSH. The output is fed into a VT100
//! emulator that the terminal panel draws.

use std::sync::{Arc, Mutex};

use crossbeam_channel::{Receiver, Sender};
use eframe::egui;
use russh::ChannelMsg;

use super::HostKeyPrompt;
use super::sftp::{PromptFn, RUNTIME, open_session};
use crate::settings::Site;

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
