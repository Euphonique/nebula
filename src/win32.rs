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
