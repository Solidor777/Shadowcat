//! Engine body for a rollable table (`TABLE_DOC_TYPE`): a set of weighted or
//! formula-ranged rows a `DrawTable` frame draws from server-side (see
//! `crate::tables`). Envelope `name` is the table's display name; this module
//! owns only the stored shape and its ingress validation.

// Ratchet: every item in this module must carry a doc comment, enforced by
// the two crate-level deny attributes this module declares.
#![deny(missing_docs)]
#![deny(clippy::missing_docs_in_private_items)]

use serde::{Deserialize, Serialize};
use ts_rs::TS;
use uuid::Uuid;

use crate::chat::{DocLinkTarget, MAX_IMAGE_ALT_CHARS};

/// Doc_type for a rollable table: a standalone world document, never embedded
/// (see `data::validation::validate_containment`'s `table` arm).
pub const TABLE_DOC_TYPE: &str = "table";

/// Row cap: `TableEngine::validate` refuses a table beyond this size.
pub const MAX_TABLE_ROWS: usize = 1000;
/// Cap on `TableRow.label`.
pub const MAX_ROW_LABEL_CHARS: usize = 200;
/// Cap on `TableEntry::Text.text`.
pub const MAX_ROW_TEXT_CHARS: usize = 2000;
/// Cap on `TableEngine.description`.
pub const MAX_TABLE_DESCRIPTION_CHARS: usize = 2000;
/// Cap on `TableEntry::Draw.count`; also the floor (`1..=MAX_NESTED_DRAW_COUNT`).
pub const MAX_NESTED_DRAW_COUNT: u32 = 10;

/// The engine body of a rollable table. Envelope `name` is the table's name.
///
/// # Examples
///
/// ```
/// use shadowcat::data::engine::table::{DrawRule, TableEngine, TableRow};
///
/// let table = TableEngine {
///     draw: DrawRule::Weighted,
///     rows: vec![TableRow { weight: 1, range: None, label: "Miss".into(), results: vec![] }],
///     description: "A simple hit table.".into(),
/// };
/// assert_eq!(table.rows.len(), 1);
/// assert!(table.validate().is_ok());
/// ```
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/engine/")]
#[serde(deny_unknown_fields)]
pub struct TableEngine {
    /// How a draw selects a row.
    pub draw: DrawRule,
    /// The rows, in display order. Whole-array replaced on edit (`set_pointer`
    /// cannot grow arrays), same as every other engine array.
    pub rows: Vec<TableRow>,
    /// Plain-text description shown on the sheet (never rendered as markup).
    #[serde(default)]
    pub description: String,
}

/// How a draw selects a row from `TableEngine.rows`.
///
/// # Examples
///
/// ```
/// use shadowcat::data::engine::table::DrawRule;
///
/// let rule = DrawRule::Formula { notation: "1d20".into() };
/// assert_ne!(rule, DrawRule::Weighted);
/// ```
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/engine/")]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DrawRule {
    /// Uniform over the sum of row weights: the draw rolls `1d<sum>` and the
    /// cumulative-weight row containing the total wins.
    Weighted,
    /// A reference-free dice formula evaluated in Total mode; the row whose
    /// `range` contains the total wins (no row ⇒ a "no matching row" draw).
    Formula {
        /// Dice notation; validated at ingress via
        /// `crate::chat::rolls::validate_table_formula`.
        notation: String,
    },
}

/// One row of a table.
///
/// # Examples
///
/// ```
/// use shadowcat::data::engine::table::TableRow;
///
/// let row = TableRow { weight: 3, range: None, label: "Treasure".into(), results: vec![] };
/// assert_eq!(row.weight, 3);
/// assert!(row.results.is_empty()); // a "nothing happens" row is legal
/// ```
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/engine/")]
#[serde(deny_unknown_fields)]
pub struct TableRow {
    /// ≥ 1. Used by `DrawRule::Weighted`; must still be ≥ 1 under `Formula`.
    pub weight: u32,
    /// Inclusive total range under `DrawRule::Formula`; must be `None` under `Weighted`.
    #[serde(default)]
    pub range: Option<RowRange>,
    /// Short plain-text headline for the chat card (≤ `MAX_ROW_LABEL_CHARS`).
    pub label: String,
    /// What the row yields, in order. May be empty (a "nothing happens" row).
    #[serde(default)]
    pub results: Vec<TableEntry>,
}

/// An inclusive total range (`lo <= hi`) a `DrawRule::Formula` row matches
/// against. `i32`, not `i64`: a row range bounds a dice roll's total, which
/// `TableEngine::validate` already caps well within `i32` via
/// `chat::rolls::MAX_DIE_SIDES`/`MAX_ROLL_DICE`, and an `i64` here forces
/// ts-rs to emit a `bigint` field on a type constructed by client authoring
/// code (`table-docs.ts`'s `buildTableDoc` callers), which `JSON.stringify`
/// (`WsClient.send`) cannot serialize.
///
/// # Examples
///
/// ```
/// use shadowcat::data::engine::table::RowRange;
///
/// let range = RowRange { lo: 1, hi: 10 };
/// assert!(range.lo <= range.hi);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/engine/")]
#[serde(deny_unknown_fields)]
pub struct RowRange {
    /// Inclusive lower bound.
    pub lo: i32,
    /// Inclusive upper bound.
    pub hi: i32,
}

/// One thing a drawn row yields, resolved by `crate::tables::draw::draw_table`
/// at draw time (never at ingress — the `Segment::DocLink` precedent).
///
/// # Examples
///
/// ```
/// use shadowcat::data::engine::table::TableEntry;
///
/// let entry = TableEntry::Text { text: "You find a rusty key.".into() };
/// let TableEntry::Text { text } = &entry else { unreachable!() };
/// assert_eq!(text, "You find a rusty key.");
/// ```
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/engine/")]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TableEntry {
    /// Markdown; sanitized AT DRAW TIME under the world's chat policy.
    Text {
        /// ≤ `MAX_ROW_TEXT_CHARS`.
        text: String,
    },
    /// Resolves to a `Segment::DocLink` (reuses `chat::DocLinkTarget`).
    Doc {
        /// The referenced document/token.
        target: DocLinkTarget,
        /// Display label for the link.
        label: String,
    },
    /// Resolves to a `Segment::Image`; the asset must exist in the drawing
    /// world at draw time.
    Image {
        /// The asset to render.
        asset_id: Uuid,
        /// ≤ `MAX_IMAGE_ALT_CHARS`.
        alt: String,
    },
    /// Nested draw: `count` draws from `table_id`, each its own roll.
    Draw {
        /// The nested table to draw from.
        table_id: Uuid,
        /// `1..=MAX_NESTED_DRAW_COUNT`.
        count: u32,
    },
}

impl TableEngine {
    /// Validates every bullet of the table engine's ingress contract: row/
    /// text/label/description caps, `Draw.count` bounds, and the
    /// `DrawRule`-specific row-shape rules (`Weighted`'s weight-sum bound
    /// reads `chat::rolls::MAX_DIE_SIDES` directly -- never a copied
    /// literal -- since the draw rolls `1d<sum>` through that exact
    /// boundary; `Formula`'s notation and per-row ranges).
    ///
    /// # Examples
    ///
    /// ```
    /// use shadowcat::data::engine::table::{DrawRule, TableEngine, TableRow};
    ///
    /// let table = TableEngine {
    ///     draw: DrawRule::Weighted,
    ///     rows: vec![TableRow { weight: 1, range: None, label: "a".into(), results: vec![] }],
    ///     description: String::new(),
    /// };
    /// assert!(table.validate().is_ok());
    /// ```
    pub fn validate(&self) -> Result<(), String> {
        if self.rows.len() > MAX_TABLE_ROWS {
            return Err(format!(
                "a table may have at most {MAX_TABLE_ROWS} rows, got {}",
                self.rows.len()
            ));
        }
        if self.description.chars().count() > MAX_TABLE_DESCRIPTION_CHARS {
            return Err(format!(
                "description exceeds {MAX_TABLE_DESCRIPTION_CHARS} characters"
            ));
        }
        for row in &self.rows {
            if row.weight < 1 {
                return Err("every row weight must be at least 1".into());
            }
            if row.label.trim().is_empty() {
                return Err("every row label must be non-empty".into());
            }
            if row.label.chars().count() > MAX_ROW_LABEL_CHARS {
                return Err(format!(
                    "row label exceeds {MAX_ROW_LABEL_CHARS} characters"
                ));
            }
            for entry in &row.results {
                match entry {
                    TableEntry::Text { text } => {
                        if text.chars().count() > MAX_ROW_TEXT_CHARS {
                            return Err(format!(
                                "row text exceeds {MAX_ROW_TEXT_CHARS} characters"
                            ));
                        }
                    }
                    TableEntry::Image { alt, .. } => {
                        if alt.chars().count() > MAX_IMAGE_ALT_CHARS {
                            return Err(format!(
                                "image alt text exceeds {MAX_IMAGE_ALT_CHARS} characters"
                            ));
                        }
                    }
                    TableEntry::Draw { count, .. } => {
                        if *count < 1 || *count > MAX_NESTED_DRAW_COUNT {
                            return Err(format!(
                                "nested draw count must be between 1 and {MAX_NESTED_DRAW_COUNT}"
                            ));
                        }
                    }
                    TableEntry::Doc { .. } => {}
                }
            }
        }
        match &self.draw {
            DrawRule::Weighted => {
                for row in &self.rows {
                    if row.range.is_some() {
                        return Err("a Weighted table's rows must not carry a range".into());
                    }
                }
                let sum: u64 = self.rows.iter().map(|r| r.weight as u64).sum();
                if sum > crate::chat::rolls::MAX_DIE_SIDES as u64 {
                    return Err(format!(
                        "a Weighted table's row weights sum to {sum}, more than {} allowed",
                        crate::chat::rolls::MAX_DIE_SIDES
                    ));
                }
            }
            DrawRule::Formula { notation } => {
                crate::chat::rolls::validate_table_formula(notation).map_err(|e| e.to_string())?;
                let mut ranges: Vec<RowRange> = Vec::with_capacity(self.rows.len());
                for row in &self.rows {
                    match row.range {
                        None => {
                            return Err("every row of a Formula table must carry a range".into());
                        }
                        Some(r) => {
                            if r.lo > r.hi {
                                return Err("a row range's lo must not exceed hi".into());
                            }
                            for existing in &ranges {
                                if r.lo <= existing.hi && existing.lo <= r.hi {
                                    return Err("row ranges must not overlap".into());
                                }
                            }
                            ranges.push(r);
                        }
                    }
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;
