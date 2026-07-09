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
  OwnerOrGm }` (the per-property visibility tiers); `PermissionSet.gm_role: Option<DocRole>`
  (`#[serde(default)]`, ts-rs exported) — see Hard Invariants below.
- `src/server/src/data/permission.rs` — the redaction core:
  - `resolve_access(user, world_role, doc) -> Access` (and `resolve_access_world`) builds the
    per-connection `Access { caps, all, see_gm_only, is_owner }`.
  - `effective_role(user, world_role, doc) -> Option<DocRole>` — the shared floor-resolution
    helper both `resolve_access` and `resolve_access_world` call; `None` means the unconditional
    GM/admin short-circuit applies (see `gm_role` invariant below), `Some(role)` means the caller
    must resolve capabilities from that per-document role floor like any other actor.
  - `Access::can_see(v: Visibility)` is the single predicate: `GmOnly => see_gm_only`,
    `OwnerOrGm => see_gm_only || is_owner`, `All => true`.
  - `filter_properties(doc, access)` strips hidden **properties** from an outgoing doc — a
    PROPERTY-visibility gate only (see Hard Invariants: it does NOT decide whole-document
    withholding).
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
- **`filter_properties` is a PROPERTY-visibility gate, NOT a whole-document READ gate.** It only
  strips individual properties whose override is `GmOnly`; it does not withhold, and cannot be
  used to withhold, an entire document. Whole-doc withholding is decided entirely by callers
  checking `access.has(cap::READ)` BEFORE including the op/hit/row at all (see the
  `filter_command`'s `Create`/`Delete`/`Update` branches, `search`'s per-hit filter, and
  `query_documents`/`get_document`) — `filter_properties` runs only after that gate has already
  let the doc through. Any future egress path must follow the same order: check `has(cap::READ)`
  first, then (optionally) `filter_properties` for property redaction. Gating whole-doc delivery
  on `see_gm_only`/GM-ness alone instead of `has(cap::READ)` would leak a `gm_role`-capped
  document (see below) straight past its intended cap.
- **`PermissionSet.gm_role: Option<DocRole>` makes the GM's usual unconditional access
  conditional, per document.** `resolve_access`'s GM branch normally short-circuits to
  `Access { all: true, see_gm_only: true, is_owner: true, caps: {} }` for every `WorldRole::Gm`
  user, before any document-level permission is consulted — correct and load-bearing for every
  pre-existing document type (actors, scenes, secret regions: the GM must always see a secret
  region even though it's `default: DocRole::None`).
  - `gm_role: None` (the field's default via `#[serde(default)]`; every document type that
    predates this field deserializes to `None`) preserves that unconditional short-circuit
    exactly — no behavior change for anything but the new consumer below.
  - `gm_role: Some(role)` caps a GM to the SAME per-document role-floor resolution every other
    actor uses: `effective_role` looks the GM up in `doc.permissions.users` first, falling back to
    `role` (NOT `doc.permissions.default`) only if the GM isn't individually listed. This lets a
    document deny a GM by default (`Some(DocRole::None)`) while still admitting a GM who is
    individually granted a role in `users`, or grant EVERY current GM a role
    (`Some(DocRole::Observer)`) without listing any of them by name — resolved fresh on every call,
    so promotion/demotion to `WorldRole::Gm` takes effect immediately, not a frozen snapshot.
  - `resolve_access_world` deliberately reuses this SAME `effective_role` helper (not
    `doc.permissions.default`) to layer world-level capability grants, so a world-default grant
    for the GM's fallback role applies consistently even when that GM is `gm_role`-capped — the
    original (pre-refactor) sketch would have recomputed the role independently from
    `doc.permissions.default` here and silently diverged for a capped GM; this was a real bug
    caught before it shipped, not a hypothetical.
  - First (and so far only) consumer: `shadowcat-codebase-chat`'s `Audience`→`PermissionSet`
    mapping (`Whisper` sets `Some(DocRole::None)`, `GmOnly` sets `Some(DocRole::Observer)`,
    `Public` leaves it `None`).
  - `see_gm_only` stays `true` for any `WorldRole::Gm` actor regardless of `gm_role` capping —
    only `all`/`caps` (whole-document READ) become floor-gated. A `gm_role`-capped GM therefore
    still passes property-tier (`GmOnly`/`OwnerOrGm`) checks on any document they DO have READ on;
    the cap is purely about whole-document access, not GM-ness for property visibility.
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
- `shadowcat-codebase-chat` — the first (and so far only) consumer of `gm_role`, via its
  `Audience` enum's `PermissionSet` mapping (see that skill's Key files & seams).
