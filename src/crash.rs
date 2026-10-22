//! Crash reports: a panic is written to `crashes/` in the config folder and
//! offered on the next start (unsaved contents come back through the backups).

use std::path::PathBuf;

/// Reports kept; older ones are deleted.
const KEEP: usize = 10;
/// Names the report not yet shown to the user.
const PENDING: &str = "pending";

fn dir() -> PathBuf {
    crate::settings::config_dir().join("crashes")
}

/// Installs the panic hook. Call first thing in `main`.
pub fn install() {
    let default = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let message = info
            .payload()
            .downcast_ref::<&str>()
            .map(|s| s.to_string())
            .or_else(|| info.payload().downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "(unbekannt)".into());
        let location = info.location().map(|l| format!("{}:{}", l.file(), l.line())).unwrap_or_default();
        let thread = std::thread::current().name().unwrap_or("unbenannt").to_string();
        let backtrace = std::backtrace::Backtrace::force_capture();
        let _ = write_report(&dir(), &message, &location, &thread, &backtrace.to_string());
        default(info);
    }));
}

fn write_report(dir: &std::path::Path, message: &str, location: &str, thread: &str, backtrace: &str) -> std::io::Result<()> {
    let ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or_default();
    let t = crate::win32::local_time(ms);
    let stamp = format!("{:04}-{:02}-{:02} {:02}:{:02}", t.year, t.month, t.day, t.hour, t.minute);
    // Seconds are the same in every time zone.
    let name = format!("crash-{:04}{:02}{:02}-{:02}{:02}{:02}-{:03}.txt", t.year, t.month, t.day, t.hour, t.minute, ms / 1000 % 60, ms % 1000);
    let text = format!(
        "Nebula {version} – Absturzbericht\n\
         Zeit: {stamp}\n\
         System: {os} {arch}\n\
         Thread: {thread}\n\
         Meldung: {message}\n\
         Stelle: {location}\n\
         \n\
         Backtrace:\n{backtrace}\n",
        version = env!("CARGO_PKG_VERSION"),
        os = std::env::consts::OS,
        arch = std::env::consts::ARCH,
    );
    std::fs::create_dir_all(dir)?;
    std::fs::write(dir.join(&name), text)?;
    std::fs::write(dir.join(PENDING), &name)?;
    prune(dir);
    Ok(())
}

fn prune(dir: &std::path::Path) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    let mut reports: Vec<PathBuf> = rd
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "txt"))
        .collect();
    // Names start with the date, so sorting by name sorts by age.
    reports.sort();
    let excess = reports.len().saturating_sub(KEEP);
    for p in &reports[..excess] {
        let _ = std::fs::remove_file(p);
    }
}

/// The last crash, if it hasn't been shown yet.
pub struct Report {
    pub path: PathBuf,
    pub message: String,
    pub location: String,
    pub text: String,
}

/// Takes the report of the last crash (only once).
pub fn take_pending() -> Option<Report> {
    take_pending_in(&dir())
}

fn take_pending_in(dir: &std::path::Path) -> Option<Report> {
    let marker = dir.join(PENDING);
    let name = std::fs::read_to_string(&marker).ok()?;
    let _ = std::fs::remove_file(&marker);
    let path = dir.join(name.trim());
    let text = std::fs::read_to_string(&path).ok()?;
    let field = |key: &str| {
        text.lines()
            .find_map(|l| l.strip_prefix(key))
            .unwrap_or("")
            .trim()
            .to_string()
    };
    Some(Report { message: field("Meldung:"), location: field("Stelle:"), text: text.clone(), path })
}

#[cfg(test)]
mod tests {
    #[test]
    fn report_is_written_and_offered_once() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        for i in 0..12 {
            super::write_report(dir, &format!("Fehler {i}"), "src/x.rs:1", "main", "bt").unwrap();
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        let report = super::take_pending_in(dir).unwrap();
        assert_eq!(report.message, "Fehler 11");
        assert_eq!(report.location, "src/x.rs:1");
        assert!(super::take_pending_in(dir).is_none());
        let count = std::fs::read_dir(dir).unwrap().count();
        assert_eq!(count, super::KEEP);
    }
}
