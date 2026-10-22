//! Context-aware completions for HTML and CSS, with document words as fallback.

use std::collections::{HashMap, HashSet};
use std::sync::{LazyLock, RwLock};

use super::commands::is_word_char;
use super::document::{Change, Document, EditKind, Selection};
use crate::lang::Language;

const HTML_TAGS: &[&str] = &[
    "a", "abbr", "address", "area", "article", "aside", "audio", "b", "base", "bdi", "bdo",
    "blockquote", "body", "br", "button", "canvas", "caption", "cite", "code", "col", "colgroup",
    "data", "datalist", "dd", "del", "details", "dfn", "dialog", "div", "dl", "dt", "em", "embed",
    "fieldset", "figcaption", "figure", "footer", "form", "h1", "h2", "h3", "h4", "h5", "h6",
    "head", "header", "hgroup", "hr", "html", "i", "iframe", "img", "input", "ins", "kbd", "label",
    "legend", "li", "link", "main", "map", "mark", "menu", "meta", "meter", "nav", "noscript",
    "object", "ol", "optgroup", "option", "output", "p", "picture", "pre", "progress", "q", "rp",
    "rt", "ruby", "s", "samp", "script", "search", "section", "select", "slot", "small", "source",
    "span", "strong", "style", "sub", "summary", "sup", "svg", "table", "tbody", "td", "template",
    "textarea", "tfoot", "th", "thead", "time", "title", "tr", "track", "u", "ul", "var", "video",
    "wbr",
];

const HTML_GLOBAL_ATTRS: &[&str] = &[
    "accesskey", "aria-describedby", "aria-hidden", "aria-label", "aria-labelledby", "autofocus",
    "class", "contenteditable", "data-", "dir", "draggable", "hidden", "id", "inert", "lang",
    "popover", "role", "spellcheck", "style", "tabindex", "title", "translate", "onclick",
    "onchange", "oninput", "onsubmit", "onload", "onkeydown", "onmouseover",
];

fn tag_attrs(tag: &str) -> &'static [&'static str] {
    match tag {
        "a" => &["href", "target", "rel", "download", "hreflang", "type", "referrerpolicy"],
        "img" => &["src", "alt", "width", "height", "loading", "decoding", "srcset", "sizes", "fetchpriority"],
        "link" => &["rel", "href", "type", "media", "as", "crossorigin", "integrity", "sizes"],
        "script" => &["src", "type", "async", "defer", "crossorigin", "integrity", "nomodule"],
        "meta" => &["name", "content", "charset", "http-equiv", "property"],
        "form" => &["action", "method", "enctype", "novalidate", "autocomplete", "target", "name"],
        "input" => &[
            "type", "name", "value", "placeholder", "required", "disabled", "checked", "readonly",
            "min", "max", "step", "pattern", "maxlength", "minlength", "autocomplete", "list",
            "multiple", "accept", "form", "size",
        ],
        "button" => &["type", "name", "value", "disabled", "form", "popovertarget"],
        "select" => &["name", "multiple", "required", "disabled", "size", "form"],
        "option" => &["value", "selected", "disabled", "label"],
        "textarea" => &["name", "rows", "cols", "placeholder", "required", "disabled", "readonly", "maxlength", "wrap"],
        "label" => &["for", "form"],
        "iframe" => &["src", "width", "height", "allow", "allowfullscreen", "loading", "name", "sandbox", "referrerpolicy"],
        "video" | "audio" => &["src", "controls", "autoplay", "loop", "muted", "poster", "preload", "playsinline", "width", "height"],
        "source" => &["src", "srcset", "type", "media", "sizes"],
        "td" | "th" => &["colspan", "rowspan", "headers", "scope"],
        "html" => &["lang"],
        "time" => &["datetime"],
        "ol" => &["start", "reversed", "type"],
        "details" | "dialog" => &["open"],
        _ => &[],
    }
}

const CSS_PROPERTIES: &[&str] = &[
    "accent-color", "align-content", "align-items", "align-self", "all", "animation",
    "animation-delay", "animation-direction", "animation-duration", "animation-fill-mode",
    "animation-iteration-count", "animation-name", "animation-play-state",
    "animation-timing-function", "appearance", "aspect-ratio", "backdrop-filter",
    "backface-visibility", "background", "background-attachment", "background-blend-mode",
    "background-clip", "background-color", "background-image", "background-origin",
    "background-position", "background-repeat", "background-size", "block-size", "border",
    "border-block", "border-bottom", "border-bottom-color", "border-bottom-left-radius",
    "border-bottom-right-radius", "border-bottom-style", "border-bottom-width", "border-collapse",
    "border-color", "border-image", "border-inline", "border-left", "border-left-color",
    "border-left-style", "border-left-width", "border-radius", "border-right", "border-right-color",
    "border-right-style", "border-right-width", "border-spacing", "border-style", "border-top",
    "border-top-color", "border-top-left-radius", "border-top-right-radius", "border-top-style",
    "border-top-width", "border-width", "bottom", "box-shadow", "box-sizing", "break-after",
    "break-before", "break-inside", "caption-side", "caret-color", "clear", "clip-path", "color",
    "column-count", "column-gap", "column-rule", "column-span", "column-width", "columns",
    "container", "container-name", "container-type", "content", "content-visibility",
    "counter-increment", "counter-reset", "cursor", "direction", "display", "empty-cells", "fill",
    "filter", "flex", "flex-basis", "flex-direction", "flex-flow", "flex-grow", "flex-shrink",
    "flex-wrap", "float", "font", "font-display", "font-family", "font-feature-settings",
    "font-size", "font-style", "font-variant", "font-variation-settings", "font-weight", "gap",
    "grid", "grid-area", "grid-auto-columns", "grid-auto-flow", "grid-auto-rows", "grid-column",
    "grid-column-end", "grid-column-start", "grid-row", "grid-row-end", "grid-row-start",
    "grid-template", "grid-template-areas", "grid-template-columns", "grid-template-rows",
    "height", "hyphens", "image-rendering", "inline-size", "inset", "inset-block", "inset-inline",
    "isolation", "justify-content", "justify-items", "justify-self", "left", "letter-spacing",
    "line-clamp", "line-height", "list-style", "list-style-image", "list-style-position",
    "list-style-type", "margin", "margin-block", "margin-bottom", "margin-inline", "margin-left",
    "margin-right", "margin-top", "mask", "max-block-size", "max-height", "max-inline-size",
    "max-width", "min-block-size", "min-height", "min-inline-size", "min-width", "mix-blend-mode",
    "object-fit", "object-position", "opacity", "order", "outline", "outline-color",
    "outline-offset", "outline-style", "outline-width", "overflow", "overflow-wrap", "overflow-x",
    "overflow-y", "overscroll-behavior", "padding", "padding-block", "padding-bottom",
    "padding-inline", "padding-left", "padding-right", "padding-top", "place-content",
    "place-items", "place-self", "pointer-events", "position", "quotes", "resize", "right",
    "rotate", "row-gap", "scale", "scroll-behavior", "scroll-margin", "scroll-padding",
    "scroll-snap-align", "scroll-snap-type", "scrollbar-color", "scrollbar-gutter",
    "scrollbar-width", "stroke", "stroke-width", "tab-size", "table-layout", "text-align",
    "text-decoration", "text-decoration-color", "text-decoration-line", "text-decoration-style",
    "text-decoration-thickness", "text-indent", "text-overflow", "text-shadow", "text-transform",
    "text-underline-offset", "text-wrap", "top", "touch-action", "transform", "transform-origin",
    "transition", "transition-delay", "transition-duration", "transition-property",
    "transition-timing-function", "translate", "user-select", "vertical-align", "visibility",
    "white-space", "width", "will-change", "word-break", "word-spacing", "writing-mode",
    "z-index",
];

const CSS_GLOBAL_VALUES: &[&str] = &["inherit", "initial", "unset", "revert", "var()", "calc()"];

fn css_values(property: &str) -> &'static [&'static str] {
    match property {
        "display" => &["block", "inline", "inline-block", "flex", "inline-flex", "grid", "inline-grid", "contents", "none", "table", "table-cell", "list-item", "flow-root"],
        "position" => &["static", "relative", "absolute", "fixed", "sticky"],
        "flex-direction" => &["row", "row-reverse", "column", "column-reverse"],
        "flex-wrap" => &["nowrap", "wrap", "wrap-reverse"],
        "justify-content" | "align-content" | "place-content" => &["flex-start", "flex-end", "start", "end", "center", "space-between", "space-around", "space-evenly", "stretch", "normal"],
        "align-items" | "align-self" | "justify-items" | "justify-self" | "place-items" | "place-self" => &["flex-start", "flex-end", "start", "end", "center", "baseline", "stretch", "normal", "auto"],
        "text-align" => &["left", "right", "center", "justify", "start", "end"],
        "font-weight" => &["normal", "bold", "bolder", "lighter", "100", "200", "300", "400", "500", "600", "700", "800", "900"],
        "font-style" => &["normal", "italic", "oblique"],
        "text-transform" => &["none", "uppercase", "lowercase", "capitalize"],
        "text-decoration" | "text-decoration-line" => &["none", "underline", "overline", "line-through"],
        "overflow" | "overflow-x" | "overflow-y" => &["visible", "hidden", "clip", "scroll", "auto"],
        "visibility" => &["visible", "hidden", "collapse"],
        "white-space" => &["normal", "nowrap", "pre", "pre-wrap", "pre-line", "break-spaces"],
        "cursor" => &["auto", "default", "pointer", "text", "move", "grab", "grabbing", "not-allowed", "wait", "help", "crosshair", "zoom-in", "zoom-out"],
        "box-sizing" => &["border-box", "content-box"],
        "object-fit" => &["fill", "contain", "cover", "none", "scale-down"],
        "background-repeat" => &["repeat", "no-repeat", "repeat-x", "repeat-y", "space", "round"],
        "background-size" => &["auto", "cover", "contain"],
        "background-position" | "object-position" => &["center", "top", "bottom", "left", "right"],
        "border-style" | "outline-style" | "border-top-style" | "border-bottom-style" | "border-left-style" | "border-right-style" => &["none", "solid", "dashed", "dotted", "double", "groove", "ridge", "inset", "outset"],
        "float" | "clear" => &["none", "left", "right", "both", "inline-start", "inline-end"],
        "list-style-type" => &["none", "disc", "circle", "square", "decimal", "lower-alpha", "upper-alpha", "lower-roman", "upper-roman"],
        "pointer-events" => &["auto", "none"],
        "user-select" => &["auto", "none", "text", "all"],
        "word-break" => &["normal", "break-all", "keep-all", "break-word"],
        "overflow-wrap" => &["normal", "break-word", "anywhere"],
        "text-overflow" => &["clip", "ellipsis"],
        "vertical-align" => &["baseline", "top", "middle", "bottom", "text-top", "text-bottom", "sub", "super"],
        "scroll-behavior" => &["auto", "smooth"],
        "grid-auto-flow" => &["row", "column", "dense", "row dense", "column dense"],
        "transition-timing-function" | "animation-timing-function" => &["ease", "ease-in", "ease-out", "ease-in-out", "linear", "step-start", "step-end", "cubic-bezier()", "steps()"],
        "animation-direction" => &["normal", "reverse", "alternate", "alternate-reverse"],
        "animation-fill-mode" => &["none", "forwards", "backwards", "both"],
        "content" => &["\"\"", "none", "normal", "attr()", "counter()"],
        "width" | "height" | "min-width" | "max-width" | "min-height" | "max-height" | "flex-basis" => &["auto", "100%", "fit-content", "min-content", "max-content", "100vw", "100vh", "100dvh"],
        "color" | "background-color" | "border-color" | "outline-color" | "fill" | "stroke" | "caret-color" | "accent-color" | "text-decoration-color" => &["transparent", "currentColor", "black", "white", "red", "green", "blue", "gray", "orange", "rgb()", "rgba()", "hsl()", "oklch()"],
        "transform" => &["none", "translate()", "translateX()", "translateY()", "scale()", "rotate()", "skew()", "matrix()"],
        "font-family" => &["system-ui", "sans-serif", "serif", "monospace", "-apple-system", "\"Segoe UI\"", "Arial", "Helvetica", "Georgia"],
        _ => &[],
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Tag,
    Attribute,
    Property,
    Value,
    Keyword,
    Snippet,
    Word,
    /// From an extension or language server.
    Ext,
}

#[derive(Clone, Debug)]
pub struct Item {
    pub label: String,
    /// Text to insert; `$0` marks the cursor position, line breaks are
    /// indented like the current line.
    pub insert: String,
    pub kind: Kind,
    /// Short description shown next to snippets.
    pub detail: String,
    /// Range to replace, when it differs from the typed word (extension items).
    pub start: Option<usize>,
    pub end: Option<usize>,
    /// Text used for filtering instead of the label.
    pub filter: Option<String>,
}

#[derive(Clone, Debug)]
pub struct Completion {
    pub items: Vec<Item>,
    pub selected: usize,
    /// Char position where the completed prefix starts.
    pub start: usize,
}

enum Context {
    Tag,
    Attribute(String),
    Property,
    Value(String),
    Word,
    None,
}

/// User completions from the settings, per language.
static USER: LazyLock<RwLock<HashMap<Language, Vec<String>>>> = LazyLock::new(Default::default);

pub fn set_user_completions(map: HashMap<Language, Vec<String>>) {
    *USER.write().unwrap() = map;
}

/// Language at the cursor, looking into `<style>`/`<script>` blocks of HTML files.
fn language_at(doc: &Document, before: &str) -> Language {
    if !matches!(doc.language, Language::Html | Language::Php) {
        return doc.language;
    }
    let lower = before.to_ascii_lowercase();
    let open = |tag: &str| lower.rfind(&format!("<{tag}"));
    let close = |tag: &str| lower.rfind(&format!("</{tag}"));
    if open("style") > close("style") && lower[open("style").unwrap()..].contains('>') {
        return Language::Css;
    }
    if open("script") > close("script") && lower[open("script").unwrap()..].contains('>') {
        return Language::JavaScript;
    }
    if doc.language == Language::Php
        && lower.rfind("<?php").or_else(|| lower.rfind("<?=")) > lower.rfind("?>")
    {
        return Language::Php;
    }
    doc.language
}

fn context(language: Language, before: &str, prefix: &str) -> Context {
    match language {
        l if l.is_css() => {
            let last = before.rfind(['{', '}', ';', ':']);
            match last.map(|i| before.as_bytes()[i]) {
                Some(b':') => {
                    let head = &before[..last.unwrap()];
                    let open = head.rfind(['{', ';']).map_or(0, |i| i + 1);
                    let prop = head[open..].trim();
                    if prop.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') && !prop.is_empty() {
                        Context::Value(prop.to_ascii_lowercase())
                    } else {
                        Context::Word
                    }
                }
                Some(b'{') | Some(b';') => {
                    // Nested rules: inside a block a selector also looks like this,
                    // so only offer properties for lowercase identifiers.
                    if prefix.chars().all(|c| c.is_ascii_lowercase() || c == '-') {
                        Context::Property
                    } else {
                        Context::Word
                    }
                }
                _ => Context::Word,
            }
        }
        Language::Html | Language::Xml => {
            let lt = before.rfind('<');
            let gt = before.rfind('>');
            match lt {
                Some(lt) if gt.is_none_or(|gt| gt < lt) => {
                    let inside = &before[lt + 1..];
                    if inside.starts_with(['?', '!', '/']) {
                        return Context::None;
                    }
                    if !inside.contains(char::is_whitespace) {
                        return Context::Tag;
                    }
                    if inside.matches('"').count() % 2 == 1 || inside.matches('\'').count() % 2 == 1 {
                        return Context::None;
                    }
                    let tag: String = inside.chars().take_while(|c| !c.is_whitespace()).collect();
                    Context::Attribute(tag.to_ascii_lowercase())
                }
                _ => Context::Word,
            }
        }
        _ => Context::Word,
    }
}

fn is_prefix_char(c: char) -> bool {
    is_word_char(c) || c == '-'
}

/// Keywords offered for a language.
fn keywords(language: Language) -> &'static [&'static str] {
    match language {
        Language::JavaScript | Language::TypeScript | Language::Tsx => &[
            "async", "await", "break", "case", "catch", "class", "const", "continue", "debugger", "default",
            "delete", "document", "export", "extends", "false", "finally", "for", "function", "if", "import",
            "instanceof", "let", "new", "null", "return", "static", "super", "switch", "this", "throw", "true",
            "try", "typeof", "undefined", "var", "while", "yield", "console", "window", "querySelector",
            "querySelectorAll", "addEventListener", "getElementById", "fetch", "Promise", "JSON", "Array",
            "Object", "String", "Number", "Math", "localStorage", "setTimeout", "setInterval",
            // TypeScript
            "interface", "type", "enum", "implements", "private", "protected", "public", "readonly",
            "abstract", "namespace", "declare", "keyof", "never", "unknown", "any", "string", "number", "boolean",
        ],
        Language::Php => &[
            "abstract", "array", "as", "break", "case", "catch", "class", "clone", "const", "continue",
            "declare", "default", "echo", "else", "elseif", "empty", "endforeach", "endif", "extends", "false",
            "final", "finally", "foreach", "function", "global", "if", "implements", "include", "include_once",
            "instanceof", "interface", "isset", "list", "match", "namespace", "new", "null", "print", "private",
            "protected", "public", "readonly", "require", "require_once", "return", "static", "switch", "throw",
            "trait", "true", "try", "unset", "use", "while", "yield", "count", "strlen", "str_replace",
            "explode", "implode", "in_array", "array_key_exists", "json_encode", "json_decode", "htmlspecialchars",
            "sprintf", "var_dump", "die", "header", "$_GET", "$_POST", "$_SESSION", "$_SERVER", "$this",
        ],
        Language::Python => &[
            "and", "as", "assert", "async", "await", "break", "class", "continue", "def", "del", "elif", "else",
            "except", "False", "finally", "for", "from", "global", "if", "import", "in", "is", "lambda", "None",
            "nonlocal", "not", "or", "pass", "raise", "return", "True", "try", "while", "with", "yield", "print",
            "len", "range", "enumerate", "isinstance", "self", "__init__", "__name__", "dict", "list", "str", "int",
        ],
        Language::Rust => &[
            "as", "async", "await", "break", "const", "continue", "crate", "dyn", "else", "enum", "extern",
            "false", "fn", "for", "if", "impl", "in", "let", "loop", "match", "mod", "move", "mut", "pub", "ref",
            "return", "self", "Self", "static", "struct", "super", "trait", "true", "type", "unsafe", "use",
            "where", "while", "Option", "Some", "None", "Result", "Ok", "Err", "String", "Vec", "Box", "println",
        ],
        Language::Go => &[
            "break", "case", "chan", "const", "continue", "default", "defer", "else", "fallthrough", "for",
            "func", "go", "goto", "if", "import", "interface", "map", "package", "range", "return", "select",
            "struct", "switch", "type", "var", "nil", "true", "false", "string", "int", "error", "make", "append",
        ],
        Language::Java | Language::CSharp => &[
            "abstract", "boolean", "break", "case", "catch", "class", "const", "continue", "default", "do",
            "double", "else", "enum", "extends", "false", "final", "finally", "float", "for", "if", "implements",
            "import", "instanceof", "int", "interface", "long", "namespace", "new", "null", "override", "package",
            "private", "protected", "public", "return", "static", "string", "String", "super", "switch", "this",
            "throw", "true", "try", "using", "var", "virtual", "void", "while", "async", "await",
        ],
        Language::C | Language::Cpp => &[
            "auto", "break", "case", "char", "class", "const", "constexpr", "continue", "default", "delete",
            "double", "else", "enum", "extern", "float", "for", "if", "include", "inline", "int", "long",
            "namespace", "new", "nullptr", "private", "protected", "public", "return", "short", "signed",
            "sizeof", "static", "struct", "switch", "template", "typedef", "typename", "union", "unsigned",
            "using", "virtual", "void", "volatile", "while", "std", "printf", "malloc", "free",
        ],
        Language::Ruby => &[
            "alias", "and", "begin", "break", "case", "class", "def", "defined?", "do", "else", "elsif", "end",
            "ensure", "false", "for", "if", "in", "module", "next", "nil", "not", "or", "puts", "redo", "require",
            "rescue", "retry", "return", "self", "super", "then", "true", "undef", "unless", "until", "when",
            "while", "yield", "attr_accessor", "each", "map",
        ],
        Language::Shell => &[
            "if", "then", "else", "elif", "fi", "for", "in", "do", "done", "while", "until", "case", "esac",
            "function", "return", "export", "local", "echo", "printf", "read", "source", "exit", "cd", "grep",
            "sed", "awk", "chmod", "mkdir", "rm", "cp", "mv",
        ],
        Language::Sql => &[
            "SELECT", "FROM", "WHERE", "INSERT", "INTO", "VALUES", "UPDATE", "SET", "DELETE", "CREATE",
            "TABLE", "ALTER", "DROP", "INDEX", "JOIN", "LEFT", "RIGHT", "INNER", "OUTER", "ON", "GROUP", "BY",
            "ORDER", "ASC", "DESC", "LIMIT", "OFFSET", "AND", "OR", "NOT", "NULL", "IS", "IN", "LIKE", "AS",
            "DISTINCT", "COUNT", "SUM", "AVG", "MIN", "MAX", "HAVING", "PRIMARY", "KEY", "FOREIGN",
            "REFERENCES", "DEFAULT", "AUTO_INCREMENT", "VARCHAR", "INT", "TEXT", "DATETIME", "BOOLEAN",
        ],
        Language::Lua => &[
            "and", "break", "do", "else", "elseif", "end", "false", "for", "function", "goto", "if", "in",
            "local", "nil", "not", "or", "repeat", "return", "then", "true", "until", "while", "print", "pairs",
            "ipairs", "require", "table", "string",
        ],
        _ => &[],
    }
}

/// Built-in snippets: (trigger, expansion, description).
fn snippets(language: Language) -> &'static [(&'static str, &'static str, &'static str)] {
    match language {
        Language::Html | Language::Php => &[
            (
                "html5",
                "<!DOCTYPE html>\n<html lang=\"de\">\n<head>\n    <meta charset=\"utf-8\">\n    <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n    <title>$0</title>\n    <link rel=\"stylesheet\" href=\"style.css\">\n</head>\n<body>\n    \n</body>\n</html>",
                "HTML5-Grundgerüst",
            ),
            ("linkcss", "<link rel=\"stylesheet\" href=\"$0\">", "Stylesheet einbinden"),
            ("scriptsrc", "<script src=\"$0\"></script>", "Skript einbinden"),
            ("viewport", "<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">$0", "Viewport-Meta"),
            ("img", "<img src=\"$0\" alt=\"\">", "Bild"),
            ("ahref", "<a href=\"$0\"></a>", "Link"),
            ("ul", "<ul>\n    <li>$0</li>\n</ul>", "Liste"),
            ("table", "<table>\n    <tr>\n        <td>$0</td>\n    </tr>\n</table>", "Tabelle"),
            ("form", "<form action=\"$0\" method=\"post\">\n    \n</form>", "Formular"),
            ("lorem", "Lorem ipsum dolor sit amet, consectetur adipiscing elit, sed do eiusmod tempor incididunt ut labore et dolore magna aliqua.$0", "Blindtext"),
            ("php", "<?php $0 ?>", "PHP-Block"),
            ("echo", "<?= $0 ?>", "PHP-Ausgabe"),
        ],
        Language::Css | Language::Scss | Language::Less => &[
            ("media", "@media (max-width: $0) {\n    \n}", "Media Query"),
            ("flexcenter", "display: flex;\nalign-items: center;\njustify-content: center;$0", "Flex zentriert"),
            ("grid", "display: grid;\ngrid-template-columns: repeat($0, 1fr);\ngap: 1rem;", "Raster"),
            ("fontface", "@font-face {\n    font-family: \"$0\";\n    src: url(\"\") format(\"woff2\");\n}", "Schrift einbinden"),
            ("keyframes", "@keyframes $0 {\n    from {  }\n    to {  }\n}", "Animation"),
            ("transition", "transition: all 0.2s ease$0;", "Übergang"),
        ],
        Language::JavaScript | Language::TypeScript | Language::Tsx => &[
            ("log", "console.log($0);", "Konsolenausgabe"),
            ("fn", "function $0() {\n    \n}", "Funktion"),
            ("arrow", "($0) => {\n    \n}", "Pfeilfunktion"),
            ("for", "for (let i = 0; i < $0; i++) {\n    \n}", "Zählschleife"),
            ("forof", "for (const item of $0) {\n    \n}", "for…of"),
            ("if", "if ($0) {\n    \n}", "Bedingung"),
            ("trycatch", "try {\n    $0\n} catch (error) {\n    console.error(error);\n}", "try/catch"),
            ("qs", "document.querySelector('$0')", "querySelector"),
            ("ael", "addEventListener('$0', (event) => {\n    \n});", "Event-Listener"),
            ("fetchjson", "const response = await fetch('$0');\nconst data = await response.json();", "fetch + JSON"),
        ],
        Language::Python => &[
            ("main", "if __name__ == \"__main__\":\n    $0", "Einstiegspunkt"),
            ("def", "def $0():\n    pass", "Funktion"),
            ("class", "class $0:\n    def __init__(self):\n        pass", "Klasse"),
        ],
        Language::Markdown => &[
            ("link", "[$0]()", "Link"),
            ("img", "![$0]()", "Bild"),
            ("code", "```\n$0\n```", "Codeblock"),
            ("table", "| $0 |  |\n| --- | --- |\n|  |  |", "Tabelle"),
        ],
        _ => &[],
    }
}

/// Parses a user completion line: `word` or `trigger => expansion`.
fn parse_user(line: &str) -> Option<Item> {
    let line = line.trim();
    if line.is_empty() || line.starts_with("//") {
        return None;
    }
    Some(match line.split_once("=>") {
        Some((trigger, expansion)) => Item {
            label: trigger.trim().to_string(),
            insert: expansion.trim().replace("\\n", "\n").replace("\\t", "\t"),
            kind: Kind::Snippet,
            detail: "Eigenes Snippet".into(),
            start: None,
            end: None,
            filter: None,
        },
        None => Item { label: line.to_string(), insert: line.to_string(), kind: Kind::Keyword, detail: String::new(), start: None, end: None, filter: None },
    })
}

/// Computes completions for the primary cursor, if any apply: the built-in
/// ones plus items from extensions for the word being typed.
pub fn compute(doc: &mut Document) -> Option<Completion> {
    let builtin = compute_builtin(doc);
    let Some((anchor, ext)) = doc.view.ext_items.clone() else { return builtin };
    if doc.selections.len() != 1 || !doc.selections[0].is_empty() {
        return builtin;
    }
    let head = doc.selections[0].head;
    if head < anchor || !doc.slice(anchor..head).chars().all(|c| is_prefix_char(c) || c == '$') {
        doc.view.ext_items = None;
        return builtin;
    }
    let mut word_start = head;
    while word_start > 0 && doc.char_at(word_start - 1).is_some_and(|c| is_prefix_char(c) || c == '$') {
        word_start -= 1;
    }
    let mut ranked: Vec<(u8, Item)> = Vec::new();
    for item in ext {
        let s = item.start.unwrap_or(word_start).min(head);
        let typed = doc.slice(s..head).to_lowercase();
        let key = item.filter.as_deref().unwrap_or(&item.label).to_lowercase();
        let class = if typed.is_empty() || key.starts_with(&typed) {
            0
        } else if typed.len() >= 2 && key.contains(&typed) {
            1
        } else {
            continue;
        };
        ranked.push((class, item));
    }
    ranked.sort_by_key(|(c, _)| *c);
    let mut out = builtin.unwrap_or(Completion { items: Vec::new(), selected: 0, start: word_start });
    let known: HashSet<String> = out.items.iter().map(|i| i.label.clone()).collect();
    // Extension items go first: they come from language-aware tools.
    let ext_items: Vec<Item> = ranked.into_iter().map(|(_, i)| i).filter(|i| !known.contains(&i.label)).take(60).collect();
    let builtin_items = std::mem::take(&mut out.items);
    out.items = ext_items.into_iter().chain(builtin_items.into_iter().filter(|i| i.kind != Kind::Word)).take(80).collect();
    (!out.items.is_empty()).then_some(out)
}

fn compute_builtin(doc: &mut Document) -> Option<Completion> {
    if doc.selections.len() != 1 || !doc.selections[0].is_empty() {
        return None;
    }
    let pos = doc.selections[0].head;
    let mut start = pos;
    while start > 0 && doc.char_at(start - 1).is_some_and(|c| is_prefix_char(c) || c == '$') {
        start -= 1;
    }
    let prefix = doc.slice(start..pos);
    if prefix.is_empty() || prefix.starts_with('-') && prefix.len() < 2 {
        return None;
    }
    // The whole word at the cursor (it is no suggestion for itself).
    let mut end = pos;
    while doc.char_at(end).is_some_and(|c| is_prefix_char(c) || c == '$') {
        end += 1;
    }
    let current = doc.slice(start..end);
    let lower = prefix.to_lowercase();
    let before = doc.slice(start.saturating_sub(20_000)..start);
    let language = language_at(doc, &before);

    let mut items: Vec<Item> = Vec::new();
    let mut push = |label: &str, insert: String, kind: Kind| {
        items.push(Item { label: label.to_string(), insert, kind, detail: String::new(), start: None, end: None, filter: None });
    };
    let ctx = context(language, &before, &prefix);
    match &ctx {
        Context::None => return None,
        Context::Tag => {
            for t in HTML_TAGS {
                push(t, t.to_string(), Kind::Tag);
            }
        }
        Context::Attribute(tag) => {
            for a in tag_attrs(tag).iter().chain(HTML_GLOBAL_ATTRS) {
                push(a, format!("{a}=\"$0\""), Kind::Attribute);
            }
        }
        Context::Property => {
            let next = doc.char_at(pos);
            for p in CSS_PROPERTIES {
                let insert = if next == Some(':') { p.to_string() } else { format!("{p}: ") };
                push(p, insert, Kind::Property);
            }
        }
        Context::Value(prop) => {
            for v in css_values(prop).iter().chain(CSS_GLOBAL_VALUES) {
                let insert = v.strip_suffix("()").map_or_else(|| v.to_string(), |f| format!("{f}($0)"));
                push(v, insert, Kind::Value);
            }
        }
        Context::Word => {
            for k in keywords(language) {
                push(k, k.to_string(), Kind::Keyword);
            }
        }
    }
    // Snippets work everywhere except inside tags and CSS values.
    if matches!(ctx, Context::Word | Context::Property) {
        for (trigger, body, detail) in snippets(language) {
            items.push(Item {
                label: trigger.to_string(),
                insert: body.to_string(),
                kind: Kind::Snippet,
                detail: detail.to_string(),
                start: None,
                end: None,
                filter: None,
            });
        }
        if let Some(user) = USER.read().unwrap().get(&language) {
            items.extend(user.iter().filter_map(|l| parse_user(l)));
        }
    }

    // Document words complement every context.
    if lower.chars().count() >= 2 && doc.len_chars() < 4_000_000 {
        let text = doc.text();
        let mut seen: HashSet<&str> = HashSet::new();
        let known: HashSet<String> = items.iter().map(|i| i.label.to_lowercase()).collect();
        let mut words = Vec::new();
        for word in text.split(|c: char| !(is_prefix_char(c) || c == '$')) {
            if word.chars().count() > prefix.chars().count()
                && word.len() >= 3
                && word != prefix
                && word != current
                && word.to_lowercase().starts_with(&lower)
                && !known.contains(&word.to_lowercase())
                && seen.insert(word)
            {
                words.push(word.to_string());
                if words.len() >= 30 {
                    break;
                }
            }
        }
        for w in words {
            items.push(Item { label: w.clone(), insert: w, kind: Kind::Word, detail: String::new(), start: None, end: None, filter: None });
        }
    }

    // Filter: prefix matches first, then substring matches.
    let mut ranked: Vec<(u8, Item)> = items
        .into_iter()
        .filter_map(|item| {
            let l = item.label.to_lowercase();
            if l == lower && item.kind != Kind::Snippet {
                None
            } else if l.starts_with(&lower) {
                Some((0, item))
            } else if lower.len() >= 2 && l.contains(&lower) {
                Some((1, item))
            } else {
                None
            }
        })
        .collect();
    ranked.sort_by_key(|(class, _)| *class);
    let items: Vec<Item> = ranked.into_iter().map(|(_, i)| i).take(40).collect();
    (!items.is_empty()).then_some(Completion { items, selected: 0, start })
}

/// Replaces the prefix with the selected item.
pub fn accept(doc: &mut Document, completion: &Completion) {
    let Some(item) = completion.items.get(completion.selected) else { return };
    let pos = doc.primary_selection().head;
    let start = item.start.unwrap_or(completion.start).min(pos);
    // Also replace the rest of a word the cursor is in the middle of.
    let mut end = item.end.map_or(pos, |e| e.max(pos)).min(doc.len_chars());
    while doc.char_at(end).is_some_and(is_prefix_char) {
        end += 1;
    }
    // Indent continuation lines like the current line.
    let line = doc.char_to_line(start);
    let line_text = doc.line_text(line);
    let indent: String = line_text.chars().take_while(|c| *c == ' ' || *c == '\t').collect();
    let text = item.insert.replace('\n', &format!("\n{indent}"));
    if item.kind == Kind::Snippet {
        doc.view.ext_items = None;
        let vars = super::snippet::Vars { filename: doc.title(), ..Default::default() };
        doc.insert_snippet(start..end, super::snippet::expand(&text, &vars));
        return;
    }
    let (text, cursor) = match text.find("$0") {
        Some(i) => (text.replacen("$0", "", 1), start + text[..i].chars().count()),
        None => {
            let n = text.chars().count();
            (text, start + n)
        }
    };
    doc.view.ext_items = None;
    doc.apply(
        vec![Change::new(start..end, text)],
        Some(vec![Selection::cursor(cursor)]),
        EditKind::Other,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::editor::document::DocSource;
    use std::path::PathBuf;

    fn at_end(name: &str, text: &str) -> Document {
        let mut d = Document::new(DocSource::Local(PathBuf::from(name)), text);
        let n = d.len_chars();
        d.set_selection(Selection::cursor(n));
        d
    }

    fn labels(c: &Completion) -> Vec<&str> {
        c.items.iter().map(|i| i.label.as_str()).collect()
    }

    #[test]
    fn css_property_and_value() {
        let mut d = at_end("a.css", "a { dis");
        let c = compute(&mut d).unwrap();
        assert_eq!(c.items[0].label, "display");
        accept(&mut d, &c);
        assert_eq!(d.rope.to_string(), "a { display: ");

        let mut d = at_end("a.css", "a { display: fl");
        let c = compute(&mut d).unwrap();
        assert!(labels(&c).contains(&"flex"));
    }

    #[test]
    fn html_tags_and_attributes() {
        let mut d = at_end("a.html", "<p>x</p>\n<sec");
        let c = compute(&mut d).unwrap();
        assert_eq!(c.items[0].label, "section");

        let mut d = at_end("a.html", "<a hr");
        let c = compute(&mut d).unwrap();
        assert_eq!(c.items[0].label, "href");
        accept(&mut d, &c);
        assert_eq!(d.rope.to_string(), "<a href=\"\"");
        assert_eq!(d.primary_selection().head, 9);
    }

    #[test]
    fn css_inside_style_block() {
        let mut d = at_end("a.html", "<style>\n.x { backg");
        let c = compute(&mut d).unwrap();
        assert!(labels(&c).contains(&"background"));
    }

    #[test]
    fn keywords_snippets_and_user_entries() {
        let mut d = at_end("a.py", "def f():
    ret");
        let c = compute(&mut d).unwrap();
        assert_eq!(c.items[0].label, "return");

        // Snippet: indentation follows the line, cursor lands on $0.
        let mut d = at_end("a.js", "  if (x) {
    log");
        let c = compute(&mut d).unwrap();
        let i = c.items.iter().position(|i| i.label == "log").unwrap();
        accept(&mut d, &Completion { selected: i, ..c });
        assert_eq!(d.rope.to_string(), "  if (x) {
    console.log();");
        assert_eq!(d.primary_selection().head, d.len_chars() - 2);

        let mut map = HashMap::new();
        map.insert(Language::Css, vec!["brand => color: #c00;$0".to_string()]);
        set_user_completions(map);
        let mut d = at_end("a.css", "a { bra");
        let c = compute(&mut d).unwrap();
        let i = c.items.iter().position(|i| i.label == "brand").unwrap();
        accept(&mut d, &Completion { selected: i, ..c });
        assert_eq!(d.rope.to_string(), "a { color: #c00;");
        set_user_completions(HashMap::new());
    }

    #[test]
    fn nothing_inside_attribute_values() {
        let mut d = at_end("a.html", "<a href=\"ab");
        assert!(compute(&mut d).is_none_or(|c| c.items.iter().all(|i| i.kind == Kind::Word)));
    }
}
