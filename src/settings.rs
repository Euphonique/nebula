//! Persistent application settings and server definitions.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

pub const APP_NAME: &str = "Nebula";
pub const KEYRING_SERVICE: &str = "Nebula Editor";

/// Nebula's folder (`%APPDATA%\nebula`): `config` plus the agents' Node
/// runtime and package cache next to it.
pub fn data_dir() -> PathBuf {
    let config = config_dir();
    config.parent().map(Path::to_path_buf).unwrap_or(config)
}

pub fn config_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("NEBULA_CONFIG_DIR") {
        return PathBuf::from(dir);
    }
    directories::ProjectDirs::from("", "", APP_NAME)
        .map(|d| d.config_dir().to_path_buf())
        .unwrap_or_else(|| PathBuf::from("."))
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct EditorSettings {
    pub font_size: f32,
    pub tab_size: usize,
    pub use_spaces: bool,
    pub auto_pair: bool,
    pub auto_close_tags: bool,
    pub show_invisibles: bool,
    pub show_indent_guides: bool,
    pub highlight_current_line: bool,
    /// Expand Emmet abbreviations with Tab in HTML files.
    pub emmet: bool,
    /// Wrap long lines at the window edge.
    pub word_wrap: bool,
    /// Remove spaces and tabs at line ends when saving.
    pub trim_trailing_whitespace: bool,
    /// End the file with a line break when saving.
    pub insert_final_newline: bool,
    /// Editor font family ("" = Cascadia Mono / Consolas).
    pub font_family: String,
    /// Line height as a multiple of the font's height.
    pub line_height: f32,
    /// Shrunken document view on the right edge.
    pub minimap: bool,
    pub autosave: AutoSave,
    /// Seconds without typing before [`AutoSave::Delay`] saves.
    pub autosave_delay: f32,
    /// Autosave server files too (each save is an upload).
    pub autosave_remote: bool,
    /// Keep earlier versions of files (see `history.rs`).
    pub local_history: bool,
    /// Versions kept per file.
    pub history_max: usize,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum AutoSave {
    #[default]
    Off,
    /// After a pause in typing.
    Delay,
    /// When switching tabs or leaving the window.
    FocusChange,
}

impl Default for EditorSettings {
    fn default() -> Self {
        Self {
            font_size: 14.0,
            tab_size: 4,
            use_spaces: true,
            auto_pair: true,
            auto_close_tags: true,
            show_invisibles: false,
            show_indent_guides: true,
            highlight_current_line: true,
            emmet: true,
            word_wrap: false,
            trim_trailing_whitespace: false,
            insert_final_newline: false,
            font_family: String::new(),
            line_height: 1.45,
            minimap: true,
            autosave: AutoSave::Off,
            autosave_delay: 2.0,
            autosave_remote: false,
            local_history: true,
            history_max: 30,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct AppSettings {
    pub editor: EditorSettings,
    pub dark_theme: bool,
    /// Name of the selected theme (empty: follow `dark_theme`).
    pub theme: String,
    /// Windows 11 Mica backdrop behind sidebar and title bar.
    pub mica: bool,
    pub sidebar_width: f32,
    pub last_folder: Option<PathBuf>,
    pub recent_files: Vec<PathBuf>,
    /// Project folders, most recent first.
    pub recent_folders: Vec<PathBuf>,
    /// Files that were open in each project folder (path string) when switching away.
    pub project_tabs: std::collections::BTreeMap<String, ProjectTabs>,
    /// Publishing targets per local project folder (path string).
    pub publish: std::collections::BTreeMap<String, PublishTarget>,
    /// Documents open when the app was last closed.
    pub session: Vec<SessionEntry>,
    pub session_active: Option<usize>,
    /// Per-language settings, keyed by `Language::id`.
    pub languages: std::collections::BTreeMap<String, LanguageSettings>,
    /// Live preview shown next to the editor, and its share of the width.
    pub preview: bool,
    pub preview_split: f32,
    /// Panel right of the editor (preview, outline), its tab and width.
    pub right_panel: bool,
    pub right_tab: RightTab,
    pub right_width: f32,
    /// Shell for new local terminals (name, empty: first found).
    pub terminal_shell: String,
    /// Height of the terminal panel.
    pub terminal_height: f32,
    /// Changed keyboard shortcuts: command -> "Ctrl+Shift+P, F1" (empty: none).
    pub keybindings: std::collections::BTreeMap<String, String>,
    /// Identifiers of installed extensions that are turned off.
    pub disabled_extensions: Vec<String>,
    /// Coding agents (ACP) and the one used for new sessions (by name).
    pub agents: Vec<crate::acp::AgentConfig>,
    pub agent: String,
}

/// Where a local project is published to (like Nova's project publishing).
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct PublishTarget {
    pub site_id: String,
    pub remote_path: String,
    /// Upload a file to the server every time it is saved locally.
    pub upload_on_save: bool,
    /// Modification time (seconds) of each file when it was last uploaded, by relative path.
    pub uploaded: std::collections::BTreeMap<String, u64>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ProjectTabs {
    pub entries: Vec<SessionEntry>,
    pub active: Option<usize>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct SessionEntry {
    pub local: Option<PathBuf>,
    pub site_id: Option<String>,
    pub remote_path: Option<String>,
    /// Cursor position (char index).
    pub cursor: usize,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct LanguageSettings {
    /// Additional file extensions (without dot) for this language.
    pub extensions: Vec<String>,
    pub tab_size: Option<usize>,
    pub use_spaces: Option<bool>,
    /// One entry per line: `word` or `trigger => expansion` (`$0` = cursor, `\n` = line break).
    pub completions: Vec<String>,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            editor: EditorSettings::default(),
            dark_theme: true,
            theme: String::new(),
            mica: true,
            sidebar_width: 260.0,
            last_folder: None,
            recent_files: Vec::new(),
            recent_folders: Vec::new(),
            project_tabs: Default::default(),
            languages: Default::default(),
            session: Vec::new(),
            publish: Default::default(),
            session_active: None,
            preview: false,
            preview_split: 0.45,
            terminal_height: 260.0,
            terminal_shell: String::new(),
            right_panel: false,
            right_tab: RightTab::Preview,
            right_width: 460.0,
            disabled_extensions: Vec::new(),
            keybindings: Default::default(),
            agents: crate::acp::default_agents(),
            agent: "Claude Code".into(),
        }
    }
}

impl AppSettings {
    fn path() -> PathBuf {
        config_dir().join("settings.json")
    }

    pub fn load() -> Self {
        std::fs::read(Self::path())
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) -> Result<()> {
        write_json(&Self::path(), self)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Protocol {
    Ftp,
    /// FTP with explicit TLS (AUTH TLS).
    Ftps,
    Sftp,
}

impl Protocol {
    pub const ALL: [Protocol; 3] = [Protocol::Sftp, Protocol::Ftp, Protocol::Ftps];

    pub fn label(self) -> &'static str {
        match self {
            Protocol::Ftp => "FTP",
            Protocol::Ftps => "FTPS (explizites TLS)",
            Protocol::Sftp => "SFTP",
        }
    }

    /// URI scheme for documents on this kind of server.
    pub fn scheme(self) -> &'static str {
        match self {
            Protocol::Ftp => "ftp",
            Protocol::Ftps => "ftps",
            Protocol::Sftp => "sftp",
        }
    }

    pub fn default_port(self) -> u16 {
        match self {
            Protocol::Ftp | Protocol::Ftps => 21,
            Protocol::Sftp => 22,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Site {
    pub id: String,
    pub name: String,
    pub protocol: Protocol,
    pub host: String,
    pub port: u16,
    pub user: String,
    /// Remote directory to open after connecting.
    pub root_path: String,
    /// SFTP: optional private key file.
    pub key_file: Option<PathBuf>,
    /// FTP: passive mode (recommended behind NAT).
    pub passive: bool,
    /// FTPS: accept self-signed or mismatching certificates.
    pub accept_invalid_certs: bool,
    /// Public URL that corresponds to `root_path`, used for "Open in browser".
    pub web_url: String,
    /// Commands run in the SSH terminal with one click (cache, deploy, …).
    pub commands: Vec<ServerCommand>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ServerCommand {
    pub name: String,
    pub command: String,
}

impl Default for Site {
    fn default() -> Self {
        Self {
            id: new_id(),
            name: String::new(),
            protocol: Protocol::Sftp,
            host: String::new(),
            port: 22,
            user: String::new(),
            root_path: String::new(),
            key_file: None,
            passive: true,
            accept_invalid_certs: false,
            web_url: String::new(),
            commands: Vec::new(),
        }
    }
}

impl Site {
    pub fn display_name(&self) -> String {
        if self.name.trim().is_empty() {
            format!("{}@{}", self.user, self.host)
        } else {
            self.name.clone()
        }
    }

    fn keyring_entry(&self) -> Result<keyring::Entry> {
        keyring::Entry::new(KEYRING_SERVICE, &format!("{}:{}", self.id, self.user))
            .context("Zugriff auf die Windows-Anmeldeinformationsverwaltung fehlgeschlagen")
    }

    /// Password (or key passphrase) from the Windows Credential Manager.
    pub fn password(&self) -> Option<String> {
        self.keyring_entry().ok()?.get_password().ok()
    }

    pub fn set_password(&self, password: &str) -> Result<()> {
        let entry = self.keyring_entry()?;
        if password.is_empty() {
            let _ = entry.delete_credential();
            Ok(())
        } else {
            entry
                .set_password(password)
                .context("Passwort konnte nicht gespeichert werden")
        }
    }

    pub fn delete_password(&self) {
        if let Ok(entry) = self.keyring_entry() {
            let _ = entry.delete_credential();
        }
    }
}

fn new_id() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or_default();
    format!("{nanos:x}")
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Sites {
    pub sites: Vec<Site>,
}

impl Sites {
    fn path() -> PathBuf {
        config_dir().join("sites.json")
    }

    pub fn load() -> Self {
        std::fs::read(Self::path())
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) -> Result<()> {
        write_json(&Self::path(), self)
    }

    pub fn get(&self, id: &str) -> Option<&Site> {
        self.sites.iter().find(|s| s.id == id)
    }
}

fn write_json(path: &PathBuf, value: &impl Serialize) -> Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, serde_json::to_vec_pretty(value)?)?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

impl AppSettings {
    /// Editor settings with the per-language indentation applied.
    pub fn editor_for(&self, language: crate::lang::Language) -> EditorSettings {
        let mut e = self.editor.clone();
        if let Some(l) = self.languages.get(language.id()) {
            if let Some(t) = l.tab_size {
                e.tab_size = t.max(1);
            }
            if let Some(s) = l.use_spaces {
                e.use_spaces = s;
            }
        }
        e
    }

    /// Editor settings for a document: language settings, then its `.editorconfig`.
    pub fn editor_for_doc(&self, doc: &crate::editor::Document) -> EditorSettings {
        let mut e = self.editor_for(doc.language);
        // Two trailing spaces are a line break in Markdown.
        if doc.language == crate::lang::Language::Markdown {
            e.trim_trailing_whitespace = false;
        }
        let c = &doc.config;
        if let Some(v) = c.use_spaces {
            e.use_spaces = v;
        }
        if let Some(v) = c.tab_size {
            e.tab_size = v;
        }
        if let Some(v) = c.trim_trailing_whitespace {
            e.trim_trailing_whitespace = v;
        }
        if let Some(v) = c.insert_final_newline {
            e.insert_final_newline = v;
        }
        e
    }

    /// Publishes extension mappings and custom completions to the editor.
    pub fn apply_languages(&self) {
        use crate::lang::Language;
        let mut ext = std::collections::HashMap::new();
        let mut completions = std::collections::HashMap::new();
        for (id, l) in &self.languages {
            let Some(lang) = Language::from_id(id) else { continue };
            for e in &l.extensions {
                let e = e.trim().trim_start_matches('.').to_ascii_lowercase();
                if !e.is_empty() {
                    ext.insert(e, lang);
                }
            }
            if !l.completions.is_empty() {
                completions.insert(lang, l.completions.clone());
            }
        }
        Language::set_overrides(ext);
        crate::editor::complete::set_user_completions(completions);
    }
}

#[cfg(test)]
mod keyring_tests {
    /// Manual helper: `NEBULA_SITE=id:user NEBULA_PASS=secret cargo test set_site_password -- --ignored`
    /// (empty password removes the entry).
    #[test]
    #[ignore]
    fn set_site_password() {
        let spec = std::env::var("NEBULA_SITE").unwrap();
        let (id, user) = spec.split_once(':').unwrap();
        let site = super::Site { id: id.into(), user: user.into(), ..Default::default() };
        site.set_password(&std::env::var("NEBULA_PASS").unwrap_or_default()).unwrap();
    }
}

/// Content of the right panel.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum RightTab {
    #[default]
    Preview,
    Outline,
    /// Chat with a coding agent.
    Agent,
}

#[cfg(test)]
mod tests {
    #[test]
    fn sites_without_commands_still_load() {
        let old = r#"{"sites":[{"id":"a","name":"Alt","protocol":"Sftp","host":"h","port":22,"user":"u","root_path":"/"}]}"#;
        let sites: super::Sites = serde_json::from_str(old).unwrap();
        assert!(sites.sites[0].commands.is_empty());
        let mut site = sites.sites[0].clone();
        site.commands.push(super::ServerCommand { name: "Cache".into(), command: "php artisan cache:clear".into() });
        let back: super::Site = serde_json::from_str(&serde_json::to_string(&site).unwrap()).unwrap();
        assert_eq!(back.commands, site.commands);
    }
}
