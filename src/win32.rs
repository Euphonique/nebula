//! Windows integration for the frameless window: snap layouts on the
//! maximize button, Mica backdrop and the cursor position during OS drags.

use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicIsize, Ordering};

use eframe::egui::{self, Pos2, Rect};
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows_sys::Win32::Graphics::Dwm::{
    DWMSBT_MAINWINDOW, DWMSBT_NONE, DWMWA_SYSTEMBACKDROP_TYPE, DWMWA_USE_IMMERSIVE_DARK_MODE,
    DwmExtendFrameIntoClientArea, DwmSetWindowAttribute,
};
use windows_sys::Win32::Graphics::Gdi::ScreenToClient;
use windows_sys::Win32::UI::Controls::MARGINS;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{TME_LEAVE, TME_NONCLIENT, TRACKMOUSEEVENT, TrackMouseEvent};
use windows_sys::Win32::UI::Shell::{DefSubclassProc, SetWindowSubclass};
use windows_sys::Win32::UI::HiDpi::{GetDpiForWindow, GetSystemMetricsForDpi};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    GetCursorPos, HTMAXBUTTON, IsZoomed, NCCALCSIZE_PARAMS, SM_CXFRAME, SM_CXPADDEDBORDER,
    SM_CYFRAME, SW_MAXIMIZE, SW_RESTORE, SWP_FRAMECHANGED, SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER,
    GWL_STYLE, GetWindowLongPtrW, SetWindowLongPtrW, SetWindowPos, ShowWindow, WM_NCCALCSIZE,
    WM_NCHITTEST, WS_SYSMENU,
    WM_NCLBUTTONDBLCLK, WM_NCLBUTTONDOWN, WM_NCLBUTTONUP, WM_NCMOUSELEAVE, WM_NCMOUSEMOVE,
};

static WINDOW: AtomicIsize = AtomicIsize::new(0);
static CTX: OnceLock<egui::Context> = OnceLock::new();
/// Maximize button in client pixels: left, top, right, bottom.
static MAX_BUTTON: [AtomicI32; 4] = [const { AtomicI32::new(0) }; 4];
static MAX_HOVER: AtomicBool = AtomicBool::new(false);
static MAX_PRESSED: AtomicBool = AtomicBool::new(false);

fn hwnd() -> Option<HWND> {
    let h = WINDOW.load(Ordering::Relaxed);
    (h != 0).then_some(h as HWND)
}

/// Grabs the window handle and installs the message hook. Safe to call every frame.
pub fn init(frame: &eframe::Frame, ctx: &egui::Context) {
    if hwnd().is_some() {
        return;
    }
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    let Ok(handle) = frame.window_handle() else { return };
    let RawWindowHandle::Win32(h) = handle.as_raw() else { return };
    let hwnd = h.hwnd.get() as HWND;
    WINDOW.store(hwnd as isize, Ordering::Relaxed);
    let _ = CTX.set(ctx.clone());
    // Lets a second start find this window and hand over its files.
    let prop = instance_prop();
    // SAFETY: valid window; the property is a plain marker value.
    unsafe { windows_sys::Win32::UI::WindowsAndMessaging::SetPropW(hwnd, prop.as_ptr(), 1 as _) };
    // SAFETY: the window outlives the subclass; the proc only reads atomics.
    unsafe {
        // The window was created with a frame around the requested size; once the
        // frame is gone the whole window is client area, so shrink it back or it
        // would grow by the frame on every start (the size is restored from disk).
        let mut client: windows_sys::Win32::Foundation::RECT = std::mem::zeroed();
        windows_sys::Win32::UI::WindowsAndMessaging::GetClientRect(hwnd, &mut client);
        let zoomed = IsZoomed(hwnd) != 0;
        SetWindowSubclass(hwnd, Some(subclass_proc), 1, 0);
        // Without a system menu DWM does not paint its own caption buttons
        // into the extended frame behind ours.
        let style = GetWindowLongPtrW(hwnd, GWL_STYLE);
        SetWindowLongPtrW(hwnd, GWL_STYLE, style & !(WS_SYSMENU as isize));
        // Re-run WM_NCCALCSIZE so the hidden caption takes effect.
        SetWindowPos(hwnd, std::ptr::null_mut(), 0, 0, 0, 0, SWP_FRAMECHANGED | SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER);
        if !zoomed && client.right > 0 && client.bottom > 0 {
            SetWindowPos(hwnd, std::ptr::null_mut(), 0, 0, client.right, client.bottom, SWP_NOMOVE | SWP_NOZORDER);
        }
    }
}

/// Tells the hook where the maximize button is (in points).
pub fn set_maximize_button(rect: Rect, pixels_per_point: f32) {
    let v = [rect.left(), rect.top(), rect.right(), rect.bottom()];
    for (slot, x) in MAX_BUTTON.iter().zip(v) {
        slot.store((x * pixels_per_point).round() as i32, Ordering::Relaxed);
    }
}

pub fn maximize_hovered() -> bool {
    MAX_HOVER.load(Ordering::Relaxed)
}

fn in_max_button(x: i32, y: i32) -> bool {
    let [l, t, r, b] = [0, 1, 2, 3].map(|i| MAX_BUTTON[i].load(Ordering::Relaxed));
    r > l && x >= l && x < r && y >= t && y < b
}

fn repaint() {
    if let Some(ctx) = CTX.get() {
        ctx.request_repaint();
    }
}

unsafe extern "system" fn subclass_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    _id: usize,
    _data: usize,
) -> LRESULT {
    match msg {
        // The window keeps its native frame (shadow, snapping, Mica) but the
        // whole area is client area; we draw our own title bar.
        WM_NCCALCSIZE if wparam != 0 => {
            // SAFETY: for wparam != 0, lparam points to NCCALCSIZE_PARAMS.
            unsafe {
                if IsZoomed(hwnd) != 0 {
                    // Maximized windows overhang the screen by the frame size.
                    let dpi = GetDpiForWindow(hwnd);
                    let pad = GetSystemMetricsForDpi(SM_CXPADDEDBORDER, dpi);
                    let fx = GetSystemMetricsForDpi(SM_CXFRAME, dpi) + pad;
                    let fy = GetSystemMetricsForDpi(SM_CYFRAME, dpi) + pad;
                    let params = &mut *(lparam as *mut NCCALCSIZE_PARAMS);
                    let r = &mut params.rgrc[0];
                    r.left += fx;
                    r.right -= fx;
                    r.top += fy;
                    r.bottom -= fy;
                }
            }
            return 0;
        }
        // Another Nebula started with files: open them here (see `forward_to_running`).
        windows_sys::Win32::UI::WindowsAndMessaging::WM_COPYDATA => {
            // SAFETY: for WM_COPYDATA, lparam points to a COPYDATASTRUCT valid during the call.
            let cds = unsafe { &*(lparam as *const windows_sys::Win32::System::DataExchange::COPYDATASTRUCT) };
            if cds.dwData == COPYDATA_OPEN {
                let len = cds.cbData as usize / 2;
                // SAFETY: the sender passes `cbData` bytes of UTF-16.
                let data = unsafe { std::slice::from_raw_parts(cds.lpData as *const u16, len) };
                let paths = String::from_utf16_lossy(data);
                OPEN_REQUESTS
                    .lock()
                    .unwrap()
                    .extend(paths.split('\0').filter(|p| !p.is_empty()).map(std::path::PathBuf::from));
                // SAFETY: plain window calls on our own window.
                unsafe {
                    if windows_sys::Win32::UI::WindowsAndMessaging::IsIconic(hwnd) != 0 {
                        ShowWindow(hwnd, SW_RESTORE);
                    }
                    windows_sys::Win32::UI::WindowsAndMessaging::SetForegroundWindow(hwnd);
                }
                repaint();
                return 1;
            }
        }
        WM_NCHITTEST => {
            let mut p = POINT {
                x: (lparam & 0xFFFF) as i16 as i32,
                y: ((lparam >> 16) & 0xFFFF) as i16 as i32,
            };
            // SAFETY: valid window and point.
            unsafe { ScreenToClient(hwnd, &mut p) };
            let inside = in_max_button(p.x, p.y);
            if MAX_HOVER.swap(inside, Ordering::Relaxed) != inside {
                repaint();
            }
            if inside {
                // Lets Windows 11 show the snap layout flyout.
                return HTMAXBUTTON as LRESULT;
            }
            if MAX_PRESSED.swap(false, Ordering::Relaxed) {
                repaint();
            }
        }
        WM_NCMOUSEMOVE if wparam == HTMAXBUTTON as usize => {
            let mut track = TRACKMOUSEEVENT {
                cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
                dwFlags: TME_LEAVE | TME_NONCLIENT,
                hwndTrack: hwnd,
                dwHoverTime: 0,
            };
            // SAFETY: valid struct.
            unsafe { TrackMouseEvent(&mut track) };
            return 0;
        }
        WM_NCMOUSELEAVE => {
            MAX_HOVER.store(false, Ordering::Relaxed);
            MAX_PRESSED.store(false, Ordering::Relaxed);
            repaint();
        }
        WM_NCLBUTTONDOWN | WM_NCLBUTTONDBLCLK if wparam == HTMAXBUTTON as usize => {
            MAX_PRESSED.store(true, Ordering::Relaxed);
            repaint();
            return 0;
        }
        WM_NCLBUTTONUP if wparam == HTMAXBUTTON as usize => {
            if MAX_PRESSED.swap(false, Ordering::Relaxed) {
                // SAFETY: valid window.
                unsafe {
                    ShowWindow(hwnd, if IsZoomed(hwnd) != 0 { SW_RESTORE } else { SW_MAXIMIZE });
                }
            }
            repaint();
            return 0;
        }
        _ => {}
    }
    // SAFETY: forwarding to the next handler in the chain.
    unsafe { DefSubclassProc(hwnd, msg, wparam, lparam) }
}

/// Cursor position relative to the window, in points. Works during OS
/// drag-and-drop, when the window receives no mouse events.
pub fn cursor_pos(pixels_per_point: f32) -> Option<Pos2> {
    let hwnd = hwnd()?;
    let mut p = POINT { x: 0, y: 0 };
    // SAFETY: valid pointer and window.
    unsafe {
        if GetCursorPos(&mut p) == 0 || ScreenToClient(hwnd, &mut p) == 0 {
            return None;
        }
    }
    Some(Pos2::new(p.x as f32 / pixels_per_point, p.y as f32 / pixels_per_point))
}

/// Applies the dark/light title of the window and optionally the Mica backdrop.
pub fn set_backdrop(dark: bool, mica: bool) {
    let Some(hwnd) = hwnd() else { return };
    let dark_flag: i32 = dark.into();
    let backdrop = if mica { DWMSBT_MAINWINDOW } else { DWMSBT_NONE };
    let margins = if mica {
        MARGINS { cxLeftWidth: -1, cxRightWidth: -1, cyTopHeight: -1, cyBottomHeight: -1 }
    } else {
        MARGINS { cxLeftWidth: 0, cxRightWidth: 0, cyTopHeight: 0, cyBottomHeight: 0 }
    };
    // SAFETY: pointers to locals with the documented sizes.
    unsafe {
        DwmSetWindowAttribute(
            hwnd,
            DWMWA_USE_IMMERSIVE_DARK_MODE as u32,
            &dark_flag as *const i32 as *const _,
            4,
        );
        DwmSetWindowAttribute(
            hwnd,
            DWMWA_SYSTEMBACKDROP_TYPE as u32,
            &backdrop as *const i32 as *const _,
            4,
        );
        DwmExtendFrameIntoClientArea(hwnd, &margins);
    }
}

/// Windows app mode, cached: 0 = not read yet, 1 = light, 2 = dark.
static SYSTEM_DARK: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(0);

fn read_system_dark() -> bool {
    winreg::RegKey::predef(winreg::enums::HKEY_CURRENT_USER)
        .open_subkey(r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize")
        .and_then(|k| k.get_value::<u32, _>("AppsUseLightTheme"))
        .map_or(true, |light| light == 0)
}

/// Whether Windows uses dark mode for apps.
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

/// Reads the Windows app mode again; true when it changed.
pub fn refresh_system_dark() -> bool {
    let before = system_dark();
    let dark = read_system_dark();
    SYSTEM_DARK.store(if dark { 2 } else { 1 }, Ordering::Relaxed);
    dark != before
}

/// Windows contrast theme, cached: 0 = not read yet, 1 = off, 2 = on (dark), 3 = on (light).
static HIGH_CONTRAST: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(0);

fn read_high_contrast() -> u8 {
    use windows_sys::Win32::Graphics::Gdi::{COLOR_WINDOW, GetSysColor};
    use windows_sys::Win32::UI::Accessibility::{HCF_HIGHCONTRASTON, HIGHCONTRASTW};
    use windows_sys::Win32::UI::WindowsAndMessaging::{SPI_GETHIGHCONTRAST, SystemParametersInfoW};
    let mut hc = HIGHCONTRASTW { cbSize: std::mem::size_of::<HIGHCONTRASTW>() as u32, ..Default::default() };
    // SAFETY: the structure is sized and outlives the call.
    let ok = unsafe { SystemParametersInfoW(SPI_GETHIGHCONTRAST, hc.cbSize, (&mut hc as *mut HIGHCONTRASTW).cast(), 0) } != 0;
    if !ok || hc.dwFlags & HCF_HIGHCONTRASTON == 0 {
        return 1;
    }
    // Dark or light: the brightness of the window background of the contrast theme.
    // SAFETY: plain value call.
    let c = unsafe { GetSysColor(COLOR_WINDOW) };
    let (r, g, b) = (c & 0xFF, (c >> 8) & 0xFF, (c >> 16) & 0xFF);
    if (r * 299 + g * 587 + b * 114) / 1000 < 128 { 2 } else { 3 }
}

/// A Windows contrast theme is on: `Some(dark)`.
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

/// Reads the contrast theme again; true when it changed.
pub fn refresh_high_contrast() -> bool {
    let before = high_contrast();
    HIGH_CONTRAST.store(read_high_contrast(), Ordering::Relaxed);
    high_contrast() != before
}

// ---- file clipboard (interop with Explorer) ---------------------------------

use windows_sys::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, GetClipboardData, GetClipboardSequenceNumber,
    IsClipboardFormatAvailable, OpenClipboard, SetClipboardData,
};
use windows_sys::Win32::System::Memory::{GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalUnlock};
use windows_sys::Win32::System::Ole::CF_HDROP;
use windows_sys::Win32::UI::Shell::{DROPFILES, DragQueryFileW};

/// Changes every time any application writes to the clipboard.
pub fn clipboard_sequence() -> u32 {
    // SAFETY: no arguments.
    unsafe { GetClipboardSequenceNumber() }
}

/// Files copied in Explorer (CF_HDROP), if any.
pub fn clipboard_files() -> Vec<std::path::PathBuf> {
    let mut out = Vec::new();
    // SAFETY: clipboard is opened and closed in this block; the HDROP is only
    // read while open.
    unsafe {
        if IsClipboardFormatAvailable(CF_HDROP as u32) == 0 || OpenClipboard(std::ptr::null_mut()) == 0 {
            return out;
        }
        let hdrop = GetClipboardData(CF_HDROP as u32);
        if !hdrop.is_null() {
            let count = DragQueryFileW(hdrop, u32::MAX, std::ptr::null_mut(), 0);
            for i in 0..count {
                let len = DragQueryFileW(hdrop, i, std::ptr::null_mut(), 0) as usize;
                let mut buf = vec![0u16; len + 1];
                DragQueryFileW(hdrop, i, buf.as_mut_ptr(), buf.len() as u32);
                use std::os::windows::ffi::OsStringExt;
                out.push(std::path::PathBuf::from(std::ffi::OsString::from_wide(&buf[..len])));
            }
        }
        CloseClipboard();
    }
    out
}

/// Puts local files on the clipboard so they can be pasted in Explorer.
pub fn set_clipboard_files(paths: &[std::path::PathBuf]) -> bool {
    use std::os::windows::ffi::OsStrExt;
    let mut wide: Vec<u16> = Vec::new();
    for p in paths {
        wide.extend(p.as_os_str().encode_wide());
        wide.push(0);
    }
    wide.push(0);
    let header = std::mem::size_of::<DROPFILES>();
    let bytes = header + wide.len() * 2;
    // SAFETY: the global block is sized for header + list and ownership passes
    // to the clipboard on success.
    unsafe {
        let mem = GlobalAlloc(GMEM_MOVEABLE, bytes);
        if mem.is_null() {
            return false;
        }
        let ptr = GlobalLock(mem) as *mut u8;
        let df = ptr as *mut DROPFILES;
        (*df).pFiles = header as u32;
        (*df).pt = POINT { x: 0, y: 0 };
        (*df).fNC = 0;
        (*df).fWide = 1;
        std::ptr::copy_nonoverlapping(wide.as_ptr() as *const u8, ptr.add(header), wide.len() * 2);
        GlobalUnlock(mem);
        if OpenClipboard(hwnd().unwrap_or(std::ptr::null_mut())) == 0 {
            return false;
        }
        EmptyClipboard();
        let ok = !SetClipboardData(CF_HDROP as u32, mem).is_null();
        CloseClipboard();
        ok
    }
}

const CF_UNICODETEXT: u32 = 13;

/// Text on the clipboard, if any.
pub fn clipboard_text() -> Option<String> {
    // SAFETY: the clipboard is opened and closed here; the handle is only
    // read while locked.
    unsafe {
        if IsClipboardFormatAvailable(CF_UNICODETEXT) == 0 || OpenClipboard(std::ptr::null_mut()) == 0 {
            return None;
        }
        let h = GetClipboardData(CF_UNICODETEXT);
        let mut out = None;
        if !h.is_null() {
            let p = GlobalLock(h) as *const u16;
            if !p.is_null() {
                let mut len = 0;
                while *p.add(len) != 0 {
                    len += 1;
                }
                out = Some(String::from_utf16_lossy(std::slice::from_raw_parts(p, len)).replace("\r\n", "\n"));
                GlobalUnlock(h);
            }
        }
        CloseClipboard();
        out
    }
}

/// Replaces the clipboard with text.
pub fn set_clipboard_text(text: &str) -> bool {
    let wide: Vec<u16> = text.replace('\n', "\r\n").encode_utf16().chain(std::iter::once(0)).collect();
    // SAFETY: the block is sized for the text and handed to the clipboard.
    unsafe {
        let mem = GlobalAlloc(GMEM_MOVEABLE, wide.len() * 2);
        if mem.is_null() {
            return false;
        }
        let ptr = GlobalLock(mem) as *mut u16;
        std::ptr::copy_nonoverlapping(wide.as_ptr(), ptr, wide.len());
        GlobalUnlock(mem);
        if OpenClipboard(std::ptr::null_mut()) == 0 {
            return false;
        }
        EmptyClipboard();
        let ok = !SetClipboardData(CF_UNICODETEXT, mem).is_null();
        CloseClipboard();
        ok
    }
}

pub fn beep() {
    // SAFETY: plays the default system sound.
    unsafe {
        windows_sys::Win32::System::Diagnostics::Debug::MessageBeep(0);
    }
}

// ---- shell: browser, dragging files out ---------------------------------------

use windows_sys::Win32::UI::Shell::{
    ASSOCF_NONE, ASSOCSTR_EXECUTABLE, AssocQueryStringW, ILCreateFromPathW, ILFindLastID, ILFree, SHCreateDataObject,
    SHDoDragDrop, ShellExecuteW,
};

pub fn window_handle() -> Option<HWND> {
    hwnd()
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

fn shell_execute(file: &str, params: Option<&str>) -> bool {
    let file = wide(file);
    let params = params.map(wide);
    let open = wide("open");
    // SAFETY: null-terminated strings that outlive the call.
    let result = unsafe {
        ShellExecuteW(
            hwnd().unwrap_or(std::ptr::null_mut()),
            open.as_ptr(),
            file.as_ptr(),
            params.as_ref().map_or(std::ptr::null(), |p| p.as_ptr()),
            std::ptr::null(),
            windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL,
        )
    };
    result as isize > 32
}

/// The program that opens web links (the default browser).
fn browser_exe() -> Option<String> {
    let assoc = wide("http");
    let verb = wide("open");
    let mut buf = vec![0u16; 1024];
    let mut len = buf.len() as u32;
    // SAFETY: the buffer length is passed along.
    let hr = unsafe { AssocQueryStringW(ASSOCF_NONE, ASSOCSTR_EXECUTABLE, assoc.as_ptr(), verb.as_ptr(), buf.as_mut_ptr(), &mut len) };
    if hr != 0 {
        return None;
    }
    let exe = String::from_utf16_lossy(&buf[..(len as usize).saturating_sub(1).min(buf.len())]);
    (!exe.is_empty() && std::path::Path::new(&exe).is_file()).then_some(exe)
}

/// What a double-click on a `.ext` file starts, as Windows resolves it
/// (the user's choice, else the registered default).
pub struct FileTypeHandler {
    /// "Visual Studio Code"
    pub name: String,
    /// Path of the program; `OpenWith.exe` when Windows would ask.
    pub exe: String,
    pub prog_id: String,
}

pub fn file_type_handler(ext: &str) -> Option<FileTypeHandler> {
    use windows_sys::Win32::UI::Shell::{ASSOCF_INIT_IGNOREUNKNOWN, ASSOCSTR, ASSOCSTR_FRIENDLYAPPNAME, ASSOCSTR_PROGID};
    let assoc = wide(&format!(".{ext}"));
    let query = |what: ASSOCSTR| -> Option<String> {
        let mut buf = vec![0u16; 1024];
        let mut len = buf.len() as u32;
        // SAFETY: the buffer length is passed along; no verb (the default one).
        let hr = unsafe { AssocQueryStringW(ASSOCF_INIT_IGNOREUNKNOWN, what, assoc.as_ptr(), std::ptr::null(), buf.as_mut_ptr(), &mut len) };
        (hr == 0).then(|| String::from_utf16_lossy(&buf[..(len as usize).saturating_sub(1).min(buf.len())]))
    };
    let exe = query(ASSOCSTR_EXECUTABLE)?;
    Some(FileTypeHandler {
        name: query(ASSOCSTR_FRIENDLYAPPNAME).unwrap_or_default(),
        prog_id: query(ASSOCSTR_PROGID).unwrap_or_default(),
        exe,
    })
}

/// Opens a web address or a `file://` URL in the default browser. Local
/// files go to the browser explicitly; handing them to the shell would open
/// e.g. a .php file in its editor.
pub fn open_url(url: &str) -> bool {
    if url.starts_with("file:")
        && let Some(exe) = browser_exe()
        && shell_execute(&exe, Some(&format!("\"{url}\"")))
    {
        return true;
    }
    shell_execute(url, None)
}

/// Releases a COM object.
///
/// # Safety
/// `obj` must be a valid COM interface pointer.
unsafe fn com_release(obj: *mut std::ffi::c_void) {
    // SAFETY: IUnknown::Release is the third vtable entry.
    unsafe {
        let vtbl = *(obj as *const *const usize);
        let release: unsafe extern "system" fn(*mut std::ffi::c_void) -> u32 = std::mem::transmute(*vtbl.add(2));
        release(obj);
    }
}

/// Starts an OS drag of local files, e.g. into Explorer. Blocks until the
/// files are dropped (they are always copied). Returns true if a drop happened.
pub fn drag_files_out(paths: &[std::path::PathBuf]) -> bool {
    use std::os::windows::ffi::OsStrExt;
    const IID_IDATAOBJECT: windows_sys::core::GUID =
        windows_sys::core::GUID::from_u128(0x0000010e_0000_0000_c000_000000000046);
    const DROPEFFECT_COPY: u32 = 1;
    let Some(hwnd) = hwnd() else { return false };
    // SAFETY: the item ids live until after the data object took copies of
    // them; the data object is released once the drag ended.
    unsafe {
        let id_of = |p: &std::path::Path| {
            let w: Vec<u16> = p.as_os_str().encode_wide().chain(std::iter::once(0)).collect();
            ILCreateFromPathW(w.as_ptr())
        };
        // Explorer wants the items relative to their folder; a drag carries
        // the items of one folder (the first item's).
        let Some(parent) = paths.first().and_then(|p| p.parent()) else { return false };
        let folder = id_of(parent);
        if folder.is_null() {
            return false;
        }
        let pidls: Vec<*mut windows_sys::Win32::UI::Shell::Common::ITEMIDLIST> =
            paths.iter().filter(|p| p.parent() == Some(parent)).map(|p| id_of(p)).filter(|p| !p.is_null()).collect();
        let children: Vec<*const windows_sys::Win32::UI::Shell::Common::ITEMIDLIST> =
            pidls.iter().map(|p| ILFindLastID(*p) as *const _).collect();
        let mut data: *mut std::ffi::c_void = std::ptr::null_mut();
        let hr = if children.is_empty() {
            -1
        } else {
            SHCreateDataObject(folder, children.len() as u32, children.as_ptr(), std::ptr::null_mut(), &IID_IDATAOBJECT, &mut data)
        };
        for p in &pidls {
            ILFree(*p);
        }
        ILFree(folder);
        if hr != 0 || data.is_null() {
            return false;
        }
        let mut effect = 0u32;
        // Without a drop source the shell supplies its default one.
        let hr = SHDoDragDrop(hwnd, data, std::ptr::null_mut(), DROPEFFECT_COPY, &mut effect);
        com_release(data);
        // The drag loop swallowed the button release; let egui see it.
        let mut p = POINT { x: 0, y: 0 };
        GetCursorPos(&mut p);
        ScreenToClient(hwnd, &mut p);
        let lparam = ((p.y as u16 as u32) << 16 | p.x as u16 as u32) as LPARAM;
        windows_sys::Win32::UI::WindowsAndMessaging::PostMessageW(
            hwnd,
            windows_sys::Win32::UI::WindowsAndMessaging::WM_LBUTTONUP,
            0,
            lparam,
        );
        hr == 0x0004_0100 /* DRAGDROP_S_DROP */ && effect != 0
    }
}

/// Calendar date and time in the local time zone.
pub struct LocalTime {
    pub year: u16,
    pub month: u16,
    pub day: u16,
    pub hour: u16,
    pub minute: u16,
}

/// Unix milliseconds as local time (Windows rules for the time zone and DST).
pub fn local_time(unix_ms: u64) -> LocalTime {
    use windows_sys::Win32::Foundation::{FILETIME, SYSTEMTIME};
    use windows_sys::Win32::System::Time::{FileTimeToSystemTime, SystemTimeToTzSpecificLocalTime};
    // FILETIME counts 100 ns steps since 1601.
    let ticks = unix_ms * 10_000 + 116_444_736_000_000_000;
    let ft = FILETIME { dwLowDateTime: ticks as u32, dwHighDateTime: (ticks >> 32) as u32 };
    // SAFETY: plain value conversions into stack structs.
    let (mut utc, mut local): (SYSTEMTIME, SYSTEMTIME) = unsafe { (std::mem::zeroed(), std::mem::zeroed()) };
    let ok = unsafe { FileTimeToSystemTime(&ft, &mut utc) != 0 && SystemTimeToTzSpecificLocalTime(std::ptr::null(), &utc, &mut local) != 0 };
    let t = if ok { local } else { utc };
    LocalTime { year: t.wYear, month: t.wMonth, day: t.wDay, hour: t.wHour, minute: t.wMinute }
}

/// Minutes local time is ahead of UTC right now (summer time included).
pub fn utc_offset_minutes() -> i64 {
    use windows_sys::Win32::Foundation::FILETIME;
    use windows_sys::Win32::Storage::FileSystem::FileTimeToLocalFileTime;
    // FILETIME counts 100 ns steps since 1601.
    let utc = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs() as i64 + 11_644_473_600;
    let ticks = (utc * 10_000_000) as u64;
    let ft = FILETIME { dwLowDateTime: ticks as u32, dwHighDateTime: (ticks >> 32) as u32 };
    let mut local = FILETIME { dwLowDateTime: 0, dwHighDateTime: 0 };
    // SAFETY: both pointers refer to valid FILETIME values on the stack.
    if unsafe { FileTimeToLocalFileTime(&ft, &mut local) } == 0 {
        return 0;
    }
    let local_ticks = ((local.dwHighDateTime as u64) << 32 | local.dwLowDateTime as u64) as i64;
    (local_ticks - ticks as i64) / 600_000_000
}

/// Console output as text: UTF-8 if it is valid (git, node …), else in the
/// OEM code page that cmd and older tools write (umlauts as in "dir").
pub fn console_text(bytes: &[u8]) -> String {
    if let Ok(s) = std::str::from_utf8(bytes) {
        return s.to_string();
    }
    use windows_sys::Win32::Globalization::{CP_OEMCP, MultiByteToWideChar};
    let Ok(len) = i32::try_from(bytes.len()) else { return String::from_utf8_lossy(bytes).into_owned() };
    // SAFETY: the first call only measures; the buffer then has exactly that size.
    unsafe {
        let n = MultiByteToWideChar(CP_OEMCP, 0, bytes.as_ptr(), len, std::ptr::null_mut(), 0);
        if n <= 0 {
            return String::from_utf8_lossy(bytes).into_owned();
        }
        let mut wide = vec![0u16; n as usize];
        MultiByteToWideChar(CP_OEMCP, 0, bytes.as_ptr(), len, wide.as_mut_ptr(), n);
        String::from_utf16_lossy(&wide)
    }
}

/// Replaces `target` with `replacement` in one step, keeping the attributes
/// and permissions of `target` (`replacement` is gone afterwards).
pub fn replace_file(target: &std::path::Path, replacement: &std::path::Path) -> std::io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{REPLACEFILE_IGNORE_MERGE_ERRORS, ReplaceFileW};
    let wide = |p: &std::path::Path| -> Vec<u16> { p.as_os_str().encode_wide().chain([0]).collect() };
    let (t, r) = (wide(target), wide(replacement));
    // SAFETY: both paths are zero-terminated wide strings that outlive the call.
    let ok = unsafe {
        ReplaceFileW(t.as_ptr(), r.as_ptr(), std::ptr::null(), REPLACEFILE_IGNORE_MERGE_ERRORS, std::ptr::null(), std::ptr::null())
    };
    if ok != 0 { Ok(()) } else { Err(std::io::Error::last_os_error()) }
}

/// Whether deleting on this drive goes to the recycle bin: only local hard
/// disks have one (not network drives, UNC paths or USB sticks).
pub fn has_recycle_bin(path: &std::path::Path) -> bool {
    use std::path::Component;
    use windows_sys::Win32::Storage::FileSystem::GetDriveTypeW;
    const DRIVE_FIXED: u32 = 3;
    let Some(Component::Prefix(prefix)) = path.components().next() else { return false };
    if !matches!(prefix.kind(), std::path::Prefix::Disk(_) | std::path::Prefix::VerbatimDisk(_)) {
        return false;
    }
    let root: Vec<u16> = format!("{}\\", prefix.as_os_str().to_string_lossy().trim_start_matches(r"\\?\")).encode_utf16().chain([0]).collect();
    // SAFETY: `root` is a zero-terminated wide string like "C:\".
    unsafe { GetDriveTypeW(root.as_ptr()) == DRIVE_FIXED }
}

/// Moves a file or folder to the recycle bin (without asking). False on failure.
pub fn move_to_recycle_bin(path: &std::path::Path) -> bool {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::UI::Shell::{FO_DELETE, FOF_ALLOWUNDO, FOF_NOCONFIRMATION, FOF_NOERRORUI, FOF_SILENT, SHFILEOPSTRUCTW, SHFileOperationW};
    // The list of paths ends with two zeros.
    let mut from: Vec<u16> = path.as_os_str().encode_wide().collect();
    from.extend([0, 0]);
    let mut op = SHFILEOPSTRUCTW {
        hwnd: std::ptr::null_mut(),
        wFunc: FO_DELETE,
        pFrom: from.as_ptr(),
        pTo: std::ptr::null(),
        fFlags: (FOF_ALLOWUNDO | FOF_NOCONFIRMATION | FOF_NOERRORUI | FOF_SILENT) as u16,
        fAnyOperationsAborted: 0,
        hNameMappings: std::ptr::null_mut(),
        lpszProgressTitle: std::ptr::null(),
    };
    // SAFETY: `from` is a double-zero-terminated wide string that outlives the call.
    unsafe { SHFileOperationW(&mut op) == 0 && op.fAnyOperationsAborted == 0 }
}

// ---- single instance ---------------------------------------------------------

/// `WM_COPYDATA` tag for "open these paths".
const COPYDATA_OPEN: usize = 0x4E42_4F50;
/// Paths handed over by another start of Nebula, taken by the app each frame.
static OPEN_REQUESTS: std::sync::Mutex<Vec<std::path::PathBuf>> = std::sync::Mutex::new(Vec::new());

/// Window property marking the main window; per config folder, so separate
/// configurations (tests, portable copies) stay separate instances.
fn instance_prop() -> Vec<u16> {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    crate::settings::config_dir().to_string_lossy().to_lowercase().hash(&mut h);
    format!("NebulaMain-{:x}", h.finish()).encode_utf16().chain(std::iter::once(0)).collect()
}

/// Paths another start of Nebula asked this window to open.
pub fn take_open_requests() -> Vec<std::path::PathBuf> {
    std::mem::take(&mut *OPEN_REQUESTS.lock().unwrap())
}

/// Hands `paths` to an already running Nebula (same config folder) and brings
/// it to the front. False if none runs.
pub fn forward_to_running(paths: &[std::path::PathBuf]) -> bool {
    use windows_sys::core::BOOL;
    use windows_sys::Win32::System::DataExchange::COPYDATASTRUCT;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        ASFW_ANY, AllowSetForegroundWindow, EnumWindows, GetPropW, SMTO_ABORTIFHUNG, SendMessageTimeoutW, WM_COPYDATA,
    };
    struct Search {
        prop: Vec<u16>,
        found: HWND,
    }
    unsafe extern "system" fn each(hwnd: HWND, lparam: LPARAM) -> BOOL {
        // SAFETY: lparam is the `Search` passed below, alive during EnumWindows.
        let s = unsafe { &mut *(lparam as *mut Search) };
        // SAFETY: plain property lookup on a top-level window.
        if !unsafe { GetPropW(hwnd, s.prop.as_ptr()) }.is_null() {
            s.found = hwnd;
            return 0;
        }
        1
    }
    let mut search = Search { prop: instance_prop(), found: std::ptr::null_mut() };
    // SAFETY: the callback only touches `search`.
    unsafe { EnumWindows(Some(each), &mut search as *mut Search as LPARAM) };
    if search.found.is_null() {
        return false;
    }
    let mut data: Vec<u16> = Vec::new();
    for p in paths {
        let abs = std::path::absolute(p).unwrap_or_else(|_| p.clone());
        data.extend(abs.to_string_lossy().encode_utf16());
        data.push(0);
    }
    let cds = COPYDATASTRUCT { dwData: COPYDATA_OPEN, cbData: (data.len() * 2) as u32, lpData: data.as_ptr() as *mut _ };
    let mut result = 0usize;
    // SAFETY: `cds` and `data` live until SendMessageTimeoutW returns.
    unsafe {
        // The running window may take the focus.
        AllowSetForegroundWindow(ASFW_ANY);
        SendMessageTimeoutW(search.found, WM_COPYDATA, 0, &cds as *const _ as LPARAM, SMTO_ABORTIFHUNG, 5000, &mut result);
    }
    result == 1
}

// ---- shell integration -------------------------------------------------------

/// Creates a shortcut (`.lnk`) to `target` (e.g. in the start menu).
pub fn create_shortcut(link: &std::path::Path, target: &std::path::Path, description: &str) -> bool {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::System::Com::{CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx};
    const CLSID_SHELL_LINK: windows_sys::core::GUID = windows_sys::core::GUID::from_u128(0x00021401_0000_0000_c000_000000000046);
    const IID_ISHELL_LINK_W: windows_sys::core::GUID = windows_sys::core::GUID::from_u128(0x000214f9_0000_0000_c000_000000000046);
    const IID_IPERSIST_FILE: windows_sys::core::GUID = windows_sys::core::GUID::from_u128(0x0000010b_0000_0000_c000_000000000046);
    let wide = |s: &std::ffi::OsStr| -> Vec<u16> { s.encode_wide().chain(std::iter::once(0)).collect() };
    let target_w = wide(target.as_os_str());
    let dir_w = wide(target.parent().unwrap_or(target).as_os_str());
    let link_w = wide(link.as_os_str());
    let desc_w = wide(std::ffi::OsStr::new(description));
    type Fn1 = unsafe extern "system" fn(*mut std::ffi::c_void, *const u16) -> i32;
    type Qi = unsafe extern "system" fn(*mut std::ffi::c_void, *const windows_sys::core::GUID, *mut *mut std::ffi::c_void) -> i32;
    type Save = unsafe extern "system" fn(*mut std::ffi::c_void, *const u16, i32) -> i32;
    // SAFETY: COM calls through the documented vtable layouts of IShellLinkW
    // and IPersistFile; every pointer is released before returning.
    unsafe {
        CoInitializeEx(std::ptr::null(), COINIT_APARTMENTTHREADED as _);
        let mut link_obj: *mut std::ffi::c_void = std::ptr::null_mut();
        if CoCreateInstance(&CLSID_SHELL_LINK, std::ptr::null_mut(), CLSCTX_INPROC_SERVER, &IID_ISHELL_LINK_W, &mut link_obj) != 0 {
            return false;
        }
        let vtbl = *(link_obj as *const *const usize);
        let call = |index: usize, arg: &[u16]| {
            let f: Fn1 = std::mem::transmute(*vtbl.add(index));
            f(link_obj, arg.as_ptr())
        };
        // IShellLinkW: SetDescription 7, SetWorkingDirectory 9, SetPath 20.
        call(20, &target_w);
        call(9, &dir_w);
        call(7, &desc_w);
        let qi: Qi = std::mem::transmute(*vtbl);
        let mut persist: *mut std::ffi::c_void = std::ptr::null_mut();
        let mut ok = false;
        if qi(link_obj, &IID_IPERSIST_FILE, &mut persist) == 0 {
            let pv = *(persist as *const *const usize);
            // IPersistFile::Save is entry 6.
            let save: Save = std::mem::transmute(*pv.add(6));
            ok = save(persist, link_w.as_ptr(), 1) == 0;
            com_release(persist);
        }
        com_release(link_obj);
        ok
    }
}

/// Tells Explorer that file associations or context menus changed.
pub fn notify_associations_changed() {
    use windows_sys::Win32::UI::Shell::{SHCNE_ASSOCCHANGED, SHCNF_IDLIST, SHChangeNotify};
    // SAFETY: no item pointers are passed.
    unsafe { SHChangeNotify(SHCNE_ASSOCCHANGED as _, SHCNF_IDLIST, std::ptr::null(), std::ptr::null()) };
}

/// A plain information box (used without a window, e.g. after uninstalling).
pub fn message_box(title: &str, text: &str) {
    use windows_sys::Win32::UI::WindowsAndMessaging::{MB_ICONINFORMATION, MB_OK, MessageBoxW};
    let wide = |s: &str| -> Vec<u16> { s.encode_utf16().chain(std::iter::once(0)).collect() };
    let (t, m) = (wide(title), wide(text));
    // SAFETY: both strings are zero-terminated and outlive the call.
    unsafe { MessageBoxW(std::ptr::null_mut(), m.as_ptr(), t.as_ptr(), MB_OK | MB_ICONINFORMATION) };
}

#[cfg(test)]
mod tests {
    #[test]
    fn recycle_bin_only_on_local_disks() {
        assert!(super::has_recycle_bin(std::path::Path::new(r"C:\Windows")));
        assert!(!super::has_recycle_bin(std::path::Path::new(r"\server\share\a.txt")));
        assert!(!super::has_recycle_bin(std::path::Path::new("relative.txt")));
    }
}
