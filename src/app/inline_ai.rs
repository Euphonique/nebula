//! Inline AI suggestions: after a pause in typing at the end of a line, a
//! built-in agent's model is asked how the code goes on; the answer shows in
//! gray at the cursor (Tab takes it, Esc or typing on drops it). Off by
//! default – each pause is a request that the provider bills.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crossbeam_channel::Receiver;
use eframe::egui;

use super::NebulaApp;
use crate::editor::document::Ghost;

/// Characters before and after the cursor sent along.
const BEFORE: usize = 3000;
const AFTER: usize = 1000;
/// Longest suggestion in lines.
const MAX_LINES: usize = 12;

#[derive(Default)]
pub(super) struct InlineState {
    /// Document and version last seen, and when it was last edited.
    seen: Option<(u64, u64, Option<Instant>)>,
    job: Option<Job>,
    /// An error was shown already (no message on every pause).
    error_shown: bool,
}

struct Job {
    doc_id: u64,
    version: u64,
    at: usize,
    rx: Receiver<Result<String, String>>,
    cancel: Arc<AtomicBool>,
}

impl Drop for Job {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

/// Default model per agent, looked up once.
static MODELS: Mutex<Option<HashMap<String, String>>> = Mutex::new(None);

impl NebulaApp {
    /// The built-in agents that can make suggestions.
    pub(super) fn inline_agents(&self) -> Vec<String> {
        self.settings.agents.iter().filter(|a| crate::agent::is_builtin(a)).map(|a| a.name.clone()).collect()
    }

    /// Called every frame.
    pub(super) fn poll_inline_ai(&mut self, ctx: &egui::Context) {
        if !self.settings.inline_ai.enabled {
            self.inline.job = None;
            return;
        }
        self.inline_result();
        let Some(i) = self.active else { return };
        let d = &self.docs[i];
        if d.read_only || d.hex.is_some() || d.image.is_some() {
            return;
        }
        let (id, version) = (d.id, d.version);
        match self.inline.seen {
            Some((sid, sv, _)) if sid == id && sv == version => {}
            // Edited: wait for the pause.
            Some((sid, _, _)) if sid == id => {
                self.inline.seen = Some((id, version, Some(Instant::now())));
                self.inline.job = None;
            }
            // Another tab (or the first look): no suggestion before typing.
            _ => {
                self.inline.seen = Some((id, version, None));
                self.inline.job = None;
            }
        }
        let Some((_, _, Some(edited))) = self.inline.seen else { return };
        let delay = Duration::from_millis(self.settings.inline_ai.delay_ms.clamp(200, 5000));
        let waited = edited.elapsed();
        if waited < delay {
            ctx.request_repaint_after(delay - waited);
            return;
        }
        self.inline.seen = Some((id, version, None));
        let focused = self.active_editor_id().is_some_and(|eid| ctx.memory(|m| m.has_focus(eid)));
        if focused {
            self.inline_request(i, ctx);
        }
    }

    fn inline_request(&mut self, i: usize, ctx: &egui::Context) {
        let Some(cfg) = self.settings.agents.iter().find(|a| a.name == self.settings.inline_ai.agent && crate::agent::is_builtin(a)).cloned() else {
            return;
        };
        let d = &mut self.docs[i];
        let sel = d.primary_selection();
        if d.selections.len() != 1 || !sel.is_empty() || d.view.completion.is_some() {
            return;
        }
        let at = sel.head;
        let line = d.char_to_line(at);
        let line_start = d.line_to_char(line);
        let line_text = d.line_text(line);
        let col = at - line_start;
        let before_cursor: String = line_text.chars().take(col).collect();
        let after_cursor: String = line_text.chars().skip(col).collect();
        // Only at the end of a line (closing brackets and quotes may follow).
        if !after_cursor.trim_end_matches(['\n', '\r']).chars().all(|c| c.is_whitespace() || ")]}'\";>,`".contains(c)) {
            return;
        }
        let text = d.text();
        let chars: Vec<char> = text.chars().collect();
        let prefix: String = chars[at.saturating_sub(BEFORE)..at.min(chars.len())].iter().collect();
        let suffix: String = chars[at.min(chars.len())..(at + AFTER).min(chars.len())].iter().collect();
        let name = d.title();
        let language = d.language.id().to_string();
        let model = self.settings.inline_ai.model.trim().to_string();
        let (tx, rx) = crossbeam_channel::bounded(1);
        let cancel = Arc::new(AtomicBool::new(false));
        let stop = cancel.clone();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let llm = crate::agent::llm(&cfg);
            let result = model_for(&cfg.name, &model, &llm).and_then(|model| {
                let system = "You complete code in an editor. Reply with only the text to insert at <CURSOR>: no explanation, no code fences, \
                              nothing that is already before or after the cursor. Prefer finishing the current line or statement; at most a few lines. \
                              If nothing sensible fits, reply with nothing.";
                let prompt = format!("File {name} ({language}):\n{prefix}<CURSOR>{suffix}");
                llm.quick(&model, system, &prompt, 200, &stop)
            });
            let _ = tx.send(result.map(|t| clean(&t, &before_cursor)));
            ctx.request_repaint();
        });
        self.inline.job = Some(Job { doc_id: d.id, version: d.version, at, rx, cancel });
    }

    /// Shows an arrived suggestion if text and cursor are still the same.
    fn inline_result(&mut self) {
        let Some(job) = &self.inline.job else { return };
        let Ok(result) = job.rx.try_recv() else { return };
        let job = self.inline.job.take().unwrap();
        match result {
            Ok(text) if !text.is_empty() => {
                if let Some(i) = self.doc_index(job.doc_id) {
                    let d = &mut self.docs[i];
                    d.view.ghost = Some(Ghost { at: job.at, version: job.version, text });
                }
            }
            Ok(_) => {}
            Err(e) if e == crate::agent::llm::CANCELLED => {}
            Err(e) => {
                if !std::mem::replace(&mut self.inline.error_shown, true) {
                    self.error(t!("Inline-Vorschläge: {e}", e = e));
                }
            }
        }
    }
}

/// The model to use: the chosen one, else a small and fast one of the provider.
fn model_for(agent: &str, chosen: &str, llm: &crate::agent::llm::Llm) -> Result<String, String> {
    if !chosen.is_empty() {
        return Ok(chosen.to_string());
    }
    if let Some(m) = MODELS.lock().unwrap().as_ref().and_then(|m| m.get(agent).cloned()) {
        return Ok(m);
    }
    let models = llm.models().map_err(|e| e.message(&llm.base))?;
    const FAST: &[&str] = &["haiku", "mini", "flash", "coder", "qwen", "llama"];
    let pick = FAST
        .iter()
        .find_map(|f| models.iter().rev().find(|m| m.contains(f)).cloned())
        .or_else(|| models.first().cloned())
        .ok_or_else(|| t!("Der Server bietet keine Modelle an.").to_string())?;
    MODELS.lock().unwrap().get_or_insert_with(HashMap::new).insert(agent.to_string(), pick.clone());
    Ok(pick)
}

/// The suggestion without fences and without repeating what is already typed.
fn clean(text: &str, before_cursor: &str) -> String {
    let mut t = text.trim_end().to_string();
    if let Some(rest) = t.strip_prefix("```") {
        t = rest.split_once('\n').map_or("", |(_, r)| r).trim_end().trim_end_matches("```").trim_end().to_string();
    }
    // Models sometimes start with the whole current line.
    let typed = before_cursor.trim_start();
    if !typed.is_empty()
        && let Some(rest) = t.trim_start().strip_prefix(typed)
    {
        t = rest.to_string();
    }
    t.lines().take(MAX_LINES).collect::<Vec<_>>().join("\n")
}

#[cfg(test)]
mod tests {
    use super::clean;

    #[test]
    fn cleans_suggestions() {
        assert_eq!(clean("```php\nreturn $a + $b;\n```", "    "), "return $a + $b;");
        assert_eq!(clean("    $sum = $a + $b;", "    $sum = "), "$a + $b;");
        assert_eq!(clean("+ 1;\n", "x = x "), "+ 1;");
        assert_eq!(clean("", "x"), "");
    }
}
