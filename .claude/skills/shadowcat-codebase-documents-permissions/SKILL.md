---
name: shadowcat-codebase-documents-permissions
description: "Use when touching Shadowcat documents, permissions, redaction, visibility tiers (all / gm_only / owner_or_gm), per-recipient broadcast filtering, the search index, or the client wire/Zod types. Covers src/server/src/data and its src/client/core wire mirror. Invoke shadowcat-codebase-core first."
---

# Shadowcat — Documents & Permissions

Orientation for the document data model and the server-side, per-recipient redaction layer.
Server is the source of truth; the client only mirrors the wire shape.

## Purpose

A document is a typed envelope (id, type, owner, permissions, `schema_version`) + an opaque
`system` JSONB body the engine never interprets semantically. Permissions are enforced
server-side **per recipient**: hidden fields are stripped before transmission, never
sent-then-hidden. This subsystem also owns the visibility-partitioned full-text index.

## Key files & seams

- `src/server/src/data/document.rs` — the `Document` envelope; `enum Visibility { All, GmOnly,
  OwnerOrGm }` (the per-property visibility tiers).
- `src/server/src/data/permission.rs` — the redaction core:
  - `resolve_access(user, world_role, doc) -> Access` (and `resolve_access_world`) builds the
    per-connection `Access { is_owner, see_gm_only, … }`.
  - `Access::can_see(v: Visibility)` is the single predicate: `GmOnly => see_gm_only`,
    `OwnerOrGm => see_gm_only || is_owner`, `All => true`.
  - `filter_properties(doc, access)` strips hidden properties from an outgoing doc.
  - `redact_change(change, gm_only)` redacts field-level change events on the broadcast path.
- `src/server/src/data/search.rs` — `index_content` (full) vs `index_content_public` (redacted):
  the index is **partitioned by visibility**, not redacted after the fact.
- `src/server/src/data/{repository.rs,validation.rs}` — `Repository` trait (storage seam; SQLite today, Postgres-capable later) +
  structural validation (size caps, field-path validity, `deny_unknown_fields`).
- `src/client/core/src/wire.ts` — Zod mirror: `VisibilitySchema = z.enum(["all","gm_only",
  "owner_or_gm"])`, `property_overrides`. ts-rs generates the TS types from the Rust source.

## Hard invariants

- **Redaction is fail-closed and owner-aware.** `can_see` is the one chokepoint across every
  egress path; a partial-visibility tier (`OwnerOrGm`) uses a distinct flag — never overload the
  GM see-all boolean, or you leak `GmOnly` to owners [[ownerorgm-tier-no-widen]].
- **The search index is visibility-partitioned.** Redacting only the returned doc leaks GM-only
  text via snippet/match/score — index public and full content separately
  [[search-index-must-be-visibility-partitioned]].
- **Path-prefix authz covers ancestor (subtree-replacing) writes AND whole-doc Create**, not just
  descendant field updates [[path-prefix-authz-covers-ancestor-and-create]].
- **Check-then-act across two queries needs one transaction** — TOCTOU-racy even at
  `max_connections(1)` [[two-query-guard-needs-tx]].
- **`INSERT … ON CONFLICT(id)` on a mutated id duplicates rather than moves** the row
  [[upsert-on-conflict-duplicates-not-moves]].

## Gotchas

- **Wire types are generated** — change the Rust `Visibility`/`Document`, regenerate ts-rs, then
  mirror in the Zod schema (a drift guard enforces parity). Never hand-edit `src/types/generated`.
- **Embedded copies need a deep clone** — `{...doc}` aliases nested `system`/`permissions`/
  `embedded` until the wire round-trip; use `structuredClone` at construction
  [[embedded-copy-needs-deep-clone]].
- **Test harness:** `doc(perms, system)` not `doc(id)`; an `owner_id` is a FK, so a test owner
  must be a real `create_user`, not a synthetic `Uuid` [[server-test-doc-helper-and-owner-fk]].

## Pointers

- Rationale: `docs/design/M2-data-foundation.md`; invariants in `docs/design/ARCHITECTURE.md`
  §2 invariant 4 (per-recipient permissions) + §6 (data model).
- Relationships: `graphify query "document permissions redaction filter_properties can_see"`,
  `graphify path "permission.rs" "search.rs"`.
- Deferred merge model: [[document-inheritance-merge-model]].
