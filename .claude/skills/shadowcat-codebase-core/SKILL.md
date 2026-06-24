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
- **Cross-platform from day one (CI-verified).** `std::path` only (no hardcoded separators),
  `#[cfg]`-gate OS-specific code for every target, three-OS CI matrix, responsive/touch UI.
  [CLAUDE.md Cross-Platform; ARCHITECTURE §2 invariant 10]
- **`dist/` must be built before any `cargo` build of the server** — `rust-embed` validates
  `../../dist/` at COMPILE time. [[embed-dist-compile-ordering]]
- **Capability/permission model** layered server/world/document roles. [[capability-permissions]]
- **Server runs no third-party code**; authority over the `system` body is structural only
  (size/field-path/`deny_unknown_fields`), except engine-owned geometry (movement-collision,
  vision) (ARCHITECTURE §2 invariant 6).

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
`realtime-sync`, `client-shell`, `assets` (all `shadowcat-codebase-*`).

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
