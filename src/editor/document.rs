//! Text document: rope buffer, selections, undo history and file metadata.

use std::ops::Range;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use ropey::Rope;

use crate::highlight::LineSpans;
use crate::lang::Language;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DocSource {
    Untitled(u32),
    Local(PathBuf),
    Remote { site_id: String, path: String },
}

impl DocSource {
    pub fn file_name(&self) -> String {
        match self {
            DocSource::Untitled(n) => format!("Ohne Titel {n}"),
            DocSource::Local(p) => p
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| p.display().to_string()),
            DocSource::Remote { path, .. } => {
                path.rsplit('/').next().unwrap_or(path).to_string()
            }
        }
    }

    pub fn path_string(&self) -> String {
        match self {
            DocSource::Untitled(_) => String::new(),
            DocSource::Local(p) => p.display().to_string(),
            DocSource::Remote { path, .. } => path.clone(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LineEnding {
    Lf,
    CrLf,
}

impl LineEnding {
    pub fn label(self) -> &'static str {
        match self {
            LineEnding::Lf => "LF",
            LineEnding::CrLf => "CRLF",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Encoding {
    Utf8,
    Utf8Bom,
    /// Windows-1252 / Latin-1, used when a file is not valid UTF-8.
    Latin1,
    Utf16Le,
    Utf16Be,
}

impl Encoding {
    pub const ALL: [Encoding; 5] = [Encoding::Utf8, Encoding::Utf8Bom, Encoding::Latin1, Encoding::Utf16Le, Encoding::Utf16Be];

    pub fn label(self) -> &'static str {
        match self {
            Encoding::Utf8 => "UTF-8",
            Encoding::Utf8Bom => "UTF-8 BOM",
            Encoding::Latin1 => "Latin-1",
            Encoding::Utf16Le => "UTF-16 LE",
            Encoding::Utf16Be => "UTF-16 BE",
        }
    }

    /// Longer name for menus.
    pub fn description(self) -> &'static str {
        match self {
            Encoding::Utf8 => "UTF-8",
            Encoding::Utf8Bom => "UTF-8 mit BOM",
            Encoding::Latin1 => "Westeuropäisch (Windows-1252 / Latin-1)",
            Encoding::Utf16Le => "UTF-16 Little Endian",
            Encoding::Utf16Be => "UTF-16 Big Endian",
        }
    }

    /// Detects the encoding of file contents (BOM, then UTF-8, else Windows-1252).
    pub fn detect(bytes: &[u8]) -> Encoding {
        if bytes.starts_with(b"\xEF\xBB\xBF") {
            Encoding::Utf8Bom
        } else if bytes.starts_with(b"\xFF\xFE") {
            Encoding::Utf16Le
        } else if bytes.starts_with(b"\xFE\xFF") {
            Encoding::Utf16Be
        } else if std::str::from_utf8(bytes).is_ok() {
            Encoding::Utf8
        } else {
            Encoding::Latin1
        }
    }

    /// Decodes bytes in this encoding (a matching BOM is skipped).
    pub fn decode(self, bytes: &[u8]) -> String {
        let utf16 = |rest: &[u8], le: bool| {
            let units: Vec<u16> = rest
                .chunks_exact(2)
                .map(|c| if le { u16::from_le_bytes([c[0], c[1]]) } else { u16::from_be_bytes([c[0], c[1]]) })
                .collect();
            String::from_utf16_lossy(&units)
        };
        match self {
            Encoding::Utf8 | Encoding::Utf8Bom => {
                String::from_utf8_lossy(bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(bytes)).into_owned()
            }
            Encoding::Latin1 => decode_latin1(bytes),
            Encoding::Utf16Le => utf16(bytes.strip_prefix(b"\xFF\xFE").unwrap_or(bytes), true),
            Encoding::Utf16Be => utf16(bytes.strip_prefix(b"\xFE\xFF").unwrap_or(bytes), false),
        }
    }

    pub fn encode(self, text: &str) -> Vec<u8> {
        match self {
            Encoding::Utf8 => text.as_bytes().to_vec(),
            Encoding::Utf8Bom => [b"\xEF\xBB\xBF".as_slice(), text.as_bytes()].concat(),
            Encoding::Latin1 => encode_latin1(text),
            Encoding::Utf16Le => [0xFF, 0xFE].into_iter().chain(text.encode_utf16().flat_map(u16::to_le_bytes)).collect(),
            Encoding::Utf16Be => [0xFE, 0xFF].into_iter().chain(text.encode_utf16().flat_map(u16::to_be_bytes)).collect(),
        }
    }

    /// Characters this encoding cannot represent (they would be lost on saving).
    pub fn can_encode(self, text: &str) -> bool {
        self != Encoding::Latin1 || !text.chars().any(|c| c != '?' && encode_latin1(c.encode_utf8(&mut [0; 4])) == b"?")
    }
}

/// Modification time and size of a local file, to notice changes made by other programs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DiskStamp {
    pub modified: Option<std::time::SystemTime>,
    pub len: u64,
}

impl DiskStamp {
    pub fn of(path: &std::path::Path) -> Option<Self> {
        let meta = std::fs::metadata(path).ok()?;
        Some(Self { modified: meta.modified().ok(), len: meta.len() })
    }
}

/// A selection in char indices. `anchor == head` is a plain cursor.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Selection {
    pub anchor: usize,
    pub head: usize,
    /// Preferred display column for vertical movement.
    pub goal: Option<usize>,
}

impl Selection {
    pub fn cursor(pos: usize) -> Self {
        Self {
            anchor: pos,
            head: pos,
            goal: None,
        }
    }

    pub fn new(anchor: usize, head: usize) -> Self {
        Self {
            anchor,
            head,
            goal: None,
        }
    }

    pub fn range(&self) -> Range<usize> {
        self.anchor.min(self.head)..self.anchor.max(self.head)
    }

    pub fn is_empty(&self) -> bool {
        self.anchor == self.head
    }
}

/// One replacement, expressed in the coordinates of the document at the
/// moment it is applied.
#[derive(Clone, Debug)]
struct AppliedChange {
    start: usize,
    removed: String,
    inserted: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EditKind {
    Typing,
    Deleting,
    Other,
}

#[derive(Clone, Debug)]
struct Transaction {
    id: u64,
    kind: EditKind,
    changes: Vec<AppliedChange>,
    before: Vec<Selection>,
    after: Vec<Selection>,
    time: Instant,
}

/// A requested replacement of `range` (char indices, current document) with `text`.
#[derive(Clone, Debug)]
pub struct Change {
    pub range: Range<usize>,
    pub text: String,
}

impl Change {
    pub fn new(range: Range<usize>, text: impl Into<String>) -> Self {
        Self {
            range,
            text: text.into(),
        }
    }
}

/// Per-document view state kept between frames.
#[derive(Clone, Debug, Default)]
pub struct ViewState {
    pub scroll_to_cursor: bool,
    /// Line to center on when the next frame is drawn.
    pub center_line: Option<usize>,
    /// Open completion popup.
    pub completion: Option<super::complete::Completion>,
    /// Start lines of folded blocks.
    pub folds: std::collections::BTreeSet<usize>,
    /// Bumped whenever `folds` changes.
    pub fold_gen: u64,
    pub fold_ranges: Option<(u64, Arc<Vec<(usize, usize)>>)>,
    /// Visual rows for (version, fold generation, wrap width, tab size).
    pub rows_cache: Option<((u64, u64, Option<usize>, usize), Arc<Vec<super::folding::Row>>)>,
    /// Problems reported by extensions, sorted by start.
    pub diagnostics: Vec<Diagnostic>,
    /// Completion items from extensions for the word starting at the given position.
    pub ext_items: Option<(usize, Vec<super::complete::Item>)>,
    /// The view wants extension completions (position, typed trigger character).
    pub completion_request: Option<(usize, Option<char>)>,
    /// The pointer rests on this position: ask extensions for hover information.
    pub hover_request: Option<usize>,
    /// Hover text from extensions for a position.
    pub hover: Option<(usize, String)>,
    /// Go to the definition of the symbol at this position.
    pub definition_request: Option<usize>,
    /// Text of the file in the last Git commit (LF line breaks), for change markers.
    pub git_base: Option<Arc<String>>,
    /// Git marker per line (0 none, 1 added, 2 modified, 3 lines deleted above) for a document version.
    pub git_marks: Option<(u64, Arc<Vec<u8>>)>,
    /// Matching tag names around the cursor for (version, cursor).
    pub tag_pair: Option<((u64, usize), Option<(Range<usize>, Range<usize>)>)>,
}

/// A problem shown in the editor (squiggle and gutter marker).
#[derive(Clone, Debug)]
pub struct Diagnostic {
    pub range: Range<usize>,
    /// 1 error, 2 warning, 3 info, 4 hint.
    pub severity: u8,
    pub message: String,
}

pub struct Document {
    pub id: u64,
    pub source: DocSource,
    pub language: Language,
    pub line_ending: LineEnding,
    pub encoding: Encoding,
    pub rope: Rope,
    /// Incremented on every content change.
    pub version: u64,
    pub selections: Vec<Selection>,
    /// Index of the selection that the view follows.
    pub primary: usize,
    pub view: ViewState,
    pub highlights: Arc<LineSpans>,
    pub highlight_version: u64,
    /// Version requested from the highlighter but not yet received.
    pub highlight_pending: Option<u64>,
    /// True while an upload/save is in flight.
    pub saving: bool,
    /// Version of the server file this document was loaded from or last saved as.
    pub remote_stamp: Option<crate::remote::Stamp>,
    /// Name shown and proposed on "Save As" for an untitled copy of a file.
    pub suggested_name: Option<String>,
    /// State of the local file when it was loaded or last saved.
    pub disk_stamp: Option<DiskStamp>,
    /// Number of checks in a row that found the local file missing.
    pub disk_missing: u8,
    /// Settings from `.editorconfig` files for this file.
    pub config: crate::editorconfig::Props,
    undo: Vec<Transaction>,
    redo: Vec<Transaction>,
    next_tx_id: u64,
    saved_tx: u64,
    text_cache: Option<(u64, Arc<String>)>,
    /// (tab size, widest line) and line ranges edited since it was computed.
    max_line_cache: Option<(usize, usize)>,
    touched_lines: Vec<(usize, usize)>,
}

static NEXT_DOC_ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

impl Document {
    pub fn new(source: DocSource, text: &str) -> Self {
        let language = Language::from_path(&source.path_string());
        let mut doc = Self {
            id: NEXT_DOC_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
            source,
            language,
            line_ending: LineEnding::Lf,
            encoding: Encoding::Utf8,
            rope: Rope::from_str(text),
            version: 0,
            selections: vec![Selection::cursor(0)],
            primary: 0,
            view: ViewState::default(),
            highlights: Arc::new(Vec::new()),
            highlight_version: u64::MAX,
            highlight_pending: None,
            saving: false,
            remote_stamp: None,
            suggested_name: None,
            disk_stamp: None,
            disk_missing: 0,
            config: Default::default(),
            undo: Vec::new(),
            redo: Vec::new(),
            next_tx_id: 1,
            saved_tx: 0,
            text_cache: None,
            max_line_cache: None,
            touched_lines: Vec::new(),
        };
        if text.contains('\r') {
            doc.rope = Rope::from_str(&text.replace("\r\n", "\n").replace('\r', "\n"));
        }
        doc
    }

    /// Decodes raw file bytes, detecting BOM, encoding and line endings.
    pub fn from_bytes(source: DocSource, bytes: &[u8]) -> Self {
        Self::from_bytes_as(source, bytes, None)
    }

    /// Like [`Self::from_bytes`], but with a given encoding instead of detecting it.
    pub fn from_bytes_as(source: DocSource, bytes: &[u8], encoding: Option<Encoding>) -> Self {
        let encoding = encoding.unwrap_or_else(|| Encoding::detect(bytes));
        let text = encoding.decode(bytes);
        let line_ending = if text.contains("\r\n") {
            LineEnding::CrLf
        } else {
            LineEnding::Lf
        };
        let mut doc = Self::new(source, &text);
        doc.encoding = encoding;
        doc.line_ending = line_ending;
        doc
    }

    /// Encodes the document for writing with its original encoding and line endings.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut text = self.rope.to_string();
        if self.line_ending == LineEnding::CrLf {
            text = text.replace('\n', "\r\n");
        }
        self.encoding.encode(&text)
    }

    pub fn title(&self) -> String {
        match (&self.source, &self.suggested_name) {
            (DocSource::Untitled(_), Some(name)) => name.clone(),
            (source, _) => source.file_name(),
        }
    }

    pub fn is_dirty(&self) -> bool {
        self.undo.last().map_or(0, |t| t.id) != self.saved_tx
    }

    pub fn mark_saved(&mut self) {
        self.saved_tx = self.undo.last().map_or(0, |t| t.id);
    }

    /// Identifies the current undo state; pass it to [`Self::mark_saved_at`]
    /// once an asynchronous save of this state completed.
    pub fn save_point(&mut self) -> u64 {
        // Later typing must start a new undo step, or it would share this id.
        self.break_undo_group();
        self.undo.last().map_or(0, |t| t.id)
    }

    pub fn mark_saved_at(&mut self, point: u64) {
        self.saved_tx = point;
    }

    /// Marks the document as modified without a content change (e.g. after "Save As").
    pub fn mark_unsaved(&mut self) {
        self.saved_tx = u64::MAX;
    }

    /// Full text, cached per version.
    pub fn text(&mut self) -> Arc<String> {
        if let Some((v, text)) = &self.text_cache
            && *v == self.version
        {
            return text.clone();
        }
        let text = Arc::new(self.rope.to_string());
        self.text_cache = Some((self.version, text.clone()));
        text
    }

    /// Width (in display columns) of the longest line. Kept up to date
    /// incrementally: edits only re-measure the lines they touched, so the
    /// value may stay wider than necessary after deleting a long line.
    pub fn max_line_columns(&mut self, tab_size: usize) -> usize {
        let measure = |rope: &Rope, l: usize| {
            let mut col = 0;
            for ch in rope.line(l).chars() {
                match ch {
                    '\t' => col = (col / tab_size + 1) * tab_size,
                    '\n' | '\r' => {}
                    _ => col += 1,
                }
            }
            col
        };
        let lines = self.rope.len_lines();
        let n = match self.max_line_cache {
            Some((tab, n)) if tab == tab_size => self
                .touched_lines
                .iter()
                .flat_map(|&(a, b)| a.min(lines)..=b.min(lines - 1))
                .map(|l| measure(&self.rope, l))
                .fold(n, usize::max),
            _ => (0..lines).map(|l| measure(&self.rope, l)).max().unwrap_or(0),
        };
        self.touched_lines.clear();
        self.max_line_cache = Some((tab_size, n));
        n
    }

    // ---- geometry helpers -------------------------------------------------

    pub fn len_chars(&self) -> usize {
        self.rope.len_chars()
    }

    pub fn len_lines(&self) -> usize {
        self.rope.len_lines()
    }

    pub fn char_to_line(&self, pos: usize) -> usize {
        self.rope.char_to_line(pos.min(self.rope.len_chars()))
    }

    pub fn line_to_char(&self, line: usize) -> usize {
        self.rope.line_to_char(line.min(self.rope.len_lines()))
    }

    /// Number of chars in a line, excluding the line break.
    pub fn line_len(&self, line: usize) -> usize {
        let slice = self.rope.line(line);
        let mut len = slice.len_chars();
        if len > 0 && slice.char(len - 1) == '\n' {
            len -= 1;
        }
        len
    }

    pub fn line_end(&self, line: usize) -> usize {
        self.line_to_char(line) + self.line_len(line)
    }

    pub fn line_text(&self, line: usize) -> String {
        let start = self.line_to_char(line);
        self.rope
            .slice(start..start + self.line_len(line))
            .to_string()
    }

    pub fn slice(&self, range: Range<usize>) -> String {
        let len = self.rope.len_chars();
        self.rope
            .slice(range.start.min(len)..range.end.min(len))
            .to_string()
    }

    pub fn char_at(&self, pos: usize) -> Option<char> {
        (pos < self.rope.len_chars()).then(|| self.rope.char(pos))
    }

    /// Display column of a char position (tabs expanded).
    pub fn column_of(&self, pos: usize, tab_size: usize) -> usize {
        let line = self.char_to_line(pos);
        let start = self.line_to_char(line);
        let mut col = 0;
        for ch in self.rope.slice(start..pos).chars() {
            col = if ch == '\t' {
                (col / tab_size + 1) * tab_size
            } else {
                col + 1
            };
        }
        col
    }

    /// Char position on `line` closest to display column `col`.
    pub fn pos_at_column(&self, line: usize, col: usize, tab_size: usize) -> usize {
        let start = self.line_to_char(line);
        let len = self.line_len(line);
        let mut c = 0;
        for (i, ch) in self.rope.slice(start..start + len).chars().enumerate() {
            let next = if ch == '\t' {
                (c / tab_size + 1) * tab_size
            } else {
                c + 1
            };
            if next > col {
                // Snap to the nearer edge of the character.
                return if col - c > (next - c) / 2 {
                    start + i + 1
                } else {
                    start + i
                };
            }
            c = next;
        }
        start + len
    }

    pub fn primary_selection(&self) -> Selection {
        self.selections[self.primary.min(self.selections.len() - 1)]
    }

    pub fn set_selection(&mut self, sel: Selection) {
        self.selections = vec![sel];
        self.primary = 0;
        self.view.scroll_to_cursor = true;
    }

    /// Sorts selections and merges overlapping ones, keeping the primary.
    pub fn normalize_selections(&mut self) {
        let len = self.len_chars();
        let primary = self.selections.get(self.primary).copied();
        for s in &mut self.selections {
            s.anchor = s.anchor.min(len);
            s.head = s.head.min(len);
        }
        self.selections.sort_by_key(|s| s.range().start);
        let mut merged: Vec<Selection> = Vec::with_capacity(self.selections.len());
        for s in self.selections.drain(..) {
            if let Some(last) = merged.last_mut() {
                let (lr, sr) = (last.range(), s.range());
                if sr.start < lr.end || lr == sr {
                    let start = lr.start.min(sr.start);
                    let end = lr.end.max(sr.end);
                    *last = if last.head >= last.anchor {
                        Selection::new(start, end)
                    } else {
                        Selection::new(end, start)
                    };
                    continue;
                }
            }
            merged.push(s);
        }
        self.selections = merged;
        self.primary = primary
            .and_then(|p| {
                self.selections
                    .iter()
                    .position(|s| s.range().contains(&p.head) || s.head == p.head)
            })
            .unwrap_or(self.selections.len() - 1);
    }

    // ---- editing -----------------------------------------------------------

    /// Applies non-overlapping changes atomically. When `after` is `None`,
    /// selections are mapped through the changes and collapse to the end of
    /// each insertion that covered them.
    pub fn apply(&mut self, mut changes: Vec<Change>, after: Option<Vec<Selection>>, kind: EditKind) {
        changes.retain(|c| !(c.range.is_empty() && c.text.is_empty()));
        if changes.is_empty() {
            if let Some(after) = after {
                self.selections = after;
                self.normalize_selections();
            }
            return;
        }
        changes.sort_by_key(|c| c.range.start);
        let before = self.selections.clone();

        let mut applied = Vec::with_capacity(changes.len());
        let mut delta: isize = 0;
        // (original start, original end, new end) for mapping selections.
        let mut map: Vec<(usize, usize, usize)> = Vec::with_capacity(changes.len());
        for change in changes {
            let start = (change.range.start as isize + delta) as usize;
            let end = (change.range.end as isize + delta) as usize;
            let removed = self.rope.slice(start..end).to_string();
            if !self.view.folds.is_empty() {
                // Keep folds attached to their lines when lines are added or removed above.
                let line = self.rope.char_to_line(start);
                let delta = change.text.matches('\n').count() as isize - removed.matches('\n').count() as isize;
                if delta != 0 {
                    let removed_lines = removed.matches('\n').count();
                    self.view.folds = std::mem::take(&mut self.view.folds)
                        .into_iter()
                        .filter(|&f| f <= line || f > line + removed_lines)
                        .map(|f| if f > line { (f as isize + delta).max(0) as usize } else { f })
                        .collect();
                    self.view.fold_gen += 1;
                }
            }
            self.rope.remove(start..end);
            self.rope.insert(start, &change.text);
            let inserted_len = change.text.chars().count();
            map.push((change.range.start, change.range.end, start + inserted_len));
            self.touched_lines
                .push((self.rope.char_to_line(start), self.rope.char_to_line(start + inserted_len)));
            delta += inserted_len as isize - (end - start) as isize;
            applied.push(AppliedChange {
                start,
                removed,
                inserted: change.text,
            });
        }

        let after = after.unwrap_or_else(|| {
            let map_pos = |pos: usize| -> usize {
                let mut shift: isize = 0;
                for &(s, e, new_end) in &map {
                    if pos < s {
                        break;
                    }
                    if pos <= e {
                        return new_end;
                    }
                    shift = new_end as isize - e as isize;
                }
                (pos as isize + shift) as usize
            };
            before
                .iter()
                .map(|s| Selection {
                    anchor: map_pos(s.anchor),
                    head: map_pos(s.head),
                    goal: None,
                })
                .collect()
        });

        self.redo.clear();
        let now = Instant::now();
        let merge = matches!(kind, EditKind::Typing | EditKind::Deleting)
            && self.undo.last().is_some_and(|t| {
                t.kind == kind
                    && t.id != self.saved_tx
                    && now.duration_since(t.time) < Duration::from_millis(1200)
            });
        if merge {
            let t = self.undo.last_mut().unwrap();
            t.changes.extend(applied);
            t.after = after.clone();
            t.time = now;
        } else {
            let id = self.next_tx_id;
            self.next_tx_id += 1;
            self.undo.push(Transaction {
                id,
                kind,
                changes: applied,
                before,
                after: after.clone(),
                time: now,
            });
        }
        self.selections = after;
        self.normalize_selections();
        self.version += 1;
        self.view.scroll_to_cursor = true;
    }

    /// Replaces every selection with the text returned by `f`.
    pub fn replace_selections(&mut self, kind: EditKind, mut f: impl FnMut(&str) -> String) {
        let changes: Vec<Change> = self
            .selections
            .iter()
            .map(|s| {
                let r = s.range();
                let selected = self.slice(r.clone());
                Change::new(r, f(&selected))
            })
            .collect();
        self.apply(changes, None, kind);
    }

    pub fn undo(&mut self) {
        let Some(tx) = self.undo.pop() else { return };
        self.view.folds.clear();
        for c in tx.changes.iter().rev() {
            let n = c.inserted.chars().count();
            self.rope.remove(c.start..c.start + n);
            self.rope.insert(c.start, &c.removed);
        }
        self.max_line_cache = None;
        self.selections = tx.before.clone();
        self.normalize_selections();
        self.redo.push(tx);
        self.version += 1;
        self.view.scroll_to_cursor = true;
    }

    pub fn redo(&mut self) {
        let Some(tx) = self.redo.pop() else { return };
        self.view.folds.clear();
        for c in &tx.changes {
            let n = c.removed.chars().count();
            self.rope.remove(c.start..c.start + n);
            self.rope.insert(c.start, &c.inserted);
        }
        self.max_line_cache = None;
        self.selections = tx.after.clone();
        self.normalize_selections();
        self.undo.push(tx);
        self.version += 1;
        self.view.scroll_to_cursor = true;
    }

    /// Applies changes one after another, each in the coordinates left by the
    /// previous one (as extensions describe edits), as a single undo step.
    pub fn apply_sequential(&mut self, count: usize, mut change: impl FnMut(&Rope, usize) -> Change) {
        let mut added = 0;
        for i in 0..count {
            let change = change(&self.rope, i);
            let before = self.undo.len();
            self.apply(vec![change], None, EditKind::Other);
            if self.undo.len() > before {
                added += 1;
            }
        }
        if added > 1 {
            let txs: Vec<Transaction> = self.undo.drain(self.undo.len() - added..).collect();
            let last = txs.last().unwrap();
            let (id, after) = (last.id, last.after.clone());
            let before = txs[0].before.clone();
            let changes = txs.into_iter().flat_map(|t| t.changes).collect();
            self.undo.push(Transaction {
                id,
                kind: EditKind::Other,
                changes,
                before,
                after,
                time: Instant::now(),
            });
        }
    }

    /// Replaces the content with new file contents (changed by another
    /// program) as one undoable step. Only the differing middle part is
    /// replaced, so cursors, folds and the scroll position stay in place.
    pub fn reload_bytes(&mut self, bytes: &[u8]) {
        let detected = Encoding::detect(bytes);
        // Keep a chosen Latin-1 unless the file now has a BOM.
        let encoding = match (self.encoding, detected) {
            (Encoding::Latin1, Encoding::Utf8) => Encoding::Latin1,
            _ => detected,
        };
        let text = encoding.decode(bytes);
        self.encoding = encoding;
        self.line_ending = if text.contains("\r\n") { LineEnding::CrLf } else { LineEnding::Lf };
        self.replace_text(&text);
        self.mark_saved();
    }

    /// Replaces the content with `text` as one undoable step, touching only
    /// the differing middle part so cursors, folds and scrolling stay in place.
    pub fn replace_text(&mut self, text: &str) {
        let text = text.replace("\r\n", "\n").replace('\r', "\n");
        let old: Vec<char> = self.rope.chars().collect();
        let new: Vec<char> = text.chars().collect();
        let prefix = old.iter().zip(&new).take_while(|(a, b)| a == b).count();
        let room = old.len().min(new.len()) - prefix;
        let suffix = old.iter().rev().zip(new.iter().rev()).take(room).take_while(|(a, b)| a == b).count();
        let (end, new_end) = (old.len() - suffix, new.len() - suffix);
        let inserted: String = new[prefix..new_end].iter().collect();
        let map = |p: usize| {
            if p <= prefix {
                p
            } else if p >= end {
                p - end + new_end
            } else {
                p.min(new_end)
            }
        };
        let after = self.selections.iter().map(|s| Selection { anchor: map(s.anchor), head: map(s.head), goal: None }).collect();
        self.break_undo_group();
        self.apply(vec![Change::new(prefix..end, inserted)], Some(after), EditKind::Other);
        self.break_undo_group();
        self.view.scroll_to_cursor = false;
    }

    /// Applies a change that belongs to the previous edit (e.g. the partner
    /// tag of a renamed tag), so one undo reverts both.
    pub fn apply_linked(&mut self, change: Change) {
        let before = self.undo.len();
        self.apply(vec![change], None, EditKind::Other);
        if self.undo.len() == before + 1 && before > 0 {
            let tx = self.undo.pop().unwrap();
            let prev = self.undo.last_mut().unwrap();
            prev.changes.extend(tx.changes);
            prev.after = tx.after;
        }
    }

    /// Prevents the next edit from being merged into the previous undo step.
    pub fn break_undo_group(&mut self) {
        if let Some(t) = self.undo.last_mut() {
            t.kind = EditKind::Other;
        }
    }
}

fn decode_latin1(bytes: &[u8]) -> String {
    // Windows-1252 differs from Latin-1 only in 0x80..0x9F.
    const CP1252: [char; 32] = [
        '€', '\u{81}', '‚', 'ƒ', '„', '…', '†', '‡', 'ˆ', '‰', 'Š', '‹', 'Œ', '\u{8D}', 'Ž',
        '\u{8F}', '\u{90}', '‘', '’', '“', '”', '•', '–', '—', '˜', '™', 'š', '›', 'œ', '\u{9D}',
        'ž', 'Ÿ',
    ];
    bytes
        .iter()
        .map(|&b| match b {
            0x80..=0x9F => CP1252[(b - 0x80) as usize],
            _ => b as char,
        })
        .collect()
}

fn encode_latin1(text: &str) -> Vec<u8> {
    text.chars()
        .map(|c| match c as u32 {
            0..=0x7F | 0xA0..=0xFF => c as u8,
            _ => match c {
                '€' => 0x80,
                '‚' => 0x82,
                'ƒ' => 0x83,
                '„' => 0x84,
                '…' => 0x85,
                '†' => 0x86,
                '‡' => 0x87,
                'ˆ' => 0x88,
                '‰' => 0x89,
                'Š' => 0x8A,
                '‹' => 0x8B,
                'Œ' => 0x8C,
                'Ž' => 0x8E,
                '‘' => 0x91,
                '’' => 0x92,
                '“' => 0x93,
                '”' => 0x94,
                '•' => 0x95,
                '–' => 0x96,
                '—' => 0x97,
                '˜' => 0x98,
                '™' => 0x99,
                'š' => 0x9A,
                '›' => 0x9B,
                'œ' => 0x9C,
                'ž' => 0x9E,
                'Ÿ' => 0x9F,
                _ => b'?',
            },
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc(text: &str) -> Document {
        Document::new(DocSource::Untitled(1), text)
    }

    #[test]
    fn multi_cursor_insert_and_undo() {
        let mut d = doc("ab\ncd\n");
        d.selections = vec![Selection::cursor(1), Selection::cursor(4)];
        d.replace_selections(EditKind::Typing, |_| "X".into());
        assert_eq!(d.rope.to_string(), "aXb\ncXd\n");
        assert_eq!(
            d.selections.iter().map(|s| s.head).collect::<Vec<_>>(),
            vec![2, 6]
        );
        assert!(d.is_dirty());
        d.undo();
        assert_eq!(d.rope.to_string(), "ab\ncd\n");
        assert!(!d.is_dirty());
        d.redo();
        assert_eq!(d.rope.to_string(), "aXb\ncXd\n");
    }

    #[test]
    fn typing_merges_into_one_undo_step() {
        let mut d = doc("");
        for ch in ["a", "b", "c"] {
            d.replace_selections(EditKind::Typing, |_| ch.into());
        }
        assert_eq!(d.rope.to_string(), "abc");
        d.undo();
        assert_eq!(d.rope.to_string(), "");
    }

    #[test]
    fn extension_edits_undo_in_one_step() {
        // What Beautify sends: replace everything, then an empty insert
        // that only places the cursor.
        let mut d = doc("a{b:c}");
        d.replace_selections(EditKind::Typing, |_| "x".into());
        let edits = [Change::new(0..7, "a {\n  b: c;\n}x"), Change::new(5..5, "")];
        d.apply_sequential(edits.len(), |_, n| edits[n].clone());
        assert_eq!(d.rope.to_string(), "a {\n  b: c;\n}x");
        d.undo();
        assert_eq!(d.rope.to_string(), "xa{b:c}");
        d.redo();
        assert_eq!(d.rope.to_string(), "a {\n  b: c;\n}x");
    }

    #[test]
    fn crlf_and_latin1_roundtrip() {
        let bytes = b"caf\xE9\r\nok\r\n";
        let d = Document::from_bytes(DocSource::Untitled(1), bytes);
        assert_eq!(d.encoding, Encoding::Latin1);
        assert_eq!(d.line_ending, LineEnding::CrLf);
        assert_eq!(d.rope.to_string(), "café\nok\n");
        assert_eq!(d.to_bytes(), bytes);
    }

    #[test]
    fn utf16_roundtrip_and_forced_encoding() {
        let bytes = Encoding::Utf16Le.encode("äb\r\n");
        let d = Document::from_bytes(DocSource::Untitled(1), &bytes);
        assert_eq!(d.encoding, Encoding::Utf16Le);
        assert_eq!(d.rope.to_string(), "äb\n");
        assert_eq!(d.to_bytes(), bytes);
        // UTF-8 bytes read as Windows-1252 show the two bytes of "ä".
        let d = Document::from_bytes_as(DocSource::Untitled(1), "ä".as_bytes(), Some(Encoding::Latin1));
        assert_eq!(d.rope.to_string(), "Ã¤");
        assert!(!Encoding::Latin1.can_encode("€ und 😀"));
        assert!(Encoding::Latin1.can_encode("€ und ä?"));
    }

    #[test]
    fn columns_with_tabs() {
        let d = doc("\tab");
        assert_eq!(d.column_of(1, 4), 4);
        assert_eq!(d.pos_at_column(0, 5, 4), 2);
        assert_eq!(d.pos_at_column(0, 1, 4), 0);
    }

    #[test]
    fn reload_keeps_cursor_and_is_undoable() {
        let mut d = doc("one\ntwo\nthree\n");
        d.set_selection(Selection::cursor(10)); // in "three"
        d.reload_bytes(b"one\nTWO and more\nthree\n");
        assert_eq!(d.rope.to_string(), "one\nTWO and more\nthree\n");
        assert_eq!(d.primary_selection().head, 19);
        assert!(!d.is_dirty());
        d.undo();
        assert_eq!(d.rope.to_string(), "one\ntwo\nthree\n");
        assert!(d.is_dirty());
        d.redo();
        d.reload_bytes(b"one\r\nTWO and more\r\nthree\r\n");
        assert_eq!(d.line_ending, LineEnding::CrLf);
        assert!(!d.is_dirty());
    }
}
