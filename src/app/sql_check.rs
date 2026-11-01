//! Gives PHP and SQL files the database schema of their server (for
//! completion) and marks unknown tables and columns in their SQL.

use std::sync::Arc;

use super::NebulaApp;
use crate::editor::DocSource;
use crate::editor::document::{Diagnostic, Origin};
use crate::lang::Language;
use crate::sql_assist;

impl NebulaApp {
    /// The server a document belongs to: its own, or the one its project publishes to.
    fn site_of(&self, source: &DocSource) -> Option<String> {
        match source {
            DocSource::Remote { site_id, .. } => Some(site_id.clone()),
            DocSource::Local(p) => self.settings.publish.iter().find(|(root, _)| p.starts_with(root.as_str())).map(|(_, t)| t.site_id.clone()),
            DocSource::Untitled(_) => None,
        }
    }

    /// Called every frame; cheap unless a file or schema changed.
    pub(super) fn sql_tick(&mut self) {
        for i in 0..self.docs.len() {
            let language = self.docs[i].language;
            if !matches!(language, Language::Php | Language::Sql) || self.docs[i].hex.is_some() {
                continue;
            }
            let schema = self.site_of(&self.docs[i].source).and_then(|s| sql_assist::get(&s));
            let key = (self.docs[i].version, schema.as_ref().map_or(0, |s| Arc::as_ptr(s) as usize));
            let doc = &mut self.docs[i];
            doc.view.sql_schema = schema.clone();
            if self.sql_checked.get(&doc.id) == Some(&key) {
                continue;
            }
            self.sql_checked.insert(doc.id, key);
            let mut marks = Vec::new();
            if let Some(schema) = &schema {
                let text = doc.text();
                // Very large files are not checked (this runs while typing).
                if text.len() <= 512 * 1024 {
                    for range in sql_assist::sql_ranges(&text, language == Language::Sql) {
                        for (r, message) in sql_assist::check(&text[range.clone()], schema) {
                            let start = text[..range.start + r.start].chars().count();
                            let len = text[range.start + r.start..range.start + r.end].chars().count();
                            marks.push(Diagnostic { range: start..start + len.max(1), severity: 2, message: format!("{}: {message}", t!("Datenbank")), origin: Origin::Sql });
                        }
                    }
                }
            }
            if marks.is_empty() && !doc.view.diagnostics.iter().any(|d| d.origin == Origin::Sql) {
                continue;
            }
            doc.view.diagnostics.retain(|d| d.origin != Origin::Sql);
            doc.view.diagnostics.extend(marks);
            doc.view.diagnostics.sort_by_key(|d| d.range.start);
        }
        let ids: Vec<u64> = self.docs.iter().map(|d| d.id).collect();
        self.sql_checked.retain(|id, _| ids.contains(id));
    }
}
