# Move-Stream Live Clip Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** An observer whose own token is moving sees a concurrently moving token appear mid-stride the instant their sweeping fog clears it, instead of at that token's stop.

**Architecture:** The room retains every in-flight `MoveStream` (full frame, `mover_vision` included) in the existing per-token moving-lock map. The egress clip evaluates each of A's samples against the clip target's vision *at that sample's instant* — the target's own `mover_vision` timeline sample when the target is mid-move, the committed-position vision otherwise. When a recipient's own move starts, the egress loop re-clips and re-emits every other in-flight stream in that scene to that recipient; the client's keyed playback overwrites in place, so no protocol or client production change is needed.

**Tech Stack:** Rust (tokio, axum ws), existing `ServerMsg::MoveStream` / `VisionSample` wire types; TypeScript client test only (Vitest).

**Spec:** `docs/superpowers/specs/2026-08-27-move-stream-live-clip-design.md`

## Global Constraints

- ARCHITECTURE.md invariant 11: UX outranks data secrecy. This plan keeps wire secrecy intact; if an implementer finds that impossible for a task, they STOP and report — they do not degrade the visual.
- Secrecy invariants of `clip_move_stream` are preserved verbatim: observers never receive `mover_vision`, `cost`, or `truncated`; `stop`/`duration_ms` clip to the last visible sample; fully-occluded → `None`.
- No lock across an await (`publish_guard` is the only Mutex held across awaits; ECS `RwLock` read guards are scoped and dropped before any await).
- No `#[allow]`/`#[expect]` suppressions. Rust test bodies live in sibling files (`<stem>/tests.rs`), never inline `mod tests { … }`. Files stay under 5,000 lines.
- Comments: present-tense invariants and coupling; no history, no task ids.
- Commit each task with `git commit -m "…" -- <explicit paths>`.

### Gate battery (copied from `.github/workflows/ci.yml`; run the relevant subset per task, ALL before final review)

Rust job: `cargo fmt --all -- --check` · `cargo clippy --all-targets -- -D warnings` · `cargo test --all` · `git diff --exit-code src/types/generated` (all with `--manifest-path src/server/Cargo.toml` where applicable).
TS job: `pnpm -r typecheck` · `pnpm -r test` · `pnpm run test:scripts` · `pnpm docs:check-examples` · `pnpm lint` · `pnpm --filter @shadowcat/shell build` · `pnpm --filter "shadowcat-example-*" build` · `pnpm run check:svelte-runtime`.
server-e2e: `pnpm --filter @shadowcat/core test:e2e`. UI-e2e: `pnpm --filter @shadowcat/shell e2e`.
docs job: `pnpm lint:docs` · `pnpm lint:props` · `pnpm lint:comments` · `pnpm lint:allowances` · `pnpm lint:file-size` · `pnpm lint:inline-tests` · `cargo clippy --manifest-path src/server/Cargo.toml --all-targets -- -D missing-docs -D clippy::missing-docs-in-private-items` · nightly doc-examples gate, run EXACTLY as CI does: `RUSTDOCFLAGS="-D rustdoc::missing_doc_code_examples" cargo +nightly doc --manifest-path src/server/Cargo.toml --document-private-items --no-deps --target-dir target/nightly-doc` (the env var is what arms the lint; every new `pub`/`pub(crate)` item in `move_clip.rs` and `room.rs` needs a `# Examples` section).
Local-only (NOT in `ci.yml` — CI has no `~/.claude/`; required by `.claude/CLAUDE.md` after any skill edit): `node scripts/check-skill-api-refs-cli.mjs` · `node scripts/check-skill-symbol-refs-cli.mjs`.
Never pipe a gate into `tail`/`echo` before checking its exit code — redirect to a file and read the file.

## Model/Effort directives

Plan written mainline (user choice, 2026-08-27). Execution: `shadowcat-codebase:shadowcat-coder` (effort medium) per task; reviewers `shadowcat-codebase:shadowcat-spec-reviewer` + `shadowcat-codebase:shadowcat-code-reviewer` (effort high). Escalate a BLOCKED coder to `shadowcat-coder-opus` before the human.

## Buddy-check directives

User elected a buddy-check of THIS PLAN before Task 1 is dispatched (secrecy boundary + `publish_guard` concurrency). Record the outcome here before execution:

- Plan buddy check: done 2026-08-27 (two blind `shadowcat-spec-reviewer`s, 3 rounds, CONVERGED). Agreed + folded in: zero-progress `execute_move` branch had no `frame` (Task 2 step 10 added); `include_str!` fixture path had one `../` too many (fixed); `chooseVisionSample(sweep, …)` type slip (fixed); `concurrent_streams` excluded by token where spec §2.3 excludes by mover (fixed in Tasks 2 and 4); gate battery mislabelled the two skill-ref checkers as `ci.yml` steps and compressed the nightly-doc command (both fixed); call-site counts stated (20 `execute_move`, 9 `setup_clip_room`). No unresolved disagreements. Broker note: the plan was edited while round 3 ran, which made the last severity question moot rather than debated — the fixes were identical under either severity.
- Flagged tasks: 3, 4 — buddy check replaces both review stages (egress secrecy boundary + `publish_guard`/registry concurrency).
- Unflagged tasks showing risk signals: ask.

---

## File structure

| File | Responsibility |
|---|---|
| `src/server/src/ws/move_clip.rs` (new) | Pure, lock-free sample-clipping arithmetic: choose the timeline sample for an instant; union timeline polygons; clip a sample list against static-or-timeline vision. No room, no I/O. |
| `src/server/src/ws/move_clip/tests.rs` (new) | Unit tests for the above + the server side of the chosen-sample parity fixture. |
| `src/client/render/src/__fixtures__/chosen-vision-sample.json` (new) | Shared parity fixture (samples + probes) read by both the Rust and the Vitest test. |
| `src/client/render/src/fog-blend.ts` | Gains the exported pure `chooseVisionSample`; `RenderEngine.chosenSample` delegates to it. |
| `src/client/render/src/fog-blend.test.ts` | Client side of the parity fixture. |
| `src/server/src/ws/room.rs` | `ActiveStream` registry replacing the bare `moving` map; `execute_move` builds + registers the wire frame; accessors for the egress loop. |
| `src/server/src/ws/room/room_tests.rs` | Registry lifecycle tests. |
| `src/server/src/ws/conn.rs` | `clip_move_stream` consults the timeline; egress arm re-emits concurrent streams on the recipient's own move. |
| `src/server/src/ws/conn/tests.rs` | Timeline-clip scenarios, GM see-as, re-emit egress integration test. |

---

### Task 1: Pure clip arithmetic (`move_clip.rs`) + chosen-sample parity fixture

**Files:**
- Create: `src/server/src/ws/move_clip.rs`
- Create: `src/server/src/ws/move_clip/tests.rs`
- Modify: `src/server/src/ws/mod.rs` (add `pub(crate) mod move_clip;` after `pub mod conn;`)
- Create: `src/client/render/src/__fixtures__/chosen-vision-sample.json`
- Modify: `src/client/render/src/fog-blend.ts`, `src/client/render/src/engine.ts:1046-1069`
- Test: `src/client/render/src/fog-blend.test.ts`

**Interfaces:**
- Produces (Rust, all `pub(crate)` in `crate::ws::move_clip`):
  - `fn chosen_vision_sample(samples: &[VisionSample], elapsed_ms: f64) -> Option<&VisionSample>` — the sample with the greatest `t_ms <= elapsed_ms`; the first sample when `elapsed_ms` precedes all; `None` only for an empty slice.
  - `struct TimelineStream<'a> { start_server_ms: f64, vision: &'a [VisionSample] }` — one in-flight move of the clip target.
  - `fn timeline_polys_at(streams: &[TimelineStream<'_>], t_abs_ms: f64) -> Option<Vec<Vec<P>>>` — `None` when no stream has `start_server_ms <= t_abs_ms`; otherwise the union (concatenation) of each such stream's chosen sample polygons, converted `[f64;2] → (f64,f64)`.
  - `fn clip_samples(samples: &[PosSample], start_server_ms: f64, static_polys: &[Vec<P>], streams: &[TimelineStream<'_>]) -> Vec<PosSample>` — per sample: `polys = timeline_polys_at(streams, start_server_ms + s.t_ms).unwrap_or(static_polys)`; keep iff `polys.iter().any(|poly| point_in_poly(poly, (s.pos[0], s.pos[1])))`.
- Produces (TS, exported from `@shadowcat/render`'s `fog-blend.ts`): `chooseVisionSample(samples: MoveVisionSample[], elapsed: number): MoveVisionSample` — same rule (caller guarantees non-empty, as today).

- [ ] **Step 1: Write the parity fixture**

`src/client/render/src/__fixtures__/chosen-vision-sample.json`:
```json
{
  "samples": [0, 250, 500, 900],
  "probes": [
    { "elapsed": -10, "expectIndex": 0 },
    { "elapsed": 0, "expectIndex": 0 },
    { "elapsed": 249.999, "expectIndex": 0 },
    { "elapsed": 250, "expectIndex": 1 },
    { "elapsed": 600, "expectIndex": 2 },
    { "elapsed": 900, "expectIndex": 3 },
    { "elapsed": 5000, "expectIndex": 3 }
  ]
}
```
`samples` are `tMs` values; each probe's expected sample is identified by index. Both tests build polygons whose first vertex encodes the index (`[[index, 0], [index, 1], [index+1, 1]]`) so the chosen sample is identifiable.

- [ ] **Step 2: Write the failing Rust tests**

`src/server/src/ws/move_clip/tests.rs`:
```rust
use super::*;
use crate::ws::protocol::{PosSample, VisionSample};

/// Polygon whose first vertex x-coordinate encodes `idx`, so a chosen sample is identifiable.
fn tagged(idx: usize) -> Vec<Vec<[f64; 2]>> {
    let i = idx as f64;
    vec![vec![[i, 0.0], [i, 1.0], [i + 1.0, 1.0]]]
}

fn fixture_samples() -> (Vec<VisionSample>, Vec<(f64, usize)>) {
    let raw: serde_json::Value = serde_json::from_str(include_str!(
        "../../../../client/render/src/__fixtures__/chosen-vision-sample.json"
    ))
    .unwrap();
    let samples = raw["samples"]
        .as_array()
        .unwrap()
        .iter()
        .enumerate()
        .map(|(i, t)| VisionSample { t_ms: t.as_f64().unwrap(), polygons: tagged(i) })
        .collect();
    let probes = raw["probes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| (p["elapsed"].as_f64().unwrap(), p["expectIndex"].as_u64().unwrap() as usize))
        .collect();
    (samples, probes)
}

#[test]
fn chosen_vision_sample_matches_the_shared_parity_fixture() {
    let (samples, probes) = fixture_samples();
    for (elapsed, expect) in probes {
        let got = chosen_vision_sample(&samples, elapsed).unwrap();
        assert_eq!(got.polygons, tagged(expect), "elapsed={elapsed}");
    }
}

#[test]
fn chosen_vision_sample_is_none_on_empty() {
    assert!(chosen_vision_sample(&[], 0.0).is_none());
}

/// A unit square at the origin, and one shifted to x in [10,11].
fn square(x0: f64) -> Vec<Vec<[f64; 2]>> {
    vec![vec![[x0, 0.0], [x0 + 1.0, 0.0], [x0 + 1.0, 1.0], [x0, 1.0]]]
}

#[test]
fn timeline_polys_at_is_none_before_any_stream_starts() {
    let v = vec![VisionSample { t_ms: 0.0, polygons: square(0.0) }];
    let streams = [TimelineStream { start_server_ms: 1000.0, vision: &v }];
    assert!(timeline_polys_at(&streams, 999.0).is_none());
    assert!(timeline_polys_at(&streams, 1000.0).is_some());
}

#[test]
fn timeline_polys_at_unions_every_started_stream_and_uses_the_last_sample_past_its_end() {
    let a = vec![
        VisionSample { t_ms: 0.0, polygons: square(0.0) },
        VisionSample { t_ms: 100.0, polygons: square(10.0) },
    ];
    let b = vec![VisionSample { t_ms: 0.0, polygons: square(20.0) }];
    let streams = [
        TimelineStream { start_server_ms: 1000.0, vision: &a },
        TimelineStream { start_server_ms: 1050.0, vision: &b },
    ];
    // t=1040: only `a` started, at its first sample.
    let p = timeline_polys_at(&streams, 1040.0).unwrap();
    assert_eq!(p, vec![vec![(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)]]);
    // t=5000: both started; `a` is past its last sample (uses square(10)), `b` at its only sample.
    let p = timeline_polys_at(&streams, 5000.0).unwrap();
    assert_eq!(p.len(), 2);
    assert_eq!(p[0][0], (10.0, 0.0));
    assert_eq!(p[1][0], (20.0, 0.0));
}

#[test]
fn clip_samples_uses_the_timeline_only_at_instants_a_stream_is_active() {
    // A's samples: (0.5,0.5) at t=0 and (10.5,0.5) at t=200, starting at 1000.
    let samples = vec![
        PosSample { t_ms: 0.0, pos: [0.5, 0.5] },
        PosSample { t_ms: 200.0, pos: [10.5, 0.5] },
    ];
    // Static (committed) vision covers only the origin square.
    let static_polys: Vec<Vec<P>> = vec![vec![(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)]];
    // Target R's sweep starts at 1100 and sees the far square from its first sample.
    let r = vec![VisionSample { t_ms: 0.0, polygons: square(10.0) }];
    let streams = [TimelineStream { start_server_ms: 1100.0, vision: &r }];
    let out = clip_samples(&samples, 1000.0, &static_polys, &streams);
    // t_abs=1000 → static → origin visible; t_abs=1200 → timeline → far square visible.
    assert_eq!(out.len(), 2);
    assert_eq!(out[0].pos, [0.5, 0.5]);
    assert_eq!(out[1].pos, [10.5, 0.5]);
}

#[test]
fn clip_samples_with_no_streams_equals_static_clip() {
    let samples = vec![
        PosSample { t_ms: 0.0, pos: [0.5, 0.5] },
        PosSample { t_ms: 200.0, pos: [10.5, 0.5] },
    ];
    let static_polys: Vec<Vec<P>> = vec![vec![(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)]];
    let out = clip_samples(&samples, 1000.0, &static_polys, &[]);
    assert_eq!(out, vec![PosSample { t_ms: 0.0, pos: [0.5, 0.5] }]);
}
```

- [ ] **Step 3: Run to verify they fail**

Run: `cargo test --manifest-path src/server/Cargo.toml move_clip 2> C:\Users\emper\AppData\Local\Temp\claude\C--Dev-Shadowcat\8b04b8bf-9450-46f6-9326-557dec9a205c\scratchpad\t1.txt; echo $?` then read the file.
Expected: compile failure — `move_clip` module not found.

- [ ] **Step 4: Implement `move_clip.rs`**

```rust
//! Pure sample-clipping arithmetic for the per-recipient `MoveStream` egress clip.
//!
//! No locks, no I/O. `clip_move_stream` (in `conn`) resolves the clip target's committed
//! vision and in-flight move timelines, then delegates the per-sample decision here.
//!
//! INVARIANT (client parity): `chosen_vision_sample` implements the same rule as the client's
//! `chooseVisionSample` (`fog-blend.ts`) — greatest `t_ms <= elapsed`, first sample before
//! that — so a sample admitted here is exactly one the recipient's sweeping fog will show.
//! The shared fixture `src/client/render/src/__fixtures__/chosen-vision-sample.json` is
//! asserted by both sides.

#![deny(missing_docs)]
#![deny(clippy::missing_docs_in_private_items)]

use crate::scene::vision::{point_in_poly, P};
use crate::ws::protocol::{PosSample, VisionSample};

/// One in-flight move of the clip target: its wall-clock origin and vision sweep.
pub(crate) struct TimelineStream<'a> {
    /// The move's `MoveStream.start_server_ms`.
    pub start_server_ms: f64,
    /// The move's `mover_vision` samples (elapsed-ms from `start_server_ms`).
    pub vision: &'a [VisionSample],
}

/// The sample with the greatest `t_ms <= elapsed_ms`; the first sample when `elapsed_ms`
/// precedes every sample; `None` only when `samples` is empty.
pub(crate) fn chosen_vision_sample(samples: &[VisionSample], elapsed_ms: f64) -> Option<&VisionSample> {
    let mut chosen = samples.first()?;
    for s in samples {
        if s.t_ms <= elapsed_ms {
            chosen = s;
        }
    }
    Some(chosen)
}

/// Union of the chosen-sample polygons of every stream that has started by `t_abs_ms`.
/// `None` when no stream has started (the caller falls back to committed vision).
pub(crate) fn timeline_polys_at(streams: &[TimelineStream<'_>], t_abs_ms: f64) -> Option<Vec<Vec<P>>> {
    let mut out: Vec<Vec<P>> = Vec::new();
    let mut any = false;
    for st in streams.iter().filter(|st| st.start_server_ms <= t_abs_ms) {
        any = true;
        if let Some(sample) = chosen_vision_sample(st.vision, t_abs_ms - st.start_server_ms) {
            out.extend(sample.polygons.iter().map(|poly| poly.iter().map(|v| (v[0], v[1])).collect()));
        }
    }
    any.then_some(out)
}

/// Keep each sample whose position is inside the clip target's vision AT THAT SAMPLE'S
/// INSTANT: the timeline union while any target stream is active, else `static_polys`.
pub(crate) fn clip_samples(
    samples: &[PosSample],
    start_server_ms: f64,
    static_polys: &[Vec<P>],
    streams: &[TimelineStream<'_>],
) -> Vec<PosSample> {
    samples
        .iter()
        .filter(|s| {
            let p = (s.pos[0], s.pos[1]);
            let timeline = timeline_polys_at(streams, start_server_ms + s.t_ms);
            let polys: &[Vec<P>] = match &timeline {
                Some(t) => t,
                None => static_polys,
            };
            polys.iter().any(|poly| point_in_poly(poly, p))
        })
        .copied()
        .collect()
}

#[cfg(test)]
mod tests;
```
Add `pub(crate) mod move_clip;` to `src/server/src/ws/mod.rs` directly after `pub mod conn;`. Confirm `point_in_poly` and `P` are exported from `crate::scene::vision` with those exact names (`vision.rs:14` defines `pub type P = (f64, f64)`; `conn.rs:1218` already imports `point_in_poly`).

- [ ] **Step 5: Run Rust tests to verify they pass**

Run: `cargo test --manifest-path src/server/Cargo.toml move_clip`
Expected: 6 passed.

- [ ] **Step 6: Write the failing client parity test**

Append to `src/client/render/src/fog-blend.test.ts`:
```ts
import { readFileSync } from "node:fs";
import { chooseVisionSample } from "./fog-blend";
import type { MoveVisionSample } from "./types";

test("chooseVisionSample matches the server's chosen_vision_sample on the shared fixture", () => {
  const raw = JSON.parse(
    readFileSync(new URL("./__fixtures__/chosen-vision-sample.json", import.meta.url), "utf8"),
  ) as { samples: number[]; probes: { elapsed: number; expectIndex: number }[] };
  const samples: MoveVisionSample[] = raw.samples.map((tMs, i) => ({
    tMs,
    polygons: [[[i, 0], [i, 1], [i + 1, 1]]],
  }));
  for (const { elapsed, expectIndex } of raw.probes) {
    expect(chooseVisionSample(samples, elapsed).polygons[0][0][0]).toBe(expectIndex);
  }
});
```
(If the file already imports `test`/`expect` from vitest, reuse; otherwise add `import { expect, test } from "vitest";`.)

- [ ] **Step 7: Run to verify it fails**

Run: `pnpm --filter @shadowcat/render test -- fog-blend`
Expected: FAIL — `chooseVisionSample` is not exported.

- [ ] **Step 8: Implement `chooseVisionSample` and delegate**

In `src/client/render/src/fog-blend.ts` add:
```ts
import type { MoveVisionSample } from "./types";

/** The sweep sample that should be showing at `elapsed` ms: the one with the greatest
 * `tMs <= elapsed`, or the first sample when `elapsed` precedes every sample.
 * INVARIANT (server parity): mirrors the server's `chosen_vision_sample` — the egress clip admits
 * a moving token's sample only where this sample's polygons will show it. Fixture-tested on both
 * sides (`__fixtures__/chosen-vision-sample.json`).
 * @param samples The sweep's ordered vision samples (non-empty).
 * @param elapsed Milliseconds elapsed since the sweep started.
 * @returns The sample to show at `elapsed`.
 * @example
 * ```ts
 * chooseVisionSample([{ tMs: 0, polygons: [] }, { tMs: 500, polygons: [] }], 250).tMs; // 0
 * ```
 */
export function chooseVisionSample(samples: MoveVisionSample[], elapsed: number): MoveVisionSample {
  let chosen = samples[0];
  for (const s of samples) {
    if (s.tMs <= elapsed) chosen = s;
  }
  return chosen;
}
```
In `engine.ts`, replace the body of the private `chosenSample` with `return chooseVisionSample(sweep.samples, sweep.elapsed);` — keep its signature and doc, adjusting the doc to say it delegates to `chooseVisionSample` (import it from `./fog-blend`). Check `src/client/render/src/index.ts`: if `fog-blend` exports are re-exported there, add `chooseVisionSample` to the list; otherwise leave it package-internal.

- [ ] **Step 9: Run client tests + typecheck**

Run: `pnpm --filter @shadowcat/render test` and `pnpm --filter @shadowcat/render typecheck`
Expected: all pass (existing `chosenSample` behaviour tests in `engine.test.ts` unchanged).

- [ ] **Step 10: Gates + commit**

Run: `cargo fmt --all --manifest-path src/server/Cargo.toml`, `cargo clippy --manifest-path src/server/Cargo.toml --all-targets -- -D warnings`, `cargo clippy --manifest-path src/server/Cargo.toml --all-targets -- -D missing-docs -D clippy::missing-docs-in-private-items`, `pnpm lint:props`, `pnpm lint:comments`, `pnpm lint:inline-tests`, `pnpm docs:check-examples`.
```bash
git add src/server/src/ws/move_clip.rs src/server/src/ws/move_clip/tests.rs src/server/src/ws/mod.rs src/client/render/src/__fixtures__/chosen-vision-sample.json src/client/render/src/fog-blend.ts src/client/render/src/fog-blend.test.ts src/client/render/src/engine.ts
git commit -m "feat(ws): pure move-stream timeline clip arithmetic + chosen-sample parity fixture" -- src/server/src/ws/move_clip.rs src/server/src/ws/move_clip/tests.rs src/server/src/ws/mod.rs src/client/render/src/__fixtures__/chosen-vision-sample.json src/client/render/src/fog-blend.ts src/client/render/src/fog-blend.test.ts src/client/render/src/engine.ts
```

---

### Task 2: Room retains in-flight streams (`ActiveStream` registry)

**Files:**
- Modify: `src/server/src/ws/room.rs:30-60` (`MoveExecution`), `:220-223` + `:270` (`moving` field), `:302-312` (`broadcast_aux`), `:563-571` (`execute_move` signature), `:825-838` (moving-lock insert)
- Modify: `src/server/src/ws/conn.rs:904-986` (`handle_move_request`)
- Test: `src/server/src/ws/room/room_tests.rs`

**Interfaces:**
- Consumes: nothing from Task 1.
- Produces (all in `crate::ws::room`):
  - `pub(crate) struct ActiveStream { pub mover: Uuid, pub scene: Uuid, pub start_ms: i64, pub end_ms: i64, pub frame: Arc<ServerMsg> }` — `frame` is the FULL unclipped `ServerMsg::MoveStream` (with `mover_vision`).
  - `Room.moving: Mutex<HashMap<Uuid, ActiveStream>>` (token → stream). Lazy expiry unchanged: an entry with `now >= end_ms` is expired.
  - `Room::execute_move(&self, repo, ctx, scene_id, token, path, ts, request_id: Uuid) -> Result<MoveExecution, DataError>` — NEW trailing `request_id` param; builds the wire frame, registers it, and returns it as `MoveExecution.frame: Arc<ServerMsg>`.
  - `Room::broadcast_aux_shared(&self, msg: Arc<ServerMsg>)`; `broadcast_aux` delegates to it.
  - `Room::mover_streams(&self, mover: Uuid, scene: Uuid, now: i64) -> Vec<Arc<ServerMsg>>` — unexpired frames whose `mover`/`scene` match.
  - `Room::concurrent_streams(&self, scene: Uuid, exclude_mover: Uuid, now: i64) -> Vec<Arc<ServerMsg>>` — unexpired frames in `scene` whose `mover != exclude_mover` (spec §2.3 excludes by MOVER: a recipient's own other in-flight token must not be re-sent to them).
  - `#[cfg(test)] pub(crate) async fn register_stream_for_test(&self, token: Uuid, stream: ActiveStream)` — test-only insertion, used by Tasks 3–4.

- [ ] **Step 1: Write the failing registry tests**

Append to `src/server/src/ws/room/room_tests.rs` (reuse `movement_scene_with_speed`; `now_millis()` helper exists at `:1944`):
```rust
#[tokio::test]
async fn execute_move_registers_the_full_frame_and_accessors_filter_by_mover_scene_and_expiry() {
    // Slow speed so the stream stays unexpired for the duration of the assertions.
    let h = movement_scene_with_speed("unrestricted", false, 0.5).await;
    let req = Uuid::from_u128(0x5EED);
    let exec = h
        .room
        .execute_move(&h.repo, &h.player, h.scene_id, h.token_id, vec![h.start, h.adj], now_millis(), req)
        .await
        .unwrap();
    let ServerMsg::MoveStream { request_id, token_id, mover, scene, mover_vision, cost, .. } =
        exec.frame.as_ref()
    else {
        panic!("frame must be a MoveStream");
    };
    assert_eq!(*request_id, req);
    assert_eq!(*token_id, h.token_id);
    assert_eq!(*mover, h.player.user_id);
    assert_eq!(*scene, h.scene_id);
    assert!(cost.is_some(), "the registered frame is the full in-process frame");
    // A player mover on a non-GM path carries a vision timeline (None only for GM movers).
    assert!(mover_vision.is_some());

    let now = now_millis();
    let mine = h.room.mover_streams(h.player.user_id, h.scene_id, now).await;
    assert_eq!(mine.len(), 1);
    assert!(Arc::ptr_eq(&mine[0], &exec.frame));
    assert!(h.room.mover_streams(h.gm.user_id, h.scene_id, now).await.is_empty());
    assert!(h.room.mover_streams(h.player.user_id, Uuid::from_u128(0xBAD), now).await.is_empty());
    // concurrent_streams excludes every stream of the named MOVER, not just one token.
    assert!(h.room.concurrent_streams(h.scene_id, h.player.user_id, now).await.is_empty());
    assert_eq!(h.room.concurrent_streams(h.scene_id, h.gm.user_id, now).await.len(), 1);
    // Expiry: a `now` past end_ms hides it.
    assert!(h.room.mover_streams(h.player.user_id, h.scene_id, now + 3_600_000).await.is_empty());
}

#[tokio::test]
async fn moving_lock_still_refuses_a_second_move_while_the_stream_is_in_flight() {
    let h = movement_scene_with_speed("unrestricted", false, 0.5).await;
    h.room
        .execute_move(&h.repo, &h.player, h.scene_id, h.token_id, vec![h.start, h.adj], now_millis(), Uuid::from_u128(1))
        .await
        .unwrap();
    let second = h
        .room
        .execute_move(&h.repo, &h.player, h.scene_id, h.token_id, vec![h.adj, h.adj2], now_millis(), Uuid::from_u128(2))
        .await;
    assert!(matches!(second, Err(DataError::Forbidden)));
}
```
If `ServerMsg`/`Arc`/`DataError` are not already imported at the top of `room_tests.rs`, add `use crate::ws::protocol::ServerMsg; use std::sync::Arc; use crate::data::DataError;` (match the existing import style at the top of the file — check whether an existing moving-lock test already asserts `Forbidden`; if so, extend it with the `request_id` argument rather than duplicating).

- [ ] **Step 2: Run to verify they fail**

Run: `cargo test --manifest-path src/server/Cargo.toml room_tests::execute_move_registers`
Expected: compile error — `execute_move` takes 6 args, `frame` field missing.

- [ ] **Step 3: Implement the registry**

In `room.rs`:

1. Add near `MoveExecution`:
```rust
/// An in-flight token move retained for the duration of its client-side animation.
/// Serves two consumers: the per-token moving lock (`end_ms`) and the egress clip, which
/// reads `frame.mover_vision` as the mover's vision TIMELINE so concurrent moves can be
/// clipped against the recipient's vision at each sample's instant (`ws::move_clip`).
/// INVARIANT: `frame` is the full in-process `ServerMsg::MoveStream` — never a clipped copy.
pub(crate) struct ActiveStream {
    /// The user whose move this is (`MoveStream.mover`).
    pub mover: Uuid,
    /// The scene the token lives in (`MoveStream.scene`).
    pub scene: Uuid,
    /// Server epoch-ms the animation started (`MoveStream.start_server_ms`).
    pub start_ms: i64,
    /// Server epoch-ms the animation ends; the entry is expired when `now >= end_ms`.
    pub end_ms: i64,
    /// The full unclipped frame.
    pub frame: std::sync::Arc<ServerMsg>,
}
```
2. Add `pub frame: std::sync::Arc<ServerMsg>,` to `MoveExecution` with doc: `/// The full unclipped wire frame, already registered in the room's in-flight registry; the caller broadcasts it via `broadcast_aux_shared`.`
3. Change `moving: Mutex<HashMap<Uuid, i64>>` → `Mutex<HashMap<Uuid, ActiveStream>>`; rewrite its doc: "Per-token in-flight registry doubling as the moving lock: token → `ActiveStream`. Expired when `now_millis() >= end_ms` (lazy expiry, no timer)…".
4. Moving-lock check at `:590-596`: `if let Some(st) = moving.get(&token) { if now < st.end_ms { return Err(DataError::Forbidden); } }`.
5. `execute_move` gains `request_id: Uuid` as the last parameter. After `commit_ops_locked`, build the wire frame (move the `VisionSample` mapping out of `conn.rs` verbatim, including the `MAX_VISION_POLYGON_VERTS` cap and its fail-closed comment) into a private fn:
```rust
/// Map an executed move to its wire frame. Polygon vertex counts are capped at
/// `MAX_VISION_POLYGON_VERTS` (fail-closed under-reveal: truncation never over-reveals).
fn wire_move_stream(request_id: Uuid, token_id: Uuid, mover: Uuid, start_ms: i64, scene: Uuid, stop: (f64, f64), duration_ms: f64, samples: &[PosSamplePt], mover_vision: Option<Vec<VisionSamplePt>>, cost: f64, truncated: bool) -> ServerMsg
```
If clippy's `too_many_arguments` fires, group the last six into a `WireMoveInputs` struct rather than suppressing. Note `start_server_ms` must equal the `ts` argument `handle_move_request` passes (its single clock capture) — pass `ts` through, do NOT re-read the clock.
6. Replace the moving-lock insert block with:
```rust
let frame = std::sync::Arc::new(wire_move_stream(/* … */));
{
    let mut moving = self.moving.lock().await;
    moving.retain(|_, st| now < st.end_ms);
    moving.insert(token, ActiveStream {
        mover: ctx.user_id,
        scene: token_scene,
        start_ms: ts,
        end_ms: now + (duration_ms.ceil() as i64).max(1),
        frame: frame.clone(),
    });
}
```
and return `frame` in `MoveExecution`. (`mover_vision` is moved into the frame; `MoveExecution.mover_vision` stays populated for existing tests by cloning before the move, or — preferred — drop the `MoveExecution.samples`/`mover_vision` fields if no test reads them; grep `room_tests.rs` and `conn/tests.rs` for `.mover_vision`/`.samples` on a `MoveExecution` before deciding, and keep whichever are read.)
7. Accessors:
```rust
/// Unexpired in-flight frames moved by `mover` in `scene` — the mover's vision timelines the
/// egress clip evaluates a concurrent move against.
pub(crate) async fn mover_streams(&self, mover: Uuid, scene: Uuid, now: i64) -> Vec<std::sync::Arc<ServerMsg>> {
    let moving = self.moving.lock().await;
    moving.values().filter(|st| st.mover == mover && st.scene == scene && now < st.end_ms).map(|st| st.frame.clone()).collect()
}

/// Unexpired in-flight frames in `scene` moved by anyone other than `exclude_mover` —
/// re-clipped and re-emitted to a recipient whose own move just started. Excludes by
/// MOVER so a recipient's other in-flight token is never re-sent to them.
pub(crate) async fn concurrent_streams(&self, scene: Uuid, exclude_mover: Uuid, now: i64) -> Vec<std::sync::Arc<ServerMsg>> {
    let moving = self.moving.lock().await;
    moving.values().filter(|st| st.mover != exclude_mover && st.scene == scene && now < st.end_ms).map(|st| st.frame.clone()).collect()
}

/// Test-only direct registration (bypasses `execute_move`'s gate) for clip/egress tests.
#[cfg(test)]
pub(crate) async fn register_stream_for_test(&self, token: Uuid, stream: ActiveStream) {
    self.moving.lock().await.insert(token, stream);
}

/// Broadcast an already-shared out-of-band frame (see `broadcast_aux`).
pub(crate) fn broadcast_aux_shared(&self, msg: std::sync::Arc<ServerMsg>) {
    let _ = self.tx.send(RoomEvent::Other(msg));
}
```
and make `broadcast_aux` call `self.broadcast_aux_shared(Arc::new(msg))`.
8. In `conn.rs` `handle_move_request`: pass `request_id` to `execute_move`; delete the local frame construction and the `VisionSample` mapping; replace with `room.broadcast_aux_shared(exec.frame);`. Update the fn doc's `INVARIANT (mover_vision)` sentence to say the mapping/cap lives in `Room::execute_move`'s frame construction.
9. Fix every other `execute_move(` call site by appending a `request_id` argument (`Uuid::from_u128(0xF00D)` is fine where the test does not care). Expect 20 sites in `room_tests.rs` (verified by grep at plan time) plus `handle_move_request`.
10. **Zero-progress branch** (`room.rs` ~`:771-793`, `stop == start`, returns before the commit): it must ALSO build its frame through the same `wire_move_stream` helper (`duration_ms: 0.0`, one sample at `start`, `mover_vision: None`, `cost: 0.0`, `truncated: outcome.truncated`, `start_server_ms: ts`) and return it in `MoveExecution.frame`, so `handle_move_request` keeps broadcasting the zero-progress `MoveStream` exactly as today. It is NOT registered in `moving` (unchanged from today: a zero-duration move never held the lock, and there is no in-flight animation to re-clip against). Add a test in `room_tests.rs`: a first-step-blocked move returns `Ok` whose `frame` is a `MoveStream` with `duration_ms == 0.0`, and `mover_streams(player, scene, now)` stays empty. Reuse the existing fixture that produces a blocked first step (`execute_move_gate_inputs_come_from_the_tokens_own_scene` exercises this branch — copy its setup).

- [ ] **Step 4: Run tests**

Run: `cargo test --manifest-path src/server/Cargo.toml ws::`
Expected: all pass, including the two new ones and every pre-existing moving-lock test.

- [ ] **Step 5: Gates + commit**

Run: fmt, both clippy invocations, `pnpm lint:comments`, `pnpm lint:inline-tests`, `git diff --exit-code src/types/generated` (no wire change expected).
```bash
git commit -m "feat(ws): retain in-flight MoveStream frames in the room's moving registry" -- src/server/src/ws/room.rs src/server/src/ws/room/room_tests.rs src/server/src/ws/conn.rs
```

---

### Task 3: `clip_move_stream` clips against the recipient's vision timeline

**Files:**
- Modify: `src/server/src/ws/conn.rs:1099-1272` (`clip_move_stream`, `observer_vision_polys_for_scene` doc)
- Test: `src/server/src/ws/conn/tests.rs` (after the existing `clip_*` tests)

**Interfaces:**
- Consumes: Task 1 `clip_samples`/`TimelineStream`; Task 2 `Room::mover_streams`, `register_stream_for_test`, `ActiveStream`.
- Produces: `clip_move_stream` signature unchanged. Behaviour: for the clip target `T` (observer, or the GM's see-as target), `streams = room.mover_streams(T.user_id, *scene, now_millis())`; `static = observer_vision_polys_for_scene(T.user_id, *scene, room)`; `visible = clip_samples(samples, *start_server_ms, &static, &timeline)`. The GM see-as "not applicable" rule is unchanged: `static.is_empty() && streams.is_empty()` → full GM stream.

- [ ] **Step 1: Write the failing tests**

Add a helper and three tests to `conn/tests.rs` beside the `clip_*` suite:
```rust
/// Register an in-flight stream for `mover` in `scene` whose vision timeline is `vision`.
/// `start_ms` is the stream's `start_server_ms`; it stays unexpired for an hour.
async fn register_timeline(
    room: &crate::ws::room::Room,
    token: Uuid,
    mover: Uuid,
    scene: Uuid,
    start_ms: i64,
    vision: Vec<crate::ws::protocol::VisionSample>,
) {
    use crate::ws::room::ActiveStream;
    let frame = ServerMsg::MoveStream {
        request_id: Uuid::from_u128(0x7777),
        token_id: token,
        mover,
        scene,
        start_server_ms: start_ms as f64,
        duration_ms: 3_600_000.0,
        stop: [0.0, 0.0],
        samples: vec![crate::ws::protocol::PosSample { t_ms: 0.0, pos: [0.0, 0.0] }],
        mover_vision: Some(vision),
        cost: Some(0.0),
        truncated: Some(false),
    };
    room.register_stream_for_test(
        token,
        ActiveStream { mover, scene, start_ms, end_ms: start_ms + 3_600_000, frame: Arc::new(frame) },
    )
    .await;
}

/// A big square covering x∈[x0,x1], y∈[0,100].
fn band(x0: f64, x1: f64) -> Vec<Vec<[f64; 2]>> {
    vec![vec![[x0, 0.0], [x1, 0.0], [x1, 100.0], [x0, 100.0]]]
}

/// Observer at (50,50) behind a wall at x=100 — committed vision never sees x>100. The
/// observer's OWN in-flight sweep (started before A) sees x∈[100,300] from its second sample
/// (t=200 after its start). A's samples at (150,50)/(250,50) at A-times 0/200 fall at absolute
/// instants where the sweep shows sample 0 (band 0..100 → occluded) then sample 1 (band → visible).
#[tokio::test]
async fn clip_observer_mid_move_admits_samples_its_own_sweep_will_reveal() {
    use crate::ws::protocol::{PosSample, VisionSample};
    let wall_sys = json!({ "seg": { "x1": 100, "y1": -500, "x2": 100, "y2": 500 }, "blocksSight": true });
    let (room, _, obs_ctx, scene_id) = setup_clip_room(Some((50.0, 50.0)), Some(wall_sys), false).await;
    let now = crate::ws::time::now_millis();
    register_timeline(&room, Uuid::from_u128(0xE002), obs_ctx.user_id, scene_id, now, vec![
        VisionSample { t_ms: 0.0, polygons: band(0.0, 100.0) },
        VisionSample { t_ms: 200.0, polygons: band(100.0, 300.0) },
    ]).await;
    let frame = ServerMsg::MoveStream {
        request_id: Uuid::from_u128(1), token_id: Uuid::from_u128(2), mover: Uuid::from_u128(0xAABB),
        scene: scene_id, start_server_ms: (now + 100) as f64, duration_ms: 400.0, stop: [250.0, 50.0],
        samples: vec![
            PosSample { t_ms: 0.0, pos: [150.0, 50.0] },   // abs now+100 → sweep sample 0 → hidden
            PosSample { t_ms: 200.0, pos: [250.0, 50.0] }, // abs now+300 → sweep sample 1 → visible
        ],
        mover_vision: None, cost: Some(2.0), truncated: Some(false),
    };
    let out = clip_move_stream(&frame, &obs_ctx, None, &room).await.expect("one sample visible");
    let ServerMsg::MoveStream { samples, stop, duration_ms, mover_vision, cost, truncated, .. } = out else { panic!() };
    assert_eq!(samples, vec![PosSample { t_ms: 200.0, pos: [250.0, 50.0] }]);
    assert_eq!(stop, [250.0, 50.0]);
    assert!((duration_ms - 200.0).abs() < 1e-9);
    assert_eq!((mover_vision, cost, truncated), (None, None, None), "observer secrecy nulls unchanged");
}

/// Same geometry, but the observer's sweep starts AFTER every sample of A: the timeline never
/// applies and committed vision (blocked by the wall) suppresses the frame — the re-emit on the
/// observer's own move (Task 4) is what closes this ordering, not the clip.
#[tokio::test]
async fn clip_ignores_a_timeline_that_starts_after_the_move() {
    use crate::ws::protocol::{PosSample, VisionSample};
    let wall_sys = json!({ "seg": { "x1": 100, "y1": -500, "x2": 100, "y2": 500 }, "blocksSight": true });
    let (room, _, obs_ctx, scene_id) = setup_clip_room(Some((50.0, 50.0)), Some(wall_sys), false).await;
    let now = crate::ws::time::now_millis();
    register_timeline(&room, Uuid::from_u128(0xE002), obs_ctx.user_id, scene_id, now + 10_000,
        vec![VisionSample { t_ms: 0.0, polygons: band(100.0, 300.0) }]).await;
    let frame = ServerMsg::MoveStream {
        request_id: Uuid::from_u128(1), token_id: Uuid::from_u128(2), mover: Uuid::from_u128(0xAABB),
        scene: scene_id, start_server_ms: now as f64, duration_ms: 400.0, stop: [250.0, 50.0],
        samples: vec![PosSample { t_ms: 0.0, pos: [150.0, 50.0] }, PosSample { t_ms: 200.0, pos: [250.0, 50.0] }],
        mover_vision: None, cost: Some(2.0), truncated: Some(false),
    };
    assert!(clip_move_stream(&frame, &obs_ctx, None, &room).await.is_none());
}

/// GM see-as: the target's timeline, not the GM's own, drives the clip.
#[tokio::test]
async fn clip_gm_see_as_uses_the_targets_timeline() {
    use crate::ws::protocol::{PosSample, VisionSample};
    let wall_sys = json!({ "seg": { "x1": 100, "y1": -500, "x2": 100, "y2": 500 }, "blocksSight": true });
    let (room, gm_ctx, obs_ctx, scene_id) = setup_clip_room(Some((50.0, 50.0)), Some(wall_sys), false).await;
    let now = crate::ws::time::now_millis();
    register_timeline(&room, Uuid::from_u128(0xE002), obs_ctx.user_id, scene_id, now,
        vec![VisionSample { t_ms: 0.0, polygons: band(100.0, 300.0) }]).await;
    let frame = ServerMsg::MoveStream {
        request_id: Uuid::from_u128(1), token_id: Uuid::from_u128(2), mover: Uuid::from_u128(0xAABB),
        scene: scene_id, start_server_ms: (now + 50) as f64, duration_ms: 400.0, stop: [250.0, 50.0],
        samples: vec![PosSample { t_ms: 0.0, pos: [150.0, 50.0] }, PosSample { t_ms: 200.0, pos: [250.0, 50.0] }],
        mover_vision: None, cost: Some(2.0), truncated: Some(false),
    };
    let out = clip_move_stream(&frame, &gm_ctx, Some(obs_ctx), &room).await.expect("target sees both");
    let ServerMsg::MoveStream { samples, cost, .. } = out else { panic!() };
    assert_eq!(samples.len(), 2);
    assert_eq!(cost, None, "a see-as clip narrows the GM to observer secrecy");
}
```
Confirm `Arc` and `json` are imported at the top of `conn/tests.rs` (`json` is; add `use std::sync::Arc;` if absent — it is used at `:1983` so it is present).

- [ ] **Step 2: Run to verify they fail**

Run: `cargo test --manifest-path src/server/Cargo.toml clip_observer_mid_move clip_gm_see_as_uses`
Expected: the first and third FAIL (frame suppressed / only committed vision used); `clip_ignores_a_timeline_that_starts_after_the_move` passes already (it documents the boundary).

- [ ] **Step 3: Implement**

In `clip_move_stream`, replace the block from `let clip_polys: Vec<Vec<…>> = if …` through the `visible` computation with:
```rust
// Whose vision this recipient is clipped against: their own, or (a GM see-as) the target's.
let target_user = if ctx.world_role == crate::data::document::WorldRole::Gm {
    match see_as {
        Some(target) => target.user_id,
        None => return Some(full_gm_stream()),
    }
} else {
    ctx.user_id
};
let now = crate::ws::time::now_millis();
// Committed-position vision (the at-rest gate) and the target's in-flight sweep timelines.
// Both reads drop their locks before this function's caller awaits `sink.send`.
let static_polys = observer_vision_polys_for_scene(target_user, *scene, room).await;
let timeline_frames = room.mover_streams(target_user, *scene, now).await;
if ctx.world_role == crate::data::document::WorldRole::Gm && static_polys.is_empty() && timeline_frames.is_empty() {
    // See-as target has no vision source in this scene → not applicable → full GM stream.
    return Some(full_gm_stream());
}
let timelines: Vec<crate::ws::move_clip::TimelineStream<'_>> = timeline_frames
    .iter()
    .filter_map(|f| match f.as_ref() {
        ServerMsg::MoveStream { start_server_ms, mover_vision: Some(v), .. } => {
            Some(crate::ws::move_clip::TimelineStream { start_server_ms: *start_server_ms, vision: v })
        }
        _ => None,
    })
    .collect();
let visible = crate::ws::move_clip::clip_samples(samples, *start_server_ms, &static_polys, &timelines);
```
Keep everything after (`if visible.is_empty() { return None; }` … the clipped frame construction) unchanged. Update the fn doc: add
```
/// INVARIANT (timeline-clip): each sample is judged against the clip target's vision AT THAT
///   SAMPLE'S INSTANT — the target's own in-flight `mover_vision` sweep (`Room::mover_streams`,
///   `ws::move_clip`) while one is active, the committed-position vision otherwise. The timeline
///   is the target's OWN vision (already sent to them), so this never admits a sample their fog
///   will not show. A target whose move starts AFTER this frame was clipped is served by the
///   egress re-emit (`egress_loop`'s own-move arm), not by this function.
```
and rewrite the existing `INVARIANT (see-as-scene-exact)` sentence so "yields zero polygons" reads "yields zero committed polygons AND no in-flight timeline".

- [ ] **Step 4: Run the whole clip suite**

Run: `cargo test --manifest-path src/server/Cargo.toml clip_`
Expected: every pre-existing `clip_*` test passes unchanged plus the three new ones.

- [ ] **Step 5: Gates + commit**

fmt, both clippy invocations, `pnpm lint:comments`.
```bash
git commit -m "feat(ws): clip MoveStream samples against the recipient's own vision timeline" -- src/server/src/ws/conn.rs src/server/src/ws/conn/tests.rs
```

---

### Task 4: Egress re-emits in-flight streams when the recipient's own move starts

**Files:**
- Modify: `src/server/src/ws/conn.rs:1639-1668` (the `ServerMsg::MoveStream` egress arm)
- Test: `src/server/src/ws/conn/tests.rs` (egress integration, modelled on `egress_lag_triggers_resync_and_converges` at `:1-190` — reuse its `GatedSink` and `msg_text` helpers; hoist `GatedSink` to file scope if it is currently local to that test)

**Interfaces:**
- Consumes: Task 2 `Room::concurrent_streams`, `register_stream_for_test`; Task 3 `clip_move_stream`.
- Produces: no new API. Behaviour: after forwarding a `MoveStream` whose `mover` is the connection's user (or its active see-as target), the loop forwards `clip_move_stream(other, …)` for each `other` in `room.concurrent_streams(scene, clip_target, now)` (every in-flight stream in the scene by a DIFFERENT mover), skipping `None`.

- [ ] **Step 1: Write the failing egress test**

```rust
/// When the observer's OWN move starts, every other in-flight stream in the scene is re-clipped
/// against the new timeline and re-emitted to that connection only — with the other stream's
/// original request_id so the client overwrites its keyed playback in place.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn egress_reemits_concurrent_streams_when_the_recipients_own_move_starts() {
    use crate::ws::protocol::{PosSample, VisionSample};
    use tokio::sync::Semaphore;

    let wall_sys = json!({ "seg": { "x1": 100, "y1": -500, "x2": 100, "y2": 500 }, "blocksSight": true });
    let (room, _, obs_ctx, scene_id) = setup_clip_room(Some((50.0, 50.0)), Some(wall_sys), false).await;
    let repo = /* the SqliteRepository setup_clip_room built — extend setup_clip_room to also return it, or rebuild the room here the same way the lag test does */;
    let now = crate::ws::time::now_millis();

    // A: a stranger's move entirely behind the wall, in flight since `now`.
    let a_req = Uuid::from_u128(0xA11);
    let a_frame = ServerMsg::MoveStream {
        request_id: a_req, token_id: Uuid::from_u128(0xA), mover: Uuid::from_u128(0xAABB), scene: scene_id,
        start_server_ms: now as f64, duration_ms: 3_000.0, stop: [250.0, 50.0],
        samples: vec![PosSample { t_ms: 0.0, pos: [150.0, 50.0] }, PosSample { t_ms: 1_000.0, pos: [250.0, 50.0] }],
        mover_vision: None, cost: Some(2.0), truncated: Some(false),
    };
    room.register_stream_for_test(Uuid::from_u128(0xA), crate::ws::room::ActiveStream {
        mover: Uuid::from_u128(0xAABB), scene: scene_id, start_ms: now, end_ms: now + 3_000, frame: Arc::new(a_frame),
    }).await;

    // Spawn the observer's egress with plenty of credits.
    let (rx, current_seq) = room.subscribe();
    let credits = Arc::new(Semaphore::new(64));
    let (out_tx, mut out_rx) = mpsc::unbounded_channel::<Message>();
    let (_etx, erx) = mpsc::channel::<Egress>(8);
    let egress = tokio::spawn(egress_loop(
        GatedSink { out: out_tx, credits, acquiring: None }, rx, erx,
        EgressConnState { room: room.clone(), repo: repo.clone(), ctx: obs_ctx, current_seq,
            modules_dir: std::path::PathBuf::from("nonexistent-modules-dir"),
            module_scan_cache: Arc::new(crate::modules::ModuleScanCache::new()) },
    ));
    let welcome = tokio::time::timeout(std::time::Duration::from_secs(5), out_rx.recv()).await.unwrap().unwrap();
    assert_eq!(serde_json::from_str::<serde_json::Value>(msg_text(&welcome)).unwrap()["type"], "welcome");

    // R (the observer) starts a move at now+100 whose sweep sees behind the wall from t=0.
    let r_start = now + 100;
    let r_frame = ServerMsg::MoveStream {
        request_id: Uuid::from_u128(0xB11), token_id: Uuid::from_u128(0xE002), mover: obs_ctx.user_id, scene: scene_id,
        start_server_ms: r_start as f64, duration_ms: 2_000.0, stop: [60.0, 50.0],
        samples: vec![PosSample { t_ms: 0.0, pos: [50.0, 50.0] }, PosSample { t_ms: 2_000.0, pos: [60.0, 50.0] }],
        mover_vision: Some(vec![VisionSample { t_ms: 0.0, polygons: band(0.0, 300.0) }]),
        cost: Some(0.1), truncated: Some(false),
    };
    let r_arc = Arc::new(r_frame);
    room.register_stream_for_test(Uuid::from_u128(0xE002), crate::ws::room::ActiveStream {
        mover: obs_ctx.user_id, scene: scene_id, start_ms: r_start, end_ms: r_start + 2_000, frame: r_arc.clone(),
    }).await;
    room.broadcast_aux_shared(r_arc);

    let first = tokio::time::timeout(std::time::Duration::from_secs(5), out_rx.recv()).await.unwrap().unwrap();
    let first: serde_json::Value = serde_json::from_str(msg_text(&first)).unwrap();
    assert_eq!(first["type"], "move_stream");
    assert_eq!(first["request_id"], json!(Uuid::from_u128(0xB11)), "own move forwarded first, unchanged");
    assert!(first["mover_vision"].is_array());

    let second = tokio::time::timeout(std::time::Duration::from_secs(5), out_rx.recv()).await.unwrap().unwrap();
    let second: serde_json::Value = serde_json::from_str(msg_text(&second)).unwrap();
    assert_eq!(second["type"], "move_stream");
    assert_eq!(second["request_id"], json!(a_req), "A re-emitted under its original request_id");
    // A's sample at t=1000 (abs now+1000) is inside R's sweep band → admitted; t=0 (abs now)
    // precedes R's sweep start → committed vision (walled) → dropped.
    assert_eq!(second["samples"].as_array().unwrap().len(), 1);
    assert_eq!(second["samples"][0]["t_ms"], json!(1000.0));
    assert!(second["mover_vision"].is_null() && second["cost"].is_null() && second["truncated"].is_null());

    egress.abort();
}
```
`setup_clip_room` currently drops its `repo` binding; extend it to return `Arc<SqliteRepository>` as a fifth tuple element and update its 9 existing call sites (`let (room, _, obs_ctx, scene_id, _) = …`; the `async fn setup_clip_room(` definition at `:1978` is not a call site). `GatedSink` and `msg_text` are currently local to `egress_lag_triggers_resync_and_converges` — hoist them to file scope in `conn/tests.rs`.

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test --manifest-path src/server/Cargo.toml egress_reemits`
Expected: FAIL — times out waiting for the second frame.

- [ ] **Step 3: Implement the re-emit**

In the egress `ServerMsg::MoveStream { .. }` arm, replace `match clip_move_stream(inner.as_ref(), &ctx, see_as, &room).await { … }` with:
```rust
let mut failed = match clip_move_stream(inner.as_ref(), &ctx, see_as, &room).await {
    Some(out) => sink.send(text(&out)).await.is_err(),
    None => false,
};
// Own-move re-emit: the clip target's vision timeline just changed, so every OTHER in-flight
// stream in this scene is re-clipped against it and re-sent under its original request_id
// (the client overwrites keyed playback in place). Serves the ordering where the recipient's
// move starts AFTER the other stream was clipped — the clip itself cannot widen a frame
// already sent. Delivered only to this connection; other recipients' timelines are unchanged.
if !failed {
    if let ServerMsg::MoveStream { mover, scene, .. } = inner.as_ref() {
        let clip_target = see_as.map(|t| t.user_id).unwrap_or(ctx.user_id);
        if *mover == clip_target {
            let now = crate::ws::time::now_millis();
            for other in room.concurrent_streams(*scene, clip_target, now).await {
                if let Some(out) = clip_move_stream(other.as_ref(), &ctx, see_as, &room).await {
                    if sink.send(text(&out)).await.is_err() { failed = true; break; }
                }
            }
        }
    }
}
failed
```
(`see_as` is `Option<PermissionContext>`; `PermissionContext` is `Copy` — it is passed by value repeatedly in the existing code. If it is not `Copy`, clone it into the loop.)

- [ ] **Step 4: Run tests**

Run: `cargo test --manifest-path src/server/Cargo.toml ws::`
Expected: all pass.

- [ ] **Step 5: Gates + commit**

fmt, both clippy invocations, `pnpm lint:comments`.
```bash
git commit -m "feat(ws): re-emit in-flight MoveStreams to a recipient whose own move starts" -- src/server/src/ws/conn.rs src/server/src/ws/conn/tests.rs
```

---

### Task 5: Client re-emit playback test, docs, skills, full gate battery

**Files:**
- Test: `src/client/render/src/token-animator.test.ts`
- Modify: `docs/PLAN.md` (M10 track note), `docs/TODO.md` (bucket-C item 1 → DONE except the parked residual), `docs/design/ARCHITECTURE.md` (no change — invariant 11 already landed)
- Modify (plugin checkout): `~/.claude/skills/shadowcat-codebase/skills/shadowcat-codebase-scene-rendering/SKILL.md` and `…/shadowcat-codebase-realtime-sync/SKILL.md`

- [ ] **Step 1: Write the client overwrite test (may already be covered — grep first)**

Grep `token-animator.test.ts` for a test that calls `animateSamples` twice for the same id mid-playback and asserts the second sample set wins with server-aligned catch-up. If absent, add:
```ts
test("a second animateSamples for the same token replaces playback in place at the server-aligned elapsed", () => {
  const a = new TokenAnimator(/* same constructor args the neighbouring tests use */);
  a.animateSamples("tok", [{ tMs: 0, pos: [0, 0] }, { tMs: 1000, pos: [100, 0] }], 1000, 0, () => 0);
  a.tick(400); // 40% along the first set
  // Re-emitted (wider) frame, same start clock; the client is now 400ms in.
  a.animateSamples("tok", [{ tMs: 0, pos: [0, 0] }, { tMs: 500, pos: [50, 0] }, { tMs: 1000, pos: [100, 0] }], 1000, 0, () => 400);
  a.tick(0);
  expect(a.isHidden("tok")).toBe(false);
  expect(a.currentPosition("tok")?.[0]).toBeCloseTo(40, 5);
});
```
Use the file's real accessor names for elapsed ticking and position reads (`tick`/`currentPosition` are placeholders for whatever `token-animator.test.ts` already uses — copy from a neighbouring test).

- [ ] **Step 2: Run** `pnpm --filter @shadowcat/render test -- token-animator` — PASS (no production change expected; if it fails, STOP and report: the spec's "no client change" claim is false).

- [ ] **Step 3: Docs**

`docs/TODO.md` bucket-C item 1: replace the body with "DONE (2026-08-27, spec `…/2026-08-27-move-stream-live-clip-design.md`): observer's own-move timeline clip + re-emit. Residual, parked: third-party moving light source opening a sightline mid-walk still reveals at that mover's stop — needs the observer's vision recomputed per sample of the light-carrying move; cost only on request."
`docs/PLAN.md`: under the M10 track's streamed-vision entry add one sentence naming the timeline clip + re-emit and invariant 11.

- [ ] **Step 4: Skill updates (plugin checkout)**

- `shadowcat-codebase-scene-rendering`: in the streamed-continuous-vision bullet, state the clip is per-sample-instant against the recipient's own `mover_vision` timeline (`ws::move_clip`, `Room::mover_streams`), and the re-emit on own-move (`egress_loop`); note the parity fixture and `chooseVisionSample`.
- `shadowcat-codebase-realtime-sync`: `Room.moving` is now the `ActiveStream` registry (moving lock + in-flight frames); `broadcast_aux_shared`.
- Dispatch `shadowcat-codebase:shadowcat-spec-reviewer` (effort high) on the skill diffs. Run `node scripts/check-skill-symbol-refs-cli.mjs`, `node scripts/check-skill-api-refs-cli.mjs`, `pnpm run test:scripts` — read exit codes from files. Commit + push inside `~/.claude/skills/shadowcat-codebase/`.

- [ ] **Step 5: Full gate battery** — every line of the Gate battery section above, exit codes read from files. Then `graphify update .`.

- [ ] **Step 6: Commit**

```bash
git commit -m "docs: move-stream live clip shipped; residual light-source case parked" -- docs/TODO.md docs/PLAN.md src/client/render/src/token-animator.test.ts
```

---

## Self-review

- Spec §2.1 → Task 2. §2.2 (per-sample timeline, chosen-sample rule, parity test) → Tasks 1, 3. §2.3 (re-emit, own-move and see-as) → Task 4. §2.4 (sub-sample timing) → accepted, no task. §4 tests: red-first mid-move test (Task 3 step 2), both orderings (Task 3 + Task 4), see-as (Task 3), two owned tokens (Task 1 `timeline_polys_at` union test — a Room-level two-token test is not added; the union is exercised at the pure layer and `mover_streams` returns all of a user's streams), expiry (Task 2), parity (Task 1), re-emit delivery scope (Task 4), secrecy regression (Task 3 step 4, Task 4 asserts nulls).
- Types: `TimelineStream { start_server_ms: f64, vision: &[VisionSample] }`, `ActiveStream { mover, scene, start_ms: i64, end_ms: i64, frame: Arc<ServerMsg> }`, `mover_streams(mover, scene, now) -> Vec<Arc<ServerMsg>>`, `concurrent_streams(scene, exclude_mover, now)` — used consistently in Tasks 2–4.
- Open implementer decisions flagged in-task (not placeholders): whether `MoveExecution` keeps `samples`/`mover_vision` (Task 2 step 3.6, decided by grep), `GatedSink` hoisting and `setup_clip_room` repo return (Task 4), accessor names in the client test (Task 5).
