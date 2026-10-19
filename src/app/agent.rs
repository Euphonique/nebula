//! The "Agent" tab of the right panel: a chat with a coding agent over ACP
//! (Claude Code, Gemini CLI, Codex …). The agent reads files through Nebula
//! (open buffers with unsaved changes win), edits land in open tabs as one
//! undoable step and are saved, and every tool use can be confirmed first.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use eframe::egui::{self, Color32, Rect, RichText, Sense, Stroke, pos2, vec2};

use super::NebulaApp;
use crate::acp::schema::{PermissionOptionKind, ToolCallContent, ToolCallStatus, ToolKind};
use crate::acp::session::{AgentSession, Attachment, Entry, FileRequest, Phase, ToolEntry};
use crate::remote::Request;
use crate::ui::sidebar::FileRef;
use crate::editor::DocSource;
use crate::ui::{icons, markdown};

#[derive(Default)]
pub(super) struct AgentUi {
    pub session: Option<AgentSession>,
    input: String,
    /// Send the active file along with the next message.
    attach_file: bool,
    /// Message typed before the agent was ready.
    queued: Option<(String, Vec<Attachment>)>,
    /// Files attached from the sidebar for the next message, and server reads for them.
    attachments: Vec<PendingAttachment>,
    pending_attach: HashMap<u64, FileRef>,
    /// Focus the input on the next frame.
    focus_input: bool,
    /// Node.js being downloaded for the agents, and the last download error.
    node_dl: Option<NodeDownload>,
    node_error: Option<String>,
}

/// Larger files are not attached (the agent can read them itself).
const MAX_ATTACH: usize = 1024 * 1024;

struct PendingAttachment {
    file: FileRef,
    label: String,
    /// `None` while a server file is being read.
    loaded: Option<Attachment>,
}

struct NodeDownload {
    progress: std::sync::Arc<crate::acp::node::Progress>,
    handle: std::thread::JoinHandle<Result<String, String>>,
}

/// What a dropdown under the conversation changes.
#[derive(Clone)]
enum SelKind {
    Agent,
    Config(String),
    /// Older agents: modes without config options.
    Mode,
}

type Pick = (SelKind, String);

struct Selector {
    id: String,
    kind: SelKind,
    label: String,
    tip: Option<String>,
    current: String,
    choices: Vec<(String, String, Option<String>)>,
}

impl NebulaApp {
    /// Dropdowns: the agent, then its settings – model, effort, mode, others.
    fn agent_selectors(&self) -> Vec<Selector> {
        use crate::acp::session::ConfigValue;
        let agents = self.settings.agents.iter().map(|a| (a.name.clone(), a.name.clone(), None)).collect();
        let mut out = vec![Selector {
            id: "agent".into(),
            kind: SelKind::Agent,
            label: self.settings.agent.clone(),
            tip: Some("Agent".into()),
            current: self.settings.agent.clone(),
            choices: agents,
        }];
        let Some(s) = &self.agent.session else { return out };
        let rank = |c: &str| match c {
            "model" => 0,
            "thought_level" => 1,
            "mode" => 2,
            _ => 3,
        };
        let mut options: Vec<_> = s.config.iter().filter(|o| o.id != "agent").collect();
        options.sort_by_key(|o| rank(&o.category));
        for o in options {
            let prefix = match o.category.as_str() {
                "model" => String::new(),
                "thought_level" => "Aufwand: ".into(),
                "mode" => "Modus: ".into(),
                _ => format!("{}: ", o.name),
            };
            let (current, choices) = match &o.value {
                ConfigValue::Select { current, options } => (current.clone(), options.clone()),
                ConfigValue::Boolean(b) => (b.to_string(), vec![("true".into(), "an".into(), None), ("false".into(), "aus".into(), None)]),
            };
            let tip = match &o.description {
                Some(d) => format!("{}: {d}", o.name),
                None => o.name.clone(),
            };
            out.push(Selector {
                id: o.id.clone(),
                kind: SelKind::Config(o.id.clone()),
                label: format!("{prefix}{}", o.current_label()),
                tip: Some(tip),
                current,
                choices,
            });
        }
        // Modes the old way if the agent has no "mode" setting.
        if !s.config.iter().any(|o| o.category == "mode")
            && let Some(modes) = &s.modes
            && modes.available_modes.len() > 1
        {
            let current = modes.current_mode_id.to_string();
            let name = modes.available_modes.iter().find(|m| m.id.to_string() == current).map_or(current.clone(), |m| m.name.clone());
            out.push(Selector {
                id: "legacy-mode".into(),
                kind: SelKind::Mode,
                label: format!("Modus: {name}"),
                tip: Some("Modus".into()),
                current,
                choices: modes.available_modes.iter().map(|m| (m.id.to_string(), m.name.clone(), m.description.clone())).collect(),
            });
        }
        out
    }

    fn apply_agent_pick(&mut self, (kind, value): Pick) {
        match kind {
            SelKind::Agent => {
                if value != self.settings.agent {
                    self.settings.agent = value;
                    self.new_agent_session();
                }
            }
            SelKind::Mode => {
                if let Some(s) = &mut self.agent.session {
                    s.set_mode(&value);
                }
            }
            SelKind::Config(id) => {
                let Some(s) = &mut self.agent.session else { return };
                s.set_config(&id, &value);
                // Remember model, effort … for this agent; the mode deliberately not
                // (every conversation starts with the safe default).
                let remember = s.config.iter().any(|o| o.id == id && o.category != "mode");
                let name = s.agent.name.clone();
                if remember && let Some(cfg) = self.settings.agents.iter_mut().find(|a| a.name == name) {
                    cfg.options.insert(id, value);
                    let _ = self.settings.save();
                }
            }
        }
    }

    fn start_node_download(&mut self) {
        if self.agent.node_dl.is_some() {
            return;
        }
        // A running agent keeps Nebula's node.exe open; the new one replaces it.
        self.agent.session = None;
        let progress = std::sync::Arc::new(crate::acp::node::Progress::default());
        let p = progress.clone();
        let handle = std::thread::spawn(move || crate::acp::node::download(p));
        self.agent.node_dl = Some(NodeDownload { progress, handle });
        self.agent.node_error = None;
    }

    pub(super) fn agent_node_downloading(&self) -> bool {
        self.agent.node_dl.is_some()
    }

    /// Finishes a Node download; a waiting agent starts again.
    fn poll_node_download(&mut self, ctx: &egui::Context) {
        let Some(dl) = &self.agent.node_dl else { return };
        if !dl.handle.is_finished() {
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
            return;
        }
        let dl = self.agent.node_dl.take().unwrap();
        match dl.handle.join().unwrap_or_else(|_| Err("Download abgebrochen.".into())) {
            Ok(version) => {
                self.info(format!("Node.js {version} ist bereit."));
                if self.agent.session.as_ref().is_some_and(|s| s.phase == Phase::NeedsNode) {
                    self.agent.session = None;
                }
            }
            Err(e) => self.agent.node_error = Some(e),
        }
    }

    /// Button or progress bar for getting Node.js.
    pub(super) fn node_download_ui(&mut self, ui: &mut egui::Ui, label: &str) {
        let theme = self.theme.clone();
        self.poll_node_download(ui.ctx());
        match &self.agent.node_dl {
            Some(dl) => {
                let text = match dl.progress.fraction() {
                    Some(f) if f >= 1.0 => "Node.js wird entpackt …".to_string(),
                    Some(f) => format!("Node.js wird geladen … {:.0} %", f * 100.0),
                    None => "Node.js wird geladen …".into(),
                };
                ui.add(egui::ProgressBar::new(dl.progress.fraction().unwrap_or(0.0)).text(text).desired_width(320.0));
            }
            None => {
                if ui.button(label).clicked() {
                    self.start_node_download();
                }
            }
        }
        if let Some(e) = &self.agent.node_error {
            ui.label(RichText::new(e).color(theme.danger));
        }
    }

    /// Opens the agent tab of the right panel (focusing the input).
    pub(super) fn show_agent(&mut self) {
        self.settings.right_panel = true;
        self.settings.right_tab = crate::settings::RightTab::Agent;
        self.sync_preview_flag();
        self.agent.focus_input = true;
    }

    /// Shows the agent, or closes the panel if it already shows it.
    pub(super) fn toggle_agent(&mut self) {
        if self.settings.right_panel && self.settings.right_tab == crate::settings::RightTab::Agent {
            self.settings.right_panel = false;
            self.sync_preview_flag();
        } else {
            self.show_agent();
        }
    }

    /// Ends the current conversation; the next message starts a new one.
    pub(super) fn new_agent_session(&mut self) {
        self.agent.session = None;
        self.agent.queued = None;
        self.agent.focus_input = true;
    }

    /// Folder the agent works in: the project, else the active file's folder.
    fn agent_cwd(&self) -> PathBuf {
        if let Some(root) = &self.sidebar.root {
            return root.clone();
        }
        if let Some(DocSource::Local(p)) = self.active.map(|i| &self.docs[i].source)
            && let Some(dir) = p.parent()
        {
            return dir.to_path_buf();
        }
        std::env::var_os("USERPROFILE").map(PathBuf::from).unwrap_or_else(std::env::temp_dir)
    }

    fn start_agent(&mut self, ctx: &egui::Context) {
        let cfg = self
            .settings
            .agents
            .iter()
            .find(|a| a.name == self.settings.agent)
            .or(self.settings.agents.first())
            .cloned();
        let Some(cfg) = cfg else {
            self.error("Kein Agent eingerichtet (Einstellungen › KI-Agenten).");
            return;
        };
        let ctx = ctx.clone();
        self.agent.session = Some(AgentSession::start(cfg, self.agent_cwd(), move || ctx.request_repaint()));
    }

    /// Handles agent messages; called every frame.
    pub(super) fn poll_agent(&mut self) {
        let Some(session) = &mut self.agent.session else { return };
        let requests = session.poll();
        if session.phase == Phase::Ready
            && let Some((text, context)) = self.agent.queued.take()
        {
            session.prompt(&text, &context);
        }
        for req in requests {
            match req {
                FileRequest::Read { rpc_id, path, line, limit } => {
                    let result = self.agent_read(&path).map(|t| Some(crate::acp::session::slice_lines(&t, line, limit)));
                    if let Some(s) = &mut self.agent.session {
                        s.reply_file(&rpc_id, result);
                    }
                }
                FileRequest::Write { rpc_id, path, content } => {
                    let result = self.agent_write(&path, &content).map(|_| None);
                    if let Some(s) = &mut self.agent.session {
                        s.reply_file(&rpc_id, result);
                    }
                }
            }
        }
    }

    fn local_doc(&self, path: &Path) -> Option<usize> {
        let want = crate::ui::sidebar::normalize_path(path).to_string_lossy().to_lowercase();
        self.docs.iter().position(|d| match &d.source {
            DocSource::Local(p) => d.image.is_none() && p.to_string_lossy().to_lowercase() == want,
            _ => false,
        })
    }

    /// Text of a file for the agent: the open buffer if there is one.
    fn agent_read(&mut self, path: &Path) -> Result<String, String> {
        if let Some(i) = self.local_doc(path) {
            return Ok(self.docs[i].text().to_string());
        }
        let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
        if super::is_binary(&bytes) {
            return Err(format!("{} ist keine Textdatei.", path.display()));
        }
        Ok(String::from_utf8_lossy(&bytes).into_owned())
    }

    /// Writes for the agent: into the open tab (undoable, then saved) or to disk.
    fn agent_write(&mut self, path: &Path, content: &str) -> Result<(), String> {
        if let Some(i) = self.local_doc(path) {
            self.docs[i].replace_text(content);
            if !self.save_doc(i, false) {
                return Err(format!("{} konnte nicht gespeichert werden.", path.display()));
            }
            return Ok(());
        }
        let source = DocSource::Local(path.to_path_buf());
        if let Ok(old) = std::fs::read(path) {
            // The previous content stays in the local history.
            self.record_history(&source, &old, crate::history::Kind::Saved);
        } else if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        }
        std::fs::write(path, content).map_err(|e| format!("{}: {e}", path.display()))?;
        self.sidebar.refresh_local(path.parent());
        // Like saving in the editor: with "upload on save" the change goes to the server.
        self.upload_on_save(&crate::ui::sidebar::normalize_path(path));
        Ok(())
    }

    fn send_agent_message(&mut self, ctx: &egui::Context) {
        let text = self.agent.input.trim().to_string();
        if text.is_empty() {
            return;
        }
        // Files attached from the sidebar (those still loading are skipped) and the active tab.
        let mut context: Vec<Attachment> = self.agent.attachments.drain(..).filter_map(|a| a.loaded).collect();
        if self.agent.attach_file
            && let Some(i) = self.active
            && let Some(a) = self.doc_attachment(i)
            && !context.iter().any(|c| c.uri == a.uri)
        {
            context.push(a);
        }
        self.agent.pending_attach.clear();
        self.agent.input.clear();
        let needs_start = self.agent.session.as_ref().is_none_or(|s| matches!(s.phase, Phase::Failed(_) | Phase::Exited));
        if needs_start {
            self.start_agent(ctx);
        }
        match &mut self.agent.session {
            Some(s) if s.phase == Phase::Ready => s.prompt(&text, &context),
            Some(_) => self.agent.queued = Some((text, context)),
            None => {}
        }
    }

    /// An open tab as attachment, with its unsaved changes.
    fn doc_attachment(&mut self, i: usize) -> Option<Attachment> {
        if self.docs[i].image.is_some() {
            return None;
        }
        let text = self.docs[i].text().to_string();
        match self.docs[i].source.clone() {
            DocSource::Local(p) => Some(Attachment::local(&p, text)),
            DocSource::Remote { site_id, path } => {
                let server = self.sites.get(&site_id).map(|s| s.display_name()).unwrap_or(site_id);
                Some(Attachment::remote(&server, &path, text))
            }
            DocSource::Untitled(_) => None,
        }
    }

    /// Adds files from the sidebar to the next message (server files are read first).
    pub(super) fn attach_to_agent(&mut self, files: Vec<FileRef>) {
        self.show_agent();
        for file in files {
            if self.agent.attachments.iter().any(|a| a.file == file) {
                continue;
            }
            // An open tab wins: it may have unsaved changes.
            let open = self.docs.iter().position(|d| match (&d.source, &file) {
                (DocSource::Local(p), FileRef::Local(f)) => p == f,
                (DocSource::Remote { site_id, path }, FileRef::Remote { site_id: s, path: p }) => site_id == s && path == p,
                _ => false,
            });
            let loaded = match (open, &file) {
                (Some(i), _) => self.doc_attachment(i),
                (None, FileRef::Local(p)) => match std::fs::read(p) {
                    Ok(bytes) if super::is_binary(&bytes) => {
                        self.error(format!("{} ist keine Textdatei.", p.display()));
                        continue;
                    }
                    Ok(bytes) if bytes.len() > MAX_ATTACH => {
                        self.error(format!("{} ist zu groß zum Anhängen.", p.display()));
                        continue;
                    }
                    Ok(bytes) => Some(Attachment::local(p, String::from_utf8_lossy(&bytes).into_owned())),
                    Err(e) => {
                        self.error(format!("{}: {e}", p.display()));
                        continue;
                    }
                },
                (None, FileRef::Remote { site_id, path }) => {
                    match self.send_remote(site_id, Request::Read(path.clone())) {
                        Some(tag) => {
                            self.agent.pending_attach.insert(tag, file.clone());
                        }
                        None => {
                            self.error("Server nicht gefunden.");
                            continue;
                        }
                    }
                    None
                }
            };
            let label = match &file {
                FileRef::Local(p) => p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
                FileRef::Remote { path, .. } => path.rsplit('/').next().unwrap_or(path).to_string(),
            };
            self.agent.attachments.push(PendingAttachment { file, label, loaded });
        }
    }

    /// A server file read for an attachment arrived; false if `tag` is not one.
    pub(super) fn agent_read_done(&mut self, tag: u64, data: Result<&[u8], String>) -> bool {
        let Some(file) = self.agent.pending_attach.remove(&tag) else { return false };
        let FileRef::Remote { site_id, path } = &file else { return true };
        let server = self.sites.get(site_id).map(|s| s.display_name()).unwrap_or_else(|| site_id.clone());
        match data {
            Ok(bytes) if super::is_binary(bytes) || bytes.len() > MAX_ATTACH => {
                self.agent.attachments.retain(|a| a.file != file);
                self.error(format!("{path} lässt sich nicht anhängen (keine Textdatei oder zu groß)."));
            }
            Ok(bytes) => {
                let a = Attachment::remote(&server, path, String::from_utf8_lossy(bytes).into_owned());
                if let Some(p) = self.agent.attachments.iter_mut().find(|a| a.file == file) {
                    p.loaded = Some(a);
                }
            }
            Err(e) => {
                self.agent.attachments.retain(|a| a.file != file);
                self.error(format!("{path}: {e}"));
            }
        }
        true
    }

    /// The agent tab: toolbar, conversation, questions and the input.
    pub(super) fn agent_pane(&mut self, ui: &mut egui::Ui, header: Rect, body: Rect) {
        let theme = self.theme.clone();
        let ctx = ui.ctx().clone();
        // Start the agent right away: login state and plan limits show before the first message.
        if self.agent.session.is_none() && !self.settings.agents.is_empty() && self.agent.node_dl.is_none() {
            self.start_agent(&ctx);
        }

        // Header button: new conversation.
        let r = Rect::from_center_size(pos2(header.right() - 13.0, header.center().y), vec2(26.0, 24.0));
        let resp = ui.interact(r, egui::Id::unique("agent-new"), Sense::click()).on_hover_text("Neue Unterhaltung");
        if resp.hovered() {
            ui.painter().rect_filled(r, 6.0, theme.hover);
        }
        icons::plus(ui.painter(), r.center(), if resp.hovered() { theme.foreground } else { theme.text_muted });
        if resp.clicked() {
            self.new_agent_session();
        }

        let area = body.shrink2(vec2(10.0, 0.0));
        let mut ui = ui.new_child(egui::UiBuilder::new().max_rect(area).layout(egui::Layout::top_down(egui::Align::Min)));
        let ui = &mut ui;

        // Bottom part first (built upwards), so the conversation gets the rest.
        let top = ui.cursor().top();
        let bottom_rect = Rect::from_min_max(pos2(area.left(), top), pos2(area.right(), area.bottom() - 8.0));
        let mut bottom = ui.new_child(egui::UiBuilder::new().max_rect(bottom_rect).layout(egui::Layout::bottom_up(egui::Align::Min)));
        let working = self.agent.session.as_ref().is_some_and(|s| s.is_working()) || self.agent.queued.is_some();
        let mut send = false;
        {
            let ui = &mut bottom;
            // Send/stop button and agent name.
            ui.horizontal(|ui| {
                let mut refresh = false;
                if let Some(s) = &self.agent.session {
                    match &s.limits {
                        Some(l) => refresh = limits_ui(ui, &theme, l, s.agent_info.as_deref()).clicked(),
                        None => {
                            if let Some(info) = &s.agent_info {
                                ui.label(RichText::new(info).size(11.0).color(theme.text_faint));
                            }
                        }
                    }
                }
                if refresh && let Some(s) = &mut self.agent.session {
                    s.refresh_limits();
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if working {
                        let stop = egui::Button::new(RichText::new("Stopp")).min_size(vec2(70.0, 26.0));
                        if ui.add(stop).clicked() {
                            self.agent.queued = None;
                            if let Some(s) = &mut self.agent.session {
                                s.cancel();
                            }
                        }
                    } else {
                        let enabled = !self.agent.input.trim().is_empty();
                        let button = egui::Button::new(RichText::new("Senden").color(Color32::WHITE)).fill(theme.accent).min_size(vec2(70.0, 26.0));
                        if ui.add_enabled(enabled, button).clicked() {
                            send = true;
                        }
                    }
                });
            });
            ui.add_space(4.0);
            let edit = egui::TextEdit::multiline(&mut self.agent.input)
                .hint_text(if working { "Der Agent arbeitet …" } else { "Frag den Agenten oder gib ihm eine Aufgabe …" })
                .desired_rows(3)
                .desired_width(f32::INFINITY)
                .return_key(egui::KeyboardShortcut::new(egui::Modifiers::SHIFT, egui::Key::Enter));
            let resp = ui.add(edit);
            if self.agent.focus_input {
                resp.request_focus();
                self.agent.focus_input = false;
            }
            if resp.has_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter) && !i.modifiers.shift) {
                send = true;
            }
            ui.add_space(4.0);
            // Agent and its settings (model, effort, mode …), right above the input.
            let rows = self.agent_selectors();
            let width = ((ui.available_width() - 6.0) / 2.0).max(80.0);
            let mut picked: Option<Pick> = None;
            // Built upwards: the last row first.
            for row in rows.chunks(2).rev() {
                ui.horizontal(|ui| {
                    for sel in row {
                        let resp = egui::ComboBox::from_id_salt(("agent-sel", &sel.id))
                            .selected_text(&sel.label)
                            .width(width - 8.0)
                            .show_ui(ui, |ui| {
                                for (value, name, description) in &sel.choices {
                                    let resp = ui.selectable_label(*value == sel.current, name);
                                    let resp = match description {
                                        Some(d) => resp.on_hover_text(d),
                                        None => resp,
                                    };
                                    if resp.clicked() {
                                        picked = Some((sel.kind.clone(), value.clone()));
                                    }
                                }
                            });
                        if let Some(tip) = &sel.tip {
                            resp.response.on_hover_text(tip);
                        }
                    }
                });
            }
            if let Some(pick) = picked {
                self.apply_agent_pick(pick);
            }
            ui.add_space(4.0);
            // Attach the active file.
            let file_name = self.active.and_then(|i| match &self.docs[i].source {
                DocSource::Local(_) | DocSource::Remote { .. } if self.docs[i].image.is_none() => Some(self.docs[i].source.file_name()),
                _ => None,
            });
            ui.horizontal(|ui| {
                if let Some(name) = &file_name {
                    ui.toggle_value(&mut self.agent.attach_file, format!("📎 {name}")).on_hover_text("Die aktuelle Datei mitschicken");
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(RichText::new("Umschalt+Enter: neue Zeile").size(11.0).color(theme.text_faint));
                });
            });
            // Files attached from the sidebar; a click removes one.
            let mut remove = None;
            for row in self.agent.attachments.chunks(3).collect::<Vec<_>>().into_iter().rev() {
                ui.horizontal(|ui| {
                    for a in row {
                        let (text, tip) = match &a.loaded {
                            Some(l) => (format!("📄 {}  ✕", a.label), format!("{}\nKlicken zum Entfernen", l.label)),
                            None => (format!("⏳ {}", a.label), "Wird vom Server gelesen …".to_string()),
                        };
                        if ui.add(egui::Button::new(RichText::new(text).size(12.0)).corner_radius(10.0)).on_hover_text(tip).clicked() {
                            remove = Some(a.file.clone());
                        }
                    }
                });
            }
            if let Some(file) = remove {
                self.agent.attachments.retain(|a| a.file != file);
                self.agent.pending_attach.retain(|_, f| *f != file);
            }
        }

        // The conversation in the space left above; questions and the login
        // come at its end (it sticks to the bottom).
        let list_rect = Rect::from_min_max(pos2(area.left(), top), pos2(area.right(), bottom.min_rect().top() - 8.0));
        let mut list_ui = ui.new_child(egui::UiBuilder::new().max_rect(list_rect));
        if self.agent.session.as_ref().is_some_and(|s| s.phase == Phase::NeedsNode) {
            let ui = &mut list_ui;
            ui.add_space(24.0);
            ui.label(RichText::new("Node.js wird benötigt").size(15.0).strong());
            ui.add_space(4.0);
            ui.label(
                RichText::new(format!(
                    "Die KI-Agenten laufen mit Node.js. Nebula lädt es einmalig von nodejs.org (ca. 30 MB, Prüfsumme wird kontrolliert) \
                     nach {} – ohne Installation und ohne Adminrechte.",
                    crate::acp::node::runtime_dir().display()
                ))
                .color(theme.text_muted),
            );
            ui.add_space(8.0);
            self.node_download_ui(ui, "Node.js herunterladen");
            return;
        }
        let mut open_location: Option<(PathBuf, Option<u32>)> = None;
        let mut answer: Option<Option<String>> = None;
        let mut auth: Option<crate::acp::session::AuthOption> = None;
        egui::ScrollArea::vertical()
            .id_salt("agent-log")
            .auto_shrink([false, false])
            .stick_to_bottom(true)
            .show(&mut list_ui, |ui| {
                ui.set_width(ui.available_width());
                let Some(s) = &self.agent.session else {
                    intro(ui, &theme, &self.settings.agent);
                    return;
                };
                if s.entries.is_empty() && matches!(s.phase, Phase::Ready | Phase::Starting) {
                    intro(ui, &theme, &self.settings.agent);
                }
                for (n, entry) in s.entries.iter().enumerate() {
                    entry_ui(ui, &theme, n, entry, &mut open_location);
                    ui.add_space(6.0);
                }
                status(ui, &theme, s);
                if let Some(p) = s.permissions.front() {
                    answer = permission_card(ui, &theme, p);
                }
                if let Phase::NeedsAuth(methods) = &s.phase {
                    ui.label(RichText::new("Der Agent möchte, dass du dich anmeldest:").color(theme.foreground));
                    for m in methods {
                        let resp = ui.button(&m.name);
                        let resp = match &m.description {
                            Some(d) => resp.on_hover_text(d),
                            None => resp,
                        };
                        if resp.clicked() {
                            auth = Some(m.clone());
                        }
                    }
                }
            });
        if let Some(a) = answer
            && let Some(s) = &mut self.agent.session
        {
            s.answer_permission(a.as_deref());
        }
        if let Some(m) = auth {
            self.agent_login(&m);
        }
        if let Some((path, line)) = open_location {
            self.open_local(&path);
            if let (Some(line), Some(i)) = (line, self.active) {
                self.jump_to_line(i, line.saturating_sub(1) as usize);
            }
        }
        if send && !working {
            self.send_agent_message(&ctx);
        }
    }

    /// Logs in: interactive methods run in a console window, others through the agent.
    fn agent_login(&mut self, m: &crate::acp::session::AuthOption) {
        if m.api_key {
            // Keys are entered on the settings page.
            self.open_settings(super::settings_page::Page::Agents);
            return;
        }
        match &m.terminal {
            Some(login) => {
                let Some(s) = &self.agent.session else { return };
                match crate::acp::run_login_console(&s.agent, login, &s.cwd) {
                    Ok(()) => self.info("Melde dich im geöffneten Fenster an und starte danach eine neue Unterhaltung."),
                    Err(e) => self.error(e),
                }
            }
            None => {
                if let Some(s) = &mut self.agent.session {
                    s.authenticate(&m.id);
                }
            }
        }
    }
}

/// Remaining plan limits ("5 Std. 88 % frei · Woche 12 % frei"); click reloads.
fn limits_ui(ui: &mut egui::Ui, theme: &crate::theme::Theme, l: &crate::acp::usage::Limits, info: Option<&str>) -> egui::Response {
    use crate::acp::usage;
    let now = usage::now();
    let color = |free: f32| {
        if free < 10.0 {
            theme.danger
        } else if free < 25.0 {
            theme.modified
        } else {
            theme.text_muted
        }
    };
    let mut job = egui::text::LayoutJob::default();
    let mut tip = String::new();
    for (name, long, w) in [("5 Std.", "5-Stunden-Limit", l.five_hour()), ("Woche", "Wochenlimit", l.weekly())] {
        let Some(w) = w else { continue };
        let free = (100.0 - w.used).max(0.0);
        if !job.sections.is_empty() {
            job.append("  ·  ", 0.0, egui::TextFormat::simple(egui::FontId::proportional(11.5), theme.text_faint));
        }
        job.append(&format!("{name} "), 0.0, egui::TextFormat::simple(egui::FontId::proportional(11.5), theme.text_faint));
        job.append(&format!("{free:.0} % frei"), 0.0, egui::TextFormat::simple(egui::FontId::proportional(11.5), color(free)));
        tip.push_str(&format!("{long}: {:.0} % verbraucht", w.used));
        if let Some(r) = &w.reset {
            tip.push_str(&format!(", wieder voll in {} ({:02}.{:02}. {:02}:{:02} Uhr)", usage::until(r.at, now), r.day, r.month, r.hour, r.minute));
        }
        tip.push('\n');
    }
    if let Some(plan) = &l.plan {
        tip.push_str(&format!("Abo: Claude {plan}\n"));
    }
    if let Some(info) = info {
        tip.push_str(info);
        tip.push('\n');
    }
    let age = if now - l.fetched < 60 { "gerade eben".to_string() } else { format!("vor {}", usage::until(now, l.fetched)) };
    tip.push_str(&format!("Stand: {age} – klicken zum Aktualisieren"));
    ui.add(egui::Label::new(job).sense(Sense::click())).on_hover_text(tip)
}

/// The agent asks before running a tool; returns the chosen option.
fn permission_card(ui: &mut egui::Ui, theme: &crate::theme::Theme, p: &crate::acp::session::Permission) -> Option<Option<String>> {
    let mut answer = None;
    egui::Frame::new()
        .fill(theme.background)
        .stroke(Stroke::new(1.0, theme.accent.gamma_multiply(0.7)))
        .corner_radius(8.0)
        .inner_margin(egui::Margin::same(8))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new("Der Agent möchte:").size(11.5).color(theme.text_faint));
            ui.label(RichText::new(if p.tool.title.is_empty() { "ein Werkzeug ausführen" } else { &p.tool.title }).strong());
            tool_content(ui, theme, &p.tool, &mut None);
            ui.add_space(4.0);
            ui.horizontal_wrapped(|ui| {
                for o in &p.options {
                    let allow = matches!(o.kind, PermissionOptionKind::AllowOnce | PermissionOptionKind::AllowAlways);
                    let button = if allow {
                        egui::Button::new(RichText::new(&o.name).color(Color32::WHITE)).fill(theme.accent)
                    } else {
                        egui::Button::new(&o.name)
                    };
                    if ui.add(button).clicked() {
                        answer = Some(Some(o.option_id.to_string()));
                    }
                }
            });
        });
    answer
}

fn intro(ui: &mut egui::Ui, theme: &crate::theme::Theme, agent: &str) {
    ui.add_space(30.0);
    ui.vertical_centered(|ui| {
        let (r, _) = ui.allocate_exact_size(vec2(40.0, 40.0), Sense::hover());
        let painter = ui.painter().clone();
        let scaled = |p: &egui::Painter, c: egui::Pos2| icons::sparkle(p, c, theme.accent);
        scaled(&painter, r.center());
        ui.add_space(8.0);
        ui.label(RichText::new(agent).size(16.0).strong());
        ui.add_space(4.0);
        ui.label(
            RichText::new("Der Agent arbeitet im geöffneten Projekt. Vor Änderungen an Dateien und vor Befehlen fragt er nach.")
                .color(theme.text_muted),
        );
    });
}

fn status(ui: &mut egui::Ui, theme: &crate::theme::Theme, s: &AgentSession) {
    match &s.phase {
        Phase::Starting => {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(RichText::new(format!("{} wird gestartet … (beim ersten Mal wird er installiert)", s.agent.name)).color(theme.text_muted));
            });
        }
        Phase::Working if s.permissions.is_empty() => {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(RichText::new("Arbeitet …").color(theme.text_muted));
            });
        }
        Phase::Failed(e) => {
            ui.label(RichText::new(e).color(theme.danger));
            if !s.log.is_empty() {
                egui::CollapsingHeader::new("Protokoll").id_salt("agent-log-out").show(ui, |ui| {
                    let text: Vec<&str> = s.log.iter().map(String::as_str).collect();
                    ui.label(RichText::new(text.join("\n")).monospace().size(11.0).color(theme.text_muted));
                });
            }
        }
        _ => {}
    }
}

fn entry_ui(ui: &mut egui::Ui, theme: &crate::theme::Theme, n: usize, entry: &Entry, open: &mut Option<(PathBuf, Option<u32>)>) {
    match entry {
        Entry::User(text) => {
            egui::Frame::new()
                .fill(theme.hover.gamma_multiply(1.4))
                .corner_radius(8.0)
                .inner_margin(egui::Margin::symmetric(10, 7))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.add(egui::Label::new(RichText::new(text).color(theme.foreground)).selectable(true));
                });
        }
        Entry::Agent(text) => markdown::show(ui, theme, text),
        Entry::Thought(text) => {
            egui::CollapsingHeader::new(RichText::new("Gedanken").size(12.0).color(theme.text_faint))
                .id_salt(("thought", n))
                .show(ui, |ui| {
                    ui.label(RichText::new(text).size(12.5).color(theme.text_muted));
                });
        }
        Entry::Tool(t) => tool_ui(ui, theme, n, t, open),
        Entry::Plan(items) => {
            egui::Frame::new().stroke(Stroke::new(1.0, theme.hover.gamma_multiply(1.6))).corner_radius(8.0).inner_margin(egui::Margin::same(8)).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.label(RichText::new("Plan").size(11.5).color(theme.text_faint));
                for item in items {
                    let (mark, color) = if item.done {
                        ("☑", theme.text_faint)
                    } else if item.active {
                        ("▶", theme.accent)
                    } else {
                        ("☐", theme.foreground)
                    };
                    let text = RichText::new(format!("{mark}  {}", item.text)).color(color);
                    ui.label(if item.done { text.strikethrough() } else { text });
                }
            });
        }
        Entry::Notice { text, error } => {
            ui.label(RichText::new(text).size(12.5).color(if *error { theme.danger } else { theme.text_muted }));
        }
    }
}

fn tool_ui(ui: &mut egui::Ui, theme: &crate::theme::Theme, n: usize, t: &ToolEntry, open: &mut Option<(PathBuf, Option<u32>)>) {
    let color = match t.status {
        ToolCallStatus::Completed => theme.success,
        ToolCallStatus::Failed => theme.danger,
        ToolCallStatus::InProgress => theme.accent,
        _ => theme.text_faint,
    };
    let kind = match t.kind {
        ToolKind::Read => "Lesen",
        ToolKind::Edit => "Bearbeiten",
        ToolKind::Delete => "Löschen",
        ToolKind::Move => "Verschieben",
        ToolKind::Search => "Suchen",
        ToolKind::Execute => "Befehl",
        ToolKind::Think => "Denken",
        ToolKind::Fetch => "Abrufen",
        ToolKind::SwitchMode => "Modus",
        _ => "Werkzeug",
    };
    let has_content = !t.content.is_empty();
    // Edits show their diff right away, other tools on demand.
    let default_open = matches!(t.kind, ToolKind::Edit | ToolKind::Delete | ToolKind::Move) || t.status == ToolCallStatus::Failed;
    let title = if t.title.is_empty() { kind.to_string() } else { t.title.clone() };
    let header = |ui: &mut egui::Ui| {
        let (r, _) = ui.allocate_exact_size(vec2(10.0, 16.0), Sense::hover());
        ui.painter().circle_filled(r.center(), 3.5, color);
        ui.label(RichText::new(kind).size(11.0).color(theme.text_faint));
        ui.add(egui::Label::new(RichText::new(&title).size(12.5).color(theme.foreground)).truncate());
    };
    if has_content {
        let id = ui.make_persistent_id(("tool", n));
        egui::collapsing_header::CollapsingState::load_with_default_open(ui.ctx(), id, default_open)
            .show_header(ui, |ui| ui.horizontal(header))
            .body(|ui| tool_content(ui, theme, t, &mut Some(open)));
    } else {
        ui.horizontal(header);
    }
    if !t.locations.is_empty() {
        ui.horizontal_wrapped(|ui| {
            ui.add_space(20.0);
            for (path, line) in t.locations.iter().take(6) {
                let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                let label = match line {
                    Some(l) => format!("{name}:{l}"),
                    None => name,
                };
                if ui.link(RichText::new(label).size(11.5)).on_hover_text(path.display().to_string()).clicked() {
                    *open = Some((path.clone(), *line));
                }
            }
        });
    }
}

fn tool_content(ui: &mut egui::Ui, theme: &crate::theme::Theme, t: &ToolEntry, open: &mut Option<&mut Option<(PathBuf, Option<u32>)>>) {
    for c in &t.content {
        match c {
            ToolCallContent::Diff(d) => {
                let name = d.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                let resp = ui.link(RichText::new(&name).size(11.5)).on_hover_text(d.path.display().to_string());
                if resp.clicked()
                    && let Some(open) = open
                {
                    **open = Some((d.path.clone(), None));
                }
                markdown::diff_rows(ui, theme, d.old_text.as_deref().unwrap_or(""), &d.new_text, 60);
            }
            ToolCallContent::Content(c) => {
                let text = crate::acp::session::text_of(&c.content);
                let shown: String = text.lines().take(30).collect::<Vec<_>>().join("\n");
                let more = text.lines().count().saturating_sub(30);
                egui::Frame::new().fill(theme.background).corner_radius(6.0).inner_margin(egui::Margin::symmetric(8, 5)).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.add(egui::Label::new(RichText::new(shown).monospace().size(11.5).color(theme.text_muted)).selectable(true));
                    if more > 0 {
                        ui.label(RichText::new(format!("… {more} weitere Zeilen")).size(11.0).color(theme.text_faint));
                    }
                });
            }
            _ => {}
        }
    }
}

