//! `draw_table`: server-side resolution of one table draw, recursive through
//! nested draws.

#![deny(missing_docs)]
#![deny(clippy::missing_docs_in_private_items)]

use uuid::Uuid;

use crate::chat::{self, DrawnRow, Segment, TableDrawSegment};
use crate::data::document::WorldCapDefaults;
use crate::data::engine::{DrawRule, TableEngine, TableEntry, TableRow, TABLE_DOC_TYPE};
use crate::data::membership::PermissionContext;
use crate::data::permission::{cap, effective_owner, resolve_access_world};
use crate::data::repository::Repository;

use super::{DrawTableError, MAX_DRAWS_PER_REQUEST, MAX_DRAW_DEPTH};

/// Everything `draw_table` needs across its recursive call tree: repository
/// access, the caller's identity/authz inputs, the world's chat content
/// policy (row text sanitizes under it), the drawing world, the chain of
/// table ids on the CURRENT path (cycle detection), and a per-request budget
/// counting every resolved draw.
pub(crate) struct DrawCtx<'a> {
    /// The document repository.
    pub repo: &'a dyn Repository,
    /// The drawing user's identity/world role.
    pub ctx: &'a PermissionContext,
    /// The world's capability-grant defaults (authz on every table in the chain).
    pub world_defaults: &'a WorldCapDefaults,
    /// The world's resolved chat content policy (row `Text` entries sanitize under it).
    pub policy: &'a chat::ChatContentPolicy,
    /// The world this draw is happening in.
    pub world_id: Uuid,
    /// Table ids on the current recursion path -- a table naming itself,
    /// directly or through a chain of nested draws, is refused
    /// (`DrawTableError::Cycle`) rather than recursing forever.
    pub chain: Vec<Uuid>,
    /// Draws resolved so far across the WHOLE request (top-level plus every
    /// nested draw); `MAX_DRAWS_PER_REQUEST` bounds it regardless of depth.
    pub budget: usize,
    /// Inline markdown image sources collected from every `TableEntry::Text`
    /// entry resolved so far, across the WHOLE request (top-level plus every
    /// nested draw) -- accumulated here so `handle_draw_table` can run ONE
    /// `link_preview::enrich` call over the whole message, mirroring
    /// `chat::body::compose_message`'s own collection.
    pub image_urls: Vec<chat::ImageSource>,
    /// Test-only deterministic seed for every roll `draw_table` performs
    /// (including recursive nested draws, which all read the SAME field) --
    /// never compiles into the release binary. `None` (the only value in
    /// production, and in most tests) draws fresh OS entropy via
    /// `chat::rolls::execute_roll`; `Some(seed)` drives
    /// `chat::rolls::execute_roll_with_seed` instead, for a deterministic
    /// row-selection assertion. Set via `draw_table_with_seed`, never by
    /// hand.
    #[cfg(test)]
    pub seed: Option<u64>,
}

/// Resolves one draw from `table_id`: loads and authorizes the table,
/// deserializes and validates its stored `TableEngine`, rolls, matches a row,
/// and resolves that row's `results` into `content`/`nested` -- recursing
/// into any `TableEntry::Draw` entries at `depth + 1`. `table_id` is pushed
/// onto `cx.chain` for the duration of this call (popped before returning,
/// on every path) so a sibling branch of the SAME request tree is not
/// spuriously refused by a cousin's chain membership.
pub(crate) async fn draw_table(
    cx: &mut DrawCtx<'_>,
    table_id: Uuid,
    depth: usize,
) -> Result<TableDrawSegment, DrawTableError> {
    if cx.budget >= MAX_DRAWS_PER_REQUEST {
        return Err(DrawTableError::TooMany);
    }
    if depth > MAX_DRAW_DEPTH {
        return Err(DrawTableError::TooDeep);
    }
    if cx.chain.contains(&table_id) {
        return Err(DrawTableError::Cycle);
    }
    cx.budget += 1;

    let doc = cx
        .repo
        .get_document(table_id)
        .await
        .map_err(DrawTableError::Data)?;
    let doc = match &doc {
        Some(d)
            if d.doc_type == TABLE_DOC_TYPE
                && crate::data::document::world_of(d) == Some(cx.world_id) =>
        {
            d
        }
        _ => return Err(DrawTableError::NotFound),
    };

    let access = resolve_access_world(
        cx.ctx.user_id,
        cx.ctx.world_role,
        doc,
        &cx.world_defaults.grants_for(TABLE_DOC_TYPE),
        effective_owner(doc, None),
    );
    if !access.has(cap::READ) {
        return Err(DrawTableError::Forbidden);
    }

    let engine: TableEngine = doc
        .engine
        .clone()
        .ok_or_else(|| {
            DrawTableError::Data(crate::data::DataError::BadEngine(
                "table: missing engine body".to_string(),
            ))
        })
        .and_then(|v| {
            serde_json::from_value(v)
                .map_err(|e| DrawTableError::Data(crate::data::DataError::BadEngine(e.to_string())))
        })?;
    if engine.rows.is_empty() {
        return Err(DrawTableError::EmptyTable);
    }

    let notation = match &engine.draw {
        DrawRule::Weighted => {
            let sum: u32 = engine.rows.iter().map(|r| r.weight).sum();
            format!("1d{sum}")
        }
        DrawRule::Formula { notation } => notation.clone(),
    };
    #[cfg(test)]
    let (formula, outcome, spec, raw) = match cx.seed {
        Some(seed) => {
            chat::rolls::execute_roll_with_seed(&notation, super::TABLE_PARSE_CONTEXT, None, seed)
                .map_err(DrawTableError::Roll)?
        }
        None => chat::rolls::execute_roll(&notation, super::TABLE_PARSE_CONTEXT, None)
            .map_err(DrawTableError::Roll)?,
    };
    #[cfg(not(test))]
    let (formula, outcome, spec, raw) =
        chat::rolls::execute_roll(&notation, super::TABLE_PARSE_CONTEXT, None)
            .map_err(DrawTableError::Roll)?;

    let matched = match &engine.draw {
        DrawRule::Weighted => weighted_row(&engine.rows, outcome.total),
        DrawRule::Formula { .. } => ranged_row(&engine.rows, outcome.total),
    };

    // `table_id` is pushed for the duration of resolving the matched row
    // (a `TableEntry::Draw` entry recurses through `draw_table` again, which
    // is where `cx.chain.contains` actually matters). The pop happens
    // UNCONDITIONALLY -- on the error path too -- by capturing the `Result`
    // before popping and propagating only after: an early `?` inside the
    // pushed scope would otherwise leak `table_id` on `cx.chain` past this
    // call's return, corrupting cycle detection for a LATER sibling draw in
    // the same request that happens to reuse the same table id.
    cx.chain.push(table_id);
    let result = match matched {
        None => Ok(None),
        Some(idx) => {
            let row = &engine.rows[idx];
            let label = row.label.clone();
            resolve_row_results(cx, row, depth)
                .await
                .map(|(content, nested)| {
                    Some(DrawnRow {
                        index: idx,
                        label,
                        content,
                        nested,
                    })
                })
        }
    };
    cx.chain.pop();
    let row = result?;

    Ok(TableDrawSegment {
        table_id,
        table_name: doc.name.clone().unwrap_or_default(),
        roll_id: Uuid::new_v4(),
        formula,
        outcome,
        spec: Some(Box::new(spec)),
        raw: Some(Box::new(raw)),
        row,
    })
}

/// Test seam: identical to `draw_table`, but drives every roll it (and any
/// recursive nested draw) performs through `chat::rolls::execute_roll_with_seed`
/// via `cx.seed`, so a row-selection test can assert a deterministic matched
/// row directly instead of relying on a degenerate single-row/weighted
/// fixture to make the outcome predictable. `#[cfg(test)]`: never compiles
/// into the release binary.
#[cfg(test)]
pub(crate) async fn draw_table_with_seed(
    cx: &mut DrawCtx<'_>,
    table_id: Uuid,
    depth: usize,
    seed: u64,
) -> Result<TableDrawSegment, DrawTableError> {
    cx.seed = Some(seed);
    draw_table(cx, table_id, depth).await
}

/// Resolves one matched row's `results` into `(content, nested)`, recursing
/// into any `TableEntry::Draw` at `depth + 1`.
async fn resolve_row_results(
    cx: &mut DrawCtx<'_>,
    row: &TableRow,
    depth: usize,
) -> Result<(Vec<Segment>, Vec<TableDrawSegment>), DrawTableError> {
    let mut content = Vec::new();
    let mut nested = Vec::new();
    for entry in &row.results {
        match entry {
            TableEntry::Text { text } => {
                let sanitized = chat::sanitize(text, cx.policy);
                // Mirrors `chat::body::compose_message`'s own collection: a
                // row's inline markdown image must reach `link_preview::enrich`
                // exactly like a chat message's, or it is silently never
                // asset-ified. Accumulated on `cx` across the whole draw
                // tree (top-level and every nested fan-out) so ONE `enrich`
                // call at `handle_draw_table` covers every row.
                cx.image_urls.extend(sanitized.image_urls);
                content.extend(sanitized.segments);
            }
            TableEntry::Doc { target, label } => {
                content.push(Segment::DocLink {
                    target: target.clone(),
                    label: label.clone(),
                });
            }
            TableEntry::Image { asset_id, alt } => {
                let asset = cx
                    .repo
                    .get_asset(*asset_id)
                    .await
                    .map_err(DrawTableError::Data)?;
                let in_world = matches!(&asset, Some(a) if a.world_id == cx.world_id);
                if !in_world {
                    return Err(DrawTableError::MissingAsset);
                }
                content.push(Segment::Image {
                    asset_id: *asset_id,
                    alt: alt.clone(),
                });
            }
            TableEntry::Draw { table_id, count } => {
                for _ in 0..*count {
                    let seg = Box::pin(draw_table(cx, *table_id, depth + 1)).await?;
                    nested.push(seg);
                }
            }
        }
    }
    Ok((content, nested))
}

/// `Weighted` row selection: the first row whose CUMULATIVE weight (in row
/// order) is `>= total`. `total` is the roll's `1d<sum>` outcome, so it is
/// always `>= 1`; a well-formed table (every weight `>= 1`, `validate`d at
/// ingress) always has a matching row for any total in `1..=sum`.
pub(crate) fn weighted_row(rows: &[TableRow], total: i64) -> Option<usize> {
    let mut cumulative: i64 = 0;
    for (i, row) in rows.iter().enumerate() {
        cumulative += row.weight as i64;
        if cumulative >= total {
            return Some(i);
        }
    }
    None
}

/// `Formula` row selection: the row whose inclusive `range` contains `total`.
/// `None` when no row's range matches (a "no matching row" draw) -- not an
/// error; `TableEngine::validate` only guarantees ranges are non-overlapping,
/// never that they are exhaustive over every possible roll total.
pub(crate) fn ranged_row(rows: &[TableRow], total: i64) -> Option<usize> {
    rows.iter().position(|r| {
        r.range
            .is_some_and(|range| i64::from(range.lo) <= total && total <= i64::from(range.hi))
    })
}

#[cfg(test)]
mod tests;
