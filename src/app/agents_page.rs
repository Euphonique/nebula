//! Settings page "KI-Agenten": the coding agents (ACP) with their login state.
//! Logging in is done by the agent itself – in the browser (ChatGPT, Google)
//! or in a console window that opens the browser (Claude) – Nebula only
//! starts it and checks the result.

use std::collections::HashMap;

use eframe::egui::{self, Color32, RichText, Ui};

use super::{NebulaApp, primary_button};
use crate::acp::session::{AgentSession, Phase};
use crate::acp::{AgentConfig, PRESETS, preset, split_args};

/// Transient state of the page.
#[derive(Default)]
pub struct AgentsPage {
    /// Background sessions that check the login, by agent name.
    probes: HashMap<String, AgentSession>,
    edit: Option<Edit>,
    /// Hint shown after starting a console login, by agent name.
    console_hint: HashMap<String, String>,
    /// Space used by Node and the agents (measured in the background).
    sizes: Option<std::sync::Arc<std::sync::Mutex<Option<(u64, u64)>>>>,
}

impl AgentsPage {
    /// Ends the background sessions (when the settings close).
    pub fn stop(&mut self) {
        self.probes.clear();
        self.console_hint.clear();
        self.sizes = None;
    }
}

/// The agent being added or changed.
struct Edit {
    /// Position in the list; `None`: new agent.
    index: Option<usize>,
    name: String,
    command: String,
    args: String,
    env: String,
    key_env: String,
    key: String,
    has_key: bool,
    error: Option<String>,
    /// Remembered session settings, kept as they are.
    options: std::collections::BTreeMap<String, String>,
}

impl Edit {
    fn of(index: Option<usize>, cfg: &AgentConfig) -> Self {
        let mut env: Vec<String> = cfg.env.iter().map(|(k, v)| format!("{k}={v}")).collect();
        env.sort();
        Self {
            index,
            name: cfg.name.clone(),
            command: cfg.command.clone(),
            args: cfg.args.iter().map(|a| if a.contains(' ') { format!("\"{a}\"") } else { a.clone() }).collect::<Vec<_>>().join(" "),
            env: env.join("\n"),
            key_env: cfg.api_key_env.clone(),
            key: String::new(),
            has_key: !cfg.name.is_empty() && cfg.api_key().is_some(),
            error: None,
            options: cfg.options.clone(),
        }
    }

    fn config(&self) -> AgentConfig {
        AgentConfig {
            name: self.name.trim().to_string(),
            command: self.command.trim().to_string(),
            args: split_args(&self.args),
            env: self
                .env
                .lines()
                .filter_map(|l| l.split_once('='))
                .map(|(k, v)| (k.trim().to_string(), v.trim().to_string()))
                .filter(|(k, _)| !k.is_empty())
                .collect(),
            api_key_env: self.key_env.trim().to_string(),
            options: self.options.clone(),
        }
    }
}

impl NebulaApp {
    /// Where Node.js and the agents live, with their size.
    fn runtime_info(&mut self, ui: &mut Ui, state: &mut AgentsPage) {
        use crate::acp::node;
        let theme = self.theme.clone();
        let sizes = state
            .sizes
            .get_or_insert_with(|| {
                let cell = std::sync::Arc::new(std::sync::Mutex::new(None));
                let c = cell.clone();
                let ctx = ui.ctx().clone();
                std::thread::spawn(move || {
                    let measured = (node::folder_size(&node::private_dir()), node::folder_size(&node::agents_dir()));
                    *c.lock().unwrap() = Some(measured);
                    ctx.request_repaint();
                });
                cell
            })
            .lock()
            .ok()
            .and_then(|s| *s);
        let mb = |b: u64| format!("{:.0} MB", b as f64 / 1_048_576.0);
        egui::Frame::new().fill(theme.hover.gamma_multiply(0.6)).corner_radius(8.0).inner_margin(egui::Margin::same(10)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            let status = node::status();
            ui.horizontal(|ui| {
                ui.label(RichText::new("Node.js").strong());
                let (text, color) = match &status {
                    node::Status::Private(v) => {
                        let size = sizes.map(|(n, _)| format!(", {}", mb(n))).unwrap_or_default();
                        (format!("{v} – von Nebula verwaltet{size}"), theme.success)
                    }
                    node::Status::System(dir) => (format!("installiert ({})", dir.display()), theme.success),
                    node::Status::Missing => ("fehlt – wird für die Agenten benötigt".into(), theme.modified),
                };
                ui.label(RichText::new(text).color(color));
            });
            if let Some((_, agents)) = sizes {
                ui.label(RichText::new(format!("Agenten: {}", mb(agents))).size(12.0).color(theme.text_muted));
            }
            ui.horizontal(|ui| {
                match &status {
                    node::Status::Missing => self.node_download_ui(ui, "Node.js herunterladen (ca. 30 MB)"),
                    node::Status::Private(_) => {
                        self.node_download_ui(ui, "Auf neueste LTS-Version aktualisieren");
                        // Checking agents would keep the old Node busy.
                        if self.agent_node_downloading() {
                            state.probes.clear();
                        }
                    }
                    // An installed Node is used; Nebula's own copy would take precedence.
                    node::Status::System(_) => {}
                }
                if sizes.is_some_and(|(_, a)| a > 0)
                    && ui
                        .button("Agenten-Speicher leeren")
                        .on_hover_text("Löscht die geladenen Agenten; sie werden beim nächsten Start neu geladen. Anmeldungen bleiben erhalten.")
                        .clicked()
                {
                    // Running agents keep files open: stop them first.
                    state.probes.clear();
                    self.agent.session = None;
                    let _ = std::fs::remove_dir_all(node::agents_dir());
                    state.sizes = None;
                }
                if ui.button("Ordner öffnen").on_hover_text(crate::settings::data_dir().display().to_string()).clicked() {
                    let dir = crate::settings::data_dir();
                    let _ = std::fs::create_dir_all(&dir);
                    let _ = std::process::Command::new("explorer").arg(&dir).spawn();
                }
            });
        });
    }

    pub(super) fn agents_page(&mut self, ui: &mut Ui, state: &mut AgentsPage) {
        let theme = self.theme.clone();
        let ctx = ui.ctx().clone();
        let cwd = self.sidebar.root.clone().unwrap_or_else(std::env::temp_dir);
        for probe in state.probes.values_mut() {
            probe.poll();
        }

        ui.label(RichText::new("KI-Agenten").size(16.0).strong());
        ui.add_space(2.0);
        ui.label(
            RichText::new(
                "Agenten arbeiten im rechten Panel (✦) im geöffneten Projekt. Angemeldet wird beim jeweiligen Anbieter – \
                 im Browser mit deinem Konto oder per API-Schlüssel. Die Agenten werden beim ersten Start geladen.",
            )
            .color(theme.text_muted),
        );
        ui.add_space(8.0);
        self.runtime_info(ui, state);
        ui.add_space(8.0);

        if let Some(edit) = &mut state.edit {
            if let Some(done) = edit_form(ui, &theme, edit, &self.settings.agents) {
                if done {
                    let cfg = edit.config();
                    let old = edit.index.and_then(|i| self.settings.agents.get(i).cloned());
                    // The key belongs to the name: move it on renaming.
                    let mut key = (!edit.key.is_empty()).then(|| edit.key.clone());
                    if let Some(old) = &old
                        && old.name != cfg.name
                    {
                        key = key.or_else(|| old.api_key());
                        let _ = old.set_api_key("");
                        if self.settings.agent == old.name {
                            self.settings.agent = cfg.name.clone();
                        }
                    }
                    let saved = match key {
                        Some(k) => cfg.set_api_key(&k),
                        None => Ok(()),
                    };
                    match saved {
                        Ok(()) => {
                            if let Some(old) = &old {
                                state.probes.remove(&old.name);
                            }
                            match edit.index {
                                Some(i) => self.settings.agents[i] = cfg,
                                None => self.settings.agents.push(cfg),
                            }
                            state.edit = None;
                            let _ = self.settings.save();
                        }
                        Err(e) => edit.error = Some(format!("API-Schlüssel nicht gespeichert: {e}")),
                    }
                } else {
                    state.edit = None;
                }
            }
            return;
        }

        let mut remove = None;
        let mut edit = None;
        let mut edit_key: Option<Edit> = None;
        for (i, agent) in self.settings.agents.clone().iter().enumerate() {
            egui::Frame::new()
                .fill(theme.hover.gamma_multiply(0.6))
                .corner_radius(8.0)
                .inner_margin(egui::Margin::same(10))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal(|ui| {
                        let default = self.settings.agent == agent.name;
                        if ui.radio(default, "").on_hover_text("Für neue Unterhaltungen im Panel verwenden").clicked() {
                            self.settings.agent = agent.name.clone();
                        }
                        ui.label(RichText::new(&agent.name).size(14.5).strong());
                        ui.add_space(6.0);
                        status_label(ui, &theme, state.probes.get(&agent.name));
                    });
                    ui.label(RichText::new(agent.command_line()).monospace().size(11.0).color(theme.text_faint));
                    if agent.api_key().is_some() {
                        ui.label(RichText::new(format!("API-Schlüssel gespeichert ({})", agent.api_key_env)).size(11.5).color(theme.text_muted));
                    }
                    if let Some(hint) = state.console_hint.get(&agent.name) {
                        ui.label(RichText::new(hint).size(11.5).color(theme.accent));
                    }
                    ui.add_space(4.0);
                    ui.horizontal(|ui| {
                        let probe = state.probes.get(&agent.name);
                        let busy = probe.is_some_and(|p| p.phase == Phase::Starting);
                        if ui.add_enabled(!busy, egui::Button::new("Status prüfen")).clicked() {
                            let ctx = ctx.clone();
                            state.console_hint.remove(&agent.name);
                            state.probes.insert(agent.name.clone(), AgentSession::start(agent.clone(), cwd.clone(), move || ctx.request_repaint()));
                        }
                        // Login methods are known once the agent has answered.
                        if let Some(p) = state.probes.get_mut(&agent.name)
                            && !p.auth_methods.is_empty()
                        {
                            let methods = p.auth_methods.clone();
                            ui.menu_button("Anmelden ▾", |ui| {
                                for m in &methods {
                                    let resp = ui.button(&m.name);
                                    let resp = match &m.description {
                                        Some(d) => resp.on_hover_text(d),
                                        None => resp,
                                    };
                                    if resp.clicked() {
                                        ui.close();
                                        if m.api_key {
                                            // Keys are entered in the form and given to the agent as variable.
                                            let mut e = Edit::of(Some(i), agent);
                                            e.error = Some(format!("Trage den Schlüssel für „{}“ ein und prüfe danach den Status.", m.name));
                                            edit_key = Some(e);
                                            continue;
                                        }
                                        match &m.terminal {
                                            Some(login) => match crate::acp::run_login_console(agent, login, &cwd) {
                                                Ok(()) => {
                                                    state.console_hint.insert(
                                                        agent.name.clone(),
                                                        "Melde dich im geöffneten Fenster an (es öffnet den Browser) und klicke danach auf „Status prüfen“.".into(),
                                                    );
                                                }
                                                Err(e) => {
                                                    state.console_hint.insert(agent.name.clone(), e);
                                                }
                                            },
                                            None => p.authenticate(&m.id),
                                        }
                                    }
                                }
                            });
                            if p.can_logout && p.phase == Phase::Ready && ui.button("Abmelden").clicked() {
                                p.logout();
                            }
                        }
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.button("Entfernen").clicked() {
                                remove = Some(i);
                            }
                            if ui.button("Bearbeiten").clicked() {
                                edit = Some(i);
                            }
                        });
                    });
                });
            ui.add_space(6.0);
        }
        if let Some(i) = remove {
            let agent = self.settings.agents.remove(i);
            let _ = agent.set_api_key("");
            state.probes.remove(&agent.name);
            if self.settings.agent == agent.name {
                self.settings.agent = self.settings.agents.first().map(|a| a.name.clone()).unwrap_or_default();
            }
        }
        if let Some(i) = edit {
            state.edit = Some(Edit::of(Some(i), &self.settings.agents[i]));
        }
        if edit_key.is_some() {
            state.edit = edit_key;
        }

        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.menu_button("Agent hinzufügen ▾", |ui| {
                for (name, args, key) in PRESETS {
                    let present = self.settings.agents.iter().any(|a| a.name == *name);
                    if ui.add_enabled(!present, egui::Button::new(*name)).clicked() {
                        ui.close();
                        self.settings.agents.push(preset(name, args, key));
                    }
                }
                ui.separator();
                if ui.button("Eigener Agent…").clicked() {
                    ui.close();
                    state.edit = Some(Edit::of(None, &AgentConfig::default()));
                }
            });
        });
    }
}

fn status_label(ui: &mut Ui, theme: &crate::theme::Theme, probe: Option<&AgentSession>) {
    let Some(p) = probe else {
        ui.label(RichText::new("nicht geprüft").size(12.0).color(theme.text_faint));
        return;
    };
    let (text, color): (String, Color32) = match &p.phase {
        Phase::Starting if p.is_authenticating() => ("Anmeldung im Browser läuft …".into(), theme.accent),
        Phase::Starting => ("wird geprüft …".into(), theme.text_muted),
        Phase::Ready | Phase::Working => {
            let info = p.agent_info.as_deref().map(|i| format!(" · {i}")).unwrap_or_default();
            (format!("● bereit{info}"), theme.success)
        }
        Phase::NeedsAuth(_) => ("● Anmeldung nötig".into(), theme.modified),
        Phase::Failed(e) => (format!("● {}", e.lines().next().unwrap_or("Fehler")), theme.danger),
        Phase::Exited => ("beendet".into(), theme.text_faint),
        Phase::NeedsNode => ("● Node.js wird benötigt".into(), theme.modified),
    };
    if p.phase == Phase::Starting {
        ui.spinner();
    }
    let resp = ui.label(RichText::new(text).size(12.0).color(color));
    if let Phase::Failed(e) = &p.phase {
        resp.on_hover_text(e);
    }
}

/// The form for adding or changing an agent; `Some(true)`: save, `Some(false)`: cancel.
fn edit_form(ui: &mut Ui, theme: &crate::theme::Theme, e: &mut Edit, agents: &[AgentConfig]) -> Option<bool> {
    let mut result = None;
    ui.label(RichText::new(if e.index.is_some() { "Agent bearbeiten" } else { "Eigener Agent" }).strong());
    ui.add_space(6.0);
    egui::Grid::new("agent-edit").num_columns(2).spacing([12.0, 8.0]).show(ui, |ui| {
        ui.label("Name");
        ui.add(egui::TextEdit::singleline(&mut e.name).desired_width(380.0));
        ui.end_row();
        ui.label("Befehl");
        ui.add(egui::TextEdit::singleline(&mut e.command).hint_text("npx").desired_width(380.0));
        ui.end_row();
        ui.label("Argumente");
        ui.add(egui::TextEdit::singleline(&mut e.args).hint_text("-y paket@latest --acp").desired_width(380.0));
        ui.end_row();
        ui.label("Umgebung");
        ui.add(egui::TextEdit::multiline(&mut e.env).hint_text("NAME=Wert (eine pro Zeile)").desired_rows(2).desired_width(380.0));
        ui.end_row();
        ui.label("API-Schlüssel");
        ui.vertical(|ui| {
            ui.add(egui::TextEdit::singleline(&mut e.key_env).hint_text("Variable, z. B. OPENAI_API_KEY").desired_width(380.0));
            ui.add(
                egui::TextEdit::singleline(&mut e.key)
                    .password(true)
                    .hint_text(if e.has_key { "gespeichert – leer lassen zum Behalten" } else { "optional; sonst Anmeldung mit Konto" })
                    .desired_width(380.0),
            );
            ui.label(RichText::new("Wird in der Windows-Anmeldeinformationsverwaltung gespeichert.").size(11.0).color(theme.text_faint));
        });
        ui.end_row();
    });
    if let Some(err) = &e.error {
        ui.label(RichText::new(err).color(theme.danger));
    }
    ui.add_space(8.0);
    ui.horizontal(|ui| {
        if ui.add(primary_button("Speichern", theme.accent)).clicked() {
            let name = e.name.trim();
            let taken = agents.iter().enumerate().any(|(i, a)| a.name == name && Some(i) != e.index);
            e.error = if name.is_empty() {
                Some("Bitte einen Namen eingeben.".into())
            } else if taken {
                Some("Diesen Namen gibt es schon.".into())
            } else if e.command.trim().is_empty() {
                Some("Bitte einen Befehl eingeben.".into())
            } else {
                None
            };
            if e.error.is_none() {
                result = Some(true);
            }
        }
        if ui.button("Abbrechen").clicked() {
            result = Some(false);
        }
    });
    result
}
