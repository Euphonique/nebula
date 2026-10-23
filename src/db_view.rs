//! Database tab: tables of a server's MySQL/MariaDB on the left, a query
//! field and the result as a grid. Statements that change data are
//! confirmed first.

use std::sync::Mutex;

use eframe::egui::{self, Align2, FontId, Key, Modifiers, Rect, RichText, Sense, pos2, vec2};

use crate::remote::db::{DbEvent, DbSession, Output, quote_ident};
use crate::remote::HostKeyPrompt;
use crate::settings::{DbSettings, Site};
use crate::theme::Theme;

/// Rows shown when a table is clicked.
const TABLE_ROWS: usize = 200;

enum Status {
    Idle,
    Connecting,
    Connected(String),
    Failed(String),
}

struct State {
    site_id: String,
    site_name: String,
    /// Connection form (shown until the first connect, or on request).
    form: DbSettings,
    password: String,
    show_form: bool,
    session: Option<DbSession>,
    status: Status,
    tables: Vec<String>,
    tables_query: Option<u64>,
    sql: String,
    running: Option<u64>,
    /// A statement that changes data, waiting for "Ausführen".
    confirm: Option<String>,
    outputs: Vec<Output>,
    error: Option<String>,
    /// Table filter.
    filter: String,
}

pub struct DbView {
    state: Mutex<State>,
}

/// What the tab asks the app to do.
pub enum Action {
    None,
    /// Store the form's settings (and password) for the server, then connect.
    Save { site_id: String, settings: DbSettings, password: String },
    HostKey { site_id: String, prompt: HostKeyPrompt },
}

impl DbView {
    pub fn new(site: &Site) -> Self {
        let has_settings = !site.db.user.trim().is_empty();
        let state = State {
            site_id: site.id.clone(),
            site_name: site.display_name(),
            form: site.db.clone(),
            password: String::new(),
            show_form: !has_settings,
            session: None,
            status: Status::Idle,
            tables: Vec::new(),
            tables_query: None,
            sql: String::new(),
            running: None,
            confirm: None,
            outputs: Vec::new(),
            error: None,
            filter: String::new(),
        };
        Self { state: Mutex::new(state) }
    }

    pub fn site_id(&self) -> String {
        self.state.lock().unwrap().site_id.clone()
    }

    /// Connects with the server's saved settings.
    pub fn connect(&self, site: &Site, ctx: &egui::Context) {
        let mut s = self.state.lock().unwrap();
        let password = site.db_password().unwrap_or_default();
        s.form = site.db.clone();
        s.session = Some(crate::remote::db::connect(site.clone(), password, ctx.clone()));
        s.status = Status::Connecting;
        s.show_form = false;
        s.error = None;
    }

    fn handle_events(s: &mut State) -> Action {
        let Some(session) = s.session.as_mut() else { return Action::None };
        let events: Vec<DbEvent> = session.events.try_iter().collect();
        let mut action = Action::None;
        for ev in events {
            match ev {
                DbEvent::HostKey(prompt) => action = Action::HostKey { site_id: s.site_id.clone(), prompt },
                DbEvent::Connected(version) => {
                    s.status = Status::Connected(version);
                    let session = s.session.as_mut().unwrap();
                    s.tables_query = Some(session.query("SHOW TABLES"));
                }
                DbEvent::Done(id, outputs) if Some(id) == s.tables_query => {
                    s.tables_query = None;
                    s.tables = outputs.first().map(|o| o.rows.iter().filter_map(|r| r.first().cloned().flatten()).collect()).unwrap_or_default();
                }
                DbEvent::Failed(id, e) if Some(id) == s.tables_query => {
                    s.tables_query = None;
                    s.error = Some(e);
                }
                DbEvent::Done(id, outputs) => {
                    if Some(id) == s.running {
                        s.running = None;
                        s.outputs = outputs;
                        s.error = None;
                    }
                }
                DbEvent::Failed(id, e) => {
                    if Some(id) == s.running {
                        s.running = None;
                        s.outputs.clear();
                        s.error = Some(e);
                    }
                }
                DbEvent::Closed(err) => {
                    s.session = None;
                    s.running = None;
                    s.status = match err {
                        Some(e) => Status::Failed(e),
                        None => Status::Idle,
                    };
                }
            }
        }
        action
    }

    fn run(s: &mut State, sql: String, confirmed: bool) {
        let sql = sql.trim().to_string();
        if sql.is_empty() || s.running.is_some() {
            return;
        }
        if !confirmed && !crate::remote::db::is_read_only(&sql) {
            s.confirm = Some(sql);
            return;
        }
        s.confirm = None;
        if let Some(session) = s.session.as_mut() {
            s.running = Some(session.query(&sql));
        }
    }

    /// Draws the tab.
    pub fn show(&self, ui: &mut egui::Ui, rect: Rect, theme: &Theme) -> Action {
        let mut s = self.state.lock().unwrap();
        let mut action = Self::handle_events(&mut s);
        if s.status_is_busy() {
            ui.ctx().request_repaint_after(std::time::Duration::from_millis(100));
        }
        ui.painter().rect_filled(rect, 0.0, theme.background);
        let inner = rect.shrink(12.0);
        ui.scope_builder(egui::UiBuilder::new().max_rect(inner), |ui| {
            // Header: server, state, buttons.
            ui.horizontal(|ui| {
                ui.label(RichText::new(t!("Datenbank · {name}", name = s.site_name)).strong());
                let (text, color) = match &s.status {
                    Status::Idle => (t!("nicht verbunden").to_string(), theme.text_faint),
                    Status::Connecting => (t!("verbinde …").to_string(), theme.text_faint),
                    Status::Connected(v) => (t!("verbunden ({version})", version = v), theme.success),
                    Status::Failed(e) => (e.clone(), theme.danger),
                };
                ui.label(RichText::new(text).small().color(color));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button(t!("Verbindung…")).clicked() {
                        s.show_form = !s.show_form;
                    }
                    let connected = s.session.is_some();
                    if connected {
                        if ui.button(t!("Trennen")).clicked() {
                            s.session = None;
                            s.status = Status::Idle;
                        }
                    } else if !s.show_form && ui.button(t!("Verbinden")).clicked() {
                        action = Action::Save { site_id: s.site_id.clone(), settings: s.form.clone(), password: String::new() };
                    }
                });
            });
            ui.add_space(6.0);
            if s.show_form {
                Self::form(ui, &mut s, theme, &mut action);
                return;
            }
            let body = ui.available_rect_before_wrap();
            let list = Rect::from_min_size(body.min, vec2(220.0, body.height()));
            let main = Rect::from_min_max(pos2(list.right() + 12.0, body.top()), body.max);
            ui.scope_builder(egui::UiBuilder::new().max_rect(list), |ui| Self::table_list(ui, &mut s, theme));
            ui.scope_builder(egui::UiBuilder::new().max_rect(main), |ui| Self::query_area(ui, &mut s, theme));
        });
        action
    }

    fn form(ui: &mut egui::Ui, s: &mut State, theme: &Theme, action: &mut Action) {
        ui.label(
            RichText::new(t!("Die Verbindung läuft über die SSH-Anmeldung des Servers (Tunnel); Host und Port gelten vom Server aus gesehen."))
                .small()
                .color(theme.text_faint),
        );
        ui.add_space(6.0);
        egui::Grid::new("db-form").num_columns(2).spacing([12.0, 8.0]).show(ui, |ui| {
            ui.label(t!("Datenbank"));
            ui.add(egui::TextEdit::singleline(&mut s.form.database).desired_width(260.0).hint_text("wordpress"));
            ui.end_row();
            ui.label(t!("Benutzer"));
            ui.add(egui::TextEdit::singleline(&mut s.form.user).desired_width(260.0));
            ui.end_row();
            ui.label(t!("Passwort"));
            ui.add(egui::TextEdit::singleline(&mut s.password).password(true).desired_width(260.0).hint_text(t!("unverändert lassen")));
            ui.end_row();
            ui.label(t!("Host"));
            ui.horizontal(|ui| {
                ui.add(egui::TextEdit::singleline(&mut s.form.host).desired_width(180.0));
                ui.label(t!("Port"));
                ui.add(egui::DragValue::new(&mut s.form.port).range(1..=65535));
            });
            ui.end_row();
        });
        ui.add_space(8.0);
        let ready = !s.form.user.trim().is_empty() && !s.form.host.trim().is_empty();
        if ui.add_enabled(ready, egui::Button::new(t!("Speichern und verbinden"))).clicked() {
            *action = Action::Save { site_id: s.site_id.clone(), settings: s.form.clone(), password: std::mem::take(&mut s.password) };
        }
        ui.label(RichText::new(t!("Das Passwort wird in der Windows-Anmeldeinformationsverwaltung gespeichert.")).small().color(theme.text_faint));
    }

    fn table_list(ui: &mut egui::Ui, s: &mut State, theme: &Theme) {
        ui.add(egui::TextEdit::singleline(&mut s.filter).hint_text(t!("Tabellen filtern")).desired_width(f32::INFINITY));
        ui.add_space(4.0);
        if s.tables_query.is_some() {
            ui.label(RichText::new(t!("Lade Tabellen …")).small().color(theme.text_faint));
        }
        let filter = s.filter.to_lowercase();
        let mut open = None;
        egui::ScrollArea::vertical().id_salt("db-tables").auto_shrink([false, false]).show(ui, |ui| {
            for t in s.tables.iter().filter(|t| t.to_lowercase().contains(&filter)) {
                if ui.add(egui::Button::selectable(false, t.as_str()).min_size(vec2(ui.available_width(), 22.0))).clicked() {
                    open = Some(t.clone());
                }
            }
        });
        if let Some(t) = open {
            s.sql = format!("SELECT * FROM {} LIMIT {TABLE_ROWS};", quote_ident(&t));
            let sql = s.sql.clone();
            Self::run(s, sql, false);
        }
    }

    fn query_area(ui: &mut egui::Ui, s: &mut State, theme: &Theme) {
        let editor = ui.add(
            egui::TextEdit::multiline(&mut s.sql)
                .code_editor()
                .desired_rows(5)
                .desired_width(f32::INFINITY)
                .hint_text(t!("SQL, z. B. SELECT * FROM wp_options LIMIT 20")),
        );
        let ctrl_enter = editor.has_focus() && ui.input_mut(|i| i.consume_key(Modifiers::COMMAND, Key::Enter));
        ui.horizontal(|ui| {
            let can = s.session.is_some() && s.running.is_none();
            if ui.add_enabled(can, egui::Button::new(t!("Ausführen (Strg+Enter)"))).clicked() || (ctrl_enter && can) {
                let sql = s.sql.clone();
                Self::run(s, sql, false);
            }
            if s.running.is_some() {
                ui.spinner();
            }
            if let Some(o) = s.outputs.first().filter(|o| !o.columns.is_empty())
                && ui.button(t!("Als CSV kopieren")).clicked()
            {
                ui.ctx().copy_text(to_csv(o));
            }
        });
        if let Some(sql) = s.confirm.clone() {
            egui::Frame::new().fill(theme.danger.gamma_multiply(0.12)).inner_margin(8.0).corner_radius(6.0).show(ui, |ui| {
                ui.label(RichText::new(t!("Diese Abfrage ändert die Datenbank. Wirklich ausführen?")).strong());
                ui.horizontal(|ui| {
                    if ui.button(t!("Ausführen")).clicked() {
                        Self::run(s, sql.clone(), true);
                    }
                    if ui.button(t!("Abbrechen")).clicked() {
                        s.confirm = None;
                    }
                });
            });
        }
        ui.add_space(6.0);
        if let Some(e) = &s.error {
            ui.label(RichText::new(e).color(theme.danger));
            return;
        }
        for (n, o) in s.outputs.iter().enumerate() {
            let summary = if o.columns.is_empty() {
                t!("{n} Zeilen betroffen · {ms} ms", n = o.affected, ms = o.millis)
            } else if o.total > o.rows.len() {
                t!("{shown} von {total} Zeilen · {ms} ms", shown = o.rows.len(), total = o.total, ms = o.millis)
            } else {
                t!("{n} Zeilen · {ms} ms", n = o.total, ms = o.millis)
            };
            ui.label(RichText::new(summary).small().color(theme.text_faint));
            if !o.columns.is_empty() {
                grid(ui, o, theme, n);
            }
        }
    }
}

impl State {
    fn status_is_busy(&self) -> bool {
        self.running.is_some() || self.tables_query.is_some() || matches!(self.status, Status::Connecting)
    }
}

/// The result as a grid: column widths from the content, rows drawn as needed.
fn grid(ui: &mut egui::Ui, o: &Output, theme: &Theme, salt: usize) {
    let font = FontId::monospace(12.5);
    let char_w = ui.fonts_mut(|f| f.glyph_width(&font, 'M'));
    let row_h = 22.0;
    let widths: Vec<f32> = (0..o.columns.len())
        .map(|c| {
            let longest = o.rows.iter().take(200).map(|r| r.get(c).and_then(|v| v.as_ref()).map_or(4, |v| v.chars().count())).max().unwrap_or(0);
            (longest.max(o.columns[c].chars().count()).clamp(3, 40) as f32) * char_w + 16.0
        })
        .collect();
    let total_w: f32 = widths.iter().sum();
    egui::ScrollArea::both().id_salt(("db-grid", salt)).auto_shrink([false, false]).show_rows(ui, row_h, o.rows.len() + 1, |ui, range| {
        for r in range {
            let (rect, resp) = ui.allocate_exact_size(vec2(total_w.max(ui.available_width()), row_h), Sense::click());
            let painter = ui.painter_at(rect);
            let header = r == 0;
            if header {
                painter.rect_filled(rect, 0.0, theme.jumpbar_bg);
            } else if r % 2 == 0 {
                painter.rect_filled(rect, 0.0, theme.hover.gamma_multiply(0.5));
            }
            let mut x = rect.left();
            for (c, w) in widths.iter().enumerate() {
                let cell = Rect::from_min_size(pos2(x, rect.top()), vec2(*w, row_h));
                let (text, color) = if header {
                    (o.columns[c].clone(), theme.foreground)
                } else {
                    match o.rows[r - 1].get(c).cloned().flatten() {
                        Some(v) => (v.replace(['\n', '\r', '\t'], " "), theme.foreground),
                        None => ("NULL".to_string(), theme.text_faint),
                    }
                };
                painter.with_clip_rect(cell.shrink2(vec2(6.0, 0.0))).text(pos2(cell.left() + 8.0, cell.center().y), Align2::LEFT_CENTER, text, font.clone(), color);
                painter.line_segment([cell.right_top(), cell.right_bottom()], egui::Stroke::new(1.0, theme.separator.gamma_multiply(0.5)));
                x += w;
            }
            if !header {
                resp.context_menu(|ui| {
                    if ui.button(t!("Zeile kopieren")).clicked() {
                        let row: Vec<String> = o.rows[r - 1].iter().map(|v| v.clone().unwrap_or_else(|| "NULL".into())).collect();
                        ui.ctx().copy_text(row.join("\t"));
                        ui.close();
                    }
                });
            }
        }
    });
}

/// The result as CSV (semicolons, as Excel in German expects).
fn to_csv(o: &Output) -> String {
    let field = |v: &str| if v.contains([';', '"', '\n']) { format!("\"{}\"", v.replace('"', "\"\"")) } else { v.to_string() };
    let mut out = o.columns.iter().map(|c| field(c)).collect::<Vec<_>>().join(";");
    for r in &o.rows {
        out.push('\n');
        out.push_str(&r.iter().map(|v| v.as_deref().map(field).unwrap_or_default()).collect::<Vec<_>>().join(";"));
    }
    out
}
