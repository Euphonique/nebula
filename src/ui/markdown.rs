//! Compact Markdown display for chat answers: paragraphs, headings, lists,
//! quotes, code blocks and inline styles (bold, italic, code, links).

use eframe::egui::{self, Color32, FontFamily, FontId, RichText, Stroke, text::LayoutJob};
use pulldown_cmark::{CodeBlockKind, Event, HeadingLevel, Options, Parser, Tag, TagEnd};

use crate::theme::{Theme, bold_family};

#[derive(Default)]
struct Inline {
    bold: u32,
    italic: u32,
    link: u32,
    heading: Option<HeadingLevel>,
}

/// Renders `text` into `ui` (selectable, wrapping to the available width).
pub fn show(ui: &mut egui::Ui, theme: &Theme, text: &str) {
    let parser = Parser::new_ext(text, Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TASKLISTS);
    let mut job = LayoutJob::default();
    let mut style = Inline::default();
    // Open lists: next number for ordered ones.
    let mut lists: Vec<Option<u64>> = Vec::new();
    let mut quote = 0u32;
    let mut code: Option<(String, String)> = None;
    let base = 13.5;

    let flush = |ui: &mut egui::Ui, job: &mut LayoutJob, indent: f32| {
        if job.sections.is_empty() {
            return;
        }
        let mut j = std::mem::take(job);
        j.wrap.max_width = (ui.available_width() - indent).max(40.0);
        ui.horizontal(|ui| {
            ui.add_space(indent);
            ui.add(egui::Label::new(j).selectable(true));
        });
    };
    let indent = |lists: &Vec<Option<u64>>, quote: u32| lists.len().saturating_sub(1) as f32 * 16.0 + quote as f32 * 12.0;

    for event in parser {
        if let Some((_, buf)) = &mut code {
            match event {
                Event::Text(t) => buf.push_str(&t),
                Event::End(TagEnd::CodeBlock) => {
                    let (lang, buf) = code.take().unwrap();
                    code_block(ui, theme, &lang, buf.trim_end_matches('\n'));
                }
                _ => {}
            }
            continue;
        }
        match event {
            Event::Start(Tag::Heading { level, .. }) => {
                flush(ui, &mut job, indent(&lists, quote));
                ui.add_space(4.0);
                style.heading = Some(level);
            }
            Event::End(TagEnd::Heading(_)) => {
                flush(ui, &mut job, indent(&lists, quote));
                style.heading = None;
            }
            Event::Start(Tag::Paragraph) => {}
            Event::End(TagEnd::Paragraph) => {
                flush(ui, &mut job, indent(&lists, quote));
                if lists.is_empty() {
                    ui.add_space(4.0);
                }
            }
            Event::Start(Tag::List(start)) => {
                flush(ui, &mut job, indent(&lists, quote));
                lists.push(start);
            }
            Event::End(TagEnd::List(_)) => {
                flush(ui, &mut job, indent(&lists, quote));
                lists.pop();
                if lists.is_empty() {
                    ui.add_space(4.0);
                }
            }
            Event::Start(Tag::Item) => {
                flush(ui, &mut job, indent(&lists, quote));
                let bullet = match lists.last_mut() {
                    Some(Some(n)) => {
                        *n += 1;
                        format!("{}. ", *n - 1)
                    }
                    _ => "•  ".to_string(),
                };
                append(&mut job, theme, &style, &bullet, base, false);
            }
            Event::End(TagEnd::Item) => flush(ui, &mut job, indent(&lists, quote)),
            Event::Start(Tag::BlockQuote(_)) => {
                flush(ui, &mut job, indent(&lists, quote));
                quote += 1;
            }
            Event::End(TagEnd::BlockQuote(_)) => {
                flush(ui, &mut job, indent(&lists, quote));
                quote = quote.saturating_sub(1);
            }
            Event::Start(Tag::CodeBlock(kind)) => {
                flush(ui, &mut job, indent(&lists, quote));
                let lang = match kind {
                    CodeBlockKind::Fenced(l) => l.to_string(),
                    CodeBlockKind::Indented => String::new(),
                };
                code = Some((lang, String::new()));
            }
            Event::Start(Tag::Strong) => style.bold += 1,
            Event::End(TagEnd::Strong) => style.bold = style.bold.saturating_sub(1),
            Event::Start(Tag::Emphasis) => style.italic += 1,
            Event::End(TagEnd::Emphasis) => style.italic = style.italic.saturating_sub(1),
            Event::Start(Tag::Link { .. }) => style.link += 1,
            Event::End(TagEnd::Link) => style.link = style.link.saturating_sub(1),
            Event::Text(t) => append(&mut job, theme, &style, &t, base, false),
            Event::Code(t) => append(&mut job, theme, &style, &t, base, true),
            Event::SoftBreak => append(&mut job, theme, &style, " ", base, false),
            Event::HardBreak => append(&mut job, theme, &style, "\n", base, false),
            Event::TaskListMarker(done) => append(&mut job, theme, &style, if done { "☑ " } else { "☐ " }, base, false),
            Event::Rule => {
                flush(ui, &mut job, indent(&lists, quote));
                ui.separator();
            }
            // Tables: cells as plain text, one row per line.
            Event::End(TagEnd::TableCell) => append(&mut job, theme, &style, "   ", base, false),
            Event::End(TagEnd::TableRow) | Event::End(TagEnd::TableHead) => flush(ui, &mut job, indent(&lists, quote)),
            Event::End(TagEnd::Table) => ui.add_space(4.0),
            _ => {}
        }
    }
    if let Some((lang, buf)) = code {
        code_block(ui, theme, &lang, buf.trim_end_matches('\n'));
    }
    flush(ui, &mut job, indent(&lists, quote));
}

fn append(job: &mut LayoutJob, theme: &Theme, style: &Inline, text: &str, base: f32, code: bool) {
    let size = match style.heading {
        Some(HeadingLevel::H1) => base + 5.0,
        Some(HeadingLevel::H2) => base + 3.0,
        Some(_) => base + 1.0,
        None => base,
    };
    let bold = style.bold > 0 || style.heading.is_some();
    let family = if code {
        FontFamily::Monospace
    } else if bold {
        bold_family()
    } else {
        FontFamily::Proportional
    };
    let mut format = egui::TextFormat::simple(FontId::new(if code { size - 1.0 } else { size }, family), theme.foreground);
    if code {
        format.background = theme.hover.gamma_multiply(1.6);
    }
    if style.italic > 0 {
        format.italics = true;
    }
    if style.link > 0 {
        format.color = theme.accent;
        format.underline = Stroke::new(1.0, theme.accent.gamma_multiply(0.6));
    }
    job.append(text, 0.0, format);
}

fn code_block(ui: &mut egui::Ui, theme: &Theme, lang: &str, text: &str) {
    egui::Frame::new()
        .fill(theme.background)
        .stroke(Stroke::new(1.0, theme.hover.gamma_multiply(1.5)))
        .corner_radius(6.0)
        .inner_margin(egui::Margin::symmetric(8, 6))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            if !lang.is_empty() {
                ui.label(RichText::new(lang).size(10.5).color(theme.text_faint));
            }
            egui::ScrollArea::horizontal().id_salt(ui.next_auto_id()).show(ui, |ui| {
                ui.add(egui::Label::new(RichText::new(text).monospace().size(12.5).color(theme.foreground)).selectable(true).extend());
            });
        });
    ui.add_space(4.0);
}

/// Plain monospace text with a colored background per line, for diffs.
pub fn diff_rows(ui: &mut egui::Ui, theme: &Theme, old: &str, new: &str, max_rows: usize) {
    use crate::diff::DiffLine;
    let rows = crate::diff::unified(old, new, 2);
    let added_bg = theme.success.gamma_multiply(0.18);
    let removed_bg = theme.danger.gamma_multiply(0.18);
    egui::Frame::new()
        .fill(theme.background)
        .corner_radius(6.0)
        .inner_margin(egui::Margin::symmetric(0, 4))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 0.0;
            egui::ScrollArea::horizontal().id_salt(ui.next_auto_id()).show(ui, |ui| {
                for (n, row) in rows.iter().enumerate() {
                    if n == max_rows {
                        ui.label(RichText::new(format!("  … {} weitere Zeilen", rows.len() - max_rows)).size(11.5).color(theme.text_faint));
                        break;
                    }
                    let (prefix, text, bg) = match row {
                        DiffLine::Added(_, t) => ("+ ", t.as_str(), added_bg),
                        DiffLine::Removed(_, t) => ("- ", t.as_str(), removed_bg),
                        DiffLine::Context(_, _, t) => ("  ", t.as_str(), Color32::TRANSPARENT),
                        DiffLine::Skip(n) => {
                            ui.label(RichText::new(format!("  ⋯ {n} unveränderte Zeilen")).size(11.0).color(theme.text_faint));
                            continue;
                        }
                    };
                    let line = format!("{prefix}{}", text.trim_end_matches('\n'));
                    ui.add(egui::Label::new(RichText::new(line).monospace().size(12.0).background_color(bg).color(theme.foreground)).extend());
                }
            });
        });
}
