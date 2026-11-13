//! Linux integration: freedesktop conventions (trash, xdg-open, gsettings),
//! the clipboard through arboard and a Unix socket for the single instance.
//! The window itself runs on X11 (XWayland under Wayland), see Cargo.toml.

use std::io::{Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicUsize, Ordering};
use std::sync::{Mutex, OnceLock};

use eframe::egui::{self, Pos2, Rect};

static CTX: OnceLock<egui::Context> = OnceLock::new();

/// `creation_flags` (CREATE_NO_WINDOW on Windows) has no meaning here: child
/// processes never open a console window.
pub trait CommandExt {
    fn creation_flags(&mut self, flags: u32) -> &mut Self;
}

impl CommandExt for Command {
    fn creation_flags(&mut self, _flags: u32) -> &mut Self {
        self
    }
}

/// Starts a child as leader of a new process group, so `ext::process::kill`
/// ends it together with everything it started (npx → node).
pub trait ProcessGroupExt {
    fn own_process_group(&mut self) -> &mut Self;
}

impl ProcessGroupExt for Command {
    fn own_process_group(&mut self) -> &mut Self {
        std::os::unix::process::CommandExt::process_group(self, 0)
    }
}

/// The Xlib display (winit's connection) and the window, for `cursor_pos`
/// and `drag_files_out`.
static X_DISPLAY: AtomicUsize = AtomicUsize::new(0);
static X_WINDOW: AtomicUsize = AtomicUsize::new(0);

/// Remembers the context and starts listening for further starts. Safe to call every frame.
pub fn init(frame: &eframe::Frame, ctx: &egui::Context) {
    if CTX.set(ctx.clone()).is_ok() {
        use raw_window_handle::{HasDisplayHandle, HasWindowHandle, RawDisplayHandle, RawWindowHandle};
        if let Ok(handle) = frame.display_handle()
            && let RawDisplayHandle::Xlib(x) = handle.as_raw()
        {
            X_DISPLAY.store(x.display.map_or(0, |d| d.as_ptr() as usize), Ordering::Relaxed);
        }
        if let Ok(handle) = frame.window_handle()
            && let RawWindowHandle::Xlib(x) = handle.as_raw()
        {
            X_WINDOW.store(x.window as usize, Ordering::Relaxed);
        }
        start_instance_listener();
    }
}

/// Windows shows snap layouts on the maximize button; nothing to do here.
pub fn set_maximize_button(_rect: Rect, _pixels_per_point: f32) {}

pub fn maximize_hovered() -> bool {
    false
}

/// Cursor position relative to the window, in points. Asks the X server, so
/// it also works while files from another program hover over the window
/// (no mouse events arrive then); else egui's last position.
pub fn cursor_pos(pixels_per_point: f32) -> Option<Pos2> {
    x_query_pointer()
        .map(|(x, y)| Pos2::new(x as f32 / pixels_per_point, y as f32 / pixels_per_point))
        .or_else(|| CTX.get()?.input(|i| i.pointer.latest_pos()))
}

fn x_query_pointer() -> Option<(i32, i32)> {
    let display = X_DISPLAY.load(Ordering::Relaxed) as *mut x11_dl::xlib::Display;
    let window = X_WINDOW.load(Ordering::Relaxed) as x11_dl::xlib::Window;
    if display.is_null() || window == 0 {
        return None;
    }
    static XLIB: OnceLock<Option<x11_dl::xlib::Xlib>> = OnceLock::new();
    let xlib = XLIB.get_or_init(|| x11_dl::xlib::Xlib::open().ok()).as_ref()?;
    let (mut root, mut child) = (0, 0);
    let (mut rx, mut ry, mut wx, mut wy, mut mask) = (0, 0, 0, 0, 0);
    // SAFETY: winit's live display and window; plain out-parameters.
    let same_screen = unsafe {
        (xlib.XQueryPointer)(display, window, &mut root, &mut child, &mut rx, &mut ry, &mut wx, &mut wy, &mut mask)
    };
    (same_screen != 0).then_some((wx, wy))
}

/// Mica and the DWM dark title bar are Windows-only; the window draws its own frame.
pub fn set_backdrop(_dark: bool, _mica: bool) {}

// ---- system theme ------------------------------------------------------------

fn gsettings(schema: &str, key: &str) -> Option<String> {
    let out = Command::new("gsettings").args(["get", schema, key]).stderr(Stdio::null()).output().ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).trim().trim_matches('\'').to_string())
}

/// Cached: 0 = not read yet, 1 = light, 2 = dark.
static SYSTEM_DARK: AtomicU8 = AtomicU8::new(0);

fn read_system_dark() -> bool {
    // GNOME 42+ (Zorin, Ubuntu …): the global style switch.
    match gsettings("org.gnome.desktop.interface", "color-scheme").as_deref() {
        Some("prefer-dark") => return true,
        Some("prefer-light") => return false,
        _ => {}
    }
    // Older desktops: a theme named "…-dark".
    let theme = gsettings("org.gnome.desktop.interface", "gtk-theme")
        .or_else(|| std::env::var("GTK_THEME").ok())
        .unwrap_or_default();
    if theme.is_empty() { true } else { theme.to_lowercase().contains("dark") }
}

/// Whether the desktop prefers dark applications.
pub fn system_dark() -> bool {
    match SYSTEM_DARK.load(Ordering::Relaxed) {
        0 => {
            let dark = read_system_dark();
            SYSTEM_DARK.store(if dark { 2 } else { 1 }, Ordering::Relaxed);
            dark
        }
        v => v == 2,
    }
}

/// Reads the desktop style again; true when it changed.
pub fn refresh_system_dark() -> bool {
    let before = system_dark();
    let dark = read_system_dark();
    SYSTEM_DARK.store(if dark { 2 } else { 1 }, Ordering::Relaxed);
    dark != before
}

/// Cached: 0 = not read yet, 1 = off, 2 = on (dark), 3 = on (light).
static HIGH_CONTRAST: AtomicU8 = AtomicU8::new(0);

fn read_high_contrast() -> u8 {
    if gsettings("org.gnome.desktop.a11y.interface", "high-contrast").as_deref() != Some("true") {
        return 1;
    }
    if read_system_dark() { 2 } else { 3 }
}

/// High contrast is on: `Some(dark)`.
pub fn high_contrast() -> Option<bool> {
    let mut v = HIGH_CONTRAST.load(Ordering::Relaxed);
    if v == 0 {
        v = read_high_contrast();
        HIGH_CONTRAST.store(v, Ordering::Relaxed);
    }
    match v {
        2 => Some(true),
        3 => Some(false),
        _ => None,
    }
}

/// Reads the contrast setting again; true when it changed.
pub fn refresh_high_contrast() -> bool {
    let before = high_contrast();
    HIGH_CONTRAST.store(read_high_contrast(), Ordering::Relaxed);
    high_contrast() != before
}

// ---- clipboard ---------------------------------------------------------------

/// One clipboard for the whole process: on X11 the owner must stay alive
/// for other programs to paste what we copied.
fn clipboard<T>(f: impl FnOnce(&mut arboard::Clipboard) -> Option<T>) -> Option<T> {
    static CLIPBOARD: Mutex<Option<arboard::Clipboard>> = Mutex::new(None);
    let mut guard = CLIPBOARD.lock().unwrap_or_else(|e| e.into_inner());
    if guard.is_none() {
        *guard = arboard::Clipboard::new().ok();
    }
    f(guard.as_mut()?)
}

/// Changes when the clipboard's file list changes. X11 has no sequence
/// number, so this is a hash of the listed files (0: none).
pub fn clipboard_sequence() -> u32 {
    use std::hash::{Hash, Hasher};
    let files = clipboard_files();
    if files.is_empty() {
        return 0;
    }
    let mut h = std::collections::hash_map::DefaultHasher::new();
    files.hash(&mut h);
    h.finish() as u32
}

/// Files copied in the file manager (`text/uri-list`), if any.
pub fn clipboard_files() -> Vec<PathBuf> {
    clipboard(|c| c.get().file_list().ok()).unwrap_or_default()
}

/// Puts local files on the clipboard so they can be pasted in the file manager.
pub fn set_clipboard_files(paths: &[PathBuf]) -> bool {
    clipboard(|c| c.set().file_list(paths).ok()).is_some()
}

/// Text on the clipboard, if any.
pub fn clipboard_text() -> Option<String> {
    clipboard(|c| c.get_text().ok()).map(|t| t.replace("\r\n", "\n"))
}

/// Replaces the clipboard with text.
pub fn set_clipboard_text(text: &str) -> bool {
    clipboard(|c| c.set_text(text).ok()).is_some()
}

/// There is no portable system sound without extra libraries.
pub fn beep() {}

// ---- shell: browser, files ------------------------------------------------------

/// Starts a program without waiting; a thread reaps it so no zombie stays.
fn spawn_detached(program: &str, args: &[&str]) -> bool {
    let child = Command::new(program).args(args).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn();
    match child {
        Ok(mut child) => {
            std::thread::spawn(move || child.wait());
            true
        }
        Err(_) => false,
    }
}

/// The desktop file of the default browser, e.g. `firefox.desktop`.
fn browser_desktop_file() -> Option<String> {
    let out = Command::new("xdg-settings").args(["get", "default-web-browser"]).stderr(Stdio::null()).output().ok()?;
    let id = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (out.status.success() && id.ends_with(".desktop")).then_some(id)
}

/// Opens a web address or a `file://` URL in the default browser. Local
/// files go to the browser explicitly; `xdg-open` would open e.g. a .php
/// file in its editor.
pub fn open_url(url: &str) -> bool {
    if url.starts_with("file:")
        && let Some(browser) = browser_desktop_file()
        && spawn_detached("gtk-launch", &[&browser, url])
    {
        return true;
    }
    spawn_detached("xdg-open", &[url])
}

/// Shows `path` selected in the file manager (freedesktop FileManager1 over
/// D-Bus, as Nautilus, Nemo and Dolphin offer), else opens its folder.
pub fn reveal_in_file_manager(path: &Path) {
    let uri = format!("file://{}", crate::preview::percent_encode(&path.to_string_lossy()));
    let shown = Command::new("gdbus")
        .args(["call", "--session", "--dest", "org.freedesktop.FileManager1", "--object-path", "/org/freedesktop/FileManager1"])
        .args(["--method", "org.freedesktop.FileManager1.ShowItems", &format!("['{}']", uri.replace('\'', "%27")), ""])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success());
    if !shown && let Some(dir) = path.parent() {
        open_folder(dir);
    }
}

/// Opens a folder in the file manager.
pub fn open_folder(dir: &Path) {
    let _ = spawn_detached("xdg-open", &[&dir.to_string_lossy()]);
}

/// Runs a shell script in a new terminal window (the first terminal found).
pub fn open_terminal(script: &str, env: &std::collections::HashMap<String, String>, cwd: &Path) -> std::io::Result<()> {
    let candidates: [(&str, &[&str]); 6] = [
        ("gnome-terminal", &["--"]),
        ("kgx", &["--"]),
        ("konsole", &["-e"]),
        ("xfce4-terminal", &["-x"]),
        ("x-terminal-emulator", &["-e"]),
        ("xterm", &["-e"]),
    ];
    let (program, flag) = candidates
        .iter()
        .find(|(p, _)| which(p))
        .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, "no terminal emulator found"))?;
    let mut child = Command::new(program).args(*flag).args(["sh", "-c", script]).envs(env).current_dir(cwd).spawn()?;
    std::thread::spawn(move || child.wait());
    Ok(())
}

/// Quotes an argument for `sh`.
pub fn shell_quote(arg: &str) -> String {
    if !arg.is_empty() && arg.chars().all(|c| c.is_ascii_alphanumeric() || "-_./=:@,+%".contains(c)) {
        arg.to_string()
    } else {
        format!("'{}'", arg.replace('\'', "'\\''"))
    }
}

/// The button release that ended a drag out of the window went to GTK.
static LOST_RELEASE: AtomicBool = AtomicBool::new(false);

/// True once after `drag_files_out`: the app hands egui the release it missed.
pub fn take_lost_release() -> bool {
    LOST_RELEASE.swap(false, Ordering::Relaxed)
}

/// While the button is held, winit's connection has the pointer grabbed (an
/// implicit grab, through XInput2); GTK can only start its drag without it.
fn release_pointer_grab() {
    let display = X_DISPLAY.load(Ordering::Relaxed) as *mut x11_dl::xlib::Display;
    if display.is_null() {
        return;
    }
    let (Ok(xlib), Ok(xi)) = (x11_dl::xlib::Xlib::open(), x11_dl::xinput2::XInput2::open()) else { return };
    // SAFETY: winit's live display, used on the UI thread like winit itself.
    unsafe {
        (xlib.XUngrabPointer)(display, x11_dl::xlib::CurrentTime);
        let mut device = 0;
        if (xi.XIGetClientPointer)(display, 0, &mut device) != 0 {
            (xi.XIUngrabDevice)(display, device, x11_dl::xlib::CurrentTime);
        }
        (xlib.XFlush)(display);
    }
}

/// Files are fetched only when the drop target asks for them (on release),
/// so server files can be dragged out too.
pub const LAZY_DRAG_OUT: bool = true;

/// What `fetch` delivers when the drop target asks: the local paths to hand over.
pub type FetchFiles<'a> = &'a mut dyn FnMut() -> Result<Vec<PathBuf>, String>;

/// Starts an OS drag of files (XDND through GTK, offered as `text/uri-list`),
/// e.g. into the file manager. `fetch` runs when the target asks for the data,
/// i.e. on release (it may download first). Blocks until the drag ends; the
/// files are always copied. Ok(true) if a drop happened.
pub fn drag_files_out_with(fetch: FetchFiles) -> Result<bool, String> {
    use gtk::glib;
    use gtk::prelude::*;
    use std::cell::RefCell;
    use std::rc::Rc;
    if X_DISPLAY.load(Ordering::Relaxed) == 0 || gtk::init().is_err() {
        return Ok(false);
    }
    // SAFETY: GTK runs the handlers below only inside `main_loop.run()`, while
    // `fetch` is borrowed by this call; they are disconnected before returning.
    let fetch: &'static mut dyn FnMut() -> Result<Vec<PathBuf>, String> = unsafe { std::mem::transmute(fetch) };
    let fetch = Rc::new(RefCell::new(fetch));
    // None: not asked yet; Some(Ok(true)): delivered; Some(Err): fetching failed.
    let outcome: Rc<RefCell<Option<Result<bool, String>>>> = Rc::default();

    release_pointer_grab();
    LOST_RELEASE.store(true, Ordering::Relaxed);

    let source = gtk::Invisible::new();
    let main_loop = glib::MainLoop::new(None, false);
    let (f, o) = (fetch.clone(), outcome.clone());
    let data_get = source.connect_drag_data_get(move |_, _, data, _, _| {
        // Targets may ask more than once: fetch once, answer every time.
        let mut slot = o.borrow_mut();
        if slot.is_none() {
            let Ok(mut fetch) = f.try_borrow_mut() else { return };
            *slot = Some((fetch)().map(|paths| {
                let uris: Vec<String> = paths.iter().filter_map(|p| glib::filename_to_uri(p, None).ok().map(|u| u.to_string())).collect();
                PENDING_URIS.with(|u| *u.borrow_mut() = uris);
                true
            }));
        }
        if matches!(*slot, Some(Ok(true))) {
            PENDING_URIS.with(|u| {
                let uris = u.borrow();
                let list: Vec<&str> = uris.iter().map(String::as_str).collect();
                data.set_uris(&list);
            });
        }
    });
    let icon = source.connect_drag_begin(|_, context| context.drag_set_icon_name("text-x-generic", 0, 0));
    let ml = main_loop.clone();
    let end = source.connect_drag_end(move |_, _| ml.quit());
    let ml = main_loop.clone();
    let failed = source.connect_drag_failed(move |_, _, _| {
        ml.quit();
        glib::Propagation::Proceed
    });
    let targets = gtk::TargetList::new(&[gtk::TargetEntry::new("text/uri-list", gtk::TargetFlags::OTHER_APP, 0)]);
    let started = source.drag_begin_with_coordinates(&targets, gtk::gdk::DragAction::COPY, 1, None, -1, -1).is_some();
    if started {
        main_loop.run();
    }
    for id in [data_get, icon, end, failed] {
        source.disconnect(id);
    }
    PENDING_URIS.with(|u| u.borrow_mut().clear());
    let result = outcome.borrow_mut().take();
    result.unwrap_or(Ok(false))
}

thread_local! {
    /// The URIs of the running drag (fetched once, handed out per request).
    static PENDING_URIS: std::cell::RefCell<Vec<String>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// Calendar date and time in the local time zone.
pub struct LocalTime {
    pub year: u16,
    pub month: u16,
    pub day: u16,
    pub hour: u16,
    pub minute: u16,
}

fn local_tm(unix_secs: i64) -> Option<libc::tm> {
    let t = unix_secs as libc::time_t;
    // SAFETY: localtime_r only writes into the zeroed struct we pass.
    unsafe {
        let mut tm: libc::tm = std::mem::zeroed();
        (!libc::localtime_r(&t, &mut tm).is_null()).then_some(tm)
    }
}

/// Unix milliseconds as local time (time zone and DST from the system).
pub fn local_time(unix_ms: u64) -> LocalTime {
    match local_tm((unix_ms / 1000) as i64) {
        Some(tm) => LocalTime {
            year: (tm.tm_year + 1900) as u16,
            month: (tm.tm_mon + 1) as u16,
            day: tm.tm_mday as u16,
            hour: tm.tm_hour as u16,
            minute: tm.tm_min as u16,
        },
        None => LocalTime { year: 1970, month: 1, day: 1, hour: 0, minute: 0 },
    }
}

/// Minutes local time is ahead of UTC right now (summer time included).
pub fn utc_offset_minutes() -> i64 {
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs() as i64;
    local_tm(now).map_or(0, |tm| tm.tm_gmtoff as i64 / 60)
}

/// Console output as text; Linux tools write UTF-8.
pub fn console_text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

/// Replaces `target` with `replacement` in one step, keeping the permissions
/// of `target` (`replacement` is gone afterwards).
pub fn replace_file(target: &Path, replacement: &Path) -> std::io::Result<()> {
    if let Ok(meta) = std::fs::metadata(target) {
        let _ = std::fs::set_permissions(replacement, meta.permissions());
    }
    std::fs::rename(replacement, target)
}

/// Whether deleting here goes to the trash: local paths, not network mounts
/// (gvfs, which has no trash of its own).
pub fn has_recycle_bin(path: &Path) -> bool {
    path.is_absolute() && !path.to_string_lossy().contains("/gvfs/") && which("gio")
}

/// Moves a file or folder to the trash (without asking). False on failure.
pub fn move_to_recycle_bin(path: &Path) -> bool {
    Command::new("gio")
        .arg("trash")
        .arg("--")
        .arg(path)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

fn which(program: &str) -> bool {
    std::env::var_os("PATH").is_some_and(|p| std::env::split_paths(&p).any(|d| d.join(program).is_file()))
}

// ---- single instance ---------------------------------------------------------

/// Paths handed over by another start of Nebula, taken by the app each frame.
static OPEN_REQUESTS: Mutex<Vec<PathBuf>> = Mutex::new(Vec::new());

/// The socket the running instance listens on; per config folder, so
/// separate configurations (tests, portable copies) stay separate instances.
fn instance_socket() -> PathBuf {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    crate::settings::config_dir().to_string_lossy().hash(&mut h);
    let dir = std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from).unwrap_or_else(std::env::temp_dir);
    dir.join(format!("nebula-{:x}.sock", h.finish()))
}

fn start_instance_listener() {
    let path = instance_socket();
    let listener = match UnixListener::bind(&path) {
        Ok(l) => l,
        // Left over from a crash, or another instance (started with
        // --new-instance) owns it: only take it over when nobody answers.
        Err(_) if UnixStream::connect(&path).is_err() => {
            let _ = std::fs::remove_file(&path);
            match UnixListener::bind(&path) {
                Ok(l) => l,
                Err(_) => return,
            }
        }
        Err(_) => return,
    };
    std::thread::spawn(move || {
        for mut stream in listener.incoming().flatten() {
            let mut data = String::new();
            if stream.read_to_string(&mut data).is_err() {
                continue;
            }
            OPEN_REQUESTS
                .lock()
                .unwrap()
                .extend(data.split('\0').filter(|p| !p.is_empty()).map(PathBuf::from));
            let _ = stream.write_all(&[1]);
            if let Some(ctx) = CTX.get() {
                ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
                ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
                ctx.request_repaint();
            }
        }
    });
}

/// Paths another start of Nebula asked this window to open.
pub fn take_open_requests() -> Vec<PathBuf> {
    std::mem::take(&mut *OPEN_REQUESTS.lock().unwrap())
}

/// Hands `paths` to an already running Nebula (same config folder) and brings
/// it to the front. False if none runs.
pub fn forward_to_running(paths: &[PathBuf]) -> bool {
    let Ok(mut stream) = UnixStream::connect(instance_socket()) else { return false };
    let _ = stream.set_read_timeout(Some(std::time::Duration::from_secs(5)));
    let mut data = String::new();
    for p in paths {
        let abs = std::path::absolute(p).unwrap_or_else(|_| p.clone());
        data.push_str(&abs.to_string_lossy());
        data.push('\0');
    }
    if stream.write_all(data.as_bytes()).is_err() || stream.shutdown(std::net::Shutdown::Write).is_err() {
        return false;
    }
    let mut ack = [0u8; 1];
    stream.read_exact(&mut ack).is_ok() && ack[0] == 1
}

/// A plain information box (used without a window, e.g. after uninstalling).
pub fn message_box(title: &str, text: &str) {
    let _ = rfd::MessageDialog::new()
        .set_title(title)
        .set_description(text)
        .set_level(rfd::MessageLevel::Info)
        .set_buttons(rfd::MessageButtons::Ok)
        .show();
}

#[cfg(test)]
mod tests {
    #[test]
    fn local_time_is_plausible() {
        let t = super::local_time(1_700_000_000_000);
        assert_eq!(t.year, 2023);
        assert_eq!(t.month, 11);
    }

    #[test]
    fn recycle_bin_not_on_network_mounts() {
        assert!(!super::has_recycle_bin(std::path::Path::new("/run/user/1000/gvfs/smb-share:server=x/a.txt")));
        assert!(!super::has_recycle_bin(std::path::Path::new("relative.txt")));
    }
}
