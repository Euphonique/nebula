//! Printing a document: monospaced text in the syntax colors of the light
//! theme, with line numbers, wrapped long lines and page numbers.

#[cfg(windows)]
mod windows;
#[cfg(windows)]
pub use windows::print;

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
pub use linux::print;

use eframe::egui::Color32;

/// What to print, prepared on the UI thread.
pub struct PrintJob {
    pub title: String,
    /// Per document line: text runs and their colors.
    pub lines: Vec<Vec<(String, Color32)>>,
    pub tab: usize,
}
