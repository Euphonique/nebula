//! Messages between the UI thread and the extension host thread.
//!
//! The message types mirror what the JavaScript runtime sends; not every
//! field is used by the UI yet.
#![allow(dead_code)]
//!
//! Positions sent by extensions are UTF-16 offsets (JavaScript string
//! indices); the UI converts them with the document's rope.

use ropey::Rope;
use serde::Deserialize;
use serde_json::Value;

use super::manifest::Extension;

/// Snapshot of an open document for the extension host.
#[derive(Clone, Debug)]
pub struct DocInfo {
    pub id: u64,
    pub uri: String,
    pub path: Option<String>,
    pub syntax: Option<String>,
    pub remote: bool,
    pub dirty: bool,
    pub eol: String,
    pub tab_length: usize,
    pub soft_tabs: bool,
}

/// UI -> host.
pub enum ToHost {
    /// The set of enabled extensions changed (also sent at startup).
    Load(Vec<Extension>),
    Workspace(Option<String>),
    DocOpened(DocInfo, Rope),
    DocChanged { id: u64, text: Rope, dirty: bool },
    /// Selections as (start, end) char indices, primary first.
    Selection { id: u64, ranges: Vec<(usize, usize)> },
    DocMeta(DocInfo),
    DocSaved(u64),
    DocClosed(u64),
    ActiveDoc(Option<u64>),
    Command { ext: String, name: String, editor: Option<u64>, args: Vec<Value> },
    /// A preference was changed in the UI.
    ConfigChanged { workspace: bool, key: String },
    /// Completion request at a char position.
    Completion { req: u64, doc: u64, pos: usize, trigger: Option<String> },
    Hover { req: u64, doc: u64, pos: usize },
    Definition { req: u64, doc: u64, pos: usize },
    WillSave { req: u64, doc: u64 },
    /// Answer to a callback the extension is waiting for (palettes, panels...).
    Reply { ext: String, cb: u64, value: Value },
    /// Tree view requests: children of an element (`None` for the root).
    TreeChildren { ext: String, view: String, req: u64, element: Option<u64> },
    TreeEvent { ext: String, view: String, kind: String, element: Option<u64> },
}

#[derive(Deserialize, Debug, Clone)]
pub struct EditOp {
    pub start: usize,
    pub end: usize,
    #[serde(default)]
    pub text: String,
    #[serde(default)]
    pub snippet: bool,
    /// `insert` moves the cursor after the text, `replace` doesn't.
    #[serde(default)]
    pub insert: bool,
}

#[derive(Deserialize, Debug, Clone)]
pub struct IssueMsg {
    #[serde(default)]
    pub message: String,
    #[serde(default)]
    pub code: Option<Value>,
    /// 1 error, 2 warning, 3 info, 4 hint (as in LSP).
    #[serde(default)]
    pub severity: u8,
    #[serde(default)]
    pub source: Option<String>,
    /// UTF-16 range, or 1-based line/column.
    pub start: Option<usize>,
    pub end: Option<usize>,
    pub line: Option<usize>,
    pub column: Option<usize>,
    #[serde(rename = "endLine")]
    pub end_line: Option<usize>,
    #[serde(rename = "endColumn")]
    pub end_column: Option<usize>,
}

#[derive(Deserialize, Debug, Clone)]
pub struct CompletionMsg {
    pub label: String,
    #[serde(default)]
    pub kind: Option<String>,
    #[serde(default)]
    pub detail: Option<String>,
    #[serde(default)]
    pub documentation: Option<String>,
    #[serde(rename = "insertText", default)]
    pub insert_text: Option<String>,
    #[serde(rename = "filterText", default)]
    pub filter_text: Option<String>,
    #[serde(default)]
    pub snippet: bool,
    /// UTF-16 range replaced by the item.
    #[serde(default)]
    pub start: Option<usize>,
    #[serde(default)]
    pub end: Option<usize>,
    #[serde(default)]
    pub color: Option<[f32; 4]>,
}

#[derive(Deserialize, Debug, Clone)]
pub struct TreeItemMsg {
    pub id: u64,
    pub name: String,
    #[serde(default)]
    pub collapsible: u8,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub tooltip: Option<String>,
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub command: Option<String>,
    #[serde(default)]
    pub context: Option<String>,
    #[serde(default)]
    pub color: Option<[f32; 4]>,
    #[serde(default)]
    pub image: Option<String>,
}

#[derive(Deserialize, Debug, Clone)]
pub struct Location {
    pub uri: String,
    pub line: usize,
    pub column: usize,
}

/// Host -> UI. Most of these are posted by the JavaScript runtime as JSON.
#[derive(Deserialize, Debug)]
#[serde(tag = "type")]
pub enum FromHost {
    #[serde(rename = "edit")]
    Edit { doc: u64, edits: Vec<EditOp>, cb: Option<u64> },
    /// Replace the selections with text (editor.insert()).
    #[serde(rename = "insert")]
    Insert { doc: u64, text: String, #[serde(default)] snippet: bool, cb: Option<u64> },
    #[serde(rename = "select")]
    Select { doc: u64, ranges: Vec<[usize; 2]> },
    #[serde(rename = "scroll")]
    Scroll { doc: u64, pos: usize },
    #[serde(rename = "save")]
    Save { doc: u64, cb: Option<u64> },
    #[serde(rename = "message")]
    Message { level: String, text: String },
    #[serde(rename = "actionPanel")]
    ActionPanel { cb: Option<u64>, message: String, #[serde(default)] buttons: Vec<String> },
    #[serde(rename = "inputPanel")]
    InputPanel {
        cb: Option<u64>,
        message: String,
        #[serde(default)]
        label: String,
        #[serde(default)]
        placeholder: String,
        #[serde(default)]
        value: String,
        #[serde(default)]
        prompt: String,
        #[serde(default)]
        secure: bool,
    },
    #[serde(rename = "inputPalette")]
    InputPalette {
        cb: Option<u64>,
        #[serde(default)]
        message: String,
        #[serde(default)]
        placeholder: String,
        #[serde(default)]
        value: String,
    },
    #[serde(rename = "choicePalette")]
    ChoicePalette { cb: Option<u64>, choices: Vec<String>, #[serde(default)] placeholder: String },
    #[serde(rename = "fileChooser")]
    FileChooser {
        cb: Option<u64>,
        #[serde(default)]
        message: String,
        #[serde(default, rename = "allowFiles")]
        allow_files: Option<bool>,
        #[serde(default, rename = "allowFolders")]
        allow_folders: Option<bool>,
        #[serde(default, rename = "allowMultiple")]
        allow_multiple: Option<bool>,
    },
    #[serde(rename = "notify")]
    Notify {
        cb: Option<u64>,
        id: String,
        #[serde(default)]
        title: String,
        #[serde(default)]
        body: String,
        #[serde(default)]
        kind: String,
        #[serde(default)]
        value: String,
        #[serde(default)]
        placeholder: String,
        #[serde(default)]
        actions: Vec<String>,
    },
    #[serde(rename = "cancelNotification")]
    CancelNotification { id: String },
    #[serde(rename = "issues")]
    Issues { collection: u64, #[serde(default)] name: String, uri: String, issues: Vec<IssueMsg> },
    #[serde(rename = "clearIssues")]
    ClearIssues { collection: u64 },
    #[serde(rename = "completions")]
    Completions { req: u64, items: Vec<CompletionMsg> },
    #[serde(rename = "hover")]
    Hover { req: u64, text: Option<String> },
    #[serde(rename = "definition")]
    Definition { req: u64, locations: Vec<Location> },
    #[serde(rename = "willSaveDone")]
    WillSaveDone { req: u64 },
    #[serde(rename = "openFile")]
    OpenFile { cb: Option<u64>, uri: String, line: Option<usize>, column: Option<usize> },
    #[serde(rename = "newDocument")]
    NewDocument { cb: Option<u64>, #[serde(default)] content: String, syntax: Option<String> },
    #[serde(rename = "openUrl")]
    OpenUrl { url: String },
    #[serde(rename = "openConfig")]
    OpenConfig { identifier: Option<String> },
    #[serde(rename = "log")]
    Log { level: String, text: String },
    #[serde(rename = "treeChildren")]
    TreeChildren { view: String, req: u64, items: Vec<TreeItemMsg> },
    #[serde(rename = "treeReload")]
    TreeReload { view: String },
    #[serde(rename = "treeView")]
    TreeView { view: String, alive: bool },
    #[serde(rename = "contextChanged")]
    ContextChanged { key: String, value: Value },
    /// Host-internal status (activation, failures).
    #[serde(skip)]
    Activated,
    #[serde(skip)]
    Failed(String),
}

/// A message from the host, tagged with the extension it came from.
#[derive(Debug)]
pub struct HostEvent {
    pub ext: String,
    pub msg: FromHost,
}
