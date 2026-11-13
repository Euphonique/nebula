//! Printing through the GTK print dialog (printer choice, page range, "Print
//! to File" as PDF), drawn with Pango on the pages' cairo context.

use std::cell::RefCell;
use std::rc::Rc;
use std::thread::JoinHandle;

use eframe::egui::{self, Color32};
use gtk::prelude::*;
use gtk::{cairo, pango};

use super::PrintJob;

/// Page margin in millimeters.
const MARGIN_MM: f64 = 15.0;
const FONT: &str = "Monospace 9";

/// One printed row: the line number (only on a line's first row) and text runs.
struct Row {
    number: Option<usize>,
    runs: Vec<(String, Color32)>,
}

/// Splits lines into rows of at most `cols` columns, expanding tabs.
fn layout(job: &PrintJob, cols: usize) -> Vec<Row> {
    let mut rows = Vec::new();
    for (n, line) in job.lines.iter().enumerate() {
        let mut row = Row { number: Some(n + 1), runs: Vec::new() };
        let mut col = 0;
        for (text, color) in line {
            let mut run = String::new();
            for ch in text.chars() {
                let (s, w): (String, usize) = match ch {
                    '\t' => {
                        let w = job.tab.max(1) - col % job.tab.max(1);
                        (" ".repeat(w), w)
                    }
                    c if c.is_control() => (" ".into(), 1),
                    c => (c.to_string(), 1),
                };
                if col + w > cols && col > 0 {
                    if !run.is_empty() {
                        row.runs.push((std::mem::take(&mut run), *color));
                    }
                    rows.push(std::mem::replace(&mut row, Row { number: None, runs: Vec::new() }));
                    col = 0;
                }
                run.push_str(&s);
                col += w;
            }
            if !run.is_empty() {
                row.runs.push((run, *color));
            }
        }
        rows.push(row);
    }
    rows
}

/// What `begin-print` works out for `draw-page`.
struct Pages {
    rows: Vec<Row>,
    per_page: usize,
    total: usize,
    char_w: f64,
    line_h: f64,
    gutter: f64,
    digits: usize,
}

fn set_color(cr: &cairo::Context, c: Color32) {
    cr.set_source_rgb(c.r() as f64 / 255.0, c.g() as f64 / 255.0, c.b() as f64 / 255.0);
}

/// Shows the print dialog and prints. GTK runs the dialog modally on this
/// (the UI) thread; the handle yields the number of printed pages (0: cancelled).
pub fn print(job: PrintJob, ctx: &egui::Context) -> Option<JoinHandle<Result<usize, String>>> {
    let result = dialog_and_print(job, None);
    ctx.request_repaint();
    Some(std::thread::spawn(move || result))
}

/// With `pdf`, writes that file without a dialog (for tests).
fn dialog_and_print(job: PrintJob, pdf: Option<&std::path::Path>) -> Result<usize, String> {
    gtk::init().map_err(|e| t!("GTK konnte nicht gestartet werden: {e}", e = e))?;
    let op = gtk::PrintOperation::new();
    op.set_job_name(&job.title);
    op.set_use_full_page(true);
    op.set_unit(gtk::Unit::Points);
    op.set_embed_page_setup(true);
    let job = Rc::new(job);
    let pages: Rc<RefCell<Option<Pages>>> = Rc::default();

    let (j, p) = (job.clone(), pages.clone());
    op.connect_begin_print(move |op, pc| {
        let margin = MARGIN_MM / 25.4 * 72.0;
        let Some(cr) = pc.cairo_context() else { return };
        let probe = pangocairo::functions::create_layout(&cr);
        probe.set_font_description(Some(&pango::FontDescription::from_string(FONT)));
        probe.set_text("MMMMMMMMMM");
        let (w, h) = probe.pixel_size();
        let char_w = (w as f64 / 10.0).max(1.0);
        let line_h = h as f64 * 1.12;
        let digits = j.lines.len().to_string().len();
        let gutter = (digits + 2) as f64 * char_w;
        let cols = (((pc.width() - 2.0 * margin - gutter) / char_w) as usize).max(10);
        let rows = layout(&j, cols);
        let header_h = line_h * 2.0;
        let per_page = (((pc.height() - 2.0 * margin - header_h * 2.0) / line_h) as usize).max(1);
        let total = rows.len().div_ceil(per_page).max(1);
        op.set_n_pages(total as i32);
        *p.borrow_mut() = Some(Pages { rows, per_page, total, char_w, line_h, gutter, digits });
    });

    let (j, p) = (job.clone(), pages.clone());
    op.connect_draw_page(move |_, pc, page| {
        let guard = p.borrow();
        let Some(pages) = guard.as_ref() else { return };
        let page = page.max(0) as usize;
        let margin = MARGIN_MM / 25.4 * 72.0;
        let (left, top) = (margin, margin);
        let (right, bottom) = (pc.width() - margin, pc.height() - margin);
        let Some(cr) = pc.cairo_context() else { return };
        let pl = pangocairo::functions::create_layout(&cr);
        let mut font = pango::FontDescription::from_string(FONT);
        let gray = Color32::from_rgb(128, 128, 128);
        let mut text = |x: f64, y: f64, s: &str, color: Color32, bold: bool| {
            font.set_weight(if bold { pango::Weight::Bold } else { pango::Weight::Normal });
            pl.set_font_description(Some(&font));
            pl.set_text(s);
            set_color(&cr, color);
            cr.move_to(x, y);
            pangocairo::functions::show_layout(&cr, &pl);
            pl.pixel_size().0 as f64
        };
        text(left, top, &j.title, Color32::BLACK, true);
        let label = t!("Seite {number} von {total}", number = page + 1, total = pages.total);
        // Right-aligned: measure first, then draw.
        let w = text(-10_000.0, -10_000.0, &label, gray, false);
        text(right - w, bottom - pages.line_h, &label, gray, false);
        let mut y = top + pages.line_h * 2.0;
        for row in pages.rows.iter().skip(page * pages.per_page).take(pages.per_page) {
            if let Some(n) = row.number {
                text(left, y, &format!("{n:>width$}", width = pages.digits), gray, false);
            }
            let mut x = left + pages.gutter;
            for (run, color) in &row.runs {
                text(x, y, run, *color, false);
                x += run.chars().count() as f64 * pages.char_w;
            }
            y += pages.line_h;
        }
    });

    let action = match pdf {
        Some(path) => {
            op.set_export_filename(path);
            gtk::PrintOperationAction::Export
        }
        None => gtk::PrintOperationAction::PrintDialog,
    };
    match op.run(action, None::<&gtk::Window>) {
        Ok(gtk::PrintOperationResult::Apply) => {
            let n = op.n_pages_to_print();
            let total = pages.borrow().as_ref().map_or(0, |p| p.total);
            Ok(if n > 0 { n as usize } else { total })
        }
        Ok(gtk::PrintOperationResult::Error) => Err(t!("Fehler beim Drucken.").into()),
        Ok(_) => Ok(0),
        Err(e) => Err(format!("{}: {e}", t!("Fehler beim Drucken."))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Renders a PDF without the dialog (needs a display):
    /// `cargo test print_to_pdf -- --ignored`, then look at the file.
    #[test]
    #[ignore]
    fn print_to_pdf() {
        let lines: Vec<Vec<(String, Color32)>> = (0..130)
            .map(|i| {
                vec![
                    ("fn ".to_string(), Color32::from_rgb(0, 0, 200)),
                    (format!("zeile_{i}() {{ \"Grüße – {}\" }}", "lang ".repeat(i % 30)), Color32::from_rgb(160, 60, 0)),
                ]
            })
            .collect();
        let job = PrintJob { title: "Drucktest.rs".into(), lines, tab: 4 };
        let out = std::env::temp_dir().join("nebula-print-test.pdf");
        let pages = dialog_and_print(job, Some(&out)).unwrap();
        assert!(pages >= 2, "{pages}");
        assert!(std::fs::metadata(&out).unwrap().len() > 1000);
        eprintln!("{} ({pages} pages)", out.display());
    }

    #[test]
    fn long_lines_wrap_and_tabs_expand() {
        let job = PrintJob {
            title: "t".into(),
            lines: vec![vec![("\tabcdefgh".into(), Color32::BLACK)], vec![("x".into(), Color32::RED)]],
            tab: 4,
        };
        let rows = layout(&job, 6);
        let text: Vec<(Option<usize>, String)> = rows.iter().map(|r| (r.number, r.runs.iter().map(|(s, _)| s.as_str()).collect())).collect();
        assert_eq!(text, [(Some(1), "    ab".into()), (None, "cdefgh".into()), (Some(2), "x".into())]);
    }
}
