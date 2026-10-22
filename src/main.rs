#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod acp;
mod app;
mod editor;
mod editorconfig;
mod diff;
mod highlight;
mod history;
mod hex_view;
mod image_view;
mod fonts;
mod icon;
mod ext;
mod lang;
mod preview;
mod print;
mod crash;
mod remote;
mod server_import;
mod snippet_io;
mod sync;
mod settings;
mod settings_io;
mod theme;
mod theme_import;
mod ui;
mod win32;

use std::path::PathBuf;

use eframe::egui;


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
    crash::install();
    // Checks the crash report (also in release builds, e.g. for symbol names).
    if std::env::var_os("NEBULA_CRASH_TEST").is_some() {
        panic!("Absturztest (NEBULA_CRASH_TEST)");
    }
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
            cc.egui_ctx.options_mut(|o| o.zoom_with_keyboard = false);
            Ok(Box::new(app::NebulaApp::new(cc, open_paths)))
        }),
    )
}
