//! Node.js for the agents: an installed Node is used, otherwise Nebula's own
//! copy in `<data dir>/runtime/node`, downloaded from nodejs.org on
//! request (checked against the published SHA-256 sums). Packages fetched by
//! npx land in `<data dir>/agents`, so everything stays in one place.

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

/// The folder of Nebula's own `node` program (`bin` on Linux).
fn private_bin_dir() -> PathBuf {
    if cfg!(windows) { private_dir() } else { private_dir().join("bin") }
}

/// File name of `node`, `npx` or `npm` in a Node folder.
fn tool_file(base: &str) -> String {
    match (cfg!(windows), base) {
        (true, "node") => "node.exe".into(),
        (true, _) => format!("{base}.cmd"),
        (false, _) => base.into(),
    }
}

/// Node.js release flavor for this system in nodejs.org's index.
#[cfg(windows)]
const DIST: (&str, &str) = ("win-x64-zip", "win-x64.zip");
#[cfg(all(not(windows), target_arch = "aarch64"))]
const DIST: (&str, &str) = ("linux-arm64", "linux-arm64.tar.gz");
#[cfg(all(not(windows), not(target_arch = "aarch64")))]
const DIST: (&str, &str) = ("linux-x64", "linux-x64.tar.gz");

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
    if private_bin_dir().join(tool_file("node")).is_file() {
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
        let program = crate::ext::process::which(command, &env).ok_or_else(|| t!("„{command}“ wurde nicht gefunden.", command = command))?;
        return Ok((program, env));
    }
    let dir = match status() {
        Status::Private(_) => private_bin_dir(),
        Status::System(dir) => dir,
        Status::Missing => return Err(MISSING.into()),
    };
    let program = dir.join(tool_file(&base));
    if !program.is_file() {
        return Err(t!("{program} wurde nicht gefunden.", program = program.display()));
    }
    // The agent itself starts node again: our Node first in PATH.
    let path = std::env::var("PATH").unwrap_or_default();
    env.entry("PATH".into()).or_insert_with(|| {
        let dirs = std::iter::once(dir.clone()).chain(std::env::split_paths(&path));
        std::env::join_paths(dirs).map_or(path.clone(), |p| p.to_string_lossy().into_owned())
    });
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

/// Downloads the current LTS version of Node.js for this system into
/// `private_dir`. Returns the version.
pub fn download(progress: Arc<Progress>) -> Result<String, String> {
    let http = agent();
    let get = |url: &str| http.get(url).call().map_err(|e| format!("{url}: {e}"));

    // Newest LTS release that has a build for this system.
    let index: serde_json::Value =
        serde_json::from_reader(get("https://nodejs.org/dist/index.json")?.body_mut().as_reader()).map_err(|e| e.to_string())?;
    let version = index
        .as_array()
        .and_then(|list| {
            list.iter().find(|r| {
                r["lts"].as_str().is_some() && r["files"].as_array().is_some_and(|f| f.iter().any(|x| x == DIST.0))
            })
        })
        .and_then(|r| r["version"].as_str())
        .ok_or(t!("Keine passende Node.js-Version gefunden."))?
        .to_string();
    let file = format!("node-{version}-{}", DIST.1);

    // Expected checksum.
    let sums = get(&format!("https://nodejs.org/dist/{version}/SHASUMS256.txt"))?
        .body_mut()
        .read_to_string()
        .map_err(|e| e.to_string())?;
    let expected = sums
        .lines()
        .find_map(|l| l.split_once("  ").filter(|(_, name)| name.trim() == file).map(|(hash, _)| hash.trim().to_ascii_lowercase()))
        .ok_or(t!("Prüfsumme nicht gefunden."))?;

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
        return Err(t!("Die Prüfsumme des Downloads stimmt nicht – abgebrochen.").into());
    }
    install_archive(&bytes, &version)?;
    Ok(version)
}

/// Unpacks `node-vX-…/…` into `private_dir` (replacing an old copy).
fn install_archive(bytes: &[u8], version: &str) -> Result<(), String> {
    let runtime = runtime_dir();
    let tmp = runtime.join("node.download");
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).map_err(|e| e.to_string())?;
    unpack(bytes, &tmp)?;
    let node = if cfg!(windows) { tmp.join(tool_file("node")) } else { tmp.join("bin").join("node") };
    if !node.is_file() {
        return Err(t!("Im Archiv fehlt node.exe.").into());
    }
    std::fs::write(tmp.join("VERSION"), version).map_err(|e| e.to_string())?;
    // Swap by renaming: a Node still in use fails cleanly instead of being half deleted.
    let dir = private_dir();
    // Agents left over (e.g. after a crash of Nebula) keep node.exe open.
    kill_processes_in(&dir);
    let old = runtime.join("node.old");
    let _ = std::fs::remove_dir_all(&old);
    if dir.exists() {
        std::fs::rename(&dir, &old).map_err(|e| t!("Node.js wird noch benutzt – laufende Agenten beenden und erneut versuchen ({e}).", e = e))?;
    }
    std::fs::rename(&tmp, &dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let _ = std::fs::remove_dir_all(&old);
    Ok(())
}

/// The .tar.gz goes through the system's `tar`, which keeps the symlinks
/// (`bin/npx` → `../lib/node_modules/…`) and the executable bits.
#[cfg(not(windows))]
fn unpack(bytes: &[u8], dest: &Path) -> Result<(), String> {
    let archive = dest.with_extension("tar.gz");
    std::fs::write(&archive, bytes).map_err(|e| e.to_string())?;
    let status = std::process::Command::new("tar")
        .arg("-xzf")
        .arg(&archive)
        .arg("-C")
        .arg(dest)
        .arg("--strip-components=1")
        .status();
    let _ = std::fs::remove_file(&archive);
    match status {
        Ok(s) if s.success() => Ok(()),
        Ok(s) => Err(format!("tar: {s}")),
        Err(e) => Err(format!("tar: {e}")),
    }
}

#[cfg(windows)]
fn unpack(bytes: &[u8], tmp: &Path) -> Result<(), String> {
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes)).map_err(|e| e.to_string())?;
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
    Ok(())
}

/// Ends all processes whose program lies in `dir` (only Nebula's own Node, never
/// an installed one), and waits briefly until they are gone.
#[cfg(not(windows))]
pub fn kill_processes_in(dir: &Path) {
    if dir.as_os_str().is_empty() {
        return;
    }
    let Ok(procs) = std::fs::read_dir("/proc") else { return };
    let mut killed = Vec::new();
    for entry in procs.flatten() {
        let Some(pid) = entry.file_name().to_str().and_then(|n| n.parse::<libc::pid_t>().ok()) else { continue };
        if std::fs::read_link(entry.path().join("exe")).is_ok_and(|exe| exe.starts_with(dir)) {
            // SAFETY: plain signal to a process of this user.
            unsafe { libc::kill(pid, libc::SIGKILL) };
            killed.push(pid);
        }
    }
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
    while killed.iter().any(|pid| Path::new(&format!("/proc/{pid}")).exists()) && std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
}

/// Ends all processes whose program lies in `dir` (only Nebula's own Node, never
/// an installed one), and waits briefly until they are gone.
#[cfg(windows)]
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
    #[cfg(windows)]
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
        assert!(program.to_string_lossy().ends_with(&tool_file("npx")));
        assert!(env["npm_config_cache"].ends_with("agents"));
        assert!(env["PATH"].contains(if cfg!(windows) { ';' } else { ':' }));
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
    let out = std::process::Command::new(private_bin_dir().join(tool_file("node"))).arg("--version").output().unwrap();
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), version);
    assert!(private_bin_dir().join(tool_file("npx")).exists());
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
    let mut child = std::process::Command::new(private_bin_dir().join(tool_file("node"))).args(["-e", "setTimeout(() => {}, 60000)"]).spawn().unwrap();
    std::thread::sleep(std::time::Duration::from_millis(500));
    download(Arc::new(Progress::default())).unwrap();
    assert!(private_bin_dir().join(tool_file("node")).is_file());
    assert!(!runtime_dir().join("node.old").exists() && !runtime_dir().join("node.download").exists());
    // The old process was ended.
    assert!(child.try_wait().unwrap().is_some());
}
