# shadowcat — Roadmap

What remains to build, in order. MVP-first: Phase 1 (the dogfood alpha) is complete; later phases
add table features, atmosphere, then platform/scale. Each entry lists its goal, key deliverables,
and explicit exclusions. Architecture and rationale live in
[`design/ARCHITECTURE.md`](design/ARCHITECTURE.md). Completed milestones, with their delivery notes,
live in [`HISTORY.md`](HISTORY.md) — this file records nothing that has shipped.

Guiding rule: build what you cannot build on top of. Networking and permissions precede features;
features precede polish; the module API stays 0.x until evidence proves it.

## Phase 1 — MVP (→ dogfood alpha) ✅

Complete: M1–M13, the Phase-1 cleanup burndown, the close-out campaign, Phase 1b replay
redaction, all Bucket C follow-on sub-projects, and the debt-burndown campaign. Full record in
[`HISTORY.md`](HISTORY.md). `docs/OPEN_BUGS.md` is empty; `docs/TODO.md` holds only items blocked
on unbuilt Phase-2+ infrastructure or on external circumstances.

## Phase 2 — Full table

Milestones in build order; each gets its own brainstorm → spec → plan cycle and may decompose
further at design time. Numbering continues from Phase 1.

### M14 · Combat tracker
- **M14a (document/permission substrate) and M14b (combat clock) are DONE** — full delivery
  notes in [`HISTORY.md`](HISTORY.md)'s M14a and M14b entries.
- **M14c — server authority + combat client seams**, six sub-projects in build order (design:
  [`superpowers/specs/2026-08-30-m14c-1-server-formula-engine-design.md`](superpowers/specs/2026-08-30-m14c-1-server-formula-engine-design.md) §1):
  - **M14c-1 — server formula engine + invariant 6** — DONE (see [`HISTORY.md`](HISTORY.md)).
  - **M14c-2 — combat resolution server-side** — DONE (see [`HISTORY.md`](HISTORY.md)).
  - **M14c-3 — world-config authority** — DONE (see [`HISTORY.md`](HISTORY.md)).
  - **M14c-4 — dice references + chat channel** — DONE (see [`HISTORY.md`](HISTORY.md)).
  - **M14c-5 — templates merge server-side**: `MergePull`/`MergePush`/`MergeRevert` intents;
    conflict set returned for human review; `Document.base` under engine-tree validation.
  - **M14c-6 — combat client seams**: `AppContext.combat`, `CoreHooks` first entries +
    delta-derived emission, `Warn` overage label.
- **M14d — tracker module + settings editors**: the default tracker UI, the world/scene combat
  settings editors (including the combat chain editor over `resolve_combat_rules`'s
  engine→system-defaults→world→scene precedence), and end-to-end coverage.
- Depends on: M11 dice, the M10 movement executor, M14a+M14b (done).
- Excludes: automation of attacks/damage resolution (system-owned); audio/VFX cues (Phase 3).

### M15 · Asset pipeline + browser
- Design: [`superpowers/specs/2026-08-30-m15-asset-pipeline-browser-design.md`](superpowers/specs/2026-08-30-m15-asset-pipeline-browser-design.md).
- **M15a (pipeline: server + client core) is DONE** — delivery notes in
  [`HISTORY.md`](HISTORY.md)'s M15a entry.
- **Remaining (M15b — browser module)**: the GM asset browser (`@shadowcat/module-asset-browser`,
  replacing `@shadowcat/module-assets`): folder tree, filter bar (name / regex / tags / kind /
  sort), virtualized thumbnail grid over `?variant=thumb`, preview pane (metadata, tag editor,
  download original, reconvert), multi-select bulk move/tag/delete, drop-zone uploads driven by
  `startChunkedUpload`, `AssetPicker` "browse…" pick mode via `AppContext.assets`; mobile
  reflow. Open design point carried from M15a: **folder move** — `parent_id` is an immutable
  envelope path, so a move needs its own server-authored route (or delete + recreate with asset
  reparenting); decide in the M15b brainstorm. The §5 e2e ("upload a >1-chunk file and find it
  by tag") lands here with the tag UI. Re-review `AppContext` (M14c adds `.combat`), M14d's
  panel/settings-editor conventions, and `AssetPicker`'s consumers before writing the plan.
- Excludes: audio transcode + animated-WebP encoding (Phase 3 audio); FTS-backed asset search
  (M21 — M15 ships SQL substring/tag/folder filters plus a size-limited Rust `regex` filter).

### M16 · Layout + theming completion
- Drag-resize of floating panels where the M12 panel engine does not already provide it;
  multi-window arrangement persistence.
- Multiple themes + user themes over the 3-tier SCSS token system; module styling modes
  (how a module opts into or out of the host theme).
- Excludes: pop-out windows (shipped, M12e).

### M17 · Vision, lighting + movement completion
- Photometric lighting (illumination coupling replacing the flat/edge-projected environment light
  model), darkvision / tremorsense / height.
- **Per-actor/faction movement exemptions** (deferred from M10g): flying/incorporeal ignore
  difficult terrain; needs movement-type tags on actors.
- **Moving light source mid-walk** (residual of the move-stream live clip): a third-party mover
  carrying a light that opens a sightline reveals per sample of that move, not at its stop — the
  observer's vision recomputed per light-carrying sample; cost only on request.
- Depends on: M14 for anything keyed to the turn owner.
- Excludes: Web-Worker optimistic vision (stays server-authoritative by design).

### M18 · Token enrichment
- Aura / light / sound / VFX emitters as token components (sound and VFX emit into the Phase-3
  audio/VFX seams; the component model lands here).
- **Trigger regions** — mechanical/trigger effects on the M10g region primitive: damage, condition
  application, scripted triggers on enter/arrest.
- Token art tooling.
- **Generated token visuals** (deferred from M10i): a parametric compositor that frames existing
  actor art into a token — decorative border + shape-crop mask + background, distinct from the
  dynamic faction ring; a new additive `{kind:"generated"}` on the M10h `RenderVisual` union.
- **Per-token built-in fx** (deferred from M10j): condition-driven tint / desaturate / highlight +
  selection/faction/target highlight via a per-token Pixi `.filters` attach point on the M10h token
  `Container`; the custom shader-filter seam stays Phase 3 VFX.
- **Emote / reaction overlays** (deferred from M10j): transient overlay above the token via a new
  ping-style `emote` aux frame + fading child.
- Depends on: M14 (condition/damage triggers on the combat clock), M17 (light emitters).

### M19 · Tables, notes + chat media
- Rollable tables on the dice engine + document model (weighted rows, nested draws, results to
  chat as roll embeds).
- Rich-text notes on the document model (journal-style documents; reuse the chat sanitizer
  boundary and `Segment::DocLink` for cross-references).
- Chat media linking: images; YouTube as thumbnail + external link only — no IFrame / Data API.

### M20 · Full default module suite
- Every table-facing default module the dogfood alpha lacks, shipped as `src/modules/*` packages
  over the M14–M19 seams (combat tracker UI, asset browser UI, table/notes sheets, emitter
  editors), each independently replaceable.
- The suite is the second internal-module exercise of the API surface toward the Phase-4 freeze
  gate.

### M21 · Search consolidation
- One search milestone, one backend: extend the M6c FTS5 index + live subscriptions to every
  document type the suite introduces (notes, tables, assets by tag) with the same
  visibility-partitioned index — no three-backend split.
- Folds M15's asset search into FTS: M15 deliberately ships `GET /api/assets` as SQL
  substring/tag/folder filters plus a size-limited `regex` name filter (asset names are short;
  a `LIKE` + tag join suffices) — the FTS integration was deferred here, not forgotten.

## Phase 3 — Atmosphere
Audio (mixer, channels, playlists, world-clock sync; then spatial + wall occlusion; transcode via `symphonia` + `opus`/`vorbis_rs`) → VFX (sprite effects, concurrent SFX) → multi-level maps + portals → 3D dice (decide the rendering context up front: reuse the PixiJS WebGL context vs a separate three.js/WebGL + physics layer) → Discord audio-ducking module (OS audio-session monitoring — PipeWire / WASAPI / CoreAudio — never the proprietary Discord Game SDK; requires a dependency / licensing review before integration).

Also parked for Phase 3 from Phase 1: capability Phase 3 — opt-in **sandboxed** server-side
validators running third-party *code* (its own threat model; never the default path). Server-side
evaluation of the engine's own grammars is not this item — it shipped in M14c-1.

## Phase 4 — Platform & scale
**Audit-grade point-in-time replay** — a state-as-of-sequence facility: what a document, its
permissions, its effective owner and the world's capability grants were at any past sequence, so a
replayed event can be redacted against the policy that actually applied. Phase 1b established the
prerequisite: a commit-time redaction snapshot carried on every operation
(`StoredCommand`/`CommandSnapshot`/`OpSnapshot`), against which `filter_command` redacts in
conjunction with current state. This milestone generalizes that into a queryable history. **When it
lands it must become the single producer feeding the existing redaction-context interface — a
second, independently-derived source for the same decision is the fork-a-decision class this
codebase produces most.**
→ Trusted local modding hardening → freeze the module API on evidence (≥1 external module ships without core patches, **or N internal modules across M independent systems exercise the full API surface** — whichever comes first, so the freeze is not deadlocked on an external author who may never appear) → [only if a marketplace is pursued] WASM sandbox + registry + signing / SRI / CSP + package browser → native wrappers (Tauri 2, Capacitor) → hardening + distribution (backup scheduling / automation, world snapshots, WS load + resync stress tests, rate limiting, rustls-acme TLS, Steam OpenID + plain-executable distribution).
## Documentation campaign (cross-phase, runs alongside feature work)

Infrastructure, guides, and Sweeps 1–13 are complete (record in [`HISTORY.md`](HISTORY.md)).
Remaining, in order:
- **Buddy-check convergence — after the last sweep (user directive 2026-07-30).** The completed
  first-pass documentation is buddy-checked (superpowers two-reviewer cross-check debate)
  **crate by crate**: the `shadowcat` server crate, then each TS workspace package. Any problems
  surfaced → fix → re-buddy-check THAT crate; repeat until a buddy-check pass finds no problems.
  Only then does the final ratchet run. Required reading for every implementer and reviewer:
  `docs/design/doc-sweep-truthfulness-rules.md`.
- **Final ratchet — after buddy-check convergence.** Crate-root deny attributes (`lib.rs` is the
  one file in the server crate still without them, reserved for this step), TypeDoc
  `treatValidationWarningsAsErrors: true` (in `typedoc.base.json`), docs lint merged into the main
  `eslint.config.js`.
- **Skills documentation-reference pass — after the final ratchet (user directive 2026-07-30).**
  Every `shadowcat-codebase-*` skill's Pointers section gains its documentation references
  (subsystem rustdoc path under `/api/rust/`, TypeDoc package pages under `/api/ts/`, relevant
  guide/protocol/module portal pages), via the reviewed skill-update gate.

## Cross-cutting (not deferred)
- Data migrations: NONE are built pre-customers — `migrations/0001_init.sql` is a single
  baseline edited in place, and `data/migrate.rs` stays step-free machinery. This line is the
  campaign marker: when a release milestone declares live customer databases, flip this entry
  and start authoring real incremental migrations from that point on.
- Desync-convergence test (M4): maintained throughout.
- Backups: the basic backup + snapshot-restore deliverable (M12.5) satisfies the dogfood gate;
  Phase 4 adds scheduling / automation.
- Rate limiting on WS / upload: introduced with the surfaces it protects, not only at hardening.
- Account model: self-host, admin-provisioned accounts (admin-only `POST`/`GET /api/users`) plus a
  GM-minted world invite the invitee redeems from their own session. Deliberately no
  self-registration / email in v1; the invite exists so a GM never has to name a user, which is
  what keeps username existence secret.
