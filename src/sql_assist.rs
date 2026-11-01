//! SQL help from the real database: the schema (tables and columns) of a
//! server's database, read when the database tab connects and kept per
//! server, completes table and column names in SQL strings in PHP (and in
//! `.sql` files) and flags unknown tables and `table.column` references.

use std::collections::{BTreeMap, HashMap};
use std::ops::Range;
use std::path::PathBuf;
use std::sync::{Arc, LazyLock, RwLock};

use serde::{Deserialize, Serialize};

/// Tables with their columns (in table order).
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Schema {
    pub tables: BTreeMap<String, Vec<String>>,
}

impl Schema {
    /// From the rows of `information_schema.COLUMNS` (table, column).
    pub fn from_rows(rows: &[Vec<Option<String>>]) -> Self {
        let mut tables: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for r in rows {
            if let (Some(Some(t)), Some(Some(c))) = (r.first(), r.get(1)) {
                tables.entry(t.clone()).or_default().push(c.clone());
            }
        }
        Self { tables }
    }

    fn table(&self, name: &str) -> Option<(&String, &Vec<String>)> {
        let name = name.rsplit('.').next().unwrap_or(name);
        self.tables.iter().find(|(t, _)| t.eq_ignore_ascii_case(name))
    }
}

/// Schemas per server; loaded from disk on first use.
static SCHEMAS: LazyLock<RwLock<HashMap<String, Option<Arc<Schema>>>>> = LazyLock::new(Default::default);

fn path(site_id: &str) -> PathBuf {
    let safe: String = site_id.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_').collect();
    crate::settings::config_dir().join("db_schema").join(format!("{safe}.json"))
}

/// Stores the schema of a server's database (memory and disk).
pub fn set(site_id: &str, schema: Schema) {
    let p = path(site_id);
    if let (Some(dir), Ok(json)) = (p.parent(), serde_json::to_vec(&schema)) {
        let _ = std::fs::create_dir_all(dir);
        let _ = crate::safe_write::write(&p, &json);
    }
    SCHEMAS.write().unwrap().insert(site_id.to_string(), Some(Arc::new(schema)));
}

pub fn get(site_id: &str) -> Option<Arc<Schema>> {
    if let Some(s) = SCHEMAS.read().unwrap().get(site_id) {
        return s.clone();
    }
    let loaded = std::fs::read(path(site_id)).ok().and_then(|b| serde_json::from_slice::<Schema>(&b).ok()).map(Arc::new);
    SCHEMAS.write().unwrap().insert(site_id.to_string(), loaded.clone());
    loaded
}

/// The query that reads the schema of the current database.
pub const SCHEMA_QUERY: &str = "SELECT TABLE_NAME, COLUMN_NAME FROM information_schema.COLUMNS WHERE TABLE_SCHEMA = DATABASE() ORDER BY TABLE_NAME, ORDINAL_POSITION";

fn looks_like_sql(s: &str) -> bool {
    let t = s.trim_start().to_ascii_uppercase();
    ["SELECT ", "INSERT ", "UPDATE ", "DELETE ", "REPLACE ", "WITH "].iter().any(|k| t.starts_with(k))
        || t.starts_with("SELECT\n")
        || (t.contains(" FROM ") && t.contains("SELECT"))
}

/// Byte ranges of SQL: the contents of PHP string literals that hold SQL
/// (inside `<?php … ?>` only), or everything in a `.sql` file.
pub fn sql_ranges(text: &str, is_sql_file: bool) -> Vec<Range<usize>> {
    if is_sql_file {
        return vec![0..text.len()];
    }
    let b = text.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    let mut in_php = false;
    while i < b.len() {
        if !in_php {
            match text[i..].find("<?") {
                Some(o) => {
                    in_php = true;
                    i += o + 2;
                }
                None => break,
            }
            continue;
        }
        match b[i] {
            b'?' if b.get(i + 1) == Some(&b'>') => {
                in_php = false;
                i += 2;
            }
            // Comments.
            b'/' if b.get(i + 1) == Some(&b'/') => i = text[i..].find('\n').map_or(b.len(), |o| i + o),
            b'#' => i = text[i..].find('\n').map_or(b.len(), |o| i + o),
            b'/' if b.get(i + 1) == Some(&b'*') => i = text[i + 2..].find("*/").map_or(b.len(), |o| i + o + 4),
            q @ (b'"' | b'\'') => {
                let start = i + 1;
                let mut j = start;
                while j < b.len() && b[j] != q {
                    j += if b[j] == b'\\' { 2 } else { 1 };
                }
                let end = j.min(b.len());
                if looks_like_sql(&text[start..end]) {
                    out.push(start..end);
                }
                i = end + 1;
            }
            _ => i += 1,
        }
    }
    out
}

const STOP: &[&str] = &[
    "WHERE", "JOIN", "LEFT", "RIGHT", "INNER", "OUTER", "CROSS", "ON", "USING", "GROUP", "ORDER", "LIMIT", "SET", "VALUES", "VALUE", "HAVING", "UNION", "AS", "SELECT", "FROM", "INTO",
    "AND", "OR", "NATURAL", "STRAIGHT_JOIN", "FOR", "WINDOW", "OFFSET",
];

#[derive(Debug, Clone, PartialEq)]
enum Tok {
    /// Identifier (backticks removed), with its byte range in the SQL.
    Ident(String, Range<usize>),
    Dot,
    Comma,
    Other,
}

fn tokens(sql: &str) -> Vec<Tok> {
    let b = sql.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        let c = b[i];
        if c == b'`' {
            let end = sql[i + 1..].find('`').map_or(b.len(), |o| i + 1 + o);
            out.push(Tok::Ident(sql[i + 1..end].to_string(), i + 1..end));
            i = end + 1;
        } else if c.is_ascii_alphabetic() || c == b'_' || c == b'$' || c == b'{' {
            let start = i;
            // PHP variables (`$t`, `{$t}`, `$row->x`) are kept as one token and never checked.
            let var = c == b'$' || c == b'{';
            i += 1;
            while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == b'_' || b[i] >= 0x80 || var && matches!(b[i], b'$' | b'{' | b'}' | b'-' | b'>')) {
                i += 1;
            }
            out.push(Tok::Ident(sql[start..i].to_string(), start..i));
        } else if c == b'\'' || c == b'"' {
            // String values inside the SQL.
            i = sql[i + 1..].find(c as char).map_or(b.len(), |o| i + 2 + o);
            out.push(Tok::Other);
        } else {
            out.push(match c {
                b'.' => Tok::Dot,
                b',' => Tok::Comma,
                _ if c.is_ascii_whitespace() => {
                    i += 1;
                    continue;
                }
                _ => Tok::Other,
            });
            i += 1;
        }
    }
    out
}

fn is_kw(t: &Tok, kw: &str) -> bool {
    matches!(t, Tok::Ident(s, _) if s.eq_ignore_ascii_case(kw))
}

fn is_var(name: &str) -> bool {
    name.contains('$') || name.contains('{')
}

/// A table named in the statement, with its alias and where it is written.
#[derive(Debug, Clone)]
struct TableRef {
    name: String,
    alias: Option<String>,
    range: Range<usize>,
}

fn table_refs(toks: &[Tok]) -> Vec<TableRef> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < toks.len() {
        let starts = ["FROM", "JOIN", "UPDATE", "INTO"].iter().any(|k| is_kw(&toks[i], k));
        i += 1;
        if !starts {
            continue;
        }
        // `FROM a, b x, c AS y`
        loop {
            let Some(Tok::Ident(name, range)) = toks.get(i).cloned() else { break };
            if STOP.iter().any(|k| name.eq_ignore_ascii_case(k)) {
                break;
            }
            i += 1;
            // `db.table`
            let (mut name, mut range) = (name, range);
            if toks.get(i) == Some(&Tok::Dot)
                && let Some(Tok::Ident(n, r)) = toks.get(i + 1).cloned()
            {
                name = n;
                range = r;
                i += 2;
            }
            if toks.get(i).is_some_and(|t| is_kw(t, "AS")) {
                i += 1;
            }
            let alias = match toks.get(i) {
                Some(Tok::Ident(a, _)) if !STOP.iter().any(|k| a.eq_ignore_ascii_case(k)) => {
                    i += 1;
                    Some(a.clone())
                }
                _ => None,
            };
            out.push(TableRef { name, alias, range });
            if toks.get(i) == Some(&Tok::Comma) {
                i += 1;
                continue;
            }
            break;
        }
    }
    out
}

/// Problems in one SQL string: (byte range in the SQL, message).
pub fn check(sql: &str, schema: &Schema) -> Vec<(Range<usize>, String)> {
    let toks = tokens(sql);
    let refs = table_refs(&toks);
    let mut out = Vec::new();
    for r in &refs {
        if !is_var(&r.name) && schema.table(&r.name).is_none() {
            out.push((r.range.clone(), t!("Tabelle „{name}“ gibt es in der Datenbank nicht", name = r.name)));
        }
    }
    // `alias.column` and `table.column`.
    for w in toks.windows(3) {
        let (Tok::Ident(owner, _), Tok::Dot, Tok::Ident(col, range)) = (&w[0], &w[1], &w[2]) else { continue };
        if is_var(owner) || is_var(col) {
            continue;
        }
        let table = refs
            .iter()
            .find(|r| r.alias.as_deref().is_some_and(|a| a.eq_ignore_ascii_case(owner)) || r.name.eq_ignore_ascii_case(owner))
            .and_then(|r| schema.table(&r.name));
        if let Some((tname, cols)) = table
            && !cols.iter().any(|c| c.eq_ignore_ascii_case(col))
        {
            out.push((range.clone(), t!("Spalte „{col}“ gibt es in „{table}“ nicht", col = col, table = tname)));
        }
    }
    out
}

/// Suggestions at byte `pos` of `sql`: (name, detail, is_table).
pub fn suggest(sql: &str, pos: usize, schema: &Schema) -> Vec<(String, String, bool)> {
    let pos = pos.min(sql.len());
    let toks = tokens(sql);
    let refs = table_refs(&toks);
    // The word being typed and what is in front of it.
    let start = sql[..pos].rfind(|c: char| !(c.is_alphanumeric() || c == '_')).map_or(0, |i| i + 1);
    let before = sql[..start].trim_end();
    let mut out = Vec::new();
    if let Some(owner) = before.strip_suffix('.') {
        let owner = owner.rsplit(|c: char| !(c.is_alphanumeric() || c == '_' || c == '`')).next().unwrap_or("").trim_matches('`');
        let table = refs
            .iter()
            .find(|r| r.alias.as_deref().is_some_and(|a| a.eq_ignore_ascii_case(owner)))
            .map(|r| r.name.clone())
            .unwrap_or_else(|| owner.to_string());
        if let Some((t, cols)) = schema.table(&table) {
            out.extend(cols.iter().map(|c| (c.clone(), t.clone(), false)));
        }
        return out;
    }
    let last_word = before.rsplit(|c: char| c.is_whitespace() || c == ',').next().unwrap_or("").to_ascii_uppercase();
    let wants_table = ["FROM", "JOIN", "UPDATE", "INTO", "TABLE"].contains(&last_word.as_str()) || before.ends_with(',') && {
        // A comma in the FROM list.
        let up = before.to_ascii_uppercase();
        up.rfind("FROM") > up.rfind("WHERE").max(up.rfind("SELECT"))
    };
    if !wants_table {
        // Columns of the tables the statement uses first.
        for r in &refs {
            if let Some((t, cols)) = schema.table(&r.name) {
                out.extend(cols.iter().map(|c| (c.clone(), t.clone(), false)));
            }
        }
    }
    out.extend(schema.tables.keys().map(|t| (t.clone(), t!("Tabelle").to_string(), true)));
    let mut seen = std::collections::HashSet::new();
    out.retain(|(n, _, _)| seen.insert(n.to_lowercase()));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn schema() -> Schema {
        let mut tables = BTreeMap::new();
        tables.insert("users".to_string(), vec!["id".into(), "name".into(), "email".into()]);
        tables.insert("orders".to_string(), vec!["id".into(), "user_id".into(), "total".into()]);
        Schema { tables }
    }

    #[test]
    fn finds_sql_strings_in_php() {
        let php = "<p>SELECT nicht</p><?php\n// \"SELECT x FROM y\"\n$q = \"SELECT * FROM users WHERE id = $id\";\n$n = 'Hallo';\n$u = 'UPDATE orders SET total = 1'; ?>";
        let found: Vec<&str> = sql_ranges(php, false).into_iter().map(|r| &php[r]).collect();
        assert_eq!(found, ["SELECT * FROM users WHERE id = $id", "UPDATE orders SET total = 1"]);
    }

    #[test]
    fn checks_tables_and_columns() {
        let s = schema();
        let sql = "SELECT u.name, u.mail, o.total FROM users u JOIN orders AS o ON o.user_id = u.id JOIN logs l";
        let problems: Vec<(&str, String)> = check(sql, &s).into_iter().map(|(r, m)| (&sql[r], m)).collect();
        assert_eq!(problems.len(), 2, "{problems:?}");
        assert_eq!(problems[0].0, "logs");
        assert_eq!(problems[1].0, "mail");
        // Variables and database prefixes are fine.
        assert!(check("SELECT * FROM $table t WHERE t.x = 1", &s).is_empty());
        assert!(check("SELECT shop.users.name FROM shop.users", &s).is_empty());
        assert!(check("INSERT INTO `users` (name) VALUES ('a, b FROM c')", &s).is_empty());
    }

    #[test]
    fn suggests_by_context() {
        let s = schema();
        let names = |sql: &str, pos: usize| suggest(sql, pos, &s).into_iter().map(|(n, _, _)| n).collect::<Vec<_>>();
        let sql = "SELECT o. FROM orders o";
        assert_eq!(names(sql, 9), ["id", "user_id", "total"]);
        let sql = "SELECT * FROM ";
        assert_eq!(names(sql, sql.len()), ["orders", "users"]);
        // Columns of the used table come first, then tables.
        let sql = "SELECT na FROM users";
        assert_eq!(names(sql, 9)[..3], ["id", "name", "email"]);
        assert!(names(sql, 9).contains(&"orders".to_string()));
    }
}
