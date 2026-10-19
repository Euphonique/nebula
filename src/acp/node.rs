//! Node.js for the agents: an installed Node is used, otherwise Nebula's own
//! copy in `%APPDATA%\nebula\runtime\node`, downloaded from nodejs.org on
//! request (checked against the published SHA-256 sums). Packages fetched by
//! npx land in `%APPDATA%\nebula\agents`, so everything stays in one place.

use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use sha2::{Digest, Sha256};

/// Starting an agent failed because there is no Node.js.
pub const MISSING: &str = "Node.js fehlt";

pub fn runtime_dir() -> PathBuf {
    crate::settings::data_dir().join("runtime")
}

/// Nebula's own Node.js.
pub fn private_dir() -> PathBuf {
    runtime_dir().join("node")
}

/// npm cache for the agent packages.
pub fn agents_dir() -> PathBuf {
    crate::settings::data_dir().join("agents")
}

#[derive(Clone, Debug, PartialEq)]
pub enum Status {
    /// Nebula's copy (version).
    Private(String),
    /// Installed on the system (folder).
    System(PathBuf),
    Missing,
}

pub fn status() -> Status {
    if private_dir().join("node.exe").is_file() {
        let version = std::fs::read_to_string(private_dir().join("VERSION")).unwrap_or_default();
        return Status::Private(version.trim().to_string());
    }
    // For tests: behave as on a machine without Node.
    if std::env::var_os("NEBULA_IGNORE_SYSTEM_NODE").is_some() {
        return Status::Missing;
    }
    match crate::ext::process::which("node", &HashMap::new()) {
        Some(p) => Status::System(p.parent().map(Path::to_path_buf).unwrap_or_default()),
        None => Status::Missing,
    }
}

/// Program and environment for an agent command. Node tools (`npx`, `node`,
/// `npm`) run on the Node found by `status`, with Nebula's npm cache.
pub fn prepare(command: &str, mut env: HashMap<String, String>) -> Result<(PathBuf, HashMap<String, String>), String> {
    let base = command.trim().trim_end_matches(".cmd").trim_end_matches(".exe").to_ascii_lowercase();
    if !matches!(base.as_str(), "npx" | "node" | "npm") {
        let program = crate::ext::process::which(command, &env).ok_or_else(|| format!("„{command}“ wurde nicht gefunden."))?;
        return Ok((program, env));
    }
    let dir = match status() {
        Status::Private(_) => private_dir(),
        Status::System(dir) => dir,
        Status::Missing => return Err(MISSING.into()),
    };
    let program = if base == "node" { dir.join("node.exe") } else { dir.join(format!("{base}.cmd")) };
    if !program.is_file() {
        return Err(format!("{} wurde nicht gefunden.", program.display()));
    }
    // The agent itself starts node again: our Node first in PATH.
    let path = std::env::var("PATH").unwrap_or_default();
    env.entry("PATH".into()).or_insert_with(|| format!("{};{path}", dir.display()));
    env.entry("npm_config_cache".into()).or_insert_with(|| agents_dir().to_string_lossy().into_owned());
    // No prompts or update notices from npm in a pipe.
    env.entry("npm_config_update_notifier".into()).or_insert_with(|| "false".into());
    env.entry("npm_config_fund".into()).or_insert_with(|| "false".into());
    Ok((program, env))
}

/// Progress of a download (bytes so far / total, 0 if unknown).
#[derive(Default)]
pub struct Progress {
    pub done: AtomicU64,
    pub total: AtomicU64,
}

impl Progress {
    pub fn fraction(&self) -> Option<f32> {
        let total = self.total.load(Ordering::Relaxed);
        (total > 0).then(|| self.done.load(Ordering::Relaxed) as f32 / total as f32)
    }
}

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .tls_config(
            ureq::tls::TlsConfig::builder()
                .provider(ureq::tls::TlsProvider::NativeTls)
                .root_certs(ureq::tls::RootCerts::PlatformVerifier)
                .build(),
        )
        .user_agent(concat!("Nebula/", env!("CARGO_PKG_VERSION")))
        .build()
        .into()
}

/// Downloads the current LTS version of Node.js for Windows x64 into
/// `private_dir`. Returns the version.
pub fn download(progress: Arc<Progress>) -> Result<String, String> {
    let http = agent();
    let get = |url: &str| http.get(url).call().map_err(|e| format!("{url}: {e}"));

    // Newest LTS release that has a Windows zip.
    let index: serde_json::Value =
        serde_json::from_reader(get("https://nodejs.org/dist/index.json")?.body_mut().as_reader()).map_err(|e| e.to_string())?;
    let version = index
        .as_array()
        .and_then(|list| {
            list.iter().find(|r| {
                r["lts"].as_str().is_some() && r["files"].as_array().is_some_and(|f| f.iter().any(|x| x == "win-x64-zip"))
            })
        })
        .and_then(|r| r["version"].as_str())
        .ok_or("Keine passende Node.js-Version gefunden.")?
        .to_string();
    let file = format!("node-{version}-win-x64.zip");

    // Expected checksum.
    let sums = get(&format!("https://nodejs.org/dist/{version}/SHASUMS256.txt"))?
        .body_mut()
        .read_to_string()
        .map_err(|e| e.to_string())?;
    let expected = sums
        .lines()
        .find_map(|l| l.split_once("  ").filter(|(_, name)| name.trim() == file).map(|(hash, _)| hash.trim().to_ascii_lowercase()))
        .ok_or("Prüfsumme nicht gefunden.")?;

    // The archive, with progress.
    let mut resp = get(&format!("https://nodejs.org/dist/{version}/{file}"))?;
    let total = resp.headers().get("content-length").and_then(|v| v.to_str().ok()?.parse().ok()).unwrap_or(0);
    progress.total.store(total, Ordering::Relaxed);
    let mut reader = resp.body_mut().with_config().limit(200 * 1024 * 1024).reader();
    let mut bytes = Vec::with_capacity(total as usize);
    let mut buf = [0u8; 64 * 1024];
    loop {
        let n = reader.read(&mut buf).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        bytes.extend_from_slice(&buf[..n]);
        progress.done.store(bytes.len() as u64, Ordering::Relaxed);
    }
    let actual = Sha256::digest(&bytes).iter().map(|b| format!("{b:02x}")).collect::<String>();
    if actual != expected {
        return Err("Die Prüfsumme des Downloads stimmt nicht – abgebrochen.".into());
    }
    install_zip(&bytes, &version)?;
    Ok(version)
}

/// Unpacks `node-vX-win-x64/…` into `private_dir` (replacing an old copy).
fn install_zip(bytes: &[u8], version: &str) -> Result<(), String> {
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes)).map_err(|e| e.to_string())?;
    let runtime = runtime_dir();
    let tmp = runtime.join("node.download");
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).map_err(|e| e.to_string())?;
    for i in 0..zip.len() {
        let mut f = zip.by_index(i).map_err(|e| e.to_string())?;
        let Some(path) = f.enclosed_name() else { continue };
        // Drop the top folder "node-vX-win-x64".
        let rel: PathBuf = path.components().skip(1).collect();
        if rel.as_os_str().is_empty() {
            continue;
        }
        let dest = tmp.join(rel);
        if f.is_dir() {
            std::fs::create_dir_all(&dest).map_err(|e| e.to_string())?;
        } else {
            if let Some(parent) = dest.parent() {
                std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
            }
            let mut out = std::fs::File::create(&dest).map_err(|e| e.to_string())?;
            std::io::copy(&mut f, &mut out).map_err(|e| e.to_string())?;
        }
    }
    if !tmp.join("node.exe").is_file() {
        return Err("Im Archiv fehlt node.exe.".into());
    }
    std::fs::write(tmp.join("VERSION"), version).map_err(|e| e.to_string())?;
    // Swap by renaming: a Node still in use fails cleanly instead of being half deleted.
    let dir = private_dir();
    // Agents left over (e.g. after a crash of Nebula) keep node.exe open.
    kill_processes_in(&dir);
    let old = runtime.join("node.old");
    let _ = std::fs::remove_dir_all(&old);
    if dir.exists() {
        std::fs::rename(&dir, &old).map_err(|e| format!("Node.js wird noch benutzt – laufende Agenten beenden und erneut versuchen ({e})."))?;
    }
    std::fs::rename(&tmp, &dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let _ = std::fs::remove_dir_all(&old);
    Ok(())
}

/// Ends all processes whose program lies in `dir` (only Nebula's own Node, never
/// an installed one), and waits briefly until they are gone.
pub fn kill_processes_in(dir: &Path) {
    use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW, TH32CS_SNAPPROCESS,
    };
    use windows_sys::Win32::System::Threading::{
        OpenProcess, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_TERMINATE, QueryFullProcessImageNameW,
        TerminateProcess, WaitForSingleObject,
    };
    let want = dir.to_string_lossy().to_lowercase();
    if want.is_empty() {
        return;
    }
    // SAFETY: plain Win32 calls; every handle opened here is closed again.
    unsafe {
        let snap = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        if snap == INVALID_HANDLE_VALUE {
            return;
        }
        let mut entry: PROCESSENTRY32W = std::mem::zeroed();
        entry.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
        let mut ok = Process32FirstW(snap, &mut entry) != 0;
        while ok {
            let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_TERMINATE | 0x0010_0000, 0, entry.th32ProcessID);
            if !process.is_null() {
                let mut buf = [0u16; 1024];
                let mut len = buf.len() as u32;
                if QueryFullProcessImageNameW(process, PROCESS_NAME_WIN32, buf.as_mut_ptr(), &mut len) != 0 {
                    let path = String::from_utf16_lossy(&buf[..len as usize]).to_lowercase();
                    if path.starts_with(&want) {
                        TerminateProcess(process, 1);
                        // SYNCHRONIZE (0x0010_0000) lets us wait for it to end.
                        WaitForSingleObject(process, 3000);
                    }
                }
                CloseHandle(process);
            }
            ok = Process32NextW(snap, &mut entry) != 0;
        }
        CloseHandle(snap);
    }
}

/// Size of a folder in bytes (for the settings page).
pub fn folder_size(dir: &Path) -> u64 {
    let Ok(rd) = std::fs::read_dir(dir) else { return 0 };
    rd.flatten()
        .map(|e| match e.file_type() {
            Ok(t) if t.is_dir() => folder_size(&e.path()),
            Ok(_) => e.metadata().map_or(0, |m| m.len()),
            Err(_) => 0,
        })
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn other_commands_pass_through() {
        let (program, env) = prepare("cmd", HashMap::new()).unwrap();
        assert!(program.to_string_lossy().to_lowercase().ends_with("cmd.exe"));
        assert!(!env.contains_key("npm_config_cache"));
    }

    #[test]
    fn npx_gets_nebulas_cache() {
        // Only meaningful where some Node is available (the developer machine).
        if status() == Status::Missing {
            assert_eq!(prepare("npx", HashMap::new()).unwrap_err(), MISSING);
            return;
        }
        let (program, env) = prepare("npx", HashMap::new()).unwrap();
        assert!(program.to_string_lossy().ends_with("npx.cmd"));
        assert!(env["npm_config_cache"].ends_with("agents"));
        assert!(env["PATH"].contains(';'));
    }
}

/// Downloads Node.js for real (network): `cargo test download_node -- --ignored`.
#[test]
#[ignore]
fn download_node() {
    let tmp = tempfile::tempdir().unwrap();
    unsafe { std::env::set_var("NEBULA_CONFIG_DIR", tmp.path().join("config")) };
    let progress = Arc::new(Progress::default());
    let version = download(progress.clone()).unwrap();
    assert!(version.starts_with('v'));
    assert!(progress.fraction().unwrap() > 0.99);
    assert_eq!(status(), Status::Private(version.clone()));
    let out = std::process::Command::new(private_dir().join("node.exe")).arg("--version").output().unwrap();
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), version);
    assert!(private_dir().join("npx.cmd").is_file());
}


/// Replacing Nebula's Node while a process still runs from it (network):
/// `cargo test replace_node_in_use -- --ignored`.
#[test]
#[ignore]
fn replace_node_in_use() {
    let tmp = tempfile::tempdir().unwrap();
    unsafe { std::env::set_var("NEBULA_CONFIG_DIR", tmp.path().join("config")) };
    download(Arc::new(Progress::default())).unwrap();
    // An "agent" left running from the old copy.
    let mut child = std::process::Command::new(private_dir().join("node.exe")).args(["-e", "setTimeout(() => {}, 60000)"]).spawn().unwrap();
    std::thread::sleep(std::time::Duration::from_millis(500));
    download(Arc::new(Progress::default())).unwrap();
    assert!(private_dir().join("node.exe").is_file());
    assert!(!runtime_dir().join("node.old").exists() && !runtime_dir().join("node.download").exists());
    // The old process was ended.
    assert!(child.try_wait().unwrap().is_some());
}
