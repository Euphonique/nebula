//! Live preview: a WebView2 child window next to the editor. Pages are
//! served through a custom protocol so unsaved buffers and remote files
//! can be shown without writing anything to disk.

use std::borrow::Cow;

use crossbeam_channel::{Receiver, Sender};
use eframe::egui::{self, Rect};
use wry::http::{Response, StatusCode};
use wry::{RequestAsyncResponder, WebContext, WebView, WebViewBuilder};

/// Origin of served pages (WebView2 maps `nebula://localhost/` to this).
pub const ORIGIN: &str = "http://nebula.localhost";

/// A resource requested by the page; the app answers it.
pub struct ProtocolRequest {
    /// Decoded URL path, always starting with `/`.
    pub path: String,
    pub responder: RequestAsyncResponder,
}

/// Runs in every page before its own scripts: keeps the scroll position
/// across reloads and lets the app swap stylesheets without a reload.
const INIT_SCRIPT: &str = r#"(function () {
  var key = '__nebula_scroll:' + location.href.split('#')[0];
  var restore = function () {
    var v = sessionStorage.getItem(key);
    if (v) { var p = v.split(','); window.scrollTo(+p[0], +p[1]); }
  };
  addEventListener('scroll', function () {
    sessionStorage.setItem(key, window.scrollX + ',' + window.scrollY);
  }, { passive: true });
  addEventListener('DOMContentLoaded', restore);
  addEventListener('load', restore);
  window.__nebulaReloadCss = function () {
    document.querySelectorAll('link[rel~="stylesheet"]').forEach(function (l) {
      var u = new URL(l.href); u.searchParams.set('__nebula', Date.now()); l.href = u.href;
    });
  };
})();"#;

pub struct Preview {
    webview: Option<WebView>,
    _context: Option<WebContext>,
    tx: Sender<ProtocolRequest>,
    rx: Receiver<ProtocolRequest>,
    /// Creating the web view failed (e.g. no WebView2 runtime).
    pub error: Option<String>,
    bounds: Option<(i32, i32, u32, u32)>,
    visible: bool,
}

impl Default for Preview {
    fn default() -> Self {
        let (tx, rx) = crossbeam_channel::unbounded();
        Self {
            webview: None,
            _context: None,
            tx,
            rx,
            error: None,
            bounds: None,
            visible: false,
        }
    }
}

impl Preview {
    pub fn is_ready(&self) -> bool {
        self.webview.is_some()
    }

    /// Creates the web view on first use.
    pub fn ensure(&mut self, frame: &eframe::Frame, ctx: &egui::Context) -> bool {
        if self.webview.is_some() {
            return true;
        }
        if self.error.is_some() {
            return false;
        }
        let mut context = WebContext::new(Some(crate::settings::config_dir().join("webview")));
        let tx = self.tx.clone();
        let repaint = ctx.clone();
        let result = WebViewBuilder::new_with_web_context(&mut context)
            .with_visible(false)
            .with_focused(false)
            .with_devtools(true)
            .with_initialization_script(INIT_SCRIPT)
            .with_asynchronous_custom_protocol("nebula".into(), move |_id, request, responder| {
                let path = percent_decode(request.uri().path());
                let _ = tx.send(ProtocolRequest { path, responder });
                repaint.request_repaint();
            })
            .with_new_window_req_handler(|url, _| {
                // Links with target=_blank open in the system browser.
                if url.starts_with("http") && !url.starts_with(ORIGIN) {
                    let _ = std::process::Command::new("explorer").arg(&url).spawn();
                }
                wry::NewWindowResponse::Deny
            })
            .build_as_child(frame);
        match result {
            Ok(webview) => {
                self.webview = Some(webview);
                self._context = Some(context);
                self.bounds = None;
                self.visible = false;
                true
            }
            Err(e) => {
                self.error = Some(format!("Die Vorschau benötigt die WebView2-Runtime von Microsoft ({e})."));
                false
            }
        }
    }

    /// Requests from the page that wait for an answer.
    pub fn requests(&self) -> Vec<ProtocolRequest> {
        self.rx.try_iter().collect()
    }

    /// Positions the web view over `rect` (points) or hides it.
    pub fn place(&mut self, rect: Option<Rect>, pixels_per_point: f32) {
        let Some(webview) = &self.webview else { return };
        let Some(rect) = rect.filter(|r| r.width() > 20.0 && r.height() > 20.0) else {
            if self.visible {
                let _ = webview.set_visible(false);
                self.visible = false;
            }
            return;
        };
        let b = (
            (rect.left() * pixels_per_point).round() as i32,
            (rect.top() * pixels_per_point).round() as i32,
            (rect.width() * pixels_per_point).round() as u32,
            (rect.height() * pixels_per_point).round() as u32,
        );
        if self.bounds != Some(b) {
            let _ = webview.set_bounds(wry::Rect {
                position: wry::dpi::PhysicalPosition::new(b.0, b.1).into(),
                size: wry::dpi::PhysicalSize::new(b.2, b.3).into(),
            });
            self.bounds = Some(b);
        }
        if !self.visible {
            let _ = webview.set_visible(true);
            self.visible = true;
        }
    }

    pub fn navigate(&mut self, url: &str) {
        if let Some(webview) = &self.webview {
            let _ = webview.load_url(url);
            let _ = webview.focus_parent();
        }
    }

    pub fn reload(&self) {
        if let Some(webview) = &self.webview {
            let _ = webview.evaluate_script("location.reload()");
        }
    }

    /// Re-fetches the page's stylesheets without reloading the page.
    pub fn reload_css(&self) {
        if let Some(webview) = &self.webview {
            let _ = webview.evaluate_script("window.__nebulaReloadCss && window.__nebulaReloadCss()");
        }
    }

    pub fn open_devtools(&self) {
        if let Some(webview) = &self.webview {
            webview.open_devtools();
        }
    }

    /// Gives keyboard focus back to the editor window.
    pub fn release_focus(&self) {
        if let Some(webview) = &self.webview {
            let _ = webview.focus_parent();
        }
    }
}

/// Answers a protocol request.
pub fn respond(responder: RequestAsyncResponder, status: u16, content_type: &str, body: impl Into<Cow<'static, [u8]>>) {
    let response = Response::builder()
        .status(StatusCode::from_u16(status).unwrap_or(StatusCode::OK))
        .header("Content-Type", content_type)
        .header("Cache-Control", "no-store")
        .header("Access-Control-Allow-Origin", "*")
        .body(body.into())
        .unwrap();
    responder.respond(response);
}

pub fn not_found(responder: RequestAsyncResponder, path: &str) {
    let body = format!("<!doctype html><meta charset=utf-8><p style=\"font:14px system-ui;color:#888;padding:2em\">Nicht gefunden: {}</p>", html_escape(path));
    respond(responder, 404, "text/html; charset=utf-8", body.into_bytes());
}

pub fn content_type(path: &str) -> &'static str {
    let ext = path.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
    match ext.as_str() {
        "html" | "htm" | "xhtml" | "shtml" | "php" | "phtml" | "md" | "markdown" => "text/html; charset=utf-8",
        "css" | "scss" | "less" => "text/css; charset=utf-8",
        "js" | "mjs" | "cjs" => "text/javascript; charset=utf-8",
        "json" | "map" => "application/json",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "avif" => "image/avif",
        "ico" => "image/x-icon",
        "bmp" => "image/bmp",
        "woff" => "font/woff",
        "woff2" => "font/woff2",
        "ttf" => "font/ttf",
        "otf" => "font/otf",
        "mp4" => "video/mp4",
        "webm" => "video/webm",
        "mp3" => "audio/mpeg",
        "ogg" => "audio/ogg",
        "wav" => "audio/wav",
        "pdf" => "application/pdf",
        "xml" => "application/xml",
        "txt" => "text/plain; charset=utf-8",
        _ => "application/octet-stream",
    }
}

pub fn is_markdown(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    lower.ends_with(".md") || lower.ends_with(".markdown")
}

/// Whether a document of this name can be shown as a page.
pub fn is_page(path: &str) -> bool {
    let ext = path.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
    matches!(ext.as_str(), "html" | "htm" | "xhtml" | "shtml" | "php" | "phtml" | "md" | "markdown" | "svg")
}

/// Renders Markdown into a styled page.
pub fn markdown_page(title: &str, source: &str, dark: bool) -> String {
    use pulldown_cmark::{Options, Parser, html};
    let mut body = String::new();
    let parser = Parser::new_ext(
        source,
        Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TASKLISTS | Options::ENABLE_FOOTNOTES,
    );
    html::push_html(&mut body, parser);
    let (bg, fg, muted, code, border, link) = if dark {
        ("#1f2023", "#e3e3e6", "#9a9ba1", "#2a2b2f", "#3a3b40", "#6cb6ff")
    } else {
        ("#ffffff", "#24292f", "#57606a", "#f3f4f6", "#d8dee4", "#0969da")
    };
    format!(
        r#"<!doctype html><html><head><meta charset="utf-8"><title>{title}</title><style>
body{{background:{bg};color:{fg};font:15px/1.6 "Segoe UI",system-ui,sans-serif;max-width:860px;margin:0 auto;padding:28px 36px}}
h1,h2{{border-bottom:1px solid {border};padding-bottom:.3em}} h1,h2,h3,h4{{line-height:1.25;margin:1.4em 0 .6em}}
a{{color:{link};text-decoration:none}} a:hover{{text-decoration:underline}}
code{{font:13px "Cascadia Mono",Consolas,monospace;background:{code};padding:.15em .35em;border-radius:5px}}
pre{{background:{code};padding:12px 16px;border-radius:8px;overflow:auto}} pre code{{padding:0;background:none}}
blockquote{{margin:0;padding:0 1em;color:{muted};border-left:4px solid {border}}}
table{{border-collapse:collapse}} th,td{{border:1px solid {border};padding:6px 12px}}
img{{max-width:100%}} hr{{border:0;border-top:1px solid {border}}}
</style></head><body>{body}</body></html>"#,
        title = html_escape(title)
    )
}

pub fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && let Ok(v) = u8::from_str_radix(std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap_or(""), 16)
        {
            out.push(v);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Percent-encodes a path for use in a URL (keeps `/`).
pub fn percent_encode(path: &str) -> String {
    let mut out = String::with_capacity(path.len());
    for b in path.bytes() {
        if b.is_ascii_alphanumeric() || b"/-_.~!$&'()*+,;=:@".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percent_round_trip() {
        let p = "/Ordner mit Leerzeichen/ä.html";
        assert_eq!(percent_decode(&percent_encode(p)), p);
        assert_eq!(percent_encode("/a b"), "/a%20b");
    }

    #[test]
    fn markdown_renders() {
        let page = markdown_page("t", "# Hallo\n\n*x*", true);
        assert!(page.contains("<h1>Hallo</h1>") && page.contains("<em>x</em>"));
    }
}
