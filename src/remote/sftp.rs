//! SFTP over SSH (pure Rust via russh).

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Arc, LazyLock, Mutex};
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use russh::client::{self, AuthResult, KeyboardInteractiveAuthResponse};
use russh::keys::{HashAlg, PrivateKeyWithHashAlg, PublicKey, PublicKeyOrCertificate, load_secret_key};
use russh::ChannelId;
use russh_sftp::client::SftpSession;
use russh_sftp::protocol::OpenFlags;
use tokio::io::AsyncWriteExt;

use super::{Backend, HostKeyPrompt, RemoteEntry, Stamp, join};
use crate::settings::{Site, config_dir};

const TIMEOUT: Duration = Duration::from_secs(20);

static RUNTIME: LazyLock<tokio::runtime::Runtime> = LazyLock::new(|| {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .thread_name("ssh")
        .enable_all()
        .build()
        .expect("tokio runtime")
});

pub type PromptFn = Box<dyn Fn(HostKeyPrompt) + Send + Sync>;

/// Trusted host key fingerprints, stored as JSON next to the settings.
struct KnownHosts;

impl KnownHosts {
    fn path() -> PathBuf {
        config_dir().join("known_hosts.json")
    }

    fn load() -> BTreeMap<String, String> {
        std::fs::read(Self::path())
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default()
    }

    fn store(key: &str, fingerprint: &str) {
        let mut map = Self::load();
        map.insert(key.to_string(), fingerprint.to_string());
        let _ = std::fs::create_dir_all(config_dir());
        if let Ok(json) = serde_json::to_vec_pretty(&map) {
            let _ = std::fs::write(Self::path(), json);
        }
    }
}

struct Handler {
    host: String,
    port: u16,
    prompt: Arc<PromptFn>,
}

impl client::Handler for Handler {
    type Error = anyhow::Error;

    async fn check_server_key(&mut self, key: &PublicKeyOrCertificate) -> Result<bool, Self::Error> {
        let public: PublicKey = match key {
            PublicKeyOrCertificate::PublicKey { key, .. } => key.clone(),
            PublicKeyOrCertificate::Certificate(cert) => PublicKey::from(cert.public_key().clone()),
        };
        let fingerprint = public.fingerprint(HashAlg::Sha256).to_string();
        let id = format!("{}:{}", self.host, self.port);
        let known = KnownHosts::load();
        let changed = match known.get(&id) {
            Some(f) if *f == fingerprint => return Ok(true),
            Some(_) => true,
            None => false,
        };

        let (tx, rx) = tokio::sync::oneshot::channel();
        (self.prompt)(HostKeyPrompt {
            host: self.host.clone(),
            port: self.port,
            algorithm: public.algorithm().to_string(),
            fingerprint: fingerprint.clone(),
            changed,
            answer: Arc::new(Mutex::new(Some(tx))),
        });
        if rx.await.unwrap_or(false) {
            KnownHosts::store(&id, &fingerprint);
            Ok(true)
        } else {
            Ok(false)
        }
    }

    async fn data(
        &mut self,
        _channel: ChannelId,
        _data: &[u8],
        _session: &mut client::Session,
    ) -> Result<(), Self::Error> {
        Ok(())
    }
}

pub struct SftpBackend {
    // Dropping the handle closes the SSH connection, so it must stay alive.
    _session: client::Handle<Handler>,
    sftp: SftpSession,
}

impl SftpBackend {
    pub fn connect(site: &Site, password: &str, prompt: PromptFn) -> Result<Self> {
        RUNTIME.block_on(Self::connect_async(site, password, Arc::new(prompt)))
    }

    async fn connect_async(site: &Site, password: &str, prompt: Arc<PromptFn>) -> Result<Self> {
        let config = Arc::new(client::Config {
            keepalive_interval: Some(Duration::from_secs(30)),
            keepalive_max: 3,
            nodelay: true,
            ..Default::default()
        });
        let socket = tokio::time::timeout(
            TIMEOUT,
            tokio::net::TcpStream::connect((site.host.as_str(), site.port)),
        )
        .await
        .map_err(|_| anyhow!("Zeitüberschreitung beim Verbinden mit {}:{}", site.host, site.port))?
        .with_context(|| format!("Verbindung zu {}:{} fehlgeschlagen", site.host, site.port))?;
        socket.set_nodelay(true).ok();

        let handler = Handler {
            host: site.host.clone(),
            port: site.port,
            prompt,
        };
        let mut session = client::connect_stream(config, socket, handler)
            .await
            .map_err(|e| anyhow!("SSH-Verbindung fehlgeschlagen: {e}"))?;

        authenticate(&mut session, site, password).await?;

        let channel = session.channel_open_session().await?;
        channel.request_subsystem(true, "sftp").await?;
        let sftp = SftpSession::new(channel.into_stream())
            .await
            .context("SFTP-Subsystem nicht verfügbar")?;
        sftp.set_timeout(30);
        Ok(Self {
            _session: session,
            sftp,
        })
    }
}

async fn authenticate(session: &mut client::Handle<Handler>, site: &Site, password: &str) -> Result<()> {
    let user = site.user.as_str();
    let rsa_hash = session.best_supported_rsa_hash().await.ok().flatten().flatten();

    // 1. Public key: the configured file, or the default keys in ~/.ssh.
    let mut key_files: Vec<PathBuf> = Vec::new();
    if let Some(k) = &site.key_file {
        key_files.push(k.clone());
    } else if let Some(home) = directories::UserDirs::new().map(|d| d.home_dir().join(".ssh")) {
        for name in ["id_ed25519", "id_ecdsa", "id_rsa"] {
            let p = home.join(name);
            if p.exists() {
                key_files.push(p);
            }
        }
    }
    let mut key_error = None;
    for path in &key_files {
        let passphrase = (!password.is_empty()).then_some(password);
        let key = match load_secret_key(path, passphrase).or_else(|_| load_secret_key(path, None)) {
            Ok(k) => k,
            Err(e) => {
                if site.key_file.is_some() {
                    key_error = Some(format!("Schlüssel {} nicht lesbar: {e}", path.display()));
                }
                continue;
            }
        };
        let key = PrivateKeyWithHashAlg::new(Arc::new(key), rsa_hash);
        if session.authenticate_publickey(user, key).await?.success() {
            return Ok(());
        }
    }

    // 2. Password.
    if !password.is_empty() {
        if let AuthResult::Success = session.authenticate_password(user, password).await? {
            return Ok(());
        }
        // 3. Keyboard-interactive, answering every prompt with the password.
        let mut response = session
            .authenticate_keyboard_interactive_start(user, None::<String>)
            .await?;
        for _ in 0..4 {
            match response {
                KeyboardInteractiveAuthResponse::Success => return Ok(()),
                KeyboardInteractiveAuthResponse::Failure { .. } => break,
                KeyboardInteractiveAuthResponse::InfoRequest { prompts, .. } => {
                    let answers = prompts.iter().map(|_| password.to_string()).collect();
                    response = session
                        .authenticate_keyboard_interactive_respond(answers)
                        .await?;
                }
            }
        }
    }

    match key_error {
        Some(e) => bail!("Anmeldung fehlgeschlagen. {e}"),
        None => bail!("Anmeldung fehlgeschlagen (Benutzername, Passwort oder Schlüssel falsch?)"),
    }
}

impl Backend for SftpBackend {
    fn home(&mut self) -> Result<String> {
        let home = RUNTIME.block_on(self.sftp.canonicalize("."))?;
        Ok(super::normalize(&home))
    }

    fn list(&mut self, path: &str) -> Result<Vec<RemoteEntry>> {
        RUNTIME.block_on(async {
            let mut out = Vec::new();
            for entry in self.sftp.read_dir(path).await? {
                let name = entry.file_name();
                if name == "." || name == ".." {
                    continue;
                }
                let full = join(path, &name);
                let meta = entry.metadata();
                let mut is_dir = entry.file_type().is_dir();
                if entry.file_type().is_symlink()
                    && let Ok(target) = self.sftp.metadata(full.clone()).await
                {
                    is_dir = target.file_type().is_dir();
                }
                out.push(RemoteEntry {
                    name,
                    path: full,
                    is_dir,
                    size: meta.len(),
                    mode: meta.permissions.map(|p| p & 0o7777),
                });
            }
            Ok(out)
        })
    }

    fn read(&mut self, path: &str) -> Result<Vec<u8>> {
        Ok(RUNTIME.block_on(self.sftp.read(path))?)
    }

    fn write(&mut self, path: &str, data: &[u8]) -> Result<()> {
        RUNTIME.block_on(async {
            let mut file = self
                .sftp
                .open_with_flags(path, OpenFlags::CREATE | OpenFlags::TRUNCATE | OpenFlags::WRITE)
                .await?;
            file.write_all(data).await?;
            file.flush().await?;
            file.shutdown().await?;
            Ok(())
        })
    }

    fn mkdir(&mut self, path: &str) -> Result<()> {
        Ok(RUNTIME.block_on(self.sftp.create_dir(path))?)
    }

    fn remove_file(&mut self, path: &str) -> Result<()> {
        Ok(RUNTIME.block_on(self.sftp.remove_file(path))?)
    }

    fn remove_dir(&mut self, path: &str) -> Result<()> {
        Ok(RUNTIME.block_on(self.sftp.remove_dir(path))?)
    }

    fn rename(&mut self, from: &str, to: &str) -> Result<()> {
        Ok(RUNTIME.block_on(self.sftp.rename(from, to))?)
    }

    fn chmod(&mut self, path: &str, mode: u32) -> Result<()> {
        let attrs = russh_sftp::protocol::FileAttributes { permissions: Some(mode & 0o7777), ..russh_sftp::protocol::FileAttributes::empty() };
        Ok(RUNTIME.block_on(self.sftp.set_metadata(path, attrs))?)
    }

    fn ping(&mut self) -> Result<()> {
        RUNTIME.block_on(self.sftp.canonicalize("."))?;
        Ok(())
    }

    fn stat(&mut self, path: &str) -> Result<Option<Stamp>> {
        match RUNTIME.block_on(self.sftp.metadata(path)) {
            Ok(meta) => Ok(Some(Stamp {
                size: meta.len(),
                mtime: meta.mtime.map(i64::from),
            })),
            Err(_) => Ok(None),
        }
    }
}
