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
#[cfg(windows)]
mod file_icons;
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
mod server_log;
mod server_watch;
mod html_locate;
mod sql_assist;
mod optimize;
mod css_rename;
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
mod platform;

use std::path::PathBuf;

use eframe::egui;


/// DirectX 12 presenting through a DirectComposition visual: keeps the window
/// transparent for Mica and lets native child windows (the preview web view)
/// show above the rendered UI. Vulkan swapchains would cover them.
#[cfg(windows)]
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

/// Linux: wgpu's defaults (Vulkan, else OpenGL); the preview web view is an
/// X11 child window above the rendered UI.
#[cfg(not(windows))]
fn wgpu_options() -> eframe::egui_wgpu::WgpuConfiguration {
    eframe::egui_wgpu::WgpuConfiguration::default()
}

/// X11 for the window and GTK, also under Wayland (XWayland): the preview web
/// view is embedded as an X11 child window. `NEBULA_WAYLAND=1` keeps Wayland.
#[cfg(target_os = "linux")]
fn x11_event_loop() -> Option<eframe::EventLoopBuilderHook> {
    if std::env::var_os("NEBULA_WAYLAND").is_some() || std::env::var_os("DISPLAY").is_none() {
        return None;
    }
    // SAFETY: first thing in main, before any other thread exists.
    unsafe { std::env::set_var("GDK_BACKEND", "x11") };
    Some(Box::new(|builder| {
        use winit::platform::x11::EventLoopBuilderExtX11;
        builder.with_x11();
    }))
}

fn main() -> eframe::Result {
    #[cfg(target_os = "linux")]
    let event_loop_builder = x11_event_loop();
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
            Ok(()) if cfg!(windows) => t!("Nebula wurde entfernt.\n\nDeine Einstellungen bleiben im Ordner %APPDATA%\\nebula erhalten.").to_string(),
            Ok(()) => t!("Nebula wurde entfernt.\n\nDeine Einstellungen bleiben im Ordner {dir} erhalten.", dir = settings::config_dir().display()),
            Err(e) => t!("Nebula konnte nicht vollständig entfernt werden:\n{e}", e = format!("{:#}", e)),
        };
        platform::message_box("Nebula", &text);
        return Ok(());
    }
    let open_paths: Vec<PathBuf> = args.iter().filter(|a| !a.to_string_lossy().starts_with("--")).map(PathBuf::from).collect();
    // "Neues Fenster": a further window with a project of its own.
    let window = args.iter().any(|a| a == "--window");
    if window {
        settings::set_secondary();
    }
    // A second start hands its files to the running window ("Mit Nebula öffnen").
    let forwarded = !window && !args.iter().any(|a| a == "--new-instance") && platform::forward_to_running(&open_paths);
    if forwarded {
        return Ok(());
    }
    if !window {
        std::thread::spawn(integration::migrate_file_types);
    }
    let icon = egui::IconData {
        rgba: icon::render_icon(64),
        width: 64,
        height: 64,
    };
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title(settings::APP_NAME)
            // WM_CLASS on X11: matches StartupWMClass of nebula.desktop (dock icon).
            .with_app_id("nebula")
            .with_inner_size([1280.0, 820.0])
            .with_min_inner_size([720.0, 440.0])
            // Windows keeps its frame (shadow, snapping) and hides the caption
            // itself (platform/windows.rs); on Linux the window draws its own.
            .with_decorations(cfg!(windows))
            // Transparent only for the Mica backdrop.
            .with_transparent(cfg!(windows))
            .with_icon(icon)
            .with_drag_and_drop(true),
        // Further windows open where Windows puts new windows (slightly
        // offset), not exactly on top of the main window.
        persist_window: !window,
        wgpu_options: wgpu_options(),
        #[cfg(target_os = "linux")]
        event_loop_builder,
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
