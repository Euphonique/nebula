//! Remote file access over FTP, FTPS and SFTP.
//!
//! Each connected site gets its own worker thread that owns the connection and
//! processes requests in order. Results are delivered to the UI through a
//! shared event channel. A dropped connection is re-established transparently
//! and the failed request retried once.

mod ftp;
mod sftp;
pub mod terminal;
pub mod search;
pub mod db;
pub mod transfers;
pub mod journal;
#[cfg(test)]
pub(crate) mod integration_tests;

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use anyhow::{Context, Result};
use crossbeam_channel::{Receiver, Sender};
use eframe::egui;

use crate::settings::{Protocol, Site};

/// Idle time after which a connection is pinged to keep it alive.
const KEEPALIVE: Duration = Duration::from_secs(45);

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct RemoteEntry {
    pub name: String,
    pub path: String,
    pub is_dir: bool,
    pub size: u64,
    /// Unix permission bits (`0o755`), if the server reports them.
    pub mode: Option<u32>,
    /// Modification time (seconds since the Unix epoch), if reported.
    pub mtime: Option<i64>,
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
    /// The end of a file from byte `offset` on (for logs).
    fn read_from(&mut self, path: &str, offset: u64) -> Result<Vec<u8>> {
        let all = self.read(path)?;
        Ok(all.get(offset as usize..).unwrap_or_default().to_vec())
    }
    /// Moves an uploaded temporary file onto `path`, replacing it.
    fn swap_in(&mut self, tmp: &str, path: &str) -> Result<()> {
        replace_with(self, tmp, path)
    }
}

/// Puts `tmp` in place of `path`. Servers that refuse to rename onto an
/// existing file (SFTP) get the old file moved aside first; it is put back
/// if the second step fails.
fn replace_with<B: Backend + ?Sized>(b: &mut B, tmp: &str, path: &str) -> Result<()> {
    let Err(e) = b.rename(tmp, path) else { return Ok(()) };
    if b.stat(path)?.is_none() {
        return Err(e);
    }
    let old = temp_name(path, OLD_SUFFIX);
    let _ = b.remove_file(&old);
    b.rename(path, &old)?;
    if let Err(e) = b.rename(tmp, path) {
        let _ = b.rename(&old, path);
        return Err(e);
    }
    // A leftover would be harmless (hidden and replaced next time).
    let _ = b.remove_file(&old);
    Ok(())
}

const TEMP_SUFFIX: &str = ".nebula-upload";
const OLD_SUFFIX: &str = ".nebula-old";

/// `/www/.index.php.nebula-upload` for `/www/index.php`: hidden, same folder.
fn temp_name(path: &str, suffix: &str) -> String {
    let name = path.rsplit('/').next().unwrap_or(path);
    join(&parent(path), &format!(".{name}{suffix}"))
}

/// Temporary files of a publish (not worth keeping in the upload history).
pub fn is_temp(path: &str) -> bool {
    path.ends_with(TEMP_SUFFIX) || path.ends_with(OLD_SUFFIX)
}

/// Identifies a version of a remote file for conflict detection.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
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
    /// All files below `root` (recursively), skipping names in `exclude`.
    Scan { root: String, exclude: Vec<String> },
    /// Synchronizing: uploads, downloads and deletions one after another.
    Sync(Vec<SyncOp>),
    /// Compares server files with local ones byte by byte: (name, remote, local).
    Compare(Vec<(String, String, std::path::PathBuf)>),
    /// Restores the state before an operation of the upload history. Without
    /// `force`, files changed since then are reported first ([`Reply::UndoConflict`]).
    Undo { entry: u64, force: bool },
    /// Uploads files of a project (local, remote). `staged`: everything goes up
    /// under temporary names first and replaces the files only at the end, so
    /// the site is never half updated.
    Publish { files: Vec<(std::path::PathBuf, String)>, staged: bool },
    /// What was appended to a log since `offset`; without offset only the
    /// current size (watching starts at the end).
    TailLog { path: String, offset: Option<u64> },
}

/// One step of a synchronization.
#[derive(Clone, Debug)]
pub enum SyncOp {
    Upload { local: std::path::PathBuf, remote: String },
    Download { remote: String, local: std::path::PathBuf },
    Delete(String),
}

/// A file found by [`Request::Scan`], relative to the scanned folder.
#[derive(Clone, Debug)]
pub struct ScannedFile {
    pub rel: String,
    pub size: u64,
    pub mtime: Option<i64>,
}

/// Most files a scan collects (protects against scanning a whole server).
const SCAN_LIMIT: usize = 50_000;

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
            Request::List(p) => t!("Auflisten von {p}", p = p),
            Request::Read(p) => t!("Laden von {p}", p = p),
            Request::Write { path, .. } => t!("Hochladen von {path}", path = path),
            Request::CreateFile(p) => t!("Anlegen von {p}", p = p),
            Request::Mkdir(p) => t!("Anlegen von {p}", p = p),
            Request::Delete { path, .. } => t!("Löschen von {path}", path = path),
            Request::Rename { from, .. } => t!("Umbenennen von {from}", from = from),
            Request::Upload { local, .. } => t!("Hochladen von {local}", local = local.display()),
            Request::Download { remote, .. } => t!("Herunterladen von {remote}", remote = remote),
            Request::Copy { from, .. } => t!("Kopieren von {from}", from = from),
            Request::Chmod { path, .. } => t!("Ändern der Berechtigungen von {path}", path = path),
            Request::Scan { root, .. } => t!("Einlesen von {root}", root = root),
            Request::Sync(ops) => t!("Synchronisieren ({ops_count} Schritte)", ops_count = ops.len()),
            Request::Compare(files) => t!("Vergleichen von {files_count} Dateien", files_count = files.len()),
            Request::Undo { .. } => t!("Rückgängigmachen").into(),
            Request::Publish { files, .. } => t!("Veröffentlichen ({n} Dateien)", n = files.len()),
            Request::TailLog { path, .. } => t!("Lesen des Fehlerprotokolls {path}", path = path),
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
    /// Result of [`Request::Scan`]; `truncated` when the limit was reached.
    Scanned { files: Vec<ScannedFile>, truncated: bool },
    /// Result of [`Request::Compare`]: names whose contents differ.
    Compared { differing: Vec<String> },
    /// A [`Request::Sync`] finished; `failed` lists steps that went wrong.
    Synced { done: usize, failed: Vec<String> },
    /// A [`Request::Publish`] finished: server paths put in place, and failures.
    Published { done: Vec<String>, failed: Vec<String> },
    /// New lines of a log (whole lines only) and where to go on next time.
    LogTail { offset: u64, text: String },
    /// Files of an operation changed on the server since; nothing was undone.
    UndoConflict { entry: u64, paths: Vec<String> },
    /// An operation was undone (as far as possible).
    Undone { restored: usize, skipped: Vec<String>, failed: Vec<String>, paths: Vec<String> },
    Failed { request: Request, message: String },
    ConnectFailed { message: String },
    Disconnected,
    /// SFTP host key not yet trusted; the UI must answer through `answer`.
    HostKeyUnknown(HostKeyPrompt),
    /// A transfer started or finished another file (handled by the manager).
    Progress { done: usize },
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

/// Where the changes of a request go in the upload history.
#[derive(Clone, Copy, Debug)]
pub struct Batch {
    /// Entry id; requests of one batch become one entry.
    pub id: u64,
    /// False when the upload history is switched off.
    pub record: bool,
}

enum Command {
    /// `quiet`: not counted as pending (background polling).
    Run { tag: u64, request: Request, cancel: Arc<std::sync::atomic::AtomicBool>, batch: Batch, quiet: bool },
    Disconnect,
}

/// Reports per-file progress of a transfer and stops it when cancelled.
struct Progress<'a> {
    events: &'a Sender<RemoteEvent>,
    ctx: &'a egui::Context,
    site_id: &'a str,
    tag: u64,
    cancel: &'a std::sync::atomic::AtomicBool,
    done: usize,
    /// How files are optimized on their way up (from the site).
    optimize: crate::optimize::Optimize,
}

impl Progress<'_> {
    /// Called before each file; errors out if the transfer was cancelled.
    fn file(&mut self) -> Result<()> {
        if self.cancel.load(std::sync::atomic::Ordering::Relaxed) {
            anyhow::bail!(t!("Abgebrochen"));
        }
        Ok(())
    }

    fn cancelled(&self) -> bool {
        self.cancel.load(std::sync::atomic::Ordering::Relaxed)
    }

    fn finished_file(&mut self) {
        self.done += 1;
        let _ = self.events.send(RemoteEvent { site_id: self.site_id.to_string(), tag: self.tag, reply: Reply::Progress { done: self.done } });
        self.ctx.request_repaint();
    }
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
    /// Uploads, downloads, copies and saves for the transfer list.
    pub transfers: transfers::Transfers,
    /// Cancel flags of long requests outside the transfer list (comparisons).
    cancels: HashMap<u64, Arc<std::sync::atomic::AtomicBool>>,
    /// Keep the upload history (from the settings).
    pub record: bool,
    last_batch_id: u64,
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
            transfers: Default::default(),
            cancels: HashMap::new(),
            record: true,
            last_batch_id: 0,
        }
    }

    /// Ids are times in milliseconds, so they stay unique across sessions.
    fn new_batch_id(&mut self) -> u64 {
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0);
        self.last_batch_id = now.max(self.last_batch_id + 1);
        self.last_batch_id
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
        let cancel = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let batch = Batch { id: self.new_batch_id(), record: self.record };
        if transfers::is_transfer(&request) {
            self.transfers.add(transfers::Transfer {
                tag,
                site_id: site.id.clone(),
                site_name: site.display_name(),
                request: request.clone(),
                state: transfers::TransferState::Queued,
                started: std::time::Instant::now(),
                finished: None,
                cancel: cancel.clone(),
                batch: batch.id,
            });
        } else if matches!(request, Request::Compare(_)) {
            self.cancels.insert(tag, cancel.clone());
        }
        let conn = &self.connections[&site.id];
        let quiet = matches!(request, Request::TailLog { .. });
        if !quiet {
            conn.busy.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }
        if conn.tx.send(Command::Run { tag, request, cancel, batch, quiet }).is_err() && !quiet {
            conn.busy.fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
        }
        tag
    }

    /// Stops a running or queued comparison.
    pub fn cancel(&mut self, tag: u64) {
        if let Some(c) = self.cancels.get(&tag) {
            c.store(true, std::sync::atomic::Ordering::Relaxed);
        }
    }

    /// Sends a failed transfer again; the old entry is replaced.
    pub fn retry(&mut self, site: &Site, tag: u64) -> Option<u64> {
        let request = self.transfers.items.iter().find(|t| t.tag == tag && t.can_retry())?.request.clone();
        self.transfers.remove(tag);
        Some(self.send(site, request))
    }

    /// Events from the connections; progress is applied to the transfer list here.
    pub fn poll(&mut self) -> Vec<RemoteEvent> {
        use transfers::TransferState as S;
        let mut out = Vec::new();
        for ev in self.events_rx.try_iter() {
            if let Some(t) = self.transfers.get_mut(ev.tag) {
                match &ev.reply {
                    Reply::Progress { done } => {
                        // Counted once, when the transfer starts.
                        let total = match t.state {
                            S::Running { total, .. } => total,
                            _ => match &t.request {
                                Request::Upload { local, .. } => Some(transfers::count_local(local)),
                                Request::Publish { files, .. } => Some(files.len()),
                                Request::Download { is_dir: true, .. } | Request::Copy { is_dir: true, .. } => None,
                                _ => Some(1),
                            },
                        };
                        t.state = S::Running { done: *done, total };
                    }
                    Reply::Transferred { message, .. } => t.state = S::Done(message.clone()),
                    Reply::Written { bytes, .. } => t.state = S::Done(t!("{bytes} Bytes gespeichert", bytes = bytes)),
                    Reply::Published { done, failed } if failed.is_empty() => t.state = S::Done(t!("{n} Datei(en) hochgeladen", n = done.len())),
                    Reply::Published { done, failed } => t.state = S::Failed(t!("{n} Datei(en) hochgeladen, {f} fehlgeschlagen", n = done.len(), f = failed.len())),
                    Reply::Conflict { .. } => t.state = S::Failed(t!("Auf dem Server geändert – nicht überschrieben").into()),
                    Reply::Failed { message, .. } => t.state = S::Failed(message.clone()),
                    _ => {}
                }
                if t.state.finished() && t.finished.is_none() {
                    t.finished = Some(std::time::Instant::now());
                }
            }
            if !matches!(ev.reply, Reply::Progress { .. }) {
                self.cancels.remove(&ev.tag);
                out.push(ev);
            }
        }
        out
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
            Protocol::Ftp | Protocol::Ftps | Protocol::FtpsImplicit => Box::new(ftp::FtpBackend::connect(&self.site, &password)?),
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
                if let Command::Run { tag, request, quiet, .. } = cmd {
                    if !quiet {
                        self.busy.fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
                    }
                    self.emit(tag, Reply::Failed { request, message: t!("Keine Verbindung").into() });
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
            let (tag, request, cancel, batch, quiet) = match cmd {
                Command::Disconnect => break,
                Command::Run { tag, request, cancel, batch, quiet } => (tag, request, cancel, batch, quiet),
            };
            let done = |busy: &std::sync::atomic::AtomicUsize| {
                if !quiet {
                    busy.fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
                }
            };
            // Cancelled while waiting in the queue.
            if cancel.load(std::sync::atomic::Ordering::Relaxed) {
                done(&self.busy);
                self.emit(tag, Reply::Failed { request, message: t!("Abgebrochen").into() });
                continue;
            }
            let (events, ctx, site_id) = (self.events.clone(), self.ctx.clone(), self.site.id.clone());
            let mut progress = Progress { events: &events, ctx: &ctx, site_id: &site_id, tag, cancel: &cancel, done: 0, optimize: self.site.optimize };
            if transfers::is_transfer(&request) {
                // Marks the transfer as running in the list.
                let _ = events.send(RemoteEvent { site_id: site_id.clone(), tag, reply: Reply::Progress { done: 0 } });
                ctx.request_repaint();
            }
            let mut result = self.execute(&request, &mut progress, batch);
            if let Err(e) = &result
                && is_connection_error(e)
            {
                // Reconnect once and retry.
                self.backend = None;
                progress.done = 0;
                result = match self.open() {
                    Ok(()) => self.execute(&request, &mut progress, batch),
                    Err(e) => Err(e),
                };
            }
            done(&self.busy);
            match result {
                Ok(reply) => self.emit(tag, reply),
                Err(_) if cancel.load(std::sync::atomic::Ordering::Relaxed) => {
                    let message = t!("{describe} abgebrochen ({done} Datei(en) fertig)", describe = request.describe(), done = progress.done);
                    self.emit(tag, Reply::Failed { request, message });
                }
                Err(e) => {
                    let message = t!("{describe} fehlgeschlagen: {e}", describe = request.describe(), e = format!("{:#}", e));
                    self.emit(tag, Reply::Failed { request, message });
                }
            }
        }
        self.emit(0, Reply::Disconnected);
    }

    fn execute(&mut self, request: &Request, progress: &mut Progress, batch: Batch) -> Result<Reply> {
        if self.backend.is_none() {
            self.open()?;
        }
        let backup = self.site.backup_on_save;
        let site_id = self.site.id.clone();
        let b = self.backend.as_mut().unwrap().as_mut();
        let Some(kind) = journal::kind_of(request) else {
            return run_request(b, backup, &site_id, request, progress);
        };
        // Undoing is always recorded, so it can be undone again. Without the
        // upload history the changes are still noted for the change watcher.
        let keep = batch.record || kind == journal::Kind::Undo;
        let mut rec = if keep { journal::Recorder::new(b, &site_id) } else { journal::Recorder::changes_only(b, &site_id) };
        let result = run_request(&mut rec, backup, &site_id, request, progress);
        let actions = std::mem::take(&mut rec.actions);
        crate::server_watch::note_own(&site_id, &actions);
        if !keep {
            return result;
        }
        let (reverts, name) = match request {
            Request::Undo { entry, .. } => match journal::get(&site_id, *entry) {
                Some(e) => (Some((e.id, e.kind)), e.name),
                None => (None, String::new()),
            },
            _ => (None, journal::name_of(request)),
        };
        // Also after a failure: what was changed until then can be undone.
        journal::commit(&site_id, batch.id, kind, name, actions, reverts);
        result
    }
}

fn run_request(b: &mut dyn Backend, backup: bool, site_id: &str, request: &Request, progress: &mut Progress) -> Result<Reply> {
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
            if backup {
                backup_file(b, path)?;
            }
            b.write(path, data)?;
            let stamp = b.stat(path).ok().flatten();
            Reply::Written { path: path.clone(), bytes: data.len(), stamp }
        }
        Request::Upload { local, remote } => {
            if backup && local.is_file() {
                backup_file(b, remote)?;
            }
            let n = upload_recursive(b, local, remote, progress)?;
            Reply::Transferred {
                message: t!("{n} Datei(en) hochgeladen", n = n),
                remote_dir: Some(parent(remote)),
                local_dir: None,
            }
        }
        Request::Download { remote, local, is_dir } => {
            let n = download_recursive(b, remote, local, *is_dir, progress)?;
            Reply::Transferred {
                message: t!("{n} Datei(en) heruntergeladen", n = n),
                remote_dir: None,
                local_dir: local.parent().map(|p| p.to_path_buf()),
            }
        }
        Request::Copy { from, to, is_dir } => {
            let n = copy_recursive(b, from, to, *is_dir, progress)?;
            Reply::Transferred {
                message: t!("{n} Datei(en) kopiert", n = n),
                remote_dir: Some(parent(to)),
                local_dir: None,
            }
        }
        Request::CreateFile(path) => {
            // Writing would empty an existing file.
            if name_taken(b, path)? {
                anyhow::bail!(t!("{path} existiert bereits.", path = path));
            }
            b.write(path, &[])?;
            Reply::Changed { dir: parent(path), message: t!("{path} angelegt", path = path) }
        }
        Request::Mkdir(path) => {
            b.mkdir(path)?;
            Reply::Changed { dir: parent(path), message: t!("{path} angelegt", path = path) }
        }
        Request::Delete { path, is_dir } => {
            if *is_dir {
                remove_recursive(b, path)?;
            } else {
                b.remove_file(path)?;
            }
            Reply::Changed { dir: parent(path), message: t!("{path} gelöscht", path = path) }
        }
        Request::Rename { from, to } => {
            // FTP servers usually replace an existing target silently.
            if from != to && name_taken(b, to)? {
                anyhow::bail!(t!("{path} existiert bereits.", path = to));
            }
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
                n += chmod_recursive(b, path, *mode, *scope)?;
            }
            let what = if n == 1 { path.clone() } else { t!("{n} Objekte", n = n) };
            Reply::Changed { dir: parent(path), message: t!("Berechtigungen von {what} auf {mode_string} gesetzt", what = what, mode_string = mode_string(*mode)) }
        }
        Request::Scan { root, exclude } => {
            let mut files = Vec::new();
            let truncated = scan_recursive(b, root, "", exclude, &mut files)?;
            Reply::Scanned { files, truncated }
        }
        Request::Sync(ops) => {
            // Every step on its own: one failure doesn't stop the others.
            let (mut done, mut failed) = (0, Vec::new());
            for op in ops {
                let result = match op {
                    SyncOp::Upload { local, remote } => upload_recursive(b, local, remote, progress).map(|_| ()),
                    SyncOp::Download { remote, local } => download_recursive(b, remote, local, false, progress).map(|_| ()),
                    SyncOp::Delete(path) => b.remove_file(path),
                };
                match result {
                    Ok(()) => done += 1,
                    // A cancelled transfer stops everything.
                    Err(e) if progress.cancelled() => return Err(e),
                    Err(e) => {
                        let what = match op {
                            SyncOp::Upload { remote, .. } | SyncOp::Download { remote, .. } | SyncOp::Delete(remote) => remote,
                        };
                        failed.push(format!("{what}: {e:#}"));
                    }
                }
            }
            Reply::Synced { done, failed }
        }
        Request::Compare(files) => {
            let mut differing = Vec::new();
            for (name, remote, local) in files {
                progress.file()?;
                // Server errors stop the comparison (a dropped connection
                // is retried); an unreadable local file counts as different.
                let r = b.read(remote)?;
                // Compared as it would be uploaded (optimized).
                let same = std::fs::read(local).is_ok_and(|l| crate::optimize::apply(remote, l, &progress.optimize) == r);
                if !same {
                    differing.push(name.clone());
                }
            }
            Reply::Compared { differing }
        }
        Request::Publish { files, staged } => {
            let (done, failed) = if *staged { publish_staged(b, files, backup, progress)? } else { publish_direct(b, files, backup, progress)? };
            Reply::Published { done, failed }
        }
        Request::TailLog { path, offset } => {
            let Some(stamp) = b.stat(path)? else {
                anyhow::bail!(t!("{path} gibt es nicht", path = path));
            };
            let (offset, text) = match *offset {
                None => (stamp.size, String::new()),
                Some(o) if o == stamp.size => (o, String::new()),
                // Shorter than before: the log was rotated or emptied.
                Some(o) => {
                    let start = if o > stamp.size { 0 } else { o };
                    // Never more than the last 256 KB at once.
                    let start = start.max(stamp.size.saturating_sub(256 * 1024));
                    let data = b.read_from(path, start)?;
                    // Only whole lines; the rest comes next time.
                    let end = data.iter().rposition(|&c| c == b'\n').map_or(0, |i| i + 1);
                    (start + end as u64, String::from_utf8_lossy(&data[..end]).into_owned())
                }
            };
            Reply::LogTail { offset, text }
        }
        Request::Undo { entry, force } => {
            let Some(e) = journal::get(site_id, *entry) else {
                anyhow::bail!(t!("Dieser Vorgang ist nicht mehr im Upload-Verlauf."));
            };
            if !*force {
                let paths = journal::changed_since(b, &e)?;
                if !paths.is_empty() {
                    return Ok(Reply::UndoConflict { entry: *entry, paths });
                }
            }
            let u = journal::undo(b, site_id, &e);
            Reply::Undone { restored: u.restored, skipped: u.skipped, failed: u.failed, paths: u.paths }
        }
    })
}

/// Collects files below `dir`; returns true when [`SCAN_LIMIT`] was reached.
fn scan_recursive(b: &mut dyn Backend, dir: &str, rel: &str, exclude: &[String], out: &mut Vec<ScannedFile>) -> Result<bool> {
    for e in b.list(dir)? {
        if exclude.iter().any(|x| x == &e.name) {
            continue;
        }
        let rel = if rel.is_empty() { e.name.clone() } else { format!("{rel}/{}", e.name) };
        if e.is_dir {
            if scan_recursive(b, &e.path, &rel, exclude, out)? {
                return Ok(true);
            }
        } else {
            out.push(ScannedFile { rel, size: e.size, mtime: e.mtime });
            if out.len() >= SCAN_LIMIT {
                return Ok(true);
            }
        }
    }
    Ok(false)
}

/// Copies an existing server file to `<name>.bak` before it is overwritten
/// (replacing an older copy). Nothing happens if the file does not exist yet.
fn backup_file(b: &mut dyn Backend, path: &str) -> Result<()> {
    // A failing stat (e.g. the folder does not exist yet) means: nothing to keep.
    if path.ends_with(".bak") || !matches!(b.stat(path), Ok(Some(_))) {
        return Ok(());
    }
    let old = b.read(path)?;
    b.write(&format!("{path}.bak"), &old)
        .with_context(|| t!("Sicherungskopie {path}.bak konnte nicht angelegt werden", path = path))
}

fn upload_recursive(b: &mut dyn Backend, local: &std::path::Path, remote: &str, p: &mut Progress) -> Result<usize> {
    if local.is_dir() {
        // The folder may already exist; a real failure shows up on the files.
        let _ = b.mkdir(remote);
        let mut n = 0;
        for entry in std::fs::read_dir(local)? {
            let entry = entry?;
            let name = entry.file_name().to_string_lossy().into_owned();
            n += upload_recursive(b, &entry.path(), &join(remote, &name), p)?;
        }
        Ok(n)
    } else {
        p.file()?;
        let data = crate::optimize::apply(remote, std::fs::read(local)?, &p.optimize);
        write_creating(b, remote, &data)?;
        p.finished_file();
        Ok(1)
    }
}

/// Publishing file by file (the site shows the new files one after another).
fn publish_direct(b: &mut dyn Backend, files: &[(std::path::PathBuf, String)], backup: bool, p: &mut Progress) -> Result<(Vec<String>, Vec<String>)> {
    let (mut done, mut failed) = (Vec::new(), Vec::new());
    for (local, remote) in files {
        let result = if backup { backup_file(b, remote) } else { Ok(()) }.and_then(|()| upload_recursive(b, local, remote, p).map(|_| ()));
        match result {
            Ok(()) => done.push(remote.clone()),
            Err(e) if p.cancelled() || is_connection_error(&e) => return Err(e),
            Err(e) => failed.push(format!("{remote}: {e:#}")),
        }
    }
    Ok((done, failed))
}

/// Publishing in two steps: all files go up under temporary names (the site
/// does not change yet), then each replaces its file. A failed or cancelled
/// upload removes the temporary files again and changes nothing.
fn publish_staged(b: &mut dyn Backend, files: &[(std::path::PathBuf, String)], backup: bool, p: &mut Progress) -> Result<(Vec<String>, Vec<String>)> {
    let mut staged: Vec<(String, &String)> = Vec::new();
    let opt = p.optimize;
    for (local, remote) in files {
        let tmp = temp_name(remote, TEMP_SUFFIX);
        let result = p
            .file()
            .and_then(|()| std::fs::read(local).with_context(|| local.display().to_string()))
            .map(|data| crate::optimize::apply(remote, data, &opt))
            .and_then(|data| write_creating(b, &tmp, &data).with_context(|| remote.clone()));
        if let Err(e) = result {
            for (tmp, _) in &staged {
                let _ = b.remove_file(tmp);
            }
            let _ = b.remove_file(&tmp);
            return Err(e.context(t!("Nichts veröffentlicht – die Dateien auf dem Server sind unverändert")));
        }
        p.finished_file();
        staged.push((tmp, remote));
    }
    // Now quickly, one after another; listings tell the permissions to keep.
    let mut listings: HashMap<String, Vec<RemoteEntry>> = HashMap::new();
    let (mut done, mut failed) = (Vec::new(), Vec::new());
    for (tmp, remote) in staged {
        match put_in_place(b, &tmp, remote, backup, &mut listings) {
            Ok(()) => done.push(remote.clone()),
            Err(e) => {
                let _ = b.remove_file(&tmp);
                failed.push(format!("{remote}: {e:#}"));
            }
        }
    }
    Ok((done, failed))
}

fn put_in_place(b: &mut dyn Backend, tmp: &str, path: &str, backup: bool, listings: &mut HashMap<String, Vec<RemoteEntry>>) -> Result<()> {
    let dir = parent(path);
    if !listings.contains_key(&dir) {
        let entries = b.list(&dir).unwrap_or_default();
        listings.insert(dir.clone(), entries);
    }
    let name = path.rsplit('/').next().unwrap_or(path);
    if let Some(old) = listings[&dir].iter().find(|e| e.name == name && !e.is_dir) {
        if backup {
            backup_file(b, path)?;
        }
        // A replaced file keeps its permissions (e.g. 600 for a config).
        if let Some(mode) = old.mode {
            let _ = b.chmod(tmp, mode & 0o7777);
        }
    }
    b.swap_in(tmp, path)
}

/// Writes a file; if that fails, creates its parent folders and tries again
/// (the target folder may not exist yet).
fn write_creating(b: &mut dyn Backend, path: &str, data: &[u8]) -> Result<()> {
    if b.write(path, data).is_err() {
        let mut dir = String::new();
        for part in parent(path).split('/').filter(|p| !p.is_empty()) {
            dir = format!("{dir}/{part}");
            // Existing folders fail here, so the upload history only notes new ones.
            let _ = b.mkdir(&dir);
        }
        b.write(path, data)?;
    }
    Ok(())
}

fn download_recursive(b: &mut dyn Backend, remote: &str, local: &std::path::Path, is_dir: bool, p: &mut Progress) -> Result<usize> {
    if is_dir {
        std::fs::create_dir_all(local)?;
        let mut n = 0;
        for e in b.list(remote)? {
            n += download_recursive(b, &e.path, &local.join(&e.name), e.is_dir, p)?;
        }
        Ok(n)
    } else {
        p.file()?;
        let data = b.read(remote)?;
        if let Some(dir) = local.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(local, data)?;
        p.finished_file();
        Ok(1)
    }
}

fn copy_recursive(b: &mut dyn Backend, from: &str, to: &str, is_dir: bool, p: &mut Progress) -> Result<usize> {
    if is_dir {
        let _ = b.mkdir(to);
        let mut n = 0;
        for e in b.list(from)? {
            n += copy_recursive(b, &e.path, &join(to, &e.name), e.is_dir, p)?;
        }
        Ok(n)
    } else {
        p.file()?;
        let data = b.read(from)?;
        b.write(to, &data)?;
        p.finished_file();
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

/// Whether the folder of `path` already has an entry with its name
/// (exact spelling, so changing only the case stays possible).
fn name_taken(b: &mut dyn Backend, path: &str) -> Result<bool> {
    let name = path.rsplit('/').next().unwrap_or(path);
    Ok(b.list(&parent(path))?.iter().any(|e| e.name == name))
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
            // russh-sftp after the server dropped the connection.
            || text.contains("session closed")
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
