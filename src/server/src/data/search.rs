//! Full-text search: content extraction, query sanitization, and the search
//! result types. The query/rank/filter execution lives in `sqlite.rs`.
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::data::document::Document;

/// One search result: the per-recipient-filtered document, its BM25 relevance
/// (lower = more relevant, as SQLite returns it), and a highlighted snippet.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/")]
pub struct SearchHit {
    pub document: Document,
    pub score: f64,
    pub snippet: String,
}

/// A page of search hits plus an opaque cursor (raw-rank offset) for the next
/// page, or `None` when the ranked candidates are exhausted.
#[derive(Debug, Clone)]
pub struct SearchPage {
    pub hits: Vec<SearchHit>,
    pub next_cursor: Option<i64>,
}

/// Extract indexable text from a document, content-agnostically: every string
/// and number leaf value of the `system` body (recursing objects and arrays),
/// plus the `doc_type`. Keys, booleans, nulls, and the envelope are excluded.
pub fn index_content(doc: &Document) -> String {
    let mut out = String::new();
    out.push_str(&doc.doc_type);
    collect_leaves(&doc.system, &mut out);
    out
}

fn collect_leaves(value: &serde_json::Value, out: &mut String) {
    match value {
        serde_json::Value::String(s) => {
            out.push(' ');
            out.push_str(s);
        }
        serde_json::Value::Number(n) => {
            out.push(' ');
            out.push_str(&n.to_string());
        }
        serde_json::Value::Array(items) => {
            for v in items {
                collect_leaves(v, out);
            }
        }
        serde_json::Value::Object(map) => {
            for v in map.values() {
                collect_leaves(v, out);
            }
        }
        // Bool / Null carry no useful search text.
        _ => {}
    }
}

/// Build a safe FTS5 MATCH expression from untrusted input. Each whitespace
/// token is stripped of embedded quotes, wrapped in double quotes (so any
/// remaining FTS5 special characters are treated as literal token separators,
/// never operators), and AND-combined. The final token gets a trailing `*` for
/// type-ahead prefix matching. Returns `None` for an empty query.
pub fn build_match(input: &str) -> Option<String> {
    let terms: Vec<String> = input
        .split_whitespace()
        .map(|t| {
            t.replace('"', " ")
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ")
        })
        .map(|t| t.trim().to_string())
        .filter(|t| !t.is_empty())
        .collect();
    if terms.is_empty() {
        return None;
    }
    let mut parts: Vec<String> = terms.iter().map(|t| format!("\"{t}\"")).collect();
    let last = parts.len() - 1;
    parts[last].push('*');
    Some(parts.join(" "))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::document::{Document, PermissionSet, Scope};
    use uuid::Uuid;

    fn doc(doc_type: &str, system: serde_json::Value) -> Document {
        Document {
            id: Uuid::from_u128(1),
            scope: Scope::World {
                world_id: Uuid::from_u128(9),
            },
            doc_type: doc_type.into(),
            schema_version: 1,
            source: None,
            owner: None,
            permissions: PermissionSet::default(),
            embedded: Default::default(),
            system,
            created_at: 0,
            updated_at: 0,
        }
    }

    #[test]
    fn extracts_string_and_number_leaves_and_doc_type() {
        let d = doc(
            "actor",
            serde_json::json!({
                "name": "Goblin Scout",
                "hp": 12,
                "traits": ["sneaky", "cowardly"],
                "nested": { "weapon": "shortbow" },
                "hidden": true,
                "nothing": null
            }),
        );
        let c = index_content(&d);
        for needle in [
            "actor",
            "Goblin Scout",
            "12",
            "sneaky",
            "cowardly",
            "shortbow",
        ] {
            assert!(c.contains(needle), "content missing {needle:?}: {c}");
        }
        // Keys and non-text leaves are not indexed.
        assert!(!c.contains("weapon"));
        assert!(!c.contains("true"));
    }

    #[test]
    fn build_match_quotes_terms_and_prefixes_last() {
        assert_eq!(build_match("gob scout").unwrap(), "\"gob\" \"scout\"*");
        assert_eq!(build_match("dragon").unwrap(), "\"dragon\"*");
    }

    #[test]
    fn build_match_neutralizes_fts_operators() {
        let m = build_match("fire OR \"x\" -bomb").unwrap();
        // Bare operators do not reach MATCH as syntax (every token is quoted).
        assert!(!m.contains("OR "));
        assert!(m.starts_with('"'));
        // The stray quote is stripped, not emitted as a raw operator quote.
        assert!(!m.contains("\"x\"\"x\""));
    }

    #[test]
    fn build_match_empty_is_none() {
        assert!(build_match("   ").is_none());
        assert!(build_match("").is_none());
    }
}
