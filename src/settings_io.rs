//! Exporting and importing Nebula's settings as one JSON file, e.g. to move
//! to another computer. Passwords and API keys stay in the Windows
//! Credential Manager and are never part of it.

use std::collections::BTreeMap;

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

use crate::acp::AgentConfig;
use crate::settings::{AppSettings, Appearance, EditorSettings, LanguageSettings, PreviewDevice, Site, UserSnippet};
use crate::theme::ThemeFile;

/// Parts of the settings that can be exported and imported separately.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Section {
    Editor,
    Themes,
    Languages,
    Keys,
    Servers,
    Agents,
    Extensions,
}

impl Section {
    pub const ALL: [Section; 7] =
        [Section::Editor, Section::Themes, Section::Languages, Section::Keys, Section::Servers, Section::Agents, Section::Extensions];

    pub fn label(self) -> &'static str {
        match self {
            Section::Editor => t!("Editor und Darstellung"),
            Section::Themes => t!("Eigene Themes"),
            Section::Languages => t!("Sprachen und Snippets"),
            Section::Keys => t!("Tastenkürzel"),
            Section::Servers => t!("Server (ohne Passwörter)"),
            Section::Agents => t!("KI-Agenten (ohne API-Schlüssel)"),
            Section::Extensions => t!("Erweiterungen"),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EditorPart {
    pub editor: EditorSettings,
    pub appearance: Appearance,
    pub theme_light: String,
    pub theme_dark: String,
    pub mica: bool,
    pub preview_device: PreviewDevice,
    pub terminal_shell: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LanguagesPart {
    pub languages: BTreeMap<String, LanguageSettings>,
    pub snippets: BTreeMap<String, Vec<UserSnippet>>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AgentsPart {
    pub agents: Vec<AgentConfig>,
    pub agent: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ExtensionRef {
    pub identifier: String,
    pub name: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ExtensionsPart {
    pub installed: Vec<ExtensionRef>,
    pub disabled: Vec<String>,
    /// Content of `extension-config.json` (the extensions' own preferences).
    pub config: serde_json::Value,
}

/// The export file. Missing parts were not exported.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Bundle {
    /// Format marker and version.
    pub nebula_settings: u32,
    #[serde(default)]
    pub version: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub editor: Option<EditorPart>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub themes: Option<Vec<ThemeFile>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub languages: Option<LanguagesPart>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub keybindings: Option<BTreeMap<String, String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub servers: Option<Vec<Site>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agents: Option<AgentsPart>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extensions: Option<ExtensionsPart>,
}

/// What the export reads besides the settings.
pub struct Sources<'a> {
    pub settings: &'a AppSettings,
    pub sites: &'a [Site],
    pub user_themes: Vec<ThemeFile>,
    pub extensions: Vec<ExtensionRef>,
    pub extension_config: serde_json::Value,
}

impl Bundle {
    pub fn build(src: &Sources, sections: &[Section]) -> Bundle {
        let s = src.settings;
        let mut b = Bundle { nebula_settings: 1, version: env!("CARGO_PKG_VERSION").into(), ..Default::default() };
        for section in sections {
            match section {
                Section::Editor => {
                    b.editor = Some(EditorPart {
                        editor: s.editor.clone(),
                        appearance: s.appearance,
                        theme_light: s.theme_light.clone(),
                        theme_dark: s.theme_dark.clone(),
                        mica: s.mica,
                        preview_device: s.preview_device,
                        terminal_shell: s.terminal_shell.clone(),
                    })
                }
                Section::Themes => b.themes = Some(src.user_themes.clone()),
                Section::Languages => b.languages = Some(LanguagesPart { languages: s.languages.clone(), snippets: s.snippets.clone() }),
                Section::Keys => b.keybindings = Some(s.keybindings.clone()),
                Section::Servers => b.servers = Some(src.sites.to_vec()),
                Section::Agents => {
                    // Environment variables may hold tokens: left out.
                    let agents = s.agents.iter().map(|a| AgentConfig { env: Default::default(), ..a.clone() }).collect();
                    b.agents = Some(AgentsPart { agents, agent: s.agent.clone() })
                }
                Section::Extensions => {
                    b.extensions = Some(ExtensionsPart {
                        installed: src.extensions.clone(),
                        disabled: s.disabled_extensions.clone(),
                        config: src.extension_config.clone(),
                    })
                }
            }
        }
        b
    }

    pub fn has(&self, section: Section) -> bool {
        match section {
            Section::Editor => self.editor.is_some(),
            Section::Themes => self.themes.as_ref().is_some_and(|t| !t.is_empty()),
            Section::Languages => self.languages.is_some(),
            Section::Keys => self.keybindings.is_some(),
            Section::Servers => self.servers.as_ref().is_some_and(|s| !s.is_empty()),
            Section::Agents => self.agents.is_some(),
            Section::Extensions => self.extensions.is_some(),
        }
    }

    /// One line about what the section contains.
    pub fn summary(&self, section: Section) -> String {
        let n = |count: usize, one: &str, many: &str| match count {
            0 => t!("keine {many}", many = many),
            1 => format!("1 {one}"),
            _ => format!("{count} {many}"),
        };
        match section {
            Section::Editor => self.editor.as_ref().map_or_default(|e| t!("Themes „{theme_light}“ / „{theme_dark}“, Schrift {font_size} pt", theme_light = e.theme_light, theme_dark = e.theme_dark, font_size = e.editor.font_size)),
            Section::Themes => n(self.themes.as_ref().map_or(0, Vec::len), t!("Theme"), t!("Themes")),
            Section::Languages => self.languages.as_ref().map_or_default(|l| {
                format!("{}, {}", n(l.languages.len(), t!("Sprache angepasst"), t!("Sprachen angepasst")), n(l.snippets.values().map(Vec::len).sum(), t!("Snippet"), t!("Snippets")))
            }),
            Section::Keys => match self.keybindings.as_ref().map_or(0, BTreeMap::len) {
                0 => t!("keine geänderten Kürzel (Standardbelegung)").into(),
                count => n(count, t!("geändertes Kürzel"), t!("geänderte Kürzel")),
            },
            Section::Servers => n(self.servers.as_ref().map_or(0, Vec::len), t!("Server"), t!("Server")),
            Section::Agents => n(self.agents.as_ref().map_or(0, |a| a.agents.len()), t!("Agent"), t!("Agenten")),
            Section::Extensions => n(self.extensions.as_ref().map_or(0, |e| e.installed.len()), t!("Erweiterung"), t!("Erweiterungen")),
        }
    }

    pub fn to_json(&self) -> Result<Vec<u8>> {
        Ok(serde_json::to_vec_pretty(self)?)
    }

    pub fn parse(bytes: &[u8]) -> Result<Bundle> {
        let value: serde_json::Value = serde_json::from_slice(bytes).context(t!("Die Datei ist kein gültiges JSON"))?;
        if value.get("nebula_settings").is_none() {
            bail!(t!("Das ist keine Nebula-Einstellungsdatei"));
        }
        serde_json::from_value(value).context(t!("Die Einstellungsdatei ist beschädigt oder stammt von einer neueren Version"))
    }
}

/// Merges imported snippets, skipping ones that exist already. Returns how many were added.
pub fn merge_snippets(into: &mut BTreeMap<String, Vec<UserSnippet>>, from: &BTreeMap<String, Vec<UserSnippet>>) -> usize {
    let mut added = 0;
    for (key, list) in from {
        let target = into.entry(key.clone()).or_default();
        for s in list {
            if !target.iter().any(|t| t.trigger == s.trigger && t.body == s.body) {
                target.push(s.clone());
                added += 1;
            }
        }
    }
    into.retain(|_, l| !l.is_empty());
    added
}

/// Replaces agents with the same name, adds the others; keeps local env variables.
pub fn merge_agents(into: &mut Vec<AgentConfig>, from: &[AgentConfig]) {
    for a in from {
        match into.iter_mut().find(|x| x.name == a.name) {
            Some(existing) => {
                let env = std::mem::take(&mut existing.env);
                *existing = AgentConfig { env, ..a.clone() };
            }
            None => into.push(a.clone()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sources(settings: &AppSettings) -> Sources<'_> {
        Sources {
            settings,
            sites: &[],
            user_themes: Vec::new(),
            extensions: vec![ExtensionRef { identifier: "a.b".into(), name: "AB".into() }],
            extension_config: serde_json::json!({ "a.b.x": 1 }),
        }
    }

    #[test]
    fn only_chosen_sections_and_no_secrets() {
        let mut s = AppSettings::default();
        s.keybindings.insert("save".into(), "Ctrl+S".into());
        let mut agent = AgentConfig { name: "X".into(), command: "x".into(), ..Default::default() };
        agent.env.insert("TOKEN".into(), "geheim".into());
        s.agents = vec![agent];
        let b = Bundle::build(&sources(&s), &[Section::Keys, Section::Agents]);
        let json = String::from_utf8(b.to_json().unwrap()).unwrap();
        assert!(!json.contains("geheim"));
        let back = Bundle::parse(json.as_bytes()).unwrap();
        assert!(back.has(Section::Keys) && back.has(Section::Agents));
        assert!(!back.has(Section::Editor) && !back.has(Section::Extensions));
        assert_eq!(back.summary(Section::Keys), "1 geändertes Kürzel");
    }

    #[test]
    fn merging_keeps_local_data() {
        let mut mine = BTreeMap::new();
        mine.insert("php".to_string(), vec![UserSnippet { trigger: "dd".into(), body: "dd($1);".into(), ..Default::default() }]);
        let mut theirs = mine.clone();
        theirs.get_mut("php").unwrap().push(UserSnippet { trigger: "dump".into(), body: "var_dump($1);".into(), ..Default::default() });
        assert_eq!(merge_snippets(&mut mine, &theirs), 1);
        assert_eq!(mine["php"].len(), 2);

        let mut local = vec![AgentConfig { name: "X".into(), command: "old".into(), ..Default::default() }];
        local[0].env.insert("TOKEN".into(), "lokal".into());
        merge_agents(&mut local, &[AgentConfig { name: "X".into(), command: "new".into(), ..Default::default() }]);
        assert_eq!(local[0].command, "new");
        assert_eq!(local[0].env["TOKEN"], "lokal");
    }

    #[test]
    fn rejects_other_files() {
        assert!(Bundle::parse(br#"{"sites": []}"#).is_err());
        assert!(Bundle::parse(b"kein json").is_err());
    }
}
