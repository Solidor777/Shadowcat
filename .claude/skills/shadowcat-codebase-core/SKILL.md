---
name: shadowcat-codebase-core
description: "Use for any Shadowcat task: project architecture overview, how to build/test/lint, code & cross-platform conventions, the module/contribution model, and which knowledge layer (this skill / graphify / docs/design / memory) answers which question. The always-relevant base skill — invoke it first, then the matching shadowcat-codebase-<subsystem> skill."
---

# Shadowcat — Codebase Core

Orientation + index for the whole repo. This is the base every agent reads first; it points
INTO graphify (relationships), `docs/design/` (rationale), and memory (lessons) rather than
restating them.

## Purpose

Shadowcat is a self-hostable, fully-moddable open-source virtual tabletop shipped as **one
native executable**: a Rust (Cargo) server holds authoritative state + persistence +
networking, and a Svelte 5 (Runes) browser client + PixiJS canvas is built by Vite into `dist/`
and **embedded into the binary** (`rust-embed`). SCSS for styles. Source lives strictly under
`src/`; build output in `dist/` (client) and `target/` (server).

## Key files & seams

- `src/server/` — Rust workspace (authoritative). Subsystems: `data/` (documents, permissions,
  search, assets), `ws/` (realtime), `http/`, `auth/`, `scene/` (ECS, vision, fog).
- `src/client/{core,render,shell,ui-kit}` — `core` = framework-neutral headless TS (store, wire,
  module loader, hook bus; **no Svelte in its dep closure**); `render` = engine-owned PixiJS
  layer; `shell` = `@shadowcat/shell` app bootstrap/routing/session (builds `dist/`); `ui-kit` =
  `@shadowcat/ui-kit` Svelte runtime (AppContext, `<Surface>` host, i18n adapter).
- `src/modules/*` — first-party contribution packages (`actors`, `assets`, `core-ui`, `entry`,
  `factions`, `scene-tools`, `settings`, `stage`, `statusbar`, `topbar`). In-game UI is
  UI-as-modules; elements talk ONLY through seams (`provides`/`requires` contracts,
  `ContributionRegistry`, `<Surface>`, AppContext, render-layer API) — never importing each other.
- `src/types/generated` — **ts-rs output**: Rust types → TS. Edit the Rust source, regenerate;
  never hand-edit the `.ts`.

pnpm workspace = `src/types`, `src/client/*`, `src/modules/*`.

## Hard invariants

The full list is `docs/design/ARCHITECTURE.md` §2 (10 invariants) — load-bearing, treat as the
source of truth. The ones agents break most:

- **Server-authoritative, permissions per-recipient.** Client sends intents; server validates,
  applies, broadcasts. Hidden fields are stripped **before** transmission, never sent-then-hidden
  (ARCHITECTURE §2 invariant 4). See `shadowcat-codebase-documents-permissions`.
- **Optimistic with rollback.** Documents are source of truth; ECS/runtime is derived & ephemeral.
- **NEVER FORK A DECISION ACROSS TWO PATHS — the defect class this codebase produces most.**
  Whenever two code paths are *documented* to agree on something, they eventually disagree on an
  input nobody thought to check, and the disagreement is a security defect rather than a bug. Six
  instances found in one branch (the 2026-07-22 hex-grid campaign), across four subsystems:
  | Forked on | Where | Consequence |
  |---|---|---|
  | Cell indexing | `ws/room.rs`, `navmesh.rs` | square indices tested against a hex-axial mask |
  | Contract completeness in a SHARED primitive (not a fork — included because it is the same *consequence* from the opposite cause) | `HexGrid::line_traversal` | a thin line, not a supercover — ~55% of segments omitted a crossed hex the gate then never checked; see `scene-rendering`'s "a fixed-count cube lerp is a THIN LINE" gotcha |
  | Input admissibility | `Room::publish` vs `gate_walk` | one bounded coordinate magnitude, the other did not |
  | **Scene identity** | `MoveRequest` vs `Room::publish` | one took the scene from the client, the other derived it from the token ⇒ total movement-gate bypass |
  | **`remove` semantics** | `SceneEcs::apply_op` vs `apply_intent` | ECS ignored `FieldChange.remove` while the DB honoured it ⇒ vision widened where write authz refused |
  | Fail-open defaults | `execute_move` vs `publish` vs `pathfind` | a `unwrap_or(100.0)` cell size removed from ONE gate, left in the other two — created by the commit that fixed the row above. **Now removed from all three gates AND all six non-gate siblings** (`navmesh_for`, `region_field`, `player_lit_mask`, `visible_cells`, `visible_cells_cached` in `scene/mod.rs` — an absent `scene_grid_sizes()` entry now returns `None`/empty instead of synthesizing a 100-unit grid; `region_field`'s signature changed to `-> Option<RegionField>`, its three callers (`pathfind`'s two branches, `move_exec::execute_move`) refuse via `let-else` on `None`, and `MoveReject` gained a `SceneUnknown` variant mirroring `Degenerate`; `enrich_vision_explored` (`ws/conn.rs`) now `continue`s past a scene absent from either its `grid` or `grid_shapes` map, never synthesizing a fallback `SquareGrid`). The fail-open default is now removed at ALL sites — `scene_grid_sizes` remains the sole intentional defaulting SOURCE, not a survivor. See `docs/TODO.md` |
  **How to apply.** (1) When you find two paths that must agree, do not verify they agree today —
  make one *derive* from the other, or have both read one shared symbol, so agreement is structural.
  (2) When you fix one instance, grep for the other copies **in the same commit**; the last row
  above was created by the commit fixing the row above it. (3) Pin parity with an anti-drift test
  that exercises BOTH paths through the shared symbol (see `MAX_GATE_WALK_COORD`'s, which catches a
  value change or a `>`/`>=` flip on either side). (4) A test that passes because both paths are
  wrong the same way proves nothing — mutate one side and confirm the test fails.
- **Cross-platform from day one (CI-verified).** `std::path` only (no hardcoded separators),
  `#[cfg]`-gate OS-specific code for every target, three-OS CI matrix, responsive/touch UI.
  [CLAUDE.md Cross-Platform; ARCHITECTURE §2 invariant 10]
- **`dist/` must be built before any `cargo` build of the server** — `rust-embed` validates
  `../../dist/` at COMPILE time. [[embed-dist-compile-ordering]]
- **Capability/permission model** layered server/world/document roles. [[capability-permissions]]
- **Three-band document shape (M13-0): envelope `name` + typed `engine` + opaque `system`.**
  Server runs no third-party code; authority over the opaque `system` body is structural only
  (size/field-path/`deny_unknown_fields`) — no semantic validation, ever. The typed `engine` body
  (present only for the 17 engine-defined doc types: tokens, actors, scenes, walls, regions,
  lights, drawings, templates, messages, and the world/vision/lighting/chat/dice/faction/
  condition/channel config-docs) gets REAL server-side ingress validation instead
  (`validate_engine`/`validate_engine_tree`, `deny_unknown_fields` per struct) — this is the band
  engine-owned geometry (movement-collision, vision) now lives in, not a `system`-body exception
  (ARCHITECTURE §2 invariant 6). See `shadowcat-codebase-documents-permissions` for the
  `data/engine/` registry and `shadowcat-codebase-scene-rendering`/`-chat`/`-actors-tokens` for
  the per-subsystem re-root.

## Gotchas

- **`CLAUDE.md` is git-ignored** — it is local-only; durable rules live in `ARCHITECTURE.md` §2,
  the real source of truth. [[claude-md-is-git-ignored]]
- **ts-rs types are generated** — change the Rust enum/struct, regenerate, then mirror in the
  client Zod schema (a drift guard enforces parity).
- **Decide on technical merits, not "how Foundry does it."** [[decide-on-merits-not-foundry]]
- **Tests yield to correct code** — fix code only if objectively wrong; else fix the test.
  [[tests-yield-to-correct-code]]

## Pointers

**Knowledge-layer map** (which layer answers which question):
- **this skill family** (`shadowcat-codebase-*`) — orientation: what a subsystem is, its seams,
  invariants, gotchas.
- **graphify** (`graphify-out/`) — relationships: `graphify query "<q>"`,
  `graphify path "<A>" "<B>"`, `graphify explain "<concept>"`.
- **`docs/design/`** — rationale: `ARCHITECTURE.md` (invariants/tech), `M2-data-foundation.md`,
  per-system docs; `docs/PLAN.md` = milestone roadmap.
- **memory** (`~/.claude/projects/C--Dev-Shadowcat/memory/`) — cross-session lessons + resume state.

**Build / test / lint commands:**
- Client build (produces `dist/`): `pnpm build` (= `pnpm --filter @shadowcat/shell build`).
- Client tests: `pnpm -r test` (Vitest). Typecheck: `pnpm -r typecheck`. Lint: `pnpm lint` (ESLint).
- Server (from `src/server/`): `cargo test`, `cargo fmt`, `cargo clippy`.
- CI builds the client **before** `cargo` (embed ordering) across the three-OS matrix.

**Subsystem skills:** `documents-permissions`, `actors-tokens`, `scene-rendering`,
`realtime-sync`, `client-shell`, `assets`, `dice`, `chat`, `nightfox`, `module-toolchain`,
`sheets`, `panels`, `server-ops`, `templates` (all `shadowcat-codebase-*`).

## Maintaining this skill family

This family is not fixed — **create a new `shadowcat-codebase-<subsystem>` skill whenever work
opens a subsystem none of the existing skills covers** (e.g. a new milestone like effects,
pathfinding, chat, or audio). Don't stretch an unrelated skill to fit.

When adding one:
1. Follow the fixed shape — Purpose / Key files & seams / Hard invariants / Gotchas / Pointers —
   and keep it orientation+index: point INTO graphify, `docs/design/`, and memory; never duplicate
   them. Cite each invariant's memory slug or design-doc section.
2. Add it to the **Subsystem skills** list above, and add its path globs to the activation hook
   (`.claude/hooks/codebase-skill-reminder.py` `SUBSYSTEMS` map).
3. This creation step is part of the reviewed skill-update gate (see CLAUDE.md
   `## Codebase Skills & Agents`): a new subsystem with no skill is itself a gate violation.
