//! The "Git" tab of the sidebar: branch, pull/push, commit and the lists of
//! staged and unstaged changes.

use std::path::PathBuf;

use eframe::egui::{self, Align2, FontId, Key, Modifiers, RichText, Sense, Ui, pos2, vec2};

use super::git::{Change, GitStatus};
use super::icons;
use crate::theme::Theme;

/// What the panel asks the app to do.
pub enum GitAction {
    /// Show the changes of a file against the last commit.
    Compare(PathBuf),
    Open(PathBuf),
}

#[derive(Default)]
pub struct GitPanel {
    pub message: String,
    new_branch: String,
}

fn args(a: &[&str]) -> Vec<String> {
    a.iter().map(|s| s.to_string()).collect()
}

impl GitPanel {
    pub fn show(&mut self, ui: &mut Ui, git: &mut GitStatus, theme: &Theme, actions: &mut Vec<GitAction>) {
        let ctx = ui.ctx().clone();
        if git.top.is_none() {
            ui.add_space(30.0);
            ui.vertical_centered(|ui| ui.label(RichText::new(t!("Der Projektordner ist kein Git-Repository.")).color(theme.text_faint)));
            return;
        }
        let busy = git.op.is_some();
        ui.add_space(6.0);

        // Branch menu on the left, pull/push on the right.
        let mut steps: Option<(&str, Vec<Vec<String>>)> = None;
        ui.horizontal(|ui| {
            let branch = git.branch.clone().unwrap_or_else(|| "–".into());
            let resp = ui.add_enabled(!busy, egui::Button::new(format!("⎇ {branch} ▾")).frame(false)).on_hover_text(t!("Branch wechseln"));
            egui::Popup::menu(&resp).close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside).show(|ui| {
                ui.set_min_width(220.0);
                ui.label(RichText::new(t!("Branch wechseln")).small().color(theme.text_faint));
                for b in &git.branches {
                    if ui.add(egui::Button::selectable(Some(b) == git.branch.as_ref(), b)).clicked() && Some(b) != git.branch.as_ref() {
                        steps = Some((t!("Branch wechseln"), vec![args(&["switch", b])]));
                        ui.close();
                    }
                }
                ui.separator();
                let field = ui.add(egui::TextEdit::singleline(&mut self.new_branch).hint_text(t!("Neuer Branch")).desired_width(f32::INFINITY));
                let name = self.new_branch.trim().to_string();
                let enter = field.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
                if (ui.add_enabled(!name.is_empty(), egui::Button::new(t!("Anlegen und wechseln"))).clicked() || (enter && !name.is_empty()))
                    && !name.contains(char::is_whitespace)
                {
                    steps = Some((t!("Branch anlegen"), vec![args(&["switch", "-c", &name])]));
                    self.new_branch.clear();
                    ui.close();
                }
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let (behind, ahead) = git.sync.unwrap_or((0, 0));
                let push = if ahead > 0 { t!("Push ↑{ahead}", ahead = ahead) } else { t!("Push").into() };
                let tip = if git.sync.is_none() { t!("Branch hochladen (legt ihn auf „origin“ an)") } else { t!("Commits hochladen") };
                if ui.add_enabled(!busy, egui::Button::new(push).small()).on_hover_text(tip).clicked() {
                    let cmd = if git.sync.is_none() { args(&["push", "-u", "origin", "HEAD"]) } else { args(&["push"]) };
                    steps = Some((t!("Push"), vec![cmd]));
                }
                let pull = if behind > 0 { t!("Pull ↓{behind}", behind = behind) } else { t!("Pull").into() };
                if ui.add_enabled(!busy && git.sync.is_some(), egui::Button::new(pull).small()).on_hover_text(t!("Änderungen vom Server holen und einspielen")).clicked() {
                    steps = Some((t!("Pull"), vec![args(&["pull"])]));
                }
                if ui.add_enabled(!busy && git.sync.is_some(), egui::Button::new("↻").small()).on_hover_text(t!("Abrufen (fetch): prüfen, ob es Neues gibt")).clicked() {
                    steps = Some((t!("Abrufen"), vec![args(&["fetch"])]));
                }
            });
        });
        ui.add_space(4.0);

        // Commit message and button.
        let staged = git.changes.iter().filter(|c| c.staged()).count();
        let total = git.changes.len();
        let msg = ui.add(
            egui::TextEdit::multiline(&mut self.message)
                .hint_text(t!("Commit-Nachricht (Strg+Enter)"))
                .desired_rows(3)
                .desired_width(f32::INFINITY),
        );
        let message = self.message.trim().to_string();
        let label = match (staged, total) {
            (0, 0) => t!("Keine Änderungen").to_string(),
            (0, 1) => t!("Die Änderung committen").to_string(),
            (0, n) => t!("Alle {n} Änderungen committen", n = n),
            (1, _) => t!("Commit (1 Datei)").to_string(),
            (n, _) => t!("Commit ({n} Dateien)", n = n),
        };
        let can_commit = !busy && total > 0 && !message.is_empty();
        let shortcut = msg.has_focus() && ui.input_mut(|i| i.consume_key(Modifiers::COMMAND, Key::Enter));
        let button = ui.add_enabled(can_commit, egui::Button::new(label).min_size(vec2(ui.available_width(), 26.0)));
        if (button.clicked() || shortcut) && can_commit {
            let commit = args(&["commit", "-m", &message]);
            // Nothing staged: commit everything, like most editors do.
            steps = Some((t!("Commit"), if staged == 0 { vec![args(&["add", "-A"]), commit] } else { vec![commit] }));
        }
        if let Some(op) = &git.op {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(RichText::new(format!("{} …", op.label)).small().color(theme.text_faint));
            });
        }
        ui.add_space(6.0);

        egui::ScrollArea::vertical().id_salt("git-changes").auto_shrink([false, false]).show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 1.0;
            let staged_list: Vec<Change> = git.changes.iter().filter(|c| c.staged()).cloned().collect();
            let unstaged_list: Vec<Change> = git.changes.iter().filter(|c| c.unstaged()).cloned().collect();
            if !staged_list.is_empty() {
                if section(ui, theme, "VORGEMERKT", staged_list.len(), "−", t!("Alle zurücknehmen"), busy) {
                    steps = Some((t!("Zurücknehmen"), vec![args(&["restore", "--staged", "--", "."])]));
                }
                for c in &staged_list {
                    match row(ui, theme, c, "−", t!("Zurücknehmen"), busy) {
                        RowClick::Button => steps = Some((t!("Zurücknehmen"), vec![args(&["restore", "--staged", "--", &c.rel])])),
                        RowClick::Row => actions.push(open_or_compare(c)),
                        RowClick::None => {}
                    }
                }
                ui.add_space(6.0);
            }
            if section(ui, theme, "ÄNDERUNGEN", unstaged_list.len(), "+", t!("Alle vormerken"), busy || unstaged_list.is_empty()) {
                steps = Some((t!("Vormerken"), vec![args(&["add", "-A"])]));
            }
            if unstaged_list.is_empty() && staged_list.is_empty() {
                ui.add_space(8.0);
                ui.label(RichText::new(t!("Keine Änderungen seit dem letzten Commit.")).small().color(theme.text_faint));
            }
            for c in &unstaged_list {
                match row(ui, theme, c, "+", t!("Vormerken"), busy) {
                    RowClick::Button => steps = Some((t!("Vormerken"), vec![args(&["add", "-A", "--", &c.rel])])),
                    RowClick::Row => actions.push(open_or_compare(c)),
                    RowClick::None => {}
                }
            }
        });

        if let Some((label, steps)) = steps {
            git.run(label, steps, &ctx);
        }
    }
}

/// New files have no committed version to compare with; they just open.
fn open_or_compare(c: &Change) -> GitAction {
    match c.state() {
        super::git::FileState::Untracked | super::git::FileState::Added => GitAction::Open(c.path.clone()),
        _ => GitAction::Compare(c.path.clone()),
    }
}

/// Section title with a count and an action for all files; true if clicked.
fn section(ui: &mut Ui, theme: &Theme, title: &str, count: usize, sign: &str, tip: &str, disabled: bool) -> bool {
    let mut clicked = false;
    ui.horizontal(|ui| {
        ui.label(RichText::new(format!("{title}  {count}")).small().strong().color(theme.text_faint));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            clicked = ui.add_enabled(!disabled, egui::Button::new(sign).small().frame(false)).on_hover_text(tip).clicked();
        });
    });
    clicked
}

enum RowClick {
    None,
    Row,
    Button,
}

/// A changed file: icon, name, folder, state letter and (on hover) the button.
fn row(ui: &mut Ui, theme: &Theme, c: &Change, sign: &str, tip: &str, busy: bool) -> RowClick {
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 24.0), Sense::click());
    // By pointer position: the button on top would otherwise take the hover away
    // from the row, hide itself and flicker.
    let hovered = ui.rect_contains_pointer(rect);
    if hovered {
        ui.painter().rect_filled(rect, 6.0, theme.hover);
    }
    let cy = rect.center().y;
    let (dir, name) = c.rel.rsplit_once('/').unwrap_or(("", &c.rel));
    let painter = ui.painter().with_clip_rect(rect);
    icons::file(&painter, pos2(rect.left() + 12.0, cy), name);
    let g = painter.layout_no_wrap(name.to_string(), FontId::proportional(13.0), theme.foreground);
    let name_w = g.size().x;
    painter.galley(pos2(rect.left() + 26.0, cy - g.size().y / 2.0), g, theme.foreground);
    painter.text(pos2(rect.left() + 32.0 + name_w, cy), Align2::LEFT_CENTER, dir, FontId::proportional(11.0), theme.text_faint);
    let state = c.state();
    painter.text(pos2(rect.right() - 10.0, cy), Align2::RIGHT_CENTER, state.letter(), FontId::proportional(11.5), state.color(theme));
    let resp = resp.on_hover_text(&c.rel);
    // The stage/unstage button appears on hover, left of the letter.
    let button = egui::Rect::from_center_size(pos2(rect.right() - 34.0, cy), vec2(20.0, 20.0));
    if hovered && !busy {
        let b = ui.interact(button, egui::Id::unique(("git-row", &c.rel, sign)), Sense::click()).on_hover_text(tip);
        if b.hovered() {
            ui.painter().rect_filled(button, 4.0, theme.hover.gamma_multiply(2.0));
        }
        ui.painter().text(button.center(), Align2::CENTER_CENTER, sign, FontId::proportional(15.0), theme.foreground);
        if b.clicked() {
            return RowClick::Button;
        }
    }
    if resp.clicked() { RowClick::Row } else { RowClick::None }
}
