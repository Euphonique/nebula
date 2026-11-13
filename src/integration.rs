//! Desktop integration for the current user, without admin rights: install,
//! "Open with Nebula" and Nebula as the program for chosen file types.

#[cfg(windows)]
mod windows;
#[cfg(windows)]
pub use windows::*;

#[cfg(not(windows))]
mod linux;
#[cfg(not(windows))]
pub use linux::*;
