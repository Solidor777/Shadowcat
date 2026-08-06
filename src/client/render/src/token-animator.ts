import type { TokenTransform } from "./types";
import { applyEasing, type EasingMode } from "./easing";

/** Below this distance (px) a component is treated as coincident. */
const EPSILON = 0.01;

/** A server-sampled position entry: elapsed ms from startServerMs + scene-coord position. */
export interface MoveSample {
  tMs: number;
  pos: [number, number];
}

/** State for a sample-driven playback (animateSamples). Separate from the polyline Anim so
 * the two modes can coexist independently (e.g. a route-commit walk alongside a broadcast play). */
interface SamplesAnim {
  samples: MoveSample[];
  /** Accumulated elapsed time (ms) from the start of the animation; pre-seeded for catch-up. */
  elapsed: number;
  durationMs: number;
  /** Gap threshold: consecutive tMs gaps exceeding this are treated as occlusion spans.
   * Computed as minConsecutiveDelta × 1.5 (nominal-interval-based); Infinity when fewer than
   * 3 samples are present (no interior gap distinguishable with ≤ 1 delta). */
  gapThreshold: number;
}

/** Animation tuning resolved from `world-settings.animation` + the active grid. */
export interface AnimationConfig {
  speedCellsPerSec: number;
  easing: EasingMode;
  /** Pixels per grid cell (grid.size); converts pixel distance to cells for duration. */
  cellSize: number;
}

const lerp = (a: number, b: number, t: number): number => a + (b - a) * t;

/** Compute the gap detection threshold for occlusion spans from a sample list.
 * Strategy: minimum positive consecutive inter-sample interval × 1.5. Contiguous (non-occluded)
 * sample pairs produce ≈ the nominal inter-sample interval; occlusion gaps are larger and exceed
 * this threshold. Replaces the durationMs/2 heuristic, which only catches gaps larger than half
 * the total animation (misses mid-path occlusion spans shorter than that — secrecy violation).
 * Degenerate: fewer than 3 samples (≤ 1 interior delta) → Infinity (gap detection disabled;
 * no interior segment is distinguishable when only one delta exists).
 * Accepted limitation: when a clip leaves EXACTLY 2 visible samples (visible only at the move's
 * start and end, fully occluded in between), there is no third sample to derive a nominal
 * interval from, so the single interval is never flagged as a gap and the token is linearly
 * interpolated straight across the occluded span instead of hidden. This is NOT a secrecy leak —
 * the fog mask (the actual secrecy gate, see fog-is-the-secrecy-gate-fail-closed) already covers
 * any interpolated position outside the observer's vision, and both endpoints are legitimately
 * visible to this observer. A robust fix would need the animator to know the server's nominal
 * inter-sample interval independent of the clipped sample count (e.g. threaded through from
 * `SAMPLES_PER_CELL`/duration), which this pure client-side heuristic cannot
 * derive from 2 points alone without risking new false-positive/negative gap calls.
 * @param samples The full (already server-clipped) sample list for one MoveStream playback.
 * @returns The gap threshold in ms (minimum positive consecutive inter-sample delta × 1.5), or
 * `Infinity` when fewer than 3 samples are present (gap detection disabled).
 * @example
 * ```
 * // module-private; not exported from @shadowcat/render's index.ts
 * computeGapThreshold([{ tMs: 0, pos: [0, 0] }, { tMs: 100, pos: [1, 0] }, { tMs: 200, pos: [2, 0] }]); // 150
 * ```
 */
function computeGapThreshold(samples: MoveSample[]): number {
  if (samples.length < 3) return Infinity;
  let minDelta = Infinity;
  for (let i = 0; i < samples.length - 1; i++) {
    const d = samples[i + 1].tMs - samples[i].tMs;
    if (d > 0) minDelta = Math.min(minDelta, d);
  }
  // All deltas zero (degenerate timestamp data) → disable gap detection.
  if (!Number.isFinite(minDelta)) return Infinity;
  return minDelta * 1.5;
}

interface Anim {
  /** Polyline in scene px; `poly[0]` is the start captured at (re)target time. */
  poly: [number, number][];
  segLen: number[];
  total: number;
  elapsed: number;
  duration: number;
  startRot: number;
  finalRot: number;
  easing: EasingMode;
  /** True for an explicit route walk; gates the optimistic-vertex ignore rule. */
  pathDriven: boolean;
  /** Lowest vertex index the walk is still heading toward (monotonic). */
  segIndex: number;
}

/** Holds each token's rendered transform and advances it toward the document-authoritative
 * target along an eased polyline. Duration = pathDistanceCells / speedCellsPerSec.
 * New tokens snap; moves tween; a newer authoritative position retargets in place. */
export class TokenAnimator {
  private cur = new Map<string, TokenTransform>();
  private anim = new Map<string, Anim>();
  private samplesAnim = new Map<string, SamplesAnim>();
  /** Token ids currently in an occlusion gap (server-clipped visibility span). */
  private hidden = new Set<string>();
  private cfg: AnimationConfig = { speedCellsPerSec: 6, easing: "easeInOut", cellSize: 100 };

  /** Replace the tween-duration tuning (speed/easing/cellSize). Affects only FUTURE tweens started
   * by `startAnim` — an animation already in progress keeps the duration computed from the config
   * that was active when it started.
   * @param cfg The new speed/easing/cellSize tuple.
   * @example
   * ```ts
   * import { TokenAnimator } from "@shadowcat/render";
   *
   * const animator = new TokenAnimator();
   * animator.setConfig({ speedCellsPerSec: 6, easing: "easeInOut", cellSize: 100 });
   * ```
   */
  setConfig(cfg: AnimationConfig): void {
    this.cfg = cfg;
  }
  /** Whether `id` has a rendered transform recorded (i.e. `get(id)` would return non-undefined).
   * @param id The token id to check.
   * @returns True if a transform is being tracked for this token.
   * @example
   * ```ts
   * import { TokenAnimator } from "@shadowcat/render";
   *
   * const animator = new TokenAnimator();
   * animator.has("token-1"); // false
   * ```
   */
  has(id: string): boolean {
    return this.cur.has(id);
  }
  /** The token's current rendered transform (interpolated mid-tween), or `undefined` if untracked.
   * @param id The token id.
   * @returns The current `{x,y,rotation}`, or `undefined` when no transform has been set.
   * @example
   * ```ts
   * import { TokenAnimator } from "@shadowcat/render";
   *
   * const animator = new TokenAnimator();
   * animator.get("token-1"); // undefined
   * ```
   */
  get(id: string): TokenTransform | undefined {
    return this.cur.get(id);
  }
  /** Drop all tracked state for `id` — transform, polyline tween, sample playback, and hidden flag.
   * A subsequent `setTarget`/`animateAlongPath` for the same id starts fresh (snaps rather than
   * tweening from stale state).
   * @param id The token id to forget.
   * @example
   * ```ts
   * import { TokenAnimator } from "@shadowcat/render";
   *
   * const animator = new TokenAnimator();
   * animator.remove("token-1");
   * ```
   */
  remove(id: string): void {
    this.cur.delete(id);
    this.anim.delete(id);
    this.samplesAnim.delete(id);
    this.hidden.delete(id);
  }

  /** True when the token is in a server-defined occlusion gap and should not be rendered.
   * @param id The token id to check.
   * @returns True while `id` is inside an occlusion gap set by `applySamplesAt`.
   * @example
   * ```ts
   * import { TokenAnimator } from "@shadowcat/render";
   *
   * const animator = new TokenAnimator();
   * animator.isHidden("token-1"); // false
   * ```
   */
  isHidden(id: string): boolean {
    return this.hidden.has(id);
  }

  /** Begin sample-driven playback from a server broadcast MoveStream. Interpolates the token
   * position between adjacent samples by tMs on the server-aligned clock; hides the token during
   * spans whose tMs gap exceeds the nominal-interval-based threshold (minConsecutiveDelta × 1.5).
   * Cancels any competing ease-to-stop Anim entry for this id: handles the typical server ordering
   * where the authoritative position Event (→ setTarget) arrives before the MoveStream broadcast.
   * Catch-up: if the server clock (serverNow) is ahead of startServerMs, playback begins from the
   * matching elapsed offset. REPLACES, never queues or blends: a second call for an id already
   * mid-playback overwrites the `samplesAnim` entry outright, discarding the prior playback's
   * `elapsed`/progress — the token restarts from this call's own catch-up offset against the new
   * `samples`, not from wherever the discarded playback had reached.
   * @param id The token id to animate.
   * @param samples The (already server-clipped) position samples to play back.
   * @param durationMs Total playback duration in ms.
   * @param startServerMs The server clock time (ms) at which this playback begins.
   * @param serverNow Optional injected server clock, used once at call time as
   * `Math.max(0, serverNow() - startServerMs)` to compute the initial catch-up offset; when absent,
   * elapsed starts at `0` (no catch-up assumed — NOT a `Date.now` fallback). Subsequent ticks
   * advance `elapsed` by `tick`'s own `dtMs`, never by re-reading this accessor.
   * @example
   * ```ts
   * import { TokenAnimator } from "@shadowcat/render";
   *
   * const animator = new TokenAnimator();
   * animator.animateSamples("token-1", [{ tMs: 0, pos: [0, 0] }, { tMs: 500, pos: [1, 1] }], 500, Date.now());
   * ```
   */
  animateSamples(
    id: string,
    samples: MoveSample[],
    durationMs: number,
    startServerMs: number,
    serverNow?: () => number,
  ): void {
    if (samples.length === 0) return;
    const initialElapsed = serverNow ? Math.max(0, serverNow() - startServerMs) : 0;
    const gapThreshold = computeGapThreshold(samples);
    const sa: SamplesAnim = { samples, elapsed: initialElapsed, durationMs, gapThreshold };
    // Cancel any competing ease-to-stop Anim: samplesAnim takes exclusive precedence.
    // Coupling: the authoritative position Event arrives before the MoveStream broadcast
    // (normal server ordering), so reconcile() → setTarget already registered an ease entry.
    this.anim.delete(id);
    this.samplesAnim.set(id, sa);
    // Ensure cur exists so tick + push work even if the token was never setTarget-ed locally.
    if (!this.cur.has(id)) {
      this.cur.set(id, { x: samples[0].pos[0], y: samples[0].pos[1], rotation: 0 });
    }
    // Apply initial position immediately (no wait for tick).
    this.applySamplesAt(id, sa);
  }

  /** Retarget `id` to `t`, the latest document-authoritative transform (from `reconcile()`).
   * No-ops while sample-driven playback (`animateSamples`) is live for this id — the broadcast
   * trajectory takes exclusive precedence over the authoritative Event's retarget (handles the
   * typical MoveStream-before-Event server ordering; see `animateSamples`). A brand-new id snaps
   * immediately (no prior transform to tween from). While a path-driven walk
   * (`animateAlongPath`) is in progress, a `t` that matches an ahead vertex on the live polyline is
   * treated as expected progress (adopts the authoritative rotation, keeps tweening) instead of
   * restarting a new tween — see the ignore-scan loop below for the full rationale and its
   * accepted edge case. Otherwise starts a fresh two-point ease-to-stop tween from the current
   * rendered position to `t`.
   * @param id The token id to retarget.
   * @param t The new authoritative transform.
   * @example
   * ```ts
   * import { TokenAnimator } from "@shadowcat/render";
   *
   * const animator = new TokenAnimator();
   * animator.setTarget("token-1", { x: 100, y: 100, rotation: 0 });
   * ```
   */
  setTarget(id: string, t: TokenTransform): void {
    // Guard: do not start an ease tween while sample playback is live for this id.
    // Handles MoveStream-before-Event ordering: samplesAnim registered first; the late-arriving
    // authoritative Event must not override the in-progress broadcast trajectory.
    if (this.samplesAnim.has(id)) return;
    const c = this.cur.get(id);
    if (!c) {
      this.cur.set(id, { ...t }); // brand-new → snap
      return;
    }
    const active = this.anim.get(id);
    if (active?.pathDriven) {
      // Ignore-scan rationale: scan ALL vertices at segIndex or ahead (not just the immediate next).
      // The route-commit dispatcher issues each run-endpoint as a synchronous burst of separate
      // `dispatchIntent` calls; the optimistic store notifies the engine subscription synchronously
      // per call, so the animator receives `setTarget(V1), setTarget(V2), …, setTarget(goal)` ALL
      // while `segIndex` is still 0 (no tick runs between them). Narrowing to immediate-next would
      // interrupt on V2. Scanning all ahead-vertices swallows every burst endpoint cleanly.
      //
      // Edge-case: a foreign or rollback authoritative position that coincidentally equals an
      // ahead route-vertex is also swallowed. This is acceptable because routes are constructed
      // ⊆ the gate-allowed mask (spec §13/§14), so rollbacks should not occur; and if one does
      // the engine self-heals on the next store update that issues the real final position.
      for (let i = active.segIndex; i < active.poly.length; i++) {
        const v = active.poly[i];
        if (Math.abs(v[0] - t.x) < EPSILON && Math.abs(v[1] - t.y) < EPSILON) {
          active.finalRot = t.rotation; // adopt authoritative rotation as the settle value
          return;
        }
      }
    }
    this.startAnim(id, c, [[c.x, c.y], [t.x, t.y]], t.rotation, false);
  }

  /** Has no production caller today (`src/modules`/`src/client/shell` drive route playback
   * exclusively through `animateSamples`, per `commitRoute`'s own "Animation is
   * broadcast-driven via onMoveStream ... no local animation from the moveRequest resolve value"
   * comment); exercised only by tests and this package's `TokenView`/`RenderEngine`
   * passthroughs. The mechanism below is the contract it honors if called.
   *
   * Drive a smooth local walk of `id` along `path`'s scene-coord waypoints, TARGETING `rotation`
   * as the walk's settle value (`startAnim`'s `finalRot`) — the rendered rotation eases from
   * whatever it was at call time toward `rotation` over the walk's duration, held constant only
   * when the two already match at call time (the common case, since nothing else rotates a token
   * mid-walk). A brand-new id (no prior rendered transform) snaps to the path's final point
   * instead of tweening. Consecutive coincident points are deduped and the walk is anchored at the
   * token's live current position (not `path[0]`), so a mid-tween re-issue continues smoothly
   * rather than jumping back to the route's nominal start. A path that dedups down to fewer than 2
   * points delegates to `setTarget` (no distance to walk).
   * @param id The token id to animate.
   * @param path The route's scene-coord waypoints, in order.
   * @param rotation The rotation to hold for the whole walk.
   * @example
   * ```ts
   * import { TokenAnimator } from "@shadowcat/render";
   *
   * const animator = new TokenAnimator();
   * animator.animateAlongPath("token-1", [[0, 0], [1, 0], [1, 1]], 0);
   * ```
   */
  animateAlongPath(id: string, path: [number, number][], rotation: number): void {
    const c = this.cur.get(id);
    if (!c) {
      const last = path[path.length - 1] ?? [0, 0];
      this.cur.set(id, { x: last[0], y: last[1], rotation }); // no prior render → snap
      return;
    }
    // Dedupe consecutive coincident points; anchor the walk at the live current position.
    const pts: [number, number][] = [[c.x, c.y]];
    for (const p of path) {
      const prev = pts[pts.length - 1];
      if (Math.abs(prev[0] - p[0]) >= EPSILON || Math.abs(prev[1] - p[1]) >= EPSILON) pts.push([p[0], p[1]]);
    }
    if (pts.length < 2) {
      this.setTarget(id, { x: pts[0][0], y: pts[0][1], rotation });
      return;
    }
    this.startAnim(id, c, pts, rotation, true);
  }

  /** Evaluate the sample-driven position at `sa.elapsed` and apply it to `cur`.
   * Sets or clears the hidden flag based on whether elapsed falls in an occlusion gap.
   * INVARIANT: elapsed at or past the last sample tMs → settle visible at last position.
   * @param id The token id being animated.
   * @param sa The sample-playback state (samples + elapsed + gapThreshold) to evaluate.
   * @example
   * ```
   * // private method; not part of the public API
   * this.applySamplesAt("token-1", sa);
   * ```
   */
  private applySamplesAt(id: string, sa: SamplesAnim): void {
    const { samples, elapsed, gapThreshold } = sa;
    const cur = this.cur.get(id);
    if (!cur) return;
    const last = samples[samples.length - 1];
    // Single-sample degenerate: always visible at that position.
    if (samples.length === 1) {
      cur.x = samples[0].pos[0];
      cur.y = samples[0].pos[1];
      this.hidden.delete(id);
      return;
    }
    // Past the last sample's timestamp: settle visibly at last position (not a gap).
    if (elapsed >= last.tMs) {
      cur.x = last.pos[0];
      cur.y = last.pos[1];
      this.hidden.delete(id);
      return;
    }
    // Before the first sample's timestamp: this observer's clip starts with leading occlusion
    // (the move began outside their vision, so their earliest visible sample has tMs > 0). A
    // fresh-broadcast catch-up (initialElapsed ≈ network latency) can land in this window before
    // samples[0].tMs. Extrapolating backward from samples[0] would invent a position the observer
    // never had visibility into; hide instead, mirroring the mid-path occlusion-gap treatment.
    if (elapsed < samples[0].tMs) {
      cur.x = samples[0].pos[0];
      cur.y = samples[0].pos[1];
      this.hidden.add(id);
      return;
    }
    // Find the segment [i, i+1] whose interval contains elapsed.
    for (let i = 0; i < samples.length - 1; i++) {
      if (elapsed <= samples[i + 1].tMs) {
        const gap = samples[i + 1].tMs - samples[i].tMs;
        if (gap > gapThreshold) {
          // Occlusion span: visible only exactly at the left endpoint, hidden inside the gap.
          if (elapsed <= samples[i].tMs) {
            cur.x = samples[i].pos[0];
            cur.y = samples[i].pos[1];
            this.hidden.delete(id);
          } else {
            // Inside the gap: freeze cur at the left endpoint and hide.
            cur.x = samples[i].pos[0];
            cur.y = samples[i].pos[1];
            this.hidden.add(id);
          }
          return;
        }
        // Normal interpolation within this segment.
        const f = gap > 0 ? Math.min(1, (elapsed - samples[i].tMs) / gap) : 1;
        cur.x = lerp(samples[i].pos[0], samples[i + 1].pos[0], f);
        cur.y = lerp(samples[i].pos[1], samples[i + 1].pos[1], f);
        this.hidden.delete(id);
        return;
      }
    }
    // Fallback: settle at last (should be unreachable after the elapsed >= last.tMs guard above).
    cur.x = last.pos[0];
    cur.y = last.pos[1];
    this.hidden.delete(id);
  }

  /** Compute per-segment lengths for `poly` and register a new ease-to-stop `Anim` entry for `id`,
   * duration = `(total px / cfg.cellSize) / cfg.speedCellsPerSec * 1000` using the CURRENT `cfg` (a
   * later `setConfig` call does not retroactively affect this tween — see `setConfig`). Degenerate
   * input (non-finite `total`, `total < EPSILON`, non-positive `cellSize` or `speedCellsPerSec`)
   * fails closed: snaps directly to `poly`'s last vertex (when finite) instead of animating,
   * mirroring the fail-closed convention used server-side in `scene::movement`/`scene::lighting`/
   * `scene::vision` (non-finite input → snap/under-reveal, never freeze on NaN or animate forever).
   * @param id The token id to animate.
   * @param c The token's current rendered transform (tween start point).
   * @param poly The polyline to walk, in scene px; `poly[0]` should equal `(c.x, c.y)`.
   * @param finalRot The rotation to settle at once the tween completes.
   * @param pathDriven Whether this is an explicit route walk (gates the `setTarget` ignore-scan rule).
   * @example
   * ```
   * // private method; not part of the public API
   * this.startAnim("token-1", { x: 0, y: 0, rotation: 0 }, [[0, 0], [100, 0]], 0, false);
   * ```
   */
  private startAnim(id: string, c: TokenTransform, poly: [number, number][], finalRot: number, pathDriven: boolean): void {
    const segLen: number[] = [];
    let total = 0;
    for (let i = 1; i < poly.length; i++) {
      const dx = poly[i][0] - poly[i - 1][0];
      const dy = poly[i][1] - poly[i - 1][1];
      const len = Math.hypot(dx, dy);
      segLen.push(len);
      total += len;
    }
    const last = poly[poly.length - 1];
    // Non-finite total (NaN/Infinity from NaN/Infinity coordinates) is treated as degenerate:
    // NaN total makes `total < EPSILON` false, `tRaw >= 1` never true → the token is pinned to
    // NaN and re-reports `moved` every tick forever. The !isFinite guard catches this case and
    // snaps to the last vertex instead. Mirrors the fail-closed convention in `scene::movement` /
    // `scene::lighting` / `scene::vision` (non-finite inputs → under-reveal / snap, never freeze or NaN output).
    // If the last vertex itself is non-finite, fall back to leaving `cur` unchanged.
    if (!Number.isFinite(total) || total < EPSILON || this.cfg.cellSize <= 0 || this.cfg.speedCellsPerSec <= 0) {
      if (Number.isFinite(last[0]) && Number.isFinite(last[1])) {
        this.cur.set(id, { x: last[0], y: last[1], rotation: finalRot }); // degenerate → snap
      }
      // else: last vertex is non-finite; leave cur unchanged (keeps last valid rendered position).
      this.anim.delete(id);
      return;
    }
    const cells = total / this.cfg.cellSize;
    this.anim.set(id, {
      poly, segLen, total, elapsed: 0,
      duration: (cells / this.cfg.speedCellsPerSec) * 1000,
      startRot: c.rotation, finalRot, easing: this.cfg.easing,
      pathDriven, segIndex: 0,
    });
  }

  /** Advance all animations by `dtMs`; return ids whose transform changed.
   * @param dtMs Elapsed render-frame time in ms since the last tick.
   * @returns The ids whose rendered transform changed this tick (sample-driven and
   * polyline-driven animations both included).
   * @example
   * ```ts
   * import { TokenAnimator } from "@shadowcat/render";
   *
   * const animator = new TokenAnimator();
   * animator.tick(16); // []
   * ```
   */
  tick(dtMs: number): string[] {
    const moved: string[] = [];
    // Advance sample-driven playbacks (broadcast MoveStream animations).
    for (const [id, sa] of this.samplesAnim) {
      sa.elapsed += dtMs;
      if (sa.elapsed >= sa.durationMs) {
        // Playback complete: settle at the last sample, clear hidden, remove entry.
        const last = sa.samples[sa.samples.length - 1];
        const cur = this.cur.get(id);
        if (cur) { cur.x = last.pos[0]; cur.y = last.pos[1]; }
        this.hidden.delete(id);
        this.samplesAnim.delete(id);
        moved.push(id);
        continue;
      }
      this.applySamplesAt(id, sa);
      moved.push(id);
    }
    for (const [id, a] of this.anim) {
      // samplesAnim takes exclusive precedence; anim.delete(id) in animateSamples prevents this
      // in normal Event-before-MoveStream ordering. Explicit guard for reverse-ordering edge cases.
      if (this.samplesAnim.has(id)) continue;
      a.elapsed += dtMs;
      const tRaw = Math.min(1, a.elapsed / a.duration);
      const e = applyEasing(a.easing, tRaw);
      const target = e * a.total;
      // Walk segments to the eased distance.
      let acc = 0;
      let pos: [number, number] = a.poly[a.poly.length - 1];
      let idx = a.poly.length - 1;
      for (let i = 0; i < a.segLen.length; i++) {
        if (target <= acc + a.segLen[i] || i === a.segLen.length - 1) {
          const f = a.segLen[i] > 0 ? Math.min(1, (target - acc) / a.segLen[i]) : 1;
          pos = [lerp(a.poly[i][0], a.poly[i + 1][0], f), lerp(a.poly[i][1], a.poly[i + 1][1], f)];
          idx = f >= 1 ? i + 1 : i;
          break;
        }
        acc += a.segLen[i];
      }
      a.segIndex = idx; // monotonic-ish progress marker for the ignore rule
      const cur = this.cur.get(id)!;
      cur.x = pos[0];
      cur.y = pos[1];
      cur.rotation = lerp(a.startRot, a.finalRot, e);
      moved.push(id);
      if (tRaw >= 1) {
        const last = a.poly[a.poly.length - 1];
        cur.x = last[0];
        cur.y = last[1];
        cur.rotation = a.finalRot;
        this.anim.delete(id);
      }
    }
    return moved;
  }
}
