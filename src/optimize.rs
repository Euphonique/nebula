//! Optimizing files on their way to the server (set per server): CSS is
//! minified, JavaScript loses comments and indentation, very large images
//! are scaled down. The local files stay as they are. Everything is built
//! in – no tools need to be installed.

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Optimize {
    pub css: bool,
    pub js: bool,
    pub images: bool,
    /// Wider (or taller) images are scaled down to this size.
    pub max_image: u32,
}

impl Default for Optimize {
    fn default() -> Self {
        Self { css: false, js: false, images: false, max_image: 2560 }
    }
}

impl Optimize {
    pub fn any(&self) -> bool {
        self.css || self.js || self.images
    }
}

/// The content to upload for a file named `name`; unchanged if nothing
/// applies or the result would not be smaller.
pub fn apply(name: &str, data: Vec<u8>, opt: &Optimize) -> Vec<u8> {
    if !opt.any() {
        return data;
    }
    let lower = name.to_ascii_lowercase();
    // Already minified files are left alone.
    if lower.ends_with(".min.css") || lower.ends_with(".min.js") {
        return data;
    }
    let out = if opt.css && lower.ends_with(".css") {
        std::str::from_utf8(&data).ok().map(|s| css(s).into_bytes())
    } else if opt.js && (lower.ends_with(".js") || lower.ends_with(".mjs")) {
        std::str::from_utf8(&data).ok().map(|s| js(s).into_bytes())
    } else if opt.images && [".jpg", ".jpeg", ".png"].iter().any(|e| lower.ends_with(e)) {
        image(&data, lower.ends_with(".png"), opt.max_image)
    } else {
        None
    };
    match out {
        Some(o) if o.len() < data.len() => o,
        _ => data,
    }
}

/// Minified CSS: no comments, no unneeded whitespace. Strings and `url()`
/// stay untouched.
pub fn css(src: &str) -> String {
    let b = src.as_bytes();
    let mut out = String::with_capacity(src.len());
    let mut i = 0;
    let mut pending_space = false;
    while i < b.len() {
        let c = b[i];
        if c == b'/' && b.get(i + 1) == Some(&b'*') {
            // `/*! … */` (licenses) stays.
            let end = src[i + 2..].find("*/").map_or(b.len(), |e| i + 2 + e + 2);
            if b.get(i + 2) == Some(&b'!') {
                out.push_str(&src[i..end]);
            }
            i = end;
            pending_space = !out.is_empty();
            continue;
        }
        if c.is_ascii_whitespace() {
            pending_space = !out.is_empty();
            i += 1;
            continue;
        }
        let quoted = |q: u8, i: usize| -> usize {
            let mut j = i + 1;
            while j < b.len() && b[j] != q {
                j += if b[j] == b'\\' { 2 } else { 1 };
            }
            (j + 1).min(b.len())
        };
        let punct = |c: u8| matches!(c, b'{' | b'}' | b';' | b':' | b',' | b'>' | b'~' | b'(' | b')');
        if pending_space {
            let last = out.as_bytes().last().copied().unwrap_or(b'{');
            // Selectors like `a :hover` need the space; properties don't.
            let keep = !(punct(last) || punct(c) && c != b'(' && !(c == b':' && !in_block(&out)) || c == b'!');
            if keep {
                out.push(' ');
            }
            pending_space = false;
        }
        match c {
            b'"' | b'\'' => {
                let end = quoted(c, i);
                out.push_str(&src[i..end]);
                i = end;
            }
            b'}' if out.ends_with(';') => {
                out.pop();
                out.push('}');
                i += 1;
            }
            _ if src[i..].len() >= 4 && src[i..i + 4].eq_ignore_ascii_case("url(") => {
                let end = src[i..].find(')').map_or(b.len(), |e| i + e + 1);
                out.push_str(&src[i..end]);
                i = end;
            }
            _ => {
                let ch = src[i..].chars().next().unwrap();
                out.push(ch);
                i += ch.len_utf8();
            }
        }
    }
    out
}

/// Inside a declaration block (`{` open): `a:b` there, selectors outside.
fn in_block(out: &str) -> bool {
    out.rfind('{') > out.rfind('}')
}

/// JavaScript without comments, indentation and empty lines. Line breaks
/// stay (automatic semicolons), strings, template literals and regular
/// expressions stay untouched.
pub fn js(src: &str) -> String {
    let b = src.as_bytes();
    let mut out = String::with_capacity(src.len());
    let mut i = 0;
    // Whether a `/` here starts a regular expression (no value before it).
    let regex_allowed = |out: &str| {
        let t = out.trim_end();
        match t.chars().last() {
            None => true,
            Some(c) if "(,=:[!&|?{};+-*%<>~^\n".contains(c) => true,
            Some(c) if c.is_alphanumeric() || c == '_' || c == '$' => {
                let word: String = t.chars().rev().take_while(|c| c.is_alphanumeric() || *c == '_' || *c == '$').collect::<Vec<_>>().into_iter().rev().collect();
                matches!(word.as_str(), "return" | "typeof" | "instanceof" | "in" | "of" | "new" | "delete" | "void" | "throw" | "case" | "do" | "else" | "yield" | "await")
            }
            _ => false,
        }
    };
    let skip_quoted = |q: u8, i: usize| -> usize {
        let mut j = i + 1;
        while j < b.len() && b[j] != q {
            j += if b[j] == b'\\' { 2 } else { 1 };
        }
        (j + 1).min(b.len())
    };
    let mut line_start = true;
    while i < b.len() {
        let c = b[i];
        match c {
            b'/' if b.get(i + 1) == Some(&b'/') => {
                i = src[i..].find('\n').map_or(b.len(), |e| i + e);
            }
            b'/' if b.get(i + 1) == Some(&b'*') => {
                let end = src[i + 2..].find("*/").map_or(b.len(), |e| i + 2 + e + 2);
                if b.get(i + 2) == Some(&b'!') {
                    out.push_str(&src[i..end]);
                } else if src[i..end].contains('\n') && !out.ends_with('\n') && !out.is_empty() {
                    out.push('\n');
                } else if !out.ends_with([' ', '\n']) && !out.is_empty() {
                    out.push(' ');
                }
                i = end;
            }
            b'"' | b'\'' | b'`' => {
                let end = skip_quoted(c, i);
                out.push_str(&src[i..end]);
                i = end;
                line_start = false;
            }
            b'/' if regex_allowed(&out) => {
                // A regular expression: up to the closing `/` outside a class.
                let mut j = i + 1;
                let mut class = false;
                while j < b.len() && b[j] != b'\n' {
                    match b[j] {
                        b'\\' => j += 1,
                        b'[' => class = true,
                        b']' => class = false,
                        b'/' if !class => break,
                        _ => {}
                    }
                    j += 1;
                }
                let end = (j + 1).min(b.len());
                out.push_str(&src[i..end]);
                i = end;
                line_start = false;
            }
            b'\n' | b'\r' => {
                if !out.is_empty() && !out.ends_with('\n') {
                    // Trailing spaces go.
                    while out.ends_with(' ') || out.ends_with('\t') {
                        out.pop();
                    }
                    out.push('\n');
                }
                line_start = true;
                i += 1;
            }
            b' ' | b'\t' => {
                if !line_start && !out.ends_with([' ', '\n']) {
                    out.push(' ');
                }
                i += 1;
            }
            _ => {
                let ch = src[i..].chars().next().unwrap();
                out.push(ch);
                i += ch.len_utf8();
                line_start = false;
            }
        }
    }
    while out.ends_with(['\n', ' ']) {
        out.pop();
    }
    out.push('\n');
    out
}

/// A smaller version of an image larger than `max` pixels in either direction.
fn image(data: &[u8], png: bool, max: u32) -> Option<Vec<u8>> {
    let img = image::load_from_memory(data).ok()?;
    if img.width() <= max && img.height() <= max {
        return None;
    }
    let small = img.resize(max, max, image::imageops::FilterType::Lanczos3);
    let mut out = std::io::Cursor::new(Vec::new());
    if png {
        small.write_to(&mut out, image::ImageFormat::Png).ok()?;
    } else {
        let enc = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, 85);
        small.to_rgb8().write_with_encoder(enc).ok()?;
    }
    Some(out.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn minifies_css() {
        let src = "/* Kopf */\n.nav  a:hover ,\n.nav > li {\n  color : red ;\n  background: url( \"a b.png\" );\n  font-family: \"Open  Sans\", sans-serif;\n}\n\n@media (max-width: 600px) {\n  a { margin: 0 auto !important; }\n}\n/*! Lizenz */";
        assert_eq!(
            css(src),
            ".nav a:hover,.nav>li{color:red;background:url( \"a b.png\" );font-family:\"Open  Sans\",sans-serif}@media (max-width:600px){a{margin:0 auto!important}}/*! Lizenz */"
        );
        // Descendant pseudo selectors keep their space.
        assert_eq!(css("div :first-child { x: y }"), "div :first-child{x:y}");
    }

    #[test]
    fn strips_js() {
        let src = "// Kopf\nfunction a(x) {\n    // innen\n    const re = /\\/\\/ kein Kommentar/g;   \n    const s = \"http://x\"; /* weg */ const t = `a // b\n  c`;\n\n    return x / 2 / re.lastIndex;\n}\n";
        assert_eq!(js(src), "function a(x) {\nconst re = /\\/\\/ kein Kommentar/g;\nconst s = \"http://x\"; const t = `a // b\n  c`;\nreturn x / 2 / re.lastIndex;\n}\n");
        // Division after a value is no regular expression.
        assert_eq!(js("a = b / c; // x\n"), "a = b / c;\n");
    }

    #[test]
    fn leaves_other_files_and_minified_ones() {
        let opt = Optimize { css: true, js: true, images: true, max_image: 100 };
        assert_eq!(apply("a.min.css", b"a { x: y }".to_vec(), &opt), b"a { x: y }");
        assert_eq!(apply("a.php", b"<?php  ?>".to_vec(), &opt), b"<?php  ?>");
        assert_eq!(apply("a.css", b"a { x: y }".to_vec(), &opt), b"a{x:y}");
        assert_eq!(apply("a.css", b"a { x: y }".to_vec(), &Optimize::default()), b"a { x: y }");
        // Images: only larger than the limit, and only when smaller afterwards.
        let mut big = std::io::Cursor::new(Vec::new());
        image::DynamicImage::new_rgb8(400, 200).write_to(&mut big, image::ImageFormat::Png).unwrap();
        let out = apply("b.png", big.get_ref().clone(), &opt);
        let small = image::load_from_memory(&out).unwrap();
        assert_eq!((small.width(), small.height()), (100, 50));
    }
}
