//! Fonts: the UI fonts from Windows and the editor font, which can be any
//! installed font (Settings → Editor).

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Arc, OnceLock};

use eframe::egui::{self, FontData, FontDefinitions, FontFamily};

use crate::theme;

/// Style words that are not part of a family name in the registry.
const STYLES: &[&str] = &[
    "Regular", "Bold", "Italic", "Oblique", "Light", "Semilight", "SemiLight", "Semibold", "SemiBold", "Demibold", "Medium", "Black",
    "Heavy", "Thin", "ExtraLight", "Extralight", "UltraLight", "ExtraBold", "Extrabold", "UltraBold", "Book", "Condensed", "Narrow",
    // German Windows names some styles in German.
    "Fett", "Kursiv", "Halbfett", "Leicht", "Schmal", "Mager", "Standard", "Normal",
];

fn fonts_dir() -> PathBuf {
    let windir = std::env::var("WINDIR").unwrap_or_else(|_| r"C:\Windows".into());
    PathBuf::from(windir).join("Fonts")
}

/// Installed font families and the file of their regular style, for the picker.
pub fn installed() -> &'static BTreeMap<String, PathBuf> {
    static LIST: OnceLock<BTreeMap<String, PathBuf>> = OnceLock::new();
    LIST.get_or_init(|| {
        use winreg::RegKey;
        use winreg::enums::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE};
        const KEY: &str = r"SOFTWARE\Microsoft\Windows NT\CurrentVersion\Fonts";
        // (family, is regular) -> file; the regular style wins.
        let mut found: BTreeMap<String, (bool, PathBuf)> = BTreeMap::new();
        for root in [HKEY_LOCAL_MACHINE, HKEY_CURRENT_USER] {
            let Ok(key) = RegKey::predef(root).open_subkey(KEY) else { continue };
            for (name, value) in key.enum_values().flatten() {
                let file = value.to_string();
                // Collections need an index; skip them.
                let lower = file.to_lowercase();
                if !(lower.ends_with(".ttf") || lower.ends_with(".otf")) {
                    continue;
                }
                let path = if PathBuf::from(&file).is_absolute() { PathBuf::from(&file) } else { fonts_dir().join(&file) };
                let label = name.split(" (").next().unwrap_or(&name).trim();
                let words: Vec<&str> = label.split_whitespace().collect();
                let family_words: Vec<&str> = words.iter().copied().take_while(|w| !STYLES.contains(w)).collect();
                if family_words.is_empty() {
                    continue;
                }
                let regular = words.len() == family_words.len() || words[family_words.len()..] == ["Regular"];
                let family = family_words.join(" ");
                match found.get(&family) {
                    Some((true, _)) => {}
                    Some((false, _)) if !regular => {}
                    _ => {
                        found.insert(family, (regular, path));
                    }
                }
            }
        }
        found.into_iter().map(|(family, (_, path))| (family, path)).collect()
    })
}

/// Sets up all fonts; `code_family` is the editor font ("" = the default).
pub fn apply(ctx: &egui::Context, code_family: &str) {
    let mut fonts = FontDefinitions::default();
    let dir = fonts_dir();
    let mut load = |name: &str, files: &[PathBuf]| -> bool {
        for file in files {
            if let Ok(bytes) = std::fs::read(file) {
                fonts.font_data.insert(name.into(), Arc::new(FontData::from_owned(bytes)));
                return true;
            }
        }
        false
    };
    let chosen: Vec<PathBuf> = installed().get(code_family).cloned().into_iter().collect();
    let defaults = [dir.join("CascadiaMono.ttf"), dir.join("consola.ttf")];
    // The chosen font first; the default stays behind it for missing glyphs.
    let custom = !chosen.is_empty() && load("code-custom", &chosen);
    let code = load("code", &defaults);
    let ui = load("ui", &[dir.join("segoeui.ttf")]);
    let semibold = load("ui-semibold", &[dir.join("seguisb.ttf"), dir.join("segoeuib.ttf")]);
    let symbols = load("symbols", &[dir.join("seguisym.ttf")]);

    let defaults_prop = fonts.families[&FontFamily::Proportional].clone();
    let prop = fonts.families.get_mut(&FontFamily::Proportional).unwrap();
    if ui {
        prop.insert(0, "ui".into());
    }
    if symbols {
        prop.push("symbols".into());
    }
    let mono = fonts.families.get_mut(&FontFamily::Monospace).unwrap();
    if code {
        mono.insert(0, "code".into());
    }
    if custom {
        mono.insert(0, "code-custom".into());
    }
    if symbols {
        mono.push("symbols".into());
    }
    // Semibold family for headings and labels; always registered so it can be used safely.
    let mut bold = defaults_prop;
    if semibold {
        bold.insert(0, "ui-semibold".into());
    } else if ui {
        bold.insert(0, "ui".into());
    }
    if symbols {
        bold.push("symbols".into());
    }
    fonts.families.insert(theme::bold_family(), bold);
    ctx.set_fonts(fonts);
}

#[cfg(test)]
mod tests {
    use eframe::egui::{self, FontData, FontDefinitions, FontFamily, FontId};

    /// Ligatures come from egui's shaping (harfrust): a ligature font joins
    /// "=>" into one glyph, the second char gets a zero-width continuation.
    #[test]
    #[ignore]
    fn ligature_font_joins_arrows() {
        let ctx = egui::Context::default();
        let mut fonts = FontDefinitions::default();
        let bytes = std::fs::read(r"C:\Windows\Fonts\CascadiaCode.ttf").expect("Cascadia Code");
        fonts.font_data.insert("lig".into(), std::sync::Arc::new(FontData::from_owned(bytes)));
        fonts.families.get_mut(&FontFamily::Monospace).unwrap().insert(0, "lig".into());
        ctx.set_fonts(fonts);
        let mut widths: Vec<(char, f32)> = Vec::new();
        // The picture of each glyph: the same chars look different inside a ligature.
        let mut joined = Vec::new();
        let mut apart = Vec::new();
        let mut split = Vec::new();
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
            let layout = |t: &str| ui.fonts_mut(|f| f.layout_no_wrap(t.into(), FontId::monospace(14.0), egui::Color32::WHITE));
            let galley = layout("a => b");
            widths = galley.rows[0].glyphs.iter().map(|g| (g.chr, g.advance_width)).collect();
            joined = galley.rows[0].glyphs.iter().map(|g| g.uv_rect.min).collect::<Vec<_>>();
            apart = layout("a =x b").rows[0].glyphs.iter().map(|g| g.uv_rect.min).collect::<Vec<_>>();
            // One section per char, as the editor does with ligatures off.
            let mut job = egui::text::LayoutJob::default();
            for ch in "a => b".chars() {
                let start = job.text.len();
                job.text.push(ch);
                let format = egui::TextFormat::simple(FontId::monospace(14.0), egui::Color32::WHITE);
                job.sections.push(egui::text::LayoutSection { leading_space: 0.0, byte_range: start.into()..job.text.len().into(), format });
            }
            split = ui.fonts_mut(|f| f.layout_job(job)).rows[0].glyphs.iter().map(|g| g.uv_rect.min).collect::<Vec<_>>();
        });
        out.textures_delta.clear();
        // '=' in "=>" is drawn differently than a lone '='.
        assert_ne!(joined[2], apart[2], "keine Ligatur");
        // Split into sections: a plain '=' again.
        assert_eq!(split[2], apart[2]);
        let total: f32 = widths.iter().map(|w| w.1).sum();
        let a = widths[0].1;
        // Still a grid: six cells wide in total.
        assert!((total - 6.0 * a).abs() < 0.5, "{widths:?}");
    }
}
