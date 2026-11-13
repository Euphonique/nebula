//! Printing a document through the Windows print dialog: monospaced text in
//! the syntax colors of the light theme, with line numbers, wrapped long
//! lines and page numbers.

use std::thread::JoinHandle;

use eframe::egui::{self, Color32};
use windows_sys::Win32::Foundation::{GlobalFree, HWND, LPARAM, SIZE, WPARAM};
use windows_sys::Win32::System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx, CoUninitialize};
use windows_sys::Win32::Graphics::Gdi::{
    CreateFontW, DeleteDC, DeleteObject, GetDeviceCaps, GetTextExtentPoint32W, GetTextMetricsW, HDC,
    HORZRES, LOGPIXELSX, LOGPIXELSY, PHYSICALOFFSETX, PHYSICALOFFSETY, SelectObject, SetBkMode,
    SetTextColor, TEXTMETRICW, TRANSPARENT, TextOutW, VERTRES,
};
use windows_sys::Win32::Storage::Xps::{AbortDoc, DOCINFOW, EndDoc, EndPage, StartDocW, StartPage};
use windows_sys::Win32::UI::Controls::Dialogs::{
    CommDlgExtendedError, PD_ENABLEPRINTHOOK, PD_NOSELECTION, PD_PAGENUMS, PD_RETURNDC, PD_USEDEVMODECOPIESANDCOLLATE,
    PRINTDLGW, PrintDlgW,
};
use windows_sys::Win32::UI::WindowsAndMessaging::WM_INITDIALOG;

/// What to print, prepared on the UI thread.
pub struct PrintJob {
    pub title: String,
    /// Per document line: text runs and their colors.
    pub lines: Vec<Vec<(String, Color32)>>,
    pub tab: usize,
}

/// Page margin in millimeters.
const MARGIN_MM: f32 = 15.0;
const FONT_PT: i32 = 9;

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().collect()
}

fn colorref(c: Color32) -> u32 {
    c.r() as u32 | (c.g() as u32) << 8 | (c.b() as u32) << 16
}

/// Shows the print dialog and prints the pages, all on a background thread:
/// the dialog runs its own message loop, which must not run inside a frame
/// of the app. The handle yields the number of printed pages (0: cancelled).
pub fn print(job: PrintJob, ctx: &egui::Context) -> Option<JoinHandle<Result<usize, String>>> {
    let owner = crate::win32::window_handle()? as isize;
    let ctx = ctx.clone();
    Some(std::thread::spawn(move || {
        // SAFETY: COM is initialized for this thread only and released below.
        unsafe { CoInitializeEx(std::ptr::null(), COINIT_APARTMENTTHREADED as _) };
        let result = dialog_and_print(owner as HWND, &job);
        unsafe { CoUninitialize() };
        ctx.request_repaint();
        result
    }))
}

/// The print hook keeps Windows 11 from replacing the classic dialog with
/// its modern one, which crashes in this process while creating the print
/// task (null object inside Windows.Graphics.Printing.dll).
unsafe extern "system" fn print_hook(_: HWND, msg: u32, _: WPARAM, _: LPARAM) -> usize {
    // Let the dialog set the initial focus; everything else is default.
    usize::from(msg == WM_INITDIALOG)
}

fn dialog_and_print(owner: HWND, job: &PrintJob) -> Result<usize, String> {
    // SAFETY: the struct is zero-initialized and sized; the dialog fills it.
    let mut pd: PRINTDLGW = unsafe { std::mem::zeroed() };
    pd.lStructSize = std::mem::size_of::<PRINTDLGW>() as u32;
    pd.hwndOwner = owner;
    pd.Flags = PD_RETURNDC | PD_NOSELECTION | PD_USEDEVMODECOPIESANDCOLLATE | PD_ENABLEPRINTHOOK;
    pd.lpfnPrintHook = Some(print_hook);
    pd.nFromPage = 1;
    pd.nToPage = 1;
    pd.nMinPage = 1;
    pd.nMaxPage = 9999;
    pd.nCopies = 1;
    // SAFETY: valid struct; the returned handles are freed below.
    let ok = unsafe { PrintDlgW(&mut pd) };
    unsafe {
        if !pd.hDevMode.is_null() {
            GlobalFree(pd.hDevMode);
        }
        if !pd.hDevNames.is_null() {
            GlobalFree(pd.hDevNames);
        }
    }
    if ok == 0 || pd.hDC.is_null() {
        // SAFETY: no arguments.
        let err = unsafe { CommDlgExtendedError() };
        return if err == 0 { Ok(0) } else { Err(t!("Der Druckdialog konnte nicht geöffnet werden (Fehler {err}).", err = format!("{:#x}", err))) };
    }
    let pages = (pd.Flags & PD_PAGENUMS != 0).then_some([(pd.nFromPage as u32, pd.nToPage as u32)]);
    let result = print_pages(pd.hDC, job, pages.as_ref().map(|p| p.as_slice()), None);
    // SAFETY: the dialog created this DC.
    unsafe { DeleteDC(pd.hDC) };
    result
}

/// One printed row: line number (first row of a line) and colored runs.
struct Row {
    number: Option<usize>,
    runs: Vec<(Vec<u16>, u32)>,
}

/// Splits lines into rows of at most `cols` columns, expanding tabs.
fn layout(job: &PrintJob, cols: usize) -> Vec<Row> {
    let mut rows = Vec::new();
    for (n, line) in job.lines.iter().enumerate() {
        let mut row = Row { number: Some(n + 1), runs: Vec::new() };
        let mut col = 0;
        for (text, color) in line {
            let color = colorref(*color);
            let mut run: Vec<u16> = Vec::new();
            for ch in text.chars() {
                let (s, w): (String, usize) = match ch {
                    '\t' => {
                        let w = job.tab - col % job.tab;
                        (" ".repeat(w), w)
                    }
                    c if c.is_control() => (" ".into(), 1),
                    c => (c.to_string(), 1),
                };
                if col + w > cols && col > 0 {
                    if !run.is_empty() {
                        row.runs.push((std::mem::take(&mut run), color));
                    }
                    rows.push(std::mem::replace(&mut row, Row { number: None, runs: Vec::new() }));
                    col = 0;
                }
                run.extend(s.encode_utf16());
                col += w;
            }
            if !run.is_empty() {
                row.runs.push((run, color));
            }
        }
        rows.push(row);
    }
    rows
}

/// Prints all pages (or those in `pages`) to `hdc`; `output` names a file
/// for printers that write one.
fn print_pages(hdc: HDC, job: &PrintJob, pages: Option<&[(u32, u32)]>, output: Option<&str>) -> Result<usize, String> {
    // SAFETY: GDI calls on a valid printer DC; every object created here is
    // selected out and deleted before returning.
    unsafe {
        let dpi_x = GetDeviceCaps(hdc, LOGPIXELSX as _) as f32;
        let dpi_y = GetDeviceCaps(hdc, LOGPIXELSY as _) as f32;
        let (width, height) = (GetDeviceCaps(hdc, HORZRES as _), GetDeviceCaps(hdc, VERTRES as _));
        let (off_x, off_y) = (GetDeviceCaps(hdc, PHYSICALOFFSETX as _), GetDeviceCaps(hdc, PHYSICALOFFSETY as _));
        let mm = |v: f32, dpi: f32| (v / 25.4 * dpi) as i32;
        let left = (mm(MARGIN_MM, dpi_x) - off_x).max(0);
        let right = width - (mm(MARGIN_MM, dpi_x) - off_x).max(0);
        let top = (mm(MARGIN_MM, dpi_y) - off_y).max(0);
        let bottom = height - (mm(MARGIN_MM, dpi_y) - off_y).max(0);

        let face = wide("Consolas\0");
        let font_h = -(FONT_PT as f32 * dpi_y / 72.0).round() as i32;
        let font = CreateFontW(font_h, 0, 0, 0, 400, 0, 0, 0, 1, 0, 0, 5, 49, face.as_ptr());
        let bold = CreateFontW(font_h, 0, 0, 0, 700, 0, 0, 0, 1, 0, 0, 5, 49, face.as_ptr());
        let old = SelectObject(hdc, font);
        let mut tm: TEXTMETRICW = std::mem::zeroed();
        GetTextMetricsW(hdc, &mut tm);
        let line_h = ((tm.tmHeight + tm.tmExternalLeading) as f32 * 1.12).round() as i32;
        let sample = wide("MMMMMMMMMM");
        let mut size = SIZE { cx: 0, cy: 0 };
        GetTextExtentPoint32W(hdc, sample.as_ptr(), sample.len() as i32, &mut size);
        let char_w = (size.cx as f32 / 10.0).max(1.0);

        let digits = job.lines.len().to_string().len();
        let gutter = ((digits + 2) as f32 * char_w) as i32;
        let cols = (((right - left - gutter) as f32 / char_w) as usize).max(10);
        let rows = layout(job, cols);
        let header_h = line_h * 2;
        let per_page = (((bottom - top - header_h * 2) / line_h).max(1)) as usize;
        let total = rows.len().div_ceil(per_page).max(1);

        let name = wide(&format!("{}\0", job.title));
        let output = output.map(|o| wide(&format!("{o}\0")));
        let doc = DOCINFOW {
            cbSize: std::mem::size_of::<DOCINFOW>() as i32,
            lpszDocName: name.as_ptr(),
            lpszOutput: output.as_ref().map_or(std::ptr::null(), |o| o.as_ptr()),
            lpszDatatype: std::ptr::null(),
            fwType: 0,
        };
        let finish = |ok: bool| {
            SelectObject(hdc, old);
            DeleteObject(font);
            DeleteObject(bold);
            if !ok {
                AbortDoc(hdc);
            }
        };
        if StartDocW(hdc, &doc) <= 0 {
            finish(true);
            return Err(t!("Der Druckauftrag konnte nicht gestartet werden.").into());
        }
        SetBkMode(hdc, TRANSPARENT as _);
        let text = |x: i32, y: i32, s: &[u16], color: u32| {
            SetTextColor(hdc, color);
            TextOutW(hdc, x, y, s.as_ptr(), s.len() as i32);
        };
        let gray = colorref(Color32::from_rgb(128, 128, 128));
        let mut printed = 0;
        for page in 0..total {
            let number = page as u32 + 1;
            if let Some(ranges) = pages
                && !ranges.iter().any(|&(a, b)| (a..=b).contains(&number))
            {
                continue;
            }
            if StartPage(hdc) <= 0 {
                finish(false);
                return Err(t!("Fehler beim Drucken.").into());
            }
            SelectObject(hdc, bold);
            text(left, top, &wide(&job.title), 0);
            SelectObject(hdc, font);
            let label = wide(&t!("Seite {number} von {total}", number = number, total = total));
            let mut s = SIZE { cx: 0, cy: 0 };
            GetTextExtentPoint32W(hdc, label.as_ptr(), label.len() as i32, &mut s);
            text(right - s.cx, bottom - line_h, &label, gray);

            let mut y = top + header_h;
            for row in rows.iter().skip(page * per_page).take(per_page) {
                if let Some(n) = row.number {
                    let num = wide(&format!("{n:>digits$}"));
                    text(left, y, &num, gray);
                }
                let mut x = left + gutter;
                for (run, color) in &row.runs {
                    text(x, y, run, *color);
                    x += (run.len() as f32 * char_w) as i32;
                }
                y += line_h;
            }
            if EndPage(hdc) <= 0 {
                finish(false);
                return Err(t!("Fehler beim Drucken.").into());
            }
            printed += 1;
        }
        EndDoc(hdc);
        finish(true);
        Ok(printed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn long_lines_wrap_and_tabs_expand() {
        let job = PrintJob {
            title: "t".into(),
            lines: vec![
                vec![("\tab".into(), Color32::BLACK), ("cdefgh".into(), Color32::RED)],
                vec![],
            ],
            tab: 4,
        };
        let rows = layout(&job, 6);
        // "    ab" | "cdefgh" | ""
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0].number, Some(1));
        assert_eq!(rows[1].number, None);
        assert_eq!(String::from_utf16_lossy(&rows[0].runs[0].0), "    ab");
        assert_eq!(rows[2].number, Some(2));
    }

    /// Prints 150 lines into a file through "Microsoft Print to PDF" (or the
    /// printer named in `NEBULA_TEST_PRINTER`; its output goes to the file).
    #[test]
    fn prints_pages_to_pdf_printer() {
        let name = std::env::var("NEBULA_TEST_PRINTER").unwrap_or_else(|_| "Microsoft Print to PDF".into());
        let pdf_printer = name == "Microsoft Print to PDF";
        let printer = wide(&format!("{name}\0"));
        // SAFETY: plain GDI calls; the DC is deleted at the end.
        let hdc = unsafe { windows_sys::Win32::Graphics::Gdi::CreateDCW(std::ptr::null(), printer.as_ptr(), std::ptr::null(), std::ptr::null()) };
        if hdc.is_null() {
            eprintln!("{name} not installed; skipped");
            return;
        }
        let job = PrintJob {
            title: "test.php".into(),
            lines: (0..150).map(|i| vec![(format!("line {i} "), Color32::BLACK), ("<?php echo 1; ?>".into(), Color32::BLUE)]).collect(),
            tab: 4,
        };
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("out.pdf");
        let pages = print_pages(hdc, &job, None, Some(&out.to_string_lossy()));
        unsafe { DeleteDC(hdc) };
        assert_eq!(pages, Ok(3));
        // The spooler writes the file shortly after the job ends.
        for _ in 0..50 {
            if std::fs::metadata(&out).is_ok_and(|m| m.len() > 1000) {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        let data = std::fs::read(&out).unwrap();
        assert!(data.len() > 1000);
        if pdf_printer {
            assert!(data.starts_with(b"%PDF"));
        }
    }
}
