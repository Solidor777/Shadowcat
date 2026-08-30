//! The engine's default reference resolver: a dotted path is read LITERALLY
//! from a document's `system` band (`a.b.c` → `/system/a/b/c`). No vocabulary
//! is baked in — where a system keeps its variables (e.g. `system.stats`) is
//! the system's choice, expressed in the formulas it authors. The server
//! reads a number because a formula named it; it knows nothing about what
//! the leaf means.

#![deny(missing_docs)]
#![deny(clippy::missing_docs_in_private_items)]

use serde_json::Value;

use super::evaluate::Resolve;
use super::types::{FormulaError, FormulaErrorKind, FormulaValue};
use crate::data::document::Document;

/// Resolves references against one document's `system` band.
pub struct SystemLeafResolver<'a> {
    /// The document whose `system` band is read.
    doc: &'a Document,
}

impl<'a> SystemLeafResolver<'a> {
    /// A resolver over `doc.system`.
    pub fn new(doc: &'a Document) -> Self {
        Self { doc }
    }
}

impl Resolve for SystemLeafResolver<'_> {
    fn resolve(&self, path: &[String]) -> FormulaValue {
        let joined = path.join(".");
        let mut cur: &Value = &self.doc.system;
        for seg in path {
            match cur {
                Value::Object(map) => match map.get(seg) {
                    Some(v) => cur = v,
                    None => return unknown(&joined),
                },
                _ => return unknown(&joined),
            }
        }
        match cur.as_f64() {
            Some(n) => Ok(n),
            None => Err(FormulaError::new(
                FormulaErrorKind::Type,
                format!("'{joined}' is not a number"),
            )),
        }
    }
}

/// An `UnknownRef` for `joined` — the one place this wording lives; the
/// no-host resolver in `combat::eval` reuses it.
pub(crate) fn unknown(joined: &str) -> FormulaValue {
    Err(FormulaError::new(
        FormulaErrorKind::UnknownRef,
        format!("unknown reference '{joined}'"),
    ))
}

#[cfg(test)]
mod tests;
