### Task 5 Report: Client MoveStream playback — broadcast-driven, time-synced, gap-aware

**Status:** COMPLETE (including review-finding fixes)

**Original commit:** `7576efc`
**Skill commit:** `5bebf7a`
**Review-fix commit:** see below

---

## Files changed in 7576efc / 5bebf7a

| File | Change |
|---|---|
| `src/client/render/src/token-animator.ts` | New file: `TokenAnimator.animateSamples` (time-tagged sample playback, catch-up, gap/occlusion detection) |
| `src/client/render/src/token-animator.test.ts` | New tests for `animateSamples`: interpolation, gap hide/reveal, catch-up, settle |
| `src/client/render/src/token-view.ts` | `animateSamples` delegation; `push` respects `isHidden` |
| `src/client/render/src/types.ts` | `MoveSample` / `MoveVisionSample` interfaces |
| `src/client/render/src/engine.ts` | `animateSamples` forwarding through `RenderEngine` |
| `src/client/core/src/ws-client.ts` | `onMoveStream` listener (survives reconnects; not cleared in `failPending`); `moveRequest` promise resolves on `move_stream` frame |
| `src/client/core/src/ws-client.test.ts` | `onMoveStream` + `moveRequest` round-trip tests |
| `src/client/core/src/wire.ts` | Remove `MoveExecutedMsg`; add `MoveStreamMsg` |
| `src/client/core/src/index.ts` | Export updates |
| `src/client/shell/src/lib/worldSession.svelte.ts` | `onMoveStream` → `sceneInteraction.animateSamples` wiring |
| `src/client/ui-kit/src/appContext.ts` | `animateSamples` on `AppContext` seam |
| `src/client/ui-kit/src/sceneInteraction.ts` | `animateSamples` on `SceneInteractionBridge` |
| `src/client/ui-kit/src/sceneInteraction.test.ts` | Bridge delegation test |
| `src/client/ui-kit/src/__fixtures__/fakeSceneHost.ts` | Stub update |
| `src/modules/scene-tools/src/controller.svelte.ts` | Route-commit no longer calls `animateAlongPath` on resolve (animation is broadcast-driven) |
| `src/modules/scene-tools/src/measure-tool.test.ts` | 3 tests rewritten: `animateAlongPath`-assertion → `clearRoute`-count assertion |
| `src/server/src/ws/protocol.rs` | Retire `MoveExecuted`; add `MoveStream` |
| `src/types/generated/ServerMsg.ts` | Regenerated: `MoveExecuted` removed, `MoveStream` added |
| `eslint.config.js` | Add `target/` ignore + `argsIgnorePattern` for `_`-prefixed params (pre-existing lint failures) |
| `.claude/skills/shadowcat-codebase-realtime-sync/SKILL.md` | Document `MoveStream` broadcast, `animateSamples` wiring, gap threshold |

---

## Review-finding fixes

### Important 1 — Nominal-interval gap threshold (`token-animator.ts`)

**Problem:** gap threshold was `durationMs / 2`. With server sampling at ~3 samples/cell a
mid-path occlusion shorter than half the total animation duration was undetected, letting the
token visibly slide through hidden cells (secrecy violation).

**Fix:** replaced `durationMs / 2` with `computeGapThreshold(samples)`:
- Computes `minConsecutiveDelta` = minimum positive consecutive tMs delta across all pairs.
- `gapThreshold = minConsecutiveDelta * 1.5`.
- Degenerate: < 3 samples → `Infinity` (no interior gap distinguishable from a single delta).

### Important 2 — `samplesAnim` precedence over `setTarget` ease (`token-animator.ts`)

**Problem:** the authoritative position `Event` arrives before `MoveStream` (normal server
ordering), so `reconcile() → setTarget` registers an ease-to-stop entry in `this.anim` before
`animateSamples` runs. In `tick()` the `anim` loop overwrote the sample position every frame,
so the token eased straight to `stop` and the broadcast trajectory never showed.

**Fixes:**
- `animateSamples`: calls `this.anim.delete(id)` before `this.samplesAnim.set(id, sa)` —
  cancels the competing ease.
- `setTarget`: early-returns if `this.samplesAnim.has(id)` — handles reverse ordering (MoveStream
  arrives before Event).
- `tick()` anim loop: explicit `if (this.samplesAnim.has(id)) continue` guard.

### Minor 3 — Token hide is a transition, not per-tick (`token-view.ts`)

**Problem:** `push` called `backend.removeToken(id)` every tick while hidden.

**Fix:** added `private readonly wasHidden = new Set<string>()` to `TokenView`. `push` now
calls `removeToken` once on the visible→hidden transition; clears `wasHidden` on gap exit and
on token removal from `reconcile`.

### Minor 4 — Corrected task-5-report.md content

This file previously contained Task 5's server `conn.rs` report (commits cf41495/cd582f1).
Overwritten with this correct client-work report.

### Minor 5 — Skill and test description updated

- `shadowcat-codebase-realtime-sync/SKILL.md`: replaced `durationMs/2` with the
  nominal-interval-based description; documented `animateSamples`/`setTarget` precedence rules.
- `token-animator.test.ts`: gap test description updated from `durationMs/2` formula to
  "nominal-interval-based threshold"; test body updated to use 3-sample fixture (required for
  interior gap detection under the new formula).

---

## Tests added

### `token-animator.test.ts` (in `describe("TokenAnimator.animateSamples")`)

| Test | What it verifies |
|---|---|
| "hides the token across an occlusion gap (nominal-interval-based threshold)" | 3-sample: contiguous run 0→100 visible; gap 100→900 hidden; reveals past tMs=900 |
| "partial-occlusion: mid-path gap detected with nominal-interval threshold, contiguous runs stay visible" | 6-sample spec case (tMs 0,100,200,600,700,800): visible in both runs, hidden in 200→600 gap |
| "setTarget before animateSamples: sample animation takes precedence over ease-to-stop" | Simulates Event-before-MoveStream ordering; asserts token follows y=500 sample path not y=0 ease |

---

## Test commands + output

```
# Server ws::protocol suite
cd src/server && cargo test ws::protocol
test result: ok. 18 passed; 0 failed

# Full client suite
pnpm -r test
  @shadowcat/core:         167 passed
  @shadowcat/render:       121 passed  ← includes all 3 new tests
  @shadowcat/ui-kit:        17 passed
  @shadowcat/shell:         27 passed
  scene-tools:              45 passed
  (other modules):          47 passed
  TOTAL: 424 passed, 0 failed

# Typecheck
pnpm -r typecheck
  All packages: 0 errors, 0 warnings

# Lint
pnpm lint
  (no output = clean)
```

---

## Concerns

None. All review findings addressed. The `eslint.config.js` change is left as-is (necessary
pre-existing build fix; not worth history churn on an unmerged branch).
