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

/// Like [`index_content`], but indexes only text a non-GM may read: GM-only
/// properties are stripped first via the same `filter_properties` redaction used
/// for document reads, so this is the index a non-GM search matches and snippets
/// against. Keeps the searchable text in exact lockstep with what the recipient
/// could otherwise see — no GM-only leak through MATCH, score, or snippet.
pub fn index_content_public(doc: &Document) -> String {
    let non_gm = crate::data::permission::Access {
        caps: std::collections::BTreeSet::new(),
        all: false,
        see_gm_only: false,
    };
    index_content(&crate::data::permission::filter_properties(doc, &non_gm))
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
/// token is reduced to its alphanumeric runs (every non-word character — quotes,
/// FTS5 operators `-^*:()`, `NEAR`-inducing punctuation — becomes a separator),
/// then each surviving token is wrapped in double quotes and AND-combined; the
/// final token gets a trailing `*` for type-ahead prefix matching. Reducing to
/// word characters means a punctuation-only query cannot reach the MATCH parser
/// as a term-less phrase (which FTS5 would reject) — it yields `None`, an empty
/// result, instead. Returns `None` for an empty query.
pub fn build_match(input: &str) -> Option<String> {
    // Bound the work an untrusted query can drive: cap the length (by chars, so
    // never splitting a UTF-8 boundary) and the number of terms.
    const MAX_QUERY_CHARS: usize = 256;
    const MAX_TERMS: usize = 16;
    let capped: String = input.chars().take(MAX_QUERY_CHARS).collect();
    let terms: Vec<String> = capped
        .split(|c: char| !c.is_alphanumeric())
        .filter(|t| !t.is_empty())
        .take(MAX_TERMS)
        .map(|t| t.to_string())
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

    #[test]
    fn build_match_punctuation_only_is_none() {
        // Punctuation-only input must not reach FTS5 as a term-less phrase
        // (which the parser rejects). It reduces to no terms → None → empty page.
        for q in ["---", "*", "\"\"\"", "()", ":::", "^", "- ^ *"] {
            assert!(build_match(q).is_none(), "expected None for {q:?}");
        }
    }
}
