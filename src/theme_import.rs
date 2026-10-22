//! Import of Nova theme stylesheets (`*.css` from `.novaextension/Themes`).

use std::collections::BTreeMap;

use anyhow::{Result, bail};
use eframe::egui::Color32;

use crate::theme::{HIGHLIGHT_NAMES, ThemeFile, color_to_hex};

/// Parses a CSS color as used in Nova themes.
pub fn parse_color(value: &str) -> Option<Color32> {
    let v = value.trim().trim_end_matches(';').trim();
    let v = v.split("!important").next()?.trim();
    if let Some(inner) = v.strip_prefix("color(").and_then(|s| s.strip_suffix(')')) {
        // color(display-p3 r g b / a)
        let mut parts = inner.split('/');
        let rgb: Vec<f32> = parts
            .next()?
            .split_whitespace()
            .skip(1)
            .filter_map(|x| x.parse().ok())
            .collect();
        let a: f32 = parts.next().and_then(|x| x.trim().parse().ok()).unwrap_or(1.0);
        if rgb.len() != 3 {
            return None;
        }
        let c = |x: f32| (x.clamp(0.0, 1.0) * 255.0).round() as u8;
        return Some(Color32::from_rgba_unmultiplied(c(rgb[0]), c(rgb[1]), c(rgb[2]), c(a)));
    }
    if let Some(inner) = v.strip_prefix("rgba(").or_else(|| v.strip_prefix("rgb(")).and_then(|s| s.strip_suffix(')')) {
        let nums: Vec<f32> = inner
            .split([',', ' ', '/'])
            .filter(|s| !s.is_empty())
            .filter_map(|x| x.trim_end_matches('%').parse().ok())
            .collect();
        if nums.len() >= 3 {
            let a = nums.get(3).map_or(255, |a| if *a <= 1.0 { (a * 255.0) as u8 } else { *a as u8 });
            return Some(Color32::from_rgba_unmultiplied(nums[0] as u8, nums[1] as u8, nums[2] as u8, a));
        }
        return None;
    }
    if let Some(inner) = v.strip_prefix("hsla(").or_else(|| v.strip_prefix("hsl(")).and_then(|s| s.strip_suffix(')')) {
        let nums: Vec<f32> = inner
            .split([',', ' ', '/'])
            .filter(|s| !s.is_empty())
            .filter_map(|x| x.trim_end_matches("deg").trim_end_matches('%').parse().ok())
            .collect();
        if nums.len() < 3 {
            return None;
        }
        let (h, s, l) = (nums[0].rem_euclid(360.0) / 360.0, nums[1] / 100.0, nums[2] / 100.0);
        let a = nums.get(3).map_or(1.0, |a| if *a > 1.0 { a / 100.0 } else { *a });
        let q = if l < 0.5 { l * (1.0 + s) } else { l + s - l * s };
        let p = 2.0 * l - q;
        let hue = |mut t: f32| {
            t = t.rem_euclid(1.0);
            if t < 1.0 / 6.0 {
                p + (q - p) * 6.0 * t
            } else if t < 0.5 {
                q
            } else if t < 2.0 / 3.0 {
                p + (q - p) * (2.0 / 3.0 - t) * 6.0
            } else {
                p
            }
        };
        let c = |x: f32| (x.clamp(0.0, 1.0) * 255.0).round() as u8;
        return Some(Color32::from_rgba_unmultiplied(
            c(hue(h + 1.0 / 3.0)),
            c(hue(h)),
            c(hue(h - 1.0 / 3.0)),
            c(a),
        ));
    }
    if v.starts_with('#') {
        return crate::theme::color_from_hex(v);
    }
    match v {
        "white" => Some(Color32::WHITE),
        "black" => Some(Color32::BLACK),
        "transparent" => Some(Color32::TRANSPARENT),
        _ => None,
    }
}

/// Our standard tree-sitter names taken from the closest Nova scope.
const ALIASES: &[(&str, &str)] = &[
    ("attribute", "tag.attribute.name"),
    ("constant", "identifier.constant"),
    ("constant.builtin", "value.null"),
    ("constructor", "identifier.type"),
    ("escape", "string.escape"),
    ("function", "identifier.function"),
    ("function.builtin", "identifier.function"),
    ("function.method", "identifier.method"),
    ("method", "identifier.method"),
    ("number", "value.number"),
    ("float", "value.number"),
    ("boolean", "value.boolean"),
    ("property", "identifier.property"),
    ("field", "identifier.property"),
    ("punctuation", "bracket"),
    ("delimiter", "operator"),
    ("string.special", "regex"),
    ("string.special.regex", "regex"),
    ("type", "identifier.type"),
    ("module", "identifier.type"),
    ("variable.builtin", "keyword.self"),
    ("variable.parameter", "identifier.argument"),
    ("parameter", "identifier.argument"),
    ("function.macro", "identifier.decorator"),
    ("preproc", "processing"),
    ("conditional", "keyword.condition"),
    ("repeat", "keyword.statement"),
    ("storageclass", "keyword.modifier"),
    ("type.qualifier", "keyword.modifier"),
    ("label", "identifier.key"),
    ("text.title", "markup.heading"),
    ("text.literal", "markup.code"),
    ("markup.raw", "markup.code"),
    ("text.uri", "markup.link"),
    ("text.reference", "markup.link-label"),
    ("text.strong", "markup.bold"),
    ("text.emphasis", "markup.italic"),
    ("markup.list", "markup.list.item"),
    ("markup.quote", "comment"),
    ("comment.documentation", "comment.doctag"),
    ("error", "invalid"),
];

/// Nova scope -> our interface color.
const UI_MAP: &[(&str, &str, &[&str])] = &[
    ("meta.document", "background-color", &["background", "jumpbar_bg", "tab_active_bg"]),
    ("meta.text", "color", &["foreground"]),
    ("meta.text.invisible", "color", &["invisible"]),
    ("meta.cursor", "background-color", &["current_line", "gutter_active_bg"]),
    ("meta.gutter", "color", &["gutter_fg"]),
    ("meta.gutter.selected", "color", &["gutter_fg_active"]),
    ("meta.indentguide", "border-color", &["indent_guide"]),
];

pub fn import_nova_css(css: &str, fallback_name: &str) -> Result<ThemeFile> {
    // Strip comments.
    let mut text = String::with_capacity(css.len());
    let mut rest = css;
    while let Some(i) = rest.find("/*") {
        text.push_str(&rest[..i]);
        rest = rest[i + 2..].split_once("*/").map_or("", |(_, r)| r);
    }
    text.push_str(rest);

    // selector -> property -> value
    let mut rules: BTreeMap<String, BTreeMap<String, String>> = BTreeMap::new();
    let mut rest = text.as_str();
    while let Some(open) = rest.find('{') {
        let selectors = &rest[..open];
        let Some(close) = rest[open..].find('}') else { break };
        let body = &rest[open + 1..open + close];
        for sel in selectors.split(',') {
            let sel = sel.trim().to_string();
            let entry = rules.entry(sel).or_default();
            for decl in body.split(';') {
                if let Some((k, v)) = decl.split_once(':') {
                    entry.insert(k.trim().to_string(), v.trim().to_string());
                }
            }
        }
        rest = &rest[open + close + 1..];
    }
    if rules.is_empty() {
        bail!(t!("Keine CSS-Regeln gefunden – ist das ein Nova-Theme?"));
    }

    let meta = rules.get("meta").cloned().unwrap_or_default();
    let name = meta
        .get("-theme-display-name")
        .map(|n| n.trim_matches('"').to_string())
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| fallback_name.to_string());
    let dark = meta
        .get("-theme-interface-style")
        .map(|s| s.contains("dark"))
        .unwrap_or_else(|| {
            // Guess from the document background.
            rules
                .get("meta.document")
                .and_then(|r| r.get("background-color"))
                .and_then(|c| parse_color(c))
                .is_some_and(|c| (c.r() as u32 + c.g() as u32 + c.b() as u32) < 384)
        });

    let color = |sel: &str, prop: &str| rules.get(sel).and_then(|r| r.get(prop)).and_then(|v| parse_color(v));

    let mut ui = BTreeMap::new();
    for (sel, prop, keys) in UI_MAP {
        if let Some(c) = color(sel, prop) {
            for k in *keys {
                ui.insert(k.to_string(), color_to_hex(c));
            }
        }
    }
    // Derive the chrome from the document color.
    if let Some(bg) = color("meta.document", "background-color") {
        let shade = |f: f32| {
            let m = |x: u8| (x as f32 * f).clamp(0.0, 255.0) as u8;
            color_to_hex(Color32::from_rgb(m(bg.r()), m(bg.g()), m(bg.b())))
        };
        if dark {
            ui.insert("sidebar_bg".into(), shade(1.18));
            ui.insert("titlebar_bg".into(), shade(1.35));
            ui.insert("tab_hover_bg".into(), shade(1.7));
        } else {
            ui.insert("sidebar_bg".into(), shade(0.96));
            ui.insert("titlebar_bg".into(), shade(0.92));
            ui.insert("tab_hover_bg".into(), shade(0.88));
        }
    }

    // Syntax: Nova scope names first, then aliases. A scope without its own
    // color inherits from its parent (e.g. `tag.attribute` from `tag`).
    let syntax_color = |scope: &str| -> Option<Color32> {
        let mut s = scope;
        loop {
            if let Some(c) = color(s, "color") {
                return Some(c);
            }
            s = s.rsplit_once('.')?.0;
        }
    };
    let mut syntax = BTreeMap::new();
    for name in HIGHLIGHT_NAMES {
        let alias = ALIASES.iter().find(|(n, _)| n == name).map(|(_, a)| *a);
        let c = rules
            .contains_key(*name)
            .then(|| syntax_color(name))
            .flatten()
            .or_else(|| alias.and_then(syntax_color))
            .or_else(|| syntax_color(name));
        if let Some(c) = c {
            syntax.insert(name.to_string(), color_to_hex(c));
        }
    }
    if syntax.is_empty() && ui.is_empty() {
        bail!(t!("Das Stylesheet enthält keine bekannten Nova-Theme-Farben."));
    }
    Ok(ThemeFile { name, dark, ui, syntax, extension: None, overrides_extension: false })
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"
meta { -theme-display-name: "Test Dark"; -theme-interface-style: dark; }
/* comment */
meta.document { background-color: color(display-p3 0.108 0.114 0.12); }
meta.text { color: color(display-p3 0.81 0.855 0.9); }
comment { color: #7a99b8; }
keyword, keyword.condition { color: rgb(115, 172, 230); }
tag { color: #112233; }
tag.name { color: color(display-p3 0.447 0.898 0.686 / 0.5); }
identifier.function { color: #72e5af; }
"#;

    #[test]
    fn imports_nova_css() {
        let t = import_nova_css(SAMPLE, "x").unwrap();
        assert_eq!(t.name, "Test Dark");
        assert!(t.dark);
        assert_eq!(t.ui["background"], "#1c1d1f");
        assert_eq!(t.syntax["comment"], "#7a99b8");
        assert_eq!(t.syntax["keyword"], "#73ace6");
        assert_eq!(t.syntax["conditional"], "#73ace6");
        assert_eq!(t.syntax["function"], "#72e5af");
        assert_eq!(t.syntax["tag.name"], "#72e5af80");
        // Inherits from the parent scope.
        assert_eq!(t.syntax["tag.attribute.name"], "#112233");
    }

    #[test]
    fn imports_bundled_nova_themes_if_present() {
        let dir = std::path::Path::new(
            "../Nova.app/Contents/SharedSupport/Extensions/Themes.novaextension/Themes",
        );
        let Ok(rd) = std::fs::read_dir(dir) else { return };
        for e in rd.flatten() {
            let css = std::fs::read_to_string(e.path()).unwrap();
            let t = import_nova_css(&css, "x").unwrap();
            assert!(t.syntax.len() > 30, "{}: only {} colors", t.name, t.syntax.len());
        }
    }
}

