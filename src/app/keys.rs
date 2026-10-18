//! Keyboard shortcuts: defaults, user overrides from the settings, dispatch
//! and the "Tastenkürzel" settings page.

use std::cmp::Reverse;

use eframe::egui::{self, Key, KeyboardShortcut, Modifiers, RichText, Ui};

use super::NebulaApp;
use super::palette::Command;

const CTRL: Modifiers = Modifiers::COMMAND;

fn ks(m: Modifiers, k: Key) -> KeyboardShortcut {
    KeyboardShortcut::new(m, k)
}

/// Built-in shortcuts of a command.
pub(super) fn defaults(c: Command) -> Vec<KeyboardShortcut> {
    use Command::*;
    let shift = Modifiers::SHIFT;
    let alt = Modifiers::ALT;
    let none = Modifiers::NONE;
    match c {
        NewFile => vec![ks(CTRL, Key::N)],
        OpenFile => vec![ks(CTRL, Key::O)],
        OpenFolder => vec![ks(CTRL | shift, Key::O)],
        QuickOpen => vec![ks(CTRL, Key::P)],
        CommandPalette => vec![ks(CTRL | shift, Key::P)],
        Save => vec![ks(CTRL, Key::S)],
        SaveAs => vec![ks(CTRL | shift, Key::S)],
        SaveAll => vec![ks(CTRL | alt, Key::S)],
        CloseTab => vec![ks(CTRL, Key::W), ks(CTRL, Key::F4)],
        NextTab => vec![ks(CTRL, Key::Tab), ks(CTRL, Key::PageDown)],
        PrevTab => vec![ks(CTRL | shift, Key::Tab), ks(CTRL, Key::PageUp)],
        Print => vec![ks(CTRL | alt, Key::P)],
        Find => vec![ks(CTRL, Key::F)],
        Replace => vec![ks(CTRL, Key::H)],
        FindInProject => vec![ks(CTRL | shift, Key::F)],
        FindNext => vec![ks(none, Key::F3)],
        FindPrevious => vec![ks(shift, Key::F3)],
        GotoLine => vec![ks(CTRL, Key::G)],
        GotoSymbol => vec![ks(CTRL, Key::R)],
        NavBack => vec![ks(alt, Key::ArrowLeft)],
        NavForward => vec![ks(alt, Key::ArrowRight)],
        ToggleSidebar => vec![ks(CTRL, Key::B)],
        FontBigger => vec![ks(CTRL, Key::Plus), ks(CTRL, Key::Equals)],
        FontSmaller => vec![ks(CTRL, Key::Minus)],
        FontReset => vec![ks(CTRL, Key::Num0)],
        ToggleWordWrap => vec![ks(alt, Key::Z)],
        ToggleSplit => vec![ks(CTRL | alt, Key::T), ks(CTRL, Key::Backslash)],
        // Strg+Ö: German keyboards report the key by its US position (`;`).
        ToggleTerminal => vec![ks(CTRL, Key::Semicolon), ks(CTRL, Key::Backtick)],
        Settings => vec![ks(CTRL, Key::Comma)],
        TogglePreview => vec![ks(CTRL | shift, Key::V)],
        OpenInBrowser => vec![ks(none, Key::F5)],
        ToggleComment => vec![ks(CTRL, Key::Slash)],
        DuplicateLine => vec![ks(CTRL | shift, Key::D)],
        DeleteLine => vec![ks(CTRL | shift, Key::K)],
        SelectAllOccurrences => vec![ks(CTRL | shift, Key::L)],
        CursorsAtLineEnds => vec![ks(alt | shift, Key::I)],
        _ => Vec::new(),
    }
}

/// Commands the editor handles itself with their default keys.
fn editor_command(c: Command) -> bool {
    use Command::*;
    matches!(c, ToggleComment | DuplicateLine | DeleteLine | SelectAllOccurrences | CursorsAtLineEnds)
}

/// Commands that can have shortcuts (no per-item commands like languages or servers).
pub(super) fn bindable() -> Vec<(Command, String)> {
    Command::all()
        .into_iter()
        .filter(|(c, _)| !matches!(c, Command::Language(_) | Command::Ext(_) | Command::Clip(_) | Command::Terminal(_) | Command::SwitchProject(_)))
        .collect()
}

fn command_id(c: Command) -> String {
    format!("{c:?}")
}

/// Stored form, e.g. "Ctrl+Shift+P".
fn to_text(s: &KeyboardShortcut) -> String {
    let mut out = String::new();
    if s.modifiers.command || s.modifiers.ctrl {
        out.push_str("Ctrl+");
    }
    if s.modifiers.alt {
        out.push_str("Alt+");
    }
    if s.modifiers.shift {
        out.push_str("Shift+");
    }
    out.push_str(s.logical_key.name());
    out
}

fn parse(text: &str) -> Option<KeyboardShortcut> {
    let mut m = Modifiers::NONE;
    let parts: Vec<&str> = text.split('+').map(str::trim).collect();
    let (key, mods) = match parts.split_last() {
        // "Ctrl++" splits into an empty last part.
        Some((&"", rest)) if text.ends_with("++") => ("Plus", &rest[..rest.len().saturating_sub(1)]),
        Some((k, rest)) => (*k, rest),
        None => return None,
    };
    for p in mods {
        match p.to_ascii_lowercase().as_str() {
            "ctrl" | "strg" => m = m | CTRL,
            "alt" => m = m | Modifiers::ALT,
            "shift" | "umschalt" => m = m | Modifiers::SHIFT,
            _ => return None,
        }
    }
    Some(ks(m, Key::from_name(key)?))
}

/// Display form with German key names, e.g. "Strg+Umschalt+P".
pub(super) fn label(s: &KeyboardShortcut) -> String {
    let mut out = String::new();
    if s.modifiers.command || s.modifiers.ctrl {
        out.push_str("Strg+");
    }
    if s.modifiers.alt {
        out.push_str("Alt+");
    }
    if s.modifiers.shift {
        out.push_str("Umschalt+");
    }
    let key = match s.logical_key {
        Key::Semicolon => "Ö",
        Key::Quote => "Ä",
        Key::OpenBracket => "Ü",
        Key::PageUp => "Bild↑",
        Key::PageDown => "Bild↓",
        Key::Enter => "Eingabe",
        Key::Escape => "Esc",
        Key::Space => "Leertaste",
        Key::Delete => "Entf",
        Key::Backspace => "Rücktaste",
        Key::Home => "Pos1",
        Key::End => "Ende",
        Key::Insert => "Einfg",
        k => k.symbol_or_name(),
    };
    out.push_str(key);
    out
}

/// Keys that only modify or toggle (Strg, Umschalt, NumLock …) and can't be a shortcut on their own.
fn is_modifier_or_lock(key: Key) -> bool {
    let name = key.name();
    ["Control", "Shift", "Alt", "Meta", "Super", "Lock"].iter().any(|m| name.contains(m))
}

fn modifier_count(m: Modifiers) -> usize {
    usize::from(m.command || m.ctrl) + usize::from(m.alt) + usize::from(m.shift)
}

impl NebulaApp {
    /// Shortcuts of a command: the user's choice, else the defaults.
    pub(super) fn bindings(&self, c: Command) -> Vec<KeyboardShortcut> {
        match self.settings.keybindings.get(&command_id(c)) {
            Some(text) => text.split(',').filter_map(|t| parse(t.trim())).collect(),
            None => defaults(c),
        }
    }

    /// First shortcut of a command for menus and the palette.
    pub(super) fn shortcut_label(&self, c: Command) -> String {
        self.bindings(c).first().map(label).unwrap_or_default()
    }

    fn set_bindings(&mut self, c: Command, list: &[KeyboardShortcut]) {
        let id = command_id(c);
        if list == defaults(c).as_slice() {
            self.settings.keybindings.remove(&id);
        } else {
            let text: Vec<String> = list.iter().map(to_text).collect();
            self.settings.keybindings.insert(id, text.join(", "));
        }
    }

    /// Runs the command whose shortcut was pressed.
    pub(super) fn dispatch_shortcuts(&mut self, ctx: &egui::Context) {
        let terminal = self.terminal_focused(ctx);
        let editor_focused = match ctx.memory(|m| m.focused()) {
            None => true,
            Some(id) => Some(id) == self.active_editor_id(),
        };
        // (shortcut, command); `None` swallows an editor default that was reassigned.
        let mut table: Vec<(KeyboardShortcut, Option<Command>)> = Vec::new();
        for (c, _) in bindable() {
            if terminal && !matches!(c, Command::ToggleTerminal | Command::CommandPalette) {
                continue;
            }
            if editor_command(c) {
                // Unchanged editor keys are handled by the editor itself.
                if !self.settings.keybindings.contains_key(&command_id(c)) || !editor_focused {
                    continue;
                }
                table.extend(defaults(c).into_iter().map(|s| (s, None)));
            }
            table.extend(self.bindings(c).into_iter().map(|s| (s, Some(c))));
        }
        // Extra modifiers don't keep a shortcut from matching, so longer
        // combinations come first; real commands before swallowed defaults.
        table.sort_by_key(|(s, c)| (Reverse(modifier_count(s.modifiers)), c.is_none()));
        for (s, c) in table {
            if !ctx.input_mut(|i| i.consume_shortcut(&s)) {
                continue;
            }
            if s.modifiers.alt && !s.modifiers.command {
                // Alt+letter may also arrive as typed text.
                let name = s.logical_key.name().to_lowercase();
                ctx.input_mut(|i| i.events.retain(|e| !matches!(e, egui::Event::Text(t) if t.to_lowercase() == name)));
            }
            if let Some(c) = c {
                self.run_command(ctx, c);
            }
            return;
        }
    }

    /// Settings page listing every command with its shortcuts.
    pub(super) fn keys_page(&mut self, ui: &mut Ui, state: &mut KeysPage) {
        ui.label(RichText::new("Tastenkürzel").size(16.0).strong());
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.add(egui::TextEdit::singleline(&mut state.filter).hint_text("Befehl oder Taste suchen…").desired_width(300.0));
            if !self.settings.keybindings.is_empty() && ui.button("Alle zurücksetzen").clicked() {
                self.settings.keybindings.clear();
                state.recording = None;
            }
        });
        ui.add_space(8.0);

        // Recording: the next key combination becomes the shortcut.
        if let Some(c) = state.recording {
            let pressed = ui.input(|i| {
                i.events.iter().find_map(|e| match e {
                    egui::Event::Key { key, pressed: true, modifiers, .. } if !is_modifier_or_lock(*key) => Some((*key, *modifiers)),
                    _ => None,
                })
            });
            if let Some((key, mods)) = pressed {
                ui.input_mut(|i| i.events.clear());
                if key == Key::Escape && mods.is_none() {
                    state.recording = None;
                } else {
                    let mut list = self.bindings(c);
                    let s = ks(Modifiers { mac_cmd: false, command: mods.command || mods.ctrl, ctrl: false, ..mods }, key);
                    if state.replace {
                        list = vec![s];
                    } else if !list.contains(&s) {
                        list.push(s);
                    }
                    self.set_bindings(c, &list);
                    state.recording = None;
                }
            }
        }

        // Shortcut -> commands, to show conflicts.
        let all = bindable();
        let mut used: Vec<(KeyboardShortcut, Command)> = Vec::new();
        for (c, _) in &all {
            used.extend(self.bindings(*c).into_iter().map(|s| (s, *c)));
        }
        let filter = state.filter.to_lowercase();
        egui::Grid::new("keys-grid").num_columns(3).spacing([12.0, 6.0]).striped(true).show(ui, |ui| {
            for (c, name) in &all {
                let list = self.bindings(*c);
                let labels: Vec<String> = list.iter().map(label).collect();
                if !filter.is_empty() && !name.to_lowercase().contains(&filter) && !labels.iter().any(|l| l.to_lowercase().contains(&filter)) {
                    continue;
                }
                let custom = self.settings.keybindings.contains_key(&command_id(*c));
                ui.label(if custom { RichText::new(name).strong() } else { RichText::new(name) });
                ui.horizontal(|ui| {
                    if state.recording == Some(*c) {
                        ui.label(RichText::new("Tasten drücken… (Esc: abbrechen)").color(self.theme.accent));
                        return;
                    }
                    let mut remove = None;
                    for (n, s) in list.iter().enumerate() {
                        let others: Vec<String> = used
                            .iter()
                            .filter(|(u, other)| u == s && other != c)
                            .filter_map(|(_, other)| all.iter().find(|(x, _)| x == other).map(|(_, n)| n.clone()))
                            .collect();
                        let text = RichText::new(&labels[n]).monospace();
                        let text = if others.is_empty() { text } else { text.color(self.theme.danger) };
                        let mut r = ui.add(egui::Button::new(text).small()).on_hover_text("Klicken zum Entfernen");
                        if !others.is_empty() {
                            r = r.on_hover_text(format!("Auch belegt mit: {}", others.join(", ")));
                        }
                        if r.clicked() {
                            remove = Some(n);
                        }
                    }
                    if list.is_empty() {
                        ui.label(RichText::new("—").color(self.theme.text_faint));
                    }
                    if let Some(n) = remove {
                        let mut list = list.clone();
                        list.remove(n);
                        self.set_bindings(*c, &list);
                    }
                });
                ui.horizontal(|ui| {
                    if ui.small_button("Ändern").on_hover_text("Neue Tastenkombination drücken").clicked() {
                        state.recording = Some(*c);
                        state.replace = true;
                    }
                    if ui.small_button("+").on_hover_text("Weitere Tastenkombination hinzufügen").clicked() {
                        state.recording = Some(*c);
                        state.replace = false;
                    }
                    // Always present (disabled when unchanged) so the layout doesn't jump.
                    if ui.add_enabled(custom, egui::Button::new("↺").small()).on_hover_text("Standard wiederherstellen").clicked() {
                        self.settings.keybindings.remove(&command_id(*c));
                    }
                });
                ui.end_row();
            }
        });
    }
}

#[derive(Default)]
pub struct KeysPage {
    filter: String,
    /// Waiting for a key combination for this command.
    pub recording: Option<Command>,
    /// The recorded combination replaces the existing ones (else it is added).
    replace: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_roundtrip() {
        for s in [ks(CTRL | Modifiers::SHIFT, Key::P), ks(Modifiers::NONE, Key::F5), ks(CTRL, Key::Plus), ks(CTRL | Modifiers::ALT, Key::Semicolon)] {
            assert_eq!(parse(&to_text(&s)), Some(s), "{}", to_text(&s));
        }
        assert_eq!(label(&ks(CTRL, Key::Semicolon)), "Strg+Ö");
    }
}
