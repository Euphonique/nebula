// Nebula's implementation of the Nova extension API.
// Runs once per extension context before the extension's main script.
(function (global) {
  "use strict";

  const native = global.__native;
  const INFO = JSON.parse(global.__nebula_info);
  delete global.__native;
  delete global.__nebula_info;

  function call(op, args) {
    const r = native.call(op, JSON.stringify(args === undefined ? {} : args));
    return r === undefined ? undefined : JSON.parse(r);
  }
  function post(msg) {
    native.post(JSON.stringify(msg));
  }

  // ---- console --------------------------------------------------------------

  function fmt(v) {
    if (typeof v === "string") return v;
    if (v instanceof Error) return (v.name || "Error") + ": " + v.message + (v.stack ? "\n" + v.stack : "");
    if (v === undefined) return "undefined";
    try {
      const s = JSON.stringify(v);
      return s === undefined ? String(v) : s;
    } catch (e) {
      return String(v);
    }
  }
  function format(args) {
    args = Array.prototype.slice.call(args);
    if (typeof args[0] === "string" && /%[sdifoOj%]/.test(args[0])) {
      let first = args.shift();
      first = first.replace(/%([sdifoOj%])/g, (m, c) => {
        if (c === "%") return "%";
        if (!args.length) return m;
        const v = args.shift();
        if (c === "d" || c === "i") return String(parseInt(v, 10));
        if (c === "f") return String(parseFloat(v));
        return fmt(v);
      });
      args.unshift(first);
    }
    return args.map(fmt).join(" ");
  }
  const counts = {};
  const timersLog = {};
  let groupDepth = 0;
  function log(level, args) {
    post({ type: "log", level: level, text: "  ".repeat(groupDepth) + format(args) });
  }
  global.console = {
    log: function () { log("log", arguments); },
    info: function () { log("info", arguments); },
    debug: function () { log("info", arguments); },
    warn: function () { log("warning", arguments); },
    error: function () { log("error", arguments); },
    trace: function () { log("log", [new Error("trace").stack]); },
    assert: function (cond) {
      if (!cond) {
        const rest = Array.prototype.slice.call(arguments, 1);
        log("error", rest.length ? rest : ["Assertion failed"]);
      }
    },
    clear: function () {},
    group: function () { groupDepth++; },
    groupCollapsed: function () { groupDepth++; },
    groupEnd: function () { groupDepth = Math.max(0, groupDepth - 1); },
    count: function (label) {
      label = label || "default";
      counts[label] = (counts[label] || 0) + 1;
      log("log", [label + ": " + counts[label]]);
    },
    time: function (label) { timersLog[label || "default"] = Date.now(); },
    timeEnd: function (label) {
      label = label || "default";
      if (timersLog[label] !== undefined) {
        log("log", [label + ": " + (Date.now() - timersLog[label]) + "ms"]);
        delete timersLog[label];
      }
    },
    timeStamp: function (label) {
      label = label || "default";
      if (timersLog[label] !== undefined) log("log", [label + ": " + (Date.now() - timersLog[label]) + "ms"]);
    },
  };

  function reportError(e, where) {
    log("error", [(where ? where + ": " : "") + fmt(e)]);
  }

  // ---- callbacks and timers --------------------------------------------------------

  let nextId = 1;
  const callbacks = new Map();
  function callback(fn) {
    const id = nextId++;
    callbacks.set(id, fn);
    return id;
  }
  function promised(msg) {
    return new Promise((resolve) => {
      msg.cb = callback(resolve);
      post(msg);
    });
  }

  const timers = new Map();
  function addTimer(fn, ms, args, repeat) {
    const id = nextId++;
    timers.set(id, { fn: fn, ms: Math.max(0, +ms || 0), args: args, repeat: repeat });
    call("timer.set", { id: id, ms: Math.max(0, +ms || 0) });
    return id;
  }
  global.setTimeout = function (fn, ms) { return addTimer(fn, ms, Array.prototype.slice.call(arguments, 2), false); };
  global.setInterval = function (fn, ms) { return addTimer(fn, Math.max(1, +ms || 0), Array.prototype.slice.call(arguments, 2), true); };
  global.clearTimeout = global.clearInterval = function (id) {
    if (timers.delete(id)) call("timer.clear", { id: id });
  };
  global.setImmediate = function (fn) { return addTimer(fn, 0, Array.prototype.slice.call(arguments, 1), false); };
  global.clearImmediate = global.clearTimeout;
  if (!global.queueMicrotask) global.queueMicrotask = (fn) => Promise.resolve().then(fn);

  // ---- encoding ---------------------------------------------------------------------

  const B64 = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
  function bytesToB64(bytes) {
    let out = "";
    for (let i = 0; i < bytes.length; i += 3) {
      const n = (bytes[i] << 16) | ((bytes[i + 1] || 0) << 8) | (bytes[i + 2] || 0);
      out += B64[(n >> 18) & 63] + B64[(n >> 12) & 63] + (i + 1 < bytes.length ? B64[(n >> 6) & 63] : "=") + (i + 2 < bytes.length ? B64[n & 63] : "=");
    }
    return out;
  }
  function b64ToBytes(s) {
    s = String(s).replace(/[^A-Za-z0-9+/\-_]/g, "");
    const out = new Uint8Array(Math.floor((s.length * 3) / 4));
    let buf = 0, bits = 0, j = 0;
    for (let i = 0; i < s.length; i++) {
      let c = s[i];
      if (c === "-") c = "+";
      if (c === "_") c = "/";
      buf = (buf << 6) | B64.indexOf(c);
      bits += 6;
      if (bits >= 8) {
        bits -= 8;
        out[j++] = (buf >> bits) & 255;
      }
    }
    return out.slice(0, j);
  }
  function utf8Encode(str) {
    const out = [];
    for (let i = 0; i < str.length; i++) {
      let c = str.charCodeAt(i);
      if (c >= 0xd800 && c < 0xdc00 && i + 1 < str.length) {
        const d = str.charCodeAt(i + 1);
        if (d >= 0xdc00 && d < 0xe000) {
          c = 0x10000 + ((c - 0xd800) << 10) + (d - 0xdc00);
          i++;
        }
      }
      if (c < 0x80) out.push(c);
      else if (c < 0x800) out.push(0xc0 | (c >> 6), 0x80 | (c & 63));
      else if (c < 0x10000) out.push(0xe0 | (c >> 12), 0x80 | ((c >> 6) & 63), 0x80 | (c & 63));
      else out.push(0xf0 | (c >> 18), 0x80 | ((c >> 12) & 63), 0x80 | ((c >> 6) & 63), 0x80 | (c & 63));
    }
    return new Uint8Array(out);
  }
  function utf8Decode(bytes) {
    let out = "";
    const chunk = [];
    for (let i = 0; i < bytes.length; ) {
      const b = bytes[i];
      let c;
      if (b < 0x80) { c = b; i += 1; }
      else if (b >= 0xc0 && b < 0xe0) { c = ((b & 31) << 6) | (bytes[i + 1] & 63); i += 2; }
      else if (b >= 0xe0 && b < 0xf0) { c = ((b & 15) << 12) | ((bytes[i + 1] & 63) << 6) | (bytes[i + 2] & 63); i += 3; }
      else if (b >= 0xf0) { c = ((b & 7) << 18) | ((bytes[i + 1] & 63) << 12) | ((bytes[i + 2] & 63) << 6) | (bytes[i + 3] & 63); i += 4; }
      else { c = 0xfffd; i += 1; }
      if (c > 0xffff) {
        c -= 0x10000;
        chunk.push(0xd800 + (c >> 10), 0xdc00 + (c & 1023));
      } else chunk.push(c);
      if (chunk.length > 8000) {
        out += String.fromCharCode.apply(null, chunk);
        chunk.length = 0;
      }
    }
    return out + String.fromCharCode.apply(null, chunk);
  }
  function toBytes(data) {
    if (data instanceof Uint8Array) return data;
    if (data instanceof ArrayBuffer) return new Uint8Array(data);
    if (ArrayBuffer.isView(data)) return new Uint8Array(data.buffer, data.byteOffset, data.byteLength);
    return utf8Encode(String(data));
  }
  global.btoa = function (s) {
    s = String(s);
    const bytes = new Uint8Array(s.length);
    for (let i = 0; i < s.length; i++) bytes[i] = s.charCodeAt(i) & 255;
    return bytesToB64(bytes);
  };
  global.atob = function (s) {
    const b = b64ToBytes(s);
    let out = "";
    for (let i = 0; i < b.length; i++) out += String.fromCharCode(b[i]);
    return out;
  };
  global.TextEncoder = class TextEncoder {
    get encoding() { return "utf-8"; }
    encode(s) { return utf8Encode(String(s === undefined ? "" : s)); }
  };
  global.TextDecoder = class TextDecoder {
    constructor(enc) { this.encoding = (enc || "utf-8").toLowerCase(); }
    decode(data) {
      if (data === undefined) return "";
      const b = toBytes(data);
      if (this.encoding === "latin1" || this.encoding === "ascii" || this.encoding === "iso-8859-1") {
        let s = "";
        for (let i = 0; i < b.length; i++) s += String.fromCharCode(b[i]);
        return s;
      }
      return utf8Decode(b);
    }
  };

  // ---- disposables and events --------------------------------------------------------

  class Disposable {
    constructor(fn) {
      if (fn !== undefined && typeof fn !== "function") throw new Error("Disposable argument must be a function");
      this._fn = fn;
    }
    dispose() {
      const fn = this._fn;
      this._fn = null;
      if (fn) fn();
    }
    static isDisposable(o) { return !!o && typeof o.dispose === "function"; }
  }
  class CompositeDisposable {
    constructor() { this._items = new Set(); this._disposed = false; }
    add(o) {
      if (!Disposable.isDisposable(o)) throw new Error("Object added to composite must be disposable");
      if (!this._disposed) this._items.add(o);
    }
    remove(o) { this._items.delete(o); }
    delete(o) { this._items.delete(o); }
    clear() { this._items.clear(); }
    dispose() {
      this._disposed = true;
      const items = Array.from(this._items);
      this._items.clear();
      for (const i of items) {
        try { i.dispose(); } catch (e) { reportError(e, "dispose"); }
      }
    }
  }
  class Emitter {
    constructor() { this._l = new Map(); }
    _list(n) {
      if (!this._l.has(n)) this._l.set(n, []);
      return this._l.get(n);
    }
    on(n, cb) {
      const e = { cb: cb };
      this._list(n).push(e);
      return new Disposable(() => { const l = this._list(n); const i = l.indexOf(e); if (i >= 0) l.splice(i, 1); });
    }
    once(n, cb) {
      const e = { cb: cb, once: true };
      this._list(n).push(e);
      return new Disposable(() => { const l = this._list(n); const i = l.indexOf(e); if (i >= 0) l.splice(i, 1); });
    }
    preempt(n, cb) {
      const e = { cb: cb };
      this._list(n).unshift(e);
      return new Disposable(() => { const l = this._list(n); const i = l.indexOf(e); if (i >= 0) l.splice(i, 1); });
    }
    emit(n) {
      const args = Array.prototype.slice.call(arguments, 1);
      const list = this._list(n).slice();
      for (const e of list) {
        if (e.once) { const l = this._list(n); const i = l.indexOf(e); if (i >= 0) l.splice(i, 1); }
        try { e.cb.apply(undefined, args); } catch (err) { reportError(err, n); }
      }
    }
    clear(n) { if (n === undefined) this._l.clear(); else this._l.delete(n); }
    dispose() { this._l.clear(); }
  }
  /// Listener list used by API objects: returns a Disposable.
  function listen(emitter, name, cb, thisValue) {
    return emitter.on(name, thisValue === undefined ? cb : cb.bind(thisValue));
  }

  // ---- value types --------------------------------------------------------------------

  class Range {
    constructor(start, end) {
      start = +start; end = +end;
      if (end < start) throw new Error("Range end precedes start");
      Object.defineProperty(this, "start", { value: start, enumerable: true });
      Object.defineProperty(this, "end", { value: end, enumerable: true });
    }
    get length() { return this.end - this.start; }
    get empty() { return this.end === this.start; }
    isEqual(r) { return !!r && r.start === this.start && r.end === this.end; }
    compare(r) {
      if (this.start !== r.start) return this.start < r.start ? -1 : 1;
      if (this.length !== r.length) return this.length < r.length ? -1 : 1;
      return 0;
    }
    containsRange(r) { return r.start >= this.start && r.end <= this.end; }
    containsIndex(i) { return i >= this.start && (i < this.end || (this.empty && i === this.start)); }
    union(r) { return new Range(Math.min(this.start, r.start), Math.max(this.end, r.end)); }
    intersection(r) {
      const s = Math.max(this.start, r.start), e = Math.min(this.end, r.end);
      return e >= s ? new Range(s, e) : new Range(0, 0);
    }
    intersectsRange(r) { return Math.max(this.start, r.start) < Math.min(this.end, r.end) || (this.empty && r.containsIndex(this.start)) || (r.empty && this.containsIndex(r.start)); }
    toString() { return "Range(" + this.start + ", " + this.end + ")"; }
  }

  class Charset {
    constructor(chars) { this._test = typeof chars === "function" ? chars : ((s) => (c) => s.indexOf(c) >= 0)(String(chars || "")); }
    contains(c) { return this._test(c); }
    concat() {
      const sets = [this].concat(Array.prototype.slice.call(arguments));
      return new Charset((c) => sets.some((s) => s._test(c)));
    }
    intersect() {
      const sets = [this].concat(Array.prototype.slice.call(arguments));
      return new Charset((c) => sets.every((s) => s._test(c)));
    }
    invert() { const t = this._test; return new Charset((c) => !t(c)); }
  }
  Charset.alphanumeric = new Charset((c) => /[\p{L}\p{M}\p{N}]/u.test(c));
  Charset.digits = new Charset((c) => /\p{Nd}/u.test(c));
  Charset.letters = new Charset((c) => /[\p{L}\p{M}]/u.test(c));
  Charset.lower = new Charset((c) => /\p{Ll}/u.test(c));
  Charset.upper = new Charset((c) => /[\p{Lu}\p{Lt}]/u.test(c));
  Charset.symbols = new Charset((c) => /\p{S}/u.test(c));
  Charset.newlines = new Charset((c) => /[\n\r\u000b\u000c\u0085\u2028\u2029]/.test(c));
  Charset.whitespace = new Charset((c) => /[\p{Zs}\t]/u.test(c));
  Charset.whitespaceAndNewlines = new Charset((c) => /[\p{Z}\n\r\u000b\u000c\u0085\t]/u.test(c));

  class Scanner {
    constructor(string) {
      this.string = String(string);
      this.location = 0;
      this.skipChars = Charset.whitespaceAndNewlines;
      this.caseSensitive = false;
    }
    get atEnd() { this._skip(); return this.location >= this.string.length; }
    _skip() {
      if (!this.skipChars) return;
      while (this.location < this.string.length && this.skipChars.contains(this.string[this.location])) this.location++;
    }
    scanString(s) {
      this._skip();
      const part = this.string.substr(this.location, s.length);
      const eq = this.caseSensitive ? part === s : part.toLowerCase() === s.toLowerCase();
      if (!eq) return null;
      this.location += s.length;
      return part;
    }
    scanUpToString(s) {
      this._skip();
      const hay = this.caseSensitive ? this.string : this.string.toLowerCase();
      let i = hay.indexOf(this.caseSensitive ? s : s.toLowerCase(), this.location);
      if (i < 0) i = this.string.length;
      if (i === this.location) return null;
      const r = this.string.slice(this.location, i);
      this.location = i;
      return r;
    }
    scanChars(cs) {
      this._skip();
      const start = this.location;
      while (this.location < this.string.length && cs.contains(this.string[this.location])) this.location++;
      return this.location > start ? this.string.slice(start, this.location) : null;
    }
    scanUpToChars(cs) {
      this._skip();
      const start = this.location;
      while (this.location < this.string.length && !cs.contains(this.string[this.location])) this.location++;
      return this.location > start ? this.string.slice(start, this.location) : null;
    }
    _scanRe(re, conv) {
      this._skip();
      re.lastIndex = this.location;
      const m = re.exec(this.string);
      if (!m || m.index !== this.location) return null;
      this.location += m[0].length;
      return conv(m[0]);
    }
    scanInt() { return this._scanRe(/[+-]?\d+/y, (s) => parseInt(s, 10)); }
    scanFloat() { return this._scanRe(/[+-]?(\d+\.?\d*|\.\d+)([eE][+-]?\d+)?/y, parseFloat); }
    scanHexInt() { return this._scanRe(/(0[xX])?[0-9a-fA-F]+/y, (s) => parseInt(s.replace(/^0[xX]/, ""), 16)); }
    scanHexFloat() { return this.scanHexInt(); }
  }

  const ColorFormat = { rgb: "rgb", hsl: "hsl", hsb: "hsb", displayP3: "p3" };
  function hslToRgb(h, s, l) {
    const k = (n) => (n + h * 12) % 12;
    const a = s * Math.min(l, 1 - l);
    const f = (n) => l - a * Math.max(-1, Math.min(k(n) - 3, Math.min(9 - k(n), 1)));
    return [f(0), f(8), f(4)];
  }
  function hsbToRgb(h, s, v) {
    const f = (n) => { const k = (n + h * 6) % 6; return v - v * s * Math.max(0, Math.min(k, 4 - k, 1)); };
    return [f(5), f(3), f(1)];
  }
  class Color {
    constructor(format, components) {
      if (!Array.isArray(components) || components.length < 3) throw new Error("Invalid color components");
      this.format = format;
      this.components = components.length === 3 ? components.concat([1]) : components.slice(0, 4);
    }
    static rgb(r, g, b, a) { return new Color(ColorFormat.rgb, [r, g, b, a === undefined ? 1 : a]); }
    static hsl(h, s, l, a) { return new Color(ColorFormat.hsl, [h, s, l, a === undefined ? 1 : a]); }
    static hsb(h, s, b, a) { return new Color(ColorFormat.hsb, [h, s, b, a === undefined ? 1 : a]); }
    static displayP3(r, g, b, a) { return new Color(ColorFormat.displayP3, [r, g, b, a === undefined ? 1 : a]); }
    _rgb() {
      const c = this.components;
      if (this.format === "hsl") return hslToRgb(c[0], c[1], c[2]).concat([c[3]]);
      if (this.format === "hsb") return hsbToRgb(c[0], c[1], c[2]).concat([c[3]]);
      return c.slice();
    }
    convert(format) {
      const rgb = this._rgb();
      if (format === "rgb" || format === "p3") return new Color(format, rgb);
      const [r, g, b, a] = rgb;
      const max = Math.max(r, g, b), min = Math.min(r, g, b), d = max - min;
      let h = 0;
      if (d) h = max === r ? ((g - b) / d) % 6 : max === g ? (b - r) / d + 2 : (r - g) / d + 4;
      h = ((h * 60 + 360) % 360) / 360;
      if (format === "hsb") return new Color(format, [h, max ? d / max : 0, max, a]);
      const l = (max + min) / 2;
      return new Color(format, [h, d ? d / (1 - Math.abs(2 * l - 1)) : 0, l, a]);
    }
  }
  class ColorInformation { constructor(range, color, kind) { this.range = range; this.color = color; this.kind = kind; } }
  class ColorPresentation { constructor(label, kind) { this.label = label; this.kind = kind; } }
  class CompletionColorPop { constructor(range) { this.range = range; } }

  class ScopeSelector {
    constructor(string) {
      this.string = String(string);
      this.classes = this.string.replace(/[^A-Za-z0-9_.*-]/g, "").split(".").filter((s) => s);
    }
    matches(other) {
      const o = typeof other === "string" ? new ScopeSelector(other) : other;
      if (this.classes.indexOf("*") >= 0 || o.classes.indexOf("*") >= 0) return true;
      return o.classes.every((c) => this.classes.indexOf(c) >= 0);
    }
  }

  class TextEdit {
    constructor(range, newText) { this.range = range; this.newText = newText; }
    static delete(range) { return new TextEdit(range, ""); }
    static insert(pos, text) { return new TextEdit(new Range(pos, pos), text); }
    static replace(range, text) { return new TextEdit(range, text); }
  }

  // ---- paths ----------------------------------------------------------------------------

  const HOME = (call("env").USERPROFILE || "C:/Users").replace(/\\/g, "/");
  const path = {
    _norm(p) { return String(p === undefined || p === null ? "" : p).replace(/\\/g, "/"); },
    isAbsolute(p) { p = path._norm(p); return /^[A-Za-z]:(\/|$)/.test(p) || p.startsWith("/"); },
    normalize(p) {
      p = path._norm(p);
      if (!p) return ".";
      const m = p.match(/^([A-Za-z]:)?(\/)?/);
      const prefix = (m[1] || "") + (m[2] || "");
      const parts = [];
      for (const seg of p.slice(prefix.length).split("/")) {
        if (!seg || seg === ".") continue;
        if (seg === "..") {
          if (parts.length && parts[parts.length - 1] !== "..") parts.pop();
          else if (!prefix) parts.push("..");
        } else parts.push(seg);
      }
      const out = prefix + parts.join("/");
      return out || (prefix ? prefix : ".");
    },
    join() {
      const parts = Array.prototype.slice.call(arguments).map(path._norm).filter((p) => p.length);
      let out = "";
      for (const p of parts) out = !out || path.isAbsolute(p) && /^[A-Za-z]:/.test(p) ? p : out.replace(/\/+$/, "") + "/" + p.replace(/^\/+/, "");
      return path.normalize(out);
    },
    dirname(p) {
      p = path._norm(p).replace(/\/+$/, "");
      const i = p.lastIndexOf("/");
      if (i < 0) return /^[A-Za-z]:$/.test(p) ? p + "/" : ".";
      if (i === 0) return "/";
      const d = p.slice(0, i);
      return /^[A-Za-z]:$/.test(d) ? d + "/" : d;
    },
    basename(p) { p = path._norm(p).replace(/\/+$/, ""); return p.slice(p.lastIndexOf("/") + 1); },
    extname(p) {
      const b = path.basename(p);
      const i = b.lastIndexOf(".");
      return i > 0 ? b.slice(i) : "";
    },
    splitext(p) {
      const e = path.extname(p);
      p = path._norm(p);
      return [e ? p.slice(0, p.length - e.length) : p, e];
    },
    expanduser(p) { p = path._norm(p); return p === "~" ? HOME : p.startsWith("~/") ? HOME + p.slice(1) : p; },
    split(p) {
      p = path._norm(p);
      const m = p.match(/^([A-Za-z]:)?\//);
      const parts = p.split("/").filter((s) => s);
      if (m) {
        if (m[1]) parts[0] = m[1] + "/";
        else parts.unshift("/");
      }
      return parts;
    },
    relative(from, to) {
      const a = path.normalize(from).split("/").filter((s) => s), b = path.normalize(to).split("/").filter((s) => s);
      let i = 0;
      while (i < a.length && i < b.length && a[i].toLowerCase() === b[i].toLowerCase()) i++;
      const up = a.slice(i).map(() => "..");
      return up.concat(b.slice(i)).join("/") || ".";
    },
  };

  function fileUri(p) {
    p = path._norm(p);
    if (/^[a-z][a-z0-9+.-]*:\/\//i.test(p)) return p;
    const enc = encodeURI(p).replace(/#/g, "%23").replace(/\?/g, "%3F");
    return "file://" + (enc.startsWith("/") ? "" : "/") + enc;
  }
  function uriToPath(uri) {
    uri = String(uri);
    if (!uri.startsWith("file://")) return uri;
    let p = decodeURIComponent(uri.slice(7));
    if (/^\/[A-Za-z]:/.test(p)) p = p.slice(1);
    return p;
  }

  // ---- configuration -----------------------------------------------------------------------

  class Configuration {
    constructor(workspace, memory) {
      this._ws = workspace;
      this._mem = memory ? new Map() : null;
      this._em = new Emitter();
    }
    _raw(key) { return this._mem ? (this._mem.has(key) ? this._mem.get(key) : null) : call("config.get", { workspace: this._ws, key: key }); }
    get(key, coerce) {
      const v = this._raw(key);
      if (v === null || v === undefined) return null;
      switch (coerce) {
        case "string": return typeof v === "string" ? v : null;
        case "number": return typeof v === "number" ? v : (typeof v === "string" && v.trim() !== "" && !isNaN(+v) ? +v : null);
        case "boolean": return typeof v === "boolean" ? v : null;
        case "array": return Array.isArray(v) ? v.map(String) : null;
        default: return v;
      }
    }
    set(key, value) {
      if (value !== undefined && value !== null && !(typeof value === "string" || typeof value === "number" || typeof value === "boolean" || Array.isArray(value)))
        throw new Error("Configuration values must be strings, numbers, booleans or arrays");
      if (this._mem) {
        const old = this._raw(key);
        if (value === undefined || value === null) this._mem.delete(key); else this._mem.set(key, value);
        this._em.emit(key, this._raw(key), old);
        post({ type: "contextChanged", key: key, value: value === undefined ? null : value });
        return;
      }
      call("config.set", { workspace: this._ws, key: key, value: value === undefined ? null : value });
    }
    remove(key) { this.set(key, undefined); }
    onDidChange(key, cb, thisValue) { return listen(this._em, key, cb, thisValue); }
    observe(key, cb, thisValue) {
      const d = this.onDidChange(key, cb, thisValue);
      try { cb.call(thisValue, this.get(key), null); } catch (e) { reportError(e, "observe " + key); }
      return d;
    }
  }

  // ---- documents and editors ----------------------------------------------------------------

  const InsertTextFormat = { PlainText: 1, Snippet: 2 };

  const documents = new Map();
  const editors = new Map();

  class TextDocument {
    constructor(id) { this._id = id; this._em = new Emitter(); this._closed = false; this._last = null; }
    _info() {
      if (this._closed) return this._last || {};
      try { this._last = call("doc.info", { id: this._id }); } catch (e) { this._closed = true; }
      return this._last || {};
    }
    get uri() { return this._info().uri; }
    get path() { const i = this._info(); return i.path === undefined ? null : i.path; }
    get isRemote() { return !!this._info().remote; }
    get isDirty() { return !!this._info().dirty; }
    get isEmpty() { return this._info().length === 0; }
    get isUntitled() { return !this._info().path; }
    get isClosed() { return this._closed; }
    get eol() { return this._info().eol || "\n"; }
    get length() { return this._info().length || 0; }
    get syntax() { const s = this._info().syntax; return s === undefined ? null : s; }
    getTextInRange(range) { return call("doc.text", { id: this._id, start: range.start, end: range.end }); }
    getLineRangeForRange(range) { const r = call("doc.lineRange", { id: this._id, start: range.start, end: range.end }); return new Range(r[0], r[1]); }
    onDidChangePath(cb, t) { return listen(this._em, "path", cb, t); }
    onDidChangeSyntax(cb, t) { return listen(this._em, "syntax", cb, t); }
    _text() { return call("doc.text", { id: this._id, start: 0 }); }
    _position(offset) { return call("doc.position", { id: this._id, offset: offset }); }
    _offset(line, column) { return call("doc.offset", { id: this._id, line: line, column: column }); }
  }

  function doc(id) {
    let d = documents.get(id);
    if (!d) { d = new TextDocument(id); documents.set(id, d); }
    return d;
  }

  class TextEditorEdit {
    constructor() { this._ops = []; }
    delete(range) { this._ops.push({ start: range.start, end: range.end, text: "" }); }
    replace(range, text, format) { this._ops.push({ start: range.start, end: range.end, text: String(text), snippet: format === InsertTextFormat.Snippet }); }
    insert(pos, text, format) { this._ops.push({ start: pos, end: pos, text: String(text), snippet: format === InsertTextFormat.Snippet, insert: true }); }
  }

  class TextEditor {
    constructor(id) { this._id = id; this._em = new Emitter(); this.document = doc(id); }
    static isTextEditor(o) { return o instanceof TextEditor; }
    get selectedRanges() { return (this.document._info().selections || [[0, 0]]).map((r) => new Range(Math.min(r[0], r[1]), Math.max(r[0], r[1]))).sort((a, b) => a.start - b.start); }
    set selectedRanges(ranges) { call("editor.select", { id: this._id, ranges: ranges.map((r) => [r.start, r.end]) }); }
    get selectedRange() { const s = this.document._info().selections || [[0, 0]]; const r = s[0]; return new Range(Math.min(r[0], r[1]), Math.max(r[0], r[1])); }
    set selectedRange(r) { this.selectedRanges = [r]; }
    get selectedText() { return this.getTextInRange(this.selectedRange); }
    get softTabs() { return !!this.document._info().softTabs; }
    set softTabs(v) {}
    get tabLength() { return this.document._info().tabLength || 4; }
    set tabLength(v) {}
    get tabText() { return this.softTabs ? " ".repeat(this.tabLength) : "\t"; }
    getTextInRange(r) { return this.document.getTextInRange(r); }
    getLineRangeForRange(r) { return this.document.getLineRangeForRange(r); }
    edit(fn, options) {
      if (inChangeCallback) throw new Error("edit() can't be called from a text change callback");
      const e = new TextEditorEdit();
      fn(e);
      if (!e._ops.length) return Promise.resolve();
      return promised({ type: "edit", doc: this._id, edits: e._ops }).then(() => undefined);
    }
    insert(text, format) {
      if (inChangeCallback) throw new Error("insert() can't be called from a text change callback");
      return promised({ type: "insert", doc: this._id, text: String(text), snippet: format === InsertTextFormat.Snippet }).then(() => undefined);
    }
    save() { return promised({ type: "save", doc: this._id }).then((ok) => { if (ok === false) throw new Error("Save failed"); }); }
    onDidChange(cb, t) { return listen(this._em, "change", cb, t); }
    onDidStopChanging(cb, t) { return listen(this._em, "stop", cb, t); }
    onWillSave(cb, t) { return listen(this._em, "willSave", cb, t); }
    onDidSave(cb, t) { return listen(this._em, "save", cb, t); }
    onDidChangeSelection(cb, t) { return listen(this._em, "selection", cb, t); }
    onDidDestroy(cb, t) { return listen(this._em, "destroy", cb, t); }
    _moveSel(fn, extend) {
      const len = this.document.length;
      this.selectedRanges = this.selectedRanges.map((r) => {
        const p = Math.max(0, Math.min(len, fn(r)));
        return extend ? new Range(Math.min(r.start, p), Math.max(r.end, p)) : new Range(p, p);
      });
    }
    _lineOf(pos) { return this.document._position(pos); }
    moveUp(n) { n = n || 1; this._moveSel((r) => { const [l, c] = this._lineOf(r.start); return this.document._offset(Math.max(0, l - n), c); }); }
    moveDown(n) { n = n || 1; this._moveSel((r) => { const [l, c] = this._lineOf(r.end); return this.document._offset(l + n, c); }); }
    moveLeft(n) { n = n || 1; this._moveSel((r) => r.start - n); }
    moveRight(n) { n = n || 1; this._moveSel((r) => r.end + n); }
    moveToTop() { this.selectedRanges = [new Range(0, 0)]; }
    moveToBottom() { const l = this.document.length; this.selectedRanges = [new Range(l, l)]; }
    moveToBeginningOfLine() { this._moveSel((r) => this.getLineRangeForRange(new Range(r.start, r.start)).start); }
    moveToEndOfLine() { this._moveSel((r) => { const lr = this.getLineRangeForRange(new Range(r.end, r.end)); const t = this.getTextInRange(lr); return lr.end - (t.endsWith("\r\n") ? 2 : t.endsWith("\n") ? 1 : 0); }); }
    _wordBounds(pos) {
      const lr = this.getLineRangeForRange(new Range(pos, pos));
      const text = this.getTextInRange(lr);
      let s = pos - lr.start, e = s;
      const w = (c) => /[\p{L}\p{N}_$]/u.test(c);
      while (s > 0 && w(text[s - 1])) s--;
      while (e < text.length && w(text[e])) e++;
      return [lr.start + s, lr.start + e];
    }
    moveToBeginningOfWord() { this._moveSel((r) => this._wordBounds(r.start)[0]); }
    moveToEndOfWord() { this._moveSel((r) => this._wordBounds(r.end)[1]); }
    addSelectionForRange(range) { this.selectedRanges = this.selectedRanges.concat([range]); }
    selectToPosition(pos) { const r = this.selectedRange; this.selectedRanges = [new Range(Math.min(r.start, pos), Math.max(r.start, pos))]; }
    selectUp(n) { n = n || 1; this._moveSel((r) => { const [l, c] = this._lineOf(r.start); return this.document._offset(Math.max(0, l - n), c); }, true); }
    selectDown(n) { n = n || 1; this._moveSel((r) => { const [l, c] = this._lineOf(r.end); return this.document._offset(l + n, c); }, true); }
    selectLeft(n) { n = n || 1; this._moveSel((r) => r.start - n, true); }
    selectRight(n) { n = n || 1; this._moveSel((r) => r.end + n, true); }
    selectToTop() { this._moveSel(() => 0, true); }
    selectToBottom() { const l = this.document.length; this._moveSel(() => l, true); }
    selectAll() { this.selectedRanges = [new Range(0, this.document.length)]; }
    selectLinesContainingCursors() { this.selectedRanges = this.selectedRanges.map((r) => this.getLineRangeForRange(r)); }
    selectToBeginningOfLine() { this.selectedRanges = this.selectedRanges.map((r) => new Range(this.getLineRangeForRange(new Range(r.start, r.start)).start, r.end)); }
    selectToEndOfLine() { this.selectedRanges = this.selectedRanges.map((r) => { const lr = this.getLineRangeForRange(new Range(r.end, r.end)); const t = this.getTextInRange(lr); return new Range(r.start, lr.end - (t.endsWith("\r\n") ? 2 : t.endsWith("\n") ? 1 : 0)); }); }
    selectWordsContainingCursors() { this.selectedRanges = this.selectedRanges.map((r) => new Range(this._wordBounds(r.start)[0], this._wordBounds(r.end)[1])); }
    selectToBeginningOfWord() { this.selectedRanges = this.selectedRanges.map((r) => new Range(this._wordBounds(r.start)[0], r.end)); }
    selectToEndOfWord() { this.selectedRanges = this.selectedRanges.map((r) => new Range(r.start, this._wordBounds(r.end)[1])); }
    scrollToCursorPosition() { post({ type: "scroll", doc: this._id, pos: this.selectedRange.start }); }
    scrollToPosition(pos) { post({ type: "scroll", doc: this._id, pos: pos }); }
    startShadowTyping(ranges) { this.selectedRanges = ranges; }
    endShadowTyping() {}
    symbolAtPosition() { return null; }
    symbolsForSelectedRanges() { return this.selectedRanges.map(() => null); }
  }

  function editor(id) {
    if (id === null || id === undefined) return null;
    let e = editors.get(id);
    if (!e) { e = new TextEditor(id); editors.set(id, e); }
    return e;
  }

  let inChangeCallback = false;

  // ---- workspace -------------------------------------------------------------------------------

  const workspaceEm = new Emitter();
  let workspacePath = call("workspace");

  const workspace = {
    get path() { return workspacePath || null; },
    config: new Configuration(true),
    context: new Configuration(false, true),
    get activeTextEditor() { return editor(call("active")); },
    get textEditors() { return call("docs").map(editor); },
    get textDocuments() { return call("docs").map(doc); },
    get debugSessions() { return []; },
    get previewRootPath() { return workspacePath || null; },
    get previewURL() { return null; },
    contains(p) {
      if (!workspacePath) return false;
      const a = path.normalize(p).toLowerCase(), b = path.normalize(workspacePath).toLowerCase();
      return a === b || a.startsWith(b + "/");
    },
    relativizePath(p) { return workspacePath ? path.relative(workspacePath, p) : p; },
    previewURLForPath() { return null; },
    onDidAddTextEditor(cb, t) {
      const d = listen(workspaceEm, "editor", cb, t);
      for (const e of this.textEditors) {
        try { cb.call(t, e); } catch (err) { reportError(err, "onDidAddTextEditor"); }
      }
      return d;
    },
    onDidOpenTextDocument(cb, t) {
      const d = listen(workspaceEm, "document", cb, t);
      for (const x of this.textDocuments) {
        try { cb.call(t, x); } catch (err) { reportError(err, "onDidOpenTextDocument"); }
      }
      return d;
    },
    onDidChangePath(cb, t) { return listen(workspaceEm, "path", cb, t); },
    onDidStartDebugSession() { return new Disposable(); },
    onDidEndDebugSession() { return new Disposable(); },
    openConfig(identifier) { post({ type: "openConfig", identifier: identifier || INFO.identifier }); },
    openFile(uri, options) {
      options = options || {};
      return promised({ type: "openFile", uri: uriToPath(uri), line: options.line, column: options.column }).then((id) => editor(id));
    },
    openNewTextDocument(options) {
      options = options || {};
      return promised({ type: "newDocument", content: options.content || "", syntax: options.syntax }).then((id) => editor(id));
    },
    reloadTasks() {},
    showInformativeMessage(m) { post({ type: "message", level: "info", text: String(m) }); },
    showWarningMessage(m) { post({ type: "message", level: "warning", text: String(m) }); },
    showErrorMessage(m) { post({ type: "message", level: "error", text: String(m) }); },
    showActionPanel(message, options, cb) {
      if (typeof options === "function") { cb = options; options = {}; }
      options = options || {};
      return promised({ type: "actionPanel", message: String(message), buttons: options.buttons || ["OK"] }).then((i) => { if (cb) cb(i === undefined ? null : i); return i; });
    },
    showInputPanel(message, options, cb) {
      if (typeof options === "function") { cb = options; options = {}; }
      options = options || {};
      return promised({ type: "inputPanel", message: String(message), label: options.label || "", placeholder: options.placeholder || "", value: options.value || "", prompt: options.prompt || "", secure: !!options.secure })
        .then((v) => { if (cb) cb(v === undefined ? null : v); return v; });
    },
    showInputPalette(message, options, cb) {
      if (typeof options === "function") { cb = options; options = {}; }
      options = options || {};
      return promised({ type: "inputPalette", message: String(message || ""), placeholder: options.placeholder || "", value: options.value || "" })
        .then((v) => { if (cb) cb(v === undefined ? null : v); return v; });
    },
    showChoicePalette(choices, options, cb) {
      if (typeof options === "function") { cb = options; options = {}; }
      options = options || {};
      choices = (choices || []).map(String);
      return promised({ type: "choicePalette", choices: choices, placeholder: options.placeholder || "" }).then((i) => {
        const choice = i === null || i === undefined ? null : choices[i];
        if (cb) cb(choice, choice === null ? null : i);
        return choice;
      });
    },
    showFileChooser(message, options, cb) {
      if (typeof options === "function") { cb = options; options = {}; }
      options = options || {};
      return promised({ type: "fileChooser", message: String(message || ""), allowFiles: options.allowFiles, allowFolders: options.allowFolders, allowMultiple: options.allowMultiple })
        .then((paths) => {
          if (paths && options.relative && workspacePath) paths = paths.map((p) => path.relative(workspacePath, p));
          if (cb) cb(paths || null);
          return paths;
        });
    },
  };

  // ---- commands ------------------------------------------------------------------------------------

  const commandHandlers = new Map();
  const commandSections = new Map();
  for (const c of INFO.commands || []) commandSections.set(c.command, c.section);
  const commands = {
    register(name, fn, thisValue) {
      commandHandlers.set(name, { fn: fn, thisValue: thisValue });
      return new Disposable(() => { if (commandHandlers.get(name) && commandHandlers.get(name).fn === fn) commandHandlers.delete(name); });
    },
    invoke(name) {
      const h = commandHandlers.get(name);
      if (!h) return Promise.reject(new Error("No command registered for " + name));
      const args = [workspace].concat(Array.prototype.slice.call(arguments, 1));
      try { return Promise.resolve(h.fn.apply(h.thisValue, args)); } catch (e) { return Promise.reject(e); }
    },
  };

  // ---- notifications -------------------------------------------------------------------------------

  class NotificationRequest {
    constructor(identifier) {
      this.identifier = identifier || "n" + nextId++ + "-" + Date.now();
      this.title = "";
      this.body = "";
      this.type = "";
      this.textInputValue = "";
      this.textInputPlaceholder = "";
      this.actions = [];
    }
  }
  const notifications = {
    add(req) {
      return promised({
        type: "notify", id: req.identifier, title: String(req.title || ""), body: String(req.body || ""), kind: req.type || "",
        value: req.textInputValue || "", placeholder: req.textInputPlaceholder || "", actions: (req.actions || []).map(String),
      }).then((r) => {
        r = r || {};
        return { identifier: req.identifier, actionIdx: r.actionIdx === undefined ? null : r.actionIdx, textInputValue: r.textInputValue };
      });
    },
    cancel(id) { post({ type: "cancelNotification", id: String(id) }); },
  };

  // ---- issues -----------------------------------------------------------------------------------------

  const IssueSeverity = { Error: 1, Warning: 2, Info: 3, Hint: 4 };
  class Issue {
    constructor() {
      this.code = null; this.message = ""; this.severity = IssueSeverity.Warning; this.source = null;
      this.textRange = null; this.line = null; this.column = null; this.endLine = null; this.endColumn = null;
    }
  }
  function num(v) {
    const n = typeof v === "number" ? v : parseInt(v, 10);
    return Number.isFinite(n) && n >= 0 ? Math.floor(n) : null;
  }
  function issueMsg(i) {
    return {
      message: String(i.message || "").trim(), code: i.code === undefined ? null : i.code, severity: num(i.severity) || 2, source: i.source || null,
      start: i.textRange ? num(i.textRange.start) : null, end: i.textRange ? num(i.textRange.end) : null,
      line: num(i.line), column: num(i.column), endLine: num(i.endLine), endColumn: num(i.endColumn),
    };
  }
  function normUri(uri) {
    if (uri && typeof uri === "object" && uri.uri) uri = uri.uri;
    uri = String(uri);
    return /^[a-z][a-z0-9+.-]*:/i.test(uri) && !/^[A-Za-z]:[\\/]/.test(uri) ? uri : fileUri(uri);
  }
  class IssueCollection {
    constructor(name) {
      this.name = name || INFO.name;
      this._id = nextId++;
      this._map = new Map();
      this._disposed = false;
    }
    _send(uri) {
      post({ type: "issues", collection: this._id, name: this.name, uri: uri, issues: (this._map.get(uri) || []).map(issueMsg) });
    }
    append(uri, issues) {
      if (this._disposed) throw new Error("IssueCollection is disposed");
      uri = normUri(uri);
      this._map.set(uri, (this._map.get(uri) || []).concat(issues || []));
      this._send(uri);
    }
    set(uri, issues) {
      if (this._disposed) throw new Error("IssueCollection is disposed");
      uri = normUri(uri);
      this._map.set(uri, (issues || []).slice());
      this._send(uri);
    }
    get(uri) { return (this._map.get(normUri(uri)) || []).slice(); }
    has(uri) { return (this._map.get(normUri(uri)) || []).length > 0; }
    remove(uri) {
      uri = normUri(uri);
      this._map.delete(uri);
      post({ type: "issues", collection: this._id, name: this.name, uri: uri, issues: [] });
    }
    clear() { this._map.clear(); post({ type: "clearIssues", collection: this._id }); }
    dispose() { this.clear(); this._disposed = true; }
  }
  class IssueParser {
    constructor(matchers) { this._issues = []; }
    pushLine() {}
    get issues() { return this._issues; }
    clear() { this._issues = []; }
  }

  // ---- completions ------------------------------------------------------------------------------------

  const CompletionItemKind = {};
  ["Type", "Class", "Category", "Interface", "Enum", "Union", "Struct", "Function", "Method", "Closure", "Constructor", "Destructor",
    "Getter", "Setter", "StaticMethod", "Constant", "Variable", "Property", "Argument", "Color", "EnumMember", "StaticProperty",
    "Expression", "Statement", "Package", "File", "Reference", "Keyword", "StyleRuleset", "StyleDirective", "StyleID", "StyleClass",
    "StylePseudoClass", "StylePseudoElement", "Tag", "TagHead", "TagTitle", "TagMeta", "TagLink", "TagBody", "TagScript", "TagStyle",
    "TagHeading", "TagSection", "TagContainer", "TagUnorderedList", "TagOrderedList", "TagListItem", "TagAnchor", "TagImage",
    "TagMedia", "TagForm", "TagFormField", "TagFramework"].forEach((k) => { CompletionItemKind[k] = k; });
  const CompletionReason = { Invoke: 0, Character: 1 };

  class CompletionItem {
    constructor(label, kind) {
      Object.defineProperty(this, "label", { value: String(label), enumerable: true });
      Object.defineProperty(this, "kind", { value: kind, enumerable: true });
      this.additionalTextEdits = null; this.color = null; this.commitChars = null; this.detail = null; this.documentation = null;
      this.filterText = null; this.insertText = null; this.insertTextFormat = InsertTextFormat.PlainText; this.range = null;
      this.path = null; this.tokenize = false;
    }
  }
  function colorComponents(c) {
    if (!c) return null;
    const rgb = c instanceof Color ? c._rgb() : null;
    return rgb ? rgb.map(Number) : null;
  }
  function completionMsg(item) {
    let insert = item.insertText === null || item.insertText === undefined ? null : String(item.insertText);
    let snippet = item.insertTextFormat === InsertTextFormat.Snippet;
    if (item.tokenize && insert !== null) {
      insert = insert.replace(/\$\[([^\]$\[]*)\]/g, (m, n, i) => "${" + (i + 1) + ":" + n + "}");
      snippet = true;
    }
    return {
      label: item.label, kind: item.kind ? String(item.kind) : null, detail: item.detail ? String(item.detail) : null,
      documentation: item.documentation ? String(item.documentation) : null, insertText: insert,
      filterText: item.filterText ? String(item.filterText) : null, snippet: snippet,
      start: item.range ? item.range.start : null, end: item.range ? item.range.end : null, color: colorComponents(item.color),
    };
  }

  // ---- assistants ----------------------------------------------------------------------------------------

  function selectorMatches(selector, syntax) {
    if (!selector) return false;
    if (Array.isArray(selector)) return selector.some((s) => selectorMatches(s, syntax));
    if (typeof selector === "string") return selector === "*" || selector === syntax;
    if (typeof selector === "object") return selector.syntax === "*" || selector.syntax === syntax;
    return false;
  }
  const completionAssistants = [];
  const issueAssistants = [];
  const assistants = {
    registerCompletionAssistant(selector, object, options) {
      const a = { selector: selector, object: object, options: options || {} };
      completionAssistants.push(a);
      return new Disposable(() => { const i = completionAssistants.indexOf(a); if (i >= 0) completionAssistants.splice(i, 1); });
    },
    registerIssueAssistant(selector, object, options) {
      const a = { selector: selector, object: object, options: options || {}, collection: new IssueCollection(), timers: new Map() };
      issueAssistants.push(a);
      for (const e of workspace.textEditors) scheduleIssues(a, e, 50);
      return new Disposable(() => { const i = issueAssistants.indexOf(a); if (i >= 0) issueAssistants.splice(i, 1); a.collection.dispose(); });
    },
    registerColorAssistant() { return new Disposable(); },
    registerTaskAssistant() { return new Disposable(); },
    registerSymbolAssistant() { return new Disposable(); },
    registerFormatter() { return new Disposable(); },
  };

  function scheduleIssues(a, ed, delay) {
    if (!selectorMatches(a.selector, ed.document.syntax)) return;
    const old = a.timers.get(ed._id);
    if (old) clearTimeout(old);
    a.timers.set(ed._id, setTimeout(() => {
      a.timers.delete(ed._id);
      if (ed.document.isClosed) return;
      const uri = ed.document.uri;
      let result;
      try { result = a.object.provideIssues(ed); } catch (e) { reportError(e, "provideIssues"); return; }
      Promise.resolve(result).then((issues) => {
        if (!ed.document.isClosed && Array.isArray(issues)) a.collection.set(uri, issues);
      }, (e) => reportError(e, "provideIssues"));
    }, delay));
  }

  // ---- processes ---------------------------------------------------------------------------------------------

  const processes = new Map();

  class ProcessMessage {
    constructor(o) {
      this.method = o.method === undefined ? null : o.method;
      this.parameters = o.params === undefined ? null : o.params;
      this.result = o.result === undefined ? null : o.result;
      this.errorCode = o.error ? o.error.code : null;
      this.errorReason = o.error ? o.error.message : null;
      this.errorData = o.error && o.error.data !== undefined ? o.error.data : null;
      this.message = this.errorReason;
      this.code = this.errorCode;
      this.data = this.errorData;
      this.name = "ProcessMessage";
    }
  }

  class WritableStdin {
    constructor(proc) { this._p = proc; }
    getWriter() {
      const p = this._p;
      return {
        ready: Promise.resolve(),
        closed: new Promise(() => {}),
        desiredSize: 1,
        write(data) {
          if (typeof data === "string") call("proc.write", { id: p._id, text: data });
          else call("proc.write", { id: p._id, base64: bytesToB64(toBytes(data)) });
          return Promise.resolve();
        },
        close() { call("proc.closeStdin", { id: p._id }); return Promise.resolve(); },
        abort() { call("proc.closeStdin", { id: p._id }); return Promise.resolve(); },
        releaseLock() {},
      };
    }
    get locked() { return false; }
  }

  class ReadableOutput {
    constructor(proc, stderr) { this._p = proc; this._err = stderr; }
    getReader() {
      const p = this._p, key = this._err ? "stderr" : "stdout";
      const queue = [];
      const waiting = [];
      let done = false;
      p._em.on(key, (line) => {
        const chunk = utf8Encode(line);
        if (waiting.length) waiting.shift()({ value: chunk, done: false }); else queue.push(chunk);
      });
      p._em.on("exit", () => { done = true; while (waiting.length) waiting.shift()({ value: undefined, done: true }); });
      return {
        read() {
          if (queue.length) return Promise.resolve({ value: queue.shift(), done: false });
          if (done) return Promise.resolve({ value: undefined, done: true });
          return new Promise((r) => waiting.push(r));
        },
        releaseLock() {},
        cancel() { return Promise.resolve(); },
        closed: new Promise(() => {}),
      };
    }
    get locked() { return false; }
  }

  class Process {
    constructor(command, options) {
      options = options || {};
      this.command = String(command);
      this.args = (options.args || []).map(String);
      this.env = Object.assign({}, options.env || {});
      this.cwd = options.cwd || workspacePath || null;
      this._stdio = options.stdio === undefined ? ["pipe", "pipe", "pipe"] : options.stdio;
      this._shell = options.shell;
      this._jsonrpc = this._stdio === "jsonrpc";
      this._em = new Emitter();
      this._id = 0;
      this.pid = 0;
      this._nextReq = 1;
      this._pending = new Map();
      this._notify = new Map();
      this._requests = new Map();
      this._buffer = { stdout: "", stderr: "" };
    }
    _pipe(i) {
      const s = this._stdio;
      return this._jsonrpc ? false : Array.isArray(s) ? s[i] === "pipe" : s === "pipe";
    }
    get stdio() { return [this.stdin, this.stdout, this.stderr]; }
    get stdin() { return this._pipe(0) ? new WritableStdin(this) : null; }
    get stdout() { return this._pipe(1) ? new ReadableOutput(this, false) : null; }
    get stderr() { return this._pipe(2) ? new ReadableOutput(this, true) : null; }
    start() {
      if (this._id) throw new Error("Process already started");
      const r = call("proc.spawn", { command: this.command, args: this.args, env: this.env, cwd: this.cwd, stdio: this._stdio, shell: this._shell === undefined ? null : this._shell });
      this._id = r.id;
      this.pid = r.pid;
      processes.set(r.id, this);
    }
    onStdout(cb, t) { if (!this._pipe(1) && !this._jsonrpc) throw new Error("stdout is not a pipe"); return listen(this._em, "stdout", cb, t); }
    onStderr(cb, t) { return listen(this._em, "stderr", cb, t); }
    onDidExit(cb, t) { return listen(this._em, "exit", cb, t); }
    signal() { this.kill(); }
    kill() { if (this._id) call("proc.kill", { id: this._id }); }
    terminate() { this.kill(); }
    _send(o) {
      if (!this._jsonrpc) throw new Error("Process is not using JSON-RPC");
      if (!this._id) throw new Error("Process is not running");
      o.jsonrpc = "2.0";
      call("proc.send", { id: this._id, body: JSON.stringify(o) });
    }
    notify(method, params) { this._send({ method: method, params: params === undefined ? null : params }); }
    request(method, params) {
      const id = this._nextReq++;
      return new Promise((resolve, reject) => {
        this._pending.set(id, { resolve: resolve, reject: reject });
        try { this._send({ id: id, method: method, params: params === undefined ? null : params }); } catch (e) { this._pending.delete(id); reject(e); }
      });
    }
    onNotify(method, cb) {
      if (!this._jsonrpc) throw new Error("Process is not using JSON-RPC");
      this._notify.set(method, cb);
      return new Disposable(() => this._notify.delete(method));
    }
    onRequest(method, cb) {
      if (!this._jsonrpc) throw new Error("Process is not using JSON-RPC");
      this._requests.set(method, cb);
      return new Disposable(() => this._requests.delete(method));
    }
    _message(text) {
      let o;
      try { o = JSON.parse(text); } catch (e) { return; }
      if (o.method !== undefined && o.id !== undefined) {
        const h = this._requests.get(o.method);
        const reply = (fn) => {
          Promise.resolve().then(fn).then(
            (result) => this._send({ id: o.id, result: result === undefined ? null : result }),
            (err) => this._send({ id: o.id, error: { code: (err && err.code) || -32603, message: String((err && err.message) || err) } }));
        };
        if (h) reply(() => h(new ProcessMessage(o)));
        else this._send({ id: o.id, error: { code: -32601, message: "Method not found: " + o.method } });
      } else if (o.method !== undefined) {
        const h = this._notify.get(o.method);
        if (h) { try { h(new ProcessMessage(o)); } catch (e) { reportError(e, o.method); } }
      } else if (o.id !== undefined) {
        const p = this._pending.get(o.id);
        if (p) {
          this._pending.delete(o.id);
          if (o.error) p.reject(new ProcessMessage(o));
          else p.resolve(o.result === undefined ? null : o.result);
        }
      }
    }
    _exit(code) {
      for (const p of this._pending.values()) p.reject(new ProcessMessage({ error: { code: -32099, message: "Process exited" } }));
      this._pending.clear();
      processes.delete(this._id);
      this._em.emit("exit", code);
    }
  }

  // ---- language clients ----------------------------------------------------------------------------------------

  const languageClients = new Set();
  const LSP_KIND = [null, "Expression", "Method", "Function", "Constructor", "Property", "Variable", "Class", "Interface", "Package", "Property",
    "Property", "EnumMember", "Constant", "Keyword", "Expression", "Color", "File", "Reference", "File", "EnumMember", "Constant", "Struct",
    "Expression", "Expression", "Type"];

  function hoverText(contents) {
    if (!contents) return "";
    if (typeof contents === "string") return contents;
    if (Array.isArray(contents)) return contents.map(hoverText).filter((s) => s).join("\n\n");
    if (contents.kind) return contents.value || "";
    if (contents.language) return "```" + contents.language + "\n" + contents.value + "\n```";
    return contents.value || "";
  }

  function nestedConfig(section) {
    return call("config.section", { section: section });
  }

  class LanguageClient {
    constructor(identifier, name, serverOptions, clientOptions) {
      this.identifier = identifier;
      this.name = name;
      this._server = serverOptions || {};
      this._client = clientOptions || {};
      this._syntaxes = (this._client.syntaxes || []).map((s) => (typeof s === "string" ? { syntax: s, languageId: s } : { syntax: s.syntax, languageId: s.languageId || s.syntax }));
      this._em = new Emitter();
      this._proc = null;
      this._ready = null;
      this._caps = {};
      this._open = new Map();
      this._dirty = new Map();
      this._notify = new Map();
      this._requests = new Map();
      this._issues = new IssueCollection(name);
      this.running = false;
      languageClients.add(this);
    }
    _languageId(syntax) { const s = this._syntaxes.find((x) => x.syntax === syntax); return s ? s.languageId : null; }
    onDidStop(cb, t) { return listen(this._em, "stop", cb, t); }
    onNotification(method, cb) { this._notify.set(method, cb); }
    onRequest(method, cb) { this._requests.set(method, cb); }
    start() {
      if (this._proc) return;
      if (this._server.type && this._server.type !== "stdio") {
        setTimeout(() => this._em.emit("stop", new Error("Language server transport '" + this._server.type + "' is not supported")), 0);
        return;
      }
      let p = this._server.path || "";
      if (p && !path.isAbsolute(p) && !/^[\w.-]+$/.test(p)) p = path.join(INFO.path, p);
      const proc = new Process(p, { args: this._server.args || [], env: this._server.env || {}, stdio: "jsonrpc", cwd: workspacePath || undefined });
      this._proc = proc;
      proc.onDidExit((code) => {
        const was = this.running;
        this.running = false;
        this._proc = null;
        this._ready = null;
        this._open.clear();
        this._issues.clear();
        this._em.emit("stop", code === 0 || this._stopping ? undefined : new Error("Language server exited with code " + code));
        this._stopping = false;
        void was;
      });
      const handleRequest = (method, fn) => proc._requests.set(method, (m) => fn(m.parameters));
      handleRequest("workspace/configuration", (params) => (params.items || []).map((i) => (i.section ? nestedConfig(i.section) : null)));
      handleRequest("client/registerCapability", () => null);
      handleRequest("client/unregisterCapability", () => null);
      handleRequest("window/workDoneProgress/create", () => null);
      handleRequest("workspace/workspaceFolders", () => (workspacePath ? [{ uri: fileUri(workspacePath), name: path.basename(workspacePath) }] : null));
      handleRequest("window/showMessageRequest", (params) => {
        const actions = (params.actions || []).map((a) => a.title);
        if (!actions.length) { workspace.showInformativeMessage(params.message); return null; }
        return workspace.showActionPanel(params.message, { buttons: actions }).then((i) => (i === null || i === undefined ? null : params.actions[i]));
      });
      handleRequest("window/showDocument", (params) => { if (params.external) nova.openURL(params.uri); else workspace.openFile(params.uri); return { success: true }; });
      handleRequest("workspace/applyEdit", (params) => applyWorkspaceEdit(params.edit).then(() => ({ applied: true }), () => ({ applied: false })));
      proc._defaultRequest = (m) => {
        const h = this._requests.get(m.method);
        if (h) return h(m.parameters);
        throw { code: -32601, message: "Method not found: " + m.method };
      };
      const origMessage = proc._message.bind(proc);
      proc._message = (text) => {
        let o;
        try { o = JSON.parse(text); } catch (e) { return; }
        if (o.method !== undefined && o.id !== undefined && !proc._requests.has(o.method)) {
          Promise.resolve().then(() => proc._defaultRequest(new ProcessMessage(o))).then(
            (result) => proc._send({ id: o.id, result: result === undefined ? null : result }),
            (err) => proc._send({ id: o.id, error: { code: (err && err.code) || -32603, message: String((err && err.message) || err) } }));
          return;
        }
        if (o.method !== undefined && o.id === undefined) { this._onNotification(o.method, o.params); return; }
        origMessage(text);
      };
      proc.onStderr((line) => post({ type: "log", level: "info", text: this.name + ": " + line.replace(/s+$/, "") }));
      try {
        proc.start();
      } catch (e) {
        this._proc = null;
        console.error(this.name + ": Start fehlgeschlagen: " + fmt(e));
        setTimeout(() => this._em.emit("stop", e instanceof Error ? e : new Error(String(e))), 0);
        return;
      }
      this.running = true;
      const init = {
        processId: null,
        clientInfo: { name: "Nebula", version: INFO.appVersion },
        locale: "de",
        rootPath: workspacePath || null,
        rootUri: workspacePath ? fileUri(workspacePath) : null,
        workspaceFolders: workspacePath ? [{ uri: fileUri(workspacePath), name: path.basename(workspacePath) }] : null,
        initializationOptions: this._client.initializationOptions,
        capabilities: {
          general: { positionEncodings: ["utf-16"] },
          workspace: { configuration: true, workspaceFolders: true, applyEdit: true, didChangeConfiguration: { dynamicRegistration: false } },
          window: { workDoneProgress: true, showMessage: { messageActionItem: { additionalPropertiesSupport: false } }, showDocument: { support: true } },
          textDocument: {
            synchronization: { didSave: true, willSave: false, willSaveWaitUntil: false, dynamicRegistration: false },
            completion: { completionItem: { snippetSupport: true, documentationFormat: ["plaintext", "markdown"], insertReplaceSupport: false }, contextSupport: true },
            hover: { contentFormat: ["plaintext", "markdown"] },
            definition: { linkSupport: true },
            publishDiagnostics: { relatedInformation: false },
            formatting: {},
            rangeFormatting: {},
            codeAction: {},
            signatureHelp: {},
          },
        },
      };
      this._ready = proc.request("initialize", init).then((result) => {
        this._caps = (result && result.capabilities) || {};
        proc.notify("initialized", {});
        for (const e of workspace.textEditors) this._didOpen(e.document);
        return true;
      }, (err) => {
        console.error(this.name + ": initialize fehlgeschlagen: " + fmt(err.errorReason || err));
        this.stop();
        throw err;
      });
      this._ready.catch(() => {});
    }
    stop() {
      const proc = this._proc;
      if (!proc) return;
      this._stopping = true;
      const kill = setTimeout(() => proc.kill(), 2000);
      proc.request("shutdown", null).then(() => { try { proc.notify("exit", null); } catch (e) {} }, () => proc.kill()).then(() => clearTimeout(kill), () => {});
    }
    _onNotification(method, params) {
      if (method === "textDocument/publishDiagnostics") {
        const id = idForUri(params.uri);
        const issues = (params.diagnostics || []).map((d) => {
          const i = new Issue();
          i.message = d.message;
          i.code = d.code === undefined ? null : d.code;
          i.severity = d.severity || IssueSeverity.Error;
          i.source = d.source || this.name;
          i.line = d.range.start.line + 1;
          i.column = d.range.start.character + 1;
          i.endLine = d.range.end.line + 1;
          i.endColumn = d.range.end.character + 1;
          void id;
          return i;
        });
        this._issues.set(params.uri, issues);
        return;
      }
      if (method === "window/showMessage") {
        const level = params.type === 1 ? "error" : params.type === 2 ? "warning" : "info";
        if (params.type <= 2) post({ type: "message", level: level, text: this.name + ": " + params.message });
        else post({ type: "log", level: "info", text: this.name + ": " + params.message });
        return;
      }
      if (method === "window/logMessage") {
        post({ type: "log", level: params.type === 1 ? "error" : params.type === 2 ? "warning" : "info", text: this.name + ": " + params.message });
        return;
      }
      const h = this._notify.get(method);
      if (h) { try { h(params); } catch (e) { reportError(e, method); } }
    }
    sendRequest(method, params) {
      if (!this._ready) return Promise.reject(new Error("Language server is not running"));
      return this._ready.then(() => { this._flushAll(); return this._proc.request(method, params === undefined ? null : params); });
    }
    sendNotification(method, params) {
      if (!this._ready) return;
      this._ready.then(() => { if (this._proc) this._proc.notify(method, params === undefined ? null : params); });
    }
    _handles(document) { return !!this._proc && !document.isClosed && this._languageId(document.syntax) !== null; }
    _didOpen(document) {
      if (!this._handles(document) || this._open.has(document._id)) return;
      const uri = document.uri;
      this._open.set(document._id, { uri: uri, version: 1 });
      this._proc.notify("textDocument/didOpen", { textDocument: { uri: uri, languageId: this._languageId(document.syntax), version: 1, text: document._text() } });
    }
    _didChange(document) {
      if (!this._open.has(document._id)) return;
      if (this._dirty.has(document._id)) clearTimeout(this._dirty.get(document._id));
      this._dirty.set(document._id, setTimeout(() => this._flush(document._id), 150));
    }
    _flush(id) {
      if (!this._dirty.has(id)) return;
      clearTimeout(this._dirty.get(id));
      this._dirty.delete(id);
      const o = this._open.get(id);
      if (!o || !this._proc) return;
      o.version++;
      this._proc.notify("textDocument/didChange", { textDocument: { uri: o.uri, version: o.version }, contentChanges: [{ text: doc(id)._text() }] });
    }
    _flushAll() { for (const id of Array.from(this._dirty.keys())) this._flush(id); }
    _didSave(document) {
      const o = this._open.get(document._id);
      if (!o || !this._proc) return;
      this._flush(document._id);
      const save = this._caps.textDocumentSync && this._caps.textDocumentSync.save;
      const params = { textDocument: { uri: o.uri } };
      if (save && save.includeText) params.text = document._text();
      this._proc.notify("textDocument/didSave", params);
    }
    _didClose(id) {
      const o = this._open.get(id);
      if (!o || !this._proc) return;
      this._open.delete(id);
      this._dirty.delete(id);
      this._proc.notify("textDocument/didClose", { textDocument: { uri: o.uri } });
    }
    _position(document, offset) { const p = document._position(offset); return { line: p[0], character: p[1] }; }
    _offset(document, pos) { return document._offset(pos.line, pos.character); }
    _complete(document, offset, trigger) {
      if (!this._open.has(document._id) || !this._caps.completionProvider) return Promise.resolve([]);
      const triggers = this._caps.completionProvider.triggerCharacters || [];
      const context = trigger && triggers.indexOf(trigger) >= 0 ? { triggerKind: 2, triggerCharacter: trigger } : { triggerKind: 1 };
      return this.sendRequest("textDocument/completion", { textDocument: { uri: this._open.get(document._id).uri }, position: this._position(document, offset), context: context })
        .then((res) => {
          const items = Array.isArray(res) ? res : (res && res.items) || [];
          return items.slice(0, 400).map((it) => {
            const edit = it.textEdit && (it.textEdit.range || it.textEdit.insert);
            return {
              label: it.label, kind: LSP_KIND[it.kind] || null, detail: it.detail || null,
              documentation: it.documentation ? (typeof it.documentation === "string" ? it.documentation : it.documentation.value) : null,
              insertText: it.textEdit ? it.textEdit.newText : it.insertText || it.label, filterText: it.filterText || null,
              snippet: it.insertTextFormat === 2,
              start: edit ? this._offset(document, edit.start) : null, end: edit ? this._offset(document, edit.end) : null, color: null,
            };
          });
        }, () => []);
    }
    _hover(document, offset) {
      if (!this._open.has(document._id) || !this._caps.hoverProvider) return Promise.resolve(null);
      return this.sendRequest("textDocument/hover", { textDocument: { uri: this._open.get(document._id).uri }, position: this._position(document, offset) })
        .then((res) => (res ? hoverText(res.contents) : null), () => null);
    }
    _definition(document, offset) {
      if (!this._open.has(document._id) || !this._caps.definitionProvider) return Promise.resolve([]);
      return this.sendRequest("textDocument/definition", { textDocument: { uri: this._open.get(document._id).uri }, position: this._position(document, offset) })
        .then((res) => {
          const list = !res ? [] : Array.isArray(res) ? res : [res];
          return list.map((l) => {
            const uri = l.targetUri || l.uri;
            const range = l.targetSelectionRange || l.targetRange || l.range;
            return { uri: uriToPath(uri), line: range.start.line + 1, column: range.start.character + 1 };
          });
        }, () => []);
    }
  }

  function idForUri(uri) {
    for (const id of call("docs")) if (doc(id).uri === uri) return id;
    return null;
  }

  function applyWorkspaceEdit(edit) {
    const changes = [];
    if (edit.changes) for (const uri of Object.keys(edit.changes)) changes.push({ uri: uri, edits: edit.changes[uri] });
    for (const dc of edit.documentChanges || []) if (dc.edits) changes.push({ uri: dc.textDocument.uri, edits: dc.edits });
    return Promise.all(changes.map((c) => {
      const open = idForUri(c.uri);
      const apply = (ed) => {
        if (!ed) return null;
        const ops = c.edits.map((e) => ({ start: ed.document._offset(e.range.start.line, e.range.start.character), end: ed.document._offset(e.range.end.line, e.range.end.character), text: e.newText }))
          .sort((a, b) => b.start - a.start);
        return ed.edit((ee) => { for (const o of ops) ee.replace(new Range(o.start, o.end), o.text); });
      };
      return open !== null ? apply(editor(open)) : workspace.openFile(c.uri).then(apply);
    }));
  }

  // ---- tree views ------------------------------------------------------------------------------------------------

  const TreeItemCollapsibleState = { None: 0, Collapsed: 1, Expanded: 2 };
  class TreeItem {
    constructor(name, state) {
      this.name = name; this.collapsibleState = state || TreeItemCollapsibleState.None;
      this.command = null; this.color = null; this.contextValue = null; this.descriptiveText = null;
      this.identifier = null; this.image = null; this.path = null; this.tooltip = null;
    }
  }
  const treeViews = new Map();
  class TreeView {
    constructor(identifier, options) {
      this._id = identifier;
      this._provider = (options || {}).dataProvider;
      this._em = new Emitter();
      this._elements = new Map();
      this._ids = new Map();
      this._selection = [];
      this.visible = true;
      treeViews.set(identifier, this);
      post({ type: "treeView", view: identifier, alive: true });
    }
    get selection() { return this._selection.slice(); }
    _idFor(el) {
      if (this._ids.has(el)) return this._ids.get(el);
      const id = nextId++;
      this._ids.set(el, id);
      this._elements.set(id, el);
      return id;
    }
    onDidChangeSelection(cb, t) { return listen(this._em, "selection", cb, t); }
    onDidChangeVisibility(cb, t) { return listen(this._em, "visibility", cb, t); }
    onDidExpandElement(cb, t) { return listen(this._em, "expand", cb, t); }
    onDidCollapseElement(cb, t) { return listen(this._em, "collapse", cb, t); }
    reload() { this._elements.clear(); this._ids = new Map(); post({ type: "treeReload", view: this._id }); return Promise.resolve(); }
    reveal() {}
    dispose() { treeViews.delete(this._id); post({ type: "treeView", view: this._id, alive: false }); }
    _children(req, elementId) {
      const el = elementId === null || elementId === undefined ? null : this._elements.get(elementId);
      Promise.resolve()
        .then(() => this._provider.getChildren(el === undefined ? null : el))
        .then((children) => Promise.all((children || []).map((c) => Promise.resolve(this._provider.getTreeItem(c)).then((item) => ({ c: c, item: item })))))
        .then((list) => {
          const items = list.filter((x) => x.item).map(({ c, item }) => ({
            id: this._idFor(c), name: String(item.name === undefined ? "" : item.name), collapsible: item.collapsibleState || 0,
            description: item.descriptiveText ? String(item.descriptiveText) : null, tooltip: item.tooltip ? String(item.tooltip) : null,
            path: item.path || null, command: item.command || null, context: item.contextValue || null,
            color: colorComponents(item.color), image: item.image || null,
          }));
          post({ type: "treeChildren", view: this._id, req: req, items: items });
        }, (e) => { reportError(e, "getChildren"); post({ type: "treeChildren", view: this._id, req: req, items: [] }); });
    }
    _event(kind, elementId) {
      const el = this._elements.get(elementId);
      if (kind === "select") {
        this._selection = el === undefined ? [] : [el];
        this._em.emit("selection", this.selection);
      } else if (kind === "expand" && el !== undefined) this._em.emit("expand", el);
      else if (kind === "collapse" && el !== undefined) this._em.emit("collapse", el);
      else if (kind === "activate" && el !== undefined) {
        Promise.resolve(this._provider.getTreeItem(el)).then((item) => { if (item && item.command) commands.invoke(item.command, el).catch((e) => reportError(e, item.command)); });
      } else if (kind === "visibility") this._em.emit("visibility");
    }
  }

  // ---- file system ----------------------------------------------------------------------------------------------

  class FileStats {
    constructor(o) { Object.assign(this, o); for (const k of ["atime", "mtime", "ctime", "birthtime"]) this[k] = new Date(o[k]); }
    isDirectory() { return this._dir; }
    isFile() { return this._file; }
    isSymbolicLink() { return this._link; }
  }
  class File {
    constructor(p, mode, encoding) {
      this.path = p;
      this._mode = mode || "r";
      this._binary = this._mode.indexOf("b") >= 0;
      this._enc = encoding || "utf8";
      this.closed = false;
      this._pos = 0;
      const exists = !!call("fs.stat", { path: p });
      if (this._mode.indexOf("x") >= 0 && exists) throw new Error("File exists: " + p);
      if (this._mode.indexOf("w") >= 0 || (this._mode.indexOf("x") >= 0)) call("fs.write", { path: p, text: "" });
      else if (this._mode.indexOf("a") >= 0 && !exists) call("fs.write", { path: p, text: "" });
      else if (!exists) throw new Error("No such file: " + p);
      this._bytes = null;
    }
    _load() {
      if (this._bytes === null) this._bytes = b64ToBytes(call("fs.read", { path: this.path, binary: true }));
      return this._bytes;
    }
    _check() { if (this.closed) throw new Error("File is closed"); }
    close() { this.closed = true; this._bytes = null; }
    tell() { this._check(); return this._pos; }
    seek(offset, from) {
      this._check();
      const len = this._load().length;
      this._pos = Math.max(0, Math.min(len, (from === 2 ? len : from === 1 ? this._pos : 0) + offset));
    }
    read(size) {
      this._check();
      const b = this._load();
      if (this._pos >= b.length) return null;
      const end = size === undefined || size === null ? b.length : Math.min(b.length, this._pos + size);
      const chunk = b.slice(this._pos, end);
      this._pos = end;
      return this._binary ? chunk.buffer : new TextDecoder(this._enc).decode(chunk);
    }
    readline() {
      this._check();
      if (this._binary) throw new Error("readline() requires text mode");
      const b = this._load();
      if (this._pos >= b.length) return "";
      let end = this._pos;
      while (end < b.length && b[end] !== 10) end++;
      if (end < b.length) end++;
      const chunk = b.slice(this._pos, end);
      this._pos = end;
      return utf8Decode(chunk);
    }
    readlines() { const out = []; let l; while ((l = this.readline()) !== "") out.push(l); return out; }
    write(value, encoding) {
      this._check();
      const bytes = typeof value === "string" ? utf8Encode(value) : toBytes(value);
      call("fs.write", { path: this.path, base64: bytesToB64(bytes), append: true });
      this._bytes = null;
      this._pos += bytes.length;
    }
  }
  const fsConstants = { F_OK: 0, R_OK: 4, W_OK: 2, X_OK: 1, START: 0, CURRENT: 1, END: 2 };
  const fs = Object.assign({
    constants: fsConstants,
    get tempdir() { return INFO.tempdir; },
    access(p, mode) { return call("fs.access", { path: p, mode: mode || 0 }); },
    chmod() {},
    copy(src, dest) { call("fs.copy", { src: src, dest: dest }); },
    copyAsync(src, dest, cb, t) { let err; try { fs.copy(src, dest); } catch (e) { err = e; } setTimeout(() => cb && cb.call(t, err), 0); },
    move(src, dest) { call("fs.move", { src: src, dest: dest }); },
    moveAsync(src, dest, cb, t) { let err; try { fs.move(src, dest); } catch (e) { err = e; } setTimeout(() => cb && cb.call(t, err), 0); },
    eject() { throw new Error("eject is not supported"); },
    listdir(p) { return call("fs.listdir", { path: p }); },
    mkdir(p) { call("fs.mkdir", { path: p }); },
    open(p, mode, encoding) { return new File(p, mode, encoding); },
    remove(p) { call("fs.remove", { path: p }); },
    rmdir(p) { call("fs.rmdir", { path: p }); },
    reveal(p) { call("fs.reveal", { path: p }); },
    stat(p) {
      const s = call("fs.stat", { path: p });
      if (!s) return null;
      return new FileStats({ size: s.size, mode: s.mode, atime: s.atime, mtime: s.mtime, ctime: s.ctime, birthtime: s.birthtime, _dir: s.isDirectory, _file: s.isFile, _link: s.isSymbolicLink });
    },
    watch(pattern, cb) {
      const em = new Emitter();
      if (cb) em.on("change", cb);
      return { onDidChange(c) { return em.on("change", c); }, dispose() { em.clear(); } };
    },
  }, fsConstants);

  // ---- fetch ------------------------------------------------------------------------------------------------------

  class Headers {
    constructor(init) {
      this._m = new Map();
      if (init instanceof Headers) init.forEach((v, k) => this.append(k, v));
      else if (Array.isArray(init)) init.forEach((p) => this.append(p[0], p[1]));
      else if (init) Object.keys(init).forEach((k) => this.append(k, init[k]));
    }
    append(k, v) { k = String(k).toLowerCase(); this._m.set(k, this._m.has(k) ? this._m.get(k) + ", " + v : String(v)); }
    set(k, v) { this._m.set(String(k).toLowerCase(), String(v)); }
    get(k) { const v = this._m.get(String(k).toLowerCase()); return v === undefined ? null : v; }
    has(k) { return this._m.has(String(k).toLowerCase()); }
    delete(k) { this._m.delete(String(k).toLowerCase()); }
    forEach(cb, t) { this._m.forEach((v, k) => cb.call(t, v, k, this)); }
    entries() { return this._m.entries(); }
    keys() { return this._m.keys(); }
    values() { return this._m.values(); }
    [Symbol.iterator]() { return this._m.entries(); }
  }
  class Response {
    constructor(body, init) {
      init = init || {};
      this._bytes = body === undefined || body === null ? new Uint8Array(0) : toBytes(body);
      this.status = init.status === undefined ? 200 : init.status;
      this.statusText = init.statusText || "";
      this.headers = new Headers(init.headers);
      this.url = init.url || "";
      this.bodyUsed = false;
      this.type = "basic";
      this.redirected = false;
    }
    get ok() { return this.status >= 200 && this.status < 300; }
    arrayBuffer() { this.bodyUsed = true; return Promise.resolve(this._bytes.slice().buffer); }
    text() { this.bodyUsed = true; return Promise.resolve(utf8Decode(this._bytes)); }
    json() { return this.text().then(JSON.parse); }
    blob() { return this.arrayBuffer(); }
    clone() { return new Response(this._bytes, { status: this.status, statusText: this.statusText, headers: this.headers, url: this.url }); }
  }
  class Request {
    constructor(input, init) {
      init = init || {};
      this.url = input instanceof Request ? input.url : String(input);
      this.method = (init.method || (input instanceof Request ? input.method : "GET")).toUpperCase();
      this.headers = new Headers(init.headers || (input instanceof Request ? input.headers : undefined));
      this.body = init.body === undefined ? (input instanceof Request ? input.body : null) : init.body;
    }
  }
  class AbortController {
    constructor() { this.signal = { aborted: false, _em: new Emitter(), addEventListener(n, cb) { this._em.on(n, cb); }, removeEventListener() {} }; }
    abort() { this.signal.aborted = true; this.signal._em.emit("abort"); }
  }
  const httpPending = new Map();
  function fetch(input, init) {
    const req = new Request(input, init);
    const headers = {};
    req.headers.forEach((v, k) => { headers[k] = v; });
    let body = null;
    if (req.body !== null && req.body !== undefined) {
      if (typeof req.body === "string") body = bytesToB64(utf8Encode(req.body));
      else if (req.body instanceof URLSearchParams) { body = bytesToB64(utf8Encode(req.body.toString())); if (!headers["content-type"]) headers["content-type"] = "application/x-www-form-urlencoded"; }
      else body = bytesToB64(toBytes(req.body));
    }
    return new Promise((resolve, reject) => {
      const id = call("http.request", { url: req.url, method: req.method, headers: headers, body: body });
      httpPending.set(id, { resolve: resolve, reject: reject });
      const signal = init && init.signal;
      if (signal) signal.addEventListener("abort", () => { if (httpPending.delete(id)) reject(new Error("The operation was aborted")); });
    });
  }

  class URLSearchParams {
    constructor(init) {
      this._p = [];
      if (typeof init === "string") {
        init.replace(/^\?/, "").split("&").filter((s) => s).forEach((pair) => {
          const i = pair.indexOf("=");
          const dec = (s) => decodeURIComponent(s.replace(/\+/g, " "));
          this._p.push(i < 0 ? [dec(pair), ""] : [dec(pair.slice(0, i)), dec(pair.slice(i + 1))]);
        });
      } else if (init) Object.keys(init).forEach((k) => this._p.push([k, String(init[k])]));
    }
    append(k, v) { this._p.push([String(k), String(v)]); }
    set(k, v) { this.delete(k); this.append(k, v); }
    get(k) { const p = this._p.find((x) => x[0] === k); return p ? p[1] : null; }
    getAll(k) { return this._p.filter((x) => x[0] === k).map((x) => x[1]); }
    has(k) { return this._p.some((x) => x[0] === k); }
    delete(k) { this._p = this._p.filter((x) => x[0] !== k); }
    forEach(cb, t) { this._p.forEach((p) => cb.call(t, p[1], p[0], this)); }
    toString() { return this._p.map((p) => encodeURIComponent(p[0]) + "=" + encodeURIComponent(p[1])).join("&"); }
    [Symbol.iterator]() { return this._p[Symbol.iterator](); }
  }
  class URL {
    constructor(url, base) {
      url = String(url);
      if (base !== undefined && !/^[a-z][a-z0-9+.-]*:/i.test(url)) {
        const b = new URL(base);
        if (url.startsWith("//")) url = b.protocol + url;
        else if (url.startsWith("/")) url = b.origin + url;
        else url = b.origin + b.pathname.replace(/[^/]*$/, "") + url;
      }
      const m = url.match(/^([a-z][a-z0-9+.-]*:)(?:\/\/(?:([^:@/]*)(?::([^@/]*))?@)?([^:/?#]*)(?::(\d+))?)?([^?#]*)(\?[^#]*)?(#.*)?$/i);
      if (!m) throw new TypeError("Invalid URL: " + url);
      this.protocol = m[1].toLowerCase();
      this.username = m[2] || "";
      this.password = m[3] || "";
      this.hostname = m[4] || "";
      this.port = m[5] || "";
      this.pathname = m[6] || (this.hostname ? "/" : "");
      this.search = m[7] && m[7] !== "?" ? m[7] : "";
      this.hash = m[8] && m[8] !== "#" ? m[8] : "";
      this.searchParams = new URLSearchParams(this.search);
    }
    get host() { return this.hostname + (this.port ? ":" + this.port : ""); }
    get origin() { return this.protocol + "//" + this.host; }
    get href() {
      const q = this.searchParams.toString();
      const auth = this.username ? this.username + (this.password ? ":" + this.password : "") + "@" : "";
      return this.protocol + (this.hostname || this.protocol === "file:" ? "//" : "") + auth + this.host + this.pathname + (q ? "?" + q : "") + this.hash;
    }
    toString() { return this.href; }
    toJSON() { return this.href; }
  }

  // ---- tasks (not supported: inert objects so extensions load) -----------------------------------

  class Task {
    constructor(name) { this.name = name; this.image = null; this.actions = {}; this.buildBeforeRunning = false; this.cleanWithBuild = false; }
    getAction(n) { return this.actions[n] || null; }
    setAction(n, a) { this.actions[n] = a; }
    setActionGroup() {}
  }
  Task.Build = "build"; Task.Run = "run"; Task.Clean = "clean";
  class TaskProcessAction { constructor(command, options) { this.command = command; this.options = options || {}; } }
  class TaskCommandAction { constructor(command, options) { this.command = command; this.options = options || {}; } }
  class TaskResolvableAction { constructor(options) { this.options = options || {}; } }
  class TaskDebugAdapterAction { constructor(type) { this.adapterType = type; } }

  // ---- environment -----------------------------------------------------------------------------------------

  const extEm = new Emitter();
  const environment = call("env");
  if (!environment.HOME) environment.HOME = HOME;
  const nova = {
    assistants: assistants,
    clipboard: {
      readText() { return Promise.resolve(call("clipboard.read")); },
      writeText(t) { call("clipboard.write", { text: String(t) }); return Promise.resolve(); },
    },
    commands: commands,
    config: new Configuration(false),
    credentials: {
      getPassword(service, user) { return call("credentials.get", { service: String(service), user: String(user) }); },
      setPassword(service, user, password) { call("credentials.set", { service: String(service), user: String(user), password: String(password) }); },
      removePassword(service, user) { call("credentials.remove", { service: String(service), user: String(user) }); },
    },
    crypto: {
      randomUUID() {
        const h = "0123456789abcdef";
        let s = "";
        for (let i = 0; i < 36; i++) {
          if (i === 8 || i === 13 || i === 18 || i === 23) s += "-";
          else if (i === 14) s += "4";
          else if (i === 19) s += h[(Math.random() * 4) | 8];
          else s += h[(Math.random() * 16) | 0];
        }
        return s;
      },
      getRandomValues(a) { for (let i = 0; i < a.length; i++) a[i] = Math.floor(Math.random() * 4294967296); return a; },
    },
    environment: environment,
    extension: {
      identifier: INFO.identifier, name: INFO.name, vendor: INFO.vendor, version: INFO.version, path: INFO.path,
      globalStoragePath: INFO.globalStoragePath, workspaceStoragePath: INFO.workspaceStoragePath,
      openChangelog() {}, openHelp() {}, openReadme() {},
    },
    fs: fs,
    locales: ["de-DE", "en"],
    path: path,
    subscriptions: new CompositeDisposable(),
    systemVersion: [14, 0, 0],
    version: [12, 0, 0],
    versionString: "12.0",
    workspace: workspace,
    notifications: notifications,
    beep() { call("beep"); },
    localize(key, value) {
      if (key === null || key === undefined) return value === null || value === undefined ? "" : value;
      const s = INFO.strings && INFO.strings[key];
      if (s !== undefined) return s;
      return value === null || value === undefined || value === "" ? key : value;
    },
    isReleasedVersion() { return true; },
    inDevMode() { return false; },
    openConfig(identifier) { post({ type: "openConfig", identifier: identifier || INFO.identifier }); },
    openURL(url, cb) { post({ type: "openUrl", url: String(url) }); if (cb) setTimeout(() => cb(true), 0); },
  };
  global.nova = nova;

  Object.assign(global, {
    Range, Charset, Scanner, Color, ColorFormat, ColorInformation, ColorPresentation, CompletionColorPop, ScopeSelector, TextEdit,
    Disposable, CompositeDisposable, Emitter, Configuration, TextEditor, TextDocument, InsertTextFormat, NotificationRequest,
    Issue, IssueCollection, IssueSeverity, IssueParser, CompletionItem, CompletionItemKind, CompletionReason, Process, ProcessMessage,
    LanguageClient, TreeView, TreeItem, TreeItemCollapsibleState, FileStats, File, Headers, Response, Request, AbortController,
    URL, URLSearchParams, Task, TaskProcessAction, TaskCommandAction, TaskResolvableAction, TaskDebugAdapterAction, fetch,
  });
  if (!global.window) global.window = global;
  if (!global.self) global.self = global;

  // ---- modules --------------------------------------------------------------------------------------------------

  const moduleCache = new Map();
  function isFile(p) { const s = call("fs.stat", { path: p }); return !!s && s.isFile; }
  function isDir(p) { const s = call("fs.stat", { path: p }); return !!s && s.isDirectory; }
  function resolveFile(p) {
    if (isFile(p)) return p;
    for (const ext of [".js", ".json", ".cjs"]) if (isFile(p + ext)) return p + ext;
    if (isDir(p)) {
      const pkg = path.join(p, "package.json");
      if (isFile(pkg)) {
        try {
          const main = JSON.parse(call("fs.read", { path: pkg })).main;
          if (main) { const r = resolveFile(path.join(p, main)); if (r) return r; }
        } catch (e) {}
      }
      for (const n of ["index.js", "index.json"]) if (isFile(path.join(p, n))) return path.join(p, n);
    }
    return null;
  }
  const scriptsDir = path.dirname(INFO.main);
  function resolveModule(name, dir) {
    if (name.startsWith("./") || name.startsWith("../") || path.isAbsolute(name)) {
      return resolveFile(path.isAbsolute(name) ? name : path.join(dir, name));
    }
    // Nova resolves bare names relative to the module first, then Scripts/.
    const local = resolveFile(path.join(dir, name)) || resolveFile(path.join(scriptsDir, name));
    if (local) return local;
    let d = dir;
    for (;;) {
      const r = resolveFile(path.join(d, "node_modules", name));
      if (r) return r;
      const parent = path.dirname(d);
      if (parent === d) break;
      d = parent;
    }
    return null;
  }
  const builtins = {
    path: () => ({ join: path.join, dirname: path.dirname, basename: (p, e) => { const b = path.basename(p); return e && b.endsWith(e) ? b.slice(0, -e.length) : b; }, extname: path.extname, normalize: path.normalize, isAbsolute: path.isAbsolute, relative: path.relative, resolve: function () { return path.join.apply(null, arguments); }, sep: "/", delimiter: ";" }),
    events: () => { class EventEmitter extends Emitter { addListener(n, c) { this.on(n, c); return this; } removeListener() { return this; } off() { return this; } removeAllListeners(n) { this.clear(n); return this; } } return { EventEmitter: EventEmitter, default: EventEmitter }; },
  };
  function makeRequire(dir) {
    return function require(name) {
      if (builtins[name] && !resolveModule(name, dir)) return builtins[name]();
      const file = resolveModule(String(name), dir);
      if (!file) throw new Error("Cannot find module '" + name + "'");
      return loadModule(file);
    };
  }
  function loadModule(file) {
    if (moduleCache.has(file)) return moduleCache.get(file).exports;
    const module = { exports: {}, id: file, filename: file, loaded: false };
    moduleCache.set(file, module);
    const src = call("fs.read", { path: file });
    if (file.toLowerCase().endsWith(".json")) {
      try { module.exports = JSON.parse(src); } catch (e) { module.exports = call("json.lenient", { text: src }); }
    } else {
      const fn = native.compile(src, file);
      fn.call(module.exports, module.exports, makeRequire(path.dirname(file)), module, file, path.dirname(file));
    }
    module.loaded = true;
    return module.exports;
  }

  // ---- event dispatch ----------------------------------------------------------------------------------------

  const stopTimers = new Map();
  let mainExports = null;

  function forLanguageClients(fn) {
    for (const c of languageClients) {
      try { fn(c); } catch (e) { reportError(e, c.name); }
    }
  }

  function settleAll(promises) {
    return Promise.all(promises.map((p) => Promise.resolve(p).then((v) => v, (e) => { reportError(e); return null; })));
  }

  function completionContext(ed, pos, trigger) {
    const lr = ed.getLineRangeForRange(new Range(pos, pos));
    const before = ed.getTextInRange(new Range(lr.start, pos));
    const m = before.match(/[\p{L}\p{N}_$-]*$/u);
    return {
      text: m ? m[0] : "",
      line: before.replace(/^\s+/, ""),
      position: pos,
      reason: trigger ? CompletionReason.Character : CompletionReason.Invoke,
      triggerCharacter: trigger || null,
      identifierChars: Charset.alphanumeric.concat(new Charset("_-$")),
      selectors: [new ScopeSelector(ed.document.syntax || "")],
      matches: [],
    };
  }

  const handlers = {
    activate() {
      if (mainExports && typeof mainExports.activate === "function") {
        try { const r = mainExports.activate(); if (r && r.catch) r.catch((e) => reportError(e, "activate")); } catch (e) { reportError(e, "activate"); }
      }
    },
    deactivate() {
      try { if (mainExports && typeof mainExports.deactivate === "function") mainExports.deactivate(); } catch (e) { reportError(e, "deactivate"); }
      try { nova.subscriptions.dispose(); } catch (e) {}
      forLanguageClients((c) => c.stop());
      for (const p of processes.values()) p.kill();
    },
    workspace(ev) { workspacePath = ev.path; workspaceEm.emit("path", ev.path); },
    docOpened(ev) {
      const ed = editor(ev.id);
      workspaceEm.emit("document", ed.document);
      workspaceEm.emit("editor", ed);
      forLanguageClients((c) => c._didOpen(ed.document));
      for (const a of issueAssistants) scheduleIssues(a, ed, 50);
    },
    docMeta(ev) {
      const ed = editor(ev.id);
      ed.document._em.emit("syntax", ed.document, ed.document.syntax);
      ed.document._em.emit("path", ed.document, ed.document.path);
      forLanguageClients((c) => { c._didClose(ev.id); c._didOpen(ed.document); });
      for (const a of issueAssistants) scheduleIssues(a, ed, 50);
    },
    docChanged(ev) {
      const ed = editors.get(ev.id);
      if (!ed) return;
      inChangeCallback = true;
      try { ed._em.emit("change", ed); } finally { inChangeCallback = false; }
      if (stopTimers.has(ev.id)) clearTimeout(stopTimers.get(ev.id));
      stopTimers.set(ev.id, setTimeout(() => { stopTimers.delete(ev.id); ed._em.emit("stop", ed); }, 400));
      forLanguageClients((c) => c._didChange(ed.document));
      for (const a of issueAssistants) if ((a.options.event || "onChange") === "onChange") scheduleIssues(a, ed, 600);
    },
    selection(ev) { const ed = editors.get(ev.id); if (ed) ed._em.emit("selection", ed); },
    docSaved(ev) {
      const ed = editor(ev.id);
      ed._em.emit("save", ed);
      forLanguageClients((c) => c._didSave(ed.document));
      for (const a of issueAssistants) if (a.options.event === "onSave" || a.options.event === "onChange" || !a.options.event) scheduleIssues(a, ed, 10);
    },
    docClosed(ev) {
      const ed = editors.get(ev.id);
      forLanguageClients((c) => c._didClose(ev.id));
      if (ed) {
        ed._em.emit("destroy", ed);
        ed.document._closed = true;
        editors.delete(ev.id);
        documents.delete(ev.id);
      }
      const uri = ed && ed.document._last && ed.document._last.uri;
      if (uri) for (const a of issueAssistants) a.collection.remove(uri);
    },
    active() {},
    command(ev) {
      const h = commandHandlers.get(ev.name);
      if (!h) { console.warn("Kein Befehl registriert: " + ev.name); return; }
      const first = commandSections.get(ev.name) === "editor" && ev.editor !== null && ev.editor !== undefined ? editor(ev.editor) : workspace;
      try {
        const r = h.fn.apply(h.thisValue, [first].concat(ev.args || []));
        if (r && typeof r.catch === "function") r.catch((e) => reportError(e, ev.name));
      } catch (e) { reportError(e, ev.name); }
    },
    config(ev) {
      const cfg = ev.workspace ? workspace.config : nova.config;
      cfg._em.emit(ev.key, ev.value, ev.old);
    },
    reply(ev) {
      const fn = callbacks.get(ev.cb);
      callbacks.delete(ev.cb);
      if (fn) fn(ev.value);
    },
    timer(ev) {
      const t = timers.get(ev.id);
      if (!t) return;
      if (t.repeat) call("timer.set", { id: ev.id, ms: t.ms }); else timers.delete(ev.id);
      try { if (typeof t.fn === "function") t.fn.apply(undefined, t.args); else (0, eval)(String(t.fn)); } catch (e) { reportError(e, "timer"); }
    },
    proc(ev) {
      const p = processes.get(ev.id);
      if (p) p._em.emit(ev.stderr ? "stderr" : "stdout", ev.line);
    },
    procJson(ev) { const p = processes.get(ev.id); if (p) p._message(ev.text); },
    procExit(ev) { const p = processes.get(ev.id); if (p) p._exit(ev.code); },
    http(ev) {
      const p = httpPending.get(ev.id);
      if (!p) return;
      httpPending.delete(ev.id);
      if (ev.error) p.reject(new TypeError("Network request failed: " + ev.error));
      else p.resolve(new Response(b64ToBytes(ev.body || ""), { status: ev.status, statusText: ev.statusText, headers: ev.headers, url: ev.url }));
    },
    completion(ev) {
      const ed = editor(ev.id);
      const syntax = ed.document.syntax;
      const jobs = [];
      let ctx = null;
      for (const a of completionAssistants) {
        if (!selectorMatches(a.selector, syntax) || !a.object || typeof a.object.provideCompletionItems !== "function") continue;
        if (ev.trigger && !/[\p{L}\p{N}_$-]/u.test(ev.trigger) && !(a.options.triggerChars && a.options.triggerChars.contains(ev.trigger))) continue;
        ctx = ctx || completionContext(ed, ev.pos, ev.trigger);
        try {
          jobs.push(Promise.resolve(a.object.provideCompletionItems(ed, ctx)).then((items) => (items || []).filter((i) => i instanceof CompletionItem).map(completionMsg)));
        } catch (e) { reportError(e, "provideCompletionItems"); }
      }
      forLanguageClients((c) => { if (c._handles(ed.document)) jobs.push(c._complete(ed.document, ev.pos, ev.trigger)); });
      settleAll(jobs).then((lists) => post({ type: "answer", req: ev.req, value: [].concat.apply([], lists.filter((l) => l)) }));
    },
    hover(ev) {
      const ed = editor(ev.id);
      const jobs = [];
      forLanguageClients((c) => { if (c._handles(ed.document)) jobs.push(c._hover(ed.document, ev.pos)); });
      settleAll(jobs).then((texts) => post({ type: "answer", req: ev.req, value: texts.filter((t) => t).join("\n\n") || null }));
    },
    definition(ev) {
      const ed = editor(ev.id);
      const jobs = [];
      forLanguageClients((c) => { if (c._handles(ed.document)) jobs.push(c._definition(ed.document, ev.pos)); });
      settleAll(jobs).then((lists) => post({ type: "answer", req: ev.req, value: [].concat.apply([], lists.filter((l) => l)) }));
    },
    willSave(ev) {
      const ed = editors.get(ev.id);
      const jobs = [];
      if (ed) {
        for (const e of ed._em._list("willSave").slice()) {
          try { jobs.push(e.cb(ed)); } catch (err) { reportError(err, "onWillSave"); }
        }
      }
      settleAll(jobs).then(() => post({ type: "answer", req: ev.req, value: null }));
    },
    treeChildren(ev) {
      const v = treeViews.get(ev.view);
      if (v) v._children(ev.req, ev.element); else post({ type: "treeChildren", view: ev.view, req: ev.req, items: [] });
    },
    treeEvent(ev) { const v = treeViews.get(ev.view); if (v) v._event(ev.kind, ev.element); },
  };

  global.__nebula_dispatch = function (json) {
    const ev = JSON.parse(json);
    const h = handlers[ev.type];
    if (h) {
      try { h(ev); } catch (e) { reportError(e, ev.type); }
    }
  };

  global.__nebula_start = function () {
    mainExports = loadModule(INFO.main);
  };

  void extEm;
})(globalThis);
