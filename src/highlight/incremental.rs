//! Incremental highlighting for large documents. The syntax tree is kept
//! between changes and re-parsed incrementally; only the lines whose syntax
//! changed are colored again. Injections (CSS/JS in HTML, code fences, …)
//! are supported, locals (references to parameters) are not – the full
//! highlighter in `mod.rs` handles those for normal-sized files.

use std::collections::HashMap;
use std::ops::Range;
use std::rc::Rc;
use std::sync::Arc;

use tree_sitter::{InputEdit, Node, Parser, Point, Query, QueryCursor, StreamingIterator, Tree};

use super::{LineSpans, Span, empty_line, grammar_for_injection, grammar_source};
use crate::theme::{HIGHLIGHT_NAMES, PLAIN};

/// Deepest nesting of injections (e.g. JavaScript in HTML in PHP).
const MAX_DEPTH: usize = 3;

struct Grammar {
    name: &'static str,
    language: tree_sitter::Language,
    highlights: Query,
    /// Style of each capture of `highlights` (`None`: no known name).
    styles: Vec<Option<u8>>,
    injections: Option<Query>,
    /// Injection patterns marked `injection.combined` (one parse for all their nodes).
    combined: Vec<bool>,
}

/// Style for a capture name, matched like tree-sitter-highlight does:
/// the known name with the most dot-separated parts that all occur in it.
fn style_of(capture: &str) -> Option<u8> {
    let parts: Vec<&str> = capture.split('.').collect();
    let mut best = None;
    let mut best_len = 0;
    for (i, name) in HIGHLIGHT_NAMES.iter().enumerate() {
        let mut len = 0;
        let matches = name.split('.').all(|p| {
            len += 1;
            parts.contains(&p)
        });
        if matches && len > best_len {
            best = Some(i as u8);
            best_len = len;
        }
    }
    best
}

fn load(name: &'static str) -> Option<Grammar> {
    let (language, highlights, injections, _locals) = grammar_source(name)?;
    let highlights = match Query::new(&language, &highlights) {
        Ok(q) => q,
        Err(e) => {
            eprintln!("Highlighting für {name} nicht verfügbar: {e}");
            return None;
        }
    };
    let styles = highlights.capture_names().iter().map(|n| style_of(n)).collect();
    let injections = (!injections.is_empty()).then(|| Query::new(&language, &injections).ok()).flatten();
    let combined = injections
        .as_ref()
        .map(|q| (0..q.pattern_count()).map(|p| q.property_settings(p).iter().any(|s| &*s.key == "injection.combined")).collect())
        .unwrap_or_default();
    Some(Grammar { name, language, highlights, styles, injections, combined })
}

/// One match of a combined injection (all of them are parsed together).
#[derive(Clone)]
struct Combined {
    pattern: usize,
    language: String,
    ranges: Vec<tree_sitter::Range>,
}

/// What the worker remembers about a document.
struct DocState {
    grammar: &'static str,
    text: String,
    tree: Tree,
    lines: Arc<LineSpans>,
    /// Combined injections of the top layer: finding them means searching
    /// the whole file, so they are kept and only searched again where the
    /// text changed.
    combined: Vec<Combined>,
}

pub struct Engine {
    grammars: HashMap<&'static str, Option<Rc<Grammar>>>,
    parser: Parser,
    docs: HashMap<u64, DocState>,
}

fn newlines(s: &[u8]) -> usize {
    s.iter().filter(|&&b| b == b'\n').count()
}

fn point_at(text: &str, byte: usize) -> Point {
    let before = &text.as_bytes()[..byte];
    let row = newlines(before);
    let column = before.iter().rposition(|&b| b == b'\n').map_or(byte, |i| byte - i - 1);
    Point { row, column }
}

/// The changed part between two texts: (start, old end, new end) in bytes.
fn diff(old: &str, new: &str) -> (usize, usize, usize) {
    let (a, b) = (old.as_bytes(), new.as_bytes());
    let max = a.len().min(b.len());
    // Whole blocks first (compared with memcmp), then byte by byte.
    let mut start = a.chunks(256).zip(b.chunks(256)).take_while(|(x, y)| x == y).map(|(x, _)| x.len()).sum::<usize>().min(max);
    while start < max && a[start] == b[start] {
        start += 1;
    }
    while !old.is_char_boundary(start) {
        start -= 1;
    }
    let max_suffix = max - start;
    let mut suffix = a[a.len() - max_suffix..]
        .rchunks(256)
        .zip(b[b.len() - max_suffix..].rchunks(256))
        .take_while(|(x, y)| x == y)
        .map(|(x, _)| x.len())
        .sum::<usize>()
        .min(max_suffix);
    while suffix < max_suffix && a[a.len() - suffix - 1] == b[b.len() - suffix - 1] {
        suffix += 1;
    }
    while !old.is_char_boundary(a.len() - suffix) || !new.is_char_boundary(b.len() - suffix) {
        suffix -= 1;
    }
    (start, a.len() - suffix, b.len() - suffix)
}

/// Moves a range behind an edit along with the text (see `Tree::edit`).
fn shift(r: &mut tree_sitter::Range, edit: &InputEdit) {
    let point = |p: Point| {
        if p.row == edit.old_end_position.row {
            Point { row: edit.new_end_position.row, column: p.column - edit.old_end_position.column + edit.new_end_position.column }
        } else {
            Point { row: p.row + edit.new_end_position.row - edit.old_end_position.row, column: p.column }
        }
    };
    r.start_byte = r.start_byte + edit.new_end_byte - edit.old_end_byte;
    r.end_byte = r.end_byte + edit.new_end_byte - edit.old_end_byte;
    r.start_point = point(r.start_point);
    r.end_point = point(r.end_point);
}

impl Engine {
    pub fn new() -> Self {
        Self { grammars: HashMap::new(), parser: Parser::new(), docs: HashMap::new() }
    }

    fn grammar(&mut self, name: &'static str) -> Option<Rc<Grammar>> {
        self.grammars.entry(name).or_insert_with(|| load(name).map(Rc::new)).clone()
    }

    pub fn forget(&mut self, doc_id: u64) {
        self.docs.remove(&doc_id);
    }

    /// Highlights a new version of a document, reusing what is known
    /// about its previous version.
    pub fn highlight(&mut self, doc_id: u64, grammar: &'static str, text: String) -> Arc<LineSpans> {
        let Some(g) = self.grammar(grammar) else {
            self.docs.remove(&doc_id);
            return Arc::new(vec![empty_line(); newlines(text.as_bytes()) + 1]);
        };
        if let Some(state) = self.docs.remove(&doc_id)
            && state.grammar == grammar
            && let Some((tree, lines, combined)) = self.update(&g, state, &text)
        {
            self.docs.insert(doc_id, DocState { grammar, text, tree, lines: lines.clone(), combined });
            return lines;
        }
        // First version (or the update failed): everything.
        self.parser.set_language(&g.language).ok();
        let _ = self.parser.set_included_ranges(&[]);
        let Some(tree) = self.parser.parse(&text, None) else {
            return Arc::new(vec![empty_line(); newlines(text.as_bytes()) + 1]);
        };
        let mut combined = Vec::new();
        let lines = Arc::new(self.paint(&g, &tree, &text, 0..text.len(), &mut combined));
        self.docs.insert(doc_id, DocState { grammar, text, tree, lines: lines.clone(), combined });
        lines
    }

    /// Colors a document completely with the tree kept for it (tests).
    #[cfg(test)]
    fn repaint(&mut self, doc_id: u64) -> LineSpans {
        let state = self.docs.remove(&doc_id).unwrap();
        let g = self.grammar(state.grammar).unwrap();
        let lines = self.paint(&g, &state.tree, &state.text, 0..state.text.len(), &mut Vec::new());
        self.docs.insert(doc_id, state);
        lines
    }

    /// The new tree, lines and combined injections after a change, or
    /// `None` to start over.
    fn update(&mut self, g: &Rc<Grammar>, mut state: DocState, text: &str) -> Option<(Tree, Arc<LineSpans>, Vec<Combined>)> {
        let (start, old_end, new_end) = diff(&state.text, text);
        if start == old_end && start == new_end {
            return Some((state.tree, state.lines, state.combined));
        }
        let start_position = point_at(text, start);
        let advance = |from: Point, s: &str| {
            let rows = newlines(s.as_bytes());
            match s.as_bytes().iter().rposition(|&b| b == b'\n') {
                Some(i) => Point { row: from.row + rows, column: s.len() - i - 1 },
                None => Point { row: from.row, column: from.column + s.len() },
            }
        };
        let edit = InputEdit {
            start_byte: start,
            old_end_byte: old_end,
            new_end_byte: new_end,
            start_position,
            old_end_position: advance(start_position, &state.text[start..old_end]),
            new_end_position: advance(start_position, &text[start..new_end]),
        };
        state.tree.edit(&edit);
        self.parser.set_language(&g.language).ok();
        let _ = self.parser.set_included_ranges(&[]);
        let tree = self.parser.parse(text, Some(&state.tree))?;

        // Lines to color again: the edit plus where the syntax changed.
        let (mut a, mut b) = (start, new_end);
        for r in state.tree.changed_ranges(&tree) {
            a = a.min(r.start_byte);
            b = b.max(r.end_byte);
        }
        let b = b.min(text.len());
        let region_start = text.as_bytes()[..a].iter().rposition(|&c| c == b'\n').map_or(0, |i| i + 1);
        let region_end = text.as_bytes()[b..].iter().position(|&c| c == b'\n').map_or(text.len(), |i| b + i);
        let first = newlines(&text.as_bytes()[..region_start]);

        // Known combined injections outside the region stay (moved along
        // with the text); the region is searched again.
        let mut combined: Vec<Combined> = std::mem::take(&mut state.combined)
            .into_iter()
            .filter_map(|mut c| {
                let (s, e) = (c.ranges.first()?.start_byte, c.ranges.last()?.end_byte);
                if e <= start {
                } else if s >= old_end {
                    c.ranges.iter_mut().for_each(|r| shift(r, &edit));
                } else {
                    return None;
                }
                let (s, e) = (c.ranges[0].start_byte, c.ranges[c.ranges.len() - 1].end_byte);
                (e < region_start || s > region_end).then_some(c)
            })
            .collect();
        let fresh = self.paint(g, &tree, text, region_start..region_end, &mut combined);

        let removed_newlines = newlines(&state.text.as_bytes()[start..old_end]);
        let added_newlines = newlines(&text.as_bytes()[start..new_end]);
        // The old line that follows the region.
        let after = (first + fresh.len() + removed_newlines).checked_sub(added_newlines)?;
        let old = &state.lines;
        if after > old.len() || first > old.len() {
            return None;
        }
        let mut lines = Vec::with_capacity(old.len() + fresh.len());
        lines.extend_from_slice(&old[..first]);
        lines.extend(fresh);
        lines.extend_from_slice(&old[after..]);
        if lines.len() != newlines(text.as_bytes()) + 1 {
            return None;
        }
        Some((tree, Arc::new(lines), combined))
    }

    /// Colors the lines in `region` (which starts and ends at line
    /// boundaries). `combined` holds the top layer's combined injections
    /// outside the region; those inside are added.
    fn paint(&mut self, g: &Rc<Grammar>, tree: &Tree, text: &str, region: Range<usize>, combined: &mut Vec<Combined>) -> LineSpans {
        let mut styles = vec![PLAIN; region.len()];
        self.paint_layer(g, tree, text, &region, None, &mut styles, 0, Some(combined));
        to_lines(&text.as_bytes()[region], &styles)
    }

    #[allow(clippy::too_many_arguments)]
    fn paint_layer(
        &mut self,
        g: &Rc<Grammar>,
        tree: &Tree,
        text: &str,
        region: &Range<usize>,
        layer: Option<&[tree_sitter::Range]>,
        styles: &mut [u8],
        depth: usize,
        known: Option<&mut Vec<Combined>>,
    ) {
        let src = text.as_bytes();
        // (start, end, node id, pattern, style)
        let mut caps: Vec<(usize, usize, usize, usize, Option<u8>)> = Vec::new();
        let mut cursor = QueryCursor::new();
        cursor.set_byte_range(region.clone());
        let mut it = cursor.captures(&g.highlights, tree.root_node(), src);
        while let Some((m, i)) = it.next() {
            let c = m.captures()[*i];
            caps.push((c.node.start_byte(), c.node.end_byte(), c.node.id(), m.pattern_index, g.styles[c.index as usize]));
        }
        // Outer nodes first so inner ones paint over them; for one node
        // only its first pattern counts.
        caps.sort_unstable_by(|x, y| (y.1 - y.0).cmp(&(x.1 - x.0)).then(x.2.cmp(&y.2)).then(x.3.cmp(&y.3)));
        let mut last_node = usize::MAX;
        for (s, e, node, _, style) in caps {
            if node == last_node {
                continue;
            }
            last_node = node;
            if let Some(style) = style {
                fill(styles, region, layer, s..e, style);
            }
        }

        if depth >= MAX_DEPTH {
            return;
        }
        let Some(q) = &g.injections else { return };
        // (language, ranges)
        let mut injections: Vec<(String, Vec<tree_sitter::Range>)> = Vec::new();
        let mut cursor = QueryCursor::new();
        cursor.set_byte_range(region.clone());
        let mut it = cursor.matches(q, tree.root_node(), src);
        while let Some(m) = it.next() {
            if !g.combined[m.pattern_index]
                && let Some((language, node, children)) = injection_of(g, q, src, m)
            {
                injections.push((language, injection_ranges(layer, &[node], children)));
            }
        }
        if g.combined.iter().any(|&c| c) {
            // Combined injections form one document across the whole file.
            // The top layer searches only the region and keeps the rest.
            let mut found = Vec::new();
            let mut cursor = QueryCursor::new();
            if known.is_some() {
                cursor.set_byte_range(region.clone());
            }
            let mut it = cursor.matches(q, tree.root_node(), src);
            while let Some(m) = it.next() {
                if g.combined[m.pattern_index]
                    && let Some((language, node, children)) = injection_of(g, q, src, m)
                {
                    let ranges = injection_ranges(layer, &[node], children);
                    if !ranges.is_empty() {
                        found.push(Combined { pattern: m.pattern_index, language, ranges });
                    }
                }
            }
            let all: &[Combined] = match known {
                Some(known) => {
                    known.extend(found);
                    known
                }
                None => &found,
            };
            let mut groups: HashMap<(usize, &str), Vec<tree_sitter::Range>> = HashMap::new();
            for c in all {
                groups.entry((c.pattern, &c.language)).or_default().extend(c.ranges.iter().copied());
            }
            for ((_, language), mut ranges) in groups {
                ranges.sort_by_key(|r| r.start_byte);
                ranges.dedup_by_key(|r| r.start_byte);
                injections.push((language.to_string(), ranges));
            }
        }

        for (language, ranges) in injections {
            if !ranges.iter().any(|r| r.start_byte < region.end && r.end_byte > region.start) {
                continue;
            }
            let Some(name) = grammar_for_injection(&language) else { continue };
            let Some(ig) = self.grammar(name) else { continue };
            if self.parser.set_language(&ig.language).is_err() || self.parser.set_included_ranges(&ranges).is_err() {
                continue;
            }
            let parsed = self.parser.parse(text, None);
            let _ = self.parser.set_included_ranges(&[]);
            if let Some(t) = parsed {
                self.paint_layer(&ig, &t, text, region, Some(&ranges), styles, depth + 1, None);
            }
        }
    }
}

/// Language, content node and whether its children belong to it, for an
/// injection query match.
fn injection_of<'t>(g: &Grammar, q: &Query, src: &[u8], m: &tree_sitter::QueryMatch<'_, 't>) -> Option<(String, Node<'t>, bool)> {
    let mut language = None;
    let mut content = None;
    for c in m.captures() {
        match q.capture_names()[c.index as usize] {
            "injection.language" => language = c.node.utf8_text(src).ok().map(str::to_string),
            "injection.content" => content = Some(c.node),
            _ => {}
        }
    }
    let mut include_children = false;
    for p in q.property_settings(m.pattern_index) {
        match &*p.key {
            "injection.language" if language.is_none() => language = p.value.as_deref().map(str::to_string),
            "injection.self" if language.is_none() => language = Some(g.name.to_string()),
            "injection.include-children" => include_children = true,
            _ => {}
        }
    }
    Some((language?, content?, include_children))
}

/// Sets `style` for the bytes of `span` inside the region (and the layer's ranges).
fn fill(styles: &mut [u8], region: &Range<usize>, layer: Option<&[tree_sitter::Range]>, span: Range<usize>, style: u8) {
    let mut set = |a: usize, b: usize| {
        let (a, b) = (a.max(region.start), b.min(region.end));
        if a < b {
            styles[a - region.start..b - region.start].fill(style);
        }
    };
    match layer {
        None => set(span.start, span.end),
        Some(ranges) => {
            let i = ranges.partition_point(|r| r.end_byte <= span.start);
            for r in &ranges[i..] {
                if r.start_byte >= span.end {
                    break;
                }
                set(span.start.max(r.start_byte), span.end.min(r.end_byte));
            }
        }
    }
}

/// The ranges of an injection: its nodes (without their children unless
/// asked for), limited to the ranges of the layer they were found in.
fn injection_ranges(layer: Option<&[tree_sitter::Range]>, nodes: &[Node], include_children: bool) -> Vec<tree_sitter::Range> {
    let mut own = Vec::new();
    for node in nodes {
        let mut from = (node.start_byte(), node.start_position());
        let mut cursor = node.walk();
        if !include_children {
            // Named children only: some grammars (Markdown) expose punctuation inside
            // the content as anonymous nodes, which belongs to the injected text.
            for child in node.named_children(&mut cursor) {
                if child.start_byte() > from.0 {
                    own.push(tree_sitter::Range { start_byte: from.0, start_point: from.1, end_byte: child.start_byte(), end_point: child.start_position() });
                }
                from = (child.end_byte(), child.end_position());
            }
        }
        if node.end_byte() > from.0 {
            own.push(tree_sitter::Range { start_byte: from.0, start_point: from.1, end_byte: node.end_byte(), end_point: node.end_position() });
        }
    }
    let Some(layer) = layer else { return own };
    let mut out = Vec::new();
    for r in own {
        let i = layer.partition_point(|l| l.end_byte <= r.start_byte);
        for l in &layer[i..] {
            if l.start_byte >= r.end_byte {
                break;
            }
            let (start_byte, start_point) = if l.start_byte > r.start_byte { (l.start_byte, l.start_point) } else { (r.start_byte, r.start_point) };
            let (end_byte, end_point) = if l.end_byte < r.end_byte { (l.end_byte, l.end_point) } else { (r.end_byte, r.end_point) };
            if start_byte < end_byte {
                out.push(tree_sitter::Range { start_byte, start_point, end_byte, end_point });
            }
        }
    }
    out
}

/// Splits per-byte styles into per-line spans.
fn to_lines(text: &[u8], styles: &[u8]) -> LineSpans {
    let mut lines = Vec::new();
    let mut line: Vec<Span> = Vec::new();
    let mut line_start = 0;
    let mut i = 0;
    while i < text.len() {
        if text[i] == b'\n' {
            lines.push(if line.is_empty() { empty_line() } else { Arc::from(std::mem::take(&mut line)) });
            i += 1;
            line_start = i;
            continue;
        }
        let style = styles[i];
        let mut j = i + 1;
        while j < text.len() && styles[j] == style && text[j] != b'\n' {
            j += 1;
        }
        if style != PLAIN {
            line.push(Span { start: (i - line_start) as u32, end: (j - line_start) as u32, style });
        }
        i = j;
    }
    lines.push(if line.is_empty() { empty_line() } else { Arc::from(line) });
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flat(lines: &LineSpans) -> Vec<(usize, u32, u32, &'static str)> {
        lines.iter().enumerate().flat_map(|(i, l)| l.iter().map(move |s| (i, s.start, s.end, HIGHLIGHT_NAMES[s.style as usize]))).collect()
    }

    /// After every edit, the spliced lines equal coloring the whole document
    /// (with the same tree: incremental parsing may recover from errors
    /// differently than a fresh parse).
    #[test]
    fn incremental_equals_fresh() {
        let samples: &[(&str, &str)] = &[
            ("javascript", "function f(a, b) {\n  // note\n  const s = `x ${a}`;\n  return a + b * 2; /* end */\n}\n"),
            ("html", "<div class=\"a\">\n<style>\na { color: red; }\n</style>\n<script>\nlet x = 1;\n</script>\n</div>\n"),
            ("php", "<p>Hi</p>\n<?php\n$x = 1; // c\necho \"$x\";\n?>\n<script>let y = 2;</script>\n"),
            ("markdown", "# Title\n\nSome *text* and `code`.\n\n```js\nconst a = 1;\n```\n\n- item\n"),
            ("css", ".a > b:hover { color: #fff; margin: 0 2px; }\n@media (x) { .b { top: 0 } }\n"),
        ];
        let edits: &[(f32, &str, usize)] = &[
            (0.5, "/*", 0),
            (0.5, "", 2),
            (0.3, "\n\n", 0),
            (0.7, "\"", 0),
            (0.1, "x", 0),
            (0.9, "", 5),
            (0.2, "<b>", 0),
            (0.0, "```\n", 0),
            (1.0, "\n// tail", 0),
        ];
        for (grammar, sample) in samples {
            let mut text = sample.repeat(30);
            let mut inc = Engine::new();
            inc.highlight(1, grammar, text.clone());
            for (at, insert, remove) in edits {
                let mut pos = (text.len() as f32 * at) as usize;
                while !text.is_char_boundary(pos) {
                    pos -= 1;
                }
                let end = (pos + remove).min(text.len());
                text.replace_range(pos..end, insert);
                let got = inc.highlight(1, grammar, text.clone());
                let want = inc.repaint(1);
                assert_eq!(got.len(), want.len(), "{grammar}: line count after {insert:?}");
                let (got, want) = (flat(&got), flat(&want));
                if let Some(i) = (0..got.len().max(want.len())).find(|&i| got.get(i) != want.get(i)) {
                    panic!("{grammar}: after inserting {insert:?} at {pos}: {:?} != {:?}", got.get(i), want.get(i));
                }
            }
        }
    }

    #[test]
    fn injections_and_styles() {
        let mut e = Engine::new();
        let html = "<!DOCTYPE html><style>a { color: red; }</style>\n<script>let x = 1;</script>\n<p class=\"a\">Hi</p>";
        let lines = e.highlight(1, "html", html.to_string());
        let names: Vec<&str> = flat(&lines).into_iter().map(|s| s.3).collect();
        for expected in ["tag.name", "style.attribute.name", "keyword", "tag.attribute.name"] {
            assert!(names.contains(&expected), "{expected} missing in {names:?}");
        }
        let php = "<div><?php echo $x; ?></div>";
        let names: Vec<&str> = flat(&e.highlight(2, "php", php.to_string())).into_iter().map(|s| s.3).collect();
        assert!(names.contains(&"tag.name"), "HTML around PHP: {names:?}");
        let md = "# T\n\n```css\na { color: red; }\n```\n";
        let names: Vec<&str> = flat(&e.highlight(3, "markdown", md.to_string())).into_iter().map(|s| s.3).collect();
        assert!(names.contains(&"style.attribute.name"), "code fence: {names:?}");
    }

    #[test]
    fn diff_finds_the_change() {
        assert_eq!(diff("abcdef", "abXdef"), (2, 3, 3));
        assert_eq!(diff("abc", "abc"), (3, 3, 3));
        assert_eq!(diff("aaa", "aaaa"), (3, 3, 4));
        assert_eq!(diff("äöü", "äü"), (2, 4, 2));
        let long = "x".repeat(1000);
        assert_eq!(diff(&long, &format!("{long}y")), (1000, 1000, 1001));
    }
}

#[cfg(test)]
mod grammar_tests {
    #[test]
    fn every_grammar_loads() {
        for l in crate::lang::Language::ALL {
            if let Some(name) = super::super::grammar_of(l) {
                assert!(super::load(name).is_some(), "{name}");
            }
        }
        assert!(super::load("markdown_inline").is_some());
    }
}
