//! Emmet abbreviation expansion for HTML (`ul.nav>li.item$*3>a{Link}`).

#[derive(Debug, Default, Clone)]
struct Node {
    tag: Option<String>,
    id: Option<String>,
    classes: Vec<String>,
    attrs: Vec<(String, Option<String>)>,
    text: Option<String>,
    count: usize,
    children: Vec<Node>,
    /// A parenthesized group: only `children` matter.
    group: bool,
}

const VOID: &[&str] = &[
    "area", "base", "br", "col", "embed", "hr", "img", "input", "link", "meta", "param", "source", "track", "wbr",
];
const INLINE: &[&str] = &[
    "a", "abbr", "b", "bdi", "bdo", "br", "button", "cite", "code", "data", "dfn", "em", "i", "img", "input", "kbd",
    "label", "mark", "q", "s", "samp", "select", "small", "span", "strong", "sub", "sup", "textarea", "time", "u",
    "var", "option", "td", "th", "li", "title", "h1", "h2", "h3", "h4", "h5", "h6", "p", "dt", "dd", "figcaption",
];
pub const KNOWN_TAGS: &[&str] = &[
    "a", "abbr", "address", "article", "aside", "audio", "b", "blockquote", "body", "br", "button", "canvas",
    "caption", "code", "dd", "details", "dialog", "div", "dl", "dt", "em", "fieldset", "figcaption", "figure",
    "footer", "form", "h1", "h2", "h3", "h4", "h5", "h6", "head", "header", "hr", "html", "i", "iframe", "img",
    "input", "label", "legend", "li", "link", "main", "meta", "nav", "ol", "option", "p", "picture", "pre",
    "section", "select", "small", "source", "span", "strong", "style", "sub", "summary", "sup", "svg", "table",
    "tbody", "td", "template", "textarea", "tfoot", "th", "thead", "title", "tr", "u", "ul", "video",
];

struct Parser<'a> {
    s: &'a [u8],
    i: usize,
}

impl Parser<'_> {
    fn peek(&self) -> Option<u8> {
        self.s.get(self.i).copied()
    }

    fn take_until(&mut self, end: u8) -> Option<String> {
        let start = self.i;
        let mut depth = 0;
        while let Some(c) = self.peek() {
            if c == end && depth == 0 {
                let out = String::from_utf8_lossy(&self.s[start..self.i]).into_owned();
                self.i += 1;
                return Some(out);
            }
            if end == b'}' && c == b'{' {
                depth += 1;
            } else if end == b'}' && c == b'}' {
                depth -= 1;
            }
            self.i += 1;
        }
        None
    }

    fn name(&mut self) -> String {
        let start = self.i;
        while self.peek().is_some_and(|c| c.is_ascii_alphanumeric() || matches!(c, b'-' | b':' | b'_' | b'$' | b'@')) {
            self.i += 1;
        }
        String::from_utf8_lossy(&self.s[start..self.i]).into_owned()
    }

    fn term(&mut self) -> Option<Node> {
        let mut node = if self.peek() == Some(b'(') {
            self.i += 1;
            let (children, _) = self.seq()?;
            if self.peek() != Some(b')') {
                return None;
            }
            self.i += 1;
            Node { children, group: true, ..Default::default() }
        } else {
            let mut n = Node::default();
            let tag = self.name();
            if !tag.is_empty() {
                n.tag = Some(tag);
            }
            loop {
                match self.peek() {
                    Some(b'#') => {
                        self.i += 1;
                        n.id = Some(self.name());
                    }
                    Some(b'.') => {
                        self.i += 1;
                        let c = self.name();
                        if c.is_empty() {
                            return None;
                        }
                        n.classes.push(c);
                    }
                    Some(b'[') => {
                        self.i += 1;
                        let inner = self.take_until(b']')?;
                        n.attrs.extend(parse_attrs(&inner));
                    }
                    Some(b'{') => {
                        self.i += 1;
                        n.text = Some(self.take_until(b'}')?);
                    }
                    _ => break,
                }
            }
            if n.tag.is_none() && n.id.is_none() && n.classes.is_empty() && n.attrs.is_empty() && n.text.is_none() {
                return None;
            }
            n
        };
        node.count = 1;
        if self.peek() == Some(b'*') {
            self.i += 1;
            let start = self.i;
            while self.peek().is_some_and(|c| c.is_ascii_digit()) {
                self.i += 1;
            }
            node.count = std::str::from_utf8(&self.s[start..self.i]).ok()?.parse().ok()?;
            if node.count == 0 || node.count > 500 {
                return None;
            }
        }
        Some(node)
    }

    /// Parses siblings; returns how many levels a trailing `^` climbs.
    fn seq(&mut self) -> Option<(Vec<Node>, usize)> {
        let mut nodes = Vec::new();
        loop {
            let mut node = self.term()?;
            match self.peek() {
                Some(b'>') => {
                    self.i += 1;
                    let (children, climb) = self.seq()?;
                    node.children.extend(children);
                    nodes.push(node);
                    match climb {
                        0 => return Some((nodes, 0)),
                        1 => continue,
                        c => return Some((nodes, c - 1)),
                    }
                }
                Some(b'+') => {
                    self.i += 1;
                    nodes.push(node);
                }
                Some(b'^') => {
                    let mut climb = 0;
                    while self.peek() == Some(b'^') {
                        self.i += 1;
                        climb += 1;
                    }
                    nodes.push(node);
                    return Some((nodes, climb));
                }
                _ => {
                    nodes.push(node);
                    return Some((nodes, 0));
                }
            }
        }
    }
}

fn parse_attrs(s: &str) -> Vec<(String, Option<String>)> {
    let mut out = Vec::new();
    let mut rest = s.trim();
    while !rest.is_empty() {
        let end = rest.find(|c: char| c == '=' || c.is_whitespace()).unwrap_or(rest.len());
        let name = rest[..end].to_string();
        rest = rest[end..].trim_start();
        let value = if let Some(r) = rest.strip_prefix('=') {
            let r = r.trim_start();
            let (v, after) = if let Some(q) = r.chars().next().filter(|c| *c == '"' || *c == '\'') {
                let close = r[1..].find(q).map_or(r.len(), |i| i + 1);
                (r[1..close].to_string(), r.get(close + 1..).unwrap_or(""))
            } else {
                let e = r.find(char::is_whitespace).unwrap_or(r.len());
                (r[..e].to_string(), &r[e..])
            };
            rest = after.trim_start();
            Some(v)
        } else {
            None
        };
        if !name.is_empty() {
            out.push((name, value));
        }
    }
    out
}

/// Replaces `$`, `$$`, … with the 1-based index, zero padded.
fn numbered(s: &str, index: usize) -> String {
    let mut out = String::new();
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '$' {
            let mut width = 1;
            while chars.peek() == Some(&'$') {
                chars.next();
                width += 1;
            }
            out.push_str(&format!("{index:0width$}"));
        } else {
            out.push(c);
        }
    }
    out
}

struct Renderer<'a> {
    unit: &'a str,
    cursor_placed: bool,
}

impl Renderer<'_> {
    /// `$0` at the first empty spot.
    fn slot(&mut self) -> &'static str {
        if self.cursor_placed {
            ""
        } else {
            self.cursor_placed = true;
            "$0"
        }
    }

    /// `index` is the number of the nearest repeated ancestor, used for `$`
    /// in elements that are not repeated themselves.
    fn render(&mut self, nodes: &[Node], parent: Option<&str>, depth: usize, index: usize, out: &mut Vec<String>) {
        for node in nodes {
            for n in 1..=node.count {
                let i = if node.count > 1 { n } else { index };
                if node.group {
                    self.render(&node.children, parent, depth, i, out);
                    continue;
                }
                let tag = node.tag.clone().unwrap_or_else(|| match parent {
                    Some("ul" | "ol") => "li".into(),
                    Some("table" | "tbody" | "thead" | "tfoot") => "tr".into(),
                    Some("tr") => "td".into(),
                    Some("select" | "datalist") => "option".into(),
                    Some("span" | "a" | "p" | "em" | "strong" | "b" | "i" | "label") => "span".into(),
                    _ => "div".into(),
                });
                let tag = numbered(&tag, i);
                let mut attrs = String::new();
                if let Some(id) = &node.id {
                    attrs.push_str(&format!(" id=\"{}\"", numbered(id, i)));
                }
                if !node.classes.is_empty() {
                    let c: Vec<String> = node.classes.iter().map(|c| numbered(c, i)).collect();
                    attrs.push_str(&format!(" class=\"{}\"", c.join(" ")));
                }
                let mut explicit: Vec<(String, Option<String>)> = node.attrs.clone();
                // Default attributes like Emmet.
                let defaults: &[&str] = match tag.as_str() {
                    "a" => &["href"],
                    "img" => &["src", "alt"],
                    "link" => &["rel=stylesheet", "href"],
                    "script" => &["src"],
                    "input" => &["type=text"],
                    "form" => &["action"],
                    "label" => &["for"],
                    "iframe" => &["src"],
                    _ => &[],
                };
                for d in defaults {
                    let (k, v) = d.split_once('=').map_or((*d, None), |(k, v)| (k, Some(v.to_string())));
                    if !explicit.iter().any(|(n, _)| n == k) {
                        explicit.push((k.to_string(), v));
                    }
                }
                for (k, v) in explicit {
                    let v = match v {
                        Some(v) if !v.is_empty() => numbered(&v, i),
                        _ => self.slot().to_string(),
                    };
                    attrs.push_str(&format!(" {k}=\"{v}\""));
                }
                let indent = self.unit.repeat(depth);
                if VOID.contains(&tag.as_str()) {
                    out.push(format!("{indent}<{tag}{attrs}>"));
                    continue;
                }
                let text = node.text.as_ref().map(|t| numbered(t, i));
                if node.children.is_empty() {
                    let content = match text {
                        Some(t) => t,
                        None => self.slot().to_string(),
                    };
                    out.push(format!("{indent}<{tag}{attrs}>{content}</{tag}>"));
                    continue;
                }
                let inline_children = INLINE.contains(&tag.as_str())
                    && node.children.iter().all(|c| c.children.is_empty() && c.count == 1 && !c.group);
                if inline_children {
                    let mut inner = Vec::new();
                    self.render(&node.children, Some(&tag), 0, i, &mut inner);
                    out.push(format!("{indent}<{tag}{attrs}>{}{}</{tag}>", text.unwrap_or_default(), inner.concat()));
                    continue;
                }
                out.push(format!("{indent}<{tag}{attrs}>{}", text.unwrap_or_default()));
                self.render(&node.children, Some(&tag), depth + 1, i, out);
                out.push(format!("{indent}</{tag}>"));
            }
        }
    }
}

/// Expands an abbreviation, or `None` if it isn't one. The result uses `$0`
/// for the cursor and `unit` for indentation.
pub fn expand(abbr: &str, unit: &str) -> Option<String> {
    let abbr = abbr.trim();
    if abbr.is_empty() || abbr.len() > 2000 {
        return None;
    }
    // A bare word only counts if it is a real tag (so prose stays prose).
    let has_syntax = abbr.contains(['>', '+', '^', '*', '.', '#', '[', '{', '(']);
    if !has_syntax && !KNOWN_TAGS.contains(&abbr.to_ascii_lowercase().as_str()) {
        return None;
    }
    let mut p = Parser { s: abbr.as_bytes(), i: 0 };
    let (nodes, _) = p.seq()?;
    if p.i != abbr.len() {
        return None;
    }
    let mut r = Renderer { unit, cursor_placed: false };
    let mut lines = Vec::new();
    r.render(&nodes, None, 0, 1, &mut lines);
    Some(lines.join("\n"))
}

/// Finds the abbreviation that ends at the cursor: scans back to whitespace
/// or a tag boundary, keeping `{text}` and `[attrs]` (which may contain spaces).
pub fn abbreviation_before(line_before: &str) -> Option<&str> {
    // Only look at text after the last complete tag (`<div>ul>li`).
    let line_before = match line_before.rfind('<') {
        Some(lt) => match line_before[lt..].find('>') {
            Some(gt) => &line_before[lt + gt + 1..],
            None => return None, // inside a tag
        },
        None => line_before,
    };
    let bytes = line_before.as_bytes();
    let mut depth = 0i32;
    let mut i = bytes.len();
    while i > 0 {
        let c = bytes[i - 1];
        match c {
            b'}' | b']' | b')' => depth += 1,
            b'{' | b'[' | b'(' => {
                depth -= 1;
                if depth < 0 {
                    break;
                }
            }
            b' ' | b'\t' | b'<' | b';' | b',' if depth == 0 => break,
            b'>' if depth == 0 && (i < 2 || !bytes[i - 2].is_ascii_alphanumeric() && !b"]})".contains(&bytes[i - 2])) => break,
            _ => {}
        }
        i -= 1;
    }
    let abbr = &line_before[i..];
    (!abbr.is_empty()).then_some(abbr)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ex(a: &str) -> String {
        expand(a, "  ").unwrap()
    }

    #[test]
    fn basics() {
        assert_eq!(ex("div"), "<div>$0</div>");
        assert_eq!(ex(".box"), "<div class=\"box\">$0</div>");
        assert_eq!(ex("a"), "<a href=\"$0\"></a>");
        assert_eq!(ex("img"), "<img src=\"$0\" alt=\"\">");
        assert_eq!(ex("p{Hallo}"), "<p>Hallo</p>");
        assert!(expand("hello", "  ").is_none());
    }

    #[test]
    fn nesting_multiplication_and_numbering() {
        assert_eq!(
            ex("ul.nav>li.item$*3>a"),
            "<ul class=\"nav\">\n  <li class=\"item1\"><a href=\"$0\"></a></li>\n  <li class=\"item2\"><a href=\"\"></a></li>\n  <li class=\"item3\"><a href=\"\"></a></li>\n</ul>"
        );
        assert_eq!(ex("ul>li*2>a{Link $}"), "<ul>
  <li><a href=\"$0\">Link 1</a></li>
  <li><a href=\"\">Link 2</a></li>
</ul>");
        assert_eq!(ex("div#main>h1+p"), "<div id=\"main\">\n  <h1>$0</h1>\n  <p></p>\n</div>");
    }

    #[test]
    fn climb_groups_and_attributes() {
        assert_eq!(ex("div>p^span"), "<div>\n  <p>$0</p>\n</div>\n<span></span>");
        assert_eq!(ex("(header>nav)+main"), "<header>\n  <nav>$0</nav>\n</header>\n<main></main>");
        assert_eq!(ex("input[type=email name=mail]"), "<input type=\"email\" name=\"mail\">");
        assert_eq!(ex("a[href=\"/x y\"]{Mehr}"), "<a href=\"/x y\">Mehr</a>");
    }

    #[test]
    fn finds_abbreviation_before_cursor() {
        assert_eq!(abbreviation_before("    ul>li*2"), Some("ul>li*2"));
        assert_eq!(abbreviation_before("<p>text p{a b}"), Some("p{a b}"));
        assert_eq!(abbreviation_before("<div>ul>li"), Some("ul>li"));
    }
}
