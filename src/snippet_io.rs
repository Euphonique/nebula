//! Exporting and importing own snippets: Nebula's JSON and VS Code snippet
//! files (`html.json`, `*.code-snippets`).

use std::collections::BTreeMap;

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::lang::Language;
use crate::settings::{ALL_LANGUAGES, UserSnippet};

/// A snippet found in an import file, with its language key (id or `*`).
#[derive(Clone, Debug, PartialEq)]
pub struct Imported {
    pub key: String,
    pub snippet: UserSnippet,
}

#[derive(Serialize, Deserialize)]
struct Export {
    /// Format marker and version.
    nebula_snippets: u32,
    snippets: BTreeMap<String, Vec<UserSnippet>>,
}

pub fn export(snippets: &BTreeMap<String, Vec<UserSnippet>>) -> Result<Vec<u8>> {
    Ok(serde_json::to_vec_pretty(&Export { nebula_snippets: 1, snippets: snippets.clone() })?)
}

/// Reads a Nebula export or a VS Code snippet file; `file_name` gives the
/// language of VS Code's per-language files (`php.json`).
pub fn parse(bytes: &[u8], file_name: &str) -> Result<Vec<Imported>> {
    let text = std::str::from_utf8(bytes).context("Die Datei ist keine Textdatei (UTF-8)")?;
    let value: Value = serde_json::from_str(&strip_jsonc(text.trim_start_matches('\u{feff}')))
        .context("Die Datei ist kein gültiges JSON")?;
    let Value::Object(map) = value else { bail!("Unbekanntes Format – erwartet wird ein JSON-Objekt") };
    if map.contains_key("nebula_snippets") {
        let export: Export = serde_json::from_value(Value::Object(map)).context("Die Nebula-Snippet-Datei ist beschädigt")?;
        return Ok(export
            .snippets
            .into_iter()
            .flat_map(|(key, list)| list.into_iter().map(move |snippet| Imported { key: key.clone(), snippet }))
            .collect());
    }
    // VS Code: { "Name": { "prefix": …, "body": …, "description": …, "scope": … } }
    let stem = file_name.rsplit(['/', '\\']).next().unwrap_or(file_name);
    let file_lang = stem.strip_suffix(".json").and_then(language_key);
    // A collection (`laravel.code-snippets`) becomes a group of its own.
    let group = stem.strip_suffix(".code-snippets").unwrap_or("").to_string();
    let mut out = Vec::new();
    for (name, entry) in map {
        let Value::Object(entry) = entry else { continue };
        let Some(body) = entry.get("body").and_then(lines) else { continue };
        let trigger = match entry.get("prefix") {
            Some(Value::String(s)) => s.clone(),
            Some(Value::Array(a)) => a.iter().find_map(|v| v.as_str()).unwrap_or_default().to_string(),
            _ => String::new(),
        };
        let description = entry.get("description").and_then(lines).filter(|d| !d.is_empty()).unwrap_or_else(|| name.clone());
        let snippet = UserSnippet { trigger, description, body, group: group.clone() };
        let mut keys: Vec<String> = entry
            .get("scope")
            .and_then(Value::as_str)
            .map(|s| s.split(',').filter_map(|l| language_key(l.trim())).collect())
            .unwrap_or_default();
        keys.dedup();
        if keys.is_empty() {
            keys.push(file_lang.clone().unwrap_or_else(|| ALL_LANGUAGES.to_string()));
        }
        for key in keys {
            out.push(Imported { key, snippet: snippet.clone() });
        }
    }
    if out.is_empty() {
        bail!("In der Datei wurden keine Snippets gefunden");
    }
    Ok(out)
}

/// A string or an array of lines (VS Code's `body` and `description`).
fn lines(v: &Value) -> Option<String> {
    match v {
        Value::String(s) => Some(s.clone()),
        Value::Array(a) => Some(a.iter().filter_map(Value::as_str).collect::<Vec<_>>().join("\n")),
        _ => None,
    }
}

/// Nebula's language key for a VS Code language id.
fn language_key(id: &str) -> Option<String> {
    let id = match id {
        "typescriptreact" => "tsx",
        "javascriptreact" => "javascript",
        "jsonc" => "json",
        "shellscript" => "shell",
        "plaintext" => "text",
        "global" => return Some(ALL_LANGUAGES.into()),
        other => other,
    };
    Language::from_id(id).map(|l| l.id().to_string())
}

/// Removes `//` and `/* */` comments and trailing commas (JSON with comments,
/// as VS Code writes it).
fn strip_jsonc(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    let mut in_string = false;
    while i < chars.len() {
        let c = chars[i];
        if in_string {
            out.push(c);
            if c == '\\' && i + 1 < chars.len() {
                out.push(chars[i + 1]);
                i += 1;
            } else if c == '"' {
                in_string = false;
            }
        } else if c == '"' {
            in_string = true;
            out.push(c);
        } else if c == '/' && chars.get(i + 1) == Some(&'/') {
            while i < chars.len() && chars[i] != '\n' {
                i += 1;
            }
            continue;
        } else if c == '/' && chars.get(i + 1) == Some(&'*') {
            i += 2;
            while i + 1 < chars.len() && !(chars[i] == '*' && chars[i + 1] == '/') {
                i += 1;
            }
            i += 2;
            continue;
        } else if c == ',' {
            // Drop it if only whitespace and comments follow before a closing bracket.
            if !matches!(next_significant(&chars, i + 1), Some('}') | Some(']')) {
                out.push(c);
            }
        } else {
            out.push(c);
        }
        i += 1;
    }
    out
}

/// First character from `i` on that is neither whitespace nor in a comment.
fn next_significant(chars: &[char], mut i: usize) -> Option<char> {
    while i < chars.len() {
        match (chars[i], chars.get(i + 1)) {
            (c, _) if c.is_whitespace() => i += 1,
            ('/', Some('/')) => {
                while i < chars.len() && chars[i] != '\n' {
                    i += 1;
                }
            }
            ('/', Some('*')) => {
                i += 2;
                while i + 1 < chars.len() && !(chars[i] == '*' && chars[i + 1] == '/') {
                    i += 1;
                }
                i += 2;
            }
            (c, _) => return Some(c),
        }
    }
    None
}

/// The same snippet exists already.
pub fn is_duplicate(existing: &BTreeMap<String, Vec<UserSnippet>>, item: &Imported) -> bool {
    existing
        .get(&item.key)
        .is_some_and(|l| l.iter().any(|s| s.trigger == item.snippet.trigger && s.body == item.snippet.body))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nebula_round_trip() {
        let mut map = BTreeMap::new();
        map.insert("php".to_string(), vec![UserSnippet { trigger: "dd".into(), description: "Dump".into(), body: "dd($1);$0".into(), group: "Laravel".into() }]);
        map.insert("*".to_string(), vec![UserSnippet { trigger: "date".into(), description: String::new(), body: "$DATE".into(), ..Default::default() }]);
        let back = parse(&export(&map).unwrap(), "nebula-snippets.json").unwrap();
        assert_eq!(back.len(), 2);
        assert!(back.iter().any(|i| i.snippet.group == "Laravel"));
        assert!(back.iter().all(|i| is_duplicate(&map, i)));
    }

    #[test]
    fn vscode_language_file_with_comments() {
        let json = r#"{
            // Place your snippets for php here.
            "Dump and die": {
                "prefix": ["dd", "dump"],
                "body": ["dd(${1:\\$var});", "$0"],
                "description": "Laravel dd()", // comment after a value
            },
            /* block
               comment */
            "No body": { "prefix": "x" },
        }"#;
        let list = parse(json.as_bytes(), r"C:\Users\x\AppData\Roaming\Code\User\snippets\php.json").unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].key, "php");
        assert_eq!(list[0].snippet.trigger, "dd");
        assert_eq!(list[0].snippet.body, "dd(${1:\\$var});\n$0");
        assert_eq!(list[0].snippet.description, "Laravel dd()");
    }

    #[test]
    fn vscode_global_file_with_scopes() {
        let json = r#"{
            "Log": { "scope": "javascript,typescriptreact", "prefix": "cl", "body": "console.log($1);" },
            "Signature": { "prefix": "sig", "body": "Gruß, $1" }
        }"#;
        let list = parse(json.as_bytes(), "my.code-snippets").unwrap();
        assert!(list.iter().all(|i| i.snippet.group == "my"));
        let keys: Vec<(&str, &str)> = list.iter().map(|i| (i.key.as_str(), i.snippet.trigger.as_str())).collect();
        assert!(keys.contains(&("javascript", "cl")) && keys.contains(&("tsx", "cl")));
        assert!(keys.contains(&("*", "sig")));
        // Description falls back to the name.
        assert_eq!(list.iter().find(|i| i.snippet.trigger == "sig").unwrap().snippet.description, "Signature");
    }

    #[test]
    fn strings_keep_slashes_and_commas() {
        assert_eq!(strip_jsonc(r#"{"a": "http://x, y", }"#), r#"{"a": "http://x, y" }"#);
    }
}
