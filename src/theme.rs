//! Colors for the editor and the application chrome.
//!
//! The syntax palettes follow the scopes and hues of Nova's "Dark" and
//! "Bright" themes; the chrome is modeled after Nova's window layout.

use eframe::egui::{self, Color32, CornerRadius, Margin, Shadow, Stroke, Visuals};

/// Highlight names understood by the highlighter. A capture like
/// `tag.attribute.value.delimiter` uses the longest matching entry here.
/// The index into this list is the style id stored in highlight spans.
pub const HIGHLIGHT_NAMES: &[&str] = &[
    // Nova scopes (used by the HTML and CSS queries).
    "comment",
    "processing",
    "invalid",
    "bracket",
    "operator",
    "keyword",
    "value.number",
    "value.entity",
    "identifier.type",
    "identifier.variable",
    "identifier.property",
    "identifier.function",
    "string",
    "string.delimiter",
    "string.escape",
    "tag",
    "tag.name",
    "tag.attribute.name",
    "tag.attribute.operator",
    "tag.attribute.value",
    "tag.attribute.value.delimiter",
    "tag.attribute.value.link",
    "style.at",
    "style.selector.universal",
    "style.selector.nesting",
    "style.selector.element",
    "style.selector.id",
    "style.selector.id.prefix",
    "style.selector.class",
    "style.selector.class.prefix",
    "style.selector.pseudoclass",
    "style.selector.pseudoclass.prefix",
    "style.selector.pseudoelement",
    "style.selector.pseudoelement.prefix",
    "style.attribute.name",
    "style.value.color.hex",
    "style.value.keyword",
    "style.value.number.unit",
    // Standard tree-sitter names (JavaScript, PHP, JSON queries).
    "attribute",
    "constant",
    "constant.builtin",
    "constructor",
    "escape",
    "function",
    "function.builtin",
    "number",
    "property",
    "punctuation",
    "string.special",
    "type",
    "variable",
    "variable.builtin",
    "variable.parameter",
    "label",
    // Further names used by other grammars.
    "boolean",
    "conditional",
    "repeat",
    "field",
    "float",
    "parameter",
    "method",
    "module",
    "preproc",
    "storageclass",
    "type.qualifier",
    "function.macro",
    "function.method",
    "comment.documentation",
    "text.title",
    "text.literal",
    "text.uri",
    "text.reference",
    "text.strong",
    "text.emphasis",
    "markup.heading",
    "markup.raw",
    "markup.link",
    "markup.bold",
    "markup.italic",
    "markup.quote",
    "markup.list",
    "string.special.regex",
    "string.special.symbol",
    "error",
    "delimiter",
    "addition",
    "deletion",
    "diff.plus",
    "diff.minus",
];

/// Style id used for text that has no highlight.
pub const PLAIN: u8 = u8::MAX;

/// Display-P3 components as used in Nova's theme files, mapped onto sRGB.
fn p3(r: f32, g: f32, b: f32) -> Color32 {
    Color32::from_rgb((r * 255.0) as u8, (g * 255.0) as u8, (b * 255.0) as u8)
}

fn rgba(r: u8, g: u8, b: u8, a: f32) -> Color32 {
    Color32::from_rgba_unmultiplied(r, g, b, (a * 255.0) as u8)
}

fn dark_syntax(name: &str) -> Option<Color32> {
    Some(match name {
        "boolean" | "conditional" | "repeat" | "storageclass" | "type.qualifier" | "string.special.symbol" => p3(0.45, 0.675, 0.9),
        "preproc" => p3(0.48, 0.68, 0.72),
        "field" => p3(0.9, 0.63, 0.81),
        "float" => p3(0.92, 0.828, 0.552),
        "parameter" => p3(0.878, 0.63, 0.9),
        "method" | "function.method" => p3(0.447, 0.898, 0.686),
        "module" => p3(0.36, 0.9, 0.855),
        "function.macro" => p3(0.45, 0.825, 0.9),
        "comment.documentation" | "markup.quote" => p3(0.48, 0.6, 0.72),
        "text.title" | "markup.heading" => p3(1.0, 0.68, 0.4),
        "text.literal" | "markup.raw" => p3(0.85, 0.9, 0.95),
        "text.uri" | "markup.link" => p3(0.5, 0.75, 1.0),
        "text.reference" => p3(0.878, 0.63, 0.9),
        "text.strong" | "markup.bold" => p3(0.4, 0.733, 0.8),
        "text.emphasis" | "markup.italic" => p3(0.4, 0.8, 0.467),
        "markup.list" => p3(0.92, 0.828, 0.552),
        "string.special.regex" => p3(1.0, 0.771, 0.45),
        "error" => p3(1.0, 0.45, 0.35),
        "delimiter" => p3(0.6, 0.7, 0.8),
        "addition" | "diff.plus" => p3(0.45, 0.85, 0.5),
        "deletion" | "diff.minus" => p3(0.95, 0.45, 0.4),
        "comment" => p3(0.48, 0.6, 0.72),
        "processing" => p3(0.48, 0.68, 0.72),
        "invalid" => p3(1.0, 0.45, 0.35),
        "bracket" | "operator" | "punctuation" | "tag.attribute.operator" => p3(0.6, 0.7, 0.8),
        "keyword" | "constant.builtin" | "variable.builtin" | "value.entity" => p3(0.45, 0.675, 0.9),
        "value.number" | "number" | "style.value.color.hex" => p3(0.92, 0.828, 0.552),
        "identifier.type" | "type" | "constructor" => p3(0.36, 0.9, 0.855),
        "identifier.variable" | "constant" => p3(0.765, 0.63, 0.9),
        "identifier.property" | "property" => p3(0.9, 0.63, 0.81),
        "identifier.function" | "function" | "function.builtin" => p3(0.447, 0.898, 0.686),
        "string" | "tag.attribute.value" => p3(0.97, 0.584, 0.534),
        "string.delimiter" | "tag.attribute.value.delimiter" => p3(0.95, 0.405, 0.332),
        "string.escape" | "escape" => p3(1.0, 0.691, 0.65),
        "string.special" => p3(1.0, 0.771, 0.45),
        "tag" => p3(0.35, 0.5, 0.43),
        "tag.name" => p3(0.447, 0.898, 0.686),
        "tag.attribute.name" | "attribute" => p3(0.69, 0.9, 0.54),
        "tag.attribute.value.link" => p3(0.5, 0.75, 1.0),
        "style.at" | "style.selector.id" => p3(1.0, 0.775, 0.5),
        "style.selector.id.prefix" => p3(1.0, 0.865, 0.7),
        "style.selector.universal" | "style.selector.nesting" => p3(0.6, 0.7, 0.8),
        "style.selector.element" => p3(0.855, 0.903, 0.95),
        "style.selector.class" => p3(0.69, 0.9, 0.54),
        "style.selector.class.prefix" => p3(0.854, 1.0, 0.75),
        "style.selector.pseudoclass" => p3(0.9, 0.63, 0.81),
        "style.selector.pseudoclass.prefix" => p3(1.0, 0.82, 0.94),
        "style.selector.pseudoelement" => p3(0.765, 0.63, 0.9),
        "style.selector.pseudoelement.prefix" => p3(0.91, 0.82, 1.0),
        "style.attribute.name" => p3(0.45, 0.825, 0.9),
        "style.value.keyword" | "variable.parameter" | "label" => p3(0.878, 0.63, 0.9),
        "style.value.number.unit" => p3(0.82, 0.7, 0.533),
        _ => return None,
    })
}

fn light_syntax(name: &str) -> Option<Color32> {
    Some(match name {
        "boolean" | "conditional" | "repeat" | "storageclass" | "type.qualifier" | "string.special.symbol" => p3(0.139, 0.371, 0.696),
        "preproc" => p3(0.38, 0.4, 0.392),
        "field" => p3(0.65, 0.227, 0.509),
        "float" => p3(0.85, 0.212, 0.425),
        "parameter" => p3(0.509, 0.227, 0.65),
        "method" | "function.method" => p3(0.275, 0.5, 0.5),
        "module" => p3(0.193, 0.337, 0.35),
        "function.macro" => p3(0.0, 0.45, 0.55),
        "comment.documentation" | "markup.quote" => p3(0.523, 0.534, 0.55),
        "text.title" | "markup.heading" => p3(0.8, 0.4, 0.1),
        "text.literal" | "markup.raw" => p3(0.3, 0.32, 0.35),
        "text.uri" | "markup.link" => p3(0.16, 0.373, 0.8),
        "text.reference" => p3(0.509, 0.227, 0.65),
        "text.strong" | "markup.bold" => p3(0.1, 0.45, 0.55),
        "text.emphasis" | "markup.italic" => p3(0.2, 0.55, 0.25),
        "markup.list" => p3(0.85, 0.212, 0.425),
        "string.special.regex" => p3(0.8, 0.4, 0.1),
        "error" => p3(0.85, 0.15, 0.1),
        "delimiter" => p3(0.22, 0.522, 0.953),
        "addition" | "diff.plus" => p3(0.1, 0.55, 0.2),
        "deletion" | "diff.minus" => p3(0.8, 0.15, 0.1),
        "comment" => p3(0.523, 0.534, 0.55),
        "processing" => p3(0.38, 0.4, 0.392),
        "invalid" => p3(0.85, 0.15, 0.1),
        "bracket" | "operator" | "punctuation" | "tag.attribute.operator"
        | "style.selector.universal" | "style.selector.nesting" => p3(0.22, 0.522, 0.953),
        "keyword" | "constant.builtin" | "variable.builtin" | "value.entity" | "tag.name"
        | "style.attribute.name" => p3(0.139, 0.371, 0.696),
        "value.number" | "number" | "style.value.color.hex" => p3(0.85, 0.212, 0.425),
        "identifier.type" | "type" | "constructor" => p3(0.193, 0.337, 0.35),
        "identifier.variable" | "constant" => p3(0.509, 0.227, 0.65),
        "identifier.property" | "property" | "style.selector.pseudoelement" => p3(0.65, 0.227, 0.509),
        "identifier.function" | "function" | "function.builtin" => p3(0.275, 0.5, 0.5),
        "string" => p3(0.75, 0.24, 0.113),
        "string.delimiter" => p3(0.551, 0.161, 0.083),
        "string.escape" | "escape" | "string.special" => p3(0.8, 0.4, 0.1),
        "tag" => p3(0.32, 0.52, 0.8),
        "tag.attribute.name" | "attribute" => p3(0.227, 0.532, 0.749),
        "tag.attribute.value" | "style.selector.pseudoclass" | "style.value.keyword" => p3(0.456, 0.6, 0.168),
        "tag.attribute.value.delimiter" | "style.selector.pseudoclass.prefix" => p3(0.304, 0.4, 0.112),
        "tag.attribute.value.link" => p3(0.16, 0.373, 0.8),
        "style.at" => p3(0.8, 0.52, 0.24),
        "style.selector.element" => p3(0.15, 0.15, 0.15),
        "style.selector.id" => p3(0.333, 0.24, 0.8),
        "style.selector.id.prefix" => p3(0.208, 0.15, 0.5),
        "style.selector.class" => p3(0.7, 0.473, 0.245),
        "style.selector.class.prefix" => p3(0.45, 0.304, 0.158),
        "style.selector.pseudoelement.prefix" => p3(0.35, 0.122, 0.274),
        "style.value.number.unit" => p3(0.6, 0.151, 0.3),
        "variable.parameter" | "label" => p3(0.509, 0.227, 0.65),
        _ => return None,
    })
}

/// The Windows accent color, if one is configured.
fn system_accent() -> Option<Color32> {
    let key = winreg::RegKey::predef(winreg::enums::HKEY_CURRENT_USER)
        .open_subkey(r"Software\Microsoft\Windows\DWM")
        .ok()?;
    // Stored as 0xAABBGGRR.
    let v: u32 = key.get_value("AccentColor").ok()?;
    Some(Color32::from_rgb(v as u8, (v >> 8) as u8, (v >> 16) as u8))
}

#[derive(Clone)]
pub struct Theme {
    pub name: String,
    pub dark: bool,
    // Document
    pub background: Color32,
    pub foreground: Color32,
    pub gutter_bg: Color32,
    pub gutter_fg: Color32,
    pub gutter_fg_active: Color32,
    pub gutter_active_bg: Color32,
    pub current_line: Color32,
    pub selection: Color32,
    pub selection_inactive: Color32,
    pub cursor: Color32,
    pub find_match: Color32,
    pub bracket_match: Color32,
    /// Other occurrences of the word at the cursor.
    pub word_highlight: Color32,
    pub indent_guide: Color32,
    pub invisible: Color32,
    // Chrome
    pub titlebar_bg: Color32,
    pub sidebar_bg: Color32,
    pub tab_active_bg: Color32,
    pub tab_hover_bg: Color32,
    pub jumpbar_bg: Color32,
    pub separator: Color32,
    pub text_muted: Color32,
    pub text_faint: Color32,
    pub hover: Color32,
    pub row_selected: Color32,
    pub accent: Color32,
    pub mica: bool,
    pub modified: Color32,
    pub danger: Color32,
    pub success: Color32,
    syntax: Vec<Color32>,
}

impl Theme {
    pub fn dark() -> Self {
        let accent = system_accent()
            .map(|c| brighten(c, 0.15))
            .unwrap_or(Color32::from_rgb(10, 132, 255));
        let fg = p3(0.81, 0.855, 0.9);
        let syntax = HIGHLIGHT_NAMES
            .iter()
            .map(|n| dark_syntax(n).unwrap_or(fg))
            .collect();
        Self {
            name: BUILTIN_DARK.into(),
            dark: true,
            background: p3(0.108, 0.114, 0.12),
            foreground: fg,
            gutter_bg: p3(0.108, 0.114, 0.12),
            gutter_fg: p3(0.36, 0.38, 0.4),
            gutter_fg_active: p3(0.717, 0.757, 0.797),
            gutter_active_bg: p3(0.135, 0.143, 0.15),
            current_line: p3(0.135, 0.143, 0.15),
            selection: accent.gamma_multiply(0.38),
            selection_inactive: rgba(140, 150, 170, 0.18),
            cursor: accent,
            find_match: rgba(255, 210, 80, 0.28),
            bracket_match: rgba(255, 255, 255, 0.12),
            word_highlight: rgba(255, 255, 255, 0.07),
            indent_guide: rgba(117, 122, 128, 0.2),
            invisible: rgba(212, 233, 255, 0.15),
            titlebar_bg: Color32::from_rgb(41, 42, 46),
            sidebar_bg: Color32::from_rgb(35, 36, 40),
            tab_active_bg: Color32::from_rgb(62, 64, 70),
            tab_hover_bg: Color32::from_rgb(52, 54, 59),
            jumpbar_bg: p3(0.108, 0.114, 0.12),
            separator: Color32::from_rgb(16, 16, 18),
            text_muted: Color32::from_rgb(160, 165, 172),
            text_faint: Color32::from_rgb(105, 110, 118),
            hover: rgba(255, 255, 255, 0.06),
            row_selected: accent.gamma_multiply(0.55),
            accent,
            mica: false,
            modified: Color32::from_rgb(242, 190, 80),
            danger: Color32::from_rgb(232, 72, 72),
            success: Color32::from_rgb(70, 200, 120),
            syntax,
        }
    }

    pub fn light() -> Self {
        let accent = system_accent().unwrap_or(Color32::from_rgb(0, 122, 255));
        let fg = p3(0.15, 0.15, 0.15);
        let syntax = HIGHLIGHT_NAMES
            .iter()
            .map(|n| light_syntax(n).unwrap_or(fg))
            .collect();
        Self {
            name: BUILTIN_LIGHT.into(),
            dark: false,
            background: Color32::WHITE,
            foreground: fg,
            gutter_bg: Color32::WHITE,
            gutter_fg: p3(0.675, 0.706, 0.75),
            gutter_fg_active: p3(0.36, 0.377, 0.4),
            gutter_active_bg: Color32::from_rgb(250, 250, 240),
            current_line: Color32::from_rgb(255, 255, 236),
            selection: accent.gamma_multiply(0.25),
            selection_inactive: rgba(120, 130, 150, 0.18),
            cursor: accent,
            find_match: rgba(255, 196, 0, 0.35),
            bracket_match: rgba(0, 0, 0, 0.1),
            word_highlight: rgba(0, 0, 0, 0.06),
            indent_guide: rgba(0, 0, 0, 0.1),
            invisible: rgba(48, 50, 51, 0.25),
            titlebar_bg: Color32::from_rgb(232, 232, 235),
            sidebar_bg: Color32::from_rgb(242, 242, 245),
            tab_active_bg: Color32::WHITE,
            tab_hover_bg: Color32::from_rgb(222, 222, 226),
            jumpbar_bg: Color32::WHITE,
            separator: Color32::from_rgb(208, 208, 212),
            text_muted: Color32::from_rgb(95, 99, 106),
            text_faint: Color32::from_rgb(150, 154, 160),
            hover: rgba(0, 0, 0, 0.05),
            row_selected: accent.gamma_multiply(0.3),
            accent,
            mica: false,
            modified: Color32::from_rgb(210, 140, 0),
            danger: Color32::from_rgb(220, 50, 50),
            success: Color32::from_rgb(40, 170, 90),
            syntax,
        }
    }

    pub fn syntax_color(&self, style: u8) -> Color32 {
        self.syntax
            .get(style as usize)
            .copied()
            .unwrap_or(self.foreground)
    }

    /// Color of a highlight name, e.g. `"tag.name"`.
    pub fn color_of(&self, name: &str) -> Color32 {
        HIGHLIGHT_NAMES
            .iter()
            .position(|n| *n == name)
            .map_or(self.foreground, |i| self.syntax_color(i as u8))
    }

    pub fn apply(&self, ctx: &egui::Context) {
        let mut v = if self.dark { Visuals::dark() } else { Visuals::light() };
        v.panel_fill = self.sidebar_bg;
        v.window_fill = if self.dark { Color32::from_rgb(44, 45, 50) } else { Color32::from_rgb(250, 250, 252) };
        v.window_stroke = Stroke::new(1.0, if self.dark { rgba(255, 255, 255, 0.1) } else { rgba(0, 0, 0, 0.12) });
        v.window_shadow = Shadow { offset: [0, 10], blur: 40, spread: 0, color: Color32::from_black_alpha(if self.dark { 140 } else { 50 }) };
        v.popup_shadow = Shadow { offset: [0, 6], blur: 20, spread: 0, color: Color32::from_black_alpha(if self.dark { 120 } else { 40 }) };
        v.window_corner_radius = CornerRadius::same(12);
        v.menu_corner_radius = CornerRadius::same(8);
        v.extreme_bg_color = if self.dark { Color32::from_rgb(28, 29, 32) } else { Color32::WHITE };
        v.text_edit_bg_color = Some(v.extreme_bg_color);
        v.faint_bg_color = self.hover;
        v.selection.bg_fill = self.accent.gamma_multiply(if self.dark { 0.6 } else { 0.35 });
        v.selection.stroke = Stroke::new(1.0, if self.dark { Color32::WHITE } else { self.foreground });
        v.hyperlink_color = self.accent;
        v.widgets.noninteractive.fg_stroke.color = self.text_muted;
        v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, if self.dark { rgba(255, 255, 255, 0.08) } else { rgba(0, 0, 0, 0.1) });

        let (btn, btn_hover, btn_active) = if self.dark {
            (Color32::from_rgb(64, 66, 72), Color32::from_rgb(76, 78, 85), Color32::from_rgb(88, 90, 98))
        } else {
            (Color32::from_rgb(255, 255, 255), Color32::from_rgb(244, 244, 247), Color32::from_rgb(230, 230, 235))
        };
        let border = if self.dark { rgba(255, 255, 255, 0.06) } else { rgba(0, 0, 0, 0.14) };
        for (w, fill) in [
            (&mut v.widgets.inactive, btn),
            (&mut v.widgets.hovered, btn_hover),
            (&mut v.widgets.active, btn_active),
            (&mut v.widgets.open, btn_hover),
        ] {
            w.bg_fill = fill;
            w.weak_bg_fill = fill;
            w.bg_stroke = Stroke::new(1.0, border);
            w.corner_radius = CornerRadius::same(6);
            w.expansion = 0.0;
        }
        v.widgets.inactive.fg_stroke.color = self.foreground;
        v.widgets.hovered.fg_stroke.color = self.foreground;
        v.widgets.active.fg_stroke.color = self.foreground;
        v.widgets.noninteractive.corner_radius = CornerRadius::same(6);

        let (pref, which) = if self.dark {
            (egui::ThemePreference::Dark, egui::Theme::Dark)
        } else {
            (egui::ThemePreference::Light, egui::Theme::Light)
        };
        ctx.set_theme(pref);
        ctx.set_visuals_of(which, v);
        ctx.style_mut_of(which, |s| {
            s.spacing.button_padding = egui::vec2(10.0, 4.0);
            s.spacing.item_spacing = egui::vec2(8.0, 6.0);
            s.spacing.interact_size.y = 26.0;
            s.spacing.menu_margin = Margin::same(6);
            s.spacing.window_margin = Margin::same(18);
            s.spacing.scroll.bar_width = 8.0;
            s.spacing.scroll.floating_allocated_width = 0.0;
            s.text_styles.insert(egui::TextStyle::Body, egui::FontId::proportional(13.5));
            s.text_styles.insert(egui::TextStyle::Button, egui::FontId::proportional(13.5));
            s.text_styles.insert(egui::TextStyle::Heading, egui::FontId::new(19.0, bold_family()));
            s.text_styles.insert(egui::TextStyle::Small, egui::FontId::proportional(11.5));
        });
    }
}

/// Font family for semibold UI text (falls back to the regular UI font).
pub fn bold_family() -> egui::FontFamily {
    egui::FontFamily::Name("bold".into())
}

fn brighten(c: Color32, amount: f32) -> Color32 {
    let f = |x: u8| (x as f32 + (255.0 - x as f32) * amount) as u8;
    Color32::from_rgb(f(c.r()), f(c.g()), f(c.b()))
}

// ---- theme files -----------------------------------------------------------

pub const BUILTIN_DARK: &str = "Nebula Dunkel";
pub const BUILTIN_LIGHT: &str = "Nebula Hell";

/// A theme as stored on disk (JSON). Missing colors fall back to the
/// built-in dark or light theme.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct ThemeFile {
    pub name: String,
    pub dark: bool,
    #[serde(default)]
    pub ui: std::collections::BTreeMap<String, String>,
    #[serde(default)]
    pub syntax: std::collections::BTreeMap<String, String>,
    /// Name of the installed extension the theme comes from.
    #[serde(skip)]
    pub extension: Option<String>,
    /// A user theme that replaces an extension theme of the same name.
    #[serde(skip)]
    pub overrides_extension: bool,
}

impl ThemeFile {
    pub fn is_builtin(&self) -> bool {
        self.name == BUILTIN_DARK || self.name == BUILTIN_LIGHT
    }
}

/// Editable interface colors: (key, German label).
pub const UI_COLORS: &[(&str, &str)] = &[
    ("background", tk!("Editor-Hintergrund")),
    ("foreground", tk!("Text")),
    ("current_line", tk!("Aktuelle Zeile")),
    ("selection", tk!("Auswahl")),
    ("cursor", tk!("Cursor")),
    ("gutter_fg", tk!("Zeilennummern")),
    ("gutter_fg_active", tk!("Aktive Zeilennummer")),
    ("gutter_active_bg", tk!("Aktive Zeile (Rand)")),
    ("indent_guide", tk!("Einrückungslinien")),
    ("invisible", tk!("Unsichtbare Zeichen")),
    ("find_match", tk!("Suchtreffer")),
    ("bracket_match", tk!("Passende Klammer")),
    ("word_highlight", tk!("Gleiches Wort")),
    ("titlebar_bg", tk!("Titelleiste")),
    ("sidebar_bg", tk!("Seitenleiste")),
    ("tab_active_bg", tk!("Aktiver Tab")),
    ("tab_hover_bg", tk!("Tab bei Mausberührung")),
    ("jumpbar_bg", tk!("Pfadleiste")),
    ("separator", tk!("Trennlinien")),
    ("text_muted", tk!("Gedämpfter Text")),
    ("text_faint", tk!("Schwacher Text")),
    ("row_selected", tk!("Ausgewählte Zeile (Seitenleiste)")),
    ("accent", tk!("Akzentfarbe")),
    ("modified", tk!("Geändert-Markierung")),
    ("danger", tk!("Fehler")),
    ("success", tk!("Erfolg")),
];

pub fn color_to_hex(c: Color32) -> String {
    let [r, g, b, a] = c.to_srgba_unmultiplied();
    if a == 255 {
        format!("#{r:02x}{g:02x}{b:02x}")
    } else {
        format!("#{r:02x}{g:02x}{b:02x}{a:02x}")
    }
}

pub fn color_from_hex(s: &str) -> Option<Color32> {
    let h = s.trim().trim_start_matches('#');
    let v = |i: usize| u8::from_str_radix(h.get(i..i + 2)?, 16).ok();
    let short = |i: usize| u8::from_str_radix(h.get(i..i + 1)?, 16).ok().map(|x| x * 17);
    match h.len() {
        3 => Some(Color32::from_rgb(short(0)?, short(1)?, short(2)?)),
        6 => Some(Color32::from_rgb(v(0)?, v(2)?, v(4)?)),
        8 => Some(Color32::from_rgba_unmultiplied(v(0)?, v(2)?, v(4)?, v(6)?)),
        _ => None,
    }
}

impl Theme {
    pub fn ui_color_mut(&mut self, key: &str) -> Option<&mut Color32> {
        Some(match key {
            "background" => &mut self.background,
            "foreground" => &mut self.foreground,
            "current_line" => &mut self.current_line,
            "selection" => &mut self.selection,
            "cursor" => &mut self.cursor,
            "gutter_fg" => &mut self.gutter_fg,
            "gutter_fg_active" => &mut self.gutter_fg_active,
            "gutter_active_bg" => &mut self.gutter_active_bg,
            "indent_guide" => &mut self.indent_guide,
            "invisible" => &mut self.invisible,
            "find_match" => &mut self.find_match,
            "bracket_match" => &mut self.bracket_match,
            "word_highlight" => &mut self.word_highlight,
            "titlebar_bg" => &mut self.titlebar_bg,
            "sidebar_bg" => &mut self.sidebar_bg,
            "tab_active_bg" => &mut self.tab_active_bg,
            "tab_hover_bg" => &mut self.tab_hover_bg,
            "jumpbar_bg" => &mut self.jumpbar_bg,
            "separator" => &mut self.separator,
            "text_muted" => &mut self.text_muted,
            "text_faint" => &mut self.text_faint,
            "row_selected" => &mut self.row_selected,
            "accent" => &mut self.accent,
            "modified" => &mut self.modified,
            "danger" => &mut self.danger,
            "success" => &mut self.success,
            _ => return None,
        })
    }

    pub fn from_file(file: &ThemeFile) -> Self {
        let mut t = if file.dark { Theme::dark() } else { Theme::light() };
        for (key, value) in &file.ui {
            if let (Some(slot), Some(c)) = (t.ui_color_mut(key), color_from_hex(value)) {
                *slot = c;
            }
        }
        t.gutter_bg = t.background;
        for (key, value) in &file.syntax {
            if let (Some(i), Some(c)) = (HIGHLIGHT_NAMES.iter().position(|n| n == key), color_from_hex(value)) {
                t.syntax[i] = c;
            }
        }
        t.name = file.name.clone();
        t
    }

    /// Complete description of this theme, e.g. for export.
    pub fn to_file(&self, name: &str) -> ThemeFile {
        let mut copy = self.clone();
        let ui = UI_COLORS
            .iter()
            .filter_map(|(k, _)| Some((k.to_string(), color_to_hex(*copy.ui_color_mut(k)?))))
            .collect();
        let syntax = HIGHLIGHT_NAMES
            .iter()
            .enumerate()
            .map(|(i, n)| (n.to_string(), color_to_hex(self.syntax[i])))
            .collect();
        ThemeFile { name: name.to_string(), dark: self.dark, ui, syntax, extension: None, overrides_extension: false }
    }

    /// The theme selected in the settings, with Mica transparency applied.
    pub fn for_settings(settings: &crate::settings::AppSettings) -> Self {
        let dark = settings.is_dark();
        let wanted = settings.active_theme();
        let mut t = load_themes()
            .iter()
            .find(|f| f.name == wanted)
            .map(Theme::from_file)
            .unwrap_or_else(|| if dark { Theme::dark() } else { Theme::light() });
        if settings.mica {
            // Let the backdrop shine through the sidebar, slightly tinted.
            let c = t.sidebar_bg;
            let a = if t.dark { 0.55 } else { 0.5 };
            t.sidebar_bg = Color32::from_rgba_unmultiplied(c.r(), c.g(), c.b(), (a * 255.0) as u8);
        }
        t.mica = settings.mica;
        t
    }
}

/// Bumped when installed extensions change, so theme lists reload.
static THEMES_GEN: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

pub fn themes_changed() {
    THEMES_GEN.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
}

pub fn themes_generation() -> u64 {
    THEMES_GEN.load(std::sync::atomic::Ordering::Relaxed)
}

fn themes_dir() -> std::path::PathBuf {
    crate::settings::config_dir().join("themes")
}

/// Built-in themes followed by the user's themes and those of installed
/// extensions. A user theme with the name of an extension theme replaces it
/// (that is where edits to an extension theme are saved).
pub fn load_themes() -> Vec<ThemeFile> {
    let mut out = vec![Theme::dark().to_file(BUILTIN_DARK), Theme::light().to_file(BUILTIN_LIGHT)];
    let mut user: Vec<ThemeFile> = std::fs::read_dir(themes_dir())
        .map(|rd| {
            rd.flatten()
                .filter(|e| e.path().extension().is_some_and(|x| x == "json"))
                .filter_map(|e| serde_json::from_slice(&std::fs::read(e.path()).ok()?).ok())
                .filter(|f: &ThemeFile| !f.is_builtin())
                .collect()
        })
        .unwrap_or_default();
    for ext in extension_themes() {
        match user.iter_mut().find(|u| u.name == ext.name) {
            Some(u) => u.overrides_extension |= u.extension.is_none(),
            None => user.push(ext),
        }
    }
    user.sort_by_key(|t| t.name.to_lowercase());
    out.extend(user);
    out
}

/// Nova themes (`Themes/*.css`) of the installed extensions.
fn extension_themes() -> Vec<ThemeFile> {
    let mut out = Vec::new();
    let Ok(exts) = std::fs::read_dir(crate::ext::manifest::extensions_dir()) else { return out };
    for ext in exts.flatten() {
        let Ok(files) = std::fs::read_dir(ext.path().join("Themes")) else { continue };
        let ext_name = std::fs::read(ext.path().join("extension.json"))
            .ok()
            .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok())
            .and_then(|v| v.get("name").and_then(|n| n.as_str()).map(str::to_string))
            .unwrap_or_else(|| ext.file_name().to_string_lossy().into_owned());
        for f in files.flatten() {
            let path = f.path();
            if !path.extension().is_some_and(|x| x.eq_ignore_ascii_case("css")) {
                continue;
            }
            let Ok(css) = std::fs::read_to_string(&path) else { continue };
            let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
            if let Ok(file) = crate::theme_import::import_nova_css(&css, &stem)
                && !file.is_builtin()
            {
                // Colors the theme leaves out come from the base theme.
                let mut file = Theme::from_file(&file).to_file(&file.name);
                file.extension = Some(ext_name.clone());
                out.push(file);
            }
        }
    }
    out
}

fn theme_path(name: &str) -> std::path::PathBuf {
    let safe: String = name
        .chars()
        .map(|c| if c.is_alphanumeric() || c == ' ' || c == '-' || c == '_' { c } else { '_' })
        .collect();
    themes_dir().join(format!("{}.json", safe.trim()))
}

pub fn save_theme(file: &ThemeFile) -> anyhow::Result<()> {
    std::fs::create_dir_all(themes_dir())?;
    std::fs::write(theme_path(&file.name), serde_json::to_vec_pretty(file)?)?;
    Ok(())
}

pub fn delete_theme(name: &str) {
    let _ = std::fs::remove_file(theme_path(name));
}
