//! Panic's Extension Library (extensions.panic.com): search, details and
//! installing extension bundles.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde_json::Value;

use super::manifest::{Extension, extensions_dir};

const API: &str = "https://extensions.panic.com/api/v1/extensions/";

#[derive(Clone, Debug, Default)]
pub struct LibraryEntry {
    pub identifier: String,
    pub name: String,
    pub organization: String,
    pub description: String,
    pub version: String,
    pub source_url: String,
    pub categories: Vec<String>,
    pub downloads: u64,
    pub logo: Option<String>,
    pub readme: Option<String>,
    pub repository: Option<String>,
    pub entitlements: Vec<String>,
}

/// Categories of the library with German labels.
pub const CATEGORIES: &[(&str, &str)] = &[
    ("", tk!("Alle")),
    ("languages", tk!("Sprachen")),
    ("themes", tk!("Themes")),
    ("completions", tk!("Vervollständigung")),
    ("formatters", tk!("Formatierer")),
    ("issues", tk!("Prüfung (Issues)")),
    ("commands", tk!("Befehle")),
    ("clips", tk!("Clips")),
    ("sidebars", tk!("Seitenleisten")),
    ("tasks", tk!("Tasks")),
];

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .tls_config(ureq::tls::TlsConfig::builder().provider(ureq::tls::TlsProvider::NativeTls).root_certs(ureq::tls::RootCerts::PlatformVerifier).build())
        .timeout_global(Some(Duration::from_secs(60)))
        .user_agent(concat!("Nebula/", env!("CARGO_PKG_VERSION")))
        .build()
        .into()
}

fn get_json(url: &str) -> anyhow::Result<Value> {
    let mut resp = agent().get(url).call()?;
    Ok(serde_json::from_reader(resp.body_mut().as_reader())?)
}

pub fn get_bytes(url: &str) -> anyhow::Result<Vec<u8>> {
    let mut resp = agent().get(url).call()?;
    Ok(resp.body_mut().with_config().limit(200 * 1024 * 1024).read_to_vec()?)
}

fn entry(v: &Value) -> LibraryEntry {
    let s = |p: &str| v.pointer(p).and_then(|x| x.as_str()).unwrap_or("").to_string();
    LibraryEntry {
        identifier: s("/identifier"),
        name: s("/name"),
        organization: s("/organization/name"),
        description: s("/description"),
        version: s("/latest_version/name"),
        source_url: s("/latest_version/source_url"),
        categories: v["categories"].as_array().map(|a| a.iter().filter_map(|c| c.as_str().map(str::to_string)).collect()).unwrap_or_default(),
        downloads: v["downloads"].as_u64().unwrap_or(0),
        logo: v["logo2x"].as_str().or(v["logo"].as_str()).map(str::to_string),
        readme: v["readme"].as_str().map(str::to_string),
        repository: v["repository"].as_str().filter(|s| !s.is_empty()).map(str::to_string),
        entitlements: v
            .pointer("/latest_version/entitlements")
            .and_then(|e| e.as_object())
            .map(|o| {
                o.iter()
                    .filter(|(_, v)| v.as_bool() != Some(false))
                    .map(|(k, v)| match v.as_str() {
                        Some(s) => format!("{k}: {s}"),
                        None => k.clone(),
                    })
                    .collect()
            })
            .unwrap_or_default(),
    }
}

fn encode(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// The whole catalog in one request.
pub fn catalog() -> anyhow::Result<Vec<LibraryEntry>> {
    let bytes = get_bytes(&format!("{API}?page_size=2000"))?;
    let v: Value = serde_json::from_slice(&bytes)?;
    let _ = std::fs::write(cache_path(), &bytes);
    Ok(parse_catalog(&v))
}

fn cache_path() -> PathBuf {
    crate::settings::config_dir().join("library-cache.json")
}

fn parse_catalog(v: &Value) -> Vec<LibraryEntry> {
    v["results"].as_array().map(|a| a.iter().map(entry).collect()).unwrap_or_default()
}

/// The catalog from the last successful download, if any.
pub fn cached_catalog() -> Option<Vec<LibraryEntry>> {
    let bytes = std::fs::read(cache_path()).ok()?;
    let v: Value = serde_json::from_slice(&bytes).ok()?;
    Some(parse_catalog(&v)).filter(|c| !c.is_empty())
}

/// Entries matching a query and category, best matches first.
pub fn filter<'a>(all: &'a [LibraryEntry], query: &str, category: &str) -> Vec<&'a LibraryEntry> {
    let q = query.trim().to_lowercase();
    let words: Vec<&str> = q.split_whitespace().collect();
    let mut hits: Vec<(u8, &LibraryEntry)> = all
        .iter()
        .filter(|e| category.is_empty() || e.categories.iter().any(|c| c == category))
        .filter_map(|e| {
            if q.is_empty() {
                return Some((0, e));
            }
            let name = e.name.to_lowercase();
            let rest = format!("{} {} {}", e.description, e.identifier, e.organization).to_lowercase();
            if name == q {
                Some((0, e))
            } else if name.contains(&q) {
                Some((1, e))
            } else if words.iter().all(|w| name.contains(w)) {
                Some((2, e))
            } else if words.iter().all(|w| name.contains(w) || rest.contains(w)) {
                Some((3, e))
            } else {
                None
            }
        })
        .collect();
    hits.sort_by(|a, b| a.0.cmp(&b.0).then(b.1.downloads.cmp(&a.1.downloads)));
    hits.into_iter().map(|(_, e)| e).collect()
}

pub fn details(identifier: &str) -> anyhow::Result<LibraryEntry> {
    Ok(entry(&get_json(&format!("{API}{}/", encode(identifier)))?))
}

/// Downloads and installs the latest version of a library extension.
pub fn install(entry: &LibraryEntry) -> anyhow::Result<Extension> {
    let url = if entry.source_url.is_empty() { details(&entry.identifier)?.source_url } else { entry.source_url.clone() };
    let bytes = get_bytes(&url)?;
    install_zip(&bytes, &entry.identifier)
}

/// Unpacks a `.novaextension` zip into the extensions folder.
pub fn install_zip(bytes: &[u8], identifier: &str) -> anyhow::Result<Extension> {
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes))?;
    let root = extensions_dir();
    std::fs::create_dir_all(&root)?;
    let tmp = root.join(format!(".install-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp)?;
    // The bundle folder inside the archive (".../Name.novaextension/").
    let mut prefix: Option<PathBuf> = None;
    for i in 0..zip.len() {
        let f = zip.by_index(i)?;
        if let Some(p) = f.enclosed_name()
            && p.file_name().is_some_and(|n| n == "extension.json")
        {
            let parent = p.parent().map(Path::to_path_buf).unwrap_or_default();
            if prefix.as_ref().is_none_or(|old| parent.components().count() < old.components().count()) {
                prefix = Some(parent);
            }
        }
    }
    let prefix = prefix.ok_or_else(|| anyhow::anyhow!(t!("Das Archiv enthält keine extension.json")))?;
    for i in 0..zip.len() {
        let mut f = zip.by_index(i)?;
        let Some(p) = f.enclosed_name() else { continue };
        let Ok(rel) = p.strip_prefix(&prefix) else { continue };
        if rel.as_os_str().is_empty() || rel.components().any(|c| c.as_os_str() == "__MACOSX") {
            continue;
        }
        let dest = tmp.join(rel);
        if f.is_dir() {
            std::fs::create_dir_all(&dest)?;
        } else {
            if let Some(parent) = dest.parent() {
                std::fs::create_dir_all(parent)?;
            }
            let mut data = Vec::with_capacity(f.size() as usize);
            f.read_to_end(&mut data)?;
            std::fs::write(&dest, data)?;
        }
    }
    let ext = Extension::load(&tmp)?;
    let id = if identifier.is_empty() { ext.identifier.clone() } else { identifier.to_string() };
    finish_install(&tmp, &id)
}

/// Copies an extension bundle folder (development or manual install).
pub fn install_folder(src: &Path) -> anyhow::Result<Extension> {
    let ext = Extension::load(src)?;
    let root = extensions_dir();
    std::fs::create_dir_all(&root)?;
    let tmp = root.join(format!(".install-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    copy_dir(src, &tmp)?;
    finish_install(&tmp, &ext.identifier)
}

fn finish_install(tmp: &Path, identifier: &str) -> anyhow::Result<Extension> {
    let dest = extensions_dir().join(sanitize(identifier));
    if dest.exists() {
        std::fs::remove_dir_all(&dest)?;
    }
    std::fs::rename(tmp, &dest)?;
    Extension::load(&dest)
}

pub fn uninstall(ext: &Extension) -> anyhow::Result<()> {
    if ext.path.starts_with(extensions_dir()) {
        std::fs::remove_dir_all(&ext.path)?;
    }
    Ok(())
}

fn sanitize(id: &str) -> String {
    id.chars().map(|c| if c.is_ascii_alphanumeric() || "._-".contains(c) { c } else { '_' }).collect()
}

fn copy_dir(src: &Path, dest: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dest)?;
    for e in std::fs::read_dir(src)?.flatten() {
        let name = e.file_name();
        if name == ".git" || name == "node_modules" && !src.join("Scripts").exists() {
            continue;
        }
        let to = dest.join(&name);
        if e.file_type()?.is_dir() {
            copy_dir(&e.path(), &to)?;
        } else {
            std::fs::copy(e.path(), to)?;
        }
    }
    Ok(())
}

/// Decodes a PNG into RGBA pixels for a texture.
pub fn decode_png(bytes: &[u8]) -> Option<(usize, usize, Vec<u8>)> {
    let mut decoder = png::Decoder::new(std::io::Cursor::new(bytes));
    decoder.set_transformations(png::Transformations::normalize_to_color8() | png::Transformations::ALPHA);
    let mut reader = decoder.read_info().ok()?;
    let mut buf = vec![0; reader.output_buffer_size()?];
    let info = reader.next_frame(&mut buf).ok()?;
    let (w, h) = (info.width as usize, info.height as usize);
    let rgba = match info.color_type {
        png::ColorType::Rgba => buf[..w * h * 4].to_vec(),
        png::ColorType::GrayscaleAlpha => buf[..w * h * 2].chunks(2).flat_map(|p| [p[0], p[0], p[0], p[1]]).collect(),
        _ => return None,
    };
    Some((w, h, rgba))
}
