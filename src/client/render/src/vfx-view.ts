import { resolveTokenActor } from "@shadowcat/core";
import type { ReadableDocuments, ResolvedVfxSource, VfxEmission, VfxAnchor, VfxOneShotRequest } from "@shadowcat/core";
import type { DisplayBackend } from "./backend";
import type { VfxNodeSpec, TokenNodeSpec, TokenTransform } from "./types";
import { sceneScopedDocs } from "./scene-scope";

/** Concurrency cap: the oldest live one-shot is evicted once a scene would exceed this many —
 * a spam burst cannot stall the stage. */
const MAX_ONESHOTS = 64;

/** `VfxAnchor`'s draw-order key within the `vfx` layer (all three sit above `tokens`, so no
 * effect ever draws under token art): `below` = 0 (under the token's own art layer-wise, but
 * still above the `tokens` core layer since `vfx` itself sits above it), `token` = 1, `above` =
 * 2. A one-shot (`anchor: "point"`) has no token to order relative to and uses `1`.
 * @param anchor The node's anchor.
 * @returns The zIndex `PixiBackend` sorts the `vfx` layer's children by.
 * @example
 * ```ts
 * import { vfxAnchorZIndex } from "@shadowcat/render";
 *
 * vfxAnchorZIndex("above"); // 2
 * ```
 */
export function vfxAnchorZIndex(anchor: VfxAnchor | "point"): number {
  switch (anchor) {
    case "below":
      return 0;
    case "above":
      return 2;
    default:
      return 1;
  }
}

/** One live one-shot's bookkeeping: `durationMs`, when present, caps a looping asset — the
 * node's own `AnimatedSprite` loops (so a short clip repeats to fill the window) while THIS
 * class enforces the external time cap and removes it early; a node with no `durationMs`
 * plays exactly one loop of the asset and removes itself via the backend's `onDone` callback. */
interface OneShotState {
  /** The node id (`oneshot:<id>`). */
  id: string;
  /** The scene the one-shot plays on — a scene switch drops every one-shot not on the newly
   * viewed scene (its scene coordinates are meaningless over the new scene's grid). */
  scene: string;
  /** Accumulated elapsed time since `play()`, in ms. */
  elapsedMs: number;
  /** The external duration cap, or `undefined` for "one natural loop". */
  durationMs?: number;
}

/** Renders `EffectiveActor.vfx` emitters (tracking their token's live tweened transform) and
 * transient one-shots (`ServerMsg::Vfx` → `play`). The `PingView`/`EmoteView` pattern: pure
 * state + reconcile/tick, no document writes, backend-agnostic. */
export class VfxView {
  /** Currently-pushed emitter node specs, keyed by node id (`emitter:<token>`) — reconcile's
   * diff state AND the per-tick transform pass's re-push base. */
  private readonly emitters = new Map<string, VfxNodeSpec>();
  /** Live one-shots, oldest first (insertion order — the eviction/tick order). */
  private oneShots: OneShotState[] = [];

  /**
   * Constructs the view over its injected dependencies; pure wiring, no document reads until
   * the first `reconcile()`/`play()` call.
   * @param store The document store to read `token` docs from.
   * @param backend The display backend to push resolved VFX nodes to.
   * @param viewedSceneId Resolves the currently-viewed scene id.
   * @param viewedLevel Resolves the currently-viewed level id; resolving to `null` means "every
   * level" — the same `sceneScopedDocs`-style scoping `TokenView`/`WallView`/etc. already apply,
   * here filtering which tokens' emitters `reconcile()` renders (a one-shot carries no elevation
   * of its own and is never level-filtered).
   * @param vfxAssets Resolves an asset id to its playable source; `null` fails the node
   * closed (never drawn).
   * @param tokenTransform Resolves a token's CURRENT rendered (tweened) transform — normally
   * `TokenView.transformOf`, so an emitter tracks the exact same interpolation the token's
   * own sprite renders.
   * @param tokenSpec Resolves a token's last-projected `TokenNodeSpec` (for its footprint
   * height, used by the `below`/`above` anchor offsets) — normally `TokenView.specOf`.
   * @param vfxEnabled Reads the current `PerformanceSettings.vfx` flag; read on every
   * `reconcile()`/`play()` call, never cached (a mid-session toggle takes effect
   * immediately). Defaults to always-enabled — legacy/test callers, and every caller until
   * the shell binds `PerformanceSettings`.
   * @param reducedMotion Reads the current `PerformanceSettings.reducedMotion` flag, same
   * always-fresh contract as `vfxEnabled`. Defaults to always-`false`.
   * @example
   * ```ts
   * import { VfxView, MockBackend } from "@shadowcat/render";
   * import { type ReadableDocuments } from "@shadowcat/core";
   *
   * declare const store: ReadableDocuments;
   * const view = new VfxView(store, new MockBackend(), () => null, () => null, () => null, () => undefined, () => undefined);
   * ```
   */
  constructor(
    private readonly store: ReadableDocuments,
    private readonly backend: DisplayBackend,
    private readonly viewedSceneId: () => string | null,
    private readonly viewedLevel: () => string | null,
    private readonly vfxAssets: (id: string) => ResolvedVfxSource | null,
    private readonly tokenTransform: (id: string) => TokenTransform | undefined,
    private readonly tokenSpec: (id: string) => TokenNodeSpec | undefined,
    private readonly vfxEnabled: () => boolean = () => true,
    private readonly reducedMotion: () => boolean = () => false,
  ) {}

  /** Diff the store's readable, viewed-scene tokens whose `EffectiveActor.vfx` is `enabled`
   * against the currently-pushed emitter set. `PerformanceSettings.vfx == false` tears every
   * node down (emitters AND live one-shots) and returns immediately (checked first, every
   * call — never cached).
   * @example
   * ```ts
   * import { VfxView, MockBackend } from "@shadowcat/render";
   * import { type ReadableDocuments } from "@shadowcat/core";
   *
   * declare const store: ReadableDocuments;
   * const view = new VfxView(store, new MockBackend(), () => null, () => null, () => null, () => undefined, () => undefined);
   * view.reconcile();
   * ```
   */
  reconcile(): void {
    const viewed = this.viewedSceneId();
    if (!this.vfxEnabled()) {
      // Disabled tears down EVERY node — emitters and any live one-shots alike — so the
      // layer carries nothing while the setting is off.
      for (const id of this.emitters.keys()) this.backend.removeVfx(id);
      this.emitters.clear();
      for (const s of this.oneShots) this.backend.removeVfx(s.id);
      this.oneShots = [];
      return;
    }
    // A one-shot's scene coordinates are meaningless over a different scene: drop every
    // one-shot not on the newly viewed scene (emitters are already scene-scoped by the
    // store query below).
    const offScene = this.oneShots.filter((s) => viewed !== null && s.scene !== viewed);
    for (const s of offScene) this.backend.removeVfx(s.id);
    if (offScene.length > 0) {
      const drop = new Set(offScene.map((s) => s.id));
      this.oneShots = this.oneShots.filter((s) => !drop.has(s.id));
    }
    const seen = new Set<string>();
    for (const doc of sceneScopedDocs(this.store, "token", this.viewedSceneId, this.viewedLevel)) {
      const eff = resolveTokenActor(doc, this.store);
      const vfx: VfxEmission | null | undefined = eff?.vfx;
      if (!vfx || !vfx.enabled) continue;
      const source = this.vfxAssets(vfx.asset);
      if (!source) continue;
      const id = `emitter:${doc.id}`;
      const pushed = this.pushEmitter(id, doc.id, vfx, source);
      if (pushed) seen.add(id);
    }
    for (const id of this.emitters.keys()) {
      if (!seen.has(id)) this.backend.removeVfx(id);
    }
    for (const id of [...this.emitters.keys()]) {
      if (!seen.has(id)) this.emitters.delete(id);
    }
  }

  /** Resolve + push one emitter node. Returns `false` (and removes any stale node) when the
   * owning token has no live transform/spec yet (a brand-new token this same reconcile pass
   * hasn't been projected by `TokenView` before this `VfxView` — the engine's call order
   * guarantees `tokens.reconcile()` runs first, so this is a defensive fail-closed, not the
   * expected path).
   * @param id The emitter node id.
   * @param tokenId The owning token's document id.
   * @param vfx The resolved `VfxEmission` payload.
   * @param source The resolved playable source.
   * @returns Whether the node was pushed.
   * @example
   * ```
   * // private method; not part of the public API
   * ```
   */
  private pushEmitter(id: string, tokenId: string, vfx: VfxEmission, source: ResolvedVfxSource): boolean {
    const t = this.tokenTransform(tokenId);
    const spec = this.tokenSpec(tokenId);
    if (!t || !spec) {
      this.backend.removeVfx(id);
      return false;
    }
    const offset = vfx.anchor === "below" ? spec.h / 2 : vfx.anchor === "above" ? -spec.h / 2 : 0;
    const node: VfxNodeSpec = {
      layer: "vfx",
      x: t.x,
      y: t.y + offset,
      scale: 1,
      rotation: 0,
      source,
      // Reduced motion: an emitter still renders, frozen at the sequence's last frame — the
      // SAME `loop:false`-holds-final-frame convention `computeAnimatedFrame` already
      // establishes for token animations, reused here rather than inventing a second one.
      // Note: `VfxEmission.loop` on the wire (`ts-rs` mirrors the Rust struct's
      // `#[serde(rename = "loop")]` on `loop_`, so the generated TS field is spelled
      // `loop`, not `loop_` — verified against the generated `VfxEmission` binding).
      loop: vfx.loop && !this.reducedMotion(),
      anchor: vfx.anchor,
      token: tokenId,
      // Reduced motion additionally freezes the emitter at the sequence's LAST frame on load
      // (never plays through first) — the backend jumps `elapsedMs` to the sequence total.
      startAtEnd: this.reducedMotion(),
    };
    this.backend.setVfx(id, node);
    this.emitters.set(id, node);
    return true;
  }

  /** Play a one-shot from a `ServerMsg::Vfx` broadcast. `PerformanceSettings.vfx == false` or
   * `reducedMotion` (one-shots are skipped entirely under reduced motion, unlike an emitter,
   * which freezes instead) makes this a no-op — read fresh on every call, never cached. An
   * unresolvable `asset` (fails `vfxAssets`) is also a no-op (fail closed). The 64-live cap
   * evicts the OLDEST live one-shot (by `play()` call order) before adding a new one past it.
   * @param req The one-shot request, with the server-broadcast `id` naming this exact
   * playback (`oneshot:<id>` is the node id).
   * @example
   * ```ts
   * import { VfxView, MockBackend } from "@shadowcat/render";
   * import { type ReadableDocuments } from "@shadowcat/core";
   *
   * declare const store: ReadableDocuments;
   * const view = new VfxView(store, new MockBackend(), () => null, () => null, () => null, () => undefined, () => undefined);
   * view.play({ scene: "s1", asset: "a1", x: 0, y: 0, id: "one-shot-1" });
   * ```
   */
  play(req: VfxOneShotRequest): void {
    if (!this.vfxEnabled() || this.reducedMotion()) return;
    const source = this.vfxAssets(req.asset);
    if (!source) return;
    const id = `oneshot:${req.id}`;
    if (this.oneShots.length >= MAX_ONESHOTS) {
      const oldest = this.oneShots.shift();
      if (oldest) this.backend.removeVfx(oldest.id);
    }
    this.oneShots.push({ id, scene: req.scene, elapsedMs: 0, durationMs: req.durationMs });
    this.backend.setVfx(id, {
      layer: "vfx",
      x: req.x,
      y: req.y,
      scale: req.scale ?? 1,
      rotation: req.rotation ?? 0,
      source,
      // A durationMs cap loops the underlying asset (a short clip repeats to fill the
      // window); this class's own tick() enforces the external cap. No cap plays exactly
      // one loop and removes itself via the backend's onDone callback.
      loop: req.durationMs !== undefined,
      anchor: "point",
    });
  }

  /** Advance one-shot duration caps by `dtMs`, force-removing any that just exceeded their
   * external `durationMs`, then drives `DisplayBackend.tickVfx` for frame advance — its
   * `onDone` callback (fired for a `loop:false` node whose one natural loop completed) drops
   * the id from this view's own bookkeeping so a later `play()`'s 64-cap count stays accurate.
   * @param dtMs Elapsed render-frame time in ms since the last tick.
   * @example
   * ```ts
   * import { VfxView, MockBackend } from "@shadowcat/render";
   * import { type ReadableDocuments } from "@shadowcat/core";
   *
   * declare const store: ReadableDocuments;
   * const view = new VfxView(store, new MockBackend(), () => null, () => null, () => null, () => undefined, () => undefined);
   * view.tick(16);
   * ```
   */
  tick(dtMs: number): void {
    // Per-tick emitter-transform pass: re-read each emitter's LIVE tweened transform and
    // re-push on change (the backend's source-key short-circuit makes a transform-only
    // re-push cheap — no texture reload). This is what lets an effect follow a moving token
    // between store commits; the full reconcile diff runs only on commits/scene switches.
    for (const [id, spec] of this.emitters) {
      if (!spec.token) continue;
      const t = this.tokenTransform(spec.token);
      if (!t) continue;
      const tokenSpec = this.tokenSpec(spec.token);
      if (!tokenSpec) continue;
      const offset = spec.anchor === "below" ? tokenSpec.h / 2 : spec.anchor === "above" ? -tokenSpec.h / 2 : 0;
      if (spec.x === t.x && spec.y === t.y + offset) continue;
      spec.x = t.x;
      spec.y = t.y + offset;
      this.backend.setVfx(id, spec);
    }
    for (const s of this.oneShots) s.elapsedMs += dtMs;
    const expired = this.oneShots.filter((s) => s.durationMs !== undefined && s.elapsedMs >= s.durationMs);
    for (const s of expired) this.backend.removeVfx(s.id);
    const expiredIds = new Set(expired.map((s) => s.id));
    this.oneShots = this.oneShots.filter((s) => !expiredIds.has(s.id));
    this.backend.tickVfx(dtMs, (id) => {
      // A naturally-completed one-shot (or one whose load failed — the backend surfaces both
      // through this callback) leaves the display list AND the bookkeeping.
      this.backend.removeVfx(id);
      this.oneShots = this.oneShots.filter((s) => s.id !== id);
    });
  }

  /** Count of currently-live nodes (emitters + one-shots) — a read-only observability signal
   * the host (`Stage.svelte`) surfaces as `data-vfx-count` without inspecting WebGL pixels.
   * @returns The current total live node count.
   * @example
   * ```ts
   * import { VfxView, MockBackend } from "@shadowcat/render";
   * import { type ReadableDocuments } from "@shadowcat/core";
   *
   * declare const store: ReadableDocuments;
   * const view = new VfxView(store, new MockBackend(), () => null, () => null, () => null, () => undefined, () => undefined);
   * view.count(); // 0
   * ```
   */
  count(): number {
    return this.emitters.size + this.oneShots.length;
  }
}
