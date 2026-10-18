//! Nova/TextMate snippet format: `$1`, `${1:default}`, `${1|a,b|}`, `$0`,
//! variables like `$FILENAME`. Expanded to plain text plus the selection
//! for the first tab stop.

use std::ops::Range;

pub struct Expanded {
    pub text: String,
    /// Char range to select after inserting (empty for a cursor).
    pub select: Range<usize>,
}

/// Values for snippet variables.
#[derive(Default)]
pub struct Vars {
    pub selected_text: String,
    pub filename: String,
    pub file_path: String,
    pub parent_folder: String,
    pub workspace_name: String,
}

impl Vars {
    fn get(&self, name: &str) -> Option<String> {
        Some(match name {
            "SELECTED_TEXT" | "TM_SELECTED_TEXT" => self.selected_text.clone(),
            "FILENAME" | "TM_FILENAME" => self.filename.clone(),
            "FILE_PATH" | "TM_FILEPATH" => self.file_path.clone(),
            "PARENT_FOLDER" | "TM_DIRECTORY" => self.parent_folder.clone(),
            "WORKSPACE_NAME" => self.workspace_name.clone(),
            "DATE" => {
                let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
                let days = (secs / 86_400) as i64;
                let (y, m, d) = civil(days);
                format!("{d:02}.{m:02}.{y}")
            }
            "AUTHOR_NAME" => std::env::var("USERNAME").unwrap_or_default(),
            "PREVIOUS_TEXT" | "PASTEBOARD_TEXT" | "SCM_REVISION" => String::new(),
            _ => return None,
        })
    }
}

/// Days since 1970-01-01 to (year, month, day).
fn civil(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = (if mp < 10 { mp + 3 } else { mp - 9 }) as u32;
    (yoe + era * 400 + if m <= 2 { 1 } else { 0 }, m, d)
}

pub fn expand(snippet: &str, vars: &Vars) -> Expanded {
    let chars: Vec<char> = snippet.chars().collect();
    let mut out = String::new();
    let mut out_len = 0usize;
    // (tab stop, start, end) in chars of `out`.
    let mut stops: Vec<(u32, usize, usize)> = Vec::new();
    parse(&chars, &mut 0, &mut out, &mut out_len, &mut stops, vars, false);
    // First stop: $1 (lowest positive), else $0, else the end.
    let first = stops
        .iter()
        .filter(|s| s.0 > 0)
        .min_by_key(|s| (s.0, s.1))
        .or_else(|| stops.iter().find(|s| s.0 == 0))
        .map_or(out_len..out_len, |s| s.1..s.2);
    Expanded { text: out, select: first }
}

fn push(out: &mut String, len: &mut usize, c: char) {
    out.push(c);
    *len += 1;
}

/// Parses until the end (or a `}` when `nested`).
fn parse(chars: &[char], i: &mut usize, out: &mut String, len: &mut usize, stops: &mut Vec<(u32, usize, usize)>, vars: &Vars, nested: bool) {
    while *i < chars.len() {
        let c = chars[*i];
        if nested && c == '}' {
            return;
        }
        if c == '\\' && *i + 1 < chars.len() && matches!(chars[*i + 1], '$' | '}' | '\\') {
            push(out, len, chars[*i + 1]);
            *i += 2;
            continue;
        }
        if c != '$' {
            push(out, len, c);
            *i += 1;
            continue;
        }
        // `$` followed by a number, a name or `{`.
        let next = chars.get(*i + 1).copied();
        match next {
            Some(d) if d.is_ascii_digit() => {
                let mut j = *i + 1;
                let mut n = 0u32;
                while j < chars.len() && chars[j].is_ascii_digit() {
                    n = n * 10 + chars[j].to_digit(10).unwrap();
                    j += 1;
                }
                stops.push((n, *len, *len));
                *i = j;
            }
            Some('{') => {
                let mut j = *i + 2;
                if chars.get(j).is_some_and(|c| c.is_ascii_digit()) {
                    let mut n = 0u32;
                    while j < chars.len() && chars[j].is_ascii_digit() {
                        n = n * 10 + chars[j].to_digit(10).unwrap();
                        j += 1;
                    }
                    let start = *len;
                    match chars.get(j) {
                        Some(':') => {
                            j += 1;
                            parse(chars, &mut j, out, len, stops, vars, true);
                        }
                        Some('|') => {
                            // Choice: use the first option.
                            j += 1;
                            let mut first = true;
                            while j < chars.len() && chars[j] != '|' {
                                if chars[j] == ',' {
                                    first = false;
                                } else if first {
                                    push(out, len, chars[j]);
                                }
                                j += 1;
                            }
                            j += 1;
                        }
                        _ => {}
                    }
                    stops.push((n, start, *len));
                    if chars.get(j) == Some(&'}') {
                        j += 1;
                    }
                    *i = j;
                } else {
                    let name_start = j;
                    while j < chars.len() && (chars[j].is_ascii_alphanumeric() || chars[j] == '_') {
                        j += 1;
                    }
                    let name: String = chars[name_start..j].iter().collect();
                    let value = vars.get(&name);
                    if chars.get(j) == Some(&':') {
                        j += 1;
                        match value.filter(|v| !v.is_empty()) {
                            Some(v) => {
                                // Skip the default.
                                let mut tmp = String::new();
                                let mut tmp_len = 0;
                                let mut ignored = Vec::new();
                                parse(chars, &mut j, &mut tmp, &mut tmp_len, &mut ignored, vars, true);
                                for c in v.chars() {
                                    push(out, len, c);
                                }
                            }
                            None => parse(chars, &mut j, out, len, stops, vars, true),
                        }
                    } else {
                        // Skip transforms like ${VAR/regex/fmt/}.
                        while j < chars.len() && chars[j] != '}' {
                            j += 1;
                        }
                        let text = value.unwrap_or(name);
                        for c in text.chars() {
                            push(out, len, c);
                        }
                    }
                    if chars.get(j) == Some(&'}') {
                        j += 1;
                    }
                    *i = j;
                }
            }
            Some(a) if a.is_ascii_alphabetic() || a == '_' => {
                let mut j = *i + 1;
                while j < chars.len() && (chars[j].is_ascii_alphanumeric() || chars[j] == '_') {
                    j += 1;
                }
                let name: String = chars[*i + 1..j].iter().collect();
                match vars.get(&name) {
                    Some(v) => {
                        for c in v.chars() {
                            push(out, len, c);
                        }
                    }
                    None => {
                        push(out, len, '$');
                        for c in name.chars() {
                            push(out, len, c);
                        }
                    }
                }
                *i = j;
            }
            _ => {
                push(out, len, '$');
                *i += 1;
            }
        }
    }
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn placeholders_and_cursor() {
        let e = expand("function ${1:name}($2) {\n\t$0\n}", &Vars::default());
        assert_eq!(e.text, "function name() {\n\t\n}");
        assert_eq!(e.select, 9..13);
        let e = expand("<b>$0</b>", &Vars::default());
        assert_eq!(e.text, "<b></b>");
        assert_eq!(e.select, 3..3);
        let e = expand("a ${1|x,y|} \\$5", &Vars::default());
        assert_eq!(e.text, "a x $5");
        assert_eq!(e.select, 2..3);
    }

    #[test]
    fn variables() {
        let vars = Vars { filename: "a.html".into(), ..Default::default() };
        let e = expand("$FILENAME ${UNKNOWN:def} ${SELECTED_TEXT:none}", &vars);
        assert_eq!(e.text, "a.html def none");
    }

}
