# M15b — Asset browser + generic document Move — Design

Status: approved design, pending implementation plan.
Parent design: [`2026-08-30-m15-asset-pipeline-browser-design.md`](2026-08-30-m15-asset-pipeline-browser-design.md)
(§4 is elaborated here; §§1–3/5 shipped in M15a). This document settles the open points that
design carried into the M15b brainstorm and supersedes its §4 where they differ.

## Decisions (settled in brainstorm)

| Question | Decision |
|---|---|
| Folder move | **Generic document Move** — a first-class, fully generic server operation on the document layer, not an asset-folder-only route and not delete+recreate. |
| Move rollout | Fully generic from day one: any document is movable wherever *Create* with that parent would be valid. No per-type opt-in list. |
| Move authz | **GM-only.** Sidesteps the player-token/movement-gate interaction entirely; may be widened later via a capability without a wire change (no-widen default). |
| Pick-mode seam | `AppContext.pickAsset()` (the parent design's "`AppContext.assets`" is stale — that name is the `AssetResolver`). Presented as a **modal overlay**, not a floating panel: promise lifetime must not entangle persisted panel layouts. |
| Pick arity + consumers | Multi-capable from day one (`multiple` → ordered `string[]`). Converts **all three** picking surfaces: scene-tools `AssetPicker` (browse affordance) and actors `VisualKindEditor`'s face / frames / sheet ad-hoc grids. |
| Sequencing | M15b lands before M14d; **M15b sets the panel conventions M14d follows**, inverting the parent design's re-review assumption (M14c-2+ / M14d have not landed). |

Excluded (unchanged from parent design): audio, FTS search (M21), per-user asset areas.
Additionally excluded here: moving *embedded* children (Move rewrites `parent_id` only, never
restructures `embedded`); any non-GM Move path.

## §A — Generic document Move (server)

The only server work in M15b.

- **Wire.** A new `Move { id, parent_id: Option<Uuid> }` variant on the document `Operation`
  enum, riding the ordinary intent path (`apply_intent`) like Create/Update/Delete. Both wire
  mirrors (`ts-rs` + Zod) extend.
- **Authz.** GM-only, enforced at the intent chokepoint. A non-GM Move is rejected like any
  other authz failure (no partial application within the batch, per existing intent semantics).
- **Validity = Create-validity.** A Move is legal iff creating the document with the target
  `parent_id` would be: the same placement rules (`combat` never parented; `combatant` /
  `combat-history` require a parent combat; `asset_folder` parent must be an `asset_folder` in
  the same scope), parent existence, same world/scope. `parent_id: None` moves to top level and
  is legal wherever Create allows a parentless document of that type. No new per-type policy is
  invented for Move; the Create chokepoint checks are routed through, not duplicated.
- **Cycle check.** The `asset_folder` ancestor-cycle walk that M15a found unreachable (Create
  cannot form a cycle; parent was immutable) becomes reachable and is written at the Move
  chokepoint: walk the target parent's ancestor chain; reject if it contains the moved document
  (self-parent included). The walk is bounded by tree depth and runs inside the same transaction
  as the write.
- **Targets.** Top-level documents only. A Move addressed at an embedded child is rejected.
  A Move to the document's current parent is a no-op: succeeds, writes nothing, bumps nothing,
  broadcasts nothing. A real move bumps `updated_at` (not `schema_version`, not any version
  counter).
- **Per-type post-move hooks**, at the same single chokepoint (the pattern
  `delete_document_tx`'s reparenting hook established):
  - `asset_folder` → recompute derived tags for every asset in the moved subtree (M15a's
    folder-Update recompute machinery, reused).
  - `token` / `region` → scene re-derivation for **both** the source and destination scenes
    (vision, footprints, fog, navmesh dirtying — whatever the existing scene-dirty path
    invalidates on a placement-affecting Update).
  - Other types: no hook.
- **Broadcast.** A Move emits on the document stream through the same per-recipient egress
  filter every update passes. If a recipient's effective READ of the document changes with its
  ancestor chain, the existing READ-transition machinery applies (lose READ → envelope-only
  Delete; gain READ → Create). Plan-time verification item: if effective READ is in fact
  parent-independent in the current model, no transitions can arise from Move and a test pins
  that; the routing through the per-recipient filter is required either way.
- **Resync/export.** No changes: `parent_id` already round-trips in snapshots and bundles;
  sequence numbers cover Move like any op.

## §B — Client core + `pickAsset` seam

- **Move on the client.** Zod mirror; optimistic apply (store rewrites `parent_id`), rollback
  from the confirmed mirror on rejection — the existing correlated-intent machinery, one new op.
- **`AppContext.pickAsset`** (ui-kit `AppContext`, hosted by the shell):

  ```ts
  pickAsset(opts?: { kind?: "image" | "other"; tags?: string[] }): Promise<string | null>;
  pickAsset(opts: { multiple: true; kind?: "image" | "other"; tags?: string[] }):
    Promise<string[] | null>;
  ```

  Resolves with the picked uuid(s) (`multiple`: in pick order) or `null` on cancel. One pick
  active at a time: a new request resolves the previous one `null` and replaces it. The shell
  hosts the modal overlay component and wires the context method; the modal embeds the shared
  `AssetBrowser` component (§C) in pick mode. Pick mode is usable by any world member (players
  can already `listAssets`; folder documents follow ordinary document visibility) — only
  mutation affordances are GM-gated, and they are hidden in pick mode regardless.

## §C — Browser module (`@shadowcat/module-asset-browser`)

Replaces `@shadowcat/module-assets`: the old package, its panel contribution, and its portal
page are retired in this milestone, not kept alongside.

- **Panel.** One GM-only panel contribution (not reachable or visible for non-GM; whether the
  gate sits at contribution registration or component render is an implementation choice —
  requirement: players never see it). This panel is the convention-setter M14d follows.
- **Layout.** Folder tree (left) · filter bar (top) · virtualized thumbnail grid · preview pane
  (selection). Mobile: tree collapses to a drawer, grid reflows to two columns, touch targets
  ≥ 44px. All strings through `t`/locale files.
- **Folder tree.** Reactive over the store's `asset_folder` documents. Create / rename (ordinary
  document ops), drag-to-reparent (**Move op**), delete via a dialog offering *reparent assets*
  (default; `?assets=reparent`) or *purge* (`?assets=delete`), with an explicit confirmation for
  purge. Dropping selected assets on a folder node issues `PATCH` (single) / `bulk` (multi).
- **Filter bar.** Name substring with a regex toggle (`name` / `name_regex`), tag chips
  (all-of), kind, sort — mapped 1:1 onto `queryAssets` params, debounced; keyset pagination via
  the returned cursor (load-more on scroll).
- **Grid.** `?variant=thumb` tiles; virtualization reuses ChatPanel's measured-window pattern
  (real-measurement `$state` + derived window), adapted to fixed-height grid rows — no new
  dependency. Multi-select (click / ctrl / shift) feeding bulk move/tag/delete.
- **Preview pane.** `?variant=preview` image; metadata (dimensions, sizes, content types,
  `conversion_note`, `original_retained`); explicit-tag editor (derived tags shown read-only);
  rename; download original (GM, when retained); reconvert (GM); delete.
- **Uploads.** Drop-zone on grid and folder nodes plus a file-input fallback; a queue with
  per-file progress/error/retry driven by `startChunkedUpload` (single-shot under threshold is
  internal to it); uploads target the drop folder via placement opts.
- **Listing freshness.** `onAssetChanged` (`created`/`moved` invalidate listings) +
  `AssetResolver.onListingInvalidated`, with `reconcile` on refetch — the parent design's repair
  path, unchanged.
- **File discipline.** The module splits into focused components (tree / filter bar / grid /
  preview / upload queue / pick-confirm chrome) under the file-size gate; the shared
  `AssetBrowser` shell composes them and carries a `mode: "manage" | "pick"` prop (pick: hides
  mutations, shows a selection-confirm bar honoring `multiple`).

## §D — Consumers

- **scene-tools `AssetPicker`** keeps its compact inline grid and gains a "browse…" affordance
  calling `pickAsset({ kind: "image" })`; the result sets `controller.selectedAsset`.
- **actors `VisualKindEditor`**: its three ad-hoc `listAssets` grids convert — face art and
  sheet asset to single `pickAsset({ kind: "image" })`, animation frames to
  `pickAsset({ multiple: true, kind: "image" })` (ordered) — deleting the duplicated picker UI.

## §E — Testing

- **Server (Move).** Validity matrix per parented type (legal target, illegal parent type,
  cross-world, missing parent, embedded-target rejection, `None`-target rules); folder cycle
  rejection (direct + deep + self); non-GM rejection; no-op short-circuit; derived-tag
  recompute on folder move (subtree assets' folder-segment tags change); scene re-derivation on
  token/region move (both scenes); READ-transition behavior pinned per §A's verification item;
  bundle/resync round-trip of a moved tree.
- **Client.** Filter bar → `queryAssets` param mapping; pagination; multi-select → bulk calls;
  pick mode resolve/cancel/`multiple`/concurrent-request semantics; optimistic Move rollback on
  rejection; upload queue progress/error under mocked `startChunkedUpload`; `VisualKindEditor`
  conversions keep their completeness rules.
- **e2e.** The parent design's §5 scenario lands here: upload a >1-chunk file through the
  browser and find it by tag; extended with a folder drag-move reflected in the tree.
- `pnpm -r test` throughout — the shared wire type changes.

## §F — Documentation + close-out

- `ARCHITECTURE.md`: Move joins the document-mutation invariants (GM-only; Create-validity
  rule; single chokepoint with per-type hooks).
- Docs site: the assets module page is replaced by an asset-browser page; wire-protocol page
  gains the Move op.
- Skill-update gate: `shadowcat-codebase-assets`, `-documents-permissions` (Move op + envelope
  immutability now "immutable via field-path Update; rewritten only by Move"), `-client-shell`
  (`pickAsset` seam), `-actors-tokens` (VisualKindEditor picking), reviewed per the gate.
- `PLAN.md` M15 entry closes into `HISTORY.md`; the M14d entry gains a note that panel
  conventions were set by M15b.
