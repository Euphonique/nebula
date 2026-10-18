//! End-to-end tests of the FTP and SFTP backends against in-process servers.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Once};
use std::time::Duration;

use russh::server::{Auth, Msg, Server as _, Session};
use russh::{Channel, ChannelId};
use russh_sftp::protocol::{
    Attrs, Data, File, FileAttributes, Handle, Name, OpenFlags, Status, StatusCode, Version,
};

use super::{Backend, ftp::FtpBackend, sftp::SftpBackend};
use crate::settings::{Protocol, Site};

const HOST_KEY: &str = "-----BEGIN OPENSSH PRIVATE KEY-----
b3BlbnNzaC1rZXktdjEAAAAABG5vbmUAAAAEbm9uZQAAAAAAAAABAAAAMwAAAAtzc2gtZW
QyNTUxOQAAACAvaa1qb17znJFDhUvUfld/BIM4YsCbc6h7kqdBHtTU+AAAAIiuhiujroYr
owAAAAtzc2gtZWQyNTUxOQAAACAvaa1qb17znJFDhUvUfld/BIM4YsCbc6h7kqdBHtTU+A
AAAEBUl2n4l7iwgzK2e4KOIjfUniFeA1AnE0hMpOo/7LBjtS9prWpvXvOckUOFS9R+V38E
gzhiwJtzqHuSp0Ee1NT4AAAABHRlc3QB
-----END OPENSSH PRIVATE KEY-----";

fn isolated_config() {
    static INIT: Once = Once::new();
    INIT.call_once(|| {
        let dir = std::env::temp_dir().join(format!("nebula-test-{}", std::process::id()));
        // SAFETY: set once before any test reads it.
        unsafe { std::env::set_var("NEBULA_CONFIG_DIR", dir) };
    });
}

fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

/// Runs the same scenario against any backend.
fn exercise(b: &mut dyn Backend, root: &str) {
    let dir = super::join(root, "site");
    b.mkdir(&dir).unwrap();
    let file = super::join(&dir, "index.html");
    b.write(&file, b"<h1>Hallo \xC3\xBC</h1>\n").unwrap();
    // Overwriting with shorter content must truncate.
    b.write(&file, b"<p>ok</p>").unwrap();
    assert_eq!(b.read(&file).unwrap(), b"<p>ok</p>");

    let entries = b.list(&dir).unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].name, "index.html");
    assert!(!entries[0].is_dir);
    assert_eq!(entries[0].size, 9);
    let root_entries = b.list(root).unwrap();
    assert!(root_entries.iter().any(|e| e.name == "site" && e.is_dir));

    // Conflict detection: stat reflects size changes, missing files are None.
    let stamp = b.stat(&file).unwrap().expect("stat existing file");
    assert_eq!(stamp.size, 9);
    assert!(b.stat(&super::join(&dir, "missing.html")).unwrap().is_none());
    b.write(&file, b"<p>changed!</p>").unwrap();
    assert!(!b.stat(&file).unwrap().unwrap().matches(&stamp));
    b.write(&file, b"<p>ok</p>").unwrap();

    // Recursive upload, copy and download of a folder.
    let local = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(local.path().join("assets/img")).unwrap();
    std::fs::write(local.path().join("assets/a.css"), "a{}").unwrap();
    std::fs::write(local.path().join("assets/img/b.svg"), "<svg/>").unwrap();
    let up = super::join(&dir, "assets");
    assert_eq!(super::upload_recursive(b, &local.path().join("assets"), &up).unwrap(), 2);
    let copy = super::join(&dir, "assets2");
    assert_eq!(super::copy_recursive(b, &up, &copy, true).unwrap(), 2);
    let back = local.path().join("back");
    assert_eq!(super::download_recursive(b, &copy, &back, true).unwrap(), 2);
    assert_eq!(std::fs::read_to_string(back.join("img/b.svg")).unwrap(), "<svg/>");
    super::remove_recursive(b, &up).unwrap();
    super::remove_recursive(b, &copy).unwrap();

    let renamed = super::join(&dir, "start.html");
    b.rename(&file, &renamed).unwrap();
    assert_eq!(b.read(&renamed).unwrap(), b"<p>ok</p>");
    b.ping().unwrap();
    b.remove_file(&renamed).unwrap();
    b.remove_dir(&dir).unwrap();
    assert!(b.list(root).unwrap().iter().all(|e| e.name != "site"));
}

#[test]
fn ftp_roundtrip() {
    isolated_config();
    let home = tempfile::tempdir().unwrap();
    let port = free_port();
    let root = home.path().to_path_buf();
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.spawn(async move {
        let server = libunftp::ServerBuilder::new(Box::new(move || {
            unftp_sbe_fs::Filesystem::new(root.clone()).unwrap()
        }))
        .passive_ports(51000..=51999)
        .build()
        .unwrap();
        server.listen(format!("127.0.0.1:{port}")).await.unwrap();
    });
    std::thread::sleep(Duration::from_millis(300));

    let site = Site {
        protocol: Protocol::Ftp,
        host: "127.0.0.1".into(),
        port,
        user: "test".into(),
        ..Default::default()
    };
    let mut b = FtpBackend::connect(&site, "secret").expect("ftp connect");
    let home_dir = b.home().unwrap();
    exercise(&mut b, &home_dir);
}

// ---- minimal SFTP server backed by a directory ----------------------------------

#[derive(Clone)]
struct SshServer {
    root: PathBuf,
}

impl russh::server::Server for SshServer {
    type Handler = SshSession;
    fn new_client(&mut self, _: Option<std::net::SocketAddr>) -> SshSession {
        SshSession {
            root: self.root.clone(),
            channels: HashMap::new(),
        }
    }
}

struct SshSession {
    root: PathBuf,
    channels: HashMap<ChannelId, Channel<Msg>>,
}

impl russh::server::Handler for SshSession {
    type Error = anyhow::Error;

    async fn auth_password(&mut self, user: &str, password: &str) -> Result<Auth, Self::Error> {
        Ok(if user == "test" && password == "secret" {
            Auth::Accept
        } else {
            Auth::reject()
        })
    }

    async fn channel_open_session(
        &mut self,
        channel: Channel<Msg>,
        reply: russh::server::ChannelOpenHandle,
        _session: &mut Session,
    ) -> Result<(), Self::Error> {
        self.channels.insert(channel.id(), channel);
        reply.accept().await;
        Ok(())
    }

    async fn subsystem_request(
        &mut self,
        id: ChannelId,
        name: &str,
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        let channel = self.channels.remove(&id).unwrap();
        if name == "sftp" {
            session.channel_success(id)?;
            let fs = SftpFs {
                root: self.root.clone(),
                handles: HashMap::new(),
                next: 0,
                modes: HashMap::new(),
            };
            russh_sftp::server::run(channel.into_stream(), fs).await;
        } else {
            session.channel_failure(id)?;
        }
        Ok(())
    }
}

enum OpenHandle {
    File(std::fs::File),
    Dir(Option<Vec<File>>),
}

struct SftpFs {
    root: PathBuf,
    handles: HashMap<String, OpenHandle>,
    next: usize,
    /// Permissions set with chmod (the test runs on Windows file systems).
    modes: HashMap<PathBuf, u32>,
}

fn ok(id: u32) -> Status {
    Status {
        id,
        status_code: StatusCode::Ok,
        error_message: "Ok".into(),
        language_tag: "en-US".into(),
    }
}

fn attrs_of(meta: &std::fs::Metadata) -> FileAttributes {
    let mut a = FileAttributes::empty();
    a.size = Some(meta.len());
    a.permissions = Some(if meta.is_dir() { 0o040755 } else { 0o100644 });
    a
}

impl SftpFs {
    fn attrs(&self, path: &std::path::Path, meta: &std::fs::Metadata) -> FileAttributes {
        let mut a = attrs_of(meta);
        if let Some(m) = self.modes.get(path) {
            a.permissions = Some(a.permissions.unwrap_or(0) & !0o7777 | m);
        }
        a
    }

    fn local(&self, path: &str) -> PathBuf {
        self.root.join(path.trim_start_matches('/'))
    }

    fn handle(&mut self, h: OpenHandle) -> String {
        self.next += 1;
        let key = self.next.to_string();
        self.handles.insert(key.clone(), h);
        key
    }
}

fn io_err(_: std::io::Error) -> StatusCode {
    StatusCode::Failure
}

impl russh_sftp::server::Handler for SftpFs {
    type Error = StatusCode;

    fn unimplemented(&self) -> StatusCode {
        StatusCode::OpUnsupported
    }

    async fn init(&mut self, _v: u32, _e: HashMap<String, String>) -> Result<Version, StatusCode> {
        Ok(Version::new())
    }

    async fn realpath(&mut self, id: u32, path: String) -> Result<Name, StatusCode> {
        let p = if path == "." { "/".to_string() } else { super::normalize(&path) };
        Ok(Name { id, files: vec![File::dummy(p)] })
    }

    async fn open(&mut self, id: u32, filename: String, flags: OpenFlags, _a: FileAttributes) -> Result<Handle, StatusCode> {
        let file = std::fs::OpenOptions::new()
            .read(flags.contains(OpenFlags::READ))
            .write(flags.contains(OpenFlags::WRITE))
            .create(flags.contains(OpenFlags::CREATE))
            .truncate(flags.contains(OpenFlags::TRUNCATE))
            .open(self.local(&filename))
            .map_err(|_| StatusCode::NoSuchFile)?;
        Ok(Handle { id, handle: self.handle(OpenHandle::File(file)) })
    }

    async fn close(&mut self, id: u32, handle: String) -> Result<Status, StatusCode> {
        self.handles.remove(&handle);
        Ok(ok(id))
    }

    async fn read(&mut self, id: u32, handle: String, offset: u64, len: u32) -> Result<Data, StatusCode> {
        use std::io::{Read, Seek, SeekFrom};
        let Some(OpenHandle::File(f)) = self.handles.get_mut(&handle) else {
            return Err(StatusCode::Failure);
        };
        f.seek(SeekFrom::Start(offset)).map_err(io_err)?;
        let mut buf = vec![0; len as usize];
        let n = f.read(&mut buf).map_err(io_err)?;
        if n == 0 {
            return Err(StatusCode::Eof);
        }
        buf.truncate(n);
        Ok(Data { id, data: buf })
    }

    async fn write(&mut self, id: u32, handle: String, offset: u64, data: Vec<u8>) -> Result<Status, StatusCode> {
        use std::io::{Seek, SeekFrom, Write};
        let Some(OpenHandle::File(f)) = self.handles.get_mut(&handle) else {
            return Err(StatusCode::Failure);
        };
        f.seek(SeekFrom::Start(offset)).map_err(io_err)?;
        f.write_all(&data).map_err(io_err)?;
        Ok(ok(id))
    }

    async fn opendir(&mut self, id: u32, path: String) -> Result<Handle, StatusCode> {
        let files = std::fs::read_dir(self.local(&path))
            .map_err(|_| StatusCode::NoSuchFile)?
            .flatten()
            .map(|e| File::new(e.file_name().to_string_lossy(), self.attrs(&e.path(), &e.metadata().unwrap())))
            .collect();
        Ok(Handle { id, handle: self.handle(OpenHandle::Dir(Some(files))) })
    }

    async fn readdir(&mut self, id: u32, handle: String) -> Result<Name, StatusCode> {
        match self.handles.get_mut(&handle) {
            Some(OpenHandle::Dir(files)) => match files.take() {
                Some(files) => Ok(Name { id, files }),
                None => Err(StatusCode::Eof),
            },
            _ => Err(StatusCode::Failure),
        }
    }

    async fn stat(&mut self, id: u32, path: String) -> Result<Attrs, StatusCode> {
        let local = self.local(&path);
        let meta = std::fs::metadata(&local).map_err(|_| StatusCode::NoSuchFile)?;
        Ok(Attrs { id, attrs: self.attrs(&local, &meta) })
    }

    async fn setstat(&mut self, id: u32, path: String, attrs: FileAttributes) -> Result<Status, StatusCode> {
        let local = self.local(&path);
        if !local.exists() {
            return Err(StatusCode::NoSuchFile);
        }
        if let Some(m) = attrs.permissions {
            self.modes.insert(local, m & 0o7777);
        }
        Ok(ok(id))
    }

    async fn lstat(&mut self, id: u32, path: String) -> Result<Attrs, StatusCode> {
        self.stat(id, path).await
    }

    async fn mkdir(&mut self, id: u32, path: String, _a: FileAttributes) -> Result<Status, StatusCode> {
        std::fs::create_dir(self.local(&path)).map_err(io_err)?;
        Ok(ok(id))
    }

    async fn rmdir(&mut self, id: u32, path: String) -> Result<Status, StatusCode> {
        std::fs::remove_dir(self.local(&path)).map_err(io_err)?;
        Ok(ok(id))
    }

    async fn remove(&mut self, id: u32, path: String) -> Result<Status, StatusCode> {
        std::fs::remove_file(self.local(&path)).map_err(io_err)?;
        Ok(ok(id))
    }

    async fn rename(&mut self, id: u32, from: String, to: String) -> Result<Status, StatusCode> {
        std::fs::rename(self.local(&from), self.local(&to)).map_err(io_err)?;
        Ok(ok(id))
    }
}

#[test]
fn sftp_roundtrip_and_host_key_trust() {
    isolated_config();
    let home = tempfile::tempdir().unwrap();
    let port = free_port();
    let server = SshServer { root: home.path().to_path_buf() };
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.spawn(async move {
        let config = russh::server::Config {
            keys: vec![russh::keys::decode_secret_key(HOST_KEY, None).unwrap()],
            auth_rejection_time: Duration::from_millis(10),
            ..Default::default()
        };
        let mut server = server;
        server.run_on_address(Arc::new(config), ("127.0.0.1", port)).await.unwrap();
    });
    std::thread::sleep(Duration::from_millis(300));

    let site = Site {
        protocol: Protocol::Sftp,
        host: "127.0.0.1".into(),
        port,
        user: "test".into(),
        // A nonexistent key file disables probing ~/.ssh keys.
        key_file: Some(PathBuf::from("does-not-exist")),
        ..Default::default()
    };

    // Unknown host: the prompt is shown once and accepted.
    let prompts = Arc::new(AtomicUsize::new(0));
    let p = prompts.clone();
    let accept = move |prompt: super::HostKeyPrompt| {
        p.fetch_add(1, Ordering::SeqCst);
        assert!(!prompt.changed);
        assert!(prompt.fingerprint.starts_with("SHA256:"));
        prompt.answer(true);
    };
    let mut b = SftpBackend::connect(&site, "secret", Box::new(accept)).expect("sftp connect");
    assert_eq!(prompts.load(Ordering::SeqCst), 1);
    let root = b.home().unwrap();
    assert_eq!(root, "/");
    exercise(&mut b, &root);

    // Permissions: set and read back from the listing.
    b.write("/perm.txt", b"x").unwrap();
    b.chmod("/perm.txt", 0o600).unwrap();
    let entry = b.list("/").unwrap().into_iter().find(|e| e.name == "perm.txt").unwrap();
    assert_eq!(entry.mode, Some(0o600));
    drop(b);

    // Trusted now: no prompt on reconnect.
    let reject = |prompt: super::HostKeyPrompt| {
        prompt.answer(false);
        panic!("host key should already be trusted");
    };
    SftpBackend::connect(&site, "secret", Box::new(reject)).expect("reconnect");

    // Wrong password is reported as an error, not a hang.
    let err = SftpBackend::connect(&site, "wrong", Box::new(|p: super::HostKeyPrompt| p.answer(true)))
        .err()
        .expect("must fail");
    assert!(format!("{err:#}").contains("Anmeldung fehlgeschlagen"), "{err:#}");
}

/// Serves `NEBULA_SFTP_ROOT` on 127.0.0.1:2222 (user `test`, password
/// `secret`) for manual testing of the UI: `cargo test serve_sftp -- --ignored`.
#[test]
#[ignore]
fn serve_sftp() {
    let root = PathBuf::from(std::env::var("NEBULA_SFTP_ROOT").expect("NEBULA_SFTP_ROOT"));
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async move {
        let config = russh::server::Config {
            keys: vec![russh::keys::decode_secret_key(HOST_KEY, None).unwrap()],
            ..Default::default()
        };
        let mut server = SshServer { root };
        let serve = server.run_on_address(Arc::new(config), ("127.0.0.1", 2222));
        let _ = tokio::time::timeout(Duration::from_secs(600), serve).await;
    });
}
