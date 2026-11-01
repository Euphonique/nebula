//! Symbols of a whole document for the outline: functions, classes,
//! CSS rules, headings. Line-based heuristics per language, no parser.

use std::sync::LazyLock;

use regex::Regex;

use crate::lang::Language;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Function,
    Class,
    Rule,
    Heading,
    Section,
    Element,
}

impl Kind {
    /// Short badge shown before the name.
    pub fn badge(self) -> &'static str {
        match self {
            Kind::Function => t!("ƒ"),
            Kind::Class => "C",
            Kind::Rule => "{}",
            Kind::Heading => "H",
            Kind::Section => "§",
            Kind::Element => "<>",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Symbol {
    pub name: String,
    pub kind: Kind,
    /// Zero-based line.
    pub line: usize,
    /// Nesting level for indentation.
    pub depth: usize,
}

fn re(s: &str) -> Regex {
    Regex::new(s).unwrap()
}

pub fn symbols(text: &str, language: Language) -> Vec<Symbol> {
    use Language::*;
    match language {
        Markdown => markdown(text),
        Css | Scss | Less => css(text),
        Html => html(text),
        Xml => xml(text),
        Json => json(text),
        Sql => sql(text),
        Diff => diff(text),
        Php => {
            let mut s = c_like(text, &PHP);
            s.extend(html(text));
            s.sort_by_key(|s| s.line);
            s
        }
        JavaScript | TypeScript | Tsx => c_like(text, &JS),
        Rust => c_like(text, &RUST),
        Go => c_like(text, &GO),
        Java | CSharp | C | Cpp => c_like(text, &CLIKE),
        Shell => c_like(text, &SHELL),
        Python => indented(text, &PYTHON),
        Ruby | Lua => indented(text, &RUBY_LUA),
        Toml | Ini => sections(text),
        Yaml => yaml(text),
        _ => Vec::new(),
    }
}

/// Patterns with the name in group `n`; the kind is chosen by group `k` if present.
struct Rules {
    patterns: Vec<(Regex, Kind)>,
}

static PHP: LazyLock<Rules> = LazyLock::new(|| Rules {
    patterns: vec![
        (re(r"^\s*(?:abstract\s+|final\s+|readonly\s+)*(?:class|interface|trait|enum)\s+(?P<n>\w+)"), Kind::Class),
        (re(r"^\s*(?:(?:public|private|protected|static|abstract|final)\s+)*function\s+&?(?P<n>\w+)\s*\("), Kind::Function),
    ],
});

static JS: LazyLock<Rules> = LazyLock::new(|| Rules {
    patterns: vec![
        (re(r"^\s*(?:export\s+)?(?:default\s+)?(?:abstract\s+)?(?:class|interface|enum)\s+(?P<n>[\w$]+)"), Kind::Class),
        (re(r"^\s*(?:export\s+)?(?:default\s+)?(?:async\s+)?function\s*\*?\s*(?P<n>[\w$]+)\s*[(<]"), Kind::Function),
        (re(r"^\s*(?:export\s+)?(?:const|let|var)\s+(?P<n>[\w$]+)\s*(?::[^=]+)?=\s*(?:async\s+)?(?:function\b|\([^)]*\)\s*(?::[^=]+)?=>|[\w$]+\s*=>)"), Kind::Function),
        (re(r"^\s*(?:(?:public|private|protected|static|async|get|set|override|readonly)\s+)*(?P<n>[\w$]+)\s*\([^)]*\)\s*(?::\s*[^{]+)?\{\s*$"), Kind::Function),
    ],
});

static RUST: LazyLock<Rules> = LazyLock::new(|| Rules {
    patterns: vec![
        (re(r"^\s*(?:pub(?:\([^)]*\))?\s+)?(?:struct|enum|trait|union|type)\s+(?P<n>\w+)"), Kind::Class),
        (re(r"^\s*impl(?:<[^>]*>)?\s+(?P<n>[^{]+?)\s*(?:where.*)?\{?\s*$"), Kind::Class),
        (re(r"^\s*(?:pub(?:\([^)]*\))?\s+)?mod\s+(?P<n>\w+)"), Kind::Section),
        (re(r"^\s*(?:pub(?:\([^)]*\))?\s+)?(?:const\s+)?(?:async\s+)?(?:unsafe\s+)?(?:extern\s+\S+\s+)?fn\s+(?P<n>\w+)"), Kind::Function),
    ],
});

static GO: LazyLock<Rules> = LazyLock::new(|| Rules {
    patterns: vec![
        (re(r"^\s*type\s+(?P<n>\w+)\s+(?:struct|interface)"), Kind::Class),
        (re(r"^\s*func\s+(?:\([^)]*\)\s*)?(?P<n>\w+)\s*[(\[]"), Kind::Function),
    ],
});

static CLIKE: LazyLock<Rules> = LazyLock::new(|| Rules {
    patterns: vec![
        (re(r"^\s*(?:(?:public|private|protected|internal|static|abstract|sealed|partial|final|export)\s+)*(?:class|struct|interface|enum|record|namespace)\s+(?P<n>[\w.:]+)"), Kind::Class),
        (re(r"^\s*(?:[\w<>\[\],*&:~]+\s+)+\**&?(?P<n>~?[A-Za-z_][\w:]*)\s*\([^;]*\)\s*(?:const\s*)?(?:override\s*)?(?:noexcept\s*)?(?:->\s*[\w:<>]+\s*)?(?:\{.*)?$"), Kind::Function),
    ],
});

static SHELL: LazyLock<Rules> = LazyLock::new(|| Rules {
    patterns: vec![(re(r"^\s*(?:function\s+)?(?P<n>[\w-]+)\s*\(\)\s*\{?"), Kind::Function), (re(r"^\s*function\s+(?P<n>[\w-]+)"), Kind::Function)],
});

static PYTHON: LazyLock<Rules> = LazyLock::new(|| Rules {
    patterns: vec![(re(r"^\s*class\s+(?P<n>\w+)"), Kind::Class), (re(r"^\s*(?:async\s+)?def\s+(?P<n>\w+)"), Kind::Function)],
});

static RUBY_LUA: LazyLock<Rules> = LazyLock::new(|| Rules {
    patterns: vec![
        (re(r"^\s*(?:class|module)\s+(?P<n>[\w:]+)"), Kind::Class),
        (re(r"^\s*def\s+(?P<n>[\w.?!=]+)"), Kind::Function),
        (re(r"^\s*(?:local\s+)?function\s+(?P<n>[\w.:]+)"), Kind::Function),
    ],
});

const KEYWORDS: [&str; 12] = ["if", "for", "while", "switch", "catch", "return", "else", "do", "try", "with", "new", "sizeof"];

/// Brace languages: depth from `{`/`}` before the line (strings and comments are ignored roughly).
fn c_like(text: &str, rules: &Rules) -> Vec<Symbol> {
    let mut out = Vec::new();
    let mut depth: usize = 0;
    // Symbols whose body may still be open: (brace depth at their line, body entered).
    let mut open: Vec<(usize, bool)> = Vec::new();
    let mut in_comment = false;
    for (line_no, line) in text.lines().enumerate() {
        let trimmed = line.trim_start();
        let code = strip_comments(line, &mut in_comment);
        if !trimmed.starts_with("//") && !trimmed.starts_with('*') && !trimmed.starts_with('#') {
            for (pattern, kind) in &rules.patterns {
                let Some(c) = pattern.captures(&code) else { continue };
                let name = c.name("n").map(|m| m.as_str().trim()).unwrap_or_default();
                if name.is_empty() || KEYWORDS.contains(&name) {
                    continue;
                }
                // A previous symbol at this level without a body (declaration) is no parent.
                while open.last().is_some_and(|&(d, entered)| !entered && d >= depth) {
                    open.pop();
                }
                out.push(Symbol { name: name.to_string(), kind: *kind, line: line_no, depth: open.len() });
                open.push((depth, false));
                break;
            }
        }
        for ch in code.chars() {
            match ch {
                '{' => {
                    depth += 1;
                    if let Some(top) = open.last_mut()
                        && depth == top.0 + 1
                    {
                        top.1 = true;
                    }
                }
                '}' => {
                    depth = depth.saturating_sub(1);
                    while open.last().is_some_and(|&(d, entered)| entered && depth <= d) {
                        open.pop();
                    }
                }
                _ => {}
            }
        }
    }
    out
}

/// Removes `//` and `/* */` comments and string contents (so braces in them don't count).
fn strip_comments(line: &str, in_comment: &mut bool) -> String {
    let mut out = String::with_capacity(line.len());
    let mut chars = line.chars().peekable();
    let mut quote: Option<char> = None;
    while let Some(c) = chars.next() {
        if *in_comment {
            if c == '*' && chars.peek() == Some(&'/') {
                chars.next();
                *in_comment = false;
            }
            continue;
        }
        if let Some(q) = quote {
            if c == '\\' {
                chars.next();
            } else if c == q {
                quote = None;
                out.push(c);
            }
            continue;
        }
        match c {
            '/' if chars.peek() == Some(&'/') => break,
            '/' if chars.peek() == Some(&'*') => {
                chars.next();
                *in_comment = true;
            }
            '"' | '\'' | '`' => {
                quote = Some(c);
                out.push(c);
            }
            _ => out.push(c),
        }
    }
    out
}

/// Indentation languages (Python, Ruby, Lua): depth from the indentation of enclosing symbols.
fn indented(text: &str, rules: &Rules) -> Vec<Symbol> {
    let mut out = Vec::new();
    let mut stack: Vec<usize> = Vec::new();
    for (line_no, line) in text.lines().enumerate() {
        for (pattern, kind) in &rules.patterns {
            if let Some(c) = pattern.captures(line) {
                let indent = line.len() - line.trim_start().len();
                while stack.last().is_some_and(|&i| i >= indent) {
                    stack.pop();
                }
                out.push(Symbol { name: c["n"].to_string(), kind: *kind, line: line_no, depth: stack.len() });
                stack.push(indent);
                break;
            }
        }
    }
    out
}

fn markdown(text: &str) -> Vec<Symbol> {
    static H: LazyLock<Regex> = LazyLock::new(|| re(r"^(#{1,6})\s+(.+?)\s*#*\s*$"));
    let mut out = Vec::new();
    let mut fence = false;
    for (line_no, line) in text.lines().enumerate() {
        if line.trim_start().starts_with("```") || line.trim_start().starts_with("~~~") {
            fence = !fence;
            continue;
        }
        if !fence && let Some(c) = H.captures(line) {
            out.push(Symbol { name: c[2].to_string(), kind: Kind::Heading, line: line_no, depth: c[1].len() - 1 });
        }
    }
    out
}

/// CSS rules and at-rules with their nesting.
fn css(text: &str) -> Vec<Symbol> {
    let mut out = Vec::new();
    let mut depth = 0usize;
    let mut selector = String::new();
    let mut start_line = None;
    let mut in_comment = false;
    for (line_no, line) in text.lines().enumerate() {
        let code = strip_comments(line, &mut in_comment);
        for ch in code.chars() {
            match ch {
                '{' => {
                    let name = selector.split_whitespace().collect::<Vec<_>>().join(" ");
                    if !name.is_empty() {
                        let kind = if name.starts_with('@') { Kind::Section } else { Kind::Rule };
                        out.push(Symbol { name, kind, line: start_line.unwrap_or(line_no), depth });
                    }
                    depth += 1;
                    selector.clear();
                    start_line = None;
                }
                '}' => {
                    depth = depth.saturating_sub(1);
                    selector.clear();
                    start_line = None;
                }
                ';' => {
                    selector.clear();
                    start_line = None;
                }
                c => {
                    if !c.is_whitespace() && start_line.is_none() {
                        start_line = Some(line_no);
                    }
                    selector.push(c);
                }
            }
        }
        selector.push(' ');
    }
    out
}

/// HTML: headings and elements with an id.
fn html(text: &str) -> Vec<Symbol> {
    static HEADING: LazyLock<Regex> = LazyLock::new(|| re(r"(?is)<h([1-6])\b[^>]*>(.*?)</h[1-6]>"));
    static ID: LazyLock<Regex> = LazyLock::new(|| re(r#"(?i)<([a-z][\w-]*)\b[^>]*?\bid\s*=\s*["']([^"']+)["']"#));
    static TAGS: LazyLock<Regex> = LazyLock::new(|| re(r"<[^>]*>"));
    let line_of = |pos: usize| text[..pos].matches('\n').count();
    let mut out = Vec::new();
    for c in HEADING.captures_iter(text) {
        let name = TAGS.replace_all(&c[2], "").split_whitespace().collect::<Vec<_>>().join(" ");
        if !name.is_empty() {
            let level: usize = c[1].parse().unwrap_or(1);
            out.push(Symbol { name, kind: Kind::Heading, line: line_of(c.get(0).unwrap().start()), depth: level - 1 });
        }
    }
    // CSS rules inside <style> blocks.
    static STYLE: LazyLock<Regex> = LazyLock::new(|| re(r"(?is)<style\b[^>]*>(.*?)</style>"));
    for c in STYLE.captures_iter(text) {
        let inner = c.get(1).unwrap();
        let offset = line_of(inner.start());
        out.extend(css(inner.as_str()).into_iter().map(|s| Symbol { line: s.line + offset, ..s }));
    }
    for c in ID.captures_iter(text) {
        if c[1].len() == 2 && c[1].to_ascii_lowercase().starts_with('h') {
            continue; // headings are listed already
        }
        out.push(Symbol { name: format!("{}#{}", c[1].to_ascii_lowercase(), &c[2]), kind: Kind::Element, line: line_of(c.get(0).unwrap().start()), depth: 0 });
    }
    out.sort_by_key(|s| s.line);
    out
}

/// Most entries of the element/key trees (large data files stay usable).
const MAX_TREE: usize = 3000;
/// Deeper levels are left out of the element/key trees.
const MAX_TREE_DEPTH: usize = 6;

/// XML: the element tree; an element is named by its tag and a telling
/// attribute (`item · Start` for `<item name="Start">`).
fn xml(text: &str) -> Vec<Symbol> {
    const NAMED_BY: &[&str] = &["id", "name", "key", "title", "label", "type", "class", "ref", "href", "path", "Include", "Name", "Key", "Id"];
    let b = text.as_bytes();
    let mut out = Vec::new();
    let mut depth = 0usize;
    let mut line = 0usize;
    let mut counted = 0usize;
    let mut i = 0;
    while let Some(off) = text[i..].find('<') {
        let at = i + off;
        line += text[counted..at].matches('\n').count();
        counted = at;
        let rest = &text[at..];
        let skip = |pat: &str| rest.find(pat).map_or(text.len(), |e| at + e + pat.len());
        if rest.starts_with("<!--") {
            i = skip("-->");
        } else if rest.starts_with("<![CDATA[") {
            i = skip("]]>");
        } else if rest.starts_with("<?") {
            i = skip("?>");
        } else if rest.starts_with("<!") {
            i = skip(">");
        } else if rest.starts_with("</") {
            depth = depth.saturating_sub(1);
            i = skip(">");
        } else {
            // The tag up to `>` outside quotes.
            let mut j = at + 1;
            let mut quote = None;
            while j < b.len() {
                match (quote, b[j]) {
                    (None, b'"' | b'\'') => quote = Some(b[j]),
                    (Some(q), c) if c == q => quote = None,
                    (None, b'>') => break,
                    _ => {}
                }
                j += 1;
            }
            let tag = &text[at + 1..j.min(text.len())];
            let name: String = tag.chars().take_while(|c| !c.is_whitespace() && *c != '/' && *c != '>').collect();
            if name.is_empty() || !name.starts_with(|c: char| c.is_alphabetic() || c == '_') {
                i = at + 1;
                continue;
            }
            if depth < MAX_TREE_DEPTH && out.len() < MAX_TREE {
                let label = NAMED_BY.iter().find_map(|a| attr_value(tag, a));
                let shown = match label {
                    Some(l) if !l.is_empty() => format!("{name} · {l}"),
                    _ => name.clone(),
                };
                out.push(Symbol { name: shown, kind: Kind::Element, line, depth });
            }
            if !tag.trim_end().ends_with('/') {
                depth += 1;
            }
            i = (j + 1).min(text.len());
        }
    }
    out
}

/// `name="value"` in a start tag (exact attribute name).
fn attr_value(tag: &str, name: &str) -> Option<String> {
    let mut from = 0;
    while let Some(off) = tag[from..].find(name) {
        let at = from + off;
        from = at + name.len();
        let before_ok = at > 0 && tag.as_bytes()[at - 1].is_ascii_whitespace();
        let rest = tag[from..].trim_start();
        if !before_ok || !rest.starts_with('=') {
            continue;
        }
        let value = rest[1..].trim_start();
        let q = value.chars().next()?;
        if q != '"' && q != '\'' {
            return None;
        }
        let end = value[1..].find(q)?;
        return Some(value[1..1 + end].chars().take(60).collect());
    }
    None
}

/// JSON: the keys as a tree (array items are counted, not listed).
fn json(text: &str) -> Vec<Symbol> {
    let b = text.as_bytes();
    let mut out = Vec::new();
    // Per open bracket: whether it is an object.
    let mut stack: Vec<bool> = Vec::new();
    let mut line = 0usize;
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'\n' => line += 1,
            b'{' => stack.push(true),
            b'[' => stack.push(false),
            b'}' | b']' => {
                stack.pop();
            }
            b'"' => {
                let start = i + 1;
                let mut j = start;
                while j < b.len() && b[j] != b'"' {
                    j += if b[j] == b'\\' { 2 } else { 1 };
                }
                let end = j.min(b.len());
                // A key: in an object and followed by `:`.
                let is_key = stack.last() == Some(&true) && text[end + 1.min(b.len() - end)..].trim_start().starts_with(':');
                let depth = stack.len().saturating_sub(1);
                if is_key && depth < MAX_TREE_DEPTH && out.len() < MAX_TREE {
                    out.push(Symbol { name: text[start..end].to_string(), kind: Kind::Section, line, depth });
                }
                line += text[start..end].matches('\n').count();
                i = end;
            }
            _ => {}
        }
        i += 1;
    }
    out
}

/// SQL: tables, views, functions and the like that a script creates.
fn sql(text: &str) -> Vec<Symbol> {
    static CREATE: LazyLock<Regex> = LazyLock::new(|| {
        re(r"(?i)^\s*(?:CREATE|ALTER)\s+(?:OR\s+REPLACE\s+)?(?:TEMPORARY\s+|TEMP\s+|UNIQUE\s+)?(TABLE|VIEW|FUNCTION|PROCEDURE|TRIGGER|INDEX|SCHEMA|DATABASE|TYPE|SEQUENCE)\s+(?:IF\s+(?:NOT\s+)?EXISTS\s+)?([`\x22\[]?[\w.$]+[`\x22\]]?)")
    });
    text.lines()
        .enumerate()
        .filter_map(|(n, l)| {
            let c = CREATE.captures(l)?;
            let what = c[1].to_ascii_uppercase();
            let kind = match what.as_str() {
                "FUNCTION" | "PROCEDURE" | "TRIGGER" => Kind::Function,
                "TABLE" | "VIEW" | "TYPE" => Kind::Class,
                _ => Kind::Section,
            };
            let name = c[2].trim_matches(['`', '"', '[', ']']).to_string();
            Some(Symbol { name: format!("{} {name}", what.to_lowercase()), kind, line: n, depth: 0 })
        })
        .collect()
}

/// Diff: the files and their changed sections (`@@ … @@`).
fn diff(text: &str) -> Vec<Symbol> {
    let mut out = Vec::new();
    for (n, l) in text.lines().enumerate() {
        if let Some(path) = l.strip_prefix("+++ ") {
            let path = path.trim_start_matches("b/").split('\t').next().unwrap_or(path);
            if path != "/dev/null" {
                out.push(Symbol { name: path.to_string(), kind: Kind::Section, line: n, depth: 0 });
            }
        } else if l.starts_with("@@") {
            let name = l.trim_start_matches('@').split("@@").next().unwrap_or("").trim();
            let context = l.rsplit("@@").next().unwrap_or("").trim();
            let shown = if context.is_empty() { name.to_string() } else { format!("{name}  {context}") };
            out.push(Symbol { name: shown, kind: Kind::Rule, line: n, depth: 1 });
        }
    }
    out
}

fn sections(text: &str) -> Vec<Symbol> {
    static S: LazyLock<Regex> = LazyLock::new(|| re(r"^\s*\[\[?([^\]]+)\]\]?"));
    text.lines()
        .enumerate()
        .filter_map(|(n, l)| S.captures(l).map(|c| Symbol { name: c[1].trim().to_string(), kind: Kind::Section, line: n, depth: 0 }))
        .collect()
}

fn yaml(text: &str) -> Vec<Symbol> {
    static K: LazyLock<Regex> = LazyLock::new(|| re(r#"^([A-Za-z_"'][^:#]*):"#));
    text.lines()
        .enumerate()
        .filter_map(|(n, l)| K.captures(l).map(|c| Symbol { name: c[1].trim_matches(['"', '\'']).to_string(), kind: Kind::Section, line: n, depth: 0 }))
        .collect()
}

/// Index of the innermost symbol at or before `line`.
pub fn current(symbols: &[Symbol], line: usize) -> Option<usize> {
    symbols.iter().rposition(|s| s.line <= line)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(v: &[Symbol]) -> Vec<(String, usize)> {
        v.iter().map(|s| (s.name.clone(), s.depth)).collect()
    }

    #[test]
    fn javascript() {
        let src = "// helper\nfunction init(a) {\n  if (a) {\n  }\n}\nclass Shop {\n  constructor() {\n  }\n  async load(id) {\n    for (;;) {}\n  }\n}\nconst add = (a, b) => a + b;\n";
        assert_eq!(
            names(&symbols(src, Language::JavaScript)),
            vec![("init".into(), 0), ("Shop".into(), 0), ("constructor".into(), 1), ("load".into(), 1), ("add".into(), 0)]
        );
    }

    #[test]
    fn php_python_css_markdown() {
        let php = "<?php\nclass Cart {\n    public function add($x) {\n    }\n}\nfunction helper() {}\n";
        assert_eq!(names(&symbols(php, Language::Php)), vec![("Cart".into(), 0), ("add".into(), 1), ("helper".into(), 0)]);
        let py = "class A:\n    def f(self):\n        pass\ndef g():\n    pass\n";
        assert_eq!(names(&symbols(py, Language::Python)), vec![("A".into(), 0), ("f".into(), 1), ("g".into(), 0)]);
        let css = "/* x { */\nbody { margin: 0; }\n@media (max-width: 600px) {\n  .nav,\n  .menu { display: none; }\n}\n";
        assert_eq!(
            names(&symbols(css, Language::Css)),
            vec![("body".into(), 0), ("@media (max-width: 600px)".into(), 0), (".nav, .menu".into(), 1)]
        );
        let md = "# Titel\ntext\n```\n# kein Titel\n```\n## Unter\n";
        assert_eq!(names(&symbols(md, Language::Markdown)), vec![("Titel".into(), 0), ("Unter".into(), 1)]);
    }

    #[test]
    fn html_headings_and_ids() {
        let html = "<style>\n  .box { color: red; }\n</style>\n<main id=\"content\">\n<h1>Willkommen <b>hier</b></h1>\n</main>\n";
        let s = symbols(html, Language::Html);
        assert_eq!(names(&s), vec![(".box".into(), 0), ("main#content".into(), 0), ("Willkommen hier".into(), 0)]);
        assert_eq!(s[0].line, 1);
        assert_eq!(s[2].line, 4);
    }

    #[test]
    fn xml_tree() {
        let xml = "<?xml version=\"1.0\"?>\n<!-- Kommentar <x> -->\n<config>\n  <item name=\"Start\" />\n  <group id=\"g1\">\n    <entry>Text</entry>\n    <![CDATA[<kein>]]>\n  </group>\n</config>\n";
        let s = symbols(xml, Language::Xml);
        let tree: Vec<(String, usize, usize)> = s.iter().map(|s| (s.name.clone(), s.line, s.depth)).collect();
        assert_eq!(tree, vec![("config".into(), 2, 0), ("item · Start".into(), 3, 1), ("group · g1".into(), 4, 1), ("entry".into(), 5, 2)]);
    }

    #[test]
    fn json_sql_diff() {
        let json = "{\n  \"name\": \"x\",\n  \"scripts\": {\n    \"build\": \"a:b\"\n  },\n  \"list\": [\"nein\", {\"tief\": 1}]\n}";
        let s = symbols(json, Language::Json);
        let tree: Vec<(String, usize, usize)> = s.iter().map(|s| (s.name.clone(), s.line, s.depth)).collect();
        assert_eq!(tree, vec![("name".into(), 1, 0), ("scripts".into(), 2, 0), ("build".into(), 3, 1), ("list".into(), 5, 0), ("tief".into(), 5, 2)]);
        let sql = "-- Schema\nCREATE TABLE IF NOT EXISTS `kunden` (\n id INT\n);\ncreate or replace view v_aktiv as select 1;\nCREATE FUNCTION summe() RETURNS INT\n";
        let s = symbols(sql, Language::Sql);
        let lines: Vec<(String, usize)> = s.iter().map(|s| (s.name.clone(), s.line)).collect();
        assert_eq!(lines, vec![("table kunden".into(), 1), ("view v_aktiv".into(), 4), ("function summe".into(), 5)]);
        let diff = "diff --git a/x.rs b/x.rs\n--- a/x.rs\n+++ b/x.rs\n@@ -1,3 +1,4 @@ fn main()\n-a\n+b\n";
        assert_eq!(names(&symbols(diff, Language::Diff)), vec![("x.rs".into(), 0), ("-1,3 +1,4  fn main()".into(), 1)]);
    }
}
