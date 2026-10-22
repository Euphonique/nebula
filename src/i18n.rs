//! Translations of the user interface.
//!
//! The German text in the code is the key: `t!("Speichern")`, with values
//! `t!("{n} Server importiert.", n = count)`. Every other language is a JSON
//! file `{"code", "name", "strings": {German: translation}}`; built-in ones
//! live in `src/i18n/`, more (or improved) ones in the config folder's
//! `languages` folder. Missing entries fall back to English, then German.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::RwLock;

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

/// Language of the texts in the code.
pub const SOURCE: &str = "de";
/// Used when the system language has no translation.
pub const FALLBACK: &str = "en";

/// Built-in translations (German needs none).
const BUILTIN: &[(&str, &str)] = &[
    ("en", include_str!("i18n/en.json")),
    ("es", include_str!("i18n/es.json")),
    ("fr", include_str!("i18n/fr.json")),
    ("it", include_str!("i18n/it.json")),
];

/// A language file as stored on disk.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct LanguageFile {
    /// ISO 639-1 code, e.g. `fr`.
    pub code: String,
    /// Name in the language itself, e.g. `Français`.
    pub name: String,
    pub strings: std::collections::BTreeMap<String, String>,
}

struct Language {
    code: String,
    name: String,
    /// German text -> translation; leaked once so lookups return `&'static str`.
    strings: HashMap<String, &'static str>,
    /// Loaded from the config folder (can be removed again).
    user: bool,
}

struct State {
    languages: Vec<Language>,
    /// Index of the active language; `None` = German.
    current: Option<usize>,
}

static STATE: RwLock<State> = RwLock::new(State { languages: Vec::new(), current: None });

fn leak(s: String) -> &'static str {
    Box::leak(s.into_boxed_str())
}

pub fn user_dir() -> PathBuf {
    crate::settings::config_dir().join("languages")
}

fn parse(text: &str) -> Result<LanguageFile> {
    let file: LanguageFile = serde_json::from_str(text.trim_start_matches('\u{feff}')).context(tr("Die Sprachdatei ist kein gültiges JSON"))?;
    let code = file.code.trim().to_lowercase();
    if code.is_empty() || code.len() > 12 || !code.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
        bail!("{}", tr("Die Sprachdatei hat keinen gültigen Sprachcode (z. B. \"fr\")"));
    }
    Ok(LanguageFile { code, ..file })
}

fn to_language(file: LanguageFile, user: bool) -> Language {
    let strings = file
        .strings
        .into_iter()
        .filter(|(_, v)| !v.trim().is_empty())
        .map(|(k, v)| (k, leak(v)))
        .collect();
    let name = if file.name.trim().is_empty() { file.code.clone() } else { file.name };
    Language { code: file.code, name, strings, user }
}

/// Reads the built-in and the user's language files (user files win).
pub fn load() {
    let mut languages: Vec<Language> = Vec::new();
    for (_, text) in BUILTIN {
        if let Ok(file) = parse(text) {
            languages.push(to_language(file, false));
        }
    }
    if let Ok(rd) = std::fs::read_dir(user_dir()) {
        for e in rd.flatten().filter(|e| e.path().extension().is_some_and(|x| x == "json")) {
            let Ok(text) = std::fs::read_to_string(e.path()) else { continue };
            let Ok(file) = parse(&text) else { continue };
            let lang = to_language(file, true);
            match languages.iter_mut().find(|l| l.code == lang.code) {
                // A user file adds to and overrides the built-in strings.
                Some(existing) => {
                    existing.strings.extend(lang.strings);
                    if !lang.name.is_empty() {
                        existing.name = lang.name;
                    }
                    existing.user = true;
                }
                None => languages.push(lang),
            }
        }
    }
    let mut state = STATE.write().unwrap();
    let current = state.current.and_then(|i| state.languages.get(i)).map(|l| l.code.clone());
    state.languages = languages;
    state.current = current.and_then(|c| state.languages.iter().position(|l| l.code == c));
}

/// Windows display language, e.g. `de` (two letters, lower case).
pub fn system_language() -> String {
    use windows_sys::Win32::Globalization::{GetUserDefaultUILanguage, LCIDToLocaleName};
    // SAFETY: plain value call and a buffer of the documented maximum size.
    let name = unsafe {
        let lang = GetUserDefaultUILanguage();
        let mut buf = [0u16; 85];
        let n = LCIDToLocaleName(lang as u32, buf.as_mut_ptr(), buf.len() as i32, 0);
        String::from_utf16_lossy(&buf[..(n.max(1) - 1) as usize])
    };
    name.split('-').next().unwrap_or("").to_lowercase()
}

/// Sets the language: a code, or empty for the system language. Languages
/// without a translation fall back to English.
pub fn set_language(setting: &str) {
    let wanted = if setting.trim().is_empty() { system_language() } else { setting.trim().to_lowercase() };
    let mut state = STATE.write().unwrap();
    state.current = if wanted == SOURCE {
        None
    } else {
        state
            .languages
            .iter()
            .position(|l| l.code == wanted)
            .or_else(|| state.languages.iter().position(|l| l.code == FALLBACK))
    };
}

/// Code of the active language.
pub fn current() -> String {
    let state = STATE.read().unwrap();
    state.current.and_then(|i| state.languages.get(i)).map_or(SOURCE.to_string(), |l| l.code.clone())
}

/// (code, name) of every available language, German first.
pub fn available() -> Vec<(String, String)> {
    let state = STATE.read().unwrap();
    let mut out = vec![(SOURCE.to_string(), "Deutsch".to_string())];
    out.extend(state.languages.iter().map(|l| (l.code.clone(), l.name.clone())));
    out
}

/// The translation of `key` (German text) in the active language.
/// Translates the German key names in a fixed shortcut label such as "Strg+Umschalt+O".
pub fn keys(label: &str) -> String {
    label
        .split('+')
        .map(|part| match part {
            "Strg" => tr("Strg+").trim_end_matches('+'),
            "Umschalt" => tr("Umschalt+").trim_end_matches('+'),
            "Entf" => tr("Entf"),
            "Pos1" => tr("Pos1"),
            other => other,
        })
        .collect::<Vec<_>>()
        .join("+")
}

pub fn tr(key: &'static str) -> &'static str {
    let state = STATE.read().unwrap();
    let Some(lang) = state.current.and_then(|i| state.languages.get(i)) else { return key };
    if let Some(s) = lang.strings.get(key) {
        return s;
    }
    // Not translated yet: English is more widely understood than German.
    state
        .languages
        .iter()
        .find(|l| l.code == FALLBACK)
        .and_then(|en| en.strings.get(key).copied())
        .unwrap_or(key)
}

/// Puts `values` into the `{name}` placeholders of a template.
pub fn fill(template: &str, values: &[(&str, &dyn std::fmt::Display)]) -> String {
    let mut out = template.to_string();
    for (name, value) in values {
        out = out.replace(&format!("{{{name}}}"), &value.to_string());
    }
    out.replace("{{", "{").replace("}}", "}")
}

/// A language file with every text: the existing translations of `code`,
/// the missing ones empty (for translators). `keys` are all texts of the app.
pub fn export(code: &str, keys: &[&str]) -> Result<Vec<u8>> {
    let state = STATE.read().unwrap();
    let lang = state.languages.iter().find(|l| l.code == code);
    let file = LanguageFile {
        code: code.to_string(),
        name: lang.map_or(String::new(), |l| l.name.clone()),
        strings: keys
            .iter()
            .map(|k| (k.to_string(), lang.and_then(|l| l.strings.get(*k)).map_or(String::new(), |s| s.to_string())))
            .collect(),
    };
    Ok(serde_json::to_vec_pretty(&file)?)
}

/// Copies a language file into the config folder; returns its code and name.
pub fn import(bytes: &[u8]) -> Result<(String, String)> {
    let text = std::str::from_utf8(bytes).context(tr("Die Datei ist keine Textdatei (UTF-8)"))?;
    let file = parse(text)?;
    if file.code == SOURCE {
        bail!("{}", tr("Deutsch ist die Ausgangssprache und braucht keine Sprachdatei"));
    }
    let dir = user_dir();
    std::fs::create_dir_all(&dir)?;
    std::fs::write(dir.join(format!("{}.json", file.code)), serde_json::to_vec_pretty(&file)?)?;
    load();
    let name = if file.name.is_empty() { file.code.clone() } else { file.name.clone() };
    Ok((file.code, name))
}

/// All texts of the app: the keys of the (complete) English file.
pub fn all_keys() -> Vec<&'static str> {
    let Ok(file) = parse(BUILTIN[0].1) else { return Vec::new() };
    file.strings.into_keys().map(leak).collect()
}

/// Marks a text in a `const`/`static` as translatable without translating
/// it there (translate where it is shown, with [`tr`]).
#[macro_export]
macro_rules! tk {
    ($key:literal) => {
        $key
    };
}

/// `t!("Text")` translates; `t!("{n} Dateien", n = count)` fills in values.
#[macro_export]
macro_rules! t {
    ($key:literal) => {
        $crate::i18n::tr($key)
    };
    ($key:literal, $($name:ident = $value:expr),+ $(,)?) => {
        $crate::i18n::fill($crate::i18n::tr($key), &[$((stringify!($name), &$value as &dyn std::fmt::Display)),+])
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn placeholders_and_escapes() {
        let n = 3;
        assert_eq!(fill("{n} Server, {name}", &[("n", &n), ("name", &"Shop")]), "3 Server, Shop");
        assert_eq!(fill("a {{b}} {x}", &[("x", &1)]), "a {b} 1");
    }

    #[test]
    fn builtin_files_parse() {
        for (code, text) in BUILTIN {
            let file = parse(text).unwrap();
            assert_eq!(&file.code, code);
            // Placeholders must survive translation.
            for (k, v) in &file.strings {
                if v.is_empty() {
                    continue;
                }
                let names = |s: &str| {
                    let mut v: Vec<String> = s.split('{').skip(1).filter_map(|p| p.split_once('}').map(|(n, _)| n.to_string())).collect();
                    v.sort();
                    v
                };
                assert_eq!(names(k), names(v), "{code}: Platzhalter in „{k}“");
            }
        }
    }
}
