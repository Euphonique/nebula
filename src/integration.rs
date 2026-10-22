//! Windows integration for the current user, without an installer tool and
//! without admin rights: install into `%LOCALAPPDATA%\Programs\Nebula` with a
//! start menu entry and an entry under "Apps", "Open with Nebula" in
//! Explorer's context menu, and Nebula in "Open with" for text file types.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use winreg::RegKey;
use winreg::enums::{HKEY_CURRENT_USER, KEY_READ};

const UNINSTALL_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Uninstall\Nebula";
const PROG_ID: &str = "Nebula.Datei";

/// Explorer context menus: files, folders, the background of a folder.
const MENU_KEYS: [(&str, &str); 3] = [
    (r"Software\Classes\*\shell\Nebula", "%1"),
    (r"Software\Classes\Directory\shell\Nebula", "%1"),
    (r"Software\Classes\Directory\Background\shell\Nebula", "%V"),
];

/// File types Nebula offers itself for in "Open with".
pub const FILE_TYPES: &[&str] = &[
    "txt", "md", "markdown", "html", "htm", "css", "scss", "less", "js", "mjs", "ts", "tsx", "jsx", "php", "json", "xml", "svg",
    "yml", "yaml", "toml", "ini", "conf", "cfg", "env", "htaccess", "log", "sql", "sh", "bat", "ps1", "py", "rb", "rs", "go",
    "java", "c", "h", "cpp", "hpp", "cs", "lua", "csv", "diff", "patch",
];

fn hkcu() -> RegKey {
    RegKey::predef(HKEY_CURRENT_USER)
}

/// Where "Install" puts Nebula.
pub fn install_dir() -> PathBuf {
    PathBuf::from(std::env::var_os("LOCALAPPDATA").unwrap_or_default()).join("Programs").join("Nebula")
}

fn start_menu_link() -> PathBuf {
    PathBuf::from(std::env::var_os("APPDATA").unwrap_or_default())
        .join(r"Microsoft\Windows\Start Menu\Programs")
        .join("Nebula.lnk")
}

/// The installed copy, if Nebula has been installed.
pub fn installed_exe() -> Option<PathBuf> {
    let exe = install_dir().join("nebula.exe");
    exe.is_file().then_some(exe)
}

/// The program the registrations should point to: the installed copy if
/// there is one, else the running one.
pub fn target_exe() -> PathBuf {
    installed_exe().or_else(|| std::env::current_exe().ok()).unwrap_or_default()
}

/// Copies the running program into the install folder, adds the start menu
/// entry and the entry under "Apps". Returns the installed program.
pub fn install() -> Result<PathBuf> {
    let current = std::env::current_exe().context(t!("Programmpfad nicht gefunden"))?;
    let dir = install_dir();
    std::fs::create_dir_all(&dir).with_context(|| t!("{dir} konnte nicht angelegt werden", dir = dir.display()))?;
    let exe = dir.join("nebula.exe");
    if !same_file(&current, &exe) {
        // A running installed copy can't be overwritten, but renamed.
        if exe.exists() {
            let old = dir.join("nebula.old.exe");
            let _ = std::fs::remove_file(&old);
            let _ = std::fs::rename(&exe, &old);
        }
        std::fs::copy(&current, &exe).with_context(|| t!("{exe} konnte nicht kopiert werden", exe = exe.display()))?;
        // Symbols for readable crash reports, if they are next to the program.
        let pdb = current.with_extension("pdb");
        if pdb.is_file() {
            let _ = std::fs::copy(&pdb, exe.with_extension("pdb"));
        }
    }
    let link = start_menu_link();
    if let Some(parent) = link.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if !crate::win32::create_shortcut(&link, &exe, t!("Nebula – Code-Editor")) {
        anyhow::bail!(t!("Der Startmenü-Eintrag konnte nicht angelegt werden"));
    }
    let (key, _) = hkcu().create_subkey(UNINSTALL_KEY)?;
    let size_kb = std::fs::metadata(&exe).map(|m| (m.len() / 1024) as u32).unwrap_or(0);
    key.set_value("DisplayName", &"Nebula")?;
    key.set_value("DisplayVersion", &env!("CARGO_PKG_VERSION"))?;
    key.set_value("Publisher", &"Nebula")?;
    key.set_value("DisplayIcon", &format!("{},0", exe.display()))?;
    key.set_value("InstallLocation", &dir.display().to_string())?;
    key.set_value("UninstallString", &t!("\"{exe}\" --uninstall", exe = exe.display()))?;
    key.set_value("EstimatedSize", &size_kb)?;
    key.set_value("NoModify", &1u32)?;
    key.set_value("NoRepair", &1u32)?;
    // Registrations made from elsewhere now point to the installed copy.
    if context_menu_enabled() {
        set_context_menu(true)?;
    }
    if file_types_registered() {
        set_file_types(true)?;
    }
    crate::win32::notify_associations_changed();
    Ok(exe)
}

fn same_file(a: &Path, b: &Path) -> bool {
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    }
}

pub fn is_installed() -> bool {
    hkcu().open_subkey_with_flags(UNINSTALL_KEY, KEY_READ).is_ok() && installed_exe().is_some()
}

/// Removes everything `install` and the switches added. The program folder
/// is deleted shortly after Nebula exits (a running program can't delete itself).
pub fn uninstall() -> Result<()> {
    let _ = set_context_menu(false);
    let _ = set_file_types(false);
    let _ = std::fs::remove_file(start_menu_link());
    let _ = hkcu().delete_subkey_all(UNINSTALL_KEY);
    crate::win32::notify_associations_changed();
    let dir = install_dir();
    if dir.is_dir() {
        // `cmd` is part of every Windows: waits a moment, then removes the folder.
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        std::process::Command::new("cmd")
            .args(["/C", "ping", "-n", "3", "127.0.0.1", ">nul", "&", "rmdir", "/S", "/Q"])
            .arg(&dir)
            .creation_flags(CREATE_NO_WINDOW)
            .spawn()
            .context(t!("Der Programmordner konnte nicht zum Löschen vorgemerkt werden"))?;
    }
    Ok(())
}

pub fn context_menu_enabled() -> bool {
    hkcu().open_subkey_with_flags(MENU_KEYS[0].0, KEY_READ).is_ok()
}

/// "Mit Nebula öffnen" for files, folders and folder backgrounds.
pub fn set_context_menu(on: bool) -> Result<()> {
    let exe = target_exe();
    for (key, arg) in MENU_KEYS {
        if on {
            let (k, _) = hkcu().create_subkey(key)?;
            k.set_value("", &t!("Mit Nebula öffnen"))?;
            k.set_value("Icon", &format!("\"{}\",0", exe.display()))?;
            let (cmd, _) = k.create_subkey("command")?;
            cmd.set_value("", &format!("\"{}\" \"{arg}\"", exe.display()))?;
        } else {
            let _ = hkcu().delete_subkey_all(key);
        }
    }
    crate::win32::notify_associations_changed();
    Ok(())
}

pub fn file_types_registered() -> bool {
    hkcu().open_subkey_with_flags(format!(r"Software\Classes\{PROG_ID}"), KEY_READ).is_ok()
}

/// Nebula as a program for the text file types (in "Open with"; the default
/// app is chosen by the user in Windows).
pub fn set_file_types(on: bool) -> Result<()> {
    let exe = target_exe();
    let classes = format!(r"Software\Classes\{PROG_ID}");
    let app = r"Software\Classes\Applications\nebula.exe";
    if on {
        let (k, _) = hkcu().create_subkey(&classes)?;
        k.set_value("", &t!("Textdatei (Nebula)"))?;
        let (icon, _) = k.create_subkey("DefaultIcon")?;
        icon.set_value("", &format!("\"{}\",0", exe.display()))?;
        let (cmd, _) = k.create_subkey(r"shell\open\command")?;
        cmd.set_value("", &format!("\"{}\" \"%1\"", exe.display()))?;
        let (a, _) = hkcu().create_subkey(app)?;
        a.set_value("FriendlyAppName", &"Nebula")?;
        let (acmd, _) = a.create_subkey(r"shell\open\command")?;
        acmd.set_value("", &format!("\"{}\" \"%1\"", exe.display()))?;
        let (types, _) = a.create_subkey("SupportedTypes")?;
        for ext in FILE_TYPES {
            types.set_value(format!(".{ext}"), &"")?;
            let (open_with, _) = hkcu().create_subkey(format!(r"Software\Classes\.{ext}\OpenWithProgids"))?;
            open_with.set_value(PROG_ID, &"")?;
        }
    } else {
        let _ = hkcu().delete_subkey_all(&classes);
        let _ = hkcu().delete_subkey_all(app);
        for ext in FILE_TYPES {
            if let Ok(k) = hkcu().open_subkey_with_flags(format!(r"Software\Classes\.{ext}\OpenWithProgids"), winreg::enums::KEY_ALL_ACCESS) {
                let _ = k.delete_value(PROG_ID);
            }
        }
    }
    crate::win32::notify_associations_changed();
    Ok(())
}
