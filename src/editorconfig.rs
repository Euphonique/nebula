//! `.editorconfig` support: indentation, line endings, charset and
//! whitespace rules per file, as defined by https://editorconfig.org.

use std::path::Path;

use crate::editor::document::{Encoding, LineEnding};

/// Properties that apply to one file. `None`: not set (use the settings).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Props {
    pub use_spaces: Option<bool>,
    pub tab_size: Option<usize>,
    pub end_of_line: Option<LineEnding>,
    pub charset: Option<Encoding>,
    pub trim_trailing_whitespace: Option<bool>,
    pub insert_final_newline: Option<bool>,
}

/// Collects the properties for `file` from all `.editorconfig` files above it.
pub fn for_file(file: &Path) -> Props {
    let mut configs = Vec::new();
    let mut dir = file.parent();
    while let Some(d) = dir {
        if let Ok(text) = std::fs::read_to_string(d.join(".editorconfig")) {
            let root = is_root(&text);
            configs.push((d.to_path_buf(), text));
            if root {
                break;
            }
        }
        dir = d.parent();
    }
    let mut raw: Vec<(String, String)> = Vec::new();
    for (dir, text) in configs.iter().rev() {
        let Ok(rel) = file.strip_prefix(dir) else { continue };
        let rel = rel.to_string_lossy().replace('\\', "/");
        apply_sections(text, &rel, &mut raw);
    }
    resolve(&raw)
}

fn parse_line(line: &str) -> Option<(&str, &str)> {
    let (k, v) = line.split_once(['=', ':'])?;
    Some((k.trim(), v.trim()))
}

fn is_root(text: &str) -> bool {
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            return false;
        }
        if let Some((k, v)) = parse_line(line)
            && k.eq_ignore_ascii_case("root")
        {
            return v.eq_ignore_ascii_case("true");
        }
    }
    false
}

/// Adds the key/value pairs of all sections matching `rel` (later wins).
fn apply_sections(text: &str, rel: &str, out: &mut Vec<(String, String)>) {
    let mut active = false;
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
            continue;
        }
        if let Some(section) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            active = glob_matches(section, rel);
            continue;
        }
        if !active {
            continue;
        }
        if let Some((k, v)) = parse_line(line) {
            let k = k.to_ascii_lowercase();
            out.retain(|(key, _)| *key != k);
            out.push((k, v.to_ascii_lowercase()));
        }
    }
}

fn resolve(raw: &[(String, String)]) -> Props {
    let get = |key: &str| raw.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str()).filter(|v| *v != "unset");
    let bool_of = |key: &str| match get(key) {
        Some("true") => Some(true),
        Some("false") => Some(false),
        _ => None,
    };
    let use_spaces = match get("indent_style") {
        Some("space") => Some(true),
        Some("tab") => Some(false),
        _ => None,
    };
    let indent_size = get("indent_size").and_then(|v| v.parse::<usize>().ok());
    let tab_width = get("tab_width").and_then(|v| v.parse::<usize>().ok());
    let tab_size = match use_spaces {
        Some(false) => tab_width.or(indent_size),
        _ => indent_size.or(tab_width),
    }
    .filter(|&n| (1..=16).contains(&n));
    Props {
        use_spaces,
        tab_size,
        end_of_line: match get("end_of_line") {
            Some("lf") => Some(LineEnding::Lf),
            Some("crlf") => Some(LineEnding::CrLf),
            _ => None,
        },
        charset: match get("charset") {
            Some("utf-8") => Some(Encoding::Utf8),
            Some("utf-8-bom") => Some(Encoding::Utf8Bom),
            Some("latin1") => Some(Encoding::Latin1),
            Some("utf-16le") => Some(Encoding::Utf16Le),
            Some("utf-16be") => Some(Encoding::Utf16Be),
            _ => None,
        },
        trim_trailing_whitespace: bool_of("trim_trailing_whitespace"),
        insert_final_newline: bool_of("insert_final_newline"),
    }
}

/// Matches an EditorConfig glob against a path relative to the config's folder.
fn glob_matches(glob: &str, rel: &str) -> bool {
    let (anchored, glob) = match glob.strip_prefix('/') {
        Some(g) => (true, g),
        None => (glob.contains('/'), glob),
    };
    let mut re = String::from(if anchored { "^" } else { "^(?:.*/)?" });
    let chars: Vec<char> = glob.chars().collect();
    let mut i = 0;
    let mut braces = 0;
    while i < chars.len() {
        let c = chars[i];
        match c {
            '*' if chars.get(i + 1) == Some(&'*') => {
                re.push_str(".*");
                i += 1;
            }
            '*' => re.push_str("[^/]*"),
            '?' => re.push_str("[^/]"),
            '[' => match chars[i..].iter().position(|&c| c == ']') {
                Some(end) => {
                    let inner: String = chars[i + 1..i + end].iter().collect();
                    let inner = inner.strip_prefix('!').map(|s| format!("^{s}")).unwrap_or(inner);
                    re.push('[');
                    re.push_str(&inner.replace('\\', "\\\\"));
                    re.push(']');
                    i += end;
                }
                None => re.push_str("\\["),
            },
            '{' => match chars[i..].iter().position(|&c| c == '}') {
                // {1..10}: any number.
                Some(end) if chars[i + 1..i + end].iter().collect::<String>().contains("..") => {
                    re.push_str("-?[0-9]+");
                    i += end;
                }
                Some(_) => {
                    braces += 1;
                    re.push_str("(?:");
                }
                None => re.push_str("\\{"),
            },
            '}' if braces > 0 => {
                braces -= 1;
                re.push(')');
            }
            ',' if braces > 0 => re.push('|'),
            '\\' if i + 1 < chars.len() => {
                i += 1;
                re.push_str(&regex::escape(&chars[i].to_string()));
            }
            c => re.push_str(&regex::escape(&c.to_string())),
        }
        i += 1;
    }
    re.push('$');
    regex::Regex::new(&re).is_ok_and(|r| r.is_match(rel))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn globs() {
        assert!(glob_matches("*", "a/b/c.php"));
        assert!(glob_matches("*.{js,ts}", "src/app.ts"));
        assert!(!glob_matches("*.{js,ts}", "src/app.css"));
        assert!(glob_matches("lib/**.js", "lib/x/y.js"));
        assert!(!glob_matches("lib/*.js", "lib/x/y.js"));
        assert!(glob_matches("/Makefile", "Makefile"));
        assert!(!glob_matches("/Makefile", "sub/Makefile"));
        assert!(glob_matches("[Mm]akefile", "sub/makefile"));
        assert!(glob_matches("*.md", "README.md"));
    }

    #[test]
    fn nested_files_and_root() {
        let base = std::env::temp_dir().join(format!("nebula-ec-{}", std::process::id()));
        let sub = base.join("proj").join("src");
        std::fs::create_dir_all(&sub).unwrap();
        std::fs::write(base.join(".editorconfig"), "[*]\nindent_size = 8\n").unwrap();
        std::fs::write(
            base.join("proj").join(".editorconfig"),
            "root = true\n\n[*]\nindent_style = space\nindent_size = 2\ninsert_final_newline = true\n\n[*.php]\nindent_style = tab\ntab_width = 4\n",
        )
        .unwrap();
        std::fs::write(sub.join(".editorconfig"), "[*.js]\nindent_size = 3\ntrim_trailing_whitespace = true\n").unwrap();
        let js = for_file(&sub.join("a.js"));
        assert_eq!(js.use_spaces, Some(true));
        assert_eq!(js.tab_size, Some(3));
        assert_eq!(js.trim_trailing_whitespace, Some(true));
        assert_eq!(js.insert_final_newline, Some(true));
        let php = for_file(&sub.join("a.php"));
        assert_eq!(php.use_spaces, Some(false));
        assert_eq!(php.tab_size, Some(4));
        let _ = std::fs::remove_dir_all(&base);
    }
}
