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
    native::install();
}

/// Hard crashes in native code (access violations in drivers, WebView, …)
/// never reach the panic hook; Windows hands them to this filter.
mod native {
    use std::sync::atomic::{AtomicBool, Ordering};

    use windows_sys::Win32::Foundation::{GENERIC_WRITE, HMODULE, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::Storage::FileSystem::{CREATE_ALWAYS, CreateFileW, FILE_ATTRIBUTE_NORMAL};
    use windows_sys::Win32::System::Diagnostics::Debug::{
        EXCEPTION_POINTERS, MINIDUMP_EXCEPTION_INFORMATION, MiniDumpNormal, MiniDumpWriteDump, SetUnhandledExceptionFilter,
    };
    use windows_sys::Win32::System::LibraryLoader::{
        GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS, GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT, GetModuleFileNameW, GetModuleHandleExW,
    };
    use windows_sys::Win32::System::Threading::{GetCurrentProcess, GetCurrentProcessId, GetCurrentThreadId};

    /// Only the first crash is written (a crash while writing must not loop).
    static HANDLED: AtomicBool = AtomicBool::new(false);

    pub fn install() {
        // SAFETY: registers a plain function pointer.
        unsafe { SetUnhandledExceptionFilter(Some(filter)) };
    }

    /// Readable names of the usual exception codes.
    pub(super) fn code_name(code: u32) -> &'static str {
        match code {
            0xC000_0005 => "Zugriffsverletzung",
            0xC000_001D => "Ungültiger Befehl",
            0xC000_0094 => "Division durch null",
            0xC000_00FD => "Stapelüberlauf",
            0xC000_0409 => "Stapel beschädigt",
            0xC000_0374 => "Heap beschädigt",
            0x8000_0003 => "Haltepunkt",
            _ => "Ausnahme",
        }
    }

    /// Module that contains `addr` and the offset into it ("nvwgf2umx.dll+0x1A2B").
    fn module_of(addr: usize) -> String {
        let mut module: HMODULE = std::ptr::null_mut();
        // SAFETY: the address is only looked up, not dereferenced; the name buffer is sized.
        unsafe {
            let flags = GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS | GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT;
            if GetModuleHandleExW(flags, addr as *const u16, &mut module) == 0 {
                return format!("0x{addr:X}");
            }
            let mut buf = [0u16; 260];
            let n = GetModuleFileNameW(module, buf.as_mut_ptr(), buf.len() as u32) as usize;
            let path = String::from_utf16_lossy(&buf[..n]);
            let name = path.rsplit('\\').next().unwrap_or(&path).to_string();
            format!("{name}+0x{:X}", addr - module as usize)
        }
    }

    unsafe extern "system" fn filter(info: *const EXCEPTION_POINTERS) -> i32 {
        const CONTINUE_SEARCH: i32 = 0;
        if HANDLED.swap(true, Ordering::SeqCst) || info.is_null() {
            return CONTINUE_SEARCH;
        }
        // SAFETY: Windows passes valid exception pointers for the faulting thread.
        let (code, addr) = unsafe {
            let rec = &*(*info).ExceptionRecord;
            (rec.ExceptionCode as u32, rec.ExceptionAddress as usize)
        };
        let message = format!("{} (0x{code:08X})", code_name(code));
        let location = module_of(addr);
        let thread = std::thread::current().name().unwrap_or("unbenannt").to_string();
        let backtrace = std::backtrace::Backtrace::force_capture().to_string();
        if let Ok(name) = super::write_report(&super::dir(), &message, &location, &thread, &backtrace) {
            // A small memory dump next to the report, for a debugger.
            let dump = super::dir().join(name.replace(".txt", ".dmp"));
            let wide: Vec<u16> = dump.as_os_str().encode_wide().chain(std::iter::once(0)).collect();
            // SAFETY: plain Win32 calls with valid handles and the exception info of this thread.
            unsafe {
                let file = CreateFileW(wide.as_ptr(), GENERIC_WRITE, 0, std::ptr::null(), CREATE_ALWAYS, FILE_ATTRIBUTE_NORMAL, std::ptr::null_mut());
                if file != INVALID_HANDLE_VALUE {
                    let exception = MINIDUMP_EXCEPTION_INFORMATION { ThreadId: GetCurrentThreadId(), ExceptionPointers: info as *mut _, ClientPointers: 0 };
                    let ok = MiniDumpWriteDump(GetCurrentProcess(), GetCurrentProcessId(), file, MiniDumpNormal, &exception, std::ptr::null(), std::ptr::null());
                    windows_sys::Win32::Foundation::CloseHandle(file);
                    if ok != 0 {
                        use std::io::Write;
                        let note = format!("\nSpeicherabbild (für einen Debugger): {}\n", dump.display());
                        if let Ok(mut f) = std::fs::OpenOptions::new().append(true).open(super::dir().join(&name)) {
                            let _ = f.write_all(note.as_bytes());
                        }
                    }
                }
            }
        }
        // Windows ends the process as usual (and may report it).
        CONTINUE_SEARCH
    }

    use std::os::windows::ffi::OsStrExt;

    /// Crashes on purpose with an access violation (NEBULA_CRASH_TEST=native).
    pub fn crash_now() {
        use windows_sys::Win32::System::Diagnostics::Debug::{RaiseException, SEM_NOGPFAULTERRORBOX, SetErrorMode};
        // SAFETY: no Windows error dialog for the test; then an exception nobody handles ends the process.
        unsafe {
            SetErrorMode(SEM_NOGPFAULTERRORBOX);
            RaiseException(0xC000_0005, 0, 0, std::ptr::null());
        }
    }
}

pub use native::crash_now as native_crash_test;

/// Writes a report and marks it to be shown on the next start; returns its file name.
fn write_report(dir: &std::path::Path, message: &str, location: &str, thread: &str, backtrace: &str) -> std::io::Result<String> {
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
    Ok(name)
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
        // The memory dump of a native crash goes with its report.
        let _ = std::fs::remove_file(p.with_extension("dmp"));
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
