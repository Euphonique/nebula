//! MySQL/MariaDB on a server, reached through an SSH tunnel: a local port is
//! forwarded over the server's SSH login to the database (`direct-tcpip`),
//! and the client talks to that port.

use std::sync::Arc;
use std::time::Instant;

use crossbeam_channel::{Receiver, Sender};
use eframe::egui;
use mysql_async::prelude::Queryable;
use mysql_async::{Row, Value};

use super::HostKeyPrompt;
use super::sftp::{PromptFn, RUNTIME, open_session};
use crate::settings::Site;

/// Rows kept per result; the rest is only counted.
pub const MAX_ROWS: usize = 1000;
/// Longest text shown per cell.
const MAX_CELL: usize = 2000;

/// One result set of a query.
#[derive(Clone, Debug, Default)]
pub struct Output {
    pub columns: Vec<String>,
    /// Cells as text; `None` is SQL NULL.
    pub rows: Vec<Vec<Option<String>>>,
    /// Rows in the result (also those not kept).
    pub total: usize,
    pub affected: u64,
    pub millis: u128,
}

pub enum DbEvent {
    HostKey(HostKeyPrompt),
    /// Connected; the server's version.
    Connected(String),
    /// The result sets of a query (with the query's id).
    Done(u64, Vec<Output>),
    /// A query failed (with its id).
    Failed(u64, String),
    /// The connection ended or could not be made.
    Closed(Option<String>),
}

enum Command {
    Query(u64, String),
}

/// A database connection; dropping it closes connection and tunnel.
pub struct DbSession {
    pub events: Receiver<DbEvent>,
    commands: tokio::sync::mpsc::UnboundedSender<Command>,
    next: u64,
}

impl DbSession {
    /// Runs SQL; the answer comes as [`DbEvent::Done`] or [`DbEvent::Failed`] with this id.
    pub fn query(&mut self, sql: &str) -> u64 {
        self.next += 1;
        let _ = self.commands.send(Command::Query(self.next, sql.to_string()));
        self.next
    }
}

/// Connects to the database of `site` (SSH login of the site, then the database).
pub fn connect(site: Site, db_password: String, ctx: egui::Context) -> DbSession {
    connect_with(site, None, db_password, ctx)
}

/// Like [`connect`]; `ssh_password` instead of the saved one (tests).
pub fn connect_with(mut site: Site, ssh_password: Option<String>, db_password: String, ctx: egui::Context) -> DbSession {
    // FTP servers usually offer SSH on the standard port (like the terminal).
    if site.protocol != crate::settings::Protocol::Sftp {
        site.port = 22;
    }
    let (tx, events) = crossbeam_channel::unbounded();
    let (commands, rx) = tokio::sync::mpsc::unbounded_channel();
    RUNTIME.spawn(async move {
        let result = run(site, ssh_password, db_password, tx.clone(), rx, ctx.clone()).await;
        let _ = tx.send(DbEvent::Closed(result.err().map(|e| format!("{e:#}"))));
        ctx.request_repaint();
    });
    DbSession { events, commands, next: 0 }
}

async fn run(
    site: Site,
    ssh_password: Option<String>,
    db_password: String,
    tx: Sender<DbEvent>,
    mut commands: tokio::sync::mpsc::UnboundedReceiver<Command>,
    ctx: egui::Context,
) -> anyhow::Result<()> {
    let password = match ssh_password {
        Some(p) => p,
        None => {
            let site = site.clone();
            tokio::task::spawn_blocking(move || site.password()).await?.unwrap_or_default()
        }
    };
    let prompt = {
        let (tx, ctx) = (tx.clone(), ctx.clone());
        move |p: HostKeyPrompt| {
            let _ = tx.send(DbEvent::HostKey(p));
            ctx.request_repaint();
        }
    };
    let session = Arc::new(open_session(&site, &password, Arc::new(Box::new(prompt) as PromptFn)).await?);

    // The tunnel: every connection to the local port becomes a channel to the database.
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0)).await?;
    let local_port = listener.local_addr()?.port();
    let (db_host, db_port) = (site.db.host.trim().to_string(), site.db.port);
    let forward = {
        let session = session.clone();
        tokio::spawn(async move {
            while let Ok((mut socket, peer)) = listener.accept().await {
                let session = session.clone();
                let host = db_host.clone();
                tokio::spawn(async move {
                    let Ok(channel) = session.channel_open_direct_tcpip(host, db_port as u32, peer.ip().to_string(), peer.port() as u32).await else {
                        return;
                    };
                    let mut stream = channel.into_stream();
                    let _ = tokio::io::copy_bidirectional(&mut socket, &mut stream).await;
                });
            }
        })
    };

    let opts = mysql_async::OptsBuilder::default()
        .ip_or_hostname("127.0.0.1")
        .tcp_port(local_port)
        .user(Some(site.db.user.clone()))
        .pass(Some(db_password))
        .db_name((!site.db.database.trim().is_empty()).then(|| site.db.database.trim().to_string()))
        .prefer_socket(false);
    // A database that never answers must not leave the tab waiting forever.
    let connected = tokio::time::timeout(std::time::Duration::from_secs(20), mysql_async::Conn::new(opts)).await;
    let mut conn = match connected {
        Ok(Ok(c)) => c,
        Ok(Err(e)) => {
            forward.abort();
            return Err(anyhow::anyhow!(t!("Datenbank-Anmeldung fehlgeschlagen: {e}", e = e)));
        }
        Err(_) => {
            forward.abort();
            return Err(anyhow::anyhow!(t!("Die Datenbank antwortet nicht ({host}:{port} vom Server aus).", host = site.db.host, port = site.db.port)));
        }
    };
    let version = conn.server_version();
    let _ = tx.send(DbEvent::Connected(format!("{}.{}.{}", version.0, version.1, version.2)));
    ctx.request_repaint();

    while let Some(Command::Query(id, sql)) = commands.recv().await {
        let started = Instant::now();
        match run_query(&mut conn, &sql).await {
            Ok(mut outputs) => {
                let millis = started.elapsed().as_millis();
                for o in &mut outputs {
                    o.millis = millis;
                }
                let _ = tx.send(DbEvent::Done(id, outputs));
            }
            Err(e) => {
                let _ = tx.send(DbEvent::Failed(id, e.to_string()));
            }
        }
        ctx.request_repaint();
    }
    // The session was dropped.
    let _ = conn.disconnect().await;
    forward.abort();
    Ok(())
}

/// All result sets of `sql`, at most [`MAX_ROWS`] rows kept per set.
async fn run_query(conn: &mut mysql_async::Conn, sql: &str) -> mysql_async::Result<Vec<Output>> {
    let mut result = conn.query_iter(sql).await?;
    let mut outputs = Vec::new();
    loop {
        let columns: Vec<String> = result.columns().map(|c| c.iter().map(|c| c.name_str().into_owned()).collect()).unwrap_or_default();
        let mut out = Output { columns, ..Default::default() };
        result
            .for_each(|row: Row| {
                out.total += 1;
                if out.rows.len() < MAX_ROWS {
                    out.rows.push(row.unwrap().into_iter().map(cell).collect());
                }
            })
            .await?;
        out.affected = result.affected_rows();
        outputs.push(out);
        if result.is_empty() {
            break;
        }
    }
    Ok(outputs)
}

/// A value as text (`None` for NULL).
fn cell(v: Value) -> Option<String> {
    let text = match v {
        Value::NULL => return None,
        Value::Bytes(b) => match String::from_utf8(b) {
            Ok(s) => s,
            Err(e) => format!("0x{}", e.as_bytes().iter().take(64).map(|b| format!("{b:02X}")).collect::<String>()),
        },
        Value::Int(n) => n.to_string(),
        Value::UInt(n) => n.to_string(),
        Value::Float(f) => f.to_string(),
        Value::Double(f) => f.to_string(),
        Value::Date(y, m, d, h, mi, s, us) => {
            if (h, mi, s, us) == (0, 0, 0, 0) {
                format!("{y:04}-{m:02}-{d:02}")
            } else {
                format!("{y:04}-{m:02}-{d:02} {h:02}:{mi:02}:{s:02}")
            }
        }
        Value::Time(neg, d, h, m, s, _) => format!("{}{:02}:{m:02}:{s:02}", if neg { "-" } else { "" }, d * 24 + h as u32),
    };
    Some(if text.chars().count() > MAX_CELL { text.chars().take(MAX_CELL).chain(['…']).collect() } else { text })
}

/// Whether `sql` only reads (SELECT, SHOW, …); other statements are confirmed first.
pub fn is_read_only(sql: &str) -> bool {
    let statements = sql.split(';').map(str::trim).filter(|s| !s.is_empty());
    let mut any = false;
    for s in statements {
        any = true;
        // Comments in front of the statement don't count.
        let s = s.lines().map(str::trim).filter(|l| !l.starts_with("--") && !l.starts_with('#')).collect::<Vec<_>>().join(" ");
        let first = s.split_whitespace().next().unwrap_or("").to_ascii_uppercase();
        if !matches!(first.as_str(), "SELECT" | "SHOW" | "DESCRIBE" | "DESC" | "EXPLAIN" | "USE" | "WITH") {
            return false;
        }
    }
    any
}

/// A table name for SQL (`name` with backticks doubled).
pub fn quote_ident(name: &str) -> String {
    format!("`{}`", name.replace('`', "``"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_only_and_quoting() {
        assert!(is_read_only("SELECT * FROM wp_posts LIMIT 10"));
        assert!(is_read_only("-- Kommentar\nshow tables;  describe x;"));
        assert!(!is_read_only("SELECT 1; DELETE FROM x"));
        assert!(!is_read_only("update wp_options set option_value='x'"));
        assert!(!is_read_only("   "));
        assert_eq!(quote_ident("wp`posts"), "`wp``posts`");
        assert_eq!(cell(Value::Date(2026, 10, 23, 0, 0, 0, 0)), Some("2026-10-23".into()));
        assert_eq!(cell(Value::Date(2026, 10, 23, 9, 5, 1, 0)), Some("2026-10-23 09:05:01".into()));
        assert_eq!(cell(Value::NULL), None);
        assert_eq!(cell(Value::Bytes(vec![0xff, 0x00])), Some("0xFF00".into()));
    }
}
