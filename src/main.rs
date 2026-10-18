#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod editor;
mod editorconfig;
mod diff;
mod highlight;
mod history;
mod icon;
mod ext;
mod lang;
mod preview;
mod print;
mod remote;
mod settings;
mod theme;
mod theme_import;
mod ui;
mod win32;

use std::path::PathBuf;
use std::sync::Arc;

use eframe::egui::{self, FontData, FontDefinitions, FontFamily};

fn load_fonts(ctx: &egui::Context) {
    let mut fonts = FontDefinitions::default();
    let windir = std::env::var("WINDIR").unwrap_or_else(|_| r"C:\Windows".into());
    let fonts_dir = PathBuf::from(windir).join("Fonts");
    let mut load = |name: &str, files: &[&str]| -> bool {
        for file in files {
            if let Ok(bytes) = std::fs::read(fonts_dir.join(file)) {
                fonts
                    .font_data
                    .insert(name.into(), Arc::new(FontData::from_owned(bytes)));
                return true;
            }
        }
        false
    };
    let code = load("code", &["CascadiaMono.ttf", "consola.ttf"]);
    let ui = load("ui", &["segoeui.ttf"]);
    let semibold = load("ui-semibold", &["seguisb.ttf", "segoeuib.ttf"]);
    let symbols = load("symbols", &["seguisym.ttf"]);

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

/// DirectX 12 presenting through a DirectComposition visual: keeps the window
/// transparent for Mica and lets native child windows (the preview web view)
/// show above the rendered UI. Vulkan swapchains would cover them.
fn wgpu_options() -> eframe::egui_wgpu::WgpuConfiguration {
    use eframe::egui_wgpu::{WgpuConfiguration, WgpuSetup, WgpuSetupCreateNew};
    use eframe::wgpu;
    let mut setup = WgpuSetupCreateNew::without_display_handle();
    if std::env::var_os("WGPU_BACKEND").is_none() {
        setup.instance_descriptor.backends = wgpu::Backends::DX12 | wgpu::Backends::GL;
    }
    setup.instance_descriptor.backend_options.dx12.presentation_system = wgpu::Dx12SwapchainKind::DxgiFromVisual;
    WgpuConfiguration {
        wgpu_setup: WgpuSetup::CreateNew(setup),
        ..Default::default()
    }
}

fn main() -> eframe::Result {
    let open_paths: Vec<PathBuf> = std::env::args_os().skip(1).map(PathBuf::from).collect();
    let icon = egui::IconData {
        rgba: icon::render_icon(64),
        width: 64,
        height: 64,
    };
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title(settings::APP_NAME)
            .with_inner_size([1280.0, 820.0])
            .with_min_inner_size([720.0, 440.0])
            .with_decorations(true)
            .with_transparent(true)
            .with_icon(icon)
            .with_drag_and_drop(true),
        persist_window: true,
        wgpu_options: wgpu_options(),
        ..Default::default()
    };
    eframe::run_native(
        settings::APP_NAME,
        options,
        Box::new(move |cc| {
            load_fonts(&cc.egui_ctx);
            cc.egui_ctx.options_mut(|o| o.zoom_with_keyboard = false);
            Ok(Box::new(app::NebulaApp::new(cc, open_paths)))
        }),
    )
}
