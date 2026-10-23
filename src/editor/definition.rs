//! "Go to definition" without a language server: CSS classes and ids used in
//! markup and scripts, functions and classes found by the outline heuristics,
//! and file paths in strings.

use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use regex::Regex;

use super::symbols::{self, Kind};
use crate::lang::Language;

/// What the cursor points at.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Target {
    /// A CSS class (`.`) or id (`#`).
    Selector { prefix: char, name: String },
    /// A function, class or similar by name.
    Symbol(String),
    /// A file path written in a string.
    Path(String),
}

/// A definition found in a file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Found {
    /// Zero-based line and char column.
    pub line: usize,
    pub column: usize,
    /// The line, trimmed, for the list of results.
    pub preview: String,
}

static CLASS_ATTR: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"(?i)(?:\bclass|\bclassName|:class|\bclass:list)\s*=\s*\{?\s*$"#).unwrap());
static ID_ATTR: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"(?i)(?:\bid\s*=\s*|getElementById\s*\(\s*)$"#).unwrap());
static PATH: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^[\w./~@-]*\w\.[A-Za-z0-9]{1,6}(?:[?#].*)?$").unwrap());

/// Identifier characters for `language`.
fn word_char(language: Language, c: char) -> bool {
    match language {
        l if l.is_css() => c.is_alphanumeric() || c == '_' || c == '-',
        Language::JavaScript | Language::TypeScript | Language::Tsx => c.is_alphanumeric() || c == '_' || c == '$',
        _ => c.is_alphanumeric() || c == '_',
    }
}

/// The run of `ok` characters around `col` in `chars`.
fn run_at(chars: &[char], col: usize, ok: impl Fn(char) -> bool) -> Option<(usize, usize)> {
    let mut start = col.min(chars.len());
    while start > 0 && ok(chars[start - 1]) {
        start -= 1;
    }
    let mut end = col.min(chars.len());
    while end < chars.len() && ok(chars[end]) {
        end += 1;
    }
    (end > start).then_some((start, end))
}

/// The quoted string around `col`: char range of its contents.
fn string_at(chars: &[char], col: usize) -> Option<(usize, usize)> {
    let mut open: Option<(char, usize)> = None;
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        match open {
            Some((q, start)) if c == q => {
                if (start..=i).contains(&col) {
                    return Some((start, i));
                }
                open = None;
            }
            Some(_) if c == '\\' => i += 1,
            None if matches!(c, '"' | '\'' | '`') => open = Some((c, i + 1)),
            _ => {}
        }
        i += 1;
    }
    None
}

/// What the cursor at char column `col` of `line` points at.
pub fn target_at(line: &str, col: usize, language: Language) -> Option<Target> {
    let chars: Vec<char> = line.chars().collect();
    if let Some((start, end)) = string_at(&chars, col) {
        let content: String = chars[start..end].iter().collect();
        let before: String = chars[..start.saturating_sub(1)].iter().collect();
        let selector = |c: char| c.is_alphanumeric() || c == '_' || c == '-';
        if CLASS_ATTR.is_match(&before) || ID_ATTR.is_match(&before) {
            let prefix = if ID_ATTR.is_match(&before) { '#' } else { '.' };
            let (s, e) = run_at(&chars[start..end], col - start, selector)?;
            return Some(Target::Selector { prefix, name: chars[start + s..start + e].iter().collect() });
        }
        let content = content.trim();
        let selector_like = content.starts_with(['.', '#']) && content[1..].starts_with(|c: char| c.is_alphabetic() || c == '_' || c == '-');
        if !selector_like && !content.contains("://") && PATH.is_match(content) {
            return Some(Target::Path(content.to_string()));
        }
        // Selectors in scripts: querySelector('.menu .item'), $('#nav').
        if let Some((s, e)) = run_at(&chars[start..end], col - start, selector)
            && s > 0
            && let prefix @ ('.' | '#') = chars[start + s - 1]
        {
            return Some(Target::Selector { prefix, name: chars[start + s..start + e].iter().collect() });
        }
    }
    if language == Language::Html || language.is_css() {
        return None;
    }
    let (s, e) = run_at(&chars, col, |c| word_char(language, c))?;
    let word: String = chars[s..e].iter().collect();
    // PHP variables are no definitions to look up.
    if word.starts_with(|c: char| c.is_ascii_digit()) || (language == Language::Php && s > 0 && chars[s - 1] == '$') {
        return None;
    }
    Some(Target::Symbol(word))
}

/// Languages whose files may define symbols used in `language`.
pub fn same_family(language: Language, other: Language) -> bool {
    use Language::*;
    let js = |l: Language| matches!(l, JavaScript | TypeScript | Tsx);
    let c = |l: Language| matches!(l, C | Cpp);
    language == other || (js(language) && js(other)) || (c(language) && c(other))
}

/// Whether files of `file_language` can hold the definition of `target` used in `language`.
pub fn searched(target: &Target, language: Language, file_language: Language) -> bool {
    match target {
        Target::Selector { .. } => file_language.is_css() || matches!(file_language, Language::Html | Language::Php),
        Target::Symbol(_) => same_family(language, file_language),
        Target::Path(_) => false,
    }
}

/// Definitions of `target` in a file's text.
pub fn find(text: &str, file_language: Language, target: &Target) -> Vec<Found> {
    let lines: Vec<&str> = text.lines().collect();
    let found = |line: usize, needle: &str| {
        let l = lines.get(line).copied().unwrap_or("");
        let column = l.find(needle).map_or(0, |b| l[..b].chars().count());
        Found { line, column, preview: l.trim().chars().take(160).collect() }
    };
    match target {
        Target::Selector { prefix, name } => {
            let token = format!("{prefix}{name}");
            let (css, style) = if file_language.is_css() {
                (std::borrow::Cow::Borrowed(text), file_language)
            } else if text.to_ascii_lowercase().contains("<style") {
                (std::borrow::Cow::Owned(style_blocks(text)), Language::Css)
            } else {
                return Vec::new();
            };
            let lines: Vec<&str> = css.lines().collect();
            symbols::symbols(&css, style)
                .into_iter()
                .filter(|s| s.kind == Kind::Rule && contains_token(&s.name, &token))
                .map(|s| {
                    // Multi-line selector lists: the line with the token.
                    let line = (s.line..lines.len()).take(20).find(|&l| contains_token(lines[l], &token)).unwrap_or(s.line);
                    found(line, &token)
                })
                .collect()
        }
        Target::Symbol(name) => symbols::symbols(text, file_language)
            .into_iter()
            .filter(|s| matches!(s.kind, Kind::Function | Kind::Class) && s.name == *name)
            .map(|s| found(s.line, name))
            .collect(),
        Target::Path(_) => Vec::new(),
    }
}

/// The contents of `<style>` elements; everything else becomes spaces, so
/// lines and columns stay where they were.
fn style_blocks(text: &str) -> String {
    let lower = text.to_ascii_lowercase();
    let mut keep = vec![false; text.len()];
    let mut at = 0;
    while let Some(open) = lower[at..].find("<style").map(|i| at + i) {
        let Some(body) = lower[open..].find('>').map(|i| open + i + 1) else { break };
        let end = lower[body..].find("</style").map_or(text.len(), |i| body + i);
        keep[body..end].iter_mut().for_each(|k| *k = true);
        at = end;
    }
    text.char_indices().map(|(i, c)| if keep[i] || c == '\n' { c } else { ' ' }).collect()
}

/// `token` (".name") in `selector`, not followed by more of a name (".name-x").
fn contains_token(selector: &str, token: &str) -> bool {
    selector.match_indices(token).any(|(i, _)| {
        !selector[i + token.len()..].starts_with(|c: char| c.is_alphanumeric() || c == '_' || c == '-')
    })
}

/// The file a path in a string refers to: relative to the document's folder,
/// or from the project root for paths starting with `/` (and as a fallback).
pub fn resolve_path(path: &str, doc_dir: Option<&Path>, root: Option<&Path>) -> Option<PathBuf> {
    let clean = path.split(['?', '#']).next().unwrap_or(path).replace('/', "\\");
    let from_root = clean.starts_with('\\');
    let rel = clean.trim_start_matches('\\');
    let mut candidates = Vec::new();
    if !from_root && let Some(d) = doc_dir {
        candidates.push(d.join(rel));
    }
    if let Some(r) = root {
        candidates.push(r.join(rel));
    }
    candidates.into_iter().find(|p| p.is_file())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(line: &str, marker: &str, language: Language) -> Option<Target> {
        let col = line[..line.find(marker).unwrap()].chars().count() + 1;
        target_at(line, col, language)
    }

    fn sel(prefix: char, name: &str) -> Option<Target> {
        Some(Target::Selector { prefix, name: name.into() })
    }

    #[test]
    fn targets() {
        let html = r#"<div class="card card-wide" id="main"><a href="css/site.css?v=2">"#;
        assert_eq!(at(html, "ard-wide", Language::Html), sel('.', "card-wide"));
        assert_eq!(at(html, "ain", Language::Html), sel('#', "main"));
        assert_eq!(at(html, "ss/site", Language::Html), Some(Target::Path("css/site.css?v=2".into())));
        assert_eq!(at("<div>text</div>", "ext", Language::Html), None);
        let js = "document.querySelector('.menu .item'); el.className = 'open'; getElementById(\"nav\")";
        assert_eq!(at(js, "tem", Language::JavaScript), sel('.', "item"));
        assert_eq!(at(js, "pen", Language::JavaScript), sel('.', "open"));
        assert_eq!(at(js, "av", Language::JavaScript), sel('#', "nav"));
        assert_eq!(at("const x = $helper(1);", "elper", Language::JavaScript), Some(Target::Symbol("$helper".into())));
        let php = "<?php require_once 'lib/db.php'; $total = calc_total($items); ?>";
        assert_eq!(at(php, "ib/", Language::Php), Some(Target::Path("lib/db.php".into())));
        assert_eq!(at(php, "alc_", Language::Php), Some(Target::Symbol("calc_total".into())));
        assert_eq!(at(php, "otal =", Language::Php), None);
        assert_eq!(at("$this->save();", "ave", Language::Php), Some(Target::Symbol("save".into())));
    }

    #[test]
    fn finds_selectors_and_symbols() {
        let css = ".card, .card-wide {\n  margin: 0;\n}\n.list,\n.card:hover > .x { color: red; }\n#main { }\n";
        let card = Target::Selector { prefix: '.', name: "card".into() };
        let lines: Vec<usize> = find(css, Language::Css, &card).iter().map(|f| f.line).collect();
        assert_eq!(lines, [0, 4]);
        assert_eq!(find(css, Language::Css, &card)[1].column, 0);
        let main = Target::Selector { prefix: '#', name: "main".into() };
        assert_eq!(find(css, Language::Css, &main)[0].line, 5);

        let php = "<?php\nfunction calc_total($items) {\n  return 1;\n}\nclass Cart {\n  public function save() {}\n}\n";
        assert_eq!(find(php, Language::Php, &Target::Symbol("calc_total".into()))[0].line, 1);
        let save = &find(php, Language::Php, &Target::Symbol("save".into()))[0];
        assert_eq!((save.line, save.column, save.preview.as_str()), (5, 18, "public function save() {}"));
        assert!(find(php, Language::Php, &Target::Symbol("calc".into())).is_empty());

        let html = "<html><head>\n<STYLE media=\"x\">\n  .hero { color: red; }\n</style>\n</head><body class=\"hero\">ä {.hero}</body>\n";
        let hero = Target::Selector { prefix: '.', name: "hero".into() };
        let f = find(html, Language::Html, &hero);
        assert_eq!(f.len(), 1);
        assert_eq!((f[0].line, f[0].column, f[0].preview.as_str()), (2, 2, ".hero { color: red; }"));
    }
}
