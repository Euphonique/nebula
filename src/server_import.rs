//! Importing and exporting the server list: Nebula's own JSON and FileZilla's `sitemanager.xml`.

use std::path::PathBuf;

use anyhow::{Context, Result, anyhow, bail};
use base64::Engine;

use crate::settings::{Protocol, Site, Sites};

/// A server found in an import file.
#[derive(Clone, Debug)]
pub struct Imported {
    pub site: Site,
    pub password: Option<String>,
    /// Something the user should know (unsupported option, missing password, …).
    pub note: Option<String>,
}

/// Reads an export of Nebula or a FileZilla site manager, detected by content.
pub fn parse(bytes: &[u8]) -> Result<Vec<Imported>> {
    let text = std::str::from_utf8(bytes).context(t!("Die Datei ist keine Textdatei (UTF-8)"))?;
    let text = text.trim_start_matches('\u{feff}').trim_start();
    if text.starts_with('<') {
        parse_filezilla(text)
    } else if text.starts_with('{') {
        parse_nebula(text)
    } else {
        bail!(t!("Unbekanntes Format – erwartet wird eine Nebula-Export-Datei (.json) oder FileZillas sitemanager.xml"))
    }
}

pub fn parse_nebula(json: &str) -> Result<Vec<Imported>> {
    let sites: Sites = serde_json::from_str(json).context(t!("Die JSON-Datei ist keine Nebula-Serverliste"))?;
    Ok(sites
        .sites
        .into_iter()
        .map(|site| Imported { site, password: None, note: None })
        .collect())
}

/// The server list as JSON, without passwords (they stay in the Credential Manager).
pub fn export(sites: &Sites) -> Result<Vec<u8>> {
    let list = Sites { sites: sites.sites.clone(), ..Default::default() };
    Ok(serde_json::to_vec_pretty(&list)?)
}

/// `%APPDATA%\FileZilla\sitemanager.xml`, if FileZilla is installed.
pub fn filezilla_path() -> Option<PathBuf> {
    let p = PathBuf::from(std::env::var_os("APPDATA")?).join("FileZilla").join("sitemanager.xml");
    p.is_file().then_some(p)
}

/// An entry with the same connection data already exists.
pub fn is_duplicate(existing: &[Site], site: &Site) -> bool {
    let dir = |s: &Site| s.root_path.trim().trim_end_matches('/').to_string();
    existing.iter().any(|e| {
        e.protocol == site.protocol
            && e.host.eq_ignore_ascii_case(&site.host)
            && e.port == site.port
            && e.user == site.user
            && dir(e) == dir(site)
    })
}

pub fn parse_filezilla(xml: &str) -> Result<Vec<Imported>> {
    let doc = roxmltree::Document::parse(xml).context(t!("Die XML-Datei konnte nicht gelesen werden"))?;
    let root = doc.root_element();
    let servers = root
        .children()
        .find(|n| n.has_tag_name("Servers"))
        .ok_or_else(|| anyhow!("Keine FileZilla-Serverliste (Element <Servers> fehlt)"))?;
    let mut out = Vec::new();
    walk_folder(servers, &mut Vec::new(), &mut out);
    Ok(out)
}

fn walk_folder(node: roxmltree::Node, path: &mut Vec<String>, out: &mut Vec<Imported>) {
    for child in node.children().filter(|n| n.is_element()) {
        if child.has_tag_name("Server") {
            if let Some(i) = filezilla_server(child, &path.join(" / ")) {
                out.push(i);
            }
        } else if child.has_tag_name("Folder") {
            path.push(own_text(child));
            walk_folder(child, path, out);
            path.pop();
        }
    }
}

/// Text directly inside an element (FileZilla puts folder names there).
fn own_text(node: roxmltree::Node) -> String {
    node.children()
        .filter(|n| n.is_text())
        .filter_map(|n| n.text())
        .collect::<String>()
        .trim()
        .to_string()
}

fn filezilla_server(node: roxmltree::Node, group: &str) -> Option<Imported> {
    let field = |name: &str| node.children().find(|n| n.has_tag_name(name));
    let text = |name: &str| field(name).and_then(|n| n.text()).unwrap_or("").trim().to_string();
    let mut notes = Vec::new();
    let protocol = match text("Protocol").as_str() {
        "1" => Protocol::Sftp,
        "4" => Protocol::Ftps,
        "3" => Protocol::FtpsImplicit,
        "0" => {
            notes.push(t!("FileZilla nutzt hier TLS, wenn verfügbar – falls der Server es kann, auf FTPS umstellen."));
            Protocol::Ftp
        }
        "6" | "" => Protocol::Ftp,
        // Cloud storage and other protocols Nebula doesn't speak.
        _ => return None,
    };
    let host = text("Host");
    if host.is_empty() {
        return None;
    }
    let mut name = text("Name");
    if name.is_empty() {
        name = own_text(node);
    }
    let port = text("Port").parse().unwrap_or(protocol.default_port());
    let logon = text("Logontype");
    let user = if logon == "0" { String::new() } else { text("User") };

    let password = field("Pass").and_then(|p| {
        let raw = p.text().unwrap_or("").trim();
        if raw.is_empty() {
            return None;
        }
        match p.attribute("encoding") {
            Some("base64") => base64::engine::general_purpose::STANDARD
                .decode(raw)
                .ok()
                .and_then(|b| String::from_utf8(b).ok()),
            Some("crypt") => {
                notes.push(t!("Passwort ist mit FileZillas Master-Passwort verschlüsselt und wird nicht übernommen."));
                None
            }
            _ => Some(raw.to_string()),
        }
    });
    if password.is_none() && matches!(logon.as_str(), "2" | "3") {
        notes.push(t!("FileZilla fragt hier jedes Mal nach dem Passwort – bitte in Nebula hinterlegen."));
    }
    let key_file = (protocol == Protocol::Sftp)
        .then(|| text("Keyfile"))
        .filter(|k| !k.is_empty())
        .map(PathBuf::from);

    let site = Site {
        name,
        protocol,
        host,
        port,
        user,
        root_path: remote_dir(&text("RemoteDir")),
        key_file,
        passive: text("PasvMode") != "MODE_ACTIVE",
        group: group.to_string(),
        ..Default::default()
    };
    let note = (!notes.is_empty()).then(|| notes.join(" "));
    Some(Imported { site, password, note })
}

/// FileZilla's path encoding: `<type> <prefix length> [<prefix>] (<length> <segment>)*`,
/// e.g. `1 0 4 home 3 www` → `/home/www`. Segments may contain spaces.
pub fn remote_dir(encoded: &str) -> String {
    fn number(s: &mut &str) -> Option<usize> {
        let t = s.trim_start_matches(' ');
        let end = t.find(' ').unwrap_or(t.len());
        let n = t[..end].parse().ok()?;
        *s = &t[end..];
        Some(n)
    }
    fn chunk<'a>(s: &mut &'a str, len: usize) -> Option<&'a str> {
        let t = s.strip_prefix(' ')?;
        let end = t.char_indices().nth(len).map(|(i, _)| i).unwrap_or(t.len());
        if t[..end].chars().count() != len {
            return None;
        }
        *s = &t[end..];
        Some(&t[..end])
    }
    let mut s = encoded.trim();
    if s.is_empty() {
        return String::new();
    }
    let parse = |s: &mut &str| -> Option<Vec<String>> {
        number(s)?; // server type
        let prefix_len = number(s)?;
        if prefix_len > 0 {
            chunk(s, prefix_len)?;
        }
        let mut segments = Vec::new();
        while !s.trim().is_empty() {
            let len = number(s)?;
            segments.push(chunk(s, len)?.to_string());
        }
        Some(segments)
    };
    match parse(&mut s) {
        Some(segments) => format!("/{}", segments.join("/")),
        None => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SITEMANAGER: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes" ?>
<FileZilla3 version="3.66.4" platform="windows">
    <Servers>
        <Server>
            <Host>ftp.example.com</Host>
            <Port>21</Port>
            <Protocol>4</Protocol>
            <Type>0</Type>
            <User>web1</User>
            <Pass encoding="base64">Z2VoZWltIcOk</Pass>
            <Logontype>1</Logontype>
            <PasvMode>MODE_DEFAULT</PasvMode>
            <Name>Einzeln</Name>
            <RemoteDir>1 0 8 httpdocs</RemoteDir>
        </Server>
        <Folder expanded="1">Kunden
            <Folder>Müller GmbH
                <Server>
                    <Host>ssh.example.org</Host>
                    <Port>2222</Port>
                    <Protocol>1</Protocol>
                    <User>deploy</User>
                    <Logontype>5</Logontype>
                    <Keyfile>C:\keys\id_ed25519.ppk</Keyfile>
                    <Name>Shop</Name>
                    <RemoteDir>1 0 3 var 3 www 9 mein shop</RemoteDir>
                </Server>
            </Folder>
            <Server>
                <Host>s3.amazonaws.com</Host>
                <Protocol>5</Protocol>
                <Name>Bucket</Name>
            </Server>
            <Server>
                <Host>old.example.net</Host>
                <Protocol>0</Protocol>
                <User>x</User>
                <Logontype>2</Logontype>
                <PasvMode>MODE_ACTIVE</PasvMode>
                <Name>Alt</Name>
            </Server>
        </Folder>
    </Servers>
</FileZilla3>"#;

    #[test]
    fn filezilla_sites_with_folders() {
        let list = parse(SITEMANAGER.as_bytes()).unwrap();
        let names: Vec<_> = list.iter().map(|i| (i.site.name.as_str(), i.site.group.as_str())).collect();
        assert_eq!(names, [("Einzeln", ""), ("Shop", "Kunden / Müller GmbH"), ("Alt", "Kunden")]);

        let a = &list[0];
        assert_eq!(a.site.protocol, Protocol::Ftps);
        assert_eq!(a.site.root_path, "/httpdocs");
        assert_eq!(a.password.as_deref(), Some("geheim!ä"));
        assert!(a.site.passive);

        let b = &list[1];
        assert_eq!((b.site.protocol, b.site.port), (Protocol::Sftp, 2222));
        assert_eq!(b.site.root_path, "/var/www/mein shop");
        assert_eq!(b.site.key_file.as_deref(), Some(std::path::Path::new(r"C:\keys\id_ed25519.ppk")));

        let c = &list[2];
        assert_eq!((c.site.protocol, c.site.port), (Protocol::Ftp, 21));
        assert!(!c.site.passive);
        assert!(c.note.as_deref().unwrap().contains("Passwort"));
    }

    #[test]
    fn filezilla_implicit_ftps() {
        let xml = r#"<FileZilla3><Servers><Server><Host>h</Host><Protocol>3</Protocol><Name>I</Name></Server></Servers></FileZilla3>"#;
        let list = parse(xml.as_bytes()).unwrap();
        assert_eq!((list[0].site.protocol, list[0].site.port), (Protocol::FtpsImplicit, 990));
        assert!(list[0].note.is_none());
    }

    #[test]
    fn remote_dir_encoding() {
        assert_eq!(remote_dir(""), "");
        assert_eq!(remote_dir("1 0"), "/");
        assert_eq!(remote_dir("1 0 4 home 3 www"), "/home/www");
        assert_eq!(remote_dir("1 0 5 äöü x"), "/äöü x");
        assert_eq!(remote_dir("1 0 9 kaputt"), "");
    }

    #[test]
    fn nebula_round_trip_and_duplicates() {
        let mut sites = Sites::default();
        sites.sites.push(Site { name: "A".into(), host: "Example.com".into(), user: "u".into(), root_path: "/www/".into(), group: "Kunden".into(), ..Default::default() });
        let back = parse(&export(&sites).unwrap()).unwrap();
        assert_eq!(back.len(), 1);
        assert_eq!(back[0].site.group, "Kunden");
        let mut other = back[0].site.clone();
        other.host = "example.COM".into();
        other.root_path = "/www".into();
        assert!(is_duplicate(&sites.sites, &other));
        other.user = "v".into();
        assert!(!is_duplicate(&sites.sites, &other));
    }

    #[test]
    fn unknown_format() {
        assert!(parse(b"hello").is_err());
        assert!(parse(b"<html></html>").is_err());
    }
}
