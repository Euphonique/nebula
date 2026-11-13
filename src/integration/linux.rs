//! Linux: integration for the current user along the freedesktop specs,
//! without root: the program in `~/.local/bin`, a `nebula.desktop` entry
//! (application menu, "Open With" in the file manager) and Nebula as the
//! default application for chosen file types in `~/.config/mimeapps.list`.
//!
//! The desktop entry also holds Nebula's own state (`X-Nebula-*` keys), so
//! there is nothing else to keep in sync.

use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

const DESKTOP_ID: &str = "nebula.desktop";

/// File types Nebula can be the default for, with their freedesktop MIME
/// types. Endings without a type of their own (.ini, .conf, .env, .jsx,
/// .less, .htaccess …) are plain text to the file manager: `txt` covers them.
const TYPES: &[(&str, &[&str])] = &[
    ("txt", &["text/plain"]),
    ("md", &["text/markdown"]),
    ("markdown", &["text/markdown"]),
    ("html", &["text/html"]),
    ("htm", &["text/html"]),
    ("css", &["text/css"]),
    ("scss", &["text/x-scss"]),
    ("sass", &["text/x-sass"]),
    ("js", &["text/javascript"]),
    ("mjs", &["text/javascript"]),
    // shared-mime-info knows .ts only as Qt Linguist; TypeScript files get that type.
    ("ts", &["text/vnd.trolltech.linguist"]),
    ("php", &["application/x-php"]),
    ("phtml", &["application/x-php"]),
    ("json", &["application/json"]),
    ("xml", &["application/xml"]),
    ("svg", &["image/svg+xml"]),
    ("yml", &["application/yaml"]),
    ("yaml", &["application/yaml"]),
    ("toml", &["application/toml"]),
    ("log", &["text/x-log"]),
    ("sql", &["application/sql"]),
    ("sh", &["application/x-shellscript"]),
    ("py", &["text/x-python3", "text/x-python"]),
    ("rb", &["application/x-ruby"]),
    ("rs", &["text/rust"]),
    ("go", &["text/x-go"]),
    ("java", &["text/x-java"]),
    ("c", &["text/x-csrc"]),
    ("h", &["text/x-chdr"]),
    ("cpp", &["text/x-c++src"]),
    ("hpp", &["text/x-c++hdr"]),
    ("cs", &["text/x-csharp"]),
    ("lua", &["text/x-lua"]),
    ("csv", &["text/csv"]),
    ("diff", &["text/x-patch"]),
    ("patch", &["text/x-patch"]),
];

/// File types Nebula can be registered for (chosen in Settings › System).
pub const FILE_TYPES: &[&str] = &[
    "txt", "md", "markdown", "html", "htm", "css", "scss", "sass", "js", "mjs", "ts", "php", "phtml", "json", "xml", "svg", "yml", "yaml", "toml",
    "log", "sql", "sh", "py", "rb", "rs", "go", "java", "c", "h", "cpp", "hpp", "cs", "lua", "csv", "diff", "patch",
];

/// Types for "Open With" on everything textual and on folders.
const CONTEXT_MIMES: &[&str] = &["text/plain", "inode/directory"];

/// No settings page for default apps; Nebula sets the defaults itself.
pub const DEFAULT_APPS_PAGE: &str = "";

fn mimes(ext: &str) -> &'static [&'static str] {
    TYPES.iter().find(|(e, _)| *e == ext).map_or(&[], |(_, m)| m)
}

// ---- XDG folders ---------------------------------------------------------------

fn home() -> PathBuf {
    directories::BaseDirs::new().map(|d| d.home_dir().to_path_buf()).unwrap_or_default()
}

fn xdg(var: &str, fallback: &str) -> PathBuf {
    std::env::var_os(var).map(PathBuf::from).filter(|p| p.is_absolute()).unwrap_or_else(|| home().join(fallback))
}

fn data_home() -> PathBuf {
    xdg("XDG_DATA_HOME", ".local/share")
}

fn config_home() -> PathBuf {
    xdg("XDG_CONFIG_HOME", ".config")
}

fn xdg_dirs(var: &str, fallback: &str) -> Vec<PathBuf> {
    std::env::var(var)
        .ok()
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| fallback.into())
        .split(':')
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .collect()
}

fn applications_dir() -> PathBuf {
    data_home().join("applications")
}

fn desktop_file() -> PathBuf {
    applications_dir().join(DESKTOP_ID)
}

fn icon_file() -> PathBuf {
    data_home().join("icons/hicolor/256x256/apps/nebula.png")
}

/// Where "Install" puts Nebula (usually in PATH).
pub fn install_dir() -> PathBuf {
    home().join(".local").join("bin")
}

/// The installed copy, if Nebula has been installed.
pub fn installed_exe() -> Option<PathBuf> {
    let exe = install_dir().join("nebula");
    exe.is_file().then_some(exe)
}

/// The program the entries should point to: the installed copy if there is
/// one, else the running one.
fn target_exe() -> PathBuf {
    installed_exe().or_else(|| std::env::current_exe().ok()).unwrap_or_default()
}

fn same_file(a: &Path, b: &Path) -> bool {
    matches!((a.canonicalize(), b.canonicalize()), (Ok(a), Ok(b)) if a == b)
}

// ---- the desktop entry ---------------------------------------------------------------

/// What Nebula's desktop entry stands for.
#[derive(Default, Debug, PartialEq)]
struct Entry {
    /// Shown in the application menu (installed).
    menu: bool,
    /// "Open With Nebula" for text files and folders.
    context_menu: bool,
    /// File types Nebula is the default for.
    types: BTreeSet<String>,
}

fn read_entry() -> Option<Entry> {
    Some(parse_entry(&std::fs::read_to_string(desktop_file()).ok()?))
}

fn parse_entry(text: &str) -> Entry {
    let mut e = Entry::default();
    for line in text.lines() {
        match line.split_once('=') {
            Some(("NoDisplay", v)) => e.menu = v.trim() != "true",
            Some(("X-Nebula-ContextMenu", v)) => e.context_menu = v.trim() == "true",
            Some(("X-Nebula-FileTypes", v)) => e.types = v.split(';').filter(|t| !t.is_empty()).map(str::to_string).collect(),
            _ => {}
        }
    }
    e
}

/// `Exec` argument quoting of the Desktop Entry spec.
fn exec_quote(path: &Path) -> String {
    let s = path.to_string_lossy();
    if s.chars().all(|c| c.is_alphanumeric() || "/._-+".contains(c)) {
        return s.into_owned();
    }
    let mut out = String::from("\"");
    for c in s.chars() {
        if matches!(c, '"' | '`' | '$' | '\\') {
            out.push('\\');
        }
        out.push(c);
    }
    out.push('"');
    // The value itself escapes backslashes once more.
    out.replace('\\', "\\\\")
}

fn render_entry(e: &Entry, exe: &Path) -> String {
    let mut mime: Vec<&str> = Vec::new();
    if e.context_menu {
        mime.extend(CONTEXT_MIMES);
    }
    for ext in &e.types {
        for m in mimes(ext) {
            if !mime.contains(m) {
                mime.push(m);
            }
        }
    }
    let list = |items: &mut dyn Iterator<Item = &str>| items.fold(String::new(), |acc, s| acc + s + ";");
    format!(
        "[Desktop Entry]\n\
         Type=Application\n\
         Name=Nebula\n\
         GenericName={generic}\n\
         Comment={comment}\n\
         Exec={exec} %F\n\
         Icon=nebula\n\
         Terminal=false\n\
         StartupNotify=true\n\
         StartupWMClass=nebula\n\
         Categories=Development;TextEditor;IDE;\n\
         Keywords=editor;code;ftp;sftp;html;css;php;\n\
         MimeType={mime}\n\
         NoDisplay={hidden}\n\
         X-Nebula-ContextMenu={context}\n\
         X-Nebula-FileTypes={types}\n",
        generic = t!("Code-Editor"),
        comment = t!("Schneller Code-Editor mit FTP/SFTP"),
        exec = exec_quote(exe),
        mime = list(&mut mime.iter().copied()),
        hidden = !e.menu,
        context = e.context_menu,
        types = list(&mut e.types.iter().map(String::as_str)),
    )
}

/// Writes the entry (or removes it once it stands for nothing) and lets the
/// desktop pick up the change.
fn write_entry(e: &Entry) -> Result<()> {
    let file = desktop_file();
    if !e.menu && !e.context_menu && e.types.is_empty() {
        let _ = std::fs::remove_file(&file);
    } else {
        std::fs::create_dir_all(applications_dir()).with_context(|| applications_dir().display().to_string())?;
        write_icon();
        crate::safe_write::write(&file, render_entry(e, &target_exe()).as_bytes()).with_context(|| file.display().to_string())?;
    }
    refresh_desktop_database();
    Ok(())
}

fn write_icon() {
    let path = icon_file();
    if path.is_file() {
        return;
    }
    let mut png = Vec::new();
    let mut encoder = png::Encoder::new(&mut png, 256, 256);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let ok = encoder.write_header().and_then(|mut w| w.write_image_data(&crate::icon::render_icon(256))).is_ok();
    if ok && let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
        let _ = std::fs::write(&path, png);
    }
}

/// "Open With" lists come from mimeinfo.cache; rebuilt if the tool exists.
fn refresh_desktop_database() {
    let _ = std::process::Command::new("update-desktop-database")
        .arg(applications_dir())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status();
}

// ---- default applications (mimeapps.list) -----------------------------------------------

const DEFAULTS: &str = "[Default Applications]";

/// Puts Nebula first for `add` and takes it out for `remove` in the user's
/// `mimeapps.list`; other lines and sections stay as they are.
fn update_defaults(add: &BTreeSet<&str>, remove: &BTreeSet<&str>) -> Result<()> {
    if add.is_empty() && remove.is_empty() {
        return Ok(());
    }
    let path = config_home().join("mimeapps.list");
    let old = std::fs::read_to_string(&path).unwrap_or_default();
    let new = edit_defaults(&old, add, remove);
    if new != old {
        std::fs::create_dir_all(config_home())?;
        crate::safe_write::write(&path, new.as_bytes()).with_context(|| path.display().to_string())?;
    }
    Ok(())
}

fn edit_defaults(text: &str, add: &BTreeSet<&str>, remove: &BTreeSet<&str>) -> String {
    let mut lines: Vec<String> = text.lines().map(str::to_string).collect();
    let start = match lines.iter().position(|l| l.trim() == DEFAULTS) {
        Some(i) => i,
        None => {
            if lines.last().is_some_and(|l| !l.trim().is_empty()) {
                lines.push(String::new());
            }
            lines.push(DEFAULTS.into());
            lines.len() - 1
        }
    };
    let end = lines[start + 1..].iter().position(|l| l.trim_start().starts_with('[')).map_or(lines.len(), |i| start + 1 + i);
    let mut section: Vec<String> = lines[start + 1..end].to_vec();
    let mut seen = BTreeSet::new();
    section.retain_mut(|line| {
        let Some((key, value)) = line.split_once('=') else { return true };
        let key = key.trim().to_string();
        if !add.contains(key.as_str()) && !remove.contains(key.as_str()) {
            return true;
        }
        let others: String = value
            .split(';')
            .map(str::trim)
            .filter(|v| !v.is_empty() && *v != DESKTOP_ID)
            .map(|o| format!("{o};"))
            .collect();
        if add.contains(key.as_str()) {
            *line = format!("{key}={DESKTOP_ID};{others}");
            seen.insert(key);
            true
        } else if others.is_empty() {
            false
        } else {
            *line = format!("{key}={others}");
            true
        }
    });
    // Keep a trailing blank line before the next section where there was one.
    let blank_tail = section.last().is_some_and(|l| l.trim().is_empty());
    if blank_tail {
        section.pop();
    }
    for key in add.iter().filter(|k| !seen.contains(**k)) {
        section.push(format!("{key}={DESKTOP_ID};"));
    }
    if blank_tail {
        section.push(String::new());
    }
    lines.splice(start + 1..end, section);
    let mut out = lines.join("\n");
    out.push('\n');
    out
}

/// The default application of each MIME type, in the order of the spec
/// (user config before system config before system data, desktop-specific
/// lists first), as desktop ids.
fn default_apps() -> HashMap<String, String> {
    let desktops: Vec<String> = std::env::var("XDG_CURRENT_DESKTOP")
        .unwrap_or_default()
        .split(':')
        .filter(|d| !d.is_empty())
        .map(str::to_lowercase)
        .collect();
    let data_dirs = xdg_dirs("XDG_DATA_DIRS", "/usr/local/share:/usr/share");
    let mut files: Vec<PathBuf> = Vec::new();
    let mut add_dir = |dir: PathBuf| {
        for d in &desktops {
            files.push(dir.join(format!("{d}-mimeapps.list")));
        }
        files.push(dir.join("mimeapps.list"));
    };
    add_dir(config_home());
    for d in xdg_dirs("XDG_CONFIG_DIRS", "/etc/xdg") {
        add_dir(d);
    }
    add_dir(data_home().join("applications"));
    for d in &data_dirs {
        add_dir(d.join("applications"));
    }
    for d in &data_dirs {
        files.push(d.join("applications").join("defaults.list"));
    }
    let mut found = HashMap::new();
    for file in files {
        let Ok(text) = std::fs::read_to_string(&file) else { continue };
        let mut in_defaults = false;
        for line in text.lines().map(str::trim) {
            if line.starts_with('[') {
                in_defaults = line == DEFAULTS;
                continue;
            }
            let Some((mime, apps)) = line.split_once('=').filter(|_| in_defaults) else { continue };
            if found.contains_key(mime.trim()) {
                continue;
            }
            if let Some(app) = apps.split(';').map(str::trim).find(|a| !a.is_empty() && desktop_entry_path(a).is_some()) {
                found.insert(mime.trim().to_string(), app.to_string());
            }
        }
    }
    found
}

/// The file of an installed desktop entry (`org.gnome.TextEditor.desktop`).
fn desktop_entry_path(id: &str) -> Option<PathBuf> {
    std::iter::once(data_home())
        .chain(xdg_dirs("XDG_DATA_DIRS", "/usr/local/share:/usr/share"))
        .map(|d| d.join("applications").join(id))
        .find(|p| p.is_file())
}

/// The display name of a desktop entry, else its id.
fn app_name(id: &str) -> String {
    let text = desktop_entry_path(id).and_then(|p| std::fs::read_to_string(p).ok()).unwrap_or_default();
    text.lines()
        .find_map(|l| l.strip_prefix("Name="))
        .map(str::to_string)
        .unwrap_or_else(|| id.trim_end_matches(".desktop").to_string())
}

// ---- the API of the settings page -------------------------------------------------------

/// Copies the running program into `~/.local/bin` and adds the application
/// menu entry. Returns the installed program.
pub fn install() -> Result<PathBuf> {
    let current = std::env::current_exe().context(t!("Programmpfad nicht gefunden"))?;
    let dir = install_dir();
    std::fs::create_dir_all(&dir).with_context(|| t!("{dir} konnte nicht angelegt werden", dir = dir.display()))?;
    let exe = dir.join("nebula");
    if !same_file(&current, &exe) {
        // Copy next to it, then rename: a running installed copy keeps its
        // (now unlinked) file and the new one is complete or absent.
        let tmp = dir.join(".nebula.install");
        std::fs::copy(&current, &tmp).with_context(|| t!("{exe} konnte nicht kopiert werden", exe = exe.display()))?;
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o755))?;
        }
        std::fs::rename(&tmp, &exe).with_context(|| t!("{exe} konnte nicht kopiert werden", exe = exe.display()))?;
    }
    let mut entry = read_entry().unwrap_or_default();
    entry.menu = true;
    write_entry(&entry)?;
    Ok(exe)
}

pub fn is_installed() -> bool {
    installed_exe().is_some() && read_entry().is_some_and(|e| e.menu)
}

/// Removes the program, the menu entry, "Open With" and the file type defaults.
pub fn uninstall() -> Result<()> {
    let _ = set_file_types(&BTreeSet::new());
    let _ = std::fs::remove_file(desktop_file());
    let _ = std::fs::remove_file(icon_file());
    refresh_desktop_database();
    if let Some(exe) = installed_exe() {
        std::fs::remove_file(&exe).with_context(|| exe.display().to_string())?;
    }
    Ok(())
}

pub fn context_menu_enabled() -> bool {
    read_entry().is_some_and(|e| e.context_menu)
}

pub fn set_context_menu(on: bool) -> Result<()> {
    let mut entry = read_entry().unwrap_or_default();
    entry.context_menu = on;
    write_entry(&entry)
}

pub fn registered_types() -> BTreeSet<String> {
    read_entry().map(|e| e.types).unwrap_or_default()
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Handler {
    Nebula,
    /// Only on Windows (an older registration).
    #[allow(dead_code)]
    NebulaLegacy,
    Other(String),
    /// No default set: the file manager picks one.
    Ask,
}

/// The program the system opens each type with, reading the defaults once.
pub fn handlers<'a>(exts: &[&'a str]) -> HashMap<&'a str, Handler> {
    let defaults = default_apps();
    exts.iter()
        .map(|ext| {
            let handler = match mimes(ext).first().and_then(|m| defaults.get(*m)) {
                Some(id) if id == DESKTOP_ID => Handler::Nebula,
                Some(id) => Handler::Other(app_name(id)),
                None => Handler::Ask,
            };
            (*ext, handler)
        })
        .collect()
}

/// Windows only: there the user has to confirm defaults in its settings.
pub fn has_no_user_choice(_ext: &str) -> bool {
    false
}

pub fn runs_registered_program() -> bool {
    std::env::current_exe().is_ok_and(|exe| same_file(&exe, &target_exe()))
}

/// Makes Nebula the default for exactly the `wanted` types; types no longer
/// wanted go back to the system's choice.
pub fn set_file_types(wanted: &BTreeSet<String>) -> Result<()> {
    let mut entry = read_entry().unwrap_or_default();
    let wanted: BTreeSet<String> = wanted.iter().filter(|e| !mimes(e).is_empty()).cloned().collect();
    let keep: BTreeSet<&str> = wanted.iter().flat_map(|e| mimes(e).iter().copied()).collect();
    let drop: BTreeSet<&str> = entry.types.iter().flat_map(|e| mimes(e).iter().copied()).filter(|m| !keep.contains(m)).collect();
    entry.types = wanted;
    // The entry first: a default must name an application that lists the type.
    write_entry(&entry)?;
    update_defaults(&keep, &drop)
}

/// Windows only (older registrations).
pub fn migrate_file_types() {}

#[cfg(test)]
mod tests {
    use super::*;

    fn set<'a>(items: &[&'a str]) -> BTreeSet<&'a str> {
        items.iter().copied().collect()
    }

    #[test]
    fn every_offered_type_has_a_mime_type() {
        for ext in FILE_TYPES {
            assert!(!mimes(ext).is_empty(), "{ext}");
        }
        assert_eq!(TYPES.len(), FILE_TYPES.len());
    }

    #[test]
    fn entry_round_trips() {
        let e = Entry { menu: true, context_menu: true, types: ["css".to_string(), "html".to_string()].into() };
        let text = render_entry(&e, Path::new("/home/u/.local/bin/nebula"));
        assert!(text.contains("Exec=/home/u/.local/bin/nebula %F\n"));
        assert!(text.contains("MimeType=text/plain;inode/directory;text/css;text/html;\n"));
        assert_eq!(parse_entry(&text), e);
    }

    #[test]
    fn exec_paths_are_quoted() {
        assert_eq!(exec_quote(Path::new("/opt/Nebula App/nebula")), "\"/opt/Nebula App/nebula\"");
        assert_eq!(exec_quote(Path::new("/a/$x")), "\"/a/\\\\$x\"");
    }

    /// The whole cycle in a temporary home (changes process-wide environment
    /// variables, so it runs alone): `cargo test full_cycle -- --ignored`.
    #[test]
    #[ignore]
    fn full_cycle_in_a_temporary_home() {
        let home = tempfile::tempdir().unwrap();
        // SAFETY: the test runs on its own (ignored by default).
        unsafe {
            std::env::set_var("HOME", home.path());
            std::env::remove_var("XDG_DATA_HOME");
            std::env::remove_var("XDG_CONFIG_HOME");
        }
        let list = home.path().join(".config/mimeapps.list");
        std::fs::create_dir_all(list.parent().unwrap()).unwrap();
        std::fs::write(&list, "[Default Applications]\ntext/css=org.gnome.TextEditor.desktop;\n").unwrap();

        let exe = install().unwrap();
        assert_eq!(exe, home.path().join(".local/bin/nebula"));
        assert!(is_installed());
        assert!(icon_file().is_file());
        set_context_menu(true).unwrap();
        set_file_types(&["css".to_string(), "html".to_string(), "htm".to_string()].into()).unwrap();
        let entry = std::fs::read_to_string(desktop_file()).unwrap();
        assert!(entry.contains(&format!("Exec={} %F", exe.display())), "{entry}");
        assert!(entry.contains("MimeType=text/plain;inode/directory;text/css;text/html;"), "{entry}");
        assert_eq!(registered_types().len(), 3);
        assert_eq!(
            std::fs::read_to_string(&list).unwrap(),
            "[Default Applications]\ntext/css=nebula.desktop;org.gnome.TextEditor.desktop;\ntext/html=nebula.desktop;\n"
        );
        assert_eq!(handlers(&["css"])["css"], Handler::Nebula);

        // Unchecking .html keeps text/html for .htm.
        set_file_types(&["css".to_string(), "htm".to_string()].into()).unwrap();
        assert!(std::fs::read_to_string(&list).unwrap().contains("text/html=nebula.desktop;"));

        uninstall().unwrap();
        assert!(!is_installed() && !desktop_file().exists() && !exe.exists());
        assert_eq!(std::fs::read_to_string(&list).unwrap(), "[Default Applications]\ntext/css=org.gnome.TextEditor.desktop;\n");
    }

    #[test]
    fn defaults_are_added_and_removed() {
        let old = "[Added Associations]\ntext/x-go=code.desktop;\n\n[Default Applications]\ntext/html=firefox.desktop;\ntext/css=nebula.desktop;gedit.desktop;\ntext/x-log=nebula.desktop;\n";
        let new = edit_defaults(old, &set(&["text/html", "text/x-go"]), &set(&["text/css", "text/x-log"]));
        assert_eq!(
            new,
            "[Added Associations]\ntext/x-go=code.desktop;\n\n[Default Applications]\ntext/html=nebula.desktop;firefox.desktop;\ntext/css=gedit.desktop;\ntext/x-go=nebula.desktop;\n"
        );
        // A file without the section gets one.
        assert_eq!(edit_defaults("", &set(&["text/css"]), &set(&[])), "[Default Applications]\ntext/css=nebula.desktop;\n");
        // Other sections after it stay where they are.
        let two = "[Default Applications]\ntext/css=a.desktop;\n\n[Removed Associations]\nx=y;\n";
        assert_eq!(
            edit_defaults(two, &set(&["text/html"]), &set(&[])),
            "[Default Applications]\ntext/css=a.desktop;\ntext/html=nebula.desktop;\n\n[Removed Associations]\nx=y;\n"
        );
    }
}
