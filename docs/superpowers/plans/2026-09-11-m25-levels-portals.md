# M25 · Multi-level maps + portals — Implementation Plan

> **For agentic workers:** Execute task-by-task in order; each task's steps use checkbox
> (`- [ ]`) syntax. Written for a sonnet-class implementer with no conversation context —
> every path, symbol and test name below is exact; read the cited code before editing it.

**Goal:** a scene can hold several floors (`SceneEngine.levels: Vec<SceneLevel>`, each an
elevation band with its own background); a token's floor is derived from its elevation
(`scene::elevation::level_of`); walls/regions/drawings/templates gain the same elevation-band
shape a wall's occlusion already used (`WallElevation` renamed `ElevationBand`, one predicate
`band_contains` shared by occlusion, movement, region selection and render filtering); movement,
vision, lighting and explored-fog memory all become level-scoped; a region trigger can teleport a
token within or across scenes (`TriggerEffect::Teleport`, `WriteOrigin::Trigger`); the client gains
a viewed-level concept (`AppContext.viewedLevel`), a level switcher, a levels editor, and
scene-tools authoring that stamps the viewed level's band/elevation onto new geometry.

**Architecture:** server-authoritative levels — a level is data on the scene document, never a
separate scene document (decision D4, master §9); the ONE shared predicate `band_contains`
threads through every banded-geometry consumer (never-fork); the client never re-derives a token's
level, it reads `level_of`'s TS mirror over the same `SceneLevel[]` the server sends. Cross-floor
travel is a portal's job (`TriggerEffect::Teleport`), not a level's.

**Tech stack:** Rust (server engine/scene/ws/data), TypeScript (`@shadowcat/core`,
`@shadowcat/render`, `@shadowcat/ui-kit`, `@shadowcat/shell`), Svelte 5 (runes), Vitest,
Playwright (written here, dispatcher-run).

**Spec:** `docs/superpowers/specs/2026-09-11-m25-levels-portals-design.md` — read it first.
Master: `docs/superpowers/specs/2026-09-11-phase3-master-integration-design.md` §0 (campaign
directives), §2.4 (the seam this milestone owns), §2.7 (`STAGE_OVERLAY_CONTRACT` note — M25
decides at plan time; see Task 14), §3 (shared-file conventions), §4 (global constraints), §5
(merge order — M25 is 5th, after M22/M28/M24/M23), §6 (skill-update gate), §9 D4 (levels are
elevation bands, not scene documents).

**Worktree:** `C:/Dev/Shadowcat-m25`, branch `m25-levels`. Tasks 1–20 depend on nothing outside
this milestone and run in the dependency order below; Task 21 is the merge-forward integration
task and runs ONLY after the dispatcher confirms M22, M28, M24 and M23 are all on `origin/main`
(master §5) — it holds until then, per master's "never stubs the seam" rule.

## Execution directives

**Every dispatched agent's first prompt MUST contain this paragraph verbatim:**

> The iron rule is no deferrals of existing work, or new work as it comes up - we fix this now
> unless I give my EXPRESS authorization. The only exception is if a bug or to-do has a genuine
> blocker that is already logged in a milestone in PLAN.md that has not been started yet. Another
> iron clad is rule is that when faced with a design fork, determine the best long term shape in
> keeping with our plans and goals, and implement accordingly. You only need to ask me if the
> question "what is the best long term shape in keeping with our plans and goals?" is not able to
> answer the question. Churn is not a concern. This paragraph must be copied verbatim to any
> agents dispatched in this campaign.

**Reporting rule:** a subagent delivers its report as the Agent tool result, via `SendMessage` to
the dispatcher, or by writing a named file; the dispatching prompt states which. An agent given a
`name` never returns a result — omit `name` for every dispatch whose report the dispatcher needs.

**Opus is banned** for every dispatch in this campaign, at every tier.

## Model/Effort directives

- Implementation: `shadowcat-codebase:shadowcat-coder` — sonnet, effort medium. One task (or a
  tightly-coupled pair named together below) per dispatch.
- Review: `shadowcat-codebase:shadowcat-spec-reviewer` + `shadowcat-codebase:shadowcat-code-reviewer`
  — sonnet, effort high, dispatched as a pair, blind, against a dispatcher-pre-generated diff
  (reviewers have no Bash — the diff is pasted into the prompt, not fetched).
- Escalation: a coder reporting BLOCKED, or a reviewer finding shallow/uncertain, re-dispatches to
  the matching `-fable` twin (sonnet body, escalation-only capacity) — never to an `-opus` twin,
  never straight to the human before the twin has run.
- Every dispatch names its `effort` explicitly; an unspecified effort silently inherits the
  session's and defeats this tiering.

## Buddy-check directives

After every task's implementation commit, the dispatcher pre-generates the task's diff
(`git diff <task-start-sha>..HEAD -- <task paths>`) and dispatches the spec+code reviewer pair
blind against it plus this plan's task text and the relevant spec section. A finding the
dispatcher agrees with is fixed before the next task starts; a finding the dispatcher disagrees
with is a design question surfaced to the user, never silently overridden. The FULL branch diff
gets a second buddy-check pass at the end of Task 20, before Task 21's merge-forward.

## Global constraints (verbatim from master §4)

- No lint suppressions of any kind (`#[allow]`, `#[expect]`, `eslint-disable`, `@ts-ignore`);
  `pnpm lint:allowances` is a gate. Fix the code.
- File-size: 5,000-line soft limit needs the owner's allowlist signature, 10,000 hard; Rust test
  bodies in sibling files (`pnpm lint:file-size`, `pnpm lint:inline-tests`).
- Comments cite symbols, never files/lines; no milestone ids, dates, sweep markers or history
  narration in `.ts`/`.rs`/`.svelte` (`pnpm lint:comments`).
- Every new `.ts` unit test that never touches the DOM opens with `// @vitest-environment node`.
- Deletion only through `trash`; never `rm`/`Remove-Item`/`git rm` as the sole step.
- Commits name their paths: `git commit -m "..." -- <paths>`; never `git add -A`.
- Long commands (`cargo test --all`, `pnpm -r test`, `pnpm build:all`) run in the background with
  output to a log file; read the log before claiming green.
- **Cross-platform:** `std::path` only; `#[cfg]`-gated OS code has an implementation for every
  target the matrix builds (Linux, macOS, Windows); responsive + touch-sized UI.
- **Licenses:** MIT / Apache-2.0 / BSD / zlib / MPL-2.0 only; media codecs royalty-free. Every new
  dependency lands with a `Cargo.toml`/`package.json` comment naming its license. (M25 adds no
  new dependency.)
- **Binary size:** `pnpm lint:binary-size` guards the 60 MiB release binary.
- **Server by default:** computation runs on the server; client-side work needs a reason
  (presentation, input capture, optimistic prediction). `docs/design/ARCHITECTURE.md` §2
  invariants 1, 6 and 11 govern every design fork.
- **UX outranks data secrecy** (invariant 11): send-then-hide is acceptable; PII and remote-device
  security are the two ironclad exceptions.
- `pnpm build` precedes any cargo build (rust-embed validates `dist/` at compile time).
- The Playwright suite is DISPATCHER-run on port 31999 (one suite at a time on the machine); this
  milestone WRITES `levels.spec.ts` and does not run it (Task 19).

The full gate battery a milestone must show green before its PR: `cargo test --all`, `cargo fmt
--check`, `cargo clippy --all-targets -- -D warnings`, `cargo clippy -- -D missing-docs -D
clippy::missing-docs-in-private-items`, `git diff --exit-code src/types/generated` after regen,
`pnpm -r typecheck`, `pnpm -r test`, `pnpm build`, `pnpm lint`, `lint:docs`, `lint:props`,
`lint:comments`, `lint:allowances`, `lint:file-size`, `lint:inline-tests`, `lint:aria-labels`,
`lint:gate-manifest`, `lint:settings-privacy`, `lint:binary-size` (release build), `pnpm
docs:check-examples`, `pnpm docs:check-rust-examples`, `pnpm run test:scripts`, `pnpm run
check:svelte-runtime`, `pnpm --filter "shadowcat-example-*" build`, `pnpm --filter @shadowcat/core
test:e2e`, and `pnpm gate:push` (tree-keyed receipt) immediately before `git push`.

---

## Task 1: `ElevationBand` rename + `band_contains` + `SceneLevel` + `level_of` + the shared conformance corpus

Foundational data model. No dependency on any other task.

**Files:**
- Modify: `src/types/index.ts` — a HAND-MAINTAINED barrel (not ts-rs output): change the line
  `export type { WallElevation } from "./generated/engine/WallElevation";` to
  `export type { ElevationBand } from "./generated/engine/ElevationBand";` and add
  `export type { SceneLevel } from "./generated/engine/SceneLevel";` beside the `SceneEngine`
  export — the core `levels.ts` below imports both from `@shadowcat/types`, so `pnpm -r
  typecheck` fails without this edit.
- Modify: `src/server/src/scene/elevation/tests.rs` — the EXISTING file: its `use
  crate::data::engine::WallElevation;` import and its `fn band(...) -> WallElevation {
  WallElevation { bottom, top } }` helper are renamed to `ElevationBand` (the conformance test
  added further below lands in this same file).
- Modify: `src/server/src/data/engine/geometry.rs` — rename `WallElevation` → `ElevationBand`
  everywhere in this file (struct name, its doc comment's `use` line, its doc example, and
  `WallEngine.elevation: Option<ElevationBand>` plus that field's own comment's type reference).
  Add the doc-comment note that `ElevationBand` is now shared by every banded-geometry engine, not
  wall-only. Add the new field to `RegionEngine`, `DrawingEngine`, `TemplateEngine`:
  ```rust
  /// The elevation band this region's geometry occupies; absent = every level (pre-levels
  /// authoring, or a level-less scene). Read by `scene::elevation::band_contains` — the SAME
  /// predicate `WallEngine.elevation`'s occlusion test and the movement gate consult.
  #[serde(default)]
  pub elevation: Option<ElevationBand>,
  ```
  (identical field + doc shape for `DrawingEngine`/`TemplateEngine`, "region's"/"drawing's"/
  "template's" substituted). Each struct's doc example gains `elevation: None,` so the existing
  doctests keep compiling.
- Modify: `src/server/src/data/engine/mod.rs` — the `pub use geometry::{...}` re-export list:
  `WallElevation` → `ElevationBand`.
- Modify: `src/server/src/data/engine/tests.rs` — `WallElevation` → `ElevationBand` (the one
  citation at the existing line building a wall with a band).
- Modify: `src/server/src/scene/elevation.rs` — extract the point-in-band test out of
  `wall_occludes` into the new shared predicate, then have `wall_occludes` call it:
  ```rust
  /// Whether elevation band `band` contains point `e`: `bottom ≤ e ≤ top`; an absent end is
  /// unbounded, and `band: None` contains every elevation. Fail-closed: a malformed interval
  /// (`bottom > top`) or a non-finite band endpoint contains everything (the pre-elevation
  /// behavior for that wall). Shared by `wall_occludes` (vision/light occlusion), the movement
  /// gate (`SceneEcs::move_wall_entries`'s per-mover filter), `SceneEcs::region_field`'s per-mover
  /// selection, `SceneEcs::trigger_regions`'s per-mover selection, and the drawing/region render
  /// filters (`sceneScopedDocs`'s TS mirror `bandContains`) — the never-fork pin.
  pub(crate) fn band_contains(band: Option<&eng::ElevationBand>, e: f64) -> bool {
      let Some(b) = band else { return true };
      if b.bottom.is_some_and(|v| !v.is_finite()) || b.top.is_some_and(|v| !v.is_finite()) {
          return true;
      }
      let lo = b.bottom.unwrap_or(f64::NEG_INFINITY);
      let hi = b.top.unwrap_or(f64::INFINITY);
      if lo > hi {
          return true;
      }
      lo <= e && e <= hi
  }

  /// Whether a wall with elevation band `band` occludes a sight/light source at elevation `e`:
  /// `band_contains(band, e)`, additionally fail-closed (occludes) when `e` itself is
  /// non-finite — a corrupt wall or a NaN leaked past `elevation_or_ground` never opens a
  /// sightline the scene did not have.
  pub(crate) fn wall_occludes(band: Option<&eng::ElevationBand>, e: f64) -> bool {
      if !e.is_finite() {
          return true;
      }
      band_contains(band, e)
  }
  ```
  Update `BandedWall`'s type alias comment (`Option<eng::WallElevation>` → `Option<eng::ElevationBand>`)
  and every doc example in this file citing `WallElevation`.
  Add `level_of` in the same file:
  ```rust
  /// The level whose `[bottom, top)` band contains `elevation`; if none contains it, the
  /// highest level whose `bottom <= elevation` (a token on a roof is on the top floor); below
  /// every level's bottom, the lowest level; `levels` empty, `None`. Callers pass an
  /// already-clamped elevation (`elevation_or_ground`'s output), never a raw stored value.
  /// Mirrored exactly by `levelOf` in `@shadowcat/core`, pinned by the shared conformance
  /// corpus this module's tests read.
  pub(crate) fn level_of(
      levels: &[eng::SceneLevel],
      elevation: f64,
  ) -> Option<&eng::SceneLevel> {
      if levels.is_empty() {
          return None;
      }
      if let Some(l) = levels.iter().find(|l| l.bottom <= elevation && elevation < l.top) {
          return Some(l);
      }
      let mut below = levels.iter().filter(|l| l.bottom <= elevation).peekable();
      if below.peek().is_some() {
          return below.max_by(|a, b| a.bottom.partial_cmp(&b.bottom).unwrap());
      }
      levels.iter().min_by(|a, b| a.bottom.partial_cmp(&b.bottom).unwrap())
  }
  ```
- Modify: `src/server/src/data/engine/scene.rs` — add `SceneLevel` beside `SceneEngine`:
  ```rust
  /// Upper bound (levels) `SceneEngine::validate` enforces per scene.
  pub const MAX_SCENE_LEVELS: usize = 32;
  /// Upper bound (chars) for a `SceneLevel::id`.
  pub const MAX_LEVEL_ID_CHARS: usize = 64;

  /// One floor of a multi-level scene: a named elevation band with its own background. Levels
  /// are data on the scene, never separate scene documents — a token's floor is
  /// derived from its own elevation via `scene::elevation::level_of`, never authored per-token.
  ///
  /// # Examples
  ///
  /// ```
  /// use shadowcat::data::engine::SceneLevel;
  ///
  /// let ground = SceneLevel {
  ///     id: "ground".to_string(), name: "Ground Floor".to_string(),
  ///     bottom: 0.0, top: 10.0, background: None,
  /// };
  /// assert_eq!(ground.bottom, 0.0);
  /// ```
  #[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
  #[ts(export, export_to = "../../types/generated/engine/")]
  #[serde(deny_unknown_fields)]
  pub struct SceneLevel {
      /// Stable id, non-empty, `MAX_LEVEL_ID_CHARS`-bounded, unique within the scene. Named by
      /// `TriggerRegion`-independent authoring (scene-tools, `LevelsEditor`) and by
      /// `AppContext.viewedLevel`.
      pub id: String,
      /// Display name.
      pub name: String,
      /// Lower band bound, scene elevation units, inclusive.
      pub bottom: f64,
      /// Upper band bound, scene elevation units, exclusive; must exceed `bottom`.
      pub top: f64,
      /// Background image asset id for this floor; `None` falls back to
      /// `SceneEngine::background`.
      #[serde(default)]
      pub background: Option<String>,
  }
  ```
  Add `#[serde(default)] pub levels: Vec<SceneLevel>,` as the LAST field of `SceneEngine` (master
  §3's append-at-the-end convention for this struct), with a doc comment naming `level_of` and
  D4. Update `SceneEngine`'s doc example to add `levels: Vec::new(),`. Extend
  `SceneEngine::validate`:
  ```rust
  impl SceneEngine {
      /// Every combat lifecycle formula present parses, and `levels` is well-formed (see
      /// `validate_levels`).
      pub(crate) fn validate(&self) -> Result<(), String> {
          match &self.combat {
              Some(c) => c.validate("combat")?,
              None => {}
          }
          self.validate_levels()
      }

      /// `levels`: at most `MAX_SCENE_LEVELS`, every id non-empty/bounded/unique, every band
      /// finite with `bottom < top`, bands non-overlapping (sorted by `bottom`, adjacent bands
      /// compared), and a present `background` non-empty.
      fn validate_levels(&self) -> Result<(), String> {
          if self.levels.len() > MAX_SCENE_LEVELS {
              return Err(format!("levels exceeds {MAX_SCENE_LEVELS}"));
          }
          let mut seen = std::collections::HashSet::new();
          for level in &self.levels {
              if level.id.is_empty() || level.id.chars().count() > MAX_LEVEL_ID_CHARS {
                  return Err("level id must be non-empty and bounded".to_string());
              }
              if !seen.insert(level.id.as_str()) {
                  return Err(format!("duplicate level id '{}'", level.id));
              }
              if !level.bottom.is_finite() || !level.top.is_finite() || level.bottom >= level.top {
                  return Err(format!("level '{}' has an invalid band", level.id));
              }
              if level.background.as_deref() == Some("") {
                  return Err(format!("level '{}' background must be non-empty when present", level.id));
              }
          }
          let mut bands: Vec<(f64, f64)> = self.levels.iter().map(|l| (l.bottom, l.top)).collect();
          bands.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
          for w in bands.windows(2) {
              if w[0].1 > w[1].0 {
                  return Err("levels overlap".to_string());
              }
          }
          Ok(())
      }
  }
  ```
- Modify: `src/server/src/data/engine/mod.rs` — export `SceneLevel`, `MAX_SCENE_LEVELS`,
  `MAX_LEVEL_ID_CHARS` from `scene::{...}` in the existing `pub use scene::{...}` list.
- Create: `src/client/core/src/__fixtures__/levels-conformance.json` — mirrors
  `src/client/formula/src/__fixtures__/conformance.json`'s shape: a top-level `{"cases": [...]}`
  array, each case `{ "name": string, "levels": SceneLevel[], "elevation": number, "expect":
  string | null }` (`expect` is the resolved level's `id`, or `null`). Cover: empty `levels` →
  `null`; elevation inside a band; elevation exactly on a `bottom` (inclusive); exactly on a `top`
  (exclusive — resolves to the level ABOVE if one starts there, else the same level via the
  "highest whose bottom <= elevation" fallback); above every level (roof case — resolves to the
  highest); below every level (resolves to the lowest); two adjacent levels `[0,10)`/`[10,20)`
  with elevation `10.0` (must resolve to the second, not the first — the exclusive-top boundary).
  At least 8 cases.
- Create: `src/client/core/src/levels.ts` — mirrors the Rust side exactly:
  ```ts
  import type { SceneLevel, ElevationBand } from "@shadowcat/types";
  export type { SceneLevel, ElevationBand };

  /** Whether elevation band `band` contains point `e`. Mirrors `scene::elevation::band_contains`
   * exactly — the shared predicate `sceneScopedDocs`'s band-shaped filter and `LevelsEditor`'s
   * range display both call. `band: null` contains every elevation; a malformed interval
   * (`bottom > top`) or a non-finite endpoint fails closed to containing everything. */
  export function bandContains(band: ElevationBand | null, e: number): boolean {
    if (band === null) return true;
    if ((band.bottom !== null && !Number.isFinite(band.bottom)) ||
        (band.top !== null && !Number.isFinite(band.top))) return true;
    const lo = band.bottom ?? -Infinity;
    const hi = band.top ?? Infinity;
    if (lo > hi) return true;
    return lo <= e && e <= hi;
  }

  /** The level whose `[bottom, top)` band contains `elevation`; mirrors `scene::elevation::level_of`
   * exactly (see that function's doc for the roof/below-every-level fallback rules). */
  export function levelOf(levels: SceneLevel[], elevation: number): SceneLevel | null {
    if (levels.length === 0) return null;
    const containing = levels.find((l) => l.bottom <= elevation && elevation < l.top);
    if (containing) return containing;
    const below = levels.filter((l) => l.bottom <= elevation);
    if (below.length > 0) {
      return below.reduce((a, b) => (a.bottom > b.bottom ? a : b));
    }
    return levels.reduce((a, b) => (a.bottom < b.bottom ? a : b));
  }
  ```
- Modify: `src/client/core/src/index.ts` — export `bandContains`, `levelOf`, `SceneLevel`,
  `ElevationBand` from `./levels`.
- Create: `src/client/core/src/levels.test.ts` (`// @vitest-environment node`) — reads
  `__fixtures__/levels-conformance.json` via `readFileSync(new URL(...), "utf8")` (the formula
  corpus's exact pattern) and asserts `levelOf(c.levels, c.elevation)?.id ?? null === c.expect` for
  every case, plus 2–3 direct `bandContains` unit cases (band `null`, an unbounded-top band, an
  inverted `bottom > top` band failing closed).
- Create: `src/server/src/scene/elevation/tests.rs` additions — a `mod levels_conformance` (or a
  new function in the existing file) that reads the SAME JSON via `include_str!(
  "../../../../../client/core/src/__fixtures__/levels-conformance.json")` (five `../` from
  `src/server/src/scene/elevation/` to repo root, matching the formula precedent's relative-depth
  convention), deserializes into `Vec<{levels: Vec<SceneLevel>, elevation: f64, expect: Option<String>}>`
  via a local `#[derive(Deserialize)]` struct, and asserts `level_of(&c.levels,
  c.elevation).map(|l| l.id.clone()) == c.expect` for every case. Add direct unit tests for
  `band_contains`: `None` band contains everything; an unbounded-top band; `bottom > top` fails
  closed to `true`; a non-finite endpoint fails closed to `true`.

- [ ] **Step 1:** write `levels.test.ts` and the Rust conformance test FIRST against the not-yet-existing
  corpus/functions (both fail to compile/run); write the corpus JSON; implement `ElevationBand`
  rename, `band_contains`, `SceneLevel`, `level_of`, `SceneEngine.levels` + validate, the new
  `elevation` fields on Region/Drawing/Template engines.
- [ ] **Step 2:** `cargo test --all` (background + log; regenerates `src/types/generated/engine/ElevationBand.ts`,
  `SceneLevel.ts` and updates `WallElevation.ts`'s absence) — `git diff --exit-code
  src/types/generated` FAILS as expected; stage the new/changed generated files. `rg WallElevation
  src` returns ZERO hits outside `src/types/generated` history (there is none — `git mv` is
  banned, the old generated file is simply gone and the new one created; verify with `ls
  src/types/generated/engine/ElevationBand.ts`). `pnpm --filter @shadowcat/core test`, `pnpm -r
  typecheck`, `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check`, `pnpm lint:docs`,
  `pnpm docs:check-examples`, `pnpm docs:check-rust-examples` PASS.
- [ ] **Step 3:** `git commit -m "feat(scene): SceneLevel + level_of; WallElevation renamed ElevationBand and shared by region/drawing/template" -- src/server/src/data/engine/ src/server/src/scene/elevation.rs src/server/src/scene/elevation/ src/client/core/src/levels.ts src/client/core/src/levels.test.ts src/client/core/src/__fixtures__/levels-conformance.json src/client/core/src/index.ts src/types/`

## Task 2: movement gate consults the elevation band

Depends on Task 1 (`ElevationBand`, `band_contains`, `level_of`, `SceneLevel`).

**Files:**
- Modify: `src/server/src/data/engine/geometry.rs` — `WallEngine.elevation`'s doc comment: delete
  the sentence "Never consulted by the movement gate (movement is ground-plane)." and replace with
  "Consulted by the movement gate exactly as it is by sight/light occlusion — see
  `scene::elevation::band_contains` and `SceneEcs::move_wall_entries`."
- Modify: `src/server/src/scene/mod.rs` — `move_walls` (line ~2314) becomes a raw BANDED collector
  paired with a per-mover filtered accessor, mirroring `sight_wall_entries`/`sight_walls_for`
  exactly:
  ```rust
  /// The scene's `blocksMove` wall segments with their elevation bands — the raw collector
  /// behind `move_walls_for`. Two-value secrecy contract identical to `region_field`'s: `viewer:
  /// None` is authoritative (execution, GM requester); `viewer: Some(user)` is per-requester
  /// (routers). Mirrors the wall filter in `blocks_move` (doc_type "wall", parent = scene,
  /// `engine.blocksMove == true`) — see that fn's own INVARIANT.
  pub(crate) fn move_wall_entries(&self, scene: Uuid, viewer: Option<Uuid>) -> Vec<elevation::BandedWall> {
      let mut out = Vec::new();
      for w in self.world.query::<&SceneEntity>().iter() {
          if w.doc.doc_type != "wall" || w.doc.parent_id != Some(scene) {
              continue;
          }
          let Some(wall) = self.engine_as_cached::<eng::WallEngine>(w.doc.id, &w.doc) else {
              continue;
          };
          if wall.blocks_move != Some(true) {
              continue;
          }
          if !engine_tier_visible(&w.doc, viewer) {
              continue;
          }
          out.push((
              vision::Seg {
                  a: (wall.seg.x1, wall.seg.y1),
                  b: (wall.seg.x2, wall.seg.y2),
              },
              wall.elevation,
          ));
      }
      out
  }

  /// The `blocksMove` wall segments of `scene`, per-requester (`viewer`, see `move_wall_entries`),
  /// filtered to the segments whose band contains `mover_elevation`
  /// (`elevation::band_contains` — a floor-2 wall no longer blocks a floor-1 mover). Callers MUST
  /// pass the mover's OWN resolved elevation (`elevation::elevation_or_ground` of its stored
  /// value), never a client-claimed one.
  pub(crate) fn move_walls(&self, scene: Uuid, viewer: Option<Uuid>, mover_elevation: f64) -> Vec<vision::Seg> {
      elevation::walls_at_elevation(&self.move_wall_entries(scene, viewer), mover_elevation)
  }
  ```
  Update the doc comment above the OLD `move_walls` (now split) to point at `move_wall_entries` +
  `move_walls`'s new third parameter; keep the "Scope: this is the ROUTING wall set only" and
  "two-value secrecy contract" paragraphs, moved onto `move_wall_entries`.
- Modify: `src/server/src/scene/mod.rs` `RouteMover` (line ~847) — add a field:
  ```rust
  /// The mover's resolved elevation (`elevation::elevation_or_ground` of the named token's
  /// stored value, or the hypothetical wire value for a token-less preview) — filters
  /// `move_walls`/`region_field` to the mover's floor. NEVER the client's raw claim for a
  /// named-token request: the caller re-resolves it off the token exactly as `footprint_radius`
  /// is re-resolved.
  pub elevation: f64,
  ```
  Update `RouteMover`'s doc example to add `elevation: 0.0,`.
- Modify: `src/server/src/scene/mod.rs` `pathfind` (line ~2433) — destructure `mover.elevation`
  alongside the existing fields; change the wall-set line (~2470) to
  `self.move_walls(scene, if is_gm { None } else { Some(user) }, elevation)`; change BOTH
  `region_field` call sites in this function (grid-stepped ~2512 and continuous ~2539) to
  `self.region_field(scene, if is_gm { None } else { Some(user) }, elevation)` (Task 3 adds the
  third parameter — this task's edit here is coupled to Task 3's signature change; land both in
  ONE commit since `pathfind` cannot compile against only one of them).
- Modify: `src/server/src/scene/mod.rs` `blocks_move` test helper (line ~4127, `#[cfg(test)]`) —
  add elevation filtering to match: `blocks_move(&self, scene, a0, a1, mover_elevation: f64)`
  consulting `elevation::walls_at_elevation` before the `segments_cross` loop, so the anti-drift
  test this helper backs (`move_walls`/`blocks_move` agreement, cited by `move_wall_entries`'s doc
  comment) still holds under banding. Update its call sites in
  `src/server/src/scene/tests/*.rs` (`rg "\.blocks_move\(" src/server/src/scene/tests`) to pass an
  elevation argument (`elevation::GROUND` for the existing unbanded cases).
- Modify: `src/server/src/scene/move_exec.rs` — `MoveGateInputs` (line ~269) gains `mover_elevation: f64`
  (doc: "the mover's resolved elevation, filtering the wall gate to its floor — see
  `SceneEcs::move_walls`'s third parameter"); `execute_move`'s wall-fetch line (`let gate_walls =
  ecs.move_walls(scene, None);`, line ~446) becomes `ecs.move_walls(scene, None,
  inputs.mover_elevation)`.
- Modify: `src/server/src/ws/room.rs` — every `MoveGateInputs { .. }` construction site
  (`rg "MoveGateInputs \{" src/server/src/ws`) gains `mover_elevation:
  elevation::elevation_or_ground(token_engine.elevation)` read off the same `TokenEngine` decode
  the site already holds for `budget`/`traits` (per `RouteMover`'s doc comment, "mirrors
  `move_exec::MoveGateInputs`, whose `budget`/`traits` these two fields mirror").
- Modify: `src/server/src/ws/conn.rs` `handle_pathfind` — resolve `RouteMover.elevation` the same
  way: named token ⇒ `elevation::elevation_or_ground` of its stored `TokenEngine.elevation`;
  token-less hypothetical preview ⇒ `0.0` (ground — the wire carries no elevation, per spec §3
  "Movement preview").

**Tests** (add to `src/server/src/scene/tests/pathfind_and_vision.rs` and
`src/server/src/scene/elevation/tests.rs`):
- A `blocksMove` wall banded `elevation: {bottom: 10, top: 20}` (floor 2) does NOT block a
  floor-0 mover's `execute_move`/`pathfind`/`gate_walk`, through the identical corridor a
  ground-floor wall (no band) DOES block.
- The SAME wall DOES block a mover at elevation `15` (inside the band).
- `move_wall_entries`/`move_walls` agreement with the updated `blocks_move` test helper across a
  banded wall (the anti-drift parity test, extended with an elevation argument).

- [ ] **Step 1:** write the failing tests; implement.
- [ ] **Step 2:** `cargo test --all` (background + log), `cargo clippy --all-targets -- -D
  warnings`, `cargo fmt --check`, `pnpm docs:check-rust-examples` PASS.
- [ ] **Step 3:** `git commit -m "feat(scene): the movement gate respects a wall's elevation band" -- src/server/src/scene/ src/server/src/ws/room.rs src/server/src/ws/conn.rs src/server/src/data/engine/geometry.rs`

## Task 3: `region_field` gains an elevation param; `trigger_regions` selects by band

Depends on Task 1. Lands in the SAME commit-worthy change as Task 2's `pathfind` edits (both
touch `region_field`'s call sites in `pathfind`) — implement Task 3 before finishing Task 2's
Step 2, or implement them as one combined dispatch; either way `pathfind` must compile against
BOTH new signatures together. This task's own files/tests are listed separately for clarity.

**Files:**
- Modify: `src/server/src/scene/mod.rs` `region_field` (line ~2681) — add a THIRD parameter
  `elevation: f64`; inside the region-collecting loop, after the existing `engine_tier_visible`
  check, add `if !elevation::band_contains(region_eng.elevation.as_ref(), elevation) { continue; }`.
  Update the doc comment: "the composed field additionally excludes any region whose elevation
  band does not contain `elevation` (`band_contains`) — the mover's floor, never the viewer's."
- Modify: `src/server/src/scene/mod.rs` `trigger_regions` (line ~2731) — add a parameter
  `elevation: f64`; add the identical `band_contains` filter before pushing a `TriggerRegion` row.
  Update its doc comment to state the band selection alongside the existing
  visible-to-all/triggers-non-empty filters.
- Modify: `src/server/src/ws/room.rs` — the ONE call site of `trigger_regions` (inside
  `fire_region_triggers`, reading `ecs.trigger_regions(scene)`) becomes
  `ecs.trigger_regions(scene, elevation::elevation_or_ground(token_eng.as_ref().and_then(|_| None)))`
  — resolve properly: the entering token's elevation is available from `token_doc`'s decoded
  `TokenEngine` (`token_eng`, already computed later in the function for the actor join — hoist
  that decode earlier in `fire_region_triggers` so `elevation::elevation_or_ground(token_eng
  .as_ref().and_then(|t| t.elevation))` is available before the `trigger_regions` call).
- Modify: every OTHER `region_field(` call site (`rg "region_field\(" src/server/src` —
  production sites are `pathfind` (Task 2, already updated) and any test helper) to pass the
  mover's elevation; test call sites default to `elevation::GROUND` unless the test is
  specifically about banding.

**Tests:**
- `region_field(scene, viewer, elevation)` excludes a region banded to a different floor and
  includes one banded to the mover's floor (or unbanded).
- `trigger_regions` selects the same way: a floor-2 teleport/condition-add region does not fire
  for a floor-0 token walking through the identical cells.

- [ ] **Step 1:** failing tests; implement (jointly with Task 2's `pathfind` edits).
- [ ] **Step 2:** `cargo test --all`, clippy, fmt PASS.
- [ ] **Step 3:** included in Task 2's Step 3 commit, or its own:
  `git commit -m "feat(scene): region_field and trigger_regions select by elevation band" -- src/server/src/scene/mod.rs src/server/src/ws/room.rs`

## Task 4: navmesh cache keyed by level

Depends on Task 1, Task 2.

**Files:**
- Modify: `src/server/src/scene/mod.rs` `NavmeshCacheKey` (line ~1186) — becomes `(Uuid, i64,
  String, Vec<(u64, u64, u64, u64)>)`, the new `String` slot holding the level id (`""` for a
  level-less scene/ground). Update its doc comment.
- Modify: `src/server/src/scene/mod.rs` `navmesh_for` (line ~2356) — add a parameter `level: &str`;
  build the key as `(scene, quantized, level.to_string(), wall_set_key(walls))`. Update the doc
  comment: "the level id is part of the key so two levels with coincidentally identical wall
  geometry never share a cache entry — the never-fork pin extends to cache identity, not just the
  predicate."
- Modify: `src/server/src/scene/mod.rs` `pathfind`'s ONE `navmesh_for` call site (line ~2604) —
  resolve the mover's level via `elevation::level_of(&scene_levels, elevation)` (decode the
  scene's `SceneEngine.levels` once, alongside `settings`/`grid_shape` at the top of `pathfind`)
  and pass `.map(|l| l.id.as_str()).unwrap_or("")`.
- Modify: `src/server/src/scene/tests/{pathfind_and_vision.rs,resolution_and_lighting.rs}` — every
  `navmesh_for(scene, radius, walls)` call (`rg "navmesh_for\(" src/server/src/scene/tests`) gains
  a `""` (or a specific level id, for the new level-distinguishing test) as the third positional
  argument, becoming `navmesh_for(scene, radius, "", walls)`.

**Tests:** add to `resolution_and_lighting.rs` — `navmesh_for(scene, r, "l1", walls)` and
`navmesh_for(scene, r, "l2", walls)` over the IDENTICAL wall slice produce cache entries that do
not collide (the second call still builds rather than returning the first's `Arc`, verified via
`Arc::ptr_eq` returning `false`, mirroring `navmesh_for_does_not_share_a_mesh_across_differing_wall_sets`'s
existing assertion shape but varying `level` instead of `walls`).

- [ ] **Step 1:** failing test; implement.
- [ ] **Step 2:** `cargo test --all`, clippy, fmt PASS.
- [ ] **Step 3:** `git commit -m "feat(scene): navmesh cache keyed by level" -- src/server/src/scene/mod.rs src/server/src/scene/tests/`

## Task 5: vision, lighting and footprints become level-scoped

Depends on Task 1.

**Files:**
- Modify: `src/server/src/scene/mod.rs` `SightSources`/`SightSource` — no structural change needed
  (a source's `elevation: f64` is already carried); add a new accessor:
  ```rust
  /// Every source's committed LOS polygon paired with its resolved level id (`level_of` over
  /// `levels`, `""` for ground/level-less) — the level tag `player_vision_polygons` attaches to
  /// each polygon.
  pub(crate) fn polygons_with_level(&self, levels: &[eng::SceneLevel]) -> Vec<(String, Vec<vision::P>)> {
      self.sources
          .iter()
          .map(|s| {
              let level = elevation::level_of(levels, s.elevation)
                  .map(|l| l.id.clone())
                  .unwrap_or_default();
              (level, s.poly.clone())
          })
          .collect()
  }
  ```
- Modify: `src/server/src/scene/mod.rs` `player_vision_polygons` (line ~2124) — decode each
  scene's `SceneEngine.levels` (via `engine_as_cached::<eng::SceneEngine>`, empty `Vec` when
  absent/undecodable) and call `sight.polygons_with_level(&levels)` instead of `sight.polygons()`;
  the return type becomes `Vec<(Uuid, String, Vec<vision::P>)>` (`scene, level, points`). Update
  the ONE caller in `compute_derived`'s `"vision"` arm (line ~4590) to destructure the triple and
  emit `serde_json::json!({ "scene": scene, "level": level, "points": points })` — `level`
  serializes as an empty string for ground, matching `SceneSubscribe.level`'s `None` ⇒ implicit
  ground convention (Task 6); do NOT emit `null` here (the client's `levelOf`/`bandContains`
  mirrors treat `""` as the ground/default level id, never `null`, so the two sides agree on one
  spelling — state this explicitly in the field's doc comment).
- Modify: `src/server/src/scene/mod.rs` `player_lit_mask` (line ~3702) — the accumulation map
  `per_scene: BTreeMap<Uuid, (f64, CellEntry)>` becomes keyed by `(Uuid, String)` (scene, level
  id, `""` = ground): decode `scene.levels` once per scene alongside `scene_settings` (a second
  `HashMap<Uuid, Vec<eng::SceneLevel>>` built in the same first pass), and inside the per-source
  loop compute `let level = elevation::level_of(&scene_levels, src.elevation).map(|l|
  l.id.clone()).unwrap_or_default();` then key `per_scene.entry((scene, level))` instead of
  `per_scene.entry(scene)`. `LitScene` gains a `level: String` field (doc: "the level this
  entry's cells belong to; `\"\"` = ground/level-less"); the final assembly loop (building
  `Vec<LitScene>` from `per_scene`) sets it from the map key's second component.
- Modify: `src/server/src/scene/mod.rs` `lighting_inputs`/`lighting_inputs_excluding`/
  `LightingInputsSnapshot`/`lighting_inputs_cache` — add a `level: &str` parameter threaded
  through all three, plus the cache key tuple (`LightingInputsCacheKey` becomes `(Uuid, String,
  Vec<Uuid>)`); filter `lights` (and `light_walls`, already elevation-banded per-source at
  consumption but not yet filtered by LEVEL membership) to entries whose OWN `level_of(scene.levels,
  light.elevation)` equals `level` before the photometric raycast — a light on another floor
  contributes nothing to this level's field (spec §2 "a light contributes only to the level
  `level_of(light.elevation)` resolves to"). Update `player_lit_mask`'s call site (was
  `self.lighting_inputs(scene, settings, cell)`) to pass the per-source `level` computed above:
  `self.lighting_inputs(scene, &level, settings, cell)`.
- Modify: `src/server/src/scene/mod.rs` `compute_derived`'s `"vision"` arm — the `lit` array
  entries already carry `scene`/`cell`/`cells`; add `"level": s.level` to each `LitScene`'s JSON
  object.
- Modify: `src/server/src/scene/footprint.rs` `TokenFootprint` — add `#[serde(default)] pub level:
  Option<String>,` (doc: "this token's resolved level id, `None` for ground/a level-less scene —
  lets the client scope by level without re-deriving it from elevation").
- Modify: `src/server/src/scene/mod.rs` `resolved_footprints` (line ~3300) — the per-token
  construction site (line ~3356-3372): decode the scene's `levels` once per scene (reuse the
  `by_scene` map's existing per-scene loop to also stash `Vec<eng::SceneLevel>` alongside `cell`),
  decode the token's `TokenEngine.elevation` (already need `token_shape_and_size`'s join; add a
  direct `engine_as_cached::<eng::TokenEngine>` read for `elevation`), compute `level =
  elevation::level_of(&scene_levels, elevation::elevation_or_ground(token_eng.elevation)).map(|l|
  l.id.clone())`, and push `footprint::TokenFootprint { token, extent, level }`.
- Modify: `src/server/src/ws/move_clip.rs` — find the per-recipient clip predicate that already
  gates a mover by sight (`disc_touches_los`/`sees_at`-family); add the level conjunct: read the
  MOVED TOKEN's resolved level once per clip (`level_of` over the scene's decoded levels at the
  mover's CURRENT elevation, or per-sample elevation if the move changes floors mid-walk — read
  `RecipientSight`/`ClipInputs`'s existing per-sample elevation plumbing before deciding which),
  and the RECIPIENT's own level from their own primary vision source's elevation (reuse
  `RecipientSight.sensed`'s elevation, or add a `recipient_level: String` field threaded in from
  the caller alongside `sensed`); a sample is admitted only when the two match, mirroring "a mover
  on another level is clipped like a mover out of sight" — the SAME predicate the existing
  distance/LOS gate already applies, not a second door. Read `src/server/src/ws/move_clip.rs`
  fully before editing: pin the EXACT struct/function names via `grep -n "struct ClipInputs\|fn
  clip_frame\|fn sees_at\|recipient_sight\|sensed" src/server/src/ws/move_clip.rs` and thread the
  level conjunct onto the identical boolean expression that already ANDs in the LOS/distance
  checks — never a parallel `if` that could disagree with it.

**Tests:**
- `player_vision_polygons` tags a level-1 token's polygon with `"l1"` and a level-2 token's with
  `"l2"` on the same scene.
- `player_lit_mask` returns SEPARATE `LitScene` entries per level for one scene with sources on
  two different floors; a light on floor 1 contributes zero illumination to floor 2's entry (a
  cell that would be lit if the light leaked is NOT in the floor-2 mask).
- `resolved_footprints`'s `TokenFootprint.level` matches `level_of` for a token's stored elevation.
- `move_clip` clips a mover moving between floors: a sample on the mover's OWN floor stays
  visible to a same-floor recipient; the identical geometric sample on a DIFFERENT floor is
  clipped for that recipient even when the raw LOS polygon would otherwise contain it (a
  same-scene, cross-level "ghost" case only reachable via a raw geometric coincidence, which the
  level conjunct must still refuse).

- [ ] **Step 1:** failing tests; implement.
- [ ] **Step 2:** `cargo test --all` (background + log), `cargo clippy --all-targets -- -D
  warnings`, `cargo fmt --check`, `pnpm docs:check-rust-examples` PASS.
- [ ] **Step 3:** `git commit -m "feat(scene): vision, lighting and footprints scope by level" -- src/server/src/scene/ src/server/src/ws/move_clip.rs"`

## Task 6: explored fog keyed by level

Depends on Task 1, Task 5 (the `LitScene.level` field this task's accumulation reads).

**Files:**
- Modify: `src/server/migrations/0001_init.sql` — edit `explored_fog` IN PLACE. This is the
  project's iron-clad rule (no data migrations pre-customers): `0001_init.sql` is the single
  baseline and is edited directly; NEVER add a numbered migration file, and any that appears is
  deleted on sight. A dev DB predating the edit fails the sqlx checksum — delete the dev DB file
  and restart. The new table shape:
  ```sql
  CREATE TABLE explored_fog (
    world_id  TEXT NOT NULL,
    scene_id  TEXT NOT NULL,
    level_id  TEXT NOT NULL DEFAULT '',
    user_id   TEXT NOT NULL,
    cells     BLOB NOT NULL,
    PRIMARY KEY (scene_id, level_id, user_id)
  );
  ```
- Modify: `src/server/src/data/repository.rs` `Repository` trait — `get_explored`/`set_explored`
  gain a `level: &str` parameter each, inserted after `scene`:
  `async fn get_explored(&self, scene: Uuid, level: &str, user: Uuid) -> Result<Option<Vec<u8>>, DataError>;`
  and `set_explored(&self, world: Uuid, scene: Uuid, level: &str, user: Uuid, cells: &[u8]) ->
  Result<(), DataError>` (update both doc examples to pass `""`).
- Modify: `src/server/src/data/sqlite/worlds.rs` `get_explored`/`set_explored` — add the `level`
  parameter, bind it into the `WHERE level_id = ?` / `INSERT ... level_id ...` clauses (update the
  `ON CONFLICT` target to `(scene_id, level_id, user_id)`). Update both doc examples.
- Modify: `src/server/src/data/sqlite.rs` (line ~2479) — the trait-impl forwarder gains `level:
  &str` and forwards it.
- Modify every call site (`rg "\.get_explored\(|\.set_explored\(" src/server/src`): production
  sites `src/server/src/ws/conn.rs` (3 sites: ~1101, ~1360, ~1366 — see below for the exact level
  value each passes), `src/server/src/ws/room.rs` (3 sites: ~1045, ~1254, ~1414 — pass `""` unless
  the site is specifically resolving a level-scoped explored set for the `Revealed` movement-gate
  union, in which case pass the MOVER's own resolved level id); test sites in
  `src/server/src/data/sqlite/tests/rows_and_validation.rs`, `search_and_worlds.rs`,
  `src/server/src/ws/conn/tests/mod.rs`, `src/server/src/ws/room/tests/mod.rs` (including that
  file's mock `Repository` impl at line ~192) all gain a `""` (or a specific level id for a new
  level-scoping test) as the third positional argument.
- Modify: `src/server/src/ws/protocol.rs` `ClientMsg::SceneSubscribe` — add:
  ```rust
  /// The level to scope explored-fog accumulation/emission to (`None` ⇒ implicit ground, `""`
  /// on the wire's internal representation — see `enrich_vision_explored`). Consulted only for
  /// the `"vision"` channel; ignored by every other channel.
  #[serde(default)]
  #[ts(optional)]
  level: Option<String>,
  ```
- Modify: `src/server/src/ws/conn.rs` — `Egress::SceneSubscribe` gains `level: Option<String>`;
  the `ClientMsg::SceneSubscribe { request_id, channel, as_user }` destructure that builds it
  (line ~509) gains `level`, forwarded into the `Egress::SceneSubscribe { request_id, channel,
  as_user, level }` construction. `struct SceneSub` (line ~130) gains `level: String` (normalize
  `Option<String>` → `level.unwrap_or_default()` at insertion, line ~1874's match arm). BOTH
  `enrich_vision_explored` call sites (line ~1918, ~2145) pass `&sub.level`/the resolved `level`
  variable as a new final argument.
- Modify: `src/server/src/ws/conn.rs` `enrich_vision_explored` (line ~1297) — add a `level: &str`
  parameter (the CONNECTION'S requested viewed level, not the source token's — see below); change
  `by_scene: HashMap<Uuid, Vec<(i32,i32)>>` to `HashMap<(Uuid, String), Vec<(i32,i32)>>` keyed by
  `(scene, group.level)` reading the NEW `"level"` field the `"lit"` groups now carry (Task 5);
  accumulate (`mark_cells`) into EVERY `(scene, level)` key present (a player exploring floor 2
  marks floor-2 memory even while viewing floor 1, if they have a source there); but only push
  onto `explored_out` — and therefore only emit to the client — the entries whose level equals the
  requested `level` parameter (default `""`). Update `get_explored`/`set_explored` calls inside
  this function to pass the per-key level. Update the function's doc comment: "Explored is
  accumulated into the level of its SOURCE TOKEN and emitted for the recipient's VIEWED level
  only — a floor a player cannot currently see still remembers what THAT floor's tokens saw, but
  the wire payload never restates a floor the client is not rendering."
- Modify: `src/client/core/src/wire.ts` — no Zod mirror exists for `ClientMsg`/`SceneSubscribe`
  today (confirmed: `wire.ts` mirrors only documents/`WireCommand`/`WireWelcome`, never the
  outbound `ClientMsg` union) — SKIP this file; `ws-client.ts`'s `subscribeScene` sends a plain
  object literal (Task 12 wires the `level` option there).

**Tests:**
- `get_explored`/`set_explored` round-trip per `(scene, level, user)` — two levels of the same
  scene keep independent memory.
- `enrich_vision_explored` accumulates a floor-2 source's visible cells into `(scene, "l2")`
  memory while the connection's requested `level` is `"l1"`, and the emitted `explored` array
  contains ONLY the `"l1"` entry (or is empty if the recipient has no `"l1"` source) — the floor-2
  accumulation happened but was not sent.

- [ ] **Step 1:** failing tests; implement.
- [ ] **Step 2:** `cargo test --all` (background + log), clippy, fmt PASS.
- [ ] **Step 3:** `git commit -m "feat(ws): explored fog is keyed by (scene, level, user)" -- src/server/migrations/ src/server/src/data/ src/server/src/ws/`

## Task 7: `TriggerEffect::Teleport` + `PortalTarget`

Depends on Task 1.

**Files:**
- Modify: `src/types/index.ts` — add `export type { PortalTarget } from
  "./generated/engine/PortalTarget";` beside the `TriggerEffect` export (hand-maintained barrel).
- Modify: `src/server/src/data/engine/geometry.rs` — append a variant to `TriggerEffect` (internally
  tagged, `type = "teleport"` per the existing `rename_all = "snake_case"` convention):
  ```rust
  /// Move the entering token to `target` — within the same scene (`target.scene: None`) or to
  /// another scene entirely. Fires from `Enter` only; see `ws::room::Room::fire_region_triggers`
  /// for application (same-scene Update vs cross-scene Move+Update) and the one-hop anti-loop
  /// (a destination's OWN `Enter` effects fire except another `Teleport`).
  Teleport {
      /// Where to send the token.
      target: PortalTarget,
  },
  ```
  Add `PortalTarget` beside `TriggerEffect`:
  ```rust
  /// A teleport's destination. `scene: None` = the same scene the portal fired in.
  ///
  /// # Examples
  ///
  /// ```
  /// use shadowcat::data::engine::PortalTarget;
  ///
  /// let target = PortalTarget { scene: None, x: 10.0, y: 10.0, elevation: None, vfx: None };
  /// assert!(target.scene.is_none());
  /// ```
  #[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
  #[ts(export, export_to = "../../types/generated/engine/")]
  #[serde(deny_unknown_fields)]
  pub struct PortalTarget {
      /// Destination scene id; `None` = the portal's own scene.
      #[serde(default)]
      pub scene: Option<Uuid>,
      /// Destination x, destination scene units.
      pub x: f64,
      /// Destination y, destination scene units.
      pub y: f64,
      /// Destination elevation; `None` leaves the token's current elevation unchanged.
      #[serde(default)]
      pub elevation: Option<f64>,
      /// Asset id of a VFX played at BOTH ends on teleport (source + destination); validated
      /// and carried here; PLAYED once `ws::room` broadcasts it through `ServerMsg::Vfx`
      /// (a validated id with no broadcaster is stored, never dropped).
      #[serde(default)]
      pub vfx: Option<String>,
  }

  impl PortalTarget {
      /// Ingress validation: `x`/`y` finite and `MAX_GATE_WALK_COORD`-bounded (the same bound the
      /// movement gate and `TokenEngine::validate` enforce — a teleport must never place a token
      /// past the coordinate ceiling every other write path already refuses), `elevation` finite
      /// when present, `vfx` non-empty when present.
      pub(crate) fn validate(&self) -> Result<(), String> {
          let bound = crate::scene::move_exec::MAX_GATE_WALK_COORD;
          for (name, v) in [("x", self.x), ("y", self.y)] {
              if !v.is_finite() {
                  return Err(format!("{name} must be finite"));
              }
          }
          if self.x.abs() > bound || self.y.abs() > bound {
              return Err(format!("teleport target exceeds coordinate bound {bound}"));
          }
          if let Some(e) = self.elevation {
              if !e.is_finite() {
                  return Err("elevation must be finite".to_string());
              }
          }
          if self.vfx.as_deref() == Some("") {
              return Err("vfx must be non-empty when present".to_string());
          }
          Ok(())
      }
  }
  ```
- Modify: `src/server/src/data/engine/mod.rs` — export `PortalTarget` from the `pub use
  geometry::{...}` list.
- Modify: `src/server/src/data/engine/geometry.rs` `RegionEngine::validate` — add a
  `TriggerEffect::Teleport { target }` arm calling `target.validate()`.

**Tests** (in `src/server/src/data/engine/tests.rs` or a new sibling): `PortalTarget::validate`
rejects a non-finite/over-bound coordinate, a non-finite elevation, an empty `vfx`; accepts a
minimal same-scene target; `RegionEngine::validate` rejects a region whose trigger carries an
invalid `PortalTarget`.

- [ ] **Step 1:** failing tests; implement.
- [ ] **Step 2:** `cargo test --all` (background + log — regenerates `TriggerEffect.ts`/`PortalTarget.ts`),
  clippy, fmt PASS; stage regenerated types.
- [ ] **Step 3:** `git commit -m "feat(engine): TriggerEffect::Teleport + PortalTarget" -- src/server/src/data/engine/ src/types/"`

## Task 8: `WriteOrigin::Trigger` + the `apply_intent` Move-arm fix

Depends on nothing in this milestone (pure `data::command`/`data::sqlite` change) but is dispatched
after Task 7 so the coder has the full portal picture in context.

**Files:**
- Modify: `src/server/src/data/command.rs` `WriteOrigin` enum — append (master §2.7: "`WriteOrigin`
  gains two variants: `AudioTransport` (M23) and `Trigger` (M25); both append" — M23 may or may not
  have landed its variant yet in this worktree; append AFTER whatever is currently last, never
  reordering):
  ```rust
  /// Server-authored region-trigger write (condition/resource/chat-notice/teleport effects
  /// applied by `ws::room::Room::fire_region_triggers`): per-op capability gates are skipped;
  /// scope, size, engine, containment, singleton, schema and OCC checks all run; never derivable
  /// from a wire frame. Every trigger effect (`ConditionAdd`/`ConditionRemove`/`ResourceDelta`/
  /// `ChatNotice`/`Teleport`) commits under this origin; `CombatTransition` belongs to combat
  /// state transitions alone.
  Trigger,
  ```
  Add it to BOTH `is_server_authored()`'s and `skips_capability_gates()`'s `matches!` lists.
- Modify: `src/server/src/data/sqlite.rs` line ~1014 (the `Operation::Move` arm's capability
  check) — `if origin != WriteOrigin::CombatTransition {` becomes `if
  !origin.skips_capability_gates() {` (the shape every other gate site in `apply_intent` already
  uses — the doc comment immediately above this `if` already says as much; update it to drop the
  now-stale "CombatTransition skips this capability gate" phrasing in favor of "any
  capability-skipping origin (`skips_capability_gates`) does").
- Modify: `src/server/src/ws/room.rs` — every trigger-effect commit currently tagged
  `WriteOrigin::CombatTransition` INSIDE `fire_region_triggers` (line ~2074, the function's final
  `commit_ops_locked` call) becomes `WriteOrigin::Trigger`. Do NOT touch the `CombatTransition`
  sites that are genuinely combat-lifecycle (line ~1159, ~1636 — the resource-decrement/turn-clock
  commits, unrelated to region triggers).

**Tests:**
- `WriteOrigin::Trigger.skips_capability_gates()` and `.is_server_authored()` both `true`.
- `apply_intent`'s `Operation::Move` arm accepts a `WriteOrigin::Trigger`-authored move of a
  PLAYER-OWNED token (no GM role, no `all` grant) — the exact regression this fix targets (spec:
  "or a player-owned token could never be teleported across scenes").
- The existing M18 trigger-effect tests (`ConditionAdd`/`ResourceDelta`/`ChatNotice` fired via
  `fire_region_triggers`) still pass with the origin now `Trigger` — update any test asserting the
  literal `WriteOrigin::CombatTransition` on a region-trigger-authored `Command` to expect
  `WriteOrigin::Trigger` instead (`rg "CombatTransition" src/server/src/ws/room/tests` to find
  them).

- [ ] **Step 1:** failing tests; implement.
- [ ] **Step 2:** `cargo test --all` (background + log), clippy, fmt PASS.
- [ ] **Step 3:** `git commit -m "feat(data): WriteOrigin::Trigger; a player-owned token can be moved by a trusted trigger write" -- src/server/src/data/command.rs src/server/src/data/sqlite.rs src/server/src/ws/room.rs"`

## Task 9: `fire_region_triggers` applies `Teleport`

Depends on Task 3 (banded `trigger_regions`), Task 7 (`TriggerEffect::Teleport`/`PortalTarget`),
Task 8 (`WriteOrigin::Trigger`).

**Files:**
- Modify: `src/server/src/ws/room.rs` `fire_region_triggers` (line ~1759) — add a teleport branch
  alongside the existing condition/resource/chat-notice sections, following the SAME shape (a
  `needs_X` scan over `fired`, building `ops`/`failures`), inserted after the resource-effects
  block and before the chat-notices loop (so a chat notice about a FAILED teleport still posts in
  the same batch):
  ```rust
  // --- Teleport: at most ONE per token per fire (a region's own trigger list could in
  // principle carry several teleports; only the FIRST wins — a second is silently ignored
  // rather than double-moving the token, mirroring "one hop per move"). ---
  if let Some((_, trigger)) = fired.iter().find(|(_, t)| matches!(t.effect, eng::TriggerEffect::Teleport { .. })) {
      let eng::TriggerEffect::Teleport { target } = &trigger.effect else { unreachable!() };
      let dest_scene = target.scene.unwrap_or(scene);
      // Scene existence is checked here (fire time), never at ingress (`validate_engine_tree`
      // is pure — no repository access).
      let scene_exists = match repo.get_document(dest_scene).await {
          Ok(Some(doc)) => doc.doc_type == "scene",
          _ => false,
      };
      if !scene_exists {
          failures.push(format!("teleport target scene {dest_scene} does not exist"));
      } else {
          let old_x = token_eng.as_ref().map(|t| t.x);
          let old_y = token_eng.as_ref().map(|t| t.y);
          let old_elevation = token_eng.as_ref().and_then(|t| t.elevation);
          let mut update_changes = Vec::new();
          if let (Some(ox), Some(oy)) = (old_x, old_y) {
              update_changes.push(crate::data::command::FieldChange {
                  path: "/engine/x".to_string(),
                  old: serde_json::json!(ox),
                  new: serde_json::json!(target.x),
              });
              update_changes.push(crate::data::command::FieldChange {
                  path: "/engine/y".to_string(),
                  old: serde_json::json!(oy),
                  new: serde_json::json!(target.y),
              });
          }
          if let Some(new_elev) = target.elevation {
              update_changes.push(crate::data::command::FieldChange {
                  path: "/engine/elevation".to_string(),
                  old: serde_json::json!(old_elevation),
                  new: serde_json::json!(new_elev),
              });
          }
          if dest_scene != scene {
              ops.push(Operation::Move {
                  doc_id: token,
                  parent_id: Some(dest_scene),
                  old_parent_id: Some(scene),
              });
          }
          if !update_changes.is_empty() {
              ops.push(Operation::Update { doc_id: token, changes: update_changes });
          }
          // Active-combat visibility: the budget decrement for the walk that entered the
          // portal already happened; this notice only makes the scene-change visible.
          if let Some((_, ce)) = { self.scene.read().await.active_combat_for_scene(scene) } {
              let _ = ce; // engine unused beyond the Some(..) match; the notice names the token
              let notice = build_message_doc(
                  self.world_id, ctx.user_id,
                  MessageDraft {
                      channel: "region".to_string(), actor_owner: None,
                      audience: Audience::GmOnly, kind: MessageKind::System,
                      content: vec![Segment::Text {
                          text: format!("Token {token} teleported off scene {scene} while an active combat is running there"),
                      }],
                      source: None,
                  },
                  ts,
              );
              ops.push(Operation::Create { doc: notice });
          }
      }
  }
  ```
  This branch reads `token_eng` (already decoded earlier in the function) — if `token_eng` is
  `None` (an actorless/malformed token document), push a failure instead of the teleport ops
  (`"token has no engine body to teleport"`).
- Modify: `src/server/src/ws/room.rs` `fire_placement_triggers` (line ~2100, the destination-Enter
  re-fire site per spec's "destination cells fire Enter effects EXCEPT Teleport") — after a
  successful teleport commits (this happens inside `fire_region_triggers` itself, so the
  ONE-HOP rule must be enforced THERE, not by the caller): add a parameter `allow_teleport: bool`
  to `fire_region_triggers`, defaulting the TOP-level call (from `Room::execute_move`/`Room::publish`,
  wherever `fire_region_triggers`/`fire_placement_triggers` are invoked for a genuine player move
  or placement — `rg "fire_region_triggers\(|fire_placement_triggers\(" src/server/src/ws/room.rs`
  to find both call sites) to `true`; inside this task's new Teleport branch, guard the whole
  block with `if allow_teleport { ... } else { failures.push("chained portal refused (one hop per move)".to_string()); }`.
  After a successful teleport (the `ops` list gained the Move/Update), the SAME function must
  re-fire `Enter` on the destination cells EXCEPT teleport: after `commit_ops_locked` succeeds at
  the end of this function, if a teleport was applied, compute the token's NEW footprint cells on
  `dest_scene` (mirroring `fire_placement_triggers`'s own cell computation) and recursively call
  `self.fire_region_triggers(repo, ctx, TriggerReport { scene: dest_scene, token, entered:
  new_cells, arrest_stop: None }, ts_for_the_recursive_call)` with `allow_teleport: false` —
  implement this as the LAST step of `fire_region_triggers`, after its own `commit_ops_locked`
  call, guarded on `teleported: Option<Uuid>` (the destination scene id) captured from the branch
  above.
- Modify: `src/server/src/ws/room.rs` `TriggerReport`/`fire_region_triggers`'s signature — add the
  `allow_teleport: bool` parameter (or fold it into `TriggerReport` itself as a new field,
  whichever keeps every call site's diff smaller — read both call sites first via the `rg` above
  and choose the shape that touches fewer of them; document the choice in the function's own doc
  comment either way).

**Tests** (new module `src/server/src/ws/room/tests/teleport.rs` or appended to the existing
trigger-effect test file — check `rg "fn.*region_trigger\|fn.*fire_region" src/server/src/ws/room/tests`
for the existing home and follow its fixture-building precedent):
- Same-scene teleport: an `Operation::Update` on `/engine/x`/`/engine/y` (+ `/engine/elevation`
  when the target names one) lands, no `Operation::Move`.
- Cross-scene teleport: BOTH an `Operation::Move` (reparenting) and an `Operation::Update`
  (position) land in ONE committed `Command` (one `commit_ops_locked` call, one seq).
- Missing target scene: no move/update ops commit; a GM-only chat notice posts naming the failure.
- Chained portal: a destination region ALSO carrying a `Teleport` trigger does NOT fire (the
  token's second `Teleport` effect is refused with a GM-only notice, and the token ends on the
  FIRST destination, not a third scene).
- Destination `Enter` effects (a `ConditionAdd` on the destination region) DO fire after a
  teleport, alongside the position write, in the SAME overall trigger-firing pass.
- Every op `fire_region_triggers` commits (teleport, condition, resource, chat-notice) carries
  `WriteOrigin::Trigger` on its `Command` (assert via the returned/logged `Command.author`... —
  actually `WriteOrigin` is not stored on `Command`; assert instead via a test-only `Repository`
  spy capturing the `origin` argument `apply_intent` received, mirroring however the existing
  M18 trigger tests already assert the origin today — `rg "WriteOrigin::CombatTransition"
  src/server/src/ws/room/tests` to find that existing assertion shape and mirror it for
  `WriteOrigin::Trigger`).
- Active combat on the source scene: teleporting a combatant off it posts the GM-only notice
  naming the token; the combatant record (queried via `active_combat_for_scene` afterward) is
  unaffected (still exists, still active, unbudgeted on the destination scene — assert no
  additional combat-document write occurred beyond the notice).

- [ ] **Step 1:** failing tests; implement.
- [ ] **Step 2:** `cargo test --all` (background + log), clippy, fmt PASS.
- [ ] **Step 3:** `git commit -m "feat(ws): region triggers can teleport a token within or across scenes" -- src/server/src/ws/room.rs src/server/src/ws/room/tests/"`

## Task 10: client core `levels.ts` consumers regen check + `sceneScopedDocs` gains `viewedLevel`

Depends on Task 1 (client `levels.ts` already created in Task 1 — this task is the RENDER-LAYER
wiring, separate package).

**Files:**
- Modify: `src/client/render/src/scene-scope.ts` — `sceneScopedDocs` gains a FOURTH parameter
  `viewedLevel: () => string | null`:
  ```ts
  import { bandContains, levelOf, type SceneEngine } from "@shadowcat/core";

  const BAND_SHAPED_DOC_TYPES = new Set(["wall", "region", "drawing", "template"]);

  export function sceneScopedDocs(
    store: ReadableDocuments,
    docType: string,
    viewedSceneId: () => string | null,
    viewedLevel: () => string | null = () => null,
  ): WireDocument[] {
    const vsid = viewedSceneId();
    const docs = store.query(docType);
    const sceneScoped = vsid === null ? docs : docs.filter((d) => d.parent_id === vsid);
    const level = viewedLevel();
    if (level === null) return sceneScoped;
    const sceneDoc = vsid === null ? undefined : store.query("scene").find((s) => s.id === vsid);
    const levels = ((sceneDoc?.engine as SceneEngine | undefined)?.levels ?? []);
    if (levels.length === 0) return sceneScoped;
    const target = levels.find((l) => l.id === level);
    if (!target) return sceneScoped;
    if (BAND_SHAPED_DOC_TYPES.has(docType)) {
      return sceneScoped.filter((d) => {
        const eng = d.engine as { elevation?: { bottom: number | null; top: number | null } | null } | undefined;
        return bandContains(eng?.elevation ?? null, target.bottom);
      });
    }
    return sceneScoped.filter((d) => {
      const eng = d.engine as { elevation?: number | null } | undefined;
      const resolved = levelOf(levels, eng?.elevation ?? 0);
      return resolved?.id === level;
    });
  }
  ```
  Update the function's doc comment: the fourth argument, the band-vs-point dispatch by
  `docType`, and the `null`-level (level-less scene / no scene, or explicitly viewing "every
  level") passthrough behavior — preserving zero behavior change for a scene whose `levels` is
  empty (the overwhelming majority of existing scenes).
- Modify: `src/client/render/src/{wall-view,region-view,drawing-view,template-view,light-view,token-view}.ts`
  — each constructor gains a fourth parameter `viewedLevel: () => string | null = () => null`
  (mirroring `viewedSceneId`'s existing optional-with-default shape exactly); each `reconcile()`'s
  `sceneScopedDocs(this.store, "<docType>", this.viewedSceneId)` call gains `, this.viewedLevel`.
  Update each class's doc comments identically to how `viewedSceneId` is already documented there.

**Tests:** extend `src/client/render/src/scene-scope.test.ts` — a wall banded to `[0,10)` is
absent when `viewedLevel` resolves to a level `[10,20)` and present for `[0,10)`; a token at
elevation `15` is absent from level `[0,10)` and present in `[10,20)`; `viewedLevel: () => null`
preserves today's behavior exactly (existing test cases keep passing unmodified). Extend each of
`wall-view.test.ts`/`region-view.test.ts`/`drawing-view.test.ts`/`template-view.test.ts`/
`light-view.test.ts`/`token-view.test.ts` with one case: constructing the view with a
`viewedLevel` function scopes `reconcile()`'s output to that level (a doc on another level never
reaches `backend.setShape`/`setToken`).

- [ ] **Step 1:** failing tests; implement.
- [ ] **Step 2:** `pnpm --filter @shadowcat/render test`, `pnpm -r typecheck`, `pnpm lint:docs`,
  `pnpm lint:props` PASS.
- [ ] **Step 3:** `git commit -m "feat(render): scene-scoped views additionally scope by viewed level" -- src/client/render/src/"`

## Task 11: `AppContext.viewedLevel` / `setViewedLevel`

Depends on Task 10 (the `SceneLevel`/`levelOf` client types it reads).

**Files:**
- Modify: `src/client/ui-kit/src/appContext.ts` — append AFTER `panels` (master §3 convention:
  "each milestone owns exactly the member §2 names"):
  ```ts
  /** The level (of the viewed scene) this client renders/subscribes to; `null` for a
   * level-less scene, or before any scene/level is known. For a player: `levelOf` of their
   * primary token, tracked live (follows the token through a portal). For a GM: the last
   * chosen level for the viewed scene (persisted, `ui_state.worlds[id].viewedLevel`). */
  viewedLevel: string | null;
  /** Set the viewed level (GM local override; a no-op/warn for a player — mirrors
   * `setGmViewedScene`'s role gate). `WorldSession` re-subscribes the `"vision"` channel with
   * the new level. */
  setViewedLevel: (id: string | null) => void;
  ```
- Modify: `src/client/ui-kit/src/__fixtures__/appContextTest.ts` — add defaults:
  `viewedLevel: over.viewedLevel ?? null, setViewedLevel: over.setViewedLevel ?? (() => {}),`.
- Modify every OTHER `setAppContext(`/`: AppContext =` literal site: `rg "setAppContext\(|:
  AppContext =" src --type ts --type svelte -l` (excluding `node_modules`) — add both fields to
  each. Expect hits in `Stage.test.ts`, `SceneBrowserPanel.test.ts`, `ToolRail.test.ts`, and any
  other module test currently constructing a literal `AppContext`.

**Tests:** `appContextTest.ts`'s own smoke test (if one exists — `rg
"describe.*appContextTest\|appContextTest.test" src/client/ui-kit/src`) covers the new defaults
resolve without throwing; otherwise this task adds none beyond the sweep above (the member is a
plain interface addition, exercised by Task 12's implementation).

- [ ] **Step 1:** implement the type + fixture default; sweep every literal site (compiler-driven —
  `pnpm -r typecheck` will fail on every missing member; fix until clean).
- [ ] **Step 2:** `pnpm -r typecheck`, `pnpm --filter @shadowcat/ui-kit test`, `pnpm lint:docs`,
  `pnpm lint:props` PASS.
- [ ] **Step 3:** `git commit -m "feat(ui-kit): AppContext gains viewedLevel/setViewedLevel" -- src/client/ui-kit/ src/modules/"`

## Task 12: `WorldSession.viewedLevel` + re-subscribe + persistence

Depends on Task 11, Task 6 (`SceneSubscribe.level`).

**Files:**
- Modify: `src/client/shell/src/lib/sessionState.svelte.ts` — add to `WorldKey`'s backing type
  (`./api.ts`'s `UiState["worlds"][string]`, see below) and this file's dirty-tracking:
  `case "viewedLevel": slice.viewedLevel = w.viewedLevel; break;` in `copyWorldKey`'s switch; new
  functions mirroring `getPanelLayout`/`setPanelLayout` exactly:
  ```ts
  /** Reads a scene's persisted GM-viewed-level id within a world, or `null` if unset.
   * @param world - World id.
   * @param scene - Scene id.
   * @returns The stored level id, or `null`. */
  export function getViewedLevel(world: string, scene: string): string | null {
    return state.worlds[world]?.viewedLevel?.[scene] ?? null;
  }

  /** Stores a scene's GM-viewed-level id within a world, marks it dirty, and schedules a
   * persist. Creates the world's entry (and its `viewedLevel` map) if absent.
   * @param world - World id.
   * @param scene - Scene id.
   * @param level - The level id to remember, or `null` to clear it. */
  export function setViewedLevel(world: string, scene: string, level: string | null): void {
    const w = (state.worlds[world] ??= {});
    const map = (w.viewedLevel ??= {});
    if (level === null) delete map[scene];
    else map[scene] = level;
    markWorldDirty(world, "viewedLevel");
    schedulePersist();
  }
  ```
- Modify: `src/client/shell/src/lib/api.ts` `UiState["worlds"][string]` — add
  `/** GM's last-chosen level id per scene (`@shadowcat/module-stage`... — actually shell-owned).
   * Absent scene key = follow the default (first/lowest level). */
  viewedLevel?: Record<string, string>;` beside `panelLayout`/`chatRead`.
- Modify: `src/client/shell/src/lib/worldSession.svelte.ts` — add state + methods mirroring
  `#gmViewedScene`/`setGmViewedScene` exactly:
  ```ts
  /** GM per-scene viewed-level override (`sessionState`'s `viewedLevel` map), loaded lazily per
   * scene on first read. Never set for a player (they follow their primary token's level). */
  #gmViewedLevel = $state<Map<string, string | null>>(new Map());

  /** The level this client renders/subscribes to for the CURRENTLY viewed scene. A GM reads
   * `#gmViewedLevel`'s stash for that scene (seeded from `getViewedLevel` on first access);
   * a player follows `levelOf` of their primary token's elevation over the viewed scene's
   * `levels`. `null` for a level-less scene.
   * @returns The viewed level id, or `null`. */
  get viewedLevel(): string | null {
    const sceneId = this.viewedSceneId;
    if (sceneId === null) return null;
    const sceneDoc = this.#optimistic.query("scene").find((s) => s.id === sceneId);
    const levels = (sceneDoc?.engine as SceneEngine | undefined)?.levels ?? [];
    if (levels.length === 0) return null;
    if (this.role === "gm") {
      if (!this.#gmViewedLevel.has(sceneId)) {
        this.#gmViewedLevel.set(sceneId, getViewedLevel(this.world ?? "", sceneId));
      }
      const stashed = this.#gmViewedLevel.get(sceneId) ?? null;
      return levels.some((l) => l.id === stashed) ? stashed : (levels[0]?.id ?? null);
    }
    const primary = this.#primaryTokenIn(sceneId);
    const elevation = primary ? ((primary.engine as TokenEngine | undefined)?.elevation ?? 0) : 0;
    return levelOf(levels, elevation)?.id ?? null;
  }

  /** GM local viewed-level override for the current scene; ignored (warned) for a non-GM.
   * @param id - The level to view, or `null` to clear to the scene's first level.
   * @example
   * ```
   * declare const session: WorldSession;
   * session.setViewedLevel("l2"); // GM only; no-op+warns for a player
   * ```
   */
  setViewedLevel(id: string | null): void {
    if (this.role !== "gm") {
      this.#logger.warn("setViewedLevel ignored: caller is not a GM");
      return;
    }
    const sceneId = this.viewedSceneId;
    if (sceneId === null) return;
    this.#gmViewedLevel.set(sceneId, id);
    if (this.world) setViewedLevel(this.world, sceneId, id);
  }
  ```
  Add a private `#primaryTokenIn(scene: string): WireDocument | null` helper resolving "the
  caller's own effective-owned token in `scene`" — read `ActorSelection`/`TokenSelection`'s
  existing "primary token" concept if one already exists in this file (`rg "primaryToken\|effective_owner\|ownedToken"
  src/client/shell/src/lib/worldSession.svelte.ts`); if none exists, define it as the FIRST token
  (lowest `id`, for determinism) in `scene` whose `owner` equals `this.selfId` (mirroring the
  server's `token_effective_owner`'s override-first rule at the shallow client level — an
  advisory read, never authoritative).
  Add a private `#reestablishVisionForLevel(): void` that finds the `"vision"`-channel entry
  among `#sceneSubs` and calls `#establishScene` again for it (drop+recreate the live handle) so
  the new `level` takes effect on the wire — mirror `setGmViewedScene`'s "the server is unaware of
  it, but the client must still act" framing: THIS field DOES reach the server, via
  `SceneSubscribe.level`, unlike `viewedSceneId`.
  `SceneSubRecord` gains `level?: string`; `subscribeScene`'s `opts: SubscribeSceneOpts` and the
  `#establishScene`'s `ws.subscribeScene(rec.channel, rec.onUpdate, { asUser: rec.asUser })` call
  gains `level: rec.level`. The `"vision"` channel DOES flow through this wrapper today:
  `Table.svelte` composes `subscribeScene: (c, cb, opts) => session.subscribeScene(c, cb, opts)`
  as a generic pass-through, and `RenderEngine.subscribeVision()` (called from `start()`)
  invokes `this.opts.subscribeScene("vision", cb, {...})` through it. So there is NO
  channel-name special-casing anywhere: `SubscribeSceneOpts` gains `level?: string | null`,
  `SceneSubRecord` stores it, `#establishScene` forwards it on the wire call, and the CALLER
  (`RenderEngine`, Task 13) passes `level: this.opts.viewedLevel?.() ?? undefined` and
  re-subscribes when the viewed level changes. `WorldSession` owns no `#reestablishVisionForLevel`
  method and `Table.svelte`'s composition site is untouched.
- Modify: `src/client/shell/src/lib/worldSession.test.ts` — `viewedLevel` for a level-less scene
  is `null`; for a GM, defaults to the scene's first level and persists a change via
  `setViewedLevel` (assert `getViewedLevel` reflects it after); for a player, tracks a primary
  token's elevation change (moving the token to a different band changes `viewedLevel` without
  any explicit call).

- [ ] **Step 1:** failing tests; implement (including the composition-site fix described above —
  locate it via the `rg` command given and edit it in this same commit).
- [ ] **Step 2:** `pnpm --filter @shadowcat/shell test`, `pnpm -r typecheck`, `pnpm lint:docs`,
  `pnpm lint:props` PASS.
- [ ] **Step 3:** `git commit -m "feat(shell): WorldSession.viewedLevel follows the token or the GM's last choice, persisted and re-subscribed" -- src/client/shell/"`

## Task 13: `RenderEngine` reacts to `viewedLevel`; `Stage.svelte` wires it through

Depends on Task 10, Task 12.

**Files:**
- Modify: `src/client/render/src/engine.ts` `RenderEngineOpts` — add `viewedLevel?: () => string |
  null` beside `viewedSceneId` (identical shape/doc convention). Per master §3's file-ownership
  table, M25's ONLY sanctioned touch to this file is the scene-scope wiring — this addition
  (mirroring `viewedSceneId`'s existing pass-through exactly, no new render logic) stays within
  that scope; do not add anything else here.
- Modify: `src/client/render/src/engine.ts` — thread `this.opts.viewedLevel` into every
  `TokenView`/`WallView`/`RegionView`/`DrawingView`/`TemplateView`/`LightView` CONSTRUCTOR call
  (wherever `start()` builds them — `rg "new (TokenView|WallView|RegionView|DrawingView|TemplateView|LightView)\("
  src/client/render/src/engine.ts`), as the new fourth constructor argument (default omitted ⇒
  the class's own `() => null` default from Task 10 applies).
- Modify: `src/client/render/src/engine.ts` `start()`'s `subscribeScene("vision", ...)` call
  (line ~403) — add `level: this.opts.viewedLevel?.() ?? undefined` to the third-argument options
  object (`WsSubscribeSceneOptions`, extended in this same task — see below). This IS a "level
  filter" concern distinct from the scene-scope shape filtering the ownership table restricts —
  it is a WS-subscription option, not a document filter — state this distinction in a code
  comment at the edit site so a future reviewer does not read it as violating the table's
  restriction.
- Modify: `src/client/render/src/engine.ts` — add a re-subscribe-on-level-change mechanism:
  `start()` currently subscribes ONCE; add a `setViewedLevel` no-arg re-check invoked from a new
  public method `reapplyViewedLevel(): void` that tears down `this.sceneSub` and re-subscribes
  with the CURRENT `this.opts.viewedLevel?.()`, mirroring `reapplyViewedScene`'s existing
  re-filter pattern (but this one DOES re-issue the wire subscription, since level — unlike
  scene — changes what the SERVER computes for explored fog). Call `reapplyViewedScene`'s own
  body to confirm it does NOT already re-subscribe (per this task's earlier finding) before
  adding this as a genuinely new method.
- Modify: `src/client/core/src/ws-client.ts` `WsSubscribeSceneOptions` — add `level?: string`;
  `subscribeScene`'s wire send (line ~1326) gains `...(opts.level !== undefined ? { level:
  opts.level } : {})`.
- Modify: `src/modules/stage/src/Stage.svelte` — the `RenderEngine` construction (line ~127-140)
  gains `viewedLevel: () => ctx.viewedLevel,` beside `viewedSceneId: () => ctx.viewedSceneId,`;
  add an `$effect` (alongside the existing `offViewed`-style teardown handles already present in
  the component's `$effect` block) that calls `engine?.reapplyViewedLevel()` whenever
  `ctx.viewedLevel` changes — mirror however `reapplyViewedScene` is ALREADY invoked reactively
  in this file today (`rg "reapplyViewedScene" src/modules/stage/src/Stage.svelte`) and use the
  identical reactive-tracking shape for the new call.
- Modify: `src/modules/stage/src/Stage.test.ts` — the `subscribeScene` mock assertion at line ~78
  (`toHaveBeenCalledWith("vision", expect.any(Function), undefined)`) becomes `toHaveBeenCalledWith("vision",
  expect.any(Function), { level: undefined })` (or the fixture's `viewedLevel` value, if the test
  sets one) — update to match the new options object shape.

**Tests:** `RenderEngine.test.ts` (or `engine.test.ts`) — `reapplyViewedLevel()` unsubscribes the
old `"vision"` handle and re-subscribes with the new level; `Stage.test.ts` — a `viewedLevel`
context change triggers `subscribeScene` again with the new level.

- [ ] **Step 1:** failing tests; implement.
- [ ] **Step 2:** `pnpm --filter @shadowcat/render test`, `pnpm --filter @shadowcat/module-stage
  test`, `pnpm -r typecheck`, `pnpm lint:docs` PASS.
- [ ] **Step 3:** `git commit -m "feat(render,stage): the vision subscription re-establishes on a viewed-level change" -- src/client/render/src/engine.ts src/client/core/src/ws-client.ts src/modules/stage/"`

## Task 14: `LevelSwitcher.svelte` — hosted in `Stage.svelte`'s chrome

Depends on Task 13. **Design decision (master §2.7):** `STAGE_OVERLAY_CONTRACT` is owned by M26,
which merges AFTER M25 (master §5 order: M25 is 5th, M26 is 6th) — the contract does not exist in
this worktree at M25's build time, so M25 CANNOT use it. `LevelSwitcher` is hosted directly in
`Stage.svelte`'s own chrome (plain markup, not a contract surface), matching the fact that
`Stage.svelte` is already an M25 touchpoint per master §3's file-ownership table ("M25 (level
scoping, `data-level`)") and that a level switcher must be visible to every viewer (GM and player
alike), unlike the GM-only scene-tools rail.

**Files:**
- Create: `src/modules/stage/src/LevelSwitcher.svelte` — a segmented control (one `<button>` per
  `SceneLevel`, `aria-pressed` on the active one, `data-testid="level-switcher"` on the root and
  `data-testid="level-{id}"` per button, each button's visible text is the level's `name` — no
  static `aria-label` per-item) over `props: { levels: SceneLevel[]; active: string | null; onSelect:
  (id: string) => void }`. Rendered as `null`/empty markup when `levels.length === 0` (hidden when
  the scene has no levels, per spec). Touch-sized buttons (`min-height`/`min-width` ≥ 44px per the
  project's existing touch-target convention — copy the sizing from another rail control, e.g.
  `ToolRail.svelte`'s tool buttons).
- Create: `src/modules/stage/src/LevelSwitcher.test.ts` — renders nothing for an empty `levels`
  array; renders one button per level with the active one `aria-pressed="true"`; clicking a
  button calls `onSelect` with that level's id.
- Modify: `src/modules/stage/src/Stage.svelte` — import `LevelSwitcher`; render it in the
  component's chrome (alongside wherever other stage-level UI already lives — read the file's
  template root to find the right sibling position, likely near the canvas host) with `levels={
  (ctx.documents.query("scene").find((s) => s.id === ctx.viewedSceneId)?.engine as SceneEngine |
  undefined)?.levels ?? []}`, `active={ctx.viewedLevel}`, `onSelect={(id) =>
  ctx.setViewedLevel(id)}`.
- Modify: `src/client/ui-kit/src/locales/en.ts` — new top-level group `levels`: `switcher`
  (aria-label for the switcher's root, if the root itself needs one — a `role="group"` labeled
  region is the accessible pattern for a segmented control; add `aria-label={t("levels.switcher")}`
  on the root `<div role="group">`).

- [ ] **Step 1:** failing component test; implement.
- [ ] **Step 2:** `pnpm --filter @shadowcat/module-stage test`, `pnpm -r typecheck`, `pnpm
  lint:aria-labels`, `pnpm lint:docs` PASS.
- [ ] **Step 3:** `git commit -m "feat(stage): LevelSwitcher lets any viewer pick the rendered floor" -- src/modules/stage/ src/client/ui-kit/src/locales/en.ts"`

## Task 15: `LevelsEditor.svelte` in `SceneBrowserPanel.svelte`

Depends on Task 1 (server `SceneLevel` shape), Task 11 (`AppContext.pickAsset`/`searchDocuments`
already exist — no new seam needed here).

**Files:**
- Create: `src/modules/scene-browser/src/LevelsEditor.svelte` — a list editor over
  `props: { levels: SceneLevel[]; onCommit: (next: SceneLevel[]) => void }` (the whole-array write
  pattern — M20's `TableSheet`/`rowOps` precedent: every mutation clones the array via
  `structuredClone`, mutates the clone, calls `onCommit`). Per-row controls: name (`<input>`),
  bottom/top (`<input type="number">`), background (`ctx.pickAsset({ kind: "image" })` button +
  clear), remove (`<button>`). An "add level" button appends a new row with a generated id
  (`crypto.randomUUID()` truncated, or a slug from the name — pick whichever this codebase's other
  id-generating UI already does; `rg "crypto.randomUUID" src/modules` for the precedent) and
  default band `{bottom: 0, top: 10}`. Test ids: `levels-editor`, `level-row` (+
  `data-level-id`), `level-name`, `level-bottom`, `level-top`, `level-background`,
  `level-background-clear`, `level-remove`, `level-add`.
- Create: `src/modules/scene-browser/src/LevelsEditor.test.ts` — add/remove/edit a row calls
  `onCommit` with the whole updated array; the background picker calls `ctx.pickAsset` and writes
  the returned id.
- Modify: `src/modules/scene-browser/src/SceneBrowserPanel.svelte` — add an expandable "Levels"
  section per scene row (mirroring the existing background-picker's per-scene-open-state pattern
  at `backgroundPickerFor`, e.g. a sibling `levelsEditorFor: string | null` state), rendering
  `<LevelsEditor levels={(scene.engine as SceneEngine).levels ?? []} onCommit={(next) =>
  ctx.dispatchIntent([buildUpdate(scene.id, [{ path: "/engine/levels", old: (scene.engine as
  SceneEngine).levels ?? [], value: next }])])} />` when open.
- Modify: `src/modules/scene-browser/src/SceneBrowserPanel.test.ts` — opening the levels editor
  for a scene and adding a level dispatches an Update on `/engine/levels` whose `value` includes
  the new row alongside any pre-existing ones.
- Modify: `src/client/ui-kit/src/locales/en.ts` — `levels.*` group additions: `editorTitle`,
  `addLevel`, `removeLevel`, `name`, `bottom`, `top`, `background`, `backgroundClear`.

- [ ] **Step 1:** failing tests; implement.
- [ ] **Step 2:** `pnpm --filter @shadowcat/module-scene-browser test`, `pnpm -r typecheck`, `pnpm
  lint:aria-labels`, `pnpm lint:docs` PASS.
- [ ] **Step 3:** `git commit -m "feat(scene-browser): LevelsEditor authors a scene's floors" -- src/modules/scene-browser/ src/client/ui-kit/src/locales/en.ts"`

## Task 16: scene-tools stamps the viewed level; the elevation-band editor generalizes

Depends on Task 11, Task 14.

**Files:**
- Modify: `src/modules/scene-tools/src/ToolRail.svelte` — generalize `editWallElevation` into a
  shared helper (region/drawing/template each need the identical bottom/top-pair write, tripling
  the existing duplication otherwise — the best-long-term-shape call under this milestone's own
  "no unrequested duplication" discipline):
  ```ts
  /** Write one end of an edited entity's `/engine/elevation` band, preserving the other end (the
   * shared body behind `editWallElevation`/`editRegionElevation`/`editDrawingElevation`/
   * `editTemplateElevation` — wall, region, drawing and template all carry the identical
   * `Option<ElevationBand>` shape). */
  function editElevationBand(
    old: { bottom: number | null; top: number | null } | null,
    end: "bottom" | "top",
    raw: string,
  ): { bottom: number | null; top: number | null } | null | undefined {
    const parsed = parseElevation(raw);
    if (parsed === undefined) return undefined;
    const bottom = end === "bottom" ? parsed : (old?.bottom ?? null);
    const top = end === "top" ? parsed : (old?.top ?? null);
    return bottom === null && top === null ? null : { bottom, top };
  }
  ```
  `editWallElevation` becomes a thin wrapper: `const next = editElevationBand(eng.elevation ??
  null, end, raw); if (next === undefined) return; editSelected("/engine/elevation", eng.elevation
  ?? null, next);` (identical net behavior; existing `WallEngine`-specific tests still pass
  unmodified). The region/drawing/template editors (added by this milestone's authoring surfaces,
  if the region tool's controls panel has band-editing UI at all today — verify with `rg
  "region-elevation\|drawing-elevation\|template-elevation" src/modules/scene-tools/src/ToolRail.svelte`;
  if none exists yet, this task ADDS one `<input>` pair per shape using `editElevationBand`
  directly, matching the wall editor's markup shape) call the same helper.
- Modify: `src/modules/scene-tools/src/controller.svelte.ts` — the wall/region/drawing/template
  "stamp on create" sites (`makeWallTool`/`makeRegionTool`/`makeDrawTool`/`makeTemplateTool`'s
  `onPointerDown`, wherever each builds its `WallEngine`/`RegionEngine`/`DrawingEngine`/
  `TemplateEngine` literal for the Create op) gain `elevation: ctx.viewedLevelBand?.() ?? null`
  — add a NEW `ToolContext` member `viewedLevelBand?: () => { bottom: number; top: number } | null`
  (resolved by `ToolRail.svelte`'s composition of `ToolContext` from the current `ctx.viewedLevel`
  + the viewed scene's `levels` array: `find(l => l.id === ctx.viewedLevel)` projected to
  `{bottom, top}`, or `null` for no/level-less scene). The place-tool's token stamp and the
  light-tool's light stamp gain `elevation: ctx.viewedLevelBottom?.() ?? null` similarly (a POINT
  value, the level's `bottom` — spec: "the level's `bottom` onto placed tokens/lights"), via a
  second new `ToolContext` member `viewedLevelBottom?: () => number | null`.
- Modify: `src/modules/scene-tools/src/{wall-tool,region-tool,draw-tool,template-tool,place-tool,light-tool}.test.ts`
  — each gains a case: with `ctx.viewedLevelBand`/`viewedLevelBottom` set, a newly stamped
  document's engine body carries the expected `elevation` value; with them unset/`null`, the
  stamped body's `elevation` stays `null` (today's behavior, unchanged).

- [ ] **Step 1:** failing tests; implement.
- [ ] **Step 2:** `pnpm --filter @shadowcat/module-scene-tools test`, `pnpm -r typecheck`, `pnpm
  lint:docs`, `pnpm lint:aria-labels` PASS.
- [ ] **Step 3:** `git commit -m "feat(scene-tools): new geometry, tokens and lights stamp the viewed level's elevation" -- src/modules/scene-tools/"`

## Task 17: the region tool's `Teleport` trigger effect editor

Depends on Task 7 (server `TriggerEffect::Teleport`/`PortalTarget` types, regenerated), Task 16.

**Files:**
- Modify: `src/modules/scene-tools/src/controller.svelte.ts` `ToolController` — add `pickingPortalRow
  = $state<number | null>(null);` (the trigger row index currently awaiting a stage click, or
  `null`); add `beginPickPortalTarget(row: number): void` and `endPickPortalTarget(pos: { x:
  number; y: number } | null): void`:
  ```ts
  /** Original viewed scene, stashed while a portal-target pick temporarily switches it. */
  #pickOriginalScene: string | null = null;

  /** Begin capturing one stage click as `regionTriggers[row]`'s teleport target x/y. Switches
   * the viewed scene to the trigger's currently-authored target scene (or stays put if none is
   * set yet) and overrides the active tool's pointer handling with a one-shot picker.
   * @param row Index into `regionTriggers` of the `teleport` trigger being edited. */
  beginPickPortalTarget(row: number): void {
    const trig = this.regionTriggers[row];
    if (!trig || trig.effect.type !== "teleport") return;
    this.#pickOriginalScene = this.ctx.viewedSceneId?.() ?? null;
    const targetScene = trig.effect.target.scene;
    if (targetScene && targetScene !== this.#pickOriginalScene) {
      this.ctx.setGmViewedScene?.(targetScene);
    }
    this.pickingPortalRow = row;
    this.ctx.scene.setActiveTool({
      onPointerDown: (p: Point) => {
        this.endPickPortalTarget(p);
        return true;
      },
    });
  }

  /** End a portal-target pick: write the captured point (if any) into the row's target, restore
   * the original viewed scene and the tool that was active when the pick began.
   * @param pos The captured stage point, or `null` to cancel without writing. */
  endPickPortalTarget(pos: { x: number; y: number } | null): void {
    const row = this.pickingPortalRow;
    this.pickingPortalRow = null;
    if (row !== null && pos) {
      const trig = this.regionTriggers[row];
      if (trig?.effect.type === "teleport") {
        trig.effect.target.x = pos.x;
        trig.effect.target.y = pos.y;
      }
    }
    if (this.#pickOriginalScene !== null) {
      this.ctx.setGmViewedScene?.(this.#pickOriginalScene);
      this.#pickOriginalScene = null;
    }
    this.ctx.scene.setActiveTool(this.active ? this.#tools[this.active] : null);
  }
  ```
  Add `setGmViewedScene?: (id: string | null) => void` to the `ToolContext` interface (sourced
  from `AppContext.setGmViewedScene`, wired at `ToolRail.svelte`'s `ToolContext` composition
  site).
- Modify: `src/modules/scene-tools/src/ToolRail.svelte` — `triggerEffectTypes` gains `"teleport"`;
  `setRegionTriggerEffectType`'s switch gains
  `case "teleport": trig.effect = { type: "teleport", target: { scene: null, x: 0, y: 0, elevation: null, vfx: null } }; break;`;
  a new `{:else if trig.effect.type === "teleport"}` branch renders: a scene picker (live search
  via `ctx.searchDocuments({ docTypes: ["scene"] }, ...)`, mirroring whatever existing live-search
  input pattern this file or `SceneBrowserPanel.svelte` already uses — `rg "searchDocuments"
  src/modules/scene-tools/src/ToolRail.svelte src/modules/scene-browser/src/*.svelte` for the
  precedent to copy), `x`/`y` number inputs bound to `trig.effect.target.x`/`.y`, a
  `data-testid="region-trigger-teleport-pick"` button calling
  `controller.beginPickPortalTarget(i)` (disabled/relabeled while `controller.pickingPortalRow ===
  i`, showing a "click on stage" prompt), an elevation number input (empty = `null`, mirroring
  `parseElevation`'s existing null-normalization), and a VFX asset picker calling
  `ctx.pickAsset({ kind: "image", tags: ["vfx"] })` — `PickAssetOptions.kind` is `"image" |
  "other"` and `PickAssetOptions.tags` already exists and is wired through the asset query; the
  `vfx` tag is the constant marker the M24 spec defines for effect assets (an untagged image
  still picks when the user clears the filter in the overlay). Test ids: `region-trigger-teleport-scene`,
  `region-trigger-teleport-x`, `region-trigger-teleport-y`, `region-trigger-teleport-pick`,
  `region-trigger-teleport-elevation`, `region-trigger-teleport-vfx`.
- Modify: `src/client/ui-kit/src/locales/en.ts` — `tools.*` additions:
  `triggerTeleportScene`, `triggerTeleportX`, `triggerTeleportY`, `triggerTeleportPick`,
  `triggerTeleportElevation`, `triggerTeleportVfx`, `triggerTeleportPicking` (the in-progress
  prompt text).

**Tests:** `region-tool.test.ts`/`ToolRail.test.ts` — selecting "teleport" seeds the default
target; editing scene/x/y/elevation/vfx updates `regionTriggers[i].effect.target`; a persisted
region document's `triggers` includes the authored `Teleport` effect verbatim;
`beginPickPortalTarget` switches `ctx.setGmViewedScene`, overrides the active tool, and
`endPickPortalTarget` restores both and writes the captured point.

- [ ] **Step 1:** failing tests; implement.
- [ ] **Step 2:** `pnpm --filter @shadowcat/module-scene-tools test`, `pnpm -r typecheck`, `pnpm
  lint:aria-labels`, `pnpm lint:docs` PASS.
- [ ] **Step 3:** `git commit -m "feat(scene-tools): author a Teleport region trigger, including pick-on-stage targeting" -- src/modules/scene-tools/ src/client/ui-kit/src/locales/en.ts"`

## Task 18: GM ghost toggle + `data-level`/`data-token-count` stage attributes

Depends on Task 10, Task 13.

**Files:**
- Modify: `src/client/render/src/token-view.ts` — add a `ghostOtherLevels: () => boolean = () =>
  false` constructor parameter (GM-only toggle; `ToolRail`/`Stage` gates who can flip it, this
  view just renders the effect). When `true`, `reconcile()` additionally includes tokens OUTSIDE
  the viewed level (currently excluded entirely by `sceneScopedDocs`'s level filter) — the ghost
  path must therefore query `sceneScopedDocs(this.store, "token", this.viewedSceneId)` WITHOUT the
  level filter (scene-only) when ghosting is on, and apply a `TokenFx` `{ desaturate: true, alpha:
  0.3 }` to any token whose resolved level (via `levelOf`) differs from `this.viewedLevel()`. Read
  `TokenFx`'s existing shape (`rg "interface TokenFx\|TokenFx =" src/client/render/src`) before
  adding fields — reuse `desaturate`/`alpha` if they already exist (spec: "a `TokenFx`
  `desaturate` + alpha — no new mechanism"); add them to `TokenFx` if they don't.
- Modify: `src/modules/stage/src/Stage.svelte` — a GM-only toggle control (a `<label><input
  type="checkbox">` beside `LevelSwitcher`, `data-testid="ghost-other-levels"`, rendered only
  `{#if ctx.role === "gm"}`) bound to a local `$state<boolean>` passed as `ghostOtherLevels` into
  the `RenderEngine`/`TokenView` construction (thread it through `RenderEngineOpts` the same way
  `selectedTokens`/`footprints` already are — a getter function, not a raw boolean, so it stays
  reactive).
- Modify: `src/modules/stage/src/Stage.svelte` — the `onDerivedApplied`/token-reconcile
  observability block (where `host.dataset.perceivedTokens` etc. are already written) gains:
  `host.dataset.level = ctx.viewedLevel ?? "";` and `host.dataset.tokenCount =
  String(sceneScopedDocs(ctx.documents, "token", () => ctx.viewedSceneId, () =>
  ctx.viewedLevel).length);` — written wherever the existing dataset writes already happen (on
  every derived-frame apply AND on every store-commit reconcile, so both attributes stay live;
  mirror `host.dataset.perceivedTokens`'s own update-site count exactly, do not add a THIRD
  update site with different triggering).
- Modify: `src/client/ui-kit/src/locales/en.ts` — `levels.ghostOtherLevels` label key.

**Tests:** `TokenView.test.ts` — `ghostOtherLevels: () => true` renders an other-level token with
the ghost `TokenFx`; `() => false` (default) excludes it entirely, matching Task 10's plain
level-scoping test. `Stage.test.ts` — `data-level` reflects `ctx.viewedLevel`; `data-token-count`
reflects the level-scoped token count, changing when a token document's elevation moves it across
the viewed level's boundary.

- [ ] **Step 1:** failing tests; implement.
- [ ] **Step 2:** `pnpm --filter @shadowcat/render test`, `pnpm --filter @shadowcat/module-stage
  test`, `pnpm -r typecheck`, `pnpm lint:aria-labels` PASS.
- [ ] **Step 3:** `git commit -m "feat(stage): GM ghost-other-levels toggle; data-level/data-token-count observability" -- src/client/render/src/token-view.ts src/modules/stage/ src/client/ui-kit/src/locales/en.ts"`

## Task 19: e2e `levels.spec.ts` (written here, run by the dispatcher)

Depends on every prior task (drives the full stack through real UI).

**Files:**
- Create: `src/client/shell/e2e/levels.spec.ts` — dual-session (`DUAL_SESSION_TIMEOUT_MS`, the
  GM+invited-player seating flow — copy `senses.spec.ts`'s/`combat-tracker.spec.ts`'s exact setup:
  `createAccount`, invite, second-context redeem, GM re-enters). Scenario (spec §4): GM opens
  `SceneBrowserPanel`, opens the `LevelsEditor` for the active scene, authors two levels with
  distinct backgrounds (`levels-editor` test ids from Task 15, `AssetPicker`'s existing upload
  flow for each background); places a player-owned token on level 1 (via the place tool, at a
  scene point inside level 1's band — set the token's elevation through whatever authoring flow
  Task 16 wired, e.g. the GM selects level 1 in `LevelSwitcher` before stamping) and an NPC on
  level 2 → the PLAYER's stage shows `data-level="l1"` (or whatever id the test authors — use a
  stable authored id, not the UI's auto-generated one, by driving the id field directly if
  `LevelsEditor` exposes one, else assert by NAME lookup instead) and `data-token-count="1"`; the
  GM switches `LevelSwitcher` to level 2 → the GM's stage shows the NPC (assert via
  `data-token-count` or a token-position read, mirroring `senses.spec.ts`'s
  `parsePositions`/`tokenIdNear` helpers); GM draws a region on level 1 (region tool, `region-shape`
  + `region-trigger-add` + `region-trigger-effect` = `"teleport"`, target scene = level 2's OWN
  scene picker value set to the SAME scene, `region-trigger-teleport-elevation` set to a value
  inside level 2's band) covering a cell the player will walk through; the player walks into it
  (drive movement the way `stage.spec.ts`/`senses.spec.ts` already do — `clickScene`/pathfind-click
  from `stage-gestures.ts`) → the player's `data-level` becomes level 2's id and the token is
  visible to the GM (viewing level 2) at the destination. Every behavioral claim in a spec comment
  must be verified against the component before it is written (per the project's e2e-authoring
  discipline — read every UI file this spec drives BEFORE writing assertions about it, not from
  this plan's descriptions alone).
- [ ] **Step 1:** write the spec; `pnpm --filter @shadowcat/shell typecheck` PASS; `pnpm lint`
  PASS. Do NOT run the suite. Commit subject says `(written; dispatcher runs)`.
- [ ] **Step 2:** `git commit -m "test(e2e): multi-level authoring, viewing and teleport browser spec (written; dispatcher runs)" -- src/client/shell/e2e/"`

## Task 20: docs, skills, HISTORY.md, full gate battery

Depends on every prior task.

**Files:**
- `docs/site/modules/stage.md` — document `LevelSwitcher`, `viewedLevel`, the ghost-other-levels
  toggle, `data-level`/`data-token-count`.
- `docs/site/modules/scene-tools.md` — document the elevation-band stamping on new geometry, the
  `Teleport` trigger effect editor (scene picker, pick-on-stage, elevation, vfx).
- `docs/site/modules/scene-browser.md` (create if it doesn't exist yet; `rg -l "scene-browser"
  docs/site/modules/index.md` to check) — document `LevelsEditor`.
- `docs/site/protocol.md` — `scene_subscribe.level` field; the `"vision"`/`"footprints"`
  channels' new `level` fields on their payload entries.
- `docs/design/ARCHITECTURE.md` §4 — the "multi-level maps/portals" phrase in the deferred-work
  row (currently shared with VFX/post-processing/photometric lighting/advanced vision modes) is
  struck from that row (the other items in the same row stay, owned by other Phase-3 milestones)
  and the delivered shape is stated in §3 or a new row: elevation-banded levels, `level_of`,
  portals via `TriggerEffect::Teleport`.
- `src/server/src/data/engine/geometry.rs`'s `WallEngine::elevation` doc fix already lands in
  Task 2 — no further doc edit needed here beyond confirming it reads correctly in context.
- Skills (plugin checkout `~/.claude/skills/shadowcat-codebase/skills/`, edited WITHOUT
  committing there — master §6): `shadowcat-codebase-scene-rendering/SKILL.md` — add levels,
  `level_of`/`bandContains`, explored-per-level, `WriteOrigin::Trigger`, portals to its Key
  files/Hard invariants/Gotchas sections (no new skill — master §6 lists M25 as "—" for new
  skills, updates only); correct any existing "movement is ground-plane, elevation never
  consulted" statement to match Task 2's fix. `shadowcat-codebase-documents-permissions/SKILL.md`
  — add the level conjunct in the egress/vision predicate.
- `docs/HISTORY.md` — append under "## Phase 3 — Atmosphere" (create the heading if this is the
  first Phase-3 milestone to merge — check `git log origin/main -- docs/HISTORY.md` at dispatch
  time to confirm whether M22/M28/M24/M23 already created it): `### M25 · Multi-level maps +
  portals ✅`, branch, spec, every delivered symbol (`SceneLevel`, `ElevationBand`, `band_contains`,
  `level_of`, `TriggerEffect::Teleport`, `PortalTarget`, `WriteOrigin::Trigger`,
  `AppContext.viewedLevel`, `LevelSwitcher`, `LevelsEditor`), decisions (D4's realization), test
  coverage summary, the e2e spec's WRITTEN-NOT-RUN status. Do NOT touch `docs/PLAN.md` (master §3:
  only the LAST milestone to merge flips the phase heading; M25 is not that milestone).

- [ ] **Step 1:** doc edits.
- [ ] **Step 2:** skill edits in the plugin checkout; dispatch
  `shadowcat-codebase:shadowcat-spec-reviewer` (sonnet, effort high) on the skill diff alone;
  apply its findings or escalate to `-fable` per the Model/Effort directives above; run `node
  scripts/check-skill-symbol-refs-cli.mjs`, `node scripts/check-skill-api-refs-cli.mjs` (needs
  `pnpm build:all`'s `dist-docs`), `pnpm run test:scripts` — zero broken citations introduced;
  commit + push in the PLUGIN repo (its own remote), not this repo.
- [ ] **Step 3:** the FULL gate battery from Global Constraints above (background the long ones);
  paste every result line.
- [ ] **Step 4:** dispatch the spec+code reviewer pair (blind, pre-generated diff) on the FULL
  branch diff accumulated so far (Tasks 1–20); apply agreed findings as fix-forward commits before
  Task 21.
- [ ] **Step 5:** `git commit -m "docs(m25): levels + portals — history, site docs, ARCHITECTURE" -- docs/"`
- [ ] **Step 6:** report to the dispatcher: `STATUS`, every commit so far, every gate result line,
  the plugin diff stat, deviations, and explicit confirmation that Tasks 1–20 are complete and
  Task 21 is HELD pending M22/M28/M24/M23 landing on `origin/main`.

## Task 21: merge-forward integration (HELD until M22, M28, M24, M23 are on `origin/main`)

Per master §5's merge-forward protocol. The dispatcher verifies `git rev-parse origin/main`
contains M22's, M28's, M24's and M23's merge commits before starting this task; it is never
stubbed or started early.

**Steps:**
- [ ] **Step 1:** `git fetch origin && git merge origin/main` in the worktree (a merge commit,
  never a rebase). Resolve conflicts per master §3's conventions: `src/server/src/ws/protocol.rs`
  (append `TriggerEffect`/`ClientMsg`/`ServerMsg` variants — M25 already appended in earlier
  tasks; re-check ordering against whatever M23/M24 appended ahead of it), `src/client/core/src/wire.ts`
  (append after the last existing union member; re-run `wire.test.ts`), `src/client/ui-kit/src/appContext.ts`
  (member order: `viewedLevel`/`setViewedLevel` stay after `panels`, M22/M23/M24's members land
  wherever THEIR append point was), `src/client/ui-kit/src/__fixtures__/appContextTest.ts` (merge
  every milestone's default), `src/client/shell/src/App.svelte` module list (M25 adds none —
  confirm no conflict), `src/client/render/src/engine.ts` (M22 owns the ticker/`RenderEngineOpts.performance`,
  M24 adds `vfxView` reconcile as one call site, M25's `viewedLevel` addition from Task 13 must
  coexist with both without reordering either), `src/server/src/data/engine/mod.rs`
  (`ENGINE_DOC_TYPES` — M23 may have appended a doc type; M25 appends none here, confirm),
  `src/server/src/data/engine/scene.rs` (M23's `ambience`/`audio` overlay fields on
  `WorldSettingsEngine` land alongside M25's `SceneEngine.levels` — both APPEND, no conflict
  expected structurally but textually adjacent), `WriteOrigin` and every exhaustive match on it
  (M23's `AudioTransport` variant appends alongside M25's `Trigger` — both `matches!` lists gain
  both entries; re-verify `is_server_authored`/`skips_capability_gates` list BOTH), `docs/HISTORY.md`
  (each milestone's own entry under the Phase 3 heading — never reorder or edit another
  milestone's entry).
- [ ] **Step 2:** wire the M24 VFX seam this milestone consumes (master §2.4/§2.3): `VfxView`
  (from `@shadowcat/render`, M24) honours `elevation` by level — its constructor gains the SAME
  `viewedLevel: () => string | null` fourth-ish parameter `sceneScopedDocs`-style filtering this
  milestone's other views already have (Task 10's pattern, applied to `VfxView` specifically, now
  that it exists); `RenderEngine.start()`'s `vfxView` reconcile call site gains
  `this.opts.viewedLevel`.
- [ ] **Step 3:** teleport `vfx` broadcast: `ws::room::Room::fire_region_triggers`'s Teleport
  branch (Task 9) gains, after a successful teleport with `target.vfx: Some(asset)`, TWO
  `ServerMsg::Vfx` frames broadcast through M24's handler path (source position on the ORIGIN
  scene, destination position on `dest_scene`) — read M24's `ServerMsg::Vfx`/`ClientMsg::PlayVfx`
  handler (now merged) and the room-broadcast helper it uses (`rg "ServerMsg::Vfx"
  src/server/src/ws/room.rs` once merged) and call it the same way an existing aux-frame broadcast
  in this file does (`ScenePing`'s precedent — per-user rate bucket per master §2.3).
- [ ] **Step 4:** audibility test (M23b's channel, master §2.4): a floor-2 emitter is occluded for
  a floor-1 listener via the elevation-banded raycaster ALONE (no code change — this proves the
  inheritance master §2.4 states: "M23b ... inherits level separation for free"). Add the test to
  wherever M23's audibility tests live (`rg -l "audibility" src/server/src` once M23 is merged),
  using a `SceneLevel`-bearing scene with an emitter on level 2 and a listener on level 1 at the
  same (x,y): assert `through_wall_gain` (or the equivalent occluded-gain constant M23 defines)
  applies, distinguishing it from an unoccluded same-level pair at the identical horizontal
  distance.
- [ ] **Step 5:** the FULL gate battery (background the long ones); paste every result line.
  `pnpm --filter @shadowcat/core test:e2e` (the formula/levels conformance corpora both still
  agree Rust-vs-TS).
- [ ] **Step 6:** buddy-check the WHOLE branch diff (spec + code reviewer, blind, dispatcher
  pre-generated) one final time post-merge; apply agreed findings.
- [ ] **Step 7:** `pnpm gate:push`; `git push`. Report to the dispatcher: `STATUS`, the merge
  commit sha, every gate result line, confirmation `git rev-parse origin/main` (pre-merge) is
  contained in the new HEAD, and readiness for PR.

---

## Self-review (spec coverage, placeholders, consistency)

- **§1 (data):** covered — Task 1 (`SceneLevel`, `ElevationBand` rename, `band_contains`,
  `level_of`, conformance corpus, region/drawing/template `elevation` fields, `SceneEngine.levels`
  + validate).
- **§2 movement:** covered — Task 2 (`move_wall_entries`/`move_walls`, `RouteMover.elevation`,
  `MoveGateInputs.mover_elevation`, doc fix), Task 3 (`region_field`/`trigger_regions` banding),
  Task 4 (navmesh level key).
- **§2 vision/secrecy/lighting/explored:** covered — Task 5 (polygons/lit-mask/lighting-inputs/
  footprints per level, `move_clip` conjunct — explicitly states NO server-side resting-token
  fog-stripping is added, matching spec), Task 6 (schema, repository, `enrich_vision_explored`,
  `SceneSubscribe.level`).
- **§2 portals:** covered — Task 7 (`TriggerEffect::Teleport`/`PortalTarget` + validation), Task 8
  (`WriteOrigin::Trigger` + the `apply_intent` Move-arm literal-comparison fix + M18 effects
  migrated), Task 9 (same/cross-scene application, one-hop anti-loop, GM notices, combat notice,
  `vfx` carried-not-played pre-M24).
- **§3 client:** covered — Task 10 (`sceneScopedDocs` 4th arg + 6 views), Task 11
  (`AppContext.viewedLevel`), Task 12 (`WorldSession` + persistence), Task 13 (`RenderEngine`
  re-subscribe), Task 14 (`LevelSwitcher`, host decision stated per master §2.7), Task 15
  (`LevelsEditor`), Task 16 (scene-tools stamping), Task 17 (Teleport trigger editor incl.
  pick-on-stage), Task 18 (ghost toggle, `data-level`/`data-token-count`).
- **§4 tests:** every server/client test named in spec §4 is assigned to the task that implements
  its subject; the e2e spec is Task 19, written not run.
- **§5 docs/skills:** Task 20.
- **Master §2.7 UI-extension-contract note:** resolved explicitly in Task 14's header (merge-order
  makes `STAGE_OVERLAY_CONTRACT` unavailable to M25; `LevelSwitcher` hosts directly in
  `Stage.svelte`).
- **Master §5 integration:** Task 21, held on the correct upstream set (M22, M28, M24, M23), wires
  the M24 VFX seam and the M23b audibility inheritance test.
- **No placeholder steps:** every task names exact files, exact symbol signatures for every new
  type/function, and exact test assertions; no step reads "implement X" without stating X's shape.
- **Type/name consistency:** `ElevationBand`/`SceneLevel`/`band_contains`/`level_of` are the same
  identifiers from Task 1 through Task 21; `WriteOrigin::Trigger` is introduced once (Task 8) and
  consumed identically in Task 9; `AppContext.viewedLevel`/`setViewedLevel` are introduced once
  (Task 11) and consumed identically in Tasks 12–18.
