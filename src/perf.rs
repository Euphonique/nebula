//! Frame timing for performance checks: with `NEBULA_PERF=1`, frames slower
//! than 20 ms are logged to stderr together with the timed sections.

use std::cell::RefCell;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

fn enabled() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("NEBULA_PERF").is_some())
}

thread_local! {
    static SECTIONS: RefCell<Vec<(&'static str, Duration)>> = const { RefCell::new(Vec::new()) };
}

/// Times a section until dropped.
pub struct Section(Option<(&'static str, Instant)>);

impl Drop for Section {
    fn drop(&mut self) {
        if let Some((name, start)) = self.0 {
            SECTIONS.with(|s| s.borrow_mut().push((name, start.elapsed())));
        }
    }
}

pub fn section(name: &'static str) -> Section {
    if enabled() && std::env::var_os("NEBULA_PERF_TRACE").is_some() {
        eprintln!("> {name}");
    }
    Section(enabled().then(|| (name, Instant::now())))
}

/// Times a whole frame; logs it with its sections when it was slow.
pub struct Frame(Option<Instant>);

impl Drop for Frame {
    fn drop(&mut self) {
        let Some(start) = self.0 else { return };
        let total = start.elapsed();
        let sections = SECTIONS.with(|s| std::mem::take(&mut *s.borrow_mut()));
        if total >= Duration::from_millis(20) {
            let parts: Vec<String> = sections.iter().filter(|(_, d)| d.as_micros() >= 1000).map(|(n, d)| format!("{n} {:.1}", d.as_secs_f64() * 1000.0)).collect();
            eprintln!("[perf] Frame {:.1} ms: {}", total.as_secs_f64() * 1000.0, parts.join(", "));
        }
    }
}

pub fn frame() -> Frame {
    Frame(enabled().then(Instant::now))
}

#[cfg(test)]
mod bench {
    use std::time::Instant;

    use crate::editor::{DocSource, Document, commands, complete, folding};
    use crate::lang::Language;

    fn time<T>(name: &str, f: impl FnOnce() -> T) -> T {
        let start = Instant::now();
        let out = f();
        eprintln!("{name}: {:.1} ms", start.elapsed().as_secs_f64() * 1000.0);
        out
    }

    /// `NEBULA_PERF_FILE=<file> cargo test --profile perf bench_file -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn bench_file() {
        let path = std::env::var("NEBULA_PERF_FILE").expect("NEBULA_PERF_FILE");
        let bytes = time("Lesen", || std::fs::read(&path).unwrap());
        let mut doc = time("Dokument", || Document::from_bytes(DocSource::Local(path.clone().into()), &bytes));
        doc.language = Language::from_path(&path);
        let tab = 4;
        time("max_line_columns", || doc.max_line_columns(tab));
        time("text()", || doc.text());
        time("fold_ranges", || folding::fold_ranges(&mut doc, tab));
        time("rows (Umbruch 120)", || folding::rows(&mut doc, tab, Some(120)));
        let text = doc.text();
        time("Highlighting", || crate::highlight::highlight_once(doc.language, &text));
        crate::highlight::bench_incremental(doc.language, &text);
        doc.view.git_base = Some(text.clone());
        time("Git-Marken", || crate::editor::view::update_git_marks(&mut doc));
        let settings = crate::settings::EditorSettings::default();
        let mid = doc.len_chars() / 2;
        doc.set_selection(crate::editor::document::Selection::new(mid, mid));
        time("Tippen", || commands::type_text(&mut doc, "fu", &settings));
        time("Vervollständigung", || complete::compute(&mut doc));
        time("fold_ranges nach Edit", || folding::fold_ranges(&mut doc, tab));
        time("rows nach Edit", || folding::rows(&mut doc, tab, Some(120)));
        time("max_line_columns nach Edit", || doc.max_line_columns(tab));
        doc.view.git_marks_at = None;
        time("Git-Marken nach Edit", || crate::editor::view::update_git_marks(&mut doc));
    }
}
