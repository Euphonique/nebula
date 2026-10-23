//! Spell checking with the Windows spell checker (`ISpellChecker`).
//!
//! Prose is checked: comments in code, text in Markdown, HTML and plain text.
//! Lines are checked on a worker thread and cached by their content, so the
//! editor asks for the visible lines every frame and draws what is known.
//! With several languages, a word counts as wrong only if all of them reject it.

use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::ops::Range;
use std::sync::{Arc, LazyLock, Mutex, OnceLock};

use crossbeam_channel::Sender;
use eframe::egui;

use crate::highlight::Span;
use crate::lang::Language;
use crate::theme::{HIGHLIGHT_NAMES, PLAIN};

/// A word the spell checker rejects, in chars of its line.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Miss {
    pub range: Range<usize>,
    pub word: String,
}

/// A part of a line to check: start (in chars) and text.
pub type Segment = (usize, String);

enum Msg {
    Configure(Vec<String>),
    Check { key: u64, segments: Vec<Segment> },
    Suggest(String),
    Add(String),
    Ignore(String),
}

#[derive(Default)]
struct Shared {
    cache: HashMap<u64, Arc<[Miss]>>,
    pending: HashSet<u64>,
    suggestions: HashMap<String, Arc<Vec<String>>>,
    /// Language tags Windows can check.
    available: Vec<String>,
    /// Tags in use.
    active: Vec<String>,
    /// The setting last passed to `configure`.
    requested: Option<Vec<String>>,
    /// Ignored for this session (all windows' checkers forget them on exit).
    ignored: HashSet<String>,
}

static SHARED: LazyLock<Mutex<Shared>> = LazyLock::new(Default::default);
static WORKER: OnceLock<Sender<Msg>> = OnceLock::new();

/// Starts the worker; `ctx` is repainted when results arrive.
pub fn init(ctx: egui::Context) {
    WORKER.get_or_init(|| {
        let (tx, rx) = crossbeam_channel::unbounded();
        let _ = std::thread::Builder::new().name("spell".into()).spawn(move || worker::run(rx, ctx));
        tx
    });
}

fn send(msg: Msg) {
    if let Some(tx) = WORKER.get() {
        let _ = tx.send(msg);
    }
}

/// Uses these language tags (`de-DE`, `en-US`; empty: Windows language and English).
pub fn configure(tags: &[String]) {
    let mut sh = SHARED.lock().unwrap();
    if sh.requested.as_deref() == Some(tags) {
        return;
    }
    sh.requested = Some(tags.to_vec());
    drop(sh);
    send(Msg::Configure(tags.to_vec()));
}

/// Languages Windows can check and those in use.
pub fn languages() -> (Vec<String>, Vec<String>) {
    let sh = SHARED.lock().unwrap();
    (sh.available.clone(), sh.active.clone())
}

/// Misspelled words of a line given as segments; `None` while it is checked.
pub fn check(segments: Vec<Segment>) -> Option<Arc<[Miss]>> {
    if segments.is_empty() {
        return Some(Arc::from(Vec::new()));
    }
    let mut h = std::collections::hash_map::DefaultHasher::new();
    segments.hash(&mut h);
    let key = h.finish();
    let mut sh = SHARED.lock().unwrap();
    if let Some(m) = sh.cache.get(&key) {
        return Some(m.clone());
    }
    if WORKER.get().is_some() && sh.pending.insert(key) {
        drop(sh);
        send(Msg::Check { key, segments });
    }
    None
}

/// Suggestions for a word; `None` while they are looked up.
pub fn suggestions(word: &str) -> Option<Arc<Vec<String>>> {
    let mut sh = SHARED.lock().unwrap();
    if let Some(s) = sh.suggestions.get(word) {
        return Some(s.clone());
    }
    if !sh.pending.contains(&suggest_key(word)) {
        sh.pending.insert(suggest_key(word));
        drop(sh);
        send(Msg::Suggest(word.to_string()));
    }
    None
}

fn suggest_key(word: &str) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    ("suggest", word).hash(&mut h);
    h.finish()
}

/// Adds a word to the Windows user dictionary.
pub fn add(word: &str) {
    forget(word);
    send(Msg::Add(word.to_string()));
}

/// Ignores a word until Nebula is closed.
pub fn ignore(word: &str) {
    SHARED.lock().unwrap().ignored.insert(word.to_string());
    forget(word);
    send(Msg::Ignore(word.to_string()));
}

/// Drops a word from the cached results, so its marks disappear at once.
fn forget(word: &str) {
    let mut sh = SHARED.lock().unwrap();
    for misses in sh.cache.values_mut() {
        if misses.iter().any(|m| m.word == word) {
            *misses = misses.iter().filter(|m| m.word != word).cloned().collect();
        }
    }
}

fn style_id(name: &str) -> u8 {
    HIGHLIGHT_NAMES.iter().position(|n| *n == name).map_or(PLAIN, |i| i as u8)
}

static COMMENT: LazyLock<[u8; 2]> = LazyLock::new(|| [style_id("comment"), style_id("comment.documentation")]);
/// Markdown: code, addresses and markup are no prose.
static NOT_PROSE: LazyLock<Vec<u8>> = LazyLock::new(|| {
    HIGHLIGHT_NAMES
        .iter()
        .enumerate()
        .filter(|(_, n)| {
            matches!(**n, "text.literal" | "markup.raw" | "text.uri" | "markup.link" | "string.escape") || n.starts_with("tag") || n.starts_with("punctuation")
        })
        .map(|(i, _)| i as u8)
        .collect()
});

/// The parts of a line to check (text without its line break, highlight
/// spans with byte ranges); code-like words inside them become spaces.
pub fn segments(line: &str, spans: &[Span], language: Language) -> Vec<Segment> {
    use Language::*;
    let prose = |style: u8| match language {
        Markdown | PlainText => !NOT_PROSE.contains(&style),
        Html | Xml => style == PLAIN || COMMENT.contains(&style),
        Json | Diff => false,
        _ => COMMENT.contains(&style),
    };
    // Byte ranges of prose: spans with a prose style, and for text-like
    // languages also the gaps between spans.
    let mut ranges: Vec<Range<usize>> = Vec::new();
    let mut push = |r: Range<usize>| match ranges.last_mut() {
        Some(last) if last.end == r.start => last.end = r.end,
        _ => ranges.push(r),
    };
    let mut at = 0;
    for s in spans {
        let (start, end) = (s.start as usize, (s.end as usize).min(line.len()));
        if start > at && prose(PLAIN) {
            push(at..start);
        }
        if end > start && prose(s.style) {
            push(start..end);
        }
        at = at.max(end);
    }
    if at < line.len() && prose(PLAIN) {
        push(at..line.len());
    }
    ranges
        .into_iter()
        .filter(|r| line.is_char_boundary(r.start) && line.is_char_boundary(r.end))
        .map(|r| (line[..r.start].chars().count(), mask_code(&line[r])))
        .filter(|(_, t)| t.chars().any(char::is_alphabetic))
        .collect()
}

/// Markdown: line ranges of fenced code blocks (``` or ~~~, fences included).
pub fn code_fences(text: &str) -> Vec<Range<usize>> {
    let mut out = Vec::new();
    let mut open: Option<(usize, &str)> = None;
    for (n, line) in text.lines().enumerate() {
        let t = line.trim_start();
        let fence = if t.starts_with("```") { "```" } else if t.starts_with("~~~") { "~~~" } else { continue };
        match open {
            None => open = Some((n, fence)),
            Some((start, f)) if f == fence => {
                out.push(start..n + 1);
                open = None;
            }
            Some(_) => {}
        }
    }
    if let Some((start, _)) = open {
        out.push(start..usize::MAX);
    }
    out
}

/// Replaces words that look like code, paths or addresses with spaces
/// (same number of chars), so only prose is left for the checker.
fn mask_code(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for (i, token) in text.split(' ').enumerate() {
        if i > 0 {
            out.push(' ');
        }
        let word = token.trim_matches(|c: char| !c.is_alphanumeric());
        let chars: Vec<char> = word.chars().collect();
        let camel = chars.windows(2).any(|w| w[0].is_lowercase() && w[1].is_uppercase());
        let caps = chars.len() >= 2 && chars.iter().all(|c| !c.is_lowercase());
        let code = token.contains(|c: char| c.is_ascii_digit() || matches!(c, '_' | '/' | '\\' | '@' | '=' | '<' | '>' | '$' | '{' | '}'))
            || token.contains("()")
            || token.contains("::")
            || word.contains("://")
            || word.contains("..")
            || (word.contains('.') && !word.ends_with('.'))
            || camel
            || caps;
        if code {
            out.extend(std::iter::repeat_n(' ', token.chars().count()));
        } else {
            out.push_str(token);
        }
    }
    out
}

#[cfg(windows)]
mod worker {
    use super::*;
    use windows::Win32::Globalization::{CORRECTIVE_ACTION_NONE, ISpellChecker, ISpellCheckerFactory, SpellCheckerFactory};
    use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx, CoTaskMemFree, IEnumString};
    use windows::core::{HSTRING, PWSTR};

    pub(super) fn run(rx: crossbeam_channel::Receiver<Msg>, ctx: egui::Context) {
        // SAFETY: COM is initialized for this thread; all interfaces stay on it.
        let factory: ISpellCheckerFactory = unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
            match CoCreateInstance(&SpellCheckerFactory, None, CLSCTX_INPROC_SERVER) {
                Ok(f) => f,
                Err(_) => return,
            }
        };
        // SAFETY: enumerating the factory's language tags.
        let available = unsafe { factory.SupportedLanguages().map(|e| strings(&e)).unwrap_or_default() };
        SHARED.lock().unwrap().available = available.clone();
        let mut checkers: Vec<ISpellChecker> = Vec::new();
        for msg in rx {
            match msg {
                Msg::Configure(tags) => {
                    let tags = if tags.is_empty() { automatic(&available) } else { tags };
                    checkers = tags
                        .iter()
                        // SAFETY: plain factory calls with a valid string.
                        .filter_map(|t| unsafe { factory.CreateSpellChecker(&HSTRING::from(t.as_str())).ok() })
                        .collect();
                    let ignored: Vec<String> = SHARED.lock().unwrap().ignored.iter().cloned().collect();
                    for c in &checkers {
                        for w in &ignored {
                            // SAFETY: valid checker and string.
                            let _ = unsafe { c.Ignore(&HSTRING::from(w.as_str())) };
                        }
                    }
                    let mut sh = SHARED.lock().unwrap();
                    sh.active = tags.into_iter().filter(|t| available.contains(t)).collect();
                    sh.cache.clear();
                    sh.pending.clear();
                    sh.suggestions.clear();
                }
                Msg::Check { key, segments } => {
                    let misses: Vec<Miss> = segments.iter().flat_map(|(start, text)| check_text(&checkers, *start, text)).collect();
                    let mut sh = SHARED.lock().unwrap();
                    if sh.pending.remove(&key) {
                        sh.cache.insert(key, Arc::from(misses));
                        if sh.cache.len() > 50_000 {
                            sh.cache.clear();
                        }
                    }
                }
                Msg::Suggest(word) => {
                    let mut list: Vec<String> = Vec::new();
                    for c in &checkers {
                        // SAFETY: valid checker and string.
                        let more = unsafe { c.Suggest(&HSTRING::from(word.as_str())).map(|e| strings(&e)).unwrap_or_default() };
                        for s in more {
                            if !list.contains(&s) {
                                list.push(s);
                            }
                        }
                    }
                    list.truncate(8);
                    let mut sh = SHARED.lock().unwrap();
                    sh.pending.remove(&suggest_key(&word));
                    sh.suggestions.insert(word, Arc::new(list));
                }
                Msg::Add(word) => {
                    if let Some(c) = checkers.first() {
                        // SAFETY: valid checker and string.
                        let _ = unsafe { c.Add(&HSTRING::from(word.as_str())) };
                    }
                }
                Msg::Ignore(word) => {
                    for c in &checkers {
                        // SAFETY: valid checker and string.
                        let _ = unsafe { c.Ignore(&HSTRING::from(word.as_str())) };
                    }
                }
            }
            ctx.request_repaint();
        }
    }

    /// Windows' language (e.g. de-DE) and English.
    fn automatic(available: &[String]) -> Vec<String> {
        let lang = crate::i18n::system_language();
        let mut tags = Vec::new();
        for want in [lang.as_str(), "en"] {
            let pick = available
                .iter()
                .find(|t| t.eq_ignore_ascii_case(&format!("{want}-{}", want.to_uppercase())))
                .or_else(|| available.iter().find(|t| t.split('-').next().is_some_and(|p| p.eq_ignore_ascii_case(want))));
            if let Some(t) = pick
                && !tags.contains(t)
            {
                tags.push(t.clone());
            }
        }
        tags
    }

    /// Words of `text` that every checker rejects.
    fn check_text(checkers: &[ISpellChecker], start: usize, text: &str) -> Vec<Miss> {
        let Some((first, others)) = checkers.split_first() else { return Vec::new() };
        let utf16: Vec<u16> = text.encode_utf16().collect();
        let mut out = Vec::new();
        // SAFETY: valid checker, string and enumerator; returned errors are owned.
        unsafe {
            let Ok(errors) = first.Check(&HSTRING::from(text)) else { return out };
            loop {
                let mut error = None;
                if errors.Next(&mut error).is_err() {
                    break;
                }
                let Some(error) = error else { break };
                let (Ok(s), Ok(len)) = (error.StartIndex(), error.Length()) else { continue };
                if error.CorrectiveAction().is_ok_and(|a| a == CORRECTIVE_ACTION_NONE) {
                    continue;
                }
                let (s, e) = (s as usize, (s + len) as usize);
                let Some(word_utf16) = utf16.get(s..e) else { continue };
                let word = String::from_utf16_lossy(word_utf16);
                if others.iter().any(|c| c.Check(&HSTRING::from(word.as_str())).is_ok_and(|en| {
                    let mut e = None;
                    let _ = en.Next(&mut e);
                    e.is_none()
                })) {
                    continue;
                }
                let chars = |u: usize| String::from_utf16_lossy(&utf16[..u]).chars().count();
                out.push(Miss { range: start + chars(s)..start + chars(e), word });
            }
        }
        out
    }

    /// All strings of an enumerator (freeing them).
    fn strings(e: &IEnumString) -> Vec<String> {
        let mut out = Vec::new();
        loop {
            let mut item = [PWSTR::null()];
            let mut fetched = 0u32;
            // SAFETY: one slot, filled with a CoTaskMemAlloc'ed string we free.
            unsafe {
                if e.Next(&mut item, Some(&mut fetched)).is_err() || fetched == 0 {
                    break;
                }
                out.push(item[0].to_string().unwrap_or_default());
                CoTaskMemFree(Some(item[0].0 as _));
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn span(start: u32, end: u32, name: &str) -> Span {
        Span { start, end, style: style_id(name) }
    }

    /// Against the real Windows spell checker (needs the dictionaries).
    #[test]
    #[ignore]
    fn windows_spell_checker() {
        init(egui::Context::default());
        configure(&[]);
        let wait = |f: &dyn Fn() -> bool| {
            for _ in 0..100 {
                if f() {
                    return;
                }
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
            panic!("no answer");
        };
        wait(&|| !languages().1.is_empty());
        println!("{:?}", languages());
        let seg = || vec![(0, "Das ist ein Fehlr in the sum".to_string())];
        wait(&|| check(seg()).is_some());
        let words: Vec<String> = check(seg()).unwrap().iter().map(|m| m.word.clone()).collect();
        assert_eq!(words[0], "Fehlr");
        // English words count as right only with an English dictionary.
        if languages().1.iter().any(|t| t.starts_with("en")) {
            assert_eq!(words, ["Fehlr"]);
        }
    }

    #[test]
    fn code_like_words_are_masked() {
        let words = |s: &str| -> Vec<String> {
            let m = mask_code(s);
            assert_eq!(m.chars().count(), s.chars().count());
            m.split_whitespace().map(String::from).collect()
        };
        assert_eq!(words("Rufe getUserName() mit $id auf"), ["Rufe", "mit", "auf"]);
        assert_eq!(words("siehe https://x.de oder src/main.rs, HTML und v2"), ["siehe", "oder", "und"]);
        assert_eq!(words("Ende. Neuer Satz, schön! (siehe Hinweis:)"), ["Ende.", "Neuer", "Satz,", "schön!", "(siehe", "Hinweis:)"]);
        assert_eq!(words("Ruft foo() und Foo::bar auf"), ["Ruft", "und", "auf"]);
    }

    #[test]
    fn only_prose_is_checked() {
        let line = "let x = 1; // Kommentar mit Fehlr";
        let spans = [span(0, 3, "keyword"), span(11, 34, "comment")];
        assert_eq!(segments(line, &spans, Language::Rust), [(11, "   Kommentar mit Fehlr".to_string())]);

        let md = "Text mit `code` und [Link](https://x.de)";
        let spans = [
            span(9, 15, "text.literal"),
            span(20, 21, "punctuation"),
            span(21, 25, "text.reference"),
            span(25, 27, "punctuation"),
            span(27, 39, "text.uri"),
            span(39, 40, "punctuation"),
        ];
        let segs = segments(md, &spans, Language::Markdown);
        let texts: Vec<&str> = segs.iter().map(|(_, t)| t.as_str()).collect();
        assert_eq!(texts, ["Text mit ", " und ", "Link"]);
        assert_eq!(segs[2].0, 21);

        assert_eq!(segments("Hällo Welt", &[], Language::PlainText), [(0, "Hällo Welt".to_string())]);
        assert!(segments("{\"a\": \"Fehlr\"}", &[], Language::Json).is_empty());

        assert_eq!(code_fences("a\n```js\nx\n```\nb\n~~~\n```\n~~~\n```\nc"), [1..4, 5..8, 8..usize::MAX]);
    }
}
