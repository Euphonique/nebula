//! Helper for Nebula's translations.
//!
//! `i18n-wrap wrap <src> [--dry]` puts UI texts into `t!()`:
//!   "Speichern"                    -> t!("Speichern")
//!   format!("{n} Server", ...)     -> t!("{n} Server", n = n)
//! Positional `{}` and format specs become named placeholders. Tests, consts,
//! statics, patterns, identifiers, paths and code snippets are left alone.
//!
//! `i18n-wrap keys <src>` prints every `t!`/`tk!` key as a JSON array.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use proc_macro2::LineColumn;
use syn::punctuated::Punctuated;
use syn::spanned::Spanned;
use syn::visit::{self, Visit};
use syn::{Expr, ExprLit, Lit, LitStr, Macro, Token};

/// Files that hold code, data or tests rather than UI texts.
const SKIP_FILES: &[&str] = &["src/i18n.rs", "tests.rs", "integration_tests.rs", "src/editor/complete.rs", "src/editor/emmet.rs", "src/highlight.rs", "src/lang.rs", "src/preview.rs", "src/theme_import.rs", "src/fonts.rs"];
/// Names the program uses as identifiers (stored, compared or sent).
const KEEP: &[&str] = &[
    "Keyfile", "Logontype", "Pass", "Protocol", "Publisher", "User", "Host", "Port", "Name", "RemoteDir", "PasvMode", "Type",
    "Folder", "Scripts", "Icon", "Lock", "Regular", "Weekly", "Manual", "Default", "Default (recommended)", "Auto review",
    "Super", "Meta", "Plus", "Control", "Shift", "Alt", "Ctrl+", "Shift+", "Alt+", "Fonts", "Programs", "Servers",
    "QuickJS runtime", "Claude Code", "Codex (ChatGPT)", "Gemini CLI", "Windows PowerShell", "PowerShell", "Git Bash",
    "DisplayName", "DisplayVersion", "DisplayIcon", "InstallLocation", "UninstallString", "EstimatedSize", "NoModify", "NoRepair",
    "FriendlyAppName", "SupportedTypes", "DefaultIcon", "Nebula", "Eingabeaufforderung",
];
/// Methods whose text arguments are data (paths, keys, commands, patterns).
const DATA_METHODS: &[&str] = &[
    "join", "set_value", "get_value", "open_subkey", "open_subkey_with_flags", "create_subkey", "delete_subkey_all", "delete_value",
    "arg", "args", "env", "expect", "contains", "starts_with", "ends_with", "strip_prefix", "strip_suffix", "split", "split_once",
    "rsplit_once", "find", "rfind", "replace", "replacen", "eq_ignore_ascii_case", "get", "remove", "has_tag_name", "attribute",
    "with_extension", "set_extension", "parse_body", "matches", "trim_start_matches", "trim_end_matches",
    "header", "set_header", "query", "send_message", "call", "eval",
];
/// Functions whose text arguments are data.
const DATA_CALLS: &[&str] = &[
    "Command::new", "PathBuf::from", "Path::new", "OsStr::new", "OsString::from", "Regex::new", "RegexBuilder::new", "Id::new", "Id::unique",
    "Entry::new", "Url::parse", "IdSalt::new", "Atom::new",
];
/// Macros whose texts are not shown in the UI (or already translated).
const SKIP_MACROS: &[&str] = &[
    "t", "tk", "println", "eprintln", "print", "eprint", "write", "writeln", "panic", "assert", "assert_eq", "assert_ne",
    "debug_assert", "debug_assert_eq", "unreachable", "todo", "unimplemented", "include_str", "include_bytes", "concat", "env",
    "stringify", "matches", "macro_rules", "json", "dbg",
];

struct Edit {
    start: usize,
    end: usize,
    text: String,
}

struct Wrapper<'a> {
    src: &'a str,
    line_starts: Vec<usize>,
    edits: Vec<Edit>,
    report: Vec<String>,
    file: String,
}

impl<'a> Wrapper<'a> {
    fn new(src: &'a str, file: String) -> Self {
        let mut line_starts = vec![0];
        line_starts.extend(src.match_indices('\n').map(|(i, _)| i + 1));
        Self { src, line_starts, edits: Vec::new(), report: Vec::new(), file }
    }

    fn offset(&self, lc: LineColumn) -> usize {
        let start = self.line_starts[lc.line - 1];
        let line = &self.src[start..];
        start + line.char_indices().nth(lc.column).map_or(line.len(), |(i, _)| i)
    }

    fn range(&self, span: proc_macro2::Span) -> (usize, usize) {
        (self.offset(span.start()), self.offset(span.end()))
    }

    fn text(&self, span: proc_macro2::Span) -> &str {
        let (a, b) = self.range(span);
        &self.src[a..b]
    }

    fn line(&self, span: proc_macro2::Span) -> usize {
        span.start().line
    }

    fn push(&mut self, span: proc_macro2::Span, text: String, shown: &str) {
        let (start, end) = self.range(span);
        // An outer edit (a whole macro) wins over edits inside it.
        if self.edits.iter().any(|e| e.start <= start && end <= e.end) {
            return;
        }
        self.edits.retain(|e| !(start <= e.start && e.end <= end));
        self.report.push(format!("{}:{}: {shown:?}", self.file, self.line(span)));
        self.edits.push(Edit { start, end, text });
    }

    /// `format!`-style macro with a UI text: the replacement, if any.
    fn convert_format(&self, lit: &LitStr, args: &[Expr]) -> Option<(String, String, bool)> {
        let value = lit.value();
        let mut positional: Vec<&Expr> = Vec::new();
        let mut named: Vec<(String, &Expr)> = Vec::new();
        for a in args {
            match a {
                Expr::Assign(assign) => {
                    if let Expr::Path(p) = &*assign.left
                        && let Some(id) = p.path.get_ident()
                    {
                        named.push((id.to_string(), &assign.right));
                        continue;
                    }
                    positional.push(a);
                }
                _ => positional.push(a),
            }
        }
        let mut out = String::new();
        let mut values: Vec<(String, String)> = Vec::new();
        let mut next = 0;
        let chars: Vec<char> = value.chars().collect();
        let mut i = 0;
        while i < chars.len() {
            let c = chars[i];
            if c == '{' && chars.get(i + 1) == Some(&'{') {
                out.push_str("{{");
                i += 2;
                continue;
            }
            if c == '}' && chars.get(i + 1) == Some(&'}') {
                out.push_str("}}");
                i += 2;
                continue;
            }
            if c != '{' {
                out.push(c);
                i += 1;
                continue;
            }
            let close = chars[i..].iter().position(|&c| c == '}')? + i;
            let inner: String = chars[i + 1..close].iter().collect();
            let (arg, spec) = inner.split_once(':').map_or((inner.as_str(), ""), |(a, s)| (a, s));
            let (mut src, mut name) = if arg.is_empty() {
                let e = positional.get(next)?;
                next += 1;
                (self.text(e.span()).to_string(), name_for(e))
            } else if let Ok(idx) = arg.parse::<usize>() {
                let e = positional.get(idx)?;
                (self.text(e.span()).to_string(), name_for(e))
            } else if let Some((_, e)) = named.iter().find(|(n, _)| n == arg) {
                (self.text(e.span()).to_string(), arg.to_string())
            } else {
                // Inline capture: `{count}` reads the variable `count`.
                (arg.to_string(), arg.to_string())
            };
            if !spec.is_empty() {
                src = format!("format!(\"{{:{spec}}}\", {src})");
            }
            // Same name for a different value: number it.
            let base = name.clone();
            let mut n = 2;
            while values.iter().any(|(k, v)| *k == name && *v != src) {
                name = format!("{base}{n}");
                n += 1;
            }
            if !values.iter().any(|(k, _)| *k == name) {
                values.push((name.clone(), src));
            }
            out.push('{');
            out.push_str(&name);
            out.push('}');
            i = close + 1;
        }
        if values.is_empty() {
            let plain = out.replace("{{", "{").replace("}}", "}");
            return Some((format!("t!({plain:?})"), plain, false));
        }
        let list: Vec<String> = values.iter().map(|(k, v)| format!("{k} = {v}")).collect();
        Some((format!("t!({out:?}, {})", list.join(", ")), out, true))
    }
}

/// A placeholder name for a value expression.
fn name_for(e: &Expr) -> String {
    let raw = match e {
        Expr::Path(p) => p.path.segments.last().map(|s| s.ident.to_string()),
        Expr::Field(f) => match &f.member {
            syn::Member::Named(id) => Some(id.to_string()),
            syn::Member::Unnamed(i) => Some(format!("v{}", i.index)),
        },
        Expr::MethodCall(m) => {
            let method = m.method.to_string();
            match method.as_str() {
                "display" | "to_string" | "to_string_lossy" | "clone" | "as_str" | "trim" | "unwrap_or_default" | "join" => Some(name_for(&m.receiver)),
                "len" | "count" => Some(format!("{}_count", name_for(&m.receiver))),
                _ => Some(method),
            }
        }
        Expr::Reference(r) => Some(name_for(&r.expr)),
        Expr::Paren(p) => Some(name_for(&p.expr)),
        Expr::Unary(u) => Some(name_for(&u.expr)),
        Expr::Call(c) => match &*c.func {
            Expr::Path(p) => p.path.segments.last().map(|s| s.ident.to_string().to_lowercase()),
            _ => None,
        },
        _ => None,
    };
    let mut name: String = raw.unwrap_or_else(|| "value".into()).chars().filter(|c| c.is_alphanumeric() || *c == '_').collect();
    if name.is_empty() || name.chars().next().is_some_and(|c| c.is_ascii_digit()) {
        name = format!("v{name}");
    }
    let name = name.to_lowercase();
    if matches!(name.as_str(), "self" | "type" | "match" | "fn" | "mod" | "ref" | "move" | "loop" | "use" | "where" | "impl" | "in" | "as" | "crate" | "super" | "const" | "static" | "struct" | "enum" | "let" | "if" | "else" | "for" | "while" | "return" | "break" | "continue" | "true" | "false") {
        format!("{name}_value")
    } else {
        name
    }
}

/// Whether a literal looks like text shown to people (and not like code).
fn is_ui(s: &str) -> bool {
    if !s.chars().any(|c| c.is_alphabetic()) || !s.chars().any(|c| c.is_lowercase()) {
        return false;
    }
    if KEEP.contains(&s) || s.trim_start().starts_with('-') || s.contains('\0') || s.contains('\\') {
        return false;
    }
    // File names (Clips.json, Readme.md).
    if !s.contains(' ') && s.contains('.') && !s.ends_with('.') {
        return false;
    }
    let ascii = s.is_ascii();
    let space = s.contains(' ');
    let upper = s.chars().any(|c| c.is_uppercase());
    if ascii && !space && !upper {
        return false; // ids, keys, file extensions
    }
    if s.contains("://") || s.contains(":\\") || ((s.starts_with('/') || s.starts_with('\\') || s.starts_with('.')) && !space) {
        return false;
    }
    if s.contains(['<', '>', ';', '$', '`', '{', '}', '#', '=', '|']) && !s.contains(['ä', 'ö', 'ü', 'Ä', 'Ö', 'Ü', 'ß', '…', '„']) {
        return false; // code snippets
    }
    // CamelCase words (PowerShell, JavaScript) are names.
    if !space && ascii && s.chars().skip(1).any(|c| c.is_uppercase()) {
        return false;
    }
    true
}

fn has_cfg_test(attrs: &[syn::Attribute]) -> bool {
    attrs.iter().any(|a| {
        let t = quote::quote!(#a).to_string().replace(' ', "");
        t.contains("cfg(test)") || t == "#[test]" || t.contains("#[ignore")
    })
}

fn macro_name(m: &Macro) -> String {
    m.path.segments.last().map(|s| s.ident.to_string()).unwrap_or_default()
}

impl<'ast> Visit<'ast> for Wrapper<'_> {
    fn visit_item_const(&mut self, _: &'ast syn::ItemConst) {}
    fn visit_item_static(&mut self, _: &'ast syn::ItemStatic) {}
    fn visit_impl_item_const(&mut self, _: &'ast syn::ImplItemConst) {}
    fn visit_attribute(&mut self, _: &'ast syn::Attribute) {}
    fn visit_pat(&mut self, _: &'ast syn::Pat) {}
    fn visit_item_macro(&mut self, _: &'ast syn::ItemMacro) {}

    fn visit_item_mod(&mut self, m: &'ast syn::ItemMod) {
        if !has_cfg_test(&m.attrs) {
            visit::visit_item_mod(self, m);
        }
    }

    fn visit_item_fn(&mut self, f: &'ast syn::ItemFn) {
        if !has_cfg_test(&f.attrs) {
            visit::visit_item_fn(self, f);
        }
    }

    // A literal compared with something is data, not a label.
    fn visit_expr_binary(&mut self, e: &'ast syn::ExprBinary) {
        let compare = matches!(e.op, syn::BinOp::Eq(_) | syn::BinOp::Ne(_));
        for side in [&*e.left, &*e.right] {
            if compare && matches!(side, Expr::Lit(_)) {
                continue;
            }
            self.visit_expr(side);
        }
    }

    fn visit_expr_method_call(&mut self, e: &'ast syn::ExprMethodCall) {
        self.visit_expr(&e.receiver);
        let data = DATA_METHODS.contains(&e.method.to_string().as_str());
        for a in &e.args {
            if data && matches!(a, Expr::Lit(_)) {
                continue;
            }
            self.visit_expr(a);
        }
    }

    fn visit_expr_call(&mut self, e: &'ast syn::ExprCall) {
        let path = match &*e.func {
            Expr::Path(p) => {
                let segs: Vec<String> = p.path.segments.iter().map(|s| s.ident.to_string()).collect();
                segs[segs.len().saturating_sub(2)..].join("::")
            }
            _ => String::new(),
        };
        self.visit_expr(&e.func);
        let data = DATA_CALLS.contains(&path.as_str());
        for a in &e.args {
            if data && matches!(a, Expr::Lit(_)) {
                continue;
            }
            self.visit_expr(a);
        }
    }

    fn visit_expr_lit(&mut self, e: &'ast ExprLit) {
        if let Lit::Str(s) = &e.lit
            && is_ui(&s.value())
        {
            let src = self.text(s.span()).to_string();
            self.push(s.span(), format!("t!({src})"), &s.value());
        }
    }

    fn visit_macro(&mut self, m: &'ast Macro) {
        let name = macro_name(m);
        if SKIP_MACROS.contains(&name.as_str()) {
            return;
        }
        let Ok(args) = m.parse_body_with(Punctuated::<Expr, Token![,]>::parse_terminated) else { return };
        let args: Vec<Expr> = args.into_iter().collect();
        if matches!(name.as_str(), "format" | "anyhow" | "bail")
            && let Some(Expr::Lit(ExprLit { lit: Lit::Str(lit), .. })) = args.first()
        {
            // The text without its placeholders decides.
            let stripped: String = {
                let v = lit.value();
                let mut out = String::new();
                let mut depth = 0;
                for c in v.chars() {
                    match c {
                        '{' => depth += 1,
                        '}' => depth -= 1,
                        _ if depth == 0 => out.push(c),
                        _ => {}
                    }
                }
                out
            };
            if is_ui(&stripped)
                && let Some((t, shown, has_args)) = self.convert_format(lit, &args[1..])
            {
                let replacement = match name.as_str() {
                    "format" if !has_args => format!("{t}.to_string()"),
                    "format" => t,
                    _ => format!("{}!({t})", self.text(m.path.span())),
                };
                // The whole macro call, including its path and `!`.
                let span = m.path.span().join(m.delimiter.span().join()).unwrap_or_else(|| m.path.span());
                self.push(span, replacement, &shown);
                return;
            }
        }
        for a in &args {
            self.visit_expr(a);
        }
    }
}

/// Collects the keys of `t!`/`tk!`.
struct Keys(BTreeSet<String>);

impl<'ast> Visit<'ast> for Keys {
    fn visit_macro(&mut self, m: &'ast Macro) {
        let name = macro_name(m);
        if let Ok(args) = m.parse_body_with(Punctuated::<Expr, Token![,]>::parse_terminated) {
            for (i, a) in args.iter().enumerate() {
                if i == 0
                    && (name == "t" || name == "tk")
                    && let Expr::Lit(ExprLit { lit: Lit::Str(s), .. }) = a
                {
                    self.0.insert(s.value());
                }
                self.visit_expr(a);
            }
        }
    }
}

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            rust_files(&p, out);
        } else if p.extension().is_some_and(|x| x == "rs") {
            out.push(p);
        }
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (cmd, src) = (args.first().map(String::as_str).unwrap_or(""), PathBuf::from(args.get(1).cloned().unwrap_or_else(|| "src".into())));
    let dry = args.iter().any(|a| a == "--dry");
    let mut files = Vec::new();
    rust_files(&src, &mut files);
    files.sort();
    match cmd {
        "wrap" => {
            let mut total = 0;
            for path in files {
                let norm = path.to_string_lossy().replace('\\', "/");
                if SKIP_FILES.iter().any(|s| norm.ends_with(s)) {
                    continue;
                }
                let text = std::fs::read_to_string(&path).unwrap();
                let Ok(file) = syn::parse_file(&text) else {
                    eprintln!("{}: nicht lesbar", path.display());
                    continue;
                };
                let mut w = Wrapper::new(&text, path.display().to_string());
                w.visit_file(&file);
                total += w.edits.len();
                for line in &w.report {
                    println!("{line}");
                }
                if !dry && !w.edits.is_empty() {
                    let mut edits = w.edits;
                    edits.sort_by_key(|e| std::cmp::Reverse(e.start));
                    let mut out = text.clone();
                    for e in edits {
                        out.replace_range(e.start..e.end, &e.text);
                    }
                    std::fs::write(&path, out).unwrap();
                }
            }
            eprintln!("{total} Texte");
        }
        "keys" => {
            let mut keys = Keys(BTreeSet::new());
            for path in files {
                let text = std::fs::read_to_string(&path).unwrap();
                if let Ok(file) = syn::parse_file(&text) {
                    keys.visit_file(&file);
                }
            }
            let list: Vec<&String> = keys.0.iter().collect();
            println!("{}", serde_like(&list));
        }
        // merge <keys.json> <translations.txt> <code> <name> <out.json>: lines "N<TAB>text"
        // (N = 1-based position in keys.json, `\n` for line breaks).
        "merge" => {
            let keys_text = std::fs::read_to_string(&args[1]).unwrap();
            let keys: Vec<String> = parse_json_strings(&keys_text);
            let lines = std::fs::read_to_string(&args[2]).unwrap();
            let mut map: std::collections::BTreeMap<String, String> = std::collections::BTreeMap::new();
            for line in lines.lines().filter(|l| !l.trim().is_empty()) {
                let Some((n, text)) = line.split_once('\t') else {
                    eprintln!("Zeile ohne Tab: {line}");
                    continue;
                };
                let Ok(n) = n.trim().parse::<usize>() else { continue };
                let Some(key) = keys.get(n.wrapping_sub(1)) else {
                    eprintln!("Nummer {n} gibt es nicht");
                    continue;
                };
                map.insert(key.clone(), unescape(text));
            }
            let missing = keys.iter().filter(|k| !map.contains_key(*k)).count();
            let body: Vec<String> = map.iter().map(|(k, v)| format!("    {}: {}", json_string(k), json_string(v))).collect();
            let out = format!(
                "{{\n  \"code\": {},\n  \"name\": {},\n  \"strings\": {{\n{}\n  }}\n}}\n",
                json_string(&args[3]),
                json_string(&args[4]),
                body.join(",\n")
            );
            std::fs::write(&args[5], out).unwrap();
            eprintln!("{} übersetzt, {missing} fehlen", map.len());
        }
        _ => eprintln!("i18n-wrap wrap|keys <src> [--dry] | merge <keys.json> <tsv> <code> <name> <out.json>"),
    }
}

/// JSON array of strings (no serde needed for this helper).
fn serde_like(list: &[&String]) -> String {
    let items: Vec<String> = list.iter().map(|s| format!("  {}", json_string(s))).collect();
    format!("[\n{}\n]", items.join(",\n"))
}

fn json_string(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// `\n` line break, `\\` backslash, `\"` quote, `\t` tab.
fn unescape(s: &str) -> String {
    let mut out = String::new();
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some('"') => out.push('"'),
            Some('\\') => out.push('\\'),
            Some(other) => {
                out.push('\\');
                out.push(other);
            }
            None => out.push('\\'),
        }
    }
    out
}

/// The strings of a JSON array written by `keys`.
fn parse_json_strings(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let chars: Vec<char> = text.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] != '"' {
            i += 1;
            continue;
        }
        let mut s = String::new();
        i += 1;
        while i < chars.len() && chars[i] != '"' {
            if chars[i] == '\\' {
                i += 1;
                match chars[i] {
                    'n' => s.push('\n'),
                    't' => s.push('\t'),
                    'u' => {
                        let hex: String = chars[i + 1..i + 5].iter().collect();
                        s.push(char::from_u32(u32::from_str_radix(&hex, 16).unwrap()).unwrap());
                        i += 4;
                    }
                    c => s.push(c),
                }
            } else {
                s.push(chars[i]);
            }
            i += 1;
        }
        out.push(s);
        i += 1;
    }
    out
}
