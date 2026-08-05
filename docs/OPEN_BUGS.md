# Open Bugs

Currently open, confirmed-real defects. Deferrals belong in `TODO.md`, not here.

- **`property_overrides` keys are not restricted to the four egress-special-cased fields; a
  self-targeting `/permissions` key silently substitutes the fail-closed default permissions
  object for a redacted viewer.** `validate_property_overrides`
  (`src/server/src/data/validation.rs:332-337`, gated on both Create and Update ingress —
  `src/server/src/data/sqlite.rs:2041,2409`) checks only that a key is a well-formed non-empty JSON
  pointer (starts with `/`, no trailing `/`) — nothing restricts which top-level `Document` field it
  names. `filter_properties` (`src/server/src/data/permission.rs:701-756`) special-cases only
  `/system`, `/engine`, `/name`, `/base` (nulled in place, lines 746-751); any other hidden
  `property_overrides` pointer — including one naming `/permissions` or `/permissions/...` itself —
  falls through to the generic `strip_pointer` (`src/server/src/data/permission.rs:959-978`), which
  does a plain `Map::remove` on whatever top-level key the pointer names. For a single-token
  `/permissions` pointer, this removes the entire `permissions` object from the serialized document
  before `serde_json::from_value` re-deserializes it. Because `Document.permissions` carries
  `#[serde(default)]` (`src/server/src/data/document.rs:479-480`) and `PermissionSet` derives
  `Default` with `default: DocRole::None` (`src/server/src/data/document.rs:415,418-420` —
  fail-closed), re-deserialization does not panic; it silently substitutes the fail-closed default
  `PermissionSet` for the real one.
  - **A NESTED `/permissions/...` key is worse: it PANICS the request.** `PermissionSet`'s `default`,
    `users` and `property_overrides` fields carry **no** `#[serde(default)]` — only `capabilities`
    and `gm_role` do (`src/server/src/data/document.rs:417-439`). So an override naming
    `/permissions/default`, `/permissions/users` or `/permissions/property_overrides` strips a
    REQUIRED field while the enclosing `permissions` object survives, leaving a value that cannot
    deserialize as `PermissionSet` — and the tail of `filter_properties` is
    `serde_json::from_value(whole).expect("filtered document deserializes")`
    (`src/server/src/data/permission.rs:755`). The `expect` is not a cold-path assertion:
    `filter_properties` runs per-recipient on the WS broadcast egress path (`filter_command`,
    `src/server/src/data/permission.rs:833-851`), on FTS search hits
    (`src/server/src/data/sqlite.rs:2785`), and on the HTTP get-document routes
    (`src/server/src/http/routes.rs:975,1026`). Any recipient who cannot see the offending tier
    crashes the request handling their read — i.e. a denial-of-service against every such reader of
    that document, authorable by one holder of `cap::EDIT_PERMISSIONS`.
  - **Reachability:** requires `cap::EDIT_PERMISSIONS` on the document's `doc_type` — every GM has
    this; a non-GM could hold it only via an explicit `by_role`/`users` capability grant. No UI path
    in this codebase constructs a `property_overrides` key outside `/system`, `/engine`, `/name`,
    `/base` today; a raw protocol Update/Create message is not otherwise blocked from doing so.
  - **Effect:** a viewer who cannot see the offending override tier receives a document whose
    `permissions` field is the fail-closed default rather than the real one — a data-integrity
    defect (e.g. a client computing `isHidden`-style checks from the received `permissions` would
    misreport). **Not an authorization bypass**: write authorization always re-resolves server-side
    against the stored row, never against a redacted client-facing copy, and the substituted default
    is strictly more restrictive than the real value, never less.
  - **Fix shape DECIDED (user-directed, 2026-08-01): redaction operates on content bands, never on
    the envelope.** `Document`'s fields split into four CONTENT bands — `name`, `engine`, `system`,
    `base`, already the exact set `filter_properties` special-cases
    (`src/server/src/data/permission.rs:747`) and the exact set `required_cap_for_path` maps to
    `cap::WRITE_FIELDS` — and the STRUCTURAL remainder (`id`, `scope`, `doc_type`,
    `schema_version`, `source`, `owner`, `permissions`, `parent_id`, `embedded`, `created_at`,
    `updated_at`), which nothing may redact. Three parts:
    1. **One shared classifier** in `permission.rs` — `REDACTABLE_BANDS: [&str; 4]` plus
       `redaction_target(pointer) -> Option<RedactionTarget>` returning `Band` (null in place —
       today's four-arm match) or `Within` (`strip_pointer`, now provably landing inside an
       untyped `serde_json::Value` or an `Option`, never a required field). Ingress and egress
       currently duplicate the judgement of what a pointer means; this panic is what that fork
       looks like when it drifts, so the two paths must read ONE symbol, not agree by inspection.
       `collect_hidden` uses it too, so the change-delta path cannot diverge from whole-document
       egress.
    2. **Ingress rejects an unclassifiable pointer.** `validate_property_overrides`
       (`src/server/src/data/validation.rs:332`) keeps its well-formedness checks and adds the
       classifier, at both existing call sites (`src/server/src/data/sqlite.rs:2041` Create,
       `:2409` Update). `/permissions`, `/permissions/default`, `/owner`, `/id`,
       `/embedded/items/0` all become `DataError::BadPath`.
    3. **`filter_properties` returns `Result<Document, RedactionError>`**, deleting both
       `.expect()`s. Callers fail CLOSED: `filter_command` drops delivery to that recipient;
       `get_document`/`search` error rather than ship a half-redacted document. The whitelist
       alone closes the reachable bug; the `Result` covers what a whitelist structurally cannot —
       a band added to `Document` without updating the classifier, or a future nested pointer
       landing in a required field. A secrecy gate that meets an input it cannot classify must
       withhold, never panic and never guess (same posture as the fog invariant).
    **No migration and no compatibility shim**: no worlds or users exist yet, and every
    `property_overrides` key constructed anywhere in the repo — server, client, and tests — is
    already inside the whitelist (`/name`, `/engine`, `/engine/vision`, `/system/*`; verified by
    repo-wide grep 2026-08-01).
    **Tests required:** per-pointer ingress rejection for each envelope field; acceptance for the
    four bands and their nested forms; a regression test that the exact `/permissions/default`
    input returns `BadPath` instead of panicking; and a mutation check that removing a band from
    `REDACTABLE_BANDS` fails the suite — a parity test that passes because both paths are wrong
    the same way proves nothing.
    **Scheduling:** own branch, after Sweep 11 merges — a server fix does not belong batched into
    a docs sweep.

- **`makeTemplateTool`'s near-zero-drag fallback effectively never fires in a snapping scene, so a
  plain click places an arbitrarily-sized template instead of the intended one-cell default.**
  `onPointerDown` snaps the anchor (`src/modules/scene-tools/src/controller.svelte.ts:1092`,
  `anchor = ctx.scene.snap(p)`) but `onPointerMove`/`onPointerUp` pass the RAW pointer point to
  `sizeDir` (`:1098`, `:1107`). `sizeDir`'s fallback is `if (d < 1) return { size: cell,
  direction: 0 }` (`:1061`), with `d` the distance between those two points — so it fires only
  when the click lands within one scene unit of the snapped anchor. `Grid.snap` returns the cell
  CENTER (`src/client/render/src/grid.ts:61-66`), so an ordinary click sits some arbitrary
  distance from the anchor, bounded by the cell's half-diagonal; it takes the normal branch and
  yields `size = d`, an arbitrary template rather than the intended one-cell default. The
  fallback is reachable only by a click landing almost exactly on the snap point.
  - **This is a defect, not a missing feature.** The `d < 1` branch exists precisely to turn a
    click into a real default-sized template rather than a degenerate one; it was written assuming
    both points share a coordinate frame. Mixing a snapped anchor with a raw pointer defeats its
    own stated purpose.
  - **Sibling divergence:** `makeTemplateTool` is the only one of the four authoring tools with no
    extent guard on persist. `makeDrawTool` gates on `hasExtent` (`:990`), `makeWallTool` on a
    `>= 1` length check (`:299`), `makeRegionTool` at `:363-365`.
  - **Reachability/impact:** GM-only (the `template` tool is `gmOnly`) and non-destructive — no
    data loss and no authz effect. Impact is nonetheless persistent: no client code anywhere
    constructs a `delete` operation (repo-wide, `op: "delete"` outside tests exists only in the
    receive-side wire schema and store applier), so no scene-entity delete UI exists to remove
    the junk template. It persists until such a UI ships or a raw protocol Delete is sent. Cost
    is accumulating scene and event-log clutter plus a confusing authoring experience.
  - **Fix shape:** make the two `sizeDir` call sites agree on a frame — either snap the pointer
    point alongside the anchor, or compare the raw pointer against the raw pointer-down point.
    Then add the extent guard its three siblings already carry. Belongs on the runtime follow-up
    branch with the `property_overrides` fix above; found by the Sweep 11 whole-branch review,
    which is comment-only and cannot carry a behavior change.
