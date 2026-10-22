//! FTP and FTPS (explicit TLS via Windows SChannel).

use std::io::Cursor;
use std::net::ToSocketAddrs;
use std::time::Duration;

use anyhow::{Context, Result, anyhow};
use suppaftp::list::{File, ListParser, PosixPexQuery};
use suppaftp::native_tls::TlsConnector;
use suppaftp::types::FileType;
use suppaftp::{Mode, NativeTlsConnector, NativeTlsFtpStream};

use super::{Backend, RemoteEntry, Stamp, join};
use crate::settings::{Protocol, Site};

const TIMEOUT: Duration = Duration::from_secs(20);

pub struct FtpBackend {
    stream: NativeTlsFtpStream,
    /// `None` until we know whether the server supports MLSD.
    mlsd: Option<bool>,
}

impl FtpBackend {
    pub fn connect(site: &Site, password: &str) -> Result<Self> {
        let addr = (site.host.as_str(), site.port)
            .to_socket_addrs()
            .with_context(|| format!("Host {} nicht gefunden", site.host))?
            .next()
            .ok_or_else(|| anyhow!("Host {} nicht gefunden", site.host))?;
        let mut stream = NativeTlsFtpStream::connect_timeout(addr, TIMEOUT)
            .with_context(|| format!("Verbindung zu {}:{} fehlgeschlagen", site.host, site.port))?;

        if site.protocol == Protocol::Ftps {
            let connector = TlsConnector::builder()
                .danger_accept_invalid_certs(site.accept_invalid_certs)
                .danger_accept_invalid_hostnames(site.accept_invalid_certs)
                .build()
                .context("TLS konnte nicht initialisiert werden")?;
            stream = stream
                .into_secure(NativeTlsConnector::from(connector), &site.host)
                .context("TLS-Aushandlung fehlgeschlagen")?;
        }

        let user = if site.user.is_empty() { "anonymous" } else { site.user.as_str() };
        stream
            .login(user, password)
            .context("Anmeldung fehlgeschlagen (Benutzername oder Passwort falsch?)")?;
        stream.transfer_type(FileType::Binary)?;
        if site.passive {
            stream.set_mode(Mode::Passive);
            // Servers behind NAT often announce a private address for PASV.
            stream.set_passive_nat_workaround(true);
        } else {
            stream.set_mode(Mode::Active);
        }
        Ok(Self { stream, mlsd: None })
    }

    /// Entries, each with a flag telling whether the listing had permissions.
    fn parse_list(&mut self, path: &str) -> Result<Vec<(File, bool)>> {
        if self.mlsd != Some(false) {
            match self.stream.mlsd(Some(path)) {
                Ok(lines) => {
                    self.mlsd = Some(true);
                    return Ok(lines
                        .iter()
                        .filter_map(|l| {
                            let has_mode = l.to_ascii_lowercase().contains("unix.mode=");
                            ListParser::parse_mlsd(l).ok().map(|f| (f, has_mode))
                        })
                        .collect());
                }
                Err(e) if self.mlsd == Some(true) => return Err(e.into()),
                Err(_) => self.mlsd = Some(false),
            }
        }
        let lines = self.stream.list(Some(path))?;
        Ok(lines
            .iter()
            .filter_map(|l| {
                ListParser::parse_posix(l)
                    .map(|f| (f, true))
                    .or_else(|_| ListParser::parse_dos(l).map(|f| (f, false)))
                    .ok()
            })
            .collect())
    }
}

impl Backend for FtpBackend {
    fn home(&mut self) -> Result<String> {
        let pwd = self.stream.pwd()?;
        Ok(super::normalize(&pwd))
    }

    fn list(&mut self, path: &str) -> Result<Vec<RemoteEntry>> {
        let files = self.parse_list(path)?;
        Ok(files
            .into_iter()
            .filter(|(f, _)| f.name() != "." && f.name() != "..")
            .map(|(f, has_mode)| RemoteEntry {
                mode: has_mode.then(|| mode_of(&f)),
                path: join(path, f.name()),
                name: f.name().to_string(),
                // Symlinks are usually directories on web hosts; treat a
                // link without extension as one.
                is_dir: f.is_directory() || (f.is_symlink() && !f.name().contains('.')),
                size: f.size() as u64,
                mtime: f.modified().duration_since(std::time::UNIX_EPOCH).ok().map(|d| d.as_secs() as i64),
            })
            .collect())
    }

    fn read(&mut self, path: &str) -> Result<Vec<u8>> {
        Ok(self.stream.retr_as_buffer(path)?.into_inner())
    }

    fn write(&mut self, path: &str, data: &[u8]) -> Result<()> {
        self.stream.put_file(path, &mut Cursor::new(data))?;
        Ok(())
    }

    fn mkdir(&mut self, path: &str) -> Result<()> {
        Ok(self.stream.mkdir(path)?)
    }

    fn remove_file(&mut self, path: &str) -> Result<()> {
        Ok(self.stream.rm(path)?)
    }

    fn remove_dir(&mut self, path: &str) -> Result<()> {
        Ok(self.stream.rmdir(path)?)
    }

    fn rename(&mut self, from: &str, to: &str) -> Result<()> {
        Ok(self.stream.rename(from, to)?)
    }

    fn chmod(&mut self, path: &str, mode: u32) -> Result<()> {
        self.stream
            .site(format!("CHMOD {:03o} {path}", mode & 0o777))
            .context("Der Server unterstützt SITE CHMOD nicht")?;
        Ok(())
    }

    fn ping(&mut self) -> Result<()> {
        Ok(self.stream.noop()?)
    }

    fn stat(&mut self, path: &str) -> Result<Option<Stamp>> {
        let Ok(size) = self.stream.size(path) else {
            return Ok(None);
        };
        // MDTM is optional; without it conflicts are detected by size only.
        let mtime = self.stream.mdtm(path).ok().map(|t| t.and_utc().timestamp());
        Ok(Some(Stamp { size: size as u64, mtime }))
    }
}

/// Permission bits of a listing entry.
fn mode_of(f: &File) -> u32 {
    let mut mode = 0;
    for (who, shift) in [(PosixPexQuery::Owner, 6), (PosixPexQuery::Group, 3), (PosixPexQuery::Others, 0)] {
        mode |= (f.can_read(who) as u32) << (shift + 2)
            | (f.can_write(who) as u32) << (shift + 1)
            | (f.can_execute(who) as u32) << shift;
    }
    mode
}

impl Drop for FtpBackend {
    fn drop(&mut self) {
        let _ = self.stream.quit();
    }
}
