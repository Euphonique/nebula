//! Background syntax highlighting with tree-sitter.
//!
//! Documents send their full text to a worker thread whenever they change.
//! The worker keeps only the newest request per document, highlights it, and
//! sends back per-line style spans. The editor keeps drawing the previous
//! spans until fresh ones arrive, so typing never waits on the parser.

use std::collections::HashMap;
use std::sync::Arc;
use std::thread;

use crossbeam_channel::{Receiver, Sender};
use eframe::egui;
use tree_sitter_highlight::{HighlightConfiguration, HighlightEvent, Highlighter};

use crate::lang::Language;
use crate::theme::{HIGHLIGHT_NAMES, PLAIN};

/// Files larger than this are shown without syntax colors.
const MAX_HIGHLIGHT_BYTES: usize = 4 * 1024 * 1024;

#[derive(Clone, Copy, Debug)]
pub struct Span {
    /// Byte range within the line.
    pub start: u32,
    pub end: u32,
    pub style: u8,
}

pub type LineSpans = Vec<Vec<Span>>;

struct Job {
    doc_id: u64,
    version: u64,
    language: Language,
    text: Arc<String>,
}

pub struct HighlightResult {
    pub doc_id: u64,
    pub version: u64,
    pub lines: Arc<LineSpans>,
}

pub struct HighlightService {
    jobs: Sender<Job>,
    results: Receiver<HighlightResult>,
}

impl HighlightService {
    pub fn spawn(ctx: egui::Context) -> Self {
        let (job_tx, job_rx) = crossbeam_channel::unbounded::<Job>();
        let (res_tx, res_rx) = crossbeam_channel::unbounded();
        thread::Builder::new()
            .name("highlighter".into())
            .spawn(move || worker(job_rx, res_tx, ctx))
            .expect("spawn highlighter thread");
        Self {
            jobs: job_tx,
            results: res_rx,
        }
    }

    pub fn request(&self, doc_id: u64, version: u64, language: Language, text: Arc<String>) {
        let _ = self.jobs.send(Job {
            doc_id,
            version,
            language,
            text,
        });
    }

    pub fn poll(&self) -> impl Iterator<Item = HighlightResult> + '_ {
        self.results.try_iter()
    }
}

/// Highlight configurations, loaded on first use per grammar.
struct Configs {
    loaded: HashMap<&'static str, Option<HighlightConfiguration>>,
}

/// Grammar used for a language (several languages share one).
fn grammar_of(language: Language) -> Option<&'static str> {
    Some(match language {
        Language::Html => "html",
        Language::Css | Language::Scss | Language::Less => "css",
        Language::JavaScript => "javascript",
        Language::TypeScript => "typescript",
        Language::Tsx => "tsx",
        Language::Php => "php",
        Language::Json => "json",
        Language::Markdown => "markdown",
        Language::Xml => "xml",
        Language::Python => "python",
        Language::Ruby => "ruby",
        Language::Rust => "rust",
        Language::Go => "go",
        Language::Java => "java",
        Language::C => "c",
        Language::Cpp => "cpp",
        Language::CSharp => "csharp",
        Language::Shell => "bash",
        Language::Sql => "sql",
        Language::Lua => "lua",
        Language::Yaml => "yaml",
        Language::Toml => "toml",
        Language::Ini => "ini",
        Language::Diff => "diff",
        Language::PlainText => return None,
    })
}

/// Grammar for an injection name (e.g. a Markdown code fence info string).
fn grammar_for_injection(name: &str) -> Option<&'static str> {
    Some(match name.to_ascii_lowercase().as_str() {
        "html" | "htm" | "vue" => "html",
        "css" | "scss" | "less" | "sass" => "css",
        "javascript" | "js" | "jsx" | "mjs" | "node" => "javascript",
        "typescript" | "ts" => "typescript",
        "tsx" => "tsx",
        "php" => "php",
        "json" | "jsonc" => "json",
        "markdown" | "md" => "markdown",
        "markdown_inline" => "markdown_inline",
        "xml" | "svg" => "xml",
        "python" | "py" => "python",
        "ruby" | "rb" => "ruby",
        "rust" | "rs" => "rust",
        "go" | "golang" => "go",
        "java" => "java",
        "c" | "h" => "c",
        "cpp" | "c++" | "cc" | "hpp" => "cpp",
        "c_sharp" | "csharp" | "cs" | "c#" => "csharp",
        "bash" | "sh" | "shell" | "zsh" | "console" => "bash",
        "sql" | "mysql" | "postgresql" => "sql",
        "lua" => "lua",
        "yaml" | "yml" => "yaml",
        "toml" => "toml",
        "ini" | "conf" => "ini",
        "diff" | "patch" => "diff",
        _ => return None,
    })
}

/// Grammars that a grammar injects and therefore must be loaded with it.
fn dependencies(grammar: &str) -> &'static [&'static str] {
    match grammar {
        "html" => &["css", "javascript"],
        "php" => &["html", "css", "javascript"],
        "markdown" => &["markdown_inline", "html"],
        _ => &[],
    }
}

/// Language and (highlights, injections, locals) queries for a grammar.
fn grammar_source(grammar: &str) -> Option<(tree_sitter::Language, String, String, String)> {
    let s = |q: &str| q.to_string();
    let js_highlights = format!(
        "{}\n{}",
        tree_sitter_javascript::HIGHLIGHT_QUERY,
        tree_sitter_javascript::JSX_HIGHLIGHT_QUERY
    );
    Some(match grammar {
        "html" => (
            tree_sitter_html::LANGUAGE.into(),
            s(include_str!("queries/html.scm")),
            s(tree_sitter_html::INJECTIONS_QUERY),
            String::new(),
        ),
        "css" => (tree_sitter_css::LANGUAGE.into(), s(include_str!("queries/css.scm")), String::new(), String::new()),
        "javascript" => (
            tree_sitter_javascript::LANGUAGE.into(),
            js_highlights,
            s(tree_sitter_javascript::INJECTIONS_QUERY),
            s(tree_sitter_javascript::LOCALS_QUERY),
        ),
        // TypeScript's queries extend the JavaScript ones.
        "typescript" => (
            tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
            format!("{}\n{}", tree_sitter_typescript::HIGHLIGHTS_QUERY, tree_sitter_javascript::HIGHLIGHT_QUERY),
            String::new(),
            s(tree_sitter_typescript::LOCALS_QUERY),
        ),
        "tsx" => (
            tree_sitter_typescript::LANGUAGE_TSX.into(),
            format!("{}\n{}", tree_sitter_typescript::HIGHLIGHTS_QUERY, js_highlights),
            String::new(),
            s(tree_sitter_typescript::LOCALS_QUERY),
        ),
        "php" => (
            tree_sitter_php::LANGUAGE_PHP.into(),
            s(tree_sitter_php::HIGHLIGHTS_QUERY),
            // The bundled PHP injections omit the HTML around `<?php ... ?>`.
            format!(
                "{}\n((text) @injection.content (#set! injection.language \"html\") (#set! injection.combined))\n",
                tree_sitter_php::INJECTIONS_QUERY
            ),
            String::new(),
        ),
        "json" => (tree_sitter_json::LANGUAGE.into(), s(tree_sitter_json::HIGHLIGHTS_QUERY), String::new(), String::new()),
        "markdown" => (
            tree_sitter_md::LANGUAGE.into(),
            s(tree_sitter_md::HIGHLIGHT_QUERY_BLOCK),
            s(tree_sitter_md::INJECTION_QUERY_BLOCK),
            String::new(),
        ),
        "markdown_inline" => (
            tree_sitter_md::INLINE_LANGUAGE.into(),
            s(tree_sitter_md::HIGHLIGHT_QUERY_INLINE),
            s(tree_sitter_md::INJECTION_QUERY_INLINE),
            String::new(),
        ),
        "xml" => (tree_sitter_xml::LANGUAGE_XML.into(), s(tree_sitter_xml::XML_HIGHLIGHT_QUERY), String::new(), String::new()),
        "python" => (tree_sitter_python::LANGUAGE.into(), s(tree_sitter_python::HIGHLIGHTS_QUERY), String::new(), String::new()),
        "ruby" => (
            tree_sitter_ruby::LANGUAGE.into(),
            s(tree_sitter_ruby::HIGHLIGHTS_QUERY),
            String::new(),
            s(tree_sitter_ruby::LOCALS_QUERY),
        ),
        "rust" => (
            tree_sitter_rust::LANGUAGE.into(),
            s(tree_sitter_rust::HIGHLIGHTS_QUERY),
            s(tree_sitter_rust::INJECTIONS_QUERY),
            String::new(),
        ),
        "go" => (tree_sitter_go::LANGUAGE.into(), s(tree_sitter_go::HIGHLIGHTS_QUERY), String::new(), String::new()),
        "java" => (tree_sitter_java::LANGUAGE.into(), s(tree_sitter_java::HIGHLIGHTS_QUERY), String::new(), String::new()),
        "c" => (tree_sitter_c::LANGUAGE.into(), s(tree_sitter_c::HIGHLIGHT_QUERY), String::new(), String::new()),
        // C++ extends the C queries.
        "cpp" => (
            tree_sitter_cpp::LANGUAGE.into(),
            format!("{}\n{}", tree_sitter_cpp::HIGHLIGHT_QUERY, tree_sitter_c::HIGHLIGHT_QUERY),
            String::new(),
            String::new(),
        ),
        "csharp" => (
            tree_sitter_c_sharp::LANGUAGE.into(),
            s(tree_sitter_c_sharp::HIGHLIGHTS_QUERY),
            String::new(),
            String::new(),
        ),
        "bash" => (tree_sitter_bash::LANGUAGE.into(), s(tree_sitter_bash::HIGHLIGHT_QUERY), String::new(), String::new()),
        "sql" => (tree_sitter_sequel::LANGUAGE.into(), s(tree_sitter_sequel::HIGHLIGHTS_QUERY), String::new(), String::new()),
        "lua" => (
            tree_sitter_lua::LANGUAGE.into(),
            s(tree_sitter_lua::HIGHLIGHTS_QUERY),
            s(tree_sitter_lua::INJECTIONS_QUERY),
            s(tree_sitter_lua::LOCALS_QUERY),
        ),
        "yaml" => (tree_sitter_yaml::LANGUAGE.into(), s(tree_sitter_yaml::HIGHLIGHTS_QUERY), String::new(), String::new()),
        "toml" => (tree_sitter_toml_ng::LANGUAGE.into(), s(tree_sitter_toml_ng::HIGHLIGHTS_QUERY), String::new(), String::new()),
        "ini" => (tree_sitter_ini::LANGUAGE.into(), s(tree_sitter_ini::HIGHLIGHTS_QUERY), String::new(), String::new()),
        "diff" => (tree_sitter_diff::LANGUAGE.into(), s(include_str!("queries/diff.scm")), String::new(), String::new()),
        _ => return None,
    })
}

impl Configs {
    fn new() -> Self {
        Self { loaded: HashMap::new() }
    }

    /// Loads a grammar and everything it injects.
    fn ensure(&mut self, grammar: &'static str) {
        if self.loaded.contains_key(grammar) {
            return;
        }
        let config = grammar_source(grammar).and_then(|(lang, highlights, injections, locals)| {
            match HighlightConfiguration::new(lang, grammar, &highlights, &injections, &locals) {
                Ok(mut c) => {
                    c.configure(HIGHLIGHT_NAMES);
                    Some(c)
                }
                Err(e) => {
                    eprintln!("Highlighting für {grammar} nicht verfügbar: {e}");
                    None
                }
            }
        });
        self.loaded.insert(grammar, config);
        for dep in dependencies(grammar) {
            self.ensure(dep);
        }
    }

    /// Loads what `text` in `language` needs, including Markdown code fences.
    fn prepare(&mut self, language: Language, text: &str) {
        let Some(grammar) = grammar_of(language) else { return };
        self.ensure(grammar);
        if grammar == "markdown" {
            for line in text.lines() {
                let t = line.trim_start();
                if let Some(info) = t.strip_prefix("```").or_else(|| t.strip_prefix("~~~"))
                    && let Some(g) = info.split_whitespace().next().and_then(grammar_for_injection)
                {
                    self.ensure(g);
                }
            }
        }
    }

    fn get(&self, grammar: &str) -> Option<&HighlightConfiguration> {
        self.loaded.get(grammar).and_then(|c| c.as_ref())
    }

    fn for_language(&self, language: Language) -> Option<&HighlightConfiguration> {
        self.get(grammar_of(language)?)
    }

    fn for_injection(&self, name: &str) -> Option<&HighlightConfiguration> {
        self.get(grammar_for_injection(name)?)
    }
}

fn worker(jobs: Receiver<Job>, results: Sender<HighlightResult>, ctx: egui::Context) {
    let mut configs = Configs::new();
    let mut highlighter = Highlighter::new();

    while let Ok(first) = jobs.recv() {
        // Coalesce: only the newest request per document matters.
        let mut pending: HashMap<u64, Job> = HashMap::new();
        pending.insert(first.doc_id, first);
        for job in jobs.try_iter() {
            pending.insert(job.doc_id, job);
        }

        for job in pending.into_values() {
            configs.prepare(job.language, &job.text);
            let lines = highlight(&configs, &mut highlighter, job.language, &job.text);
            if results
                .send(HighlightResult {
                    doc_id: job.doc_id,
                    version: job.version,
                    lines: Arc::new(lines),
                })
                .is_err()
            {
                return;
            }
        }
        ctx.request_repaint();
    }
}

fn highlight(
    configs: &Configs,
    highlighter: &mut Highlighter,
    language: Language,
    text: &str,
) -> LineSpans {
    let mut lines: LineSpans = vec![Vec::new()];
    if text.len() > MAX_HIGHLIGHT_BYTES {
        return lines;
    }
    let Some(config) = configs.for_language(language) else {
        return lines;
    };
    let Ok(events) = highlighter.highlight(config, text.as_bytes(), None, None, |name| {
        configs.for_injection(name)
    }) else {
        return lines;
    };

    let bytes = text.as_bytes();
    let mut stack: Vec<u8> = Vec::new();
    let mut line_start = 0usize;

    for event in events {
        let Ok(event) = event else { break };
        match event {
            HighlightEvent::HighlightStart(h) => stack.push(h.0 as u8),
            HighlightEvent::HighlightEnd => {
                stack.pop();
            }
            HighlightEvent::Source { start, end } => {
                let style = stack.last().copied().unwrap_or(PLAIN);
                let mut pos = start;
                while pos < end {
                    let newline = bytes[pos..end].iter().position(|&b| b == b'\n');
                    let piece_end = newline.map_or(end, |i| pos + i);
                    if style != PLAIN && piece_end > pos {
                        let line = lines.last_mut().unwrap();
                        let s = (pos - line_start) as u32;
                        let e = (piece_end - line_start) as u32;
                        // Merge with the previous span when contiguous and equal.
                        match line.last_mut() {
                            Some(last) if last.end == s && last.style == style => last.end = e,
                            _ => line.push(Span {
                                start: s,
                                end: e,
                                style,
                            }),
                        }
                    }
                    match newline {
                        Some(_) => {
                            pos = piece_end + 1;
                            line_start = pos;
                            lines.push(Vec::new());
                        }
                        None => pos = piece_end,
                    }
                }
            }
        }
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    fn styles(lines: &LineSpans) -> Vec<&'static str> {
        lines
            .iter()
            .flatten()
            .map(|s| HIGHLIGHT_NAMES[s.style as usize])
            .collect()
    }

    #[test]
    fn all_languages_and_injections() {
        let mut configs = Configs::new();
        for l in Language::ALL {
            configs.prepare(l, "");
        }
        let mut h = Highlighter::new();

        let html = "<!DOCTYPE html><style>a { color: red; }</style>\n<script>let x = 1;</script>\n<p class=\"a\">Hi</p><a href=\"/x\">y</a>";
        let lines = highlight(&configs, &mut h, Language::Html, html);
        assert_eq!(lines.len(), 3);
        let s = styles(&lines);
        assert!(s.contains(&"tag.name"));
        assert!(s.contains(&"style.attribute.name"), "CSS injected into <style>: {s:?}");
        assert!(s.contains(&"tag.attribute.value.delimiter"), "{s:?}");
        for expected in ["tag.attribute.name", "tag.attribute.value", "tag.attribute.value.link", "tag.attribute.operator", "processing"] {
            assert!(s.contains(&expected), "{expected} missing in {s:?}");
        }
        assert!(s.contains(&"keyword"), "JS injected into <script>: {s:?}");

        let php = "<div><?php echo $x; ?></div>";
        let s = styles(&highlight(&configs, &mut h, Language::Php, php));
        assert!(s.contains(&"tag.name"), "HTML around PHP: {s:?}");
        assert!(s.contains(&"variable") || s.contains(&"keyword"), "{s:?}");

        let s = styles(&highlight(&configs, &mut h, Language::Css, "#top a.btn:hover { color: #fff; width: 10px; display: flex }"));
        for expected in [
            "style.selector.id",
            "style.selector.element",
            "style.selector.class",
            "style.selector.pseudoclass",
            "style.attribute.name",
            "style.value.color.hex",
            "style.value.number.unit",
            "style.value.keyword",
        ] {
            assert!(s.contains(&expected), "{expected} missing in {s:?}");
        }

        for (lang, src) in [(Language::Css, "a{b:1px}"), (Language::JavaScript, "const a = `x`;"), (Language::Json, "{\"a\": 1}")] {
            assert!(!styles(&highlight(&configs, &mut h, lang, src)).is_empty(), "{lang:?}");
        }
    }
}

#[cfg(test)]
mod language_tests {
    use super::*;

    #[test]
    fn every_language_highlights_sample_code() {
        let samples: &[(Language, &str)] = &[
            (Language::TypeScript, "interface A { x: number }\nconst a: A = { x: 1 };"),
            (Language::Tsx, "const App = () => <div className=\"a\">{x}</div>;"),
            (Language::Markdown, "# Titel\n\nText mit **fett** und `code`.\n\n```css\na { color: red; }\n```\n"),
            (Language::Xml, "<?xml version=\"1.0\"?>\n<root a=\"1\"><!-- c --><b/></root>"),
            (Language::Python, "def f(x):\n    return x + 1  # comment\n"),
            (Language::Ruby, "def f(x)\n  puts \"hi\"\nend\n"),
            (Language::Rust, "fn main() { let x: u32 = 1; println!(\"{x}\"); }"),
            (Language::Go, "package main\nfunc main() { x := 1 }"),
            (Language::Java, "class A { void f() { int x = 1; } }"),
            (Language::C, "#include <stdio.h>\nint main(void) { return 0; }"),
            (Language::Cpp, "template<typename T> class A { public: T x; };"),
            (Language::CSharp, "class A { public void F() { var x = 1; } }"),
            (Language::Shell, "#!/bin/bash\nfor f in *.txt; do echo \"$f\"; done"),
            (Language::Sql, "SELECT id, name FROM users WHERE id = 1;"),
            (Language::Lua, "local function f(x) return x + 1 end"),
            (Language::Yaml, "key: value\nlist:\n  - 1\n"),
            (Language::Toml, "[package]\nname = \"x\"\nversion = 1"),
            (Language::Ini, "[section]\nkey=value\n; comment"),
            (Language::Diff, "--- a\n+++ b\n@@ -1 +1 @@\n-old\n+new\n"),
            (Language::Scss, ".a { .b { color: red; } }"),
        ];
        let mut configs = Configs::new();
        let mut h = Highlighter::new();
        for (lang, src) in samples {
            configs.prepare(*lang, src);
            assert!(configs.for_language(*lang).is_some(), "{lang:?}: grammar failed to load");
            let spans = highlight(&configs, &mut h, *lang, src);
            assert!(spans.iter().flatten().count() > 0, "{lang:?}: no highlights");
        }
        // Code fences in Markdown get their own grammar.
        assert!(configs.get("css").is_some());
    }
}
