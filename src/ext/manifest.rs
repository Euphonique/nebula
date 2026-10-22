//! Installed Nova extensions: `extension.json` and the static parts of a
//! bundle (commands, preferences, themes, clips, sidebars).

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::Deserialize;
use serde_json::Value;

// Some fields mirror the manifest format without being used yet.
#[allow(dead_code)]
/// A command from the manifest's `commands` sections.
#[derive(Clone, Debug)]
pub struct CommandDef {
    pub title: String,
    pub command: String,
    /// "extensions", "editor", "command-palette" or "text".
    pub section: String,
    pub shortcut: Option<String>,
    pub syntaxes: Vec<String>,
    pub when: Option<String>,
}

/// One user-visible preference item.
#[allow(dead_code)]
#[derive(Clone, Debug)]
pub struct ConfigItem {
    pub key: Option<String>,
    pub kind: String,
    pub title: String,
    pub description: String,
    pub placeholder: String,
    pub default: Value,
    pub values: Vec<(String, String)>,
    pub min: Option<f64>,
    pub max: Option<f64>,
    pub command: Option<String>,
    pub link: Option<String>,
    pub children: Vec<ConfigItem>,
}

#[allow(dead_code)]
#[derive(Clone, Debug)]
pub struct Clip {
    pub name: String,
    pub content: String,
    pub trigger: Option<String>,
    pub syntax: Option<String>,
}

#[allow(dead_code)]
#[derive(Clone, Debug)]
pub struct SidebarSection {
    pub id: String,
    pub name: String,
    pub placeholder: String,
    pub header_commands: Vec<(String, String)>,
    pub context_commands: Vec<(String, String)>,
}

#[allow(dead_code)]
#[derive(Clone, Debug)]
pub struct SidebarDef {
    pub id: String,
    pub name: String,
    pub sections: Vec<SidebarSection>,
}

#[allow(dead_code)]
#[derive(Clone, Debug)]
pub struct Extension {
    pub identifier: String,
    pub name: String,
    pub organization: String,
    pub version: String,
    pub description: String,
    pub categories: Vec<String>,
    pub path: PathBuf,
    pub main: Option<PathBuf>,
    pub activation: Vec<String>,
    pub entitlements: Value,
    pub commands: Vec<CommandDef>,
    pub config: Vec<ConfigItem>,
    pub config_workspace: Vec<ConfigItem>,
    pub themes: Vec<PathBuf>,
    pub clips: Vec<Clip>,
    pub sidebars: Vec<SidebarDef>,
    pub strings: HashMap<String, String>,
    pub repository: Option<String>,
    pub homepage: Option<String>,
    pub bugs: Option<String>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct RawManifest {
    identifier: String,
    name: String,
    organization: Option<String>,
    vendor: Option<String>,
    version: Option<Value>,
    description: String,
    categories: Vec<String>,
    main: Option<String>,
    #[serde(rename = "activationEvents")]
    activation_events: Vec<String>,
    entitlements: Value,
    commands: HashMap<String, Vec<Value>>,
    config: Value,
    #[serde(rename = "configWorkspace")]
    config_workspace: Value,
    sidebars: Vec<Value>,
    repository: Option<Value>,
    homepage: Option<String>,
    bugs: Option<Value>,
}

/// Parses JSON that may contain comments and trailing commas (common in
/// hand-written extension files).
pub fn parse_lenient(text: &str) -> Option<Value> {
    if let Ok(v) = serde_json::from_str(text) {
        return Some(v);
    }
    // Comments first, so a comment between a comma and a bracket doesn't hide it.
    let without_comments = clean_json(text, false);
    serde_json::from_str(&clean_json(&without_comments, true)).ok()
}

fn clean_json(text: &str, commas: bool) -> String {
    let mut out = String::with_capacity(text.len());
    let chars: Vec<char> = text.trim_start_matches('\u{feff}').chars().collect();
    let mut i = 0;
    let mut in_str = false;
    while i < chars.len() {
        let c = chars[i];
        if in_str {
            out.push(c);
            if c == '\\' && i + 1 < chars.len() {
                out.push(chars[i + 1]);
                i += 1;
            } else if c == '"' {
                in_str = false;
            }
        } else if c == '"' {
            in_str = true;
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
        } else if c == ',' && commas {
            // Drop a comma that only precedes a closing bracket.
            let next = chars[i + 1..].iter().find(|c| !c.is_whitespace());
            if !matches!(next, Some('}') | Some(']')) {
                out.push(c);
            }
        } else {
            out.push(c);
        }
        i += 1;
    }
    out
}

fn read_json(path: &Path) -> Option<Value> {
    parse_lenient(&std::fs::read_to_string(path).ok()?)
}

fn link(v: &Option<Value>) -> Option<String> {
    match v {
        Some(Value::String(s)) if !s.is_empty() => Some(s.clone()),
        Some(Value::Object(o)) => o.get("url").and_then(|u| u.as_str()).map(str::to_string),
        _ => None,
    }
}

/// Loads the base and preferred localization table (`strings.json`).
fn load_strings(dir: &Path) -> HashMap<String, String> {
    let mut map = HashMap::new();
    for lang in ["en.lproj", "Base.lproj", "de.lproj"] {
        if let Some(Value::Object(o)) = read_json(&dir.join(lang).join("strings.json")) {
            for (k, v) in o {
                if let Some(s) = v.as_str() {
                    map.insert(k, s.to_string());
                }
            }
        }
    }
    map
}

impl Extension {
    pub fn load(dir: &Path) -> anyhow::Result<Extension> {
        let text = std::fs::read_to_string(dir.join("extension.json"))?;
        let value = parse_lenient(&text).ok_or_else(|| anyhow::anyhow!(t!("extension.json ist kein gültiges JSON")))?;
        let raw: RawManifest = serde_json::from_value(value)?;
        if raw.identifier.is_empty() {
            anyhow::bail!(t!("extension.json enthält keinen identifier"));
        }
        let strings = load_strings(dir);
        let loc = |s: &str| strings.get(s).cloned().unwrap_or_else(|| s.to_string());

        let main = raw.main.as_ref().map(|m| {
            let p = dir.join("Scripts").join(m);
            if p.is_file() { p } else { dir.join(m) }
        });

        let mut commands = Vec::new();
        for (section, list) in &raw.commands {
            for c in list {
                let Some(command) = c.get("command").and_then(|v| v.as_str()) else { continue };
                let title = c
                    .get("paletteTitle")
                    .or_else(|| c.get("title"))
                    .and_then(|v| v.as_str())
                    .unwrap_or(command);
                commands.push(CommandDef {
                    title: loc(title),
                    command: command.to_string(),
                    section: section.clone(),
                    shortcut: c.get("shortcut").and_then(|v| v.as_str()).map(str::to_string),
                    syntaxes: c
                        .pointer("/filters/syntaxes")
                        .and_then(|v| v.as_array())
                        .map(|a| a.iter().filter_map(|s| s.as_str().map(str::to_string)).collect())
                        .unwrap_or_default(),
                    when: c.get("when").and_then(|v| v.as_str()).map(str::to_string),
                });
            }
        }

        let config_list = |v: &Value| -> Vec<ConfigItem> {
            let v = match v {
                Value::String(rel) => read_json(&dir.join(rel)).unwrap_or(Value::Null),
                v => v.clone(),
            };
            v.as_array().map(|a| a.iter().map(|i| parse_config(i, &loc)).collect()).unwrap_or_default()
        };

        let mut themes = Vec::new();
        if let Ok(rd) = std::fs::read_dir(dir.join("Themes")) {
            for e in rd.flatten() {
                if e.path().extension().is_some_and(|x| x.eq_ignore_ascii_case("css")) {
                    themes.push(e.path());
                }
            }
        }
        themes.sort();

        let mut clips = Vec::new();
        for f in [dir.join("Clips.json"), dir.join("Clips").join("Clips.json")] {
            if let Some(v) = read_json(&f) {
                collect_clips(v.get("clips").unwrap_or(&Value::Null), &mut clips);
            }
        }

        let sidebars = raw
            .sidebars
            .iter()
            .filter_map(|s| {
                Some(SidebarDef {
                    id: s.get("id")?.as_str()?.to_string(),
                    name: loc(s.get("name").and_then(|v| v.as_str()).unwrap_or("")),
                    sections: s
                        .get("sections")
                        .and_then(|v| v.as_array())
                        .map(|a| {
                            a.iter()
                                .filter_map(|sec| {
                                    let cmds = |key: &str| -> Vec<(String, String)> {
                                        sec.get(key)
                                            .and_then(|v| v.as_array())
                                            .map(|a| {
                                                a.iter()
                                                    .filter_map(|c| {
                                                        Some((
                                                            loc(c.get("title").and_then(|v| v.as_str()).unwrap_or("")),
                                                            c.get("command")?.as_str()?.to_string(),
                                                        ))
                                                    })
                                                    .collect()
                                            })
                                            .unwrap_or_default()
                                    };
                                    Some(SidebarSection {
                                        id: sec.get("id")?.as_str()?.to_string(),
                                        name: loc(sec.get("name").and_then(|v| v.as_str()).unwrap_or("")),
                                        placeholder: loc(sec.get("placeholderText").and_then(|v| v.as_str()).unwrap_or("")),
                                        header_commands: cmds("headerCommands"),
                                        context_commands: {
                                            let mut c = cmds("contextCommands");
                                            c.extend(cmds("contextualCommands"));
                                            c
                                        },
                                    })
                                })
                                .collect()
                        })
                        .unwrap_or_default(),
                })
            })
            .collect();

        Ok(Extension {
            name: if raw.name.is_empty() { raw.identifier.clone() } else { loc(&raw.name) },
            organization: raw.organization.or(raw.vendor).unwrap_or_default(),
            version: match raw.version {
                Some(Value::String(s)) => s,
                Some(v) => v.to_string(),
                None => String::new(),
            },
            description: loc(&raw.description),
            categories: raw.categories,
            path: dir.to_path_buf(),
            main: main.filter(|p| p.is_file()),
            activation: raw.activation_events,
            entitlements: raw.entitlements,
            commands,
            config: config_list(&raw.config),
            config_workspace: config_list(&raw.config_workspace),
            themes,
            clips,
            sidebars,
            repository: link(&raw.repository),
            homepage: raw.homepage.filter(|s| !s.is_empty()),
            bugs: link(&raw.bugs),
            identifier: raw.identifier,
            strings,
        })
    }

    /// Default values of all preferences, by key.
    pub fn config_defaults(&self) -> Vec<(String, Value)> {
        fn walk(items: &[ConfigItem], out: &mut Vec<(String, Value)>) {
            for i in items {
                if let Some(k) = &i.key
                    && !i.default.is_null()
                {
                    out.push((k.clone(), i.default.clone()));
                }
                walk(&i.children, out);
            }
        }
        let mut out = Vec::new();
        walk(&self.config, &mut out);
        walk(&self.config_workspace, &mut out);
        out
    }

    pub fn readme(&self) -> Option<String> {
        ["README.md", "Readme.md", "readme.md", "README"]
            .iter()
            .find_map(|n| std::fs::read_to_string(self.path.join(n)).ok())
    }

    pub fn has_scripts(&self) -> bool {
        self.main.is_some()
    }

    pub fn image(&self) -> Option<PathBuf> {
        ["extension.png", "extension@2x.png"].iter().map(|n| self.path.join(n)).find(|p| p.is_file())
    }
}

fn parse_config(v: &Value, loc: &dyn Fn(&str) -> String) -> ConfigItem {
    let s = |k: &str| v.get(k).and_then(|x| x.as_str()).unwrap_or("").to_string();
    let values = v
        .get("values")
        .and_then(|x| x.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|e| match e {
                    Value::String(s) => Some((s.clone(), loc(s))),
                    Value::Array(t) => {
                        let raw = t.first()?.as_str()?.to_string();
                        let title = t.get(1).and_then(|x| x.as_str()).map(loc).unwrap_or_else(|| raw.clone());
                        Some((raw, title))
                    }
                    _ => None,
                })
                .collect()
        })
        .unwrap_or_default();
    ConfigItem {
        key: v.get("key").and_then(|x| x.as_str()).map(str::to_string),
        kind: s("type"),
        title: loc(&s("title")),
        description: loc(&s("description")),
        placeholder: loc(&s("placeholder")),
        default: v.get("default").cloned().unwrap_or(Value::Null),
        values,
        min: v.get("min").and_then(|x| x.as_f64()),
        max: v.get("max").and_then(|x| x.as_f64()),
        command: v.get("command").and_then(|x| x.as_str()).map(str::to_string),
        link: v.get("link").and_then(|x| x.as_str()).map(str::to_string),
        children: v
            .get("children")
            .and_then(|x| x.as_array())
            .map(|a| a.iter().map(|i| parse_config(i, loc)).collect())
            .unwrap_or_default(),
    }
}

fn collect_clips(v: &Value, out: &mut Vec<Clip>) {
    let Some(items) = v.as_array() else { return };
    for item in items {
        if let Some(children) = item.get("children") {
            collect_clips(children, out);
        }
        if let Some(content) = item.get("content").and_then(|c| c.as_str()) {
            out.push(Clip {
                name: item.get("name").and_then(|n| n.as_str()).unwrap_or(t!("Clip")).to_string(),
                content: content.to_string(),
                trigger: item.get("trigger").and_then(|n| n.as_str()).map(str::to_string),
                syntax: item.get("syntax").and_then(|n| n.as_str()).map(str::to_string),
            });
        }
    }
}

/// Folder holding installed extensions.
pub fn extensions_dir() -> PathBuf {
    crate::settings::config_dir().join("extensions")
}

/// All installed extensions, sorted by name.
pub fn load_all() -> (Vec<Extension>, Vec<(PathBuf, String)>) {
    let mut exts = Vec::new();
    let mut errors = Vec::new();
    if let Ok(rd) = std::fs::read_dir(extensions_dir()) {
        for e in rd.flatten() {
            let p = e.path();
            if !p.join("extension.json").is_file() {
                continue;
            }
            match Extension::load(&p) {
                Ok(x) => exts.push(x),
                Err(err) => errors.push((p, err.to_string())),
            }
        }
    }
    exts.sort_by_key(|e| e.name.to_lowercase());
    (exts, errors)
}

/// Nova's syntax name for one of our languages.
pub fn syntax_name(lang: crate::lang::Language) -> Option<&'static str> {
    use crate::lang::Language;
    match lang {
        Language::PlainText => None,
        l => Some(l.id()),
    }
}

/// Parses a Nova shortcut like `cmd-shift-p` (cmd maps to Ctrl, opt to Alt).
pub fn parse_shortcut(s: &str) -> Option<eframe::egui::KeyboardShortcut> {
    use eframe::egui::{Key, KeyboardShortcut, Modifiers};
    let parts: Vec<&str> = s.split('-').collect();
    let (key, mods) = parts.split_last()?;
    let mut m = Modifiers::NONE;
    for p in mods {
        match p.to_ascii_lowercase().as_str() {
            "cmd" | "ctrl" => m = m | Modifiers::COMMAND,
            "opt" | "alt" => m = m | Modifiers::ALT,
            "shift" => m = m | Modifiers::SHIFT,
            _ => return None,
        }
    }
    let key = match key.to_ascii_lowercase().as_str() {
        "escape" => Key::Escape,
        "space" => Key::Space,
        "tab" => Key::Tab,
        "return" | "enter" => Key::Enter,
        "delete" => Key::Backspace,
        "forwarddelete" => Key::Delete,
        "up" => Key::ArrowUp,
        "down" => Key::ArrowDown,
        "left" => Key::ArrowLeft,
        "right" => Key::ArrowRight,
        "home" => Key::Home,
        "end" => Key::End,
        "pageup" => Key::PageUp,
        "pagedown" => Key::PageDown,
        "insert" => Key::Insert,
        "" => Key::Minus, // "cmd--"
        k => Key::from_name(k).or_else(|| Key::from_name(&k.to_ascii_uppercase()))?,
    };
    // Menu shortcuts need a modifier besides Shift.
    if !(m.command || m.alt) {
        return None;
    }
    Some(KeyboardShortcut::new(m, key))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lenient_json() {
        let v = parse_lenient("{\n // c\n \"a\": [1, 2,],\n \"b\": \"x//y\", /* z */\n}").unwrap();
        assert_eq!(v["a"][1], 2);
        assert_eq!(v["b"], "x//y");
    }

    #[test]
    fn shortcuts() {
        let s = parse_shortcut("cmd-shift-p").unwrap();
        assert!(s.modifiers.command && s.modifiers.shift);
        assert_eq!(s.logical_key, eframe::egui::Key::P);
        assert!(parse_shortcut("shift-p").is_none());
    }
}
