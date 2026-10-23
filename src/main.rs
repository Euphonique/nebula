#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

#[macro_use]
mod i18n;
mod acp;
mod agent;
mod app;
mod editor;
mod editorconfig;
mod diff;
mod highlight;
mod history;
mod hex_view;
mod db_view;
mod image_view;
mod fonts;
mod icon;
mod integration;
mod ext;
mod lang;
mod mcp;
mod preview;
mod perf;
mod print;
mod crash;
mod remote;
mod server_import;
mod offline;
mod update;
mod snippet_io;
mod spell;
mod sync;
mod settings;
mod safe_write;
mod shared_json;
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
    match std::env::var("NEBULA_CRASH_TEST").as_deref() {
        // A hard crash in native code (access violation).
        Ok("native") => crash::native_crash_test(),
        Ok(_) => panic!("Absturztest (NEBULA_CRASH_TEST)"),
        Err(_) => {}
    }
    let args: Vec<std::ffi::OsString> = std::env::args_os().skip(1).collect();
    // "Deinstallieren" under Windows' "Apps".
    if args.iter().any(|a| a == "--uninstall") {
        // The message in the user's language.
        i18n::load();
        i18n::set_language(&settings::AppSettings::load().ui_language);
        let text = match integration::uninstall() {
            Ok(()) => t!("Nebula wurde entfernt.\n\nDeine Einstellungen bleiben im Ordner %APPDATA%\\nebula erhalten.").to_string(),
            Err(e) => t!("Nebula konnte nicht vollständig entfernt werden:\n{e}", e = format!("{:#}", e)),
        };
        win32::message_box("Nebula", &text);
        return Ok(());
    }
    let open_paths: Vec<PathBuf> = args.iter().filter(|a| !a.to_string_lossy().starts_with("--")).map(PathBuf::from).collect();
    // "Neues Fenster": a further window with a project of its own.
    let window = args.iter().any(|a| a == "--window");
    if window {
        settings::set_secondary();
    }
    // A second start hands its files to the running window ("Mit Nebula öffnen").
    let forwarded = !window && !args.iter().any(|a| a == "--new-instance") && win32::forward_to_running(&open_paths);
    if forwarded {
        return Ok(());
    }
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
        // Further windows open where Windows puts new windows (slightly
        // offset), not exactly on top of the main window.
        persist_window: !window,
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
