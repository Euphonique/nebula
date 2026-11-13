//! Live preview: a WebView2 child window next to the editor. Pages are
//! served through a custom protocol so unsaved buffers and remote files
//! can be shown without writing anything to disk.

use std::borrow::Cow;

use crossbeam_channel::{Receiver, Sender};
use eframe::egui::{self, Rect};
use wry::http::{Response, StatusCode};
use wry::{RequestAsyncResponder, WebContext, WebView, WebViewBuilder};

/// Origin of served pages (WebView2 maps `nebula://localhost/` to this).
/// Where the page's own files are served (wry's custom protocol scheme).
#[cfg(windows)]
pub const ORIGIN: &str = "http://nebula.localhost";
#[cfg(not(windows))]
pub const ORIGIN: &str = "nebula://localhost";

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
  // Slim rounded scroll bars like in the rest of the app; inserted first so
  // a page's own scroll bar styles still win.
  addEventListener('DOMContentLoaded', function () {
    var s = document.createElement('style');
    s.textContent = '::-webkit-scrollbar{width:10px;height:10px;background:transparent}'
      + '::-webkit-scrollbar-track,::-webkit-scrollbar-corner{background:transparent}'
      + '::-webkit-scrollbar-thumb{background:rgba(140,140,150,.45);border-radius:5px;border:2px solid transparent;background-clip:content-box}'
      + '::-webkit-scrollbar-thumb:hover{background-color:rgba(140,140,150,.75)}';
    (document.head || document.documentElement).prepend(s);
  });
  // Console errors and warnings of the page, for agents (MCP).
  var log = window.__nebulaConsole = [];
  var keep = function (level, text) {
    if (log.length >= 200) log.shift();
    log.push({ level: level, text: String(text).slice(0, 2000), time: Date.now() });
  };
  ['error', 'warn'].forEach(function (level) {
    var orig = console[level];
    console[level] = function () {
      keep(level, Array.prototype.map.call(arguments, function (a) {
        try { return a instanceof Error ? a.stack || a.message : typeof a === 'object' ? JSON.stringify(a) : String(a); } catch (e) { return String(a); }
      }).join(' '));
      return orig.apply(console, arguments);
    };
  });
  addEventListener('error', function (e) {
    if (e.message) keep('error', e.message + (e.filename ? ' (' + e.filename + ':' + e.lineno + ')' : ''));
    else if (e.target && (e.target.src || e.target.href)) keep('error', 'Failed to load ' + (e.target.src || e.target.href));
  }, true);
  addEventListener('unhandledrejection', function (e) { keep('error', 'Unhandled promise rejection: ' + (e.reason && e.reason.stack || e.reason)); });
  // Linking with the source: Ctrl+click reveals an element in the editor,
  // Ctrl+double-click edits plain text in place; the editor highlights the
  // element at its cursor through __nebulaHighlight.
  var sig = function (el) {
    var cls = (el.getAttribute('class') || '').trim();
    var same = Array.prototype.filter.call(document.getElementsByTagName(el.tagName), function (x) { return (x.getAttribute('class') || '').trim() === cls; });
    var attrs = [];
    ['id', 'href', 'src', 'name', 'type', 'alt', 'action', 'for', 'placeholder', 'title', 'value'].forEach(function (a) {
      var v = el.getAttribute(a); if (v) attrs.push([a, v.slice(0, 300)]);
    });
    var own = Array.prototype.filter.call(el.childNodes, function (n) { return n.nodeType === 3; })
      .map(function (n) { return n.nodeValue; }).join(' ').replace(/\s+/g, ' ').trim().slice(0, 80);
    return { tag: el.tagName.toLowerCase(), cls: cls, attrs: attrs, text: own, index: same.indexOf(el) };
  };
  var post = function (m) { if (window.ipc) window.ipc.postMessage(JSON.stringify(m)); };
  addEventListener('click', function (e) {
    if (!e.ctrlKey || e.detail > 1) return;
    e.preventDefault(); e.stopPropagation();
    post({ kind: 'reveal', el: sig(e.target) });
  }, true);
  addEventListener('dblclick', function (e) {
    var el = e.target;
    if (!e.ctrlKey || el.children.length || !el.textContent.trim()) return;
    e.preventDefault(); e.stopPropagation();
    var before = el.textContent, s = sig(el);
    el.contentEditable = 'true'; el.focus();
    var done = function (save) {
      el.removeEventListener('blur', onBlur); el.removeEventListener('keydown', onKey);
      el.removeAttribute('contenteditable');
      if (!save) { el.textContent = before; return; }
      if (el.textContent !== before) post({ kind: 'edit', el: s, before: before, after: el.textContent });
    };
    var onBlur = function () { done(true); };
    var onKey = function (k) {
      if (k.key === 'Enter') { k.preventDefault(); done(true); }
      else if (k.key === 'Escape') { done(false); }
    };
    el.addEventListener('blur', onBlur); el.addEventListener('keydown', onKey);
  }, true);
  addEventListener('load', function () { post({ kind: 'ready' }); });
  var box;
  window.__nebulaHighlight = function (q) {
    if (!box) {
      box = document.createElement('div');
      box.style.cssText = 'position:absolute;pointer-events:none;z-index:2147483647;border:2px solid rgba(59,130,246,.9);background:rgba(59,130,246,.10);border-radius:3px;box-sizing:border-box';
    }
    var el = null;
    if (q) {
      var id = (q.attrs || []).filter(function (a) { return a[0] === 'id'; })[0];
      if (id) el = document.getElementById(id[1]);
      if (!el) {
        var list = Array.prototype.filter.call(document.getElementsByTagName(q.tag), function (x) {
          return (x.getAttribute('class') || '').replace(/\s+/g, ' ').trim() === q.cls.replace(/\s+/g, ' ').trim();
        });
        el = list[q.index] || null;
      }
    }
    if (!el || el === document.body || el === document.documentElement) { if (box.parentNode) box.remove(); return; }
    var r = el.getBoundingClientRect();
    box.style.left = (r.left + scrollX - 2) + 'px'; box.style.top = (r.top + scrollY - 2) + 'px';
    box.style.width = (r.width + 4) + 'px'; box.style.height = (r.height + 4) + 'px';
    if (!box.parentNode) document.body.appendChild(box);
    if (r.bottom < 0 || r.top > innerHeight) el.scrollIntoView({ block: 'center', behavior: 'smooth' });
  };
  // Page check (I9): problems with the element they belong to.
  window.__nebulaAudit = function (texts) {
    var items = [];
    var add = function (severity, key, args, el) {
      var msg = texts[key] || key;
      Object.keys(args || {}).forEach(function (k) { msg = msg.split('{' + k + '}').join(args[k]); });
      items.push({ severity: severity, message: msg, el: el ? sig(el) : null });
    };
    var all = function (sel) { return Array.prototype.slice.call(document.querySelectorAll(sel)); };
    var visible = function (el) {
      var r = el.getBoundingClientRect(), s = getComputedStyle(el);
      return r.width > 0 && r.height > 0 && s.visibility !== 'hidden' && s.display !== 'none';
    };
    // Head.
    var head = document.head || document.documentElement;
    if (!document.title.trim()) add(1, 'title', {}, head);
    if (!document.querySelector('meta[name="description" i]')) add(2, 'description', {}, head);
    if (!document.querySelector('meta[name="viewport" i]')) add(2, 'viewport', {}, head);
    if (!document.documentElement.getAttribute('lang')) add(2, 'lang', {}, document.documentElement);
    var h1 = all('h1');
    if (!h1.length && document.body) add(3, 'h1', {}, document.body);
    if (h1.length > 1) add(3, 'h1many', { n: h1.length }, h1[1]);
    // Images.
    all('img').forEach(function (img) {
      if (!img.hasAttribute('alt')) add(1, 'alt', {}, img);
      if (img.complete && img.naturalWidth === 0 && img.getAttribute('src')) add(1, 'broken', { src: img.getAttribute('src') }, img);
      var shown = img.getBoundingClientRect().width * (window.devicePixelRatio || 1);
      if (img.naturalWidth > 1000 && shown > 0 && img.naturalWidth > shown * 2.5) add(2, 'oversized', { natural: img.naturalWidth, shown: Math.round(shown) }, img);
      var entry = performance.getEntriesByName(img.currentSrc || img.src)[0];
      var bytes = entry && (entry.encodedBodySize || entry.transferSize);
      if (bytes > 400 * 1024) add(2, 'heavy', { kb: Math.round(bytes / 1024) }, img);
    });
    // Links and buttons without a name.
    all('a[href], button').forEach(function (el) {
      var name = (el.textContent || '').trim() || el.getAttribute('aria-label') || el.getAttribute('title') || (el.querySelector('img[alt]') || {}).alt;
      if (!name && visible(el)) add(1, el.tagName === 'A' ? 'emptylink' : 'emptybutton', {}, el);
    });
    // Form fields without a label.
    all('input:not([type=hidden]):not([type=submit]):not([type=button]):not([type=reset]), select, textarea').forEach(function (el) {
      var labelled = el.getAttribute('aria-label') || el.getAttribute('aria-labelledby') || el.closest('label') || (el.id && document.querySelector('label[for="' + CSS.escape(el.id) + '"]'));
      if (!labelled && visible(el)) add(2, 'label', {}, el);
    });
    // Contrast of text (WCAG AA), against the first solid background.
    var rgb = function (s) { var m = s.match(/[\d.]+/g); return m ? m.map(Number) : [0, 0, 0, 0]; };
    var lum = function (c) {
      return [0, 1, 2].map(function (i) { var v = c[i] / 255; return v <= 0.03928 ? v / 12.92 : Math.pow((v + 0.055) / 1.055, 2.4); })
        .reduce(function (a, v, i) { return a + v * [0.2126, 0.7152, 0.0722][i]; }, 0);
    };
    var background = function (el) {
      for (var e = el; e; e = e.parentElement) {
        var s = getComputedStyle(e);
        if (s.backgroundImage !== 'none') return null; // over an image: not measurable
        var c = rgb(s.backgroundColor);
        if (c.length < 4 || c[3] > 0.9) return c;
      }
      return [255, 255, 255];
    };
    var contrastHits = 0;
    all('body *').forEach(function (el) {
      if (contrastHits >= 20 || !visible(el)) return;
      var own = Array.prototype.some.call(el.childNodes, function (n) { return n.nodeType === 3 && n.nodeValue.trim(); });
      if (!own) return;
      var s = getComputedStyle(el), fg = rgb(s.color), bg = background(el);
      if (!bg || (fg.length > 3 && fg[3] < 0.5)) return;
      var a = lum(fg), b = lum(bg), ratio = (Math.max(a, b) + 0.05) / (Math.min(a, b) + 0.05);
      var size = parseFloat(s.fontSize), bold = parseInt(s.fontWeight, 10) >= 700;
      var need = size >= 24 || (bold && size >= 18.66) ? 3 : 4.5;
      if (ratio < need) { contrastHits++; add(2, 'contrast', { ratio: ratio.toFixed(1), need: need }, el); }
    });
    // Links within the site that lead nowhere (checked by asking for them).
    var seen = {}, checks = [];
    all('a[href]').forEach(function (a) {
      var u = new URL(a.getAttribute('href'), location.href);
      if (u.origin !== location.origin || u.hash && u.pathname === location.pathname || seen[u.pathname]) return;
      seen[u.pathname] = true;
      if (checks.length < 60) checks.push(fetch(u.pathname, { method: 'GET' }).then(function (r) {
        if (r.status >= 400) add(1, 'deadlink', { href: a.getAttribute('href'), status: r.status }, a);
      }).catch(function () {}));
    });
    Promise.all(checks).then(function () { post({ kind: 'audit', url: location.href, items: items }); });
  };
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
    /// Messages of the page script (element clicked, text edited).
    ipc_tx: Sender<String>,
    ipc_rx: Receiver<String>,
    /// Creating the web view failed (e.g. no WebView2 runtime).
    pub error: Option<String>,
    bounds: Option<(i32, i32, u32, u32)>,
    visible: bool,
    zoom: f64,
}

impl Default for Preview {
    fn default() -> Self {
        let (tx, rx) = crossbeam_channel::unbounded();
        let (ipc_tx, ipc_rx) = crossbeam_channel::unbounded();
        Self {
            webview: None,
            _context: None,
            tx,
            rx,
            ipc_tx,
            ipc_rx,
            error: None,
            bounds: None,
            visible: false,
            zoom: 1.0,
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
        // WebKitGTK lives in GTK, which has to be up before the first web view.
        #[cfg(target_os = "linux")]
        if let Err(e) = gtk::init() {
            self.error = Some(format!("GTK konnte nicht gestartet werden ({e})."));
            return false;
        }
        let mut context = WebContext::new(Some(crate::settings::config_dir().join("webview")));
        let tx = self.tx.clone();
        let repaint = ctx.clone();
        let (ipc_tx, ipc_repaint) = (self.ipc_tx.clone(), ctx.clone());
        let result = WebViewBuilder::new_with_web_context(&mut context)
            .with_visible(false)
            .with_focused(false)
            .with_devtools(true)
            .with_initialization_script(INIT_SCRIPT)
            .with_ipc_handler(move |request| {
                let _ = ipc_tx.send(request.body().clone());
                ipc_repaint.request_repaint();
            })
            .with_asynchronous_custom_protocol("nebula".into(), move |_id, request, responder| {
                let path = percent_decode(request.uri().path());
                let _ = tx.send(ProtocolRequest { path, responder });
                repaint.request_repaint();
            })
            .with_new_window_req_handler(|url, _| {
                // Links with target=_blank open in the system browser.
                if url.starts_with("http") && !url.starts_with(ORIGIN) {
                    crate::platform::open_url(&url);
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
                self.error = Some(if cfg!(windows) {
                    format!("Die Vorschau benötigt die WebView2-Runtime von Microsoft ({e}).")
                } else {
                    format!("Die Vorschau benötigt WebKitGTK (libwebkit2gtk-4.1) und X11/XWayland ({e}).")
                });
                false
            }
        }
    }

    /// Messages of the page script since the last call.
    pub fn messages(&self) -> Vec<String> {
        self.ipc_rx.try_iter().collect()
    }

    /// Runs a script in the page (no answer).
    pub fn run(&self, js: &str) {
        if let Some(w) = &self.webview {
            let _ = w.evaluate_script(js);
        }
    }

    /// Whether the web view is shown right now.
    pub fn is_visible(&self) -> bool {
        self.webview.is_some() && self.visible
    }

    /// Runs pending GTK events: the web view's drawing, IPC and resource
    /// requests arrive there, not in egui's event loop. Keeps polling while
    /// a web view exists.
    pub fn pump(&self, ctx: &egui::Context) {
        #[cfg(target_os = "linux")]
        if self.webview.is_some() {
            while gtk::events_pending() {
                gtk::main_iteration_do(false);
            }
            ctx.request_repaint_after(std::time::Duration::from_millis(30));
        }
        #[cfg(not(target_os = "linux"))]
        let _ = ctx;
    }

    /// Takes a PNG of what the preview shows; `out` gets it when done (the
    /// snapshot arrives through the GTK events `pump` runs).
    #[cfg(target_os = "linux")]
    pub fn capture_png(&self, out: std::sync::Arc<std::sync::Mutex<Option<Result<Vec<u8>, String>>>>) -> Result<(), String> {
        use webkit2gtk::WebViewExt;
        use wry::WebViewExtUnix;
        let webview = self.webview.as_ref().ok_or("The preview is not open.")?;
        webview.webview().snapshot(
            webkit2gtk::SnapshotRegion::Visible,
            webkit2gtk::SnapshotOptions::NONE,
            None::<&webkit2gtk::gio::Cancellable>,
            move |result| {
                let png = result.map_err(|e| e.to_string()).and_then(|surface| {
                    let mut bytes = Vec::new();
                    surface.write_to_png(&mut bytes).map_err(|e| e.to_string())?;
                    Ok(bytes)
                });
                if let Ok(mut slot) = out.lock() {
                    *slot = Some(png);
                }
            },
        );
        Ok(())
    }

    /// Takes a PNG of what the preview shows; `out` gets it when done.
    #[cfg(windows)]
    pub fn capture_png(&self, out: std::sync::Arc<std::sync::Mutex<Option<Result<Vec<u8>, String>>>>) -> Result<(), String> {
        use webview2_com::Microsoft::Web::WebView2::Win32::COREWEBVIEW2_CAPTURE_PREVIEW_IMAGE_FORMAT_PNG;
        use wry::WebViewExtWindows;
        let webview = self.webview.as_ref().ok_or("The preview is not open.")?;
        let core = webview.webview();
        // SAFETY: COM calls on the UI thread that owns the web view.
        unsafe {
            let stream = windows::Win32::UI::Shell::SHCreateMemStream(None).ok_or("Could not create a memory stream.")?;
            let kept = stream.clone();
            let handler = webview2_com::CapturePreviewCompletedHandler::create(Box::new(move |result| {
                let png = result.map_err(|e| e.to_string()).and_then(|()| read_stream(&kept));
                if let Ok(mut slot) = out.lock() {
                    *slot = Some(png);
                }
                Ok(())
            }));
            core.CapturePreview(COREWEBVIEW2_CAPTURE_PREVIEW_IMAGE_FORMAT_PNG, &stream, &handler).map_err(|e| e.to_string())
        }
    }

    /// Console errors and warnings the page logged (JSON), into `out`.
    pub fn console(&self, out: std::sync::Arc<std::sync::Mutex<Option<String>>>) -> Result<(), String> {
        let webview = self.webview.as_ref().ok_or("The preview is not open.")?;
        let js = "JSON.stringify({ url: location.href, title: document.title, messages: window.__nebulaConsole || [] })";
        webview
            .evaluate_script_with_callback(js, move |json| {
                if let Ok(mut slot) = out.lock() {
                    *slot = Some(json);
                }
            })
            .map_err(|e| e.to_string())
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
            // GTK drops the size of a hidden widget: once shown, apply it again.
            #[cfg(target_os = "linux")]
            let _ = webview.set_bounds(wry::Rect {
                position: wry::dpi::PhysicalPosition::new(b.0, b.1).into(),
                size: wry::dpi::PhysicalSize::new(b.2, b.3).into(),
            });
        }
    }

    /// Page zoom (1.0 = 100 %); used to fit wide device widths into the panel.
    pub fn set_zoom(&mut self, zoom: f64) {
        let Some(webview) = &self.webview else { return };
        if (self.zoom - zoom).abs() > 0.001 && webview.zoom(zoom).is_ok() {
            self.zoom = zoom;
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

    /// Goes back (or forward) in the page history after following links.
    pub fn go(&self, forward: bool) {
        if let Some(webview) = &self.webview {
            let _ = webview.evaluate_script(if forward { "history.forward()" } else { "history.back()" });
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
/// Everything in a COM memory stream.
#[cfg(windows)]
unsafe fn read_stream(stream: &windows::Win32::System::Com::IStream) -> Result<Vec<u8>, String> {
    use windows::Win32::System::Com::STREAM_SEEK_SET;
    // SAFETY: the caller hands over a valid stream.
    unsafe {
        stream.Seek(0, STREAM_SEEK_SET, None).map_err(|e| e.to_string())?;
        let mut out = Vec::new();
        let mut buf = [0u8; 64 * 1024];
        loop {
            let mut read = 0u32;
            let hr = stream.Read(buf.as_mut_ptr().cast(), buf.len() as u32, Some(&mut read));
            if hr.is_err() {
                return Err(hr.message());
            }
            if read == 0 {
                break;
            }
            out.extend_from_slice(&buf[..read as usize]);
        }
        Ok(out)
    }
}

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

/// Whether a document of this name can be shown as a page (images included).
pub fn is_page(path: &str) -> bool {
    let ext = path.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
    matches!(
        ext.as_str(),
        "html" | "htm" | "xhtml" | "shtml" | "php" | "phtml" | "md" | "markdown" | "svg"
            | "png" | "jpg" | "jpeg" | "gif" | "webp" | "avif" | "bmp" | "ico"
    )
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
