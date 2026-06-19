# M8 — ECS + Scene Rendering: Architecture Design Spec

> Status: **DRAFT for review.** This is a **cross-cutting architecture pass**
> over the whole of M8, not an implementation spec. It fixes the load-bearing
> decisions that span the milestone — the ECS↔document boundary, the scene-entity
> data model, the ECS→WS dispatch contract, the render-layer/filter abstraction,
> the asset surface — and proposes the decomposition into sub-milestones. Each
> sub-milestone (M8a–M8d) gets its **own** brainstorm→spec→plan→execute cycle
> that refines its slice against the decisions recorded here, mirroring the M6/M7
> splits.
>
> **M9 vision pulled forward (as a driver, not as work).** The render-layer
> mask/compositor API is shaped by a real consumer rather than a guessed one, so
> §6 brainstorms M9's vision *architecture* to the depth needed to drive that API.
> M8 implements **none** of M9's vision; M9 keeps its own full spec later. §6 is a
> documented driving-requirements section.

## 1. Goal

M8 turns the M7 stage placeholder (`shadowcat.surface:stage`,
`StagePlaceholder.svelte`) into a playable canvas: a scene with a grid, a
pannable/zoomable camera, placed tokens, measurement/template/drawing tools, and
pings — backed by a server-side ECS hydrated from documents, an asset surface for
backgrounds and token art, and a render-layer API whose first proven consumer is
the M9 vision-mask path.

This is the largest milestone since M6 and is decomposed accordingly (§8).

## 2. Constraints inherited from ARCHITECTURE.md

The following principles are load-bearing for every decision below and are cited
inline by number:

- **#1 Server-authoritative**, clients send intents.
- **#2 Ordered, recoverable realtime** — per-world monotonic seq; gap detection →
  resync from the time-bounded event buffer or a snapshot.
- **#3 Optimistic with rollback** — except vision recomputation, which is
  server-authoritative without client prediction *by design*.
- **#4 Permissions enforced server-side, per recipient** — hidden data stripped
  before transmit, never sent-then-hidden.
- **#5 Documents are the source of truth; runtime state (ECS) is hydrated from
  documents and is ephemeral.**
- **#6 Server is relay + persistence + structural validation only** — no
  third-party code, no semantic/mechanical validation of the `system` body.
- **#7 Public module API is framework-neutral; UI extendable/replaceable, but the
  PixiJS canvas host is engine-owned** — modules draw into it through the
  render-layer API; they cannot replace the renderer.
- **#8 Mutations flow through an undoable boundary** — document and ECS mutations
  are reversible command/event records; one shared substrate for rollback + undo.

## 3. ECS ↔ document boundary & authority

**Decision: the server-side ECS is a derived read-model; documents remain the
sole authority.**

- Token placement/movement and every scene edit are **ordinary document
  mutations** through the existing M5/M6 `Intent → Event` pipeline — so
  optimistic-apply + rollback (M6a) come for free, with no new machinery.
- The server hydrates a **per-world `hecs` world** from Scene + scene-entity
  documents and keeps it in sync as committed document mutations apply. This is
  the "hydration/mutation boundary."
- The ECS exists so **engine-owned** systems (M9 vision raycast, M10
  pathfinding/auras) have a queryable spatial representation. In M8 it has **no
  active system** — `ECS→WS dispatch` is the plumbing (§5), exercised by a thin
  identity consumer and proven against the vision-mask render seam, ready for M9.
- The client renders directly from the Zod document store; **there is no
  client-side ECS.** PixiJS's own container tree is the client scene graph (§7).

**Ephemeral interactions are never documents and never ECS entities.** Pings,
in-progress drag/measurement/template previews are client-local or lightweight
transient broadcasts — they carry no authority and persist nothing.

This satisfies #5 (ECS derived/ephemeral), #8 (one undoable substrate — the
document command/event log), and avoids dual authority.

## 4. Scene-entity document model

**Decision: every scene entity is a top-level `Document`, scoped to its scene by a
new `parent_id` reference, hydrated into the ECS as an entity.**

### 4.1 Why not embedded children

The `documents` table stores each document — envelope + `system` + the entire
`embedded` tree — as a single `json TEXT` blob (full-row rewrite on any change).
Embedding scene contents would mean:

- **Write amplification:** moving one token rewrites the entire scene blob.
- **Quadratic authoring:** drawing N walls = N rewrites of a growing blob = O(N²).
- **Needless coupling:** concurrent edits to different entities collide on one
  document.

Embedding remains correct for what it was built for — **intrinsic, low-cardinality
children that live and die with their parent** (an Actor's items/effects). The
existing `embedded` model, `core:manage_embedded`, and `/embedded/...`
path-scoped capabilities are **retained unchanged** for that use.

### 4.2 The uniform scene-entity pattern

- Token, wall, tile, region, light, sound, drawing, note, template — **all** use
  the identical pattern: a top-level `Document` (same envelope, same opaque
  `system` body, same `PermissionSet`), `doc_type` distinguishing them,
  `parent_id` = the scene's id. Adding a new entity kind later is purely a new
  `doc_type` string — zero new machinery.
- **Schema:** add `parent_id TEXT REFERENCES documents(id) ON DELETE CASCADE`
  (nullable; indexed). Deleting a scene cascades to its entities.
- **Hydration:** load Scene doc → `SELECT … WHERE parent_id = ?` → spawn one ECS
  entity per child. Each child's UUID maps 1:1 to a hecs entity.
- **Engine-owned fields live in the opaque `system` body under reserved engine
  keys** (Scene: grid type/size, dimensions, background asset-UUID; Token: x/y/
  width/height/image asset-UUID). The server keeps validating **structurally
  only** (#6) — it never gains a typed scene/token schema. The **client core +
  ECS hydration layer** own and interpret that shape.
- **Per-entity permissions + per-recipient filtering (#4)** mean M9 token/wall
  visibility is just filtering *which child docs a recipient receives* — a hidden
  wall is never transmitted, not sent-then-stripped.
- **Per-document intent/rollback (#3):** moving token A is an intent on doc A;
  it never touches doc B. Independent optimistic rollback.
- **Resync/snapshot:** a scene snapshot = the Scene doc + its children; child
  mutations are ordinary per-world events in the M4 buffer.

## 5. ECS → WS dispatch contract

**Decision: derived state dispatches on a distinct, coalesced, per-recipient
channel that generalizes the M6c-2 live-search egress pattern.**

`ECS→WS dispatch` is **not** token moves (those are document Events). It is
engine-**derived** state that is not a document — canonically M9's per-player
visibility polygons. Derived state is not authoritative and not reversible, so it
needs its own channel.

- Document mutations stay entirely on the existing `Intent → Event` channel
  (authoritative, reversible, the undo substrate #8). Untouched.
- A **`SceneDerived` frame class**, distinct from document Events: per-connection,
  **leading-edge coalesced** (reusing M6c-2's egress machinery), carrying a
  **computed-at-seq watermark** so clients apply it *after* reaching the document
  events it reflects (preserves #2; a vision mask never precedes the token-move it
  derives from).
- **On resync, derived state is recomputed fresh** from the snapshot, never
  replayed — the M4 event ring buffer stays document-only and the derived channel
  needs no history.
- This is structurally identical to M6c-2 (per-connection subscription,
  recompute-on-change, coalesce, push). M9 vision and M10 auras become "just
  another subscription + system," not new transport.

**M8 scope = the seam, not a real system.** M8 builds the hydration boundary, the
`SceneDerived` frame + coalesced-emit plumbing, and the client route that hands a
derived update to a render layer — proven end-to-end with an **identity/placeholder
derived consumer wired to exactly where M9's vision-mask layer plugs in.** No
vision, no auras ship in M8.

## 6. Render-layer / filter abstraction (M9 vision pulled forward as driver)

**Decision: an ordered, named layer stack — the canvas analog of M7's
`ui.surfaces` — engine-owned and engine-hosted (#7).**

### 6.1 Structure

- **Fixed core z-order:** background → grid → tiles → drawings → walls (GM debug
  viz) → tokens → templates → **vision/fog mask slot** → pings/overlays.
- **Scene-graph reconciler:** subscribes to the document store; maps each
  scene-entity `doc_type` → display objects in its layer (create/update/destroy
  reactively as document Events arrive). The engine owns reconcilers for core
  entity types. PixiJS's container tree *is* the client scene graph (no separate
  client scene model).
- **Mask/filter slot — the spike target:** the vision layer is a mask over the
  layers vision hides. In M8 it is an **identity mask** fed by the §5
  `SceneDerived` placeholder (everything visible); M9 swaps in real per-player
  visibility polygons with **zero structural change**.
- **Camera + grid:** camera = pan/zoom transform on the root container; grid =
  engine-owned square/hex model. Grid coordinate math is engine-owned and shared
  by snapping + measurement/templates. **Input is pointer-event-based from the
  start** (unified mouse/touch/pen, pinch-zoom, drag-pan) — the cross-platform /
  mobile invariant (#10), not a later port.

### 6.2 Public API scope

The render-layer API is public (#7: modules draw through it). Scope decision:

- **Full API now for: layer contracts + camera API + grid API + the
  mask/visibility/render-target *compositor* surface.** These have knowable
  requirements (the layer/camera/grid half from first principles; the
  mask/compositor half driven by the M9 vision path in §6.3), and building them as
  a coherent surface front-loads the hard problems (mask composition across
  layers, render-target ownership) while they are cheap to change.
- **Module-facing *shader-filter* registration is deferred behind a clean, typed
  extension seam.** Even with M9 pulled forward it has **no consumer** — M9's fog
  shader is *engine-owned*, not module-registered; the first real consumer is
  Phase-3 VFX. Building it now would design against a vacuum.
- 0.x / unfrozen, like M7 — hardens through internal use (the freeze gate needs N
  internal systems exercising the API).

### 6.3 M9 vision path (driving requirements; **not** M8 work)

The path is mostly determined by the PLAN + §3–§5:

1. **Walls** are scene-entity documents (`doc_type:"wall"`, §4), hydrated into the
   ECS as vector-segment components.
2. **Server raycasting** (a derived ECS system): raycast each player's controlled
   tokens against the wall set → per-token visibility polygon; **`geo`-union**
   into one per-player polygon. Server-authoritative, exempt from the optimistic
   path (#3).
3. **Dispatch** via the §5 `SceneDerived` channel — per-recipient, coalesced,
   computed-at-seq.
4. **Client render:** polygon → mask over the fog-affected layers, composited with
   persistent fog into a three-state overlay (unexplored = black,
   explored-not-visible = dimmed, visible = clear).

Three decisions that shape the API:

- **D-V1 — Dispatch payload is polygon geometry, not a rasterized mask.** Compact,
  resolution-independent; the client rasterizes and runs the fog compositor.
- **D-V2 — Persistent fog is server-authoritative, per-(scene, player).** Explored
  area is path-dependent (not recomputable from current positions), so it is
  stored and must be consistent across a player's devices → server owns it,
  persisted, accumulated, dispatched. Exact storage shape is M9-internal; the
  **API** only needs: the fog layer composites a persisted *explored* mask + a
  live *visible* mask + *unexplored* default.
- **D-V3 — GM vision mode is a client-side toggle over full data.** The GM is
  authoritative and receives everything; "see all / see as player X" swaps which
  mask the client applies. No extra server path.

The fog compositor (render-target for visibility + accumulated explored texture +
**engine-owned** fog shader → 3-state overlay) is the real consumer that validates
the mask/render-target half of the §6.2 API.

## 7. Assets — upload, serving, identity, replace/delete

**Decision: files on disk, metadata in a dedicated `assets` table, stable UUID
identity decoupled from name, location, *and* bytes.**

The asset UUID reference is **orthogonal** to the linked/unlinked actor split
(§7.1): linked/unlinked governs whether *actor data* is shared; the *image* is
just an asset-UUID value that any number of actors — shared or independent — may
reference. Asset operations key on the UUID and therefore fan out to **every**
referencer by definition.

- **Storage:** raw bytes under a configurable `assets_dir` (new figment config
  field beside `db`, resolved via `std::path` — #10), laid out by world + UUID.
  Dedicated **`assets` table:** `{id (uuid), world_id, storage_key, original_name,
  content_type, byte_size, created_by, created_at, version}`. *(Not SQLite blobs —
  large images stream better off disk; not `documents` — the system body / intents
  / rollback machinery is overkill for an immutable file pointer.)*
- **Identity:** Scene `system.background` and Token `system.image` store the asset
  **UUID**, never a path. Rename/move = metadata change only; the UUID and every
  reference are untouched (stable identity from first upload).
- **Upload:** gated multipart `POST` (GM/owner capability), **size cap + upload
  rate-limit** (the cross-cutting rate-limit introduced with the surface it
  protects), **content-type + magic-byte validation** (images only in M8), stored
  **as-is — no conversion** (Phase 2). Returns the asset record.
- **Serving:** `GET /assets/{uuid}` streams from disk with correct `Content-Type`,
  `Content-Disposition: inline`, and **`ETag` + revalidation** (bytes are mutable
  per UUID, so *not* immutable caching; conditional GET → 304 when unchanged).
  **Read-gated by world membership** + unguessable UUIDs as defense-in-depth.
- **Replace = swap the bytes behind a stable UUID.** Same UUID, new file; update
  `storage_key`/`content_type`/`byte_size`/`version`, delete old file. Every
  referencer — linked tokens (via the shared actor's `image`), unlinked tokens
  (via their copied UUID value), compendium entries, scene backgrounds — resolves
  the new image with **zero reference updates.** GM/owner-gated, magic-byte
  validated.
- **Delete = remove file + record.** Every referencer falls back to the
  placeholder, uniformly. No per-reference cleanup.
- **Live update:** replace/delete change no document (same UUID), so a lightweight
  world-scoped **`AssetChanged{uuid, op: replaced|deleted}`** frame tells holders
  to re-resolve (re-fetch) or placeholder. Source of truth is the record's
  `version`, so reconnect/resync re-reads current metadata regardless.
- **Undo exemption:** asset replace/delete are filesystem+metadata operations, not
  document/ECS mutations — **outside the undo substrate (#8)**; they are hard,
  gated, validated operations. Soft-delete/retention is a Phase-2 concern.
- **Deferred to Phase 2 (explicitly):** conversion, dedup/content-hashing,
  browsing/tags/folders, the asset browser over `Core.search`,
  reference-counting/GC, **per-asset read visibility** (M8 gates by world
  membership; finer gating by the visibility of the referencing document is harder
  because an asset may be referenced by many docs). The UUID identity means none of
  this breaks references later.

### 7.1 Actor link/unlink lifecycle (M10-forward foundation)

M10 implements actor-linking; M8 only ensures the token document shape does not
preclude it. A token always carries its own **scene data** (position, size, image,
vision config) in its `system` body. Its **actor data** attaches in one of two
modes:

- **Linked** — token holds an `actor_id` reference to a top-level Actor document;
  actor data lives there and is shared by every linked token + the directory entry.
- **Unlinked (synthetic)** — the token has an **embedded Actor document**
  (`token.embedded["actor"]`), independent, with `source` provenance to its origin.
  Embedded model in its ideal role: intrinsic, single, lifecycle-bound to the
  token, kept out of the world actor directory.

Transitions (M10 work; the model already permits each):

| Transition | Operation | Required? |
|---|---|---|
| Linked → unlinked | Copy referenced Actor doc into `token.embedded["actor"]`, stamp `source`, clear `actor_id` (= M5 embedded copy independence) | **Must** |
| Unlinked → linked | Set `actor_id`, drop the embedded copy | Optional (good UX) |
| Unlinked → import as compendium actor | Write the embedded actor as a new top-level Actor doc in compendium scope; optionally re-link | Good UX |

**M8 constraint:** keep a token's scene-fields separate from any actor-data, so
M10 can attach either mode without reshaping the token document. In M8's basic
placement, a token is image + position; no actor required.

## 8. Decomposition

Dependency order: **M8a + M8b (parallelizable) → M8c → M8d.** Respects "build what
you cannot build on top of": data/ECS before render, assets before art, render
before tokens/tools, and the vision-mask spike rides with the API it drives.

### M8a — Server scene foundation: data model + ECS boundary + dispatch seam
*(pure server; headless-testable via the M4/M5 test-server, like M6)*
- `parent_id` + `ON DELETE CASCADE` + index; the top-level scene-entity document
  pattern; Scene + entity `doc_type`s; query-based scene hydration (§4).
- Per-world `hecs` world; hydration/mutation boundary; the `SceneDerived` channel
  generalizing M6c-2 egress (per-connection, coalesced, computed-at-seq;
  recompute-on-resync); identity/placeholder derived consumer (§3, §5).
- No deps within M8.

### M8b — Assets: upload, serving, identity, replace/delete
*(server-centric + thin client resolution)*
- `assets` table + `assets_dir`; gated upload; `GET /assets/{uuid}` (ETag);
  replace (byte-swap behind stable UUID) + delete; `AssetChanged` broadcast;
  magic-byte validation; size cap + upload rate-limit. Client: UUID→URL
  resolution, placeholder, re-resolve on `AssetChanged` (§7).
- Independent of M8a (parallelizable); both feed M8c.

### M8c — Client render foundation + render-layer API + vision-mask spike
*(client; first real pixels)*
- PixiJS v8 into `shadowcat.surface:stage` (replaces `StagePlaceholder`); ordered
  layer stack; scene-graph reconciler; camera (pan/zoom, pointer/touch); grid
  (square/hex + coordinate math) (§6.1).
- Render-layer public API (full-B: layer contracts, camera, grid,
  mask/visibility/render-target compositor; shader-filter extension seam) (§6.2).
- Vision-mask spike: identity mask via `SceneDerived` → mask slot → fog compositor
  placeholder, proving the M9 path end-to-end (§6.3).
- Depends on M8a + M8b. **Likely internal split at spec time:** **c-1** render
  foundation, **c-2** render-layer API + vision spike — mirroring M6c.

### M8d — Scene entities + interaction tools
*(client + entities-as-documents)*
- Token placement + movement (document CRUD + optimistic, rendered via the M8c
  reconciler); measurement + template + drawing tools (client ephemerals +
  persisted drawing entities); pings (transient broadcast).
- Depends on M8c (+ M8a). **Possible internal split:** **d-1** tokens, **d-2**
  tools/pings.

## 9. Deferred / explicitly out of scope for M8

- All M9 vision implementation (raycasting, real fog, GM vision mode) — only the
  seam + identity consumer ship; §6.3 is a driver, not work.
- Module-facing shader-filter registration (Phase-3 VFX; seam only in M8).
- Asset conversion, dedup, browsing/tags/folders, asset browser, ref-counting/GC,
  per-asset read visibility (Phase 2).
- Actor link/unlink transitions (M10; M8 only preserves the foundation).
- Post-processing, multi-level maps, portals (per PLAN M8 exclusions).
- Token enrichment (auras/lights/emitters), A* pathfinding, conditions, factions
  (M10).

## 10. Token-set re-audit

Per the M7 spec, the 3-tier SCSS token set is **re-audited when the first themed
canvas overlays land (M8)** — canvas chrome, tool rail active states, ping/measure
colors, fog dim level. This audit is owned by M8c/M8d and recorded against the M7
token system, not treated as a new theme.

## 11. Open items for sub-milestone specs

- **M8a:** exact `SceneDerived` frame shape + correlation/coalescing reuse from
  M6c-2; whether the ECS update runs inline in the commit path or in the egress
  task; the identity consumer's concrete form.
- **M8b:** `assets_dir` default + layout; upload size cap + rate-limit values;
  `AssetChanged` sequencing (in-band world event vs out-of-band); ETag source
  (content hash vs `version`).
- **M8c:** whether render-layer contracts reuse the M7b server-mirrored contract
  registry or are client-only; PixiJS v8 integration into the Svelte `<Surface>`
  host + teardown; the mask/render-target compositor's concrete API.
- **M8d:** drawing-entity document shape; measurement/template math on the grid
  model; ping transient-broadcast frame; token drag → field-path-update
  throttling/coalescing on the optimistic path.
