//! Operating system integration behind one API: `windows.rs` (Win32, DWM,
//! shell) and `linux.rs` (freedesktop, X11/Wayland via egui/winit).

#[cfg(windows)]
mod windows;
#[cfg(windows)]
pub use windows::*;

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
pub use linux::*;
