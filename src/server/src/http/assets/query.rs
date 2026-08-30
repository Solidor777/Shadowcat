//! `GET /api/worlds/{world}/assets` — the filtered, sorted, keyset-paginated
//! asset query (and, with no query parameters at all, the bare listing the
//! pre-pipeline clients read).
#![deny(missing_docs)]
#![deny(clippy::missing_docs_in_private_items)]

use axum::extract::{Path, Query, State};
use axum::response::{IntoResponse, Response};
use axum::Json;
use base64::Engine;
use serde::{Deserialize, Serialize};
use ts_rs::TS;
use uuid::Uuid;

use crate::auth::session::AuthUser;
use crate::data::asset::query::{
    sort_key_of, AssetCursor, AssetFilter, AssetKind, AssetSort, FolderFilter,
};
use crate::data::asset::Asset;
use crate::http::error::AppError;
use crate::http::AppState;

/// Default page size.
pub const DEFAULT_LIMIT: u32 = 200;
/// Largest page a caller may ask for.
pub const MAX_LIMIT: u32 = 500;
/// Longest accepted `name_regex`, in bytes.
pub const MAX_REGEX_BYTES: usize = 256;
/// Compiled-program and lazy-DFA caps for `name_regex` (1 MiB each), so an
/// adversarial pattern cannot consume unbounded memory or compile time.
const REGEX_SIZE_LIMIT: usize = 1 << 20;
/// Upper bound on SQL pages one regex query walks before returning what it
/// has (with a cursor), so a sparse match cannot scan a whole world per call.
const MAX_REGEX_PAGES: usize = 10;
/// Separator between the sort key and the id inside a cursor.
const CURSOR_SEP: char = '\u{1f}';

/// Query string of `GET /api/worlds/{world}/assets`. Every field optional;
/// with none present the route answers the bare `Asset[]` listing.
#[derive(Debug, Default, Deserialize)]
pub struct AssetQuery {
    /// A folder document id, or `root`; absent = whole world.
    pub folder: Option<String>,
    /// With `folder=<id>`: include descendant folders.
    pub recursive: Option<bool>,
    /// Comma-separated tags; every one must be present.
    pub tags: Option<String>,
    /// `image` | `other`.
    pub kind: Option<String>,
    /// Case-insensitive substring of the display name.
    pub name: Option<String>,
    /// Rust-syntax regex over the display name (size-capped).
    pub name_regex: Option<String>,
    /// `name` | `created` | `size` (default `created`).
    pub sort: Option<String>,
    /// Page size, 1..=`MAX_LIMIT` (default `DEFAULT_LIMIT`).
    pub limit: Option<u32>,
    /// Opaque keyset cursor from a previous page's `next_cursor`.
    pub cursor: Option<String>,
}

impl AssetQuery {
    /// Whether no parameter at all was given (the bare-listing contract).
    fn is_bare(&self) -> bool {
        self.folder.is_none()
            && self.recursive.is_none()
            && self.tags.is_none()
            && self.kind.is_none()
            && self.name.is_none()
            && self.name_regex.is_none()
            && self.sort.is_none()
            && self.limit.is_none()
            && self.cursor.is_none()
    }
}

/// One page of query results.
#[derive(Debug, Serialize, TS)]
#[ts(export, export_to = "../../types/generated/")]
pub struct AssetPage {
    /// The page's assets, in query order.
    pub items: Vec<Asset>,
    /// Pass back as `cursor` for the next page; `None` when this is the last.
    pub next_cursor: Option<String>,
}

/// Compile `name_regex` under the size caps; a pattern over
/// `MAX_REGEX_BYTES` or one that fails to compile is a 400.
pub fn compile_regex(pattern: &str) -> Result<regex::Regex, AppError> {
    if pattern.len() > MAX_REGEX_BYTES {
        return Err(AppError::BadRequest(format!(
            "name_regex longer than {MAX_REGEX_BYTES} bytes"
        )));
    }
    regex::RegexBuilder::new(pattern)
        .size_limit(REGEX_SIZE_LIMIT)
        .dfa_size_limit(REGEX_SIZE_LIMIT)
        .build()
        .map_err(|e| AppError::BadRequest(format!("invalid name_regex: {e}")))
}

/// Opaque cursor text for a keyset position.
pub fn encode_cursor(cursor: &AssetCursor) -> String {
    let raw = format!("{}{CURSOR_SEP}{}", cursor.sort_key, cursor.id);
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(raw)
}

/// Inverse of `encode_cursor`; a malformed cursor is a 400.
pub fn decode_cursor(text: &str) -> Result<AssetCursor, AppError> {
    let bad = || AppError::BadRequest("malformed cursor".into());
    let raw = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(text)
        .map_err(|_| bad())?;
    let raw = String::from_utf8(raw).map_err(|_| bad())?;
    let (sort_key, id) = raw.rsplit_once(CURSOR_SEP).ok_or_else(bad)?;
    let id = Uuid::parse_str(id).map_err(|_| bad())?;
    Ok(AssetCursor {
        sort_key: sort_key.to_string(),
        id,
    })
}

/// The parsed, validated form of an `AssetQuery`.
struct Parsed {
    /// SQL-side filter.
    filter: AssetFilter,
    /// Sort key.
    sort: AssetSort,
    /// Page size.
    limit: u32,
    /// Start position.
    after: Option<AssetCursor>,
    /// Post-SQL name matcher.
    regex: Option<regex::Regex>,
}

/// Validate every parameter (each failure is a 400).
fn parse(q: AssetQuery) -> Result<Parsed, AppError> {
    let folder = match q.folder.as_deref() {
        None => None,
        Some("root") => Some(FolderFilter::Root),
        Some(text) => {
            let folder = Uuid::parse_str(text)
                .map_err(|_| AppError::BadRequest("folder must be a uuid or 'root'".into()))?;
            Some(FolderFilter::In {
                folder,
                recursive: q.recursive.unwrap_or(false),
            })
        }
    };
    let tags: Vec<String> = q
        .tags
        .as_deref()
        .unwrap_or("")
        .split(',')
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .map(str::to_string)
        .collect();
    let kind = match q.kind.as_deref() {
        None => None,
        Some("image") => Some(AssetKind::Image),
        Some("other") => Some(AssetKind::Other),
        Some(other) => {
            return Err(AppError::BadRequest(format!("unknown kind '{other}'")));
        }
    };
    let sort = match q.sort.as_deref() {
        None | Some("created") => AssetSort::Created,
        Some("name") => AssetSort::Name,
        Some("size") => AssetSort::Size,
        Some(other) => {
            return Err(AppError::BadRequest(format!("unknown sort '{other}'")));
        }
    };
    let limit = q.limit.unwrap_or(DEFAULT_LIMIT);
    if limit == 0 || limit > MAX_LIMIT {
        return Err(AppError::BadRequest(format!(
            "limit must be 1..={MAX_LIMIT}"
        )));
    }
    let after = q.cursor.as_deref().map(decode_cursor).transpose()?;
    let regex = q.name_regex.as_deref().map(compile_regex).transpose()?;
    Ok(Parsed {
        filter: AssetFilter {
            folder,
            tags,
            kind,
            name: q.name.filter(|n| !n.is_empty()),
        },
        sort,
        limit,
        after,
        regex,
    })
}

/// `GET /api/worlds/{world}/assets` — membership-gated. With no query
/// parameters: the bare `Asset[]` listing (the contract every pre-query
/// consumer reads). With any parameter: an `AssetPage`. The SQL filters
/// narrow first; `name_regex` is then applied in Rust to the rows the SQL
/// selected, walking further SQL pages (up to `MAX_REGEX_PAGES`) until the
/// page fills — `next_cursor` always marks the last row EXAMINED, so a
/// caller following cursors never skips a row.
pub async fn list(
    State(state): State<AppState>,
    user: AuthUser,
    Path(world): Path<Uuid>,
    Query(q): Query<AssetQuery>,
) -> Result<Response, AppError> {
    state
        .repo
        .permission_context(world, user.id, user.role)
        .await?;
    if q.is_bare() {
        return Ok(Json(state.repo.list_assets_by_world(world).await?).into_response());
    }
    let Parsed {
        filter,
        sort,
        limit,
        after,
        regex,
    } = parse(q)?;

    let mut items: Vec<Asset> = Vec::new();
    let mut after = after;
    let mut next_cursor = None;
    for _ in 0..MAX_REGEX_PAGES {
        // One extra row tells us whether anything follows the page.
        let mut rows = state
            .repo
            .query_assets(world, &filter, sort, after.as_ref(), limit + 1)
            .await?;
        let has_more = rows.len() as u32 > limit;
        rows.truncate(limit as usize);
        let last_examined = rows.last().map(|a| AssetCursor {
            sort_key: sort_key_of(a, sort),
            id: a.id,
        });
        match &regex {
            None => {
                items = rows;
                next_cursor = if has_more { last_examined } else { None };
                break;
            }
            Some(re) => {
                for a in rows {
                    if items.len() as u32 >= limit {
                        break;
                    }
                    if re.is_match(&a.original_name) {
                        items.push(a);
                    }
                }
                if !has_more {
                    next_cursor = None;
                    break;
                }
                next_cursor = last_examined.clone();
                if items.len() as u32 >= limit {
                    break;
                }
                after = last_examined;
            }
        }
    }
    Ok(Json(AssetPage {
        items,
        next_cursor: next_cursor.as_ref().map(encode_cursor),
    })
    .into_response())
}

#[cfg(test)]
mod tests;
