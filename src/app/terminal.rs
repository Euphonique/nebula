//! Terminals in a panel below the editor, one tab per session: local
//! shells (PowerShell, Git Bash, …) and SSH sessions on servers.

use eframe::egui::{self, Color32, CursorIcon, Event, FontId, Id, Key, Pos2, Rect, Sense, Stroke, pos2, vec2};

use super::NebulaApp;
use crate::remote::terminal::{LocalShell, TermStatus, Terminal, key_bytes, local_shells};
use crate::ui::icons;

const PADDING: f32 = 8.0;
/// As high as the tab rows of the side panels.
const HEADER_H: f32 = 40.0;

/// A terminal with what the panel remembers about its view.
pub(super) struct TermTab {
    term: Terminal,
    title: String,
    /// Lines scrolled back from the live screen.
    scroll: usize,
    /// Selection in screen cells: (anchor, head) as (row, col).
    selection: Option<((u16, u16), (u16, u16))>,
    /// Changed into the site's start folder after connecting.
    entered_root: bool,
    /// Local shell and folder it started in (`None`: SSH, see `term.site_id`).
    local: Option<(LocalShell, Option<std::path::PathBuf>)>,
    /// Take the keyboard focus when drawn next (a closing menu would steal it now).
    want_focus: bool,
    /// Saved commands to run once the session is in the start folder.
    queued: Vec<String>,
    /// Search in the scrollback (Ctrl+F), while open.
    search: Option<TermSearch>,
}

/// Search in a terminal's history and screen.
#[derive(Default)]
struct TermSearch {
    query: String,
    focus: bool,
    /// Matches as (line, start column, end column); line 0 is the oldest kept line.
    matches: Vec<(usize, usize, usize)>,
    current: Option<usize>,
    /// What the matches were computed for, and when (new output shifts lines).
    computed: Option<(String, std::time::Instant)>,
}

/// Case-insensitive matches of `query` in `lines` (columns in chars).
fn find_in_lines(lines: &[String], query: &str) -> Vec<(usize, usize, usize)> {
    let fold = |c: char| c.to_lowercase().next().unwrap_or(c);
    let q: Vec<char> = query.chars().map(fold).collect();
    if q.is_empty() {
        return Vec::new();
    }
    let mut out = Vec::new();
    for (n, line) in lines.iter().enumerate() {
        let l: Vec<char> = line.chars().map(fold).collect();
        let mut i = 0;
        while i + q.len() <= l.len() {
            if l[i..i + q.len()] == q[..] {
                out.push((n, i, i + q.len()));
                i += q.len();
            } else {
                i += 1;
            }
        }
    }
    out
}


/// Lines in the history right now (vt100 only tells its capacity).
fn history_len(screen: &mut vt100::Screen) -> usize {
    let keep = screen.scrollback();
    screen.set_scrollback(usize::MAX);
    let n = screen.scrollback();
    screen.set_scrollback(keep);
    n
}
/// All lines of the history and the screen, oldest first, and how many
/// belong to the history. Leaves the view where it was.
fn all_lines(parser: &mut vt100::Parser, rows: u16, cols: u16) -> (Vec<String>, usize) {
    let screen = parser.screen_mut();
    let keep = screen.scrollback();
    let hist = history_len(screen);
    let total = hist + rows as usize;
    let mut lines = Vec::with_capacity(total);
    while lines.len() < total {
        // At scrollback s the view starts at line hist - s.
        let start = lines.len();
        let s = hist.saturating_sub(start);
        screen.set_scrollback(s);
        let skip = start - (hist - s);
        let before = lines.len();
        lines.extend(screen.rows(0, cols).skip(skip).take(total - start));
        if lines.len() == before {
            break;
        }
    }
    screen.set_scrollback(keep);
    (lines, hist)
}

#[derive(Default)]
pub(super) struct Terminals {
    tabs: Vec<TermTab>,
    active: usize,
    pub visible: bool,
    /// Form for a new saved command in the "Befehle" menu.
    new_name: String,
    new_command: String,
}

impl Terminals {
    pub fn is_empty(&self) -> bool {
        self.tabs.is_empty()
    }
}

fn term_id(id: u64) -> Id {
    Id::unique(("terminal", id))
}

impl NebulaApp {
    /// Opens a new SSH terminal for a server and shows the panel.
    pub(super) fn open_terminal(&mut self, ctx: &egui::Context, site_id: &str) {
        let Some(mut site) = self.sites.get(site_id).cloned() else { return };
        // FTP servers usually offer SSH on the standard port.
        if site.protocol != crate::settings::Protocol::Sftp {
            site.port = 22;
        }
        let name = site.display_name();
        let same = self.terminals.tabs.iter().filter(|t| t.term.site_id == site.id).count();
        let title = if same == 0 { name } else { format!("{name} ({})", same + 1) };
        let mut term = Terminal::open(&site, ctx, 24, 80);
        term.site_id = site.id.clone();
        self.terminals.tabs.push(TermTab { term, title, scroll: 0, selection: None, entered_root: false, local: None, want_focus: true, queued: Vec::new(), search: None });
        self.terminals.active = self.terminals.tabs.len() - 1;
        self.terminals.visible = true;
        let id = self.terminals.tabs[self.terminals.active].term.id;
        ctx.memory_mut(|m| m.request_focus(term_id(id)));
    }

    /// Runs a saved command in an SSH terminal of the server: the active one
    /// if it belongs to the server, else another one, else a new one.
    pub(super) fn run_server_command(&mut self, ctx: &egui::Context, site_id: &str, command: &str) {
        let is_site = |t: &TermTab| t.local.is_none() && t.term.site_id == site_id && !matches!(t.term.status, TermStatus::Closed(_));
        let index = match self.terminals.tabs.get(self.terminals.active) {
            Some(t) if is_site(t) => Some(self.terminals.active),
            _ => self.terminals.tabs.iter().position(is_site),
        };
        let index = match index {
            Some(i) => i,
            None => {
                self.open_terminal(ctx, site_id);
                self.terminals.tabs.len() - 1
            }
        };
        let tab = &mut self.terminals.tabs[index];
        tab.queued.push(command.to_string());
        // Shown at once if the session is ready; otherwise after connecting.
        tab.scroll = 0;
        self.terminals.active = index;
        self.terminals.visible = true;
        self.poll_terminals();
        let id = self.terminals.tabs[index].term.id;
        ctx.memory_mut(|m| m.request_focus(term_id(id)));
    }

    /// The shell chosen in the settings, else the first one found.
    fn default_shell(&self) -> LocalShell {
        let shells = local_shells();
        shells.iter().find(|s| s.name == self.settings.terminal_shell).cloned().unwrap_or_else(|| shells[0].clone())
    }

    /// Opens a local shell in the project folder (or the active file's folder).
    pub(super) fn open_local_terminal(&mut self, ctx: &egui::Context, shell: Option<LocalShell>, cwd: Option<std::path::PathBuf>) {
        let shell = shell.unwrap_or_else(|| self.default_shell());
        let cwd = cwd.or_else(|| self.sidebar.root.clone()).or_else(|| {
            match self.active.and_then(|i| self.docs.get(i)).map(|d| &d.source) {
                Some(crate::editor::DocSource::Local(p)) => p.parent().map(|p| p.to_path_buf()),
                _ => None,
            }
        });
        let same = self.terminals.tabs.iter().filter(|t| t.local.as_ref().is_some_and(|(s, _)| s.name == shell.name)).count();
        let title = if same == 0 { shell.name.clone() } else { format!("{} ({})", shell.name, same + 1) };
        let term = Terminal::open_local(&shell, cwd.as_deref(), ctx, 24, 80);
        self.terminals.tabs.push(TermTab { term, title, scroll: 0, selection: None, entered_root: true, local: Some((shell, cwd)), want_focus: true, queued: Vec::new(), search: None });
        self.terminals.active = self.terminals.tabs.len() - 1;
        self.terminals.visible = true;
        let id = self.terminals.tabs[self.terminals.active].term.id;
        ctx.memory_mut(|m| m.request_focus(term_id(id)));
    }

    /// Shows or hides the panel; with no terminal yet, opens an SSH terminal
    /// for a server file, else a local shell.
    /// Zen mode: only the editor, full screen; Esc or the shortcut leaves it.
    pub(super) fn toggle_zen(&mut self, ctx: &egui::Context) {
        self.zen = !self.zen;
        ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(self.zen));
        if self.zen {
            self.info(t!("Zen-Modus – Esc zum Beenden"));
        }
        self.focus_editor = true;
    }

    pub(super) fn toggle_terminal(&mut self, ctx: &egui::Context) {
        if !self.terminals.is_empty() {
            self.terminals.visible = !self.terminals.visible;
            if self.terminals.visible {
                let id = self.terminals.tabs[self.terminals.active].term.id;
                ctx.memory_mut(|m| m.request_focus(term_id(id)));
            } else {
                self.focus_editor = true;
            }
            return;
        }
        match self.active.and_then(|i| self.docs.get(i)).map(|d| &d.source) {
            Some(crate::editor::DocSource::Remote { site_id, .. }) => {
                let id = site_id.clone();
                self.open_terminal(ctx, &id);
            }
            _ => self.open_local_terminal(ctx, None, None),
        }
    }

    /// True while a terminal has keyboard focus (app shortcuts stay out of its way).
    pub(super) fn terminal_focused(&self, ctx: &egui::Context) -> bool {
        self.terminals.visible
            && self.terminals.tabs.get(self.terminals.active).is_some_and(|t| {
                // The search field counts too: Ctrl+F there must not open the editor's search.
                ctx.memory(|m| m.has_focus(term_id(t.term.id)) || m.has_focus(term_id(t.term.id).with("search")))
            })
    }

    /// Connection events of all terminals.
    pub(super) fn poll_terminals(&mut self) {
        for tab in &mut self.terminals.tabs {
            for prompt in tab.term.poll() {
                self.host_keys.push_back((tab.term.site_id.clone(), prompt));
            }
            if tab.term.status == TermStatus::Connected && !tab.entered_root {
                tab.entered_root = true;
                if let Some(site) = self.sites.get(&tab.term.site_id) {
                    let root = site.root_path.trim();
                    if !root.is_empty() {
                        tab.term.send(format!("cd '{}'\r", root.replace('\'', "'\\''")).into_bytes());
                    }
                }
            }
            // Saved commands wait until the session is in its start folder.
            if tab.entered_root {
                for cmd in tab.queued.drain(..) {
                    tab.term.send(format!("{cmd}\r").into_bytes());
                }
            }
        }
    }

    /// The terminal panel at the bottom of the editor area.
    pub(super) fn terminal_panel(&mut self, ui: &mut egui::Ui) {
        if !self.terminals.visible || self.terminals.is_empty() {
            return;
        }
        let theme = self.theme.clone();
        let resp = egui::Panel::bottom("terminal-panel")
            .resizable(true)
            .default_size(self.settings.terminal_height)
            .size_range(120.0..=900.0)
            .show_separator_line(false)
            .frame(egui::Frame::new().fill(theme.sidebar_bg))
            .show(ui, |ui| {
                let full = ui.max_rect();
                ui.painter().line_segment([full.left_top(), full.right_top()], Stroke::new(1.0, theme.separator));
                self.terminal_header(ui, Rect::from_min_size(full.min, vec2(full.width(), HEADER_H)));
                let body = Rect::from_min_max(pos2(full.left(), full.top() + HEADER_H), full.max);
                if let Some(tab) = self.terminals.tabs.get_mut(self.terminals.active) {
                    let font = FontId::monospace((self.settings.editor.font_size - 1.0).max(9.0));
                    let reconnect = terminal_view(ui, tab, body, &theme, font);
                    if reconnect {
                        let site = tab.term.site_id.clone();
                        let local = tab.local.clone();
                        let i = self.terminals.active;
                        self.terminals.tabs.remove(i);
                        match local {
                            Some((shell, cwd)) => self.open_local_terminal(ui.ctx(), Some(shell), cwd),
                            None => self.open_terminal(ui.ctx(), &site),
                        }
                        // Keep the tab where it was.
                        let tab = self.terminals.tabs.pop().unwrap();
                        self.terminals.tabs.insert(i, tab);
                        self.terminals.active = i;
                    }
                }
                ui.advance_cursor_after_rect(full);
            });
        self.settings.terminal_height = resp.response.rect.height();
    }

    fn terminal_header(&mut self, ui: &mut egui::Ui, rect: Rect) {
        let theme = self.theme.clone();
        // The tab row sits like the button rows of the side panels: 10 pt in, 26 pt high.
        let row = Rect::from_min_size(rect.min + vec2(10.0, 10.0), vec2(rect.width() - 20.0, 26.0));
        let mut child = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(row)
                .layout(egui::Layout::left_to_right(egui::Align::Center)),
        );
        let ui = &mut child;
        ui.spacing_mut().item_spacing.x = 4.0;
        let mut activate = None;
        let mut close = None;
        for (i, tab) in self.terminals.tabs.iter().enumerate() {
            let active = i == self.terminals.active;
            let font = FontId::proportional(12.5);
            let w = ui.fonts_mut(|f| f.layout_no_wrap(tab.title.clone(), font.clone(), theme.foreground).size().x) + 50.0;
            let (r, resp) = ui.allocate_exact_size(vec2(w, 26.0), Sense::click());
            // Highlighted like the selected icon buttons of the side panels.
            if active {
                ui.painter().rect_filled(r, 6.0, theme.hover.gamma_multiply(2.2));
            } else if resp.hovered() {
                ui.painter().rect_filled(r, 6.0, theme.hover);
            }
            let dot = match &tab.term.status {
                TermStatus::Connecting => theme.modified,
                TermStatus::Connected => theme.success,
                TermStatus::Closed(_) => theme.text_faint,
            };
            ui.painter().circle_filled(pos2(r.left() + 11.0, r.center().y), 3.5, dot);
            let color = if active { theme.foreground } else { theme.text_muted };
            ui.painter().text(pos2(r.left() + 21.0, r.center().y), egui::Align2::LEFT_CENTER, &tab.title, font, color);
            let x = Rect::from_center_size(pos2(r.right() - 12.0, r.center().y), vec2(16.0, 16.0));
            let xr = ui.interact(x, Id::unique(("term-close", tab.term.id)), Sense::click());
            if xr.hovered() {
                ui.painter().rect_filled(x, 4.0, theme.hover.gamma_multiply(2.0));
            }
            if resp.hovered() || active || xr.hovered() {
                icons::close(ui.painter(), x.center(), 6.0, color);
            }
            if xr.clicked() || resp.middle_clicked() {
                close = Some(i);
            } else if resp.clicked() {
                activate = Some(i);
            }
        }
        // New terminal: a local shell or an SSH session on a server.
        let plus = icons::icon_button(ui, &theme, vec2(26.0, 26.0), false, |p, c, col| icons::plus(p, c, col)).on_hover_text(t!("Neues Terminal"));
        let sites: Vec<(String, String)> = self.sites.sites.iter().map(|s| (s.id.clone(), s.display_name())).collect();
        let mut open = None;
        let mut open_local = None;
        egui::Popup::menu(&plus).show(|ui| {
            ui.set_min_width(200.0);
            ui.label(egui::RichText::new(t!("Lokal")).small().color(theme.text_faint));
            for shell in local_shells() {
                if ui.button(&shell.name).clicked() {
                    open_local = Some(shell);
                    ui.close();
                }
            }
            if !sites.is_empty() {
                ui.separator();
                ui.label(egui::RichText::new("SSH").small().color(theme.text_faint));
            }
            for (id, name) in &sites {
                if ui.button(name).clicked() {
                    open = Some(id.clone());
                    ui.close();
                }
            }
        });
        // Saved commands of the server behind the active SSH terminal.
        let site_id = self.terminals.tabs.get(self.terminals.active).filter(|t| t.local.is_none()).map(|t| t.term.site_id.clone());
        let mut run: Option<String> = None;
        let mut sites_changed = false;
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if icons::icon_button(ui, &theme, vec2(26.0, 26.0), false, |p, c, col| icons::close(p, c, 7.0, col))
                .on_hover_text(t!("Terminal ausblenden (Strg+Ö)"))
                .clicked()
            {
                self.terminals.visible = false;
                self.focus_editor = true;
            }
            let Some(site) = site_id.as_ref().and_then(|id| self.sites.sites.iter_mut().find(|s| &s.id == id)) else { return };
            let menu = ui.add(egui::Button::new(egui::RichText::new(t!("Befehle ▾")).size(12.0)).frame(false)).on_hover_text(t!("Gespeicherte Befehle für diesen Server"));
            let form = (&mut self.terminals.new_name, &mut self.terminals.new_command);
            egui::Popup::menu(&menu).close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside).show(|ui| {
                ui.set_width(320.0);
                if site.commands.is_empty() {
                    ui.label(egui::RichText::new(t!("Noch keine Befehle gespeichert.")).small().color(theme.text_faint));
                }
                let mut remove = None;
                for (n, c) in site.commands.iter().enumerate() {
                    ui.horizontal(|ui| {
                        let w = ui.available_width() - 28.0;
                        if ui.add(egui::Button::new(&c.name).min_size(vec2(w, 24.0))).on_hover_text(&c.command).clicked() {
                            run = Some(c.command.clone());
                            ui.close();
                        }
                        if ui.small_button("✕").on_hover_text(t!("Befehl löschen")).clicked() {
                            remove = Some(n);
                        }
                    });
                }
                if let Some(n) = remove {
                    site.commands.remove(n);
                    sites_changed = true;
                }
                ui.separator();
                ui.label(egui::RichText::new(t!("Neuer Befehl")).small().color(theme.text_faint));
                ui.add(egui::TextEdit::singleline(form.0).hint_text(t!("Name, z. B. Cache leeren")).desired_width(f32::INFINITY));
                ui.add(egui::TextEdit::singleline(form.1).hint_text(t!("Befehl, z. B. php artisan cache:clear")).desired_width(f32::INFINITY).font(egui::TextStyle::Monospace));
                let (name, command) = (form.0.trim().to_string(), form.1.trim().to_string());
                if ui.add_enabled(!command.is_empty(), egui::Button::new(t!("Speichern"))).clicked() {
                    let name = if name.is_empty() { command.clone() } else { name };
                    site.commands.push(crate::settings::ServerCommand { name, command });
                    form.0.clear();
                    form.1.clear();
                    sites_changed = true;
                }
            });
        });
        if sites_changed && let Err(e) = self.sites.save() {
            self.error(t!("Server konnten nicht gespeichert werden: {e}", e = e));
        }
        if let (Some(site_id), Some(command)) = (site_id, run) {
            self.run_server_command(&ui.ctx().clone(), &site_id, &command);
        }
        if let Some(i) = activate {
            self.terminals.active = i;
            let id = self.terminals.tabs[i].term.id;
            ui.ctx().memory_mut(|m| m.request_focus(term_id(id)));
        }
        if let Some(i) = close {
            // Dropping the terminal ends its session.
            self.terminals.tabs.remove(i);
            if self.terminals.tabs.is_empty() {
                self.terminals.visible = false;
                self.focus_editor = true;
            }
            self.terminals.active = self.terminals.active.min(self.terminals.tabs.len().saturating_sub(1));
        }
        if let Some(id) = open {
            self.open_terminal(&ui.ctx().clone(), &id);
        }
        if let Some(shell) = open_local {
            self.open_local_terminal(&ui.ctx().clone(), Some(shell), None);
        }
    }
}

/// Draws a terminal and handles its input. Returns true when the user asks
/// to reconnect a closed session.
fn terminal_view(ui: &mut egui::Ui, tab: &mut TermTab, body: Rect, theme: &crate::theme::Theme, font: FontId) -> bool {
    let id = term_id(tab.term.id);
    let (row_h, char_w) = ui.fonts_mut(|f| ((f.row_height(&font) * 1.18).round(), f.glyph_width(&font, 'M')));
    let inner = body.shrink(PADDING);
    let rows = ((inner.height() / row_h).floor() as u16).max(2);
    let cols = ((inner.width() / char_w).floor() as u16).max(10);
    tab.term.resize(rows, cols);

    let resp = ui.interact(body, id, Sense::click_and_drag());
    if resp.hovered() {
        ui.ctx().set_cursor_icon(CursorIcon::Text);
    }
    if resp.clicked() || resp.drag_started() || std::mem::take(&mut tab.want_focus) {
        resp.request_focus();
    }
    let focused = resp.has_focus();
    let closed = matches!(tab.term.status, TermStatus::Closed(_));
    let cell_at = |p: Pos2| -> (u16, u16) {
        let r = ((p.y - inner.top()) / row_h).floor().clamp(0.0, (rows - 1) as f32) as u16;
        let c = ((p.x - inner.left()) / char_w).round().clamp(0.0, cols as f32) as u16;
        (r, c)
    };

    // Mouse: selection and scrolling back.
    if resp.drag_started()
        && let Some(p) = resp.interact_pointer_pos()
    {
        let c = cell_at(p);
        tab.selection = Some((c, c));
    } else if resp.dragged()
        && let (Some(p), Some((a, _))) = (resp.interact_pointer_pos(), tab.selection)
    {
        tab.selection = Some((a, cell_at(p)));
    } else if resp.clicked() {
        tab.selection = None;
    }
    let wheel = if resp.hovered() { ui.input(|i| i.smooth_scroll_delta.y) } else { 0.0 };
    if wheel != 0.0 {
        let lines = (wheel / row_h).round() as isize;
        tab.scroll = (tab.scroll as isize + lines).max(0) as usize;
    }

    // Take what is visible out of the emulator; the connection keeps
    // writing into it from another thread.
    let sel = tab.selection.filter(|(a, b)| a != b).map(|(a, b)| if a <= b { (a, b) } else { (b, a) });
    let in_sel = |r: u16, c: u16| sel.is_some_and(|(s, e)| (r, c) >= s && (r, c) < e);
    let mut parser = tab.term.parser.lock().unwrap();
    // Search: matches over history and screen, refreshed as output arrives.
    let hist = history_len(parser.screen_mut());
    if let Some(se) = &mut tab.search {
        let changed = se.computed.as_ref().is_none_or(|(q, _)| *q != se.query);
        if changed || se.computed.as_ref().is_some_and(|(_, t)| t.elapsed() > std::time::Duration::from_millis(300)) {
            let (lines, _) = all_lines(&mut parser, rows, cols);
            se.matches = find_in_lines(&lines, &se.query);
            se.computed = Some((se.query.clone(), std::time::Instant::now()));
            if changed {
                // The newest match first, like searching upwards from the prompt.
                se.current = se.matches.len().checked_sub(1);
                if let Some(&(line, ..)) = se.current.and_then(|c| se.matches.get(c)) {
                    tab.scroll = scroll_to_line(line, hist, rows);
                }
            } else {
                se.current = se.current.map(|c| c.min(se.matches.len().saturating_sub(1))).filter(|_| !se.matches.is_empty());
            }
        }
        if !se.query.is_empty() {
            ui.ctx().request_repaint_after(std::time::Duration::from_millis(300));
        }
    }
    parser.screen_mut().set_scrollback(tab.scroll);
    // Clamped to the available history.
    tab.scroll = parser.screen().scrollback();
    let screen = parser.screen();
    let selected_text = sel.map(|(s, e)| screen.contents_between(s.0, s.1, e.0, e.1));
    let modes = Modes {
        app_cursor: screen.application_cursor(),
        bracketed_paste: screen.bracketed_paste(),
        hide_cursor: screen.hide_cursor(),
        cursor: screen.cursor_position(),
    };
    let mut runs: Vec<Run> = Vec::new();
    for r in 0..rows {
        let mut c = 0;
        while c < cols {
            let Some(cell) = screen.cell(r, c) else { break };
            let look = Look::of(cell, in_sel(r, c));
            let start = c;
            let mut text = String::new();
            while c < cols {
                let Some(cell) = screen.cell(r, c) else { break };
                if Look::of(cell, in_sel(r, c)) != look {
                    break;
                }
                if !cell.is_wide_continuation() {
                    text.push_str(if cell.has_contents() { cell.contents() } else { " " });
                }
                c += 1;
            }
            runs.push(Run { row: r, start, end: c, text, look });
        }
    }
    drop(parser);

    // Keyboard.
    let mut reconnect = false;
    let mut copy = None;
    let mut open_search = false;
    if focused {
        ui.memory_mut(|m| {
            m.set_focus_lock_filter(id, egui::EventFilter { tab: true, horizontal_arrows: true, vertical_arrows: true, escape: true })
        });
        let app_cursor = modes.app_cursor;
        let mut out: Vec<u8> = Vec::new();
        for ev in ui.input(|i| i.events.clone()) {
            match ev {
                Event::Text(t) => out.extend(t.as_bytes()),
                Event::Copy => match &selected_text {
                    Some(t) => copy = Some(t.clone()),
                    None => out.push(0x03),
                },
                Event::Cut => out.push(0x18),
                Event::Paste(t) => {
                    let t = t.replace("\r\n", "\n").replace('\n', "\r");
                    if modes.bracketed_paste {
                        out.extend(b"\x1b[200~");
                        out.extend(t.as_bytes());
                        out.extend(b"\x1b[201~");
                    } else {
                        out.extend(t.as_bytes());
                    }
                }
                Event::Key { key, pressed: true, modifiers, .. } => {
                    if closed && key == Key::Enter {
                        reconnect = true;
                    } else if modifiers.ctrl && !modifiers.shift && key == Key::F {
                        open_search = true;
                    } else if modifiers.ctrl && modifiers.shift && key == Key::C {
                        copy = selected_text.clone();
                    } else if let Some(b) = key_bytes(key, modifiers, app_cursor) {
                        out.extend(b);
                    }
                }
                _ => {}
            }
        }
        if !out.is_empty() && !closed {
            tab.term.send(out);
            tab.scroll = 0;
            tab.selection = None;
        }
    }
    if let Some(t) = copy {
        ui.ctx().copy_text(t.trim_end().to_string());
    }
    if open_search {
        let se = tab.search.get_or_insert_with(TermSearch::default);
        if let Some(t) = selected_text.as_deref().map(str::trim).filter(|t| !t.is_empty() && !t.contains('\n')) {
            se.query = t.to_string();
        }
        se.focus = true;
    }
    resp.context_menu(|ui| {
        if ui.add_enabled(selected_text.is_some(), egui::Button::new(t!("Kopieren")).shortcut_text(&crate::i18n::keys("Strg+C"))).clicked() {
            ui.ctx().copy_text(selected_text.clone().unwrap_or_default().trim_end().to_string());
            ui.close();
        }
        if ui.add(egui::Button::new(t!("Einfügen")).shortcut_text(&crate::i18n::keys("Strg+V"))).clicked() {
            if let Some(t) = crate::win32::clipboard_text() {
                tab.term.send(t.replace('\n', "\r").into_bytes());
            }
            ui.close();
        }
        ui.separator();
        if ui.button(t!("Verlauf leeren")).clicked() {
            tab.term.parser.lock().unwrap().screen_mut().clear_scrollback();
            tab.scroll = 0;
            ui.close();
        }
        if closed && ui.button(t!("Neu verbinden")).clicked() {
            reconnect = true;
            ui.close();
        }
    });

    // Drawing.
    let painter = ui.painter_at(body);
    // The panel's (possibly translucent) background shows through; the opaque
    // version stands in where a color is needed (e.g. inverse text).

    let default_bg = theme.sidebar_bg.to_opaque();
    let palette = ansi_palette(theme.dark);
    let color = |c: vt100::Color, fg: bool, bold: bool| -> Color32 {
        match c {
            vt100::Color::Default => if fg { theme.foreground } else { default_bg },
            vt100::Color::Idx(i) => {
                // Bold text uses the bright variant of the basic colors.
                let i = if fg && bold && i < 8 { i + 8 } else { i };
                indexed(i, &palette)
            }
            vt100::Color::Rgb(r, g, b) => Color32::from_rgb(r, g, b),
        }
    };
    let glyph_y = (row_h - ui.fonts_mut(|f| f.row_height(&font))) / 2.0;
    // Search matches in view (behind the text).
    if let Some(se) = &tab.search {
        let top = hist - tab.scroll;
        let first = se.matches.partition_point(|m| m.0 < top);
        for (n, &(line, a, b)) in se.matches.iter().enumerate().skip(first).take_while(|(_, m)| m.0 < top + rows as usize) {
            let y = inner.top() + (line - top) as f32 * row_h;
            let r = Rect::from_min_max(pos2(inner.left() + a as f32 * char_w, y), pos2(inner.left() + b as f32 * char_w, y + row_h));
            let current = se.current == Some(n);
            painter.rect_filled(r, 2.0, if current { theme.find_match } else { theme.find_match.gamma_multiply(0.55) });
            if current {
                painter.rect_stroke(r, 2.0, Stroke::new(1.0, theme.accent), egui::StrokeKind::Outside);
            }
        }
    }
    for run in &runs {
        let y = inner.top() + run.row as f32 * row_h;
        let l = run.look;
        let (mut fg, mut bg) = (color(l.fg, true, l.bold), color(l.bg, false, false));
        if l.inverse {
            std::mem::swap(&mut fg, &mut bg);
        }
        if l.selected {
            bg = theme.selection;
        }
        let x0 = inner.left() + run.start as f32 * char_w;
        let x1 = inner.left() + run.end as f32 * char_w;
        if bg != default_bg {
            painter.rect_filled(Rect::from_min_max(pos2(x0, y), pos2(x1, y + row_h)), 0.0, bg);
        }
        if !run.text.trim().is_empty() {
            let mut job = egui::text::LayoutJob::default();
            let mut format = egui::TextFormat::simple(font.clone(), fg);
            format.italics = l.italic;
            if l.underline {
                format.underline = Stroke::new(1.0, fg);
            }
            job.append(&run.text, 0.0, format);
            let galley = painter.layout_job(job);
            painter.galley(pos2(x0, y + glyph_y), galley.clone(), fg);
            if l.bold {
                // Faux bold: a second pass a hair to the right.
                painter.galley(pos2(x0 + 0.6, y + glyph_y), galley, fg);
            }
        }
    }
    // Cursor.
    if tab.scroll == 0 && !modes.hide_cursor && !closed {
        let (r, c) = modes.cursor;
        let rect = Rect::from_min_size(pos2(inner.left() + c as f32 * char_w, inner.top() + r as f32 * row_h), vec2(char_w, row_h));
        if focused {
            painter.rect_filled(rect, 0.0, theme.cursor.gamma_multiply(0.7));
        } else {
            painter.rect_stroke(rect, 0.0, Stroke::new(1.0, theme.cursor), egui::StrokeKind::Inside);
        }
    }
    // Scroll position and connection state.
    if tab.scroll > 0 {
        let label = t!("↑ {scroll} Zeilen zurück", scroll = tab.scroll);
        // Below the search field while it is open.
        let y = if tab.search.is_some() { 44.0 } else { 8.0 };
        painter.text(pos2(body.right() - 12.0, body.top() + y), egui::Align2::RIGHT_TOP, label, FontId::proportional(11.5), theme.text_faint);
    }
    let note = match &tab.term.status {
        TermStatus::Connecting => Some(t!("Verbinde …").to_string()),
        TermStatus::Closed(Some(err)) if tab.local.is_some() => Some(t!("{err}  –  Enter startet neu", err = err)),
        TermStatus::Closed(None) if tab.local.is_some() => Some(t!("Shell beendet  –  Enter startet neu").to_string()),
        TermStatus::Closed(Some(err)) => Some(t!("Verbindung fehlgeschlagen: {err}  –  Enter verbindet neu", err = err)),
        TermStatus::Closed(None) => Some(t!("Sitzung beendet  –  Enter verbindet neu").to_string()),
        TermStatus::Connected => None,
    };
    if let Some(note) = note {
        let r = Rect::from_min_size(pos2(body.left(), body.bottom() - 26.0), vec2(body.width(), 26.0));
        painter.rect_filled(r, 0.0, theme.jumpbar_bg);
        painter.text(pos2(r.left() + 12.0, r.center().y), egui::Align2::LEFT_CENTER, note, FontId::proportional(12.0), theme.text_muted);
    }
    if !closed && focused {
        // Keep the blinking-free cursor current while output arrives.
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(500));
    }
    search_bar(ui, tab, body, theme, hist, rows);
    reconnect
}

/// The view position (lines scrolled back) that shows `line` in the middle.
fn scroll_to_line(line: usize, hist: usize, rows: u16) -> usize {
    let top = line.saturating_sub(rows as usize / 2).min(hist);
    hist - top
}

/// The search field at the top right of the terminal (Ctrl+F).
fn search_bar(ui: &mut egui::Ui, tab: &mut TermTab, body: Rect, theme: &crate::theme::Theme, hist: usize, rows: u16) {
    let Some(se) = &mut tab.search else { return };
    let field_id = term_id(tab.term.id).with("search");
    let mut close = false;
    let mut step: isize = 0;
    egui::Area::new(term_id(tab.term.id).with("search-area"))
        .fixed_pos(pos2(body.right() - 380.0, body.top() + 6.0))
        .order(egui::Order::Foreground)
        .show(ui.ctx(), |ui| {
            egui::Frame::popup(ui.style()).inner_margin(egui::Margin::symmetric(8, 4)).show(ui, |ui| {
                ui.horizontal(|ui| {
                    let resp = ui.add(egui::TextEdit::singleline(&mut se.query).id(field_id).hint_text(t!("Im Verlauf suchen")).desired_width(180.0));
                    if std::mem::take(&mut se.focus) {
                        resp.request_focus();
                    }
                    if resp.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
                        step = if ui.input(|i| i.modifiers.shift) { 1 } else { -1 };
                        resp.request_focus();
                    }
                    // The field gives up its focus on Escape by itself.
                    if (resp.has_focus() || resp.lost_focus()) && ui.input(|i| i.key_pressed(Key::Escape)) {
                        close = true;
                    }
                    let count = match (se.matches.len(), se.current) {
                        (0, _) if se.query.is_empty() => String::new(),
                        (0, _) => t!("Keine Treffer").to_string(),
                        (n, Some(c)) => t!("{value} von {n}", value = c + 1, n = n),
                        (n, None) => t!("{n} Treffer", n = n),
                    };
                    ui.label(egui::RichText::new(count).small().color(theme.text_faint));
                    if ui.small_button("▲").on_hover_text(t!("Älterer Treffer (Enter)")).clicked() {
                        step = -1;
                    }
                    if ui.small_button("▼").on_hover_text(t!("Neuerer Treffer (Umschalt+Enter)")).clicked() {
                        step = 1;
                    }
                    if ui.small_button("×").on_hover_text(t!("Schließen (Esc)")).clicked() {
                        close = true;
                    }
                });
            });
        });
    if step != 0 && !se.matches.is_empty() {
        let n = se.matches.len() as isize;
        let c = se.current.map_or(if step < 0 { n - 1 } else { 0 }, |c| (c as isize + step).rem_euclid(n));
        se.current = Some(c as usize);
        tab.scroll = scroll_to_line(se.matches[c as usize].0, hist, rows);
    }
    if close {
        tab.search = None;
        tab.want_focus = true;
    }
}

/// Screen modes read along with the cells.
struct Modes {
    app_cursor: bool,
    bracketed_paste: bool,
    hide_cursor: bool,
    cursor: (u16, u16),
}

/// How a cell looks; neighbouring cells that look the same form a run.
#[derive(Clone, Copy, PartialEq)]
struct Look {
    fg: vt100::Color,
    bg: vt100::Color,
    bold: bool,
    italic: bool,
    underline: bool,
    inverse: bool,
    selected: bool,
}

impl Look {
    fn of(cell: &vt100::Cell, selected: bool) -> Self {
        Look {
            fg: cell.fgcolor(),
            bg: cell.bgcolor(),
            bold: cell.bold(),
            italic: cell.italic(),
            underline: cell.underline(),
            inverse: cell.inverse(),
            selected,
        }
    }
}

/// Cells of one row drawn together.
struct Run {
    row: u16,
    start: u16,
    end: u16,
    text: String,
    look: Look,
}

/// The 16 basic colors, tuned for dark or light backgrounds.
fn ansi_palette(dark: bool) -> [Color32; 16] {
    let c = Color32::from_rgb;
    if dark {
        [
            c(30, 30, 30), c(229, 83, 75), c(80, 200, 120), c(229, 192, 80), c(80, 150, 240), c(200, 110, 220), c(60, 190, 210), c(210, 212, 216),
            c(110, 112, 118), c(255, 110, 100), c(110, 230, 150), c(250, 220, 110), c(120, 180, 255), c(230, 140, 250), c(100, 220, 235), c(255, 255, 255),
        ]
    } else {
        [
            c(0, 0, 0), c(200, 40, 40), c(20, 140, 50), c(150, 120, 0), c(20, 80, 190), c(160, 40, 170), c(10, 130, 150), c(110, 110, 110),
            c(90, 90, 90), c(220, 60, 60), c(30, 170, 70), c(180, 150, 0), c(40, 110, 220), c(190, 60, 200), c(20, 160, 180), c(60, 60, 60),
        ]
    }
}

/// xterm's 256 colors: 16 basic, a 6×6×6 cube and a gray ramp.
fn indexed(i: u8, palette: &[Color32; 16]) -> Color32 {
    match i {
        0..=15 => palette[i as usize],
        16..=231 => {
            let i = i - 16;
            let level = |v: u8| if v == 0 { 0 } else { 55 + v * 40 };
            Color32::from_rgb(level(i / 36), level(i / 6 % 6), level(i % 6))
        }
        _ => {
            let v = 8 + (i - 232) * 10;
            Color32::from_rgb(v, v, v)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn search_in_history() {
        let mut p = vt100::Parser::new(4, 40, 100);
        for n in 0..10 {
            p.process(format!("Zeile {n} ERROR x\r\n").as_bytes());
        }
        let (lines, hist) = all_lines(&mut p, 4, 40);
        assert_eq!(hist, 7);
        assert_eq!(lines.len(), 11);
        assert_eq!(lines[0], "Zeile 0 ERROR x");
        assert_eq!(lines[9], "Zeile 9 ERROR x");
        // The view stays where it was.
        assert_eq!(p.screen().scrollback(), 0);
        let m = find_in_lines(&lines, "error");
        assert_eq!(m.len(), 10);
        assert_eq!(m[3], (3, 8, 13));
        assert_eq!(find_in_lines(&["aaaa".into()], "aa"), [(0, 0, 2), (0, 2, 4)]);
        // Line 3 in the middle of a 4-row view.
        assert_eq!(scroll_to_line(3, 7, 4), 6);
        assert_eq!(scroll_to_line(10, 7, 4), 0);
    }

    #[test]
    fn indexed_colors() {
        let p = ansi_palette(true);
        assert_eq!(indexed(1, &p), p[1]);
        assert_eq!(indexed(16, &p), Color32::from_rgb(0, 0, 0));
        assert_eq!(indexed(231, &p), Color32::from_rgb(255, 255, 255));
        assert_eq!(indexed(232, &p), Color32::from_rgb(8, 8, 8));
    }
}
