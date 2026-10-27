//! Windows integration for the current user, without an installer tool and
//! without admin rights: install into `%LOCALAPPDATA%\Programs\Nebula` with a
//! start menu entry and an entry under "Apps", "Open with Nebula" in
//! Explorer's context menu, and Nebula for chosen file types (each with the
//! file list's badge as its icon).

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use winreg::RegKey;
use winreg::enums::{HKEY_CURRENT_USER, KEY_ALL_ACCESS, KEY_READ};

const UNINSTALL_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Uninstall\Nebula";
/// The shared program ID of older versions (all types with the Nebula logo).
const LEGACY_PROG_ID: &str = "Nebula.Datei";
/// What Windows' default apps settings list for Nebula.
const CAPABILITIES_KEY: &str = r"Software\Nebula\Capabilities";
/// Other programs' defaults Nebula replaced, restored when a type is unchecked.
const PREVIOUS_KEY: &str = r"Software\Nebula\PreviousDefaults";

/// Explorer context menus: files, folders, the background of a folder.
const MENU_KEYS: [(&str, &str); 3] = [
    (r"Software\Classes\*\shell\Nebula", "%1"),
    (r"Software\Classes\Directory\shell\Nebula", "%1"),
    (r"Software\Classes\Directory\Background\shell\Nebula", "%V"),
];

/// File types Nebula can be registered for (chosen in Settings › Windows).
pub const FILE_TYPES: &[&str] = &[
    "txt", "md", "markdown", "html", "htm", "css", "scss", "sass", "less", "js", "mjs", "cjs", "ts", "tsx", "jsx", "php", "phtml", "json", "jsonc", "xml", "svg",
    "yml", "yaml", "toml", "ini", "conf", "cfg", "env", "htaccess", "log", "sql", "sh", "bat", "cmd", "ps1", "py", "rb", "rs", "go",
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
    key.set_value("UninstallString", &format!("\"{}\" --uninstall", exe.display()))?;
    key.set_value("EstimatedSize", &size_kb)?;
    key.set_value("NoModify", &1u32)?;
    key.set_value("NoRepair", &1u32)?;
    // Registrations made from elsewhere now point to the installed copy.
    if context_menu_enabled() {
        set_context_menu(true)?;
    }
    refresh_file_types()?;
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
    let _ = remove_file_types();
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

/// Program ID of one file type; each has its own icon (the file list badge).
fn prog_id(ext: &str) -> String {
    format!("Nebula.{ext}")
}

/// The file types Nebula is registered for.
pub fn registered_types() -> BTreeSet<String> {
    FILE_TYPES
        .iter()
        .filter(|ext| hkcu().open_subkey_with_flags(format!(r"Software\Classes\{}", prog_id(ext)), KEY_READ).is_ok())
        .map(|ext| ext.to_string())
        .collect()
}

/// Which program Windows opens a file type with.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Handler {
    Nebula,
    /// Nebula, chosen with an older version: still shows the Nebula logo.
    NebulaLegacy,
    /// Another program (its name).
    Other(String),
    /// No working program: Windows asks on a double-click.
    Ask,
}

/// Who opens `.ext` now, as Windows resolves a double-click.
pub fn handler_of(ext: &str) -> Handler {
    let Some(h) = crate::win32::file_type_handler(ext) else { return Handler::Ask };
    let file = Path::new(&h.exe).file_name().map(|f| f.to_string_lossy().to_lowercase()).unwrap_or_default();
    match file.as_str() {
        "nebula.exe" if h.prog_id == LEGACY_PROG_ID => Handler::NebulaLegacy,
        "nebula.exe" => Handler::Nebula,
        "openwith.exe" | "" => Handler::Ask,
        _ => {
            let name = if h.name.trim().is_empty() {
                Path::new(&h.exe).file_stem().map(|f| f.to_string_lossy().into_owned()).unwrap_or_default()
            } else {
                h.name
            };
            Handler::Other(name)
        }
    }
}

/// Nebula's page under Settings › Apps › Default apps, listing its types.
pub const DEFAULT_APPS_PAGE: &str = "ms-settings:defaultapps?registeredAppUser=Nebula";

/// Whether the user never chose a program for `.ext` in Windows (then the
/// registered default applies, which Nebula may set).
pub fn has_no_user_choice(ext: &str) -> bool {
    hkcu()
        .open_subkey_with_flags(format!(r"Software\Microsoft\Windows\CurrentVersion\Explorer\FileExts\.{ext}\UserChoice"), KEY_READ)
        .and_then(|k| k.get_value::<String, _>("ProgId"))
        .is_err()
}

/// Whether this is the program the registrations point to (the installed
/// copy, or this one when Nebula isn't installed).
pub fn runs_registered_program() -> bool {
    std::env::current_exe().is_ok_and(|exe| same_file(&exe, &target_exe()))
}

/// Registers Nebula for exactly the file types in `wanted` (in "Open with",
/// in Windows' default apps, and as default where the user hasn't chosen
/// another program yet). Each type gets its own icon.
pub fn set_file_types(wanted: &BTreeSet<String>) -> Result<()> {
    register(wanted, false)
}

/// `redraw`: draw existing icons again (after an update of Nebula).
fn register(wanted: &BTreeSet<String>, redraw: bool) -> Result<()> {
    let exe = target_exe();
    let open = format!("\"{}\" \"%1\"", exe.display());
    let app = r"Software\Classes\Applications\nebula.exe";
    let mut painter: Option<crate::file_icons::IconPainter> = None;
    remove_legacy();
    for ext in FILE_TYPES {
        let prog = prog_id(ext);
        let classes = format!(r"Software\Classes\{prog}");
        let ext_key = format!(r"Software\Classes\.{ext}");
        if wanted.contains(*ext) {
            let mut icon = crate::file_icons::dir().join(format!("{ext}.ico"));
            if redraw || !icon.is_file() {
                icon = painter
                    .get_or_insert_with(crate::file_icons::IconPainter::new)
                    .write(ext)
                    .with_context(|| t!("Symbol für .{ext} konnte nicht angelegt werden", ext = ext))?;
            }
            let (k, _) = hkcu().create_subkey(&classes)?;
            k.set_value("", &t!("{ext}-Datei", ext = ext.to_uppercase()))?;
            let (i, _) = k.create_subkey("DefaultIcon")?;
            i.set_value("", &icon.display().to_string())?;
            let (shell, _) = k.create_subkey(r"shell\open")?;
            shell.set_value("FriendlyAppName", &"Nebula")?;
            let (cmd, _) = shell.create_subkey("command")?;
            cmd.set_value("", &open)?;
            let (e, _) = hkcu().create_subkey(&ext_key)?;
            let (with, _) = e.create_subkey("OpenWithProgids")?;
            with.set_value(&prog, &"")?;
            // The registered default (what applies without a choice of the
            // user in Windows). Another program's entry is kept for later.
            let current = e.get_value::<String, _>("").unwrap_or_default();
            if current != prog {
                if !current.is_empty() && !current.starts_with("Nebula.") {
                    let (previous, _) = hkcu().create_subkey(PREVIOUS_KEY)?;
                    previous.set_value(ext, &current)?;
                }
                e.set_value("", &prog)?;
            }
        } else {
            let _ = hkcu().delete_subkey_all(&classes);
            if let Ok(e) = hkcu().open_subkey_with_flags(&ext_key, KEY_ALL_ACCESS) {
                if let Ok(with) = e.open_subkey_with_flags("OpenWithProgids", KEY_ALL_ACCESS) {
                    let _ = with.delete_value(&prog);
                }
                if e.get_value::<String, _>("").ok().as_deref() == Some(prog.as_str()) {
                    let previous = hkcu().open_subkey_with_flags(PREVIOUS_KEY, KEY_READ).and_then(|k| k.get_value::<String, _>(ext));
                    let _ = match previous {
                        Ok(p) => e.set_value("", &p),
                        Err(_) => e.delete_value(""),
                    };
                }
            }
            if let Ok(k) = hkcu().open_subkey_with_flags(PREVIOUS_KEY, KEY_ALL_ACCESS) {
                let _ = k.delete_value(ext);
            }
            let _ = std::fs::remove_file(crate::file_icons::dir().join(format!("{ext}.ico")));
        }
    }
    if wanted.is_empty() {
        let _ = hkcu().delete_subkey_all(app);
        let _ = hkcu().delete_subkey_all(CAPABILITIES_KEY);
        if let Ok(k) = hkcu().open_subkey_with_flags(r"Software\RegisteredApplications", KEY_ALL_ACCESS) {
            let _ = k.delete_value("Nebula");
        }
        let _ = std::fs::remove_dir_all(crate::file_icons::dir());
        let _ = hkcu().delete_subkey_all(format!(r"Software\Classes\{LEGACY_PROG_ID}"));
    } else {
        let _ = hkcu().delete_subkey_all(format!(r"{app}\SupportedTypes"));
        let (a, _) = hkcu().create_subkey(app)?;
        a.set_value("FriendlyAppName", &"Nebula")?;
        let (acmd, _) = a.create_subkey(r"shell\open\command")?;
        acmd.set_value("", &open)?;
        let (types, _) = a.create_subkey("SupportedTypes")?;
        // Windows' default apps list Nebula with these types.
        let _ = hkcu().delete_subkey_all(CAPABILITIES_KEY);
        let (caps, _) = hkcu().create_subkey(CAPABILITIES_KEY)?;
        caps.set_value("ApplicationName", &"Nebula")?;
        caps.set_value("ApplicationDescription", &t!("Nebula – Code-Editor"))?;
        caps.set_value("ApplicationIcon", &format!("\"{}\",0", exe.display()))?;
        let (assoc, _) = caps.create_subkey("FileAssociations")?;
        for ext in wanted {
            types.set_value(format!(".{ext}"), &"")?;
            assoc.set_value(format!(".{ext}"), &prog_id(ext))?;
        }
        let (reg, _) = hkcu().create_subkey(r"Software\RegisteredApplications")?;
        reg.set_value("Nebula", &CAPABILITIES_KEY)?;
    }
    crate::win32::notify_associations_changed();
    Ok(())
}

/// Before each type had its own icon, one program ID served all types.
/// It is no longer offered; the key itself stays while Nebula is registered,
/// so a default chosen back then keeps working until it is chosen again
/// (removed together with the last type).
fn remove_legacy() {
    for ext in FILE_TYPES {
        if let Ok(with) = hkcu().open_subkey_with_flags(format!(r"Software\Classes\.{ext}\OpenWithProgids"), KEY_ALL_ACCESS) {
            let _ = with.delete_value(LEGACY_PROG_ID);
        }
    }
}

/// Removes every file type registration, including the old shared one.
pub fn remove_file_types() -> Result<()> {
    set_file_types(&BTreeSet::new())
}

/// Older versions registered all types under one program ID with the Nebula
/// logo: switches them to the per-type icons, once and only when that
/// registration points to the running program.
pub fn migrate_file_types() {
    let command = hkcu()
        .open_subkey_with_flags(format!(r"Software\Classes\{LEGACY_PROG_ID}\shell\open\command"), KEY_READ)
        .and_then(|k| k.get_value::<String, _>(""));
    let (Ok(command), Ok(exe)) = (command, std::env::current_exe()) else { return };
    if !command.to_lowercase().contains(&exe.display().to_string().to_lowercase()) || !registered_types().is_empty() {
        return;
    }
    let _ = set_file_types(&FILE_TYPES.iter().map(|e| e.to_string()).collect());
}

/// Re-registers the current types (after an update: new icons, new program path).
pub fn refresh_file_types() -> Result<()> {
    let types = registered_types();
    if types.is_empty() { Ok(()) } else { register(&types, true) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_types_have_icons_and_are_unique() {
        let set: BTreeSet<&str> = FILE_TYPES.iter().copied().collect();
        assert_eq!(set.len(), FILE_TYPES.len(), "duplicate file type");
        assert!(FILE_TYPES.iter().all(|e| e.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit())));
    }
}
