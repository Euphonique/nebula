//! Remote file access over FTP, FTPS and SFTP.
//!
//! Each connected site gets its own worker thread that owns the connection and
//! processes requests in order. Results are delivered to the UI through a
//! shared event channel. A dropped connection is re-established transparently
//! and the failed request retried once.

mod ftp;
mod sftp;
pub mod terminal;
#[cfg(test)]
mod integration_tests;

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use anyhow::Result;
use crossbeam_channel::{Receiver, Sender};
use eframe::egui;

use crate::settings::{Protocol, Site};

/// Idle time after which a connection is pinged to keep it alive.
const KEEPALIVE: Duration = Duration::from_secs(45);

#[derive(Clone, Debug)]
pub struct RemoteEntry {
    pub name: String,
    pub path: String,
    pub is_dir: bool,
    pub size: u64,
    /// Unix permission bits (`0o755`), if the server reports them.
    pub mode: Option<u32>,
}

/// Operations every protocol backend supports. Paths are absolute and `/`-separated.
pub trait Backend: Send {
    fn home(&mut self) -> Result<String>;
    fn list(&mut self, path: &str) -> Result<Vec<RemoteEntry>>;
    fn read(&mut self, path: &str) -> Result<Vec<u8>>;
    fn write(&mut self, path: &str, data: &[u8]) -> Result<()>;
    fn mkdir(&mut self, path: &str) -> Result<()>;
    fn remove_file(&mut self, path: &str) -> Result<()>;
    fn remove_dir(&mut self, path: &str) -> Result<()>;
    fn rename(&mut self, from: &str, to: &str) -> Result<()>;
    /// Sets the Unix permission bits.
    fn chmod(&mut self, path: &str, mode: u32) -> Result<()>;
    /// Cheap request used to detect dead connections.
    fn ping(&mut self) -> Result<()>;
    /// Size and modification time, or `None` if the file does not exist.
    fn stat(&mut self, path: &str) -> Result<Option<Stamp>>;
}

/// Identifies a version of a remote file for conflict detection.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Stamp {
    pub size: u64,
    /// Seconds since the Unix epoch, if the server reports it.
    pub mtime: Option<i64>,
}

impl Stamp {
    /// True if `other` is plausibly the same version.
    pub fn matches(&self, other: &Stamp) -> bool {
        self.size == other.size
            && match (self.mtime, other.mtime) {
                (Some(a), Some(b)) => a == b,
                _ => true,
            }
    }
}

#[derive(Clone, Debug)]
pub enum Request {
    List(String),
    Read(String),
    /// Upload editor content. With `expected`, the upload is refused with
    /// [`Reply::Conflict`] if the server file changed since it was opened.
    Write { path: String, data: Arc<Vec<u8>>, expected: Option<Stamp> },
    CreateFile(String),
    Mkdir(String),
    Delete { path: String, is_dir: bool },
    Rename { from: String, to: String },
    /// Upload a local file or folder (recursively) to `remote`.
    Upload { local: std::path::PathBuf, remote: String },
    /// Download a remote file or folder (recursively) to `local`.
    Download { remote: String, local: std::path::PathBuf, is_dir: bool },
    /// Copy within the same server (download + upload).
    Copy { from: String, to: String, is_dir: bool },
    /// Change permissions; for folders optionally of everything inside too.
    Chmod { path: String, mode: u32, is_dir: bool, recurse: Option<Recurse> },
}

/// What a recursive permission change applies to below a folder.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Recurse {
    All,
    Files,
    Folders,
}

impl Request {
    fn describe(&self) -> String {
        match self {
            Request::List(p) => format!("Auflisten von {p}"),
            Request::Read(p) => format!("Laden von {p}"),
            Request::Write { path, .. } => format!("Hochladen von {path}"),
            Request::CreateFile(p) => format!("Anlegen von {p}"),
            Request::Mkdir(p) => format!("Anlegen von {p}"),
            Request::Delete { path, .. } => format!("Löschen von {path}"),
            Request::Rename { from, .. } => format!("Umbenennen von {from}"),
            Request::Upload { local, .. } => format!("Hochladen von {}", local.display()),
            Request::Download { remote, .. } => format!("Herunterladen von {remote}"),
            Request::Copy { from, .. } => format!("Kopieren von {from}"),
            Request::Chmod { path, .. } => format!("Ändern der Berechtigungen von {path}"),
        }
    }
}

pub enum Reply {
    Connected { home: String },
    Listed { path: String, entries: Vec<RemoteEntry> },
    Read { path: String, data: Vec<u8>, stamp: Option<Stamp> },
    Written { path: String, bytes: usize, stamp: Option<Stamp> },
    /// The server file changed since it was opened; nothing was written.
    Conflict { path: String },
    /// An upload/download/copy finished.
    Transferred { message: String, remote_dir: Option<String>, local_dir: Option<std::path::PathBuf> },
    /// A create/mkdir/delete/rename finished; `dir` is the directory to refresh.
    Changed { dir: String, message: String },
    Failed { request: Request, message: String },
    ConnectFailed { message: String },
    Disconnected,
    /// SFTP host key not yet trusted; the UI must answer through `answer`.
    HostKeyUnknown(HostKeyPrompt),
}

pub struct HostKeyPrompt {
    pub host: String,
    pub port: u16,
    pub algorithm: String,
    pub fingerprint: String,
    pub changed: bool,
    answer: Arc<Mutex<Option<tokio::sync::oneshot::Sender<bool>>>>,
}

impl HostKeyPrompt {
    pub fn answer(&self, accept: bool) {
        if let Some(tx) = self.answer.lock().unwrap().take() {
            let _ = tx.send(accept);
        }
    }
}

pub struct RemoteEvent {
    pub site_id: String,
    pub tag: u64,
    pub reply: Reply,
}

enum Command {
    Run { tag: u64, request: Request },
    Disconnect,
}

struct Connection {
    tx: Sender<Command>,
    busy: Arc<std::sync::atomic::AtomicUsize>,
}

/// Owns all site connections.
pub struct RemoteManager {
    connections: HashMap<String, Connection>,
    events_tx: Sender<RemoteEvent>,
    events_rx: Receiver<RemoteEvent>,
    ctx: egui::Context,
    next_tag: u64,
}

impl RemoteManager {
    pub fn new(ctx: egui::Context) -> Self {
        let (events_tx, events_rx) = crossbeam_channel::unbounded();
        Self {
            connections: HashMap::new(),
            events_tx,
            events_rx,
            ctx,
            next_tag: 1,
        }
    }

    pub fn is_connected(&self, site_id: &str) -> bool {
        self.connections.contains_key(site_id)
    }

    pub fn total_pending(&self) -> usize {
        self.connections
            .values()
            .map(|c| c.busy.load(std::sync::atomic::Ordering::Relaxed))
            .sum()
    }

    /// Starts a connection for `site` if there is none yet.
    pub fn connect(&mut self, site: &Site) {
        if self.connections.contains_key(&site.id) {
            return;
        }
        let (tx, rx) = crossbeam_channel::unbounded();
        let busy = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let worker = Worker {
            site: site.clone(),
            password: site.password(),
            backend: None,
            events: self.events_tx.clone(),
            ctx: self.ctx.clone(),
            busy: busy.clone(),
        };
        thread::Builder::new()
            .name(format!("remote-{}", site.host))
            .spawn(move || worker.run(rx))
            .expect("spawn remote worker");
        self.connections
            .insert(site.id.clone(), Connection { tx, busy });
    }

    pub fn disconnect(&mut self, site_id: &str) {
        if let Some(conn) = self.connections.remove(site_id) {
            let _ = conn.tx.send(Command::Disconnect);
        }
    }

    /// Queues a request; returns a tag that identifies the reply.
    pub fn send(&mut self, site: &Site, request: Request) -> u64 {
        self.connect(site);
        let tag = self.next_tag;
        self.next_tag += 1;
        let conn = &self.connections[&site.id];
        conn.busy.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        if conn.tx.send(Command::Run { tag, request }).is_err() {
            conn.busy.fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
        }
        tag
    }

    pub fn poll(&self) -> impl Iterator<Item = RemoteEvent> + '_ {
        self.events_rx.try_iter()
    }

    /// Removes connections whose worker has stopped.
    pub fn forget(&mut self, site_id: &str) {
        self.connections.remove(site_id);
    }
}

struct Worker {
    site: Site,
    password: Option<String>,
    backend: Option<Box<dyn Backend>>,
    events: Sender<RemoteEvent>,
    ctx: egui::Context,
    busy: Arc<std::sync::atomic::AtomicUsize>,
}

impl Worker {
    fn emit(&self, tag: u64, reply: Reply) {
        let _ = self.events.send(RemoteEvent {
            site_id: self.site.id.clone(),
            tag,
            reply,
        });
        self.ctx.request_repaint();
    }

    fn open(&mut self) -> Result<()> {
        let password = self.password.clone().unwrap_or_default();
        let mut backend: Box<dyn Backend> = match self.site.protocol {
            Protocol::Ftp | Protocol::Ftps => Box::new(ftp::FtpBackend::connect(&self.site, &password)?),
            Protocol::Sftp => {
                let events = self.events.clone();
                let ctx = self.ctx.clone();
                let site_id = self.site.id.clone();
                let prompt = move |p: HostKeyPrompt| {
                    let _ = events.send(RemoteEvent {
                        site_id: site_id.clone(),
                        tag: 0,
                        reply: Reply::HostKeyUnknown(p),
                    });
                    ctx.request_repaint();
                };
                Box::new(sftp::SftpBackend::connect(&self.site, &password, Box::new(prompt))?)
            }
        };
        let home = if self.site.root_path.trim().is_empty() {
            backend.home()?
        } else {
            normalize(self.site.root_path.trim())
        };
        self.backend = Some(backend);
        self.emit(0, Reply::Connected { home });
        Ok(())
    }

    fn run(mut self, rx: Receiver<Command>) {
        if let Err(e) = self.open() {
            self.emit(0, Reply::ConnectFailed { message: format!("{e:#}") });
            // Fail everything still queued so the UI doesn't wait forever.
            for cmd in rx.try_iter() {
                if let Command::Run { tag, request } = cmd {
                    self.busy.fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
                    self.emit(tag, Reply::Failed { request, message: "Keine Verbindung".into() });
                }
            }
            return;
        }

        loop {
            let cmd = match rx.recv_timeout(KEEPALIVE) {
                Ok(cmd) => cmd,
                Err(crossbeam_channel::RecvTimeoutError::Timeout) => {
                    // Keep idle connections open; reconnect lazily if this fails.
                    if let Some(b) = self.backend.as_mut()
                        && b.ping().is_err()
                    {
                        self.backend = None;
                    }
                    continue;
                }
                Err(crossbeam_channel::RecvTimeoutError::Disconnected) => break,
            };
            let (tag, request) = match cmd {
                Command::Disconnect => break,
                Command::Run { tag, request } => (tag, request),
            };
            let mut result = self.execute(&request);
            if let Err(e) = &result
                && is_connection_error(e)
            {
                // Reconnect once and retry.
                self.backend = None;
                result = match self.open() {
                    Ok(()) => self.execute(&request),
                    Err(e) => Err(e),
                };
            }
            self.busy.fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
            match result {
                Ok(reply) => self.emit(tag, reply),
                Err(e) => {
                    let message = format!("{} fehlgeschlagen: {e:#}", request.describe());
                    self.emit(tag, Reply::Failed { request, message });
                }
            }
        }
        self.emit(0, Reply::Disconnected);
    }

    fn execute(&mut self, request: &Request) -> Result<Reply> {
        if self.backend.is_none() {
            self.open()?;
        }
        let b = self.backend.as_mut().unwrap();
        Ok(match request {
            Request::List(path) => {
                let mut entries = b.list(path)?;
                entries.sort_by(|a, b| {
                    b.is_dir
                        .cmp(&a.is_dir)
                        .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
                });
                Reply::Listed { path: path.clone(), entries }
            }
            Request::Read(path) => {
                let data = b.read(path)?;
                let stamp = b.stat(path).ok().flatten();
                Reply::Read { path: path.clone(), data, stamp }
            }
            Request::Write { path, data, expected } => {
                if let Some(expected) = expected
                    && let Ok(Some(current)) = b.stat(path)
                    && !current.matches(expected)
                {
                    return Ok(Reply::Conflict { path: path.clone() });
                }
                b.write(path, data)?;
                let stamp = b.stat(path).ok().flatten();
                Reply::Written { path: path.clone(), bytes: data.len(), stamp }
            }
            Request::Upload { local, remote } => {
                let n = upload_recursive(b.as_mut(), local, remote)?;
                Reply::Transferred {
                    message: format!("{n} Datei(en) hochgeladen"),
                    remote_dir: Some(parent(remote)),
                    local_dir: None,
                }
            }
            Request::Download { remote, local, is_dir } => {
                let n = download_recursive(b.as_mut(), remote, local, *is_dir)?;
                Reply::Transferred {
                    message: format!("{n} Datei(en) heruntergeladen"),
                    remote_dir: None,
                    local_dir: local.parent().map(|p| p.to_path_buf()),
                }
            }
            Request::Copy { from, to, is_dir } => {
                let n = copy_recursive(b.as_mut(), from, to, *is_dir)?;
                Reply::Transferred {
                    message: format!("{n} Datei(en) kopiert"),
                    remote_dir: Some(parent(to)),
                    local_dir: None,
                }
            }
            Request::CreateFile(path) => {
                b.write(path, &[])?;
                Reply::Changed { dir: parent(path), message: format!("{path} angelegt") }
            }
            Request::Mkdir(path) => {
                b.mkdir(path)?;
                Reply::Changed { dir: parent(path), message: format!("{path} angelegt") }
            }
            Request::Delete { path, is_dir } => {
                if *is_dir {
                    remove_recursive(b.as_mut(), path)?;
                } else {
                    b.remove_file(path)?;
                }
                Reply::Changed { dir: parent(path), message: format!("{path} gelöscht") }
            }
            Request::Rename { from, to } => {
                b.rename(from, to)?;
                Reply::Changed { dir: parent(to), message: format!("{from} → {to}") }
            }
            Request::Chmod { path, mode, is_dir, recurse } => {
                // "Only files" below a folder leaves the folder itself alone.
                let mut n = 0;
                if !(*is_dir && *recurse == Some(Recurse::Files)) {
                    b.chmod(path, *mode)?;
                    n += 1;
                }
                if let (true, Some(scope)) = (*is_dir, recurse) {
                    n += chmod_recursive(b.as_mut(), path, *mode, *scope)?;
                }
                let what = if n == 1 { path.clone() } else { format!("{n} Objekte") };
                Reply::Changed { dir: parent(path), message: format!("Berechtigungen von {what} auf {} gesetzt", mode_string(*mode)) }
            }
        })
    }
}

fn upload_recursive(b: &mut dyn Backend, local: &std::path::Path, remote: &str) -> Result<usize> {
    if local.is_dir() {
        // The folder may already exist; a real failure shows up on the files.
        let _ = b.mkdir(remote);
        let mut n = 0;
        for entry in std::fs::read_dir(local)? {
            let entry = entry?;
            let name = entry.file_name().to_string_lossy().into_owned();
            n += upload_recursive(b, &entry.path(), &join(remote, &name))?;
        }
        Ok(n)
    } else {
        let data = std::fs::read(local)?;
        if b.write(remote, &data).is_err() {
            // The target folder may not exist yet: create its parents and retry.
            let mut dir = String::new();
            for part in parent(remote).split('/').filter(|p| !p.is_empty()) {
                dir = format!("{dir}/{part}");
                let _ = b.mkdir(&dir);
            }
            b.write(remote, &data)?;
        }
        Ok(1)
    }
}

fn download_recursive(b: &mut dyn Backend, remote: &str, local: &std::path::Path, is_dir: bool) -> Result<usize> {
    if is_dir {
        std::fs::create_dir_all(local)?;
        let mut n = 0;
        for e in b.list(remote)? {
            n += download_recursive(b, &e.path, &local.join(&e.name), e.is_dir)?;
        }
        Ok(n)
    } else {
        let data = b.read(remote)?;
        std::fs::write(local, data)?;
        Ok(1)
    }
}

fn copy_recursive(b: &mut dyn Backend, from: &str, to: &str, is_dir: bool) -> Result<usize> {
    if is_dir {
        let _ = b.mkdir(to);
        let mut n = 0;
        for e in b.list(from)? {
            n += copy_recursive(b, &e.path, &join(to, &e.name), e.is_dir)?;
        }
        Ok(n)
    } else {
        let data = b.read(from)?;
        b.write(to, &data)?;
        Ok(1)
    }
}

fn chmod_recursive(b: &mut dyn Backend, dir: &str, mode: u32, scope: Recurse) -> Result<usize> {
    let mut n = 0;
    for e in b.list(dir)? {
        if e.is_dir && scope != Recurse::Files || !e.is_dir && scope != Recurse::Folders {
            b.chmod(&e.path, mode)?;
            n += 1;
        }
        if e.is_dir {
            n += chmod_recursive(b, &e.path, mode, scope)?;
        }
    }
    Ok(n)
}

/// `rwxr-xr-x (755)`
pub fn mode_string(mode: u32) -> String {
    let mut s = String::with_capacity(9);
    for shift in [6, 3, 0] {
        let bits = (mode >> shift) & 7;
        s.push(if bits & 4 != 0 { 'r' } else { '-' });
        s.push(if bits & 2 != 0 { 'w' } else { '-' });
        s.push(if bits & 1 != 0 { 'x' } else { '-' });
    }
    format!("{s} ({:03o})", mode & 0o777)
}

fn remove_recursive(b: &mut dyn Backend, path: &str) -> Result<()> {
    for entry in b.list(path)? {
        if entry.is_dir {
            remove_recursive(b, &entry.path)?;
        } else {
            b.remove_file(&entry.path)?;
        }
    }
    b.remove_dir(path)
}

fn is_connection_error(e: &anyhow::Error) -> bool {
    e.chain().any(|cause| {
        if let Some(io) = cause.downcast_ref::<std::io::Error>() {
            use std::io::ErrorKind::*;
            return matches!(
                io.kind(),
                BrokenPipe | ConnectionReset | ConnectionAborted | NotConnected | UnexpectedEof | TimedOut
            );
        }
        let text = cause.to_string().to_lowercase();
        text.contains("connection") && (text.contains("closed") || text.contains("reset"))
            || text.contains("disconnected")
            || text.contains("channel closed")
            || text.contains("421")
    })
}

/// Normalizes a remote path: forward slashes, leading `/`, no trailing `/`.
pub fn normalize(path: &str) -> String {
    let path = path.replace('\\', "/");
    let mut parts: Vec<&str> = Vec::new();
    for part in path.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            p => parts.push(p),
        }
    }
    format!("/{}", parts.join("/"))
}

pub fn join(dir: &str, name: &str) -> String {
    if dir.ends_with('/') {
        format!("{dir}{name}")
    } else {
        format!("{dir}/{name}")
    }
}

pub fn parent(path: &str) -> String {
    match path.trim_end_matches('/').rsplit_once('/') {
        Some(("", _)) | None => "/".into(),
        Some((p, _)) => p.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths() {
        assert_eq!(normalize("httpdocs/./css/../js/"), "/httpdocs/js");
        assert_eq!(parent("/a/b.css"), "/a");
        assert_eq!(parent("/b.css"), "/");
        assert_eq!(join("/", "x"), "/x");
        assert_eq!(join("/a", "x"), "/a/x");
    }

    #[test]
    fn mode_strings() {
        assert_eq!(mode_string(0o755), "rwxr-xr-x (755)");
        assert_eq!(mode_string(0o100644), "rw-r--r-- (644)");
    }
}
