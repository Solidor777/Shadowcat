import { resolveTokenActor, resolveConditions, resolveTokenBox, resolveTokenVisual } from "@shadowcat/core";
import type { ReadableDocuments, AssetResolver, WireDocument, FactionRegistryEngine, TokenEngine, AnimatedSource } from "@shadowcat/core";
import type { DisplayBackend } from "./backend";
import type { TokenNodeSpec, ResolvedAnimatedSource } from "./types";
import { parseColor } from "./geometry";
import { TokenAnimator, type MoveSample } from "./token-animator";
import type { EasingMode, TokenTweenConfig } from "./easing";
import { sceneScopedDocs } from "./scene-scope";

/** Renders `doc_type:"token"` docs as backend token nodes, tweening transforms via a
 * TokenAnimator. The visual (size + image) applies immediately; the transform tweens. */
export class TokenView {
  /** Drives every tracked token's tween/sample-playback transform. */
  private readonly animator = new TokenAnimator();
  /** Last resolved `TokenNodeSpec` per token id, from `toSpec` — the visual/size/border/badges
   * `push` applies immediately, distinct from the tweened transform `animator` owns. */
  private readonly specs = new Map<string, TokenNodeSpec>();
  /** Tracks tokens that were hidden on the previous push call, to detect visible↔hidden
   * transitions and call removeToken only once per gap entry (not every tick). */
  private readonly wasHidden = new Set<string>();
  /** A locally-dragged token id snaps to its target each reconcile (no tween lag);
   * remote tokens still tween. Set by the move tool via the engine. */
  private dragging: string | null = null;

  // Animation config fields; kept in sync with the animator via pushAnimConfig().
  /** Active grid's per-step world distance (`Grid.worldUnitsPerCell`) — see `setWorldUnitsPerCell`. */
  private worldUnitsPerCell = 100;
  /** Tween speed, in grid cells per second — see `setAnimationConfig`. */
  private animSpeed = 6;
  /** Easing curve applied to polyline tweens — see `setAnimationConfig`. */
  private animEasing: EasingMode = "easeInOut";

  /** Constructs a view bound to `store`/`assets`/`backend`; call `reconcile()` once to populate it.
   * @param store The document store to read `token` docs (and their linked actor/faction/
   * condition-registry docs) from.
   * @param assets Resolves asset ids embedded in a token's visual to serve URLs.
   * @param backend The display backend to push resolved token nodes to.
   * @param viewedSceneId Resolves the currently-viewed scene id; `reconcile()` scopes its query to
   * this scene (falls back to unscoped — every token in the store — when it resolves to `null`).
   * Defaults to always-`null` (legacy/test callers that never pass one).
   * @example
   * ```ts
   * import { TokenView, MockBackend } from "@shadowcat/render";
   * import { AssetResolver, type ReadableDocuments } from "@shadowcat/core";
   *
   * declare const store: ReadableDocuments;
   * const view = new TokenView(store, new AssetResolver(), new MockBackend());
   * ```
   */
  constructor(
    private readonly store: ReadableDocuments,
    private readonly assets: AssetResolver,
    private readonly backend: DisplayBackend,
    private readonly viewedSceneId: () => string | null = () => null,
  ) {}

  /** Mark `id` as the locally-dragged token (its sprite snaps to the authoritative transform each
   * reconcile, no tween lag) or clear the latch with `null`.
   * @param id The token id being dragged, or `null` to clear.
   * @example
   * ```ts
   * import { TokenView, MockBackend } from "@shadowcat/render";
   * import { AssetResolver, type ReadableDocuments } from "@shadowcat/core";
   *
   * declare const store: ReadableDocuments;
   * const view = new TokenView(store, new AssetResolver(), new MockBackend());
   * view.setDragging("token-1");
   * view.setDragging(null);
   * ```
   */
  setDragging(id: string | null): void {
    this.dragging = id;
  }

  /** Update the per-step world distance used to compute tween durations — the world distance
   * between adjacent cell centres (`Grid.worldUnitsPerCell`), NOT the grid's indexing scale
   * (`GridSpec.size`), which diverges from it by `sqrt(3)` on hex. Affects only FUTURE tweens
   * (`TokenAnimator.setConfig` does not retarget an animation already in progress).
   * @param units The active grid's world distance between adjacent cell centres.
   * @example
   * ```ts
   * import { TokenView, MockBackend } from "@shadowcat/render";
   * import { AssetResolver, type ReadableDocuments } from "@shadowcat/core";
   *
   * declare const store: ReadableDocuments;
   * const view = new TokenView(store, new AssetResolver(), new MockBackend());
   * view.setWorldUnitsPerCell(100);
   * ```
   */
  setWorldUnitsPerCell(units: number): void {
    this.worldUnitsPerCell = units;
    this.pushAnimConfig();
  }

  /** Update the speed + easing used to compute tween durations. Affects only FUTURE tweens, same
   * as `setWorldUnitsPerCell`.
   * @param cfg The new tween speed/easing.
   * @example
   * ```ts
   * import { TokenView, MockBackend } from "@shadowcat/render";
   * import { AssetResolver, type ReadableDocuments } from "@shadowcat/core";
   *
   * declare const store: ReadableDocuments;
   * const view = new TokenView(store, new AssetResolver(), new MockBackend());
   * view.setAnimationConfig({ speedCellsPerSec: 6, easing: "easeInOut" });
   * ```
   */
  setAnimationConfig(cfg: TokenTweenConfig): void {
    this.animSpeed = cfg.speedCellsPerSec;
    this.animEasing = cfg.easing;
    this.pushAnimConfig();
  }

  /** Merge the stored speed/easing/worldUnitsPerCell into a single AnimationConfig and forward it
   * to the animator. Coupling: both setWorldUnitsPerCell and setAnimationConfig must call this so
   * the animator's config is always the product of the latest values of all three fields.
   * @example
   * ```
   * // private method; not part of the public API
   * this.pushAnimConfig();
   * ```
   */
  private pushAnimConfig(): void {
    this.animator.setConfig({ speedCellsPerSec: this.animSpeed, easing: this.animEasing, worldUnitsPerCell: this.worldUnitsPerCell });
  }

  /** Has no production caller today (`src/modules`/`src/client/shell` drive route playback
   * exclusively through `animateSamples`, per `commitRoute`'s own "Animation is
   * broadcast-driven via onMoveStream ... no local animation from the moveRequest resolve value"
   * comment); exercised only by tests and this passthrough's own caller (`RenderEngine`'s
   * `SceneToolHost` seam). The mechanism below is the contract it honors if called.
   *
   * Drive a smooth local walk along a route's scene-coord waypoints (forwards to
   * `TokenAnimator.animateAlongPath`, which targets — not permanently holds — `rotation`; see its
   * own doc for the exact easing behavior). A later authoritative commit catches up via
   * reconcile()'s setTarget, which the animator recognizes as expected progress.
   * @param id The token id to animate.
   * @param path The route's scene-coord waypoints, in order.
   * @example
   * ```ts
   * import { TokenView, MockBackend } from "@shadowcat/render";
   * import { AssetResolver, type ReadableDocuments } from "@shadowcat/core";
   *
   * declare const store: ReadableDocuments;
   * const view = new TokenView(store, new AssetResolver(), new MockBackend());
   * view.animateAlongPath("token-1", [[0, 0], [100, 0]]);
   * ```
   */
  animateAlongPath(id: string, path: [number, number][]): void {
    const rotation = this.specs.get(id)?.rotation ?? 0;
    this.animator.animateAlongPath(id, path, rotation);
    this.push(id);
  }

  /** Drive server-broadcast sample-based playback. Interpolates position between adjacent samples
   * by tMs; hides the token across occlusion gaps (server-clipped visibility spans). Catch-up: if
   * `serverNow()` is ahead of `startServerMs`, playback begins from the matching elapsed offset.
   * @param id The token id to animate.
   * @param samples The (already server-clipped) position samples to play back.
   * @param durationMs Total playback duration in ms.
   * @param startServerMs The server clock time (ms) at which this playback begins.
   * @param serverNow Optional injected server clock, used once at call time as
   * `Math.max(0, serverNow() - startServerMs)` to compute the initial catch-up offset; when
   * absent, elapsed starts at `0` (no catch-up assumed — NOT a `Date.now` fallback).
   * @example
   * ```ts
   * import { TokenView, MockBackend } from "@shadowcat/render";
   * import { AssetResolver, type ReadableDocuments } from "@shadowcat/core";
   *
   * declare const store: ReadableDocuments;
   * const view = new TokenView(store, new AssetResolver(), new MockBackend());
   * view.animateSamples("token-1", [{ tMs: 0, pos: [0, 0] }, { tMs: 500, pos: [1, 1] }], 500, Date.now());
   * ```
   */
  animateSamples(
    id: string,
    samples: MoveSample[],
    durationMs: number,
    startServerMs: number,
    serverNow?: () => number,
  ): void {
    this.animator.animateSamples(id, samples, durationMs, startServerMs, serverNow);
    this.push(id);
  }

  /** Diff the store's `token` docs (scoped to `viewedSceneId`) against the ids currently tracked in
   * `specs`: every current doc gets a fresh spec + an immediate `push` (so a new/dragged token
   * snaps to its authoritative transform this call, not next tick), and every tracked id no longer
   * present is fully torn down (`animator.remove` + `wasHidden` cleared + `backend.removeToken`). A
   * doc whose `toSpec` resolves to `null` (e.g. an unresolvable visual) is treated as absent — it
   * is never added to `seen`, so it is torn down on this same pass if it was tracked.
   * @example
   * ```ts
   * import { TokenView, MockBackend } from "@shadowcat/render";
   * import { AssetResolver, type ReadableDocuments } from "@shadowcat/core";
   *
   * declare const store: ReadableDocuments;
   * const view = new TokenView(store, new AssetResolver(), new MockBackend());
   * view.reconcile();
   * ```
   */
  reconcile(): void {
    const seen = new Set<string>();
    for (const doc of sceneScopedDocs(this.store, "token", this.viewedSceneId)) {
      const spec = this.toSpec(doc);
      if (!spec) continue;
      seen.add(doc.id);
      this.specs.set(doc.id, spec);
      // Dragging the local token: drop its tween state so setTarget re-snaps it to
      // the authoritative position immediately (a brand-new id always snaps).
      if (doc.id === this.dragging) this.animator.remove(doc.id);
      this.animator.setTarget(doc.id, { x: spec.x, y: spec.y, rotation: spec.rotation });
      this.push(doc.id); // immediate: new/dragged tokens snapped, visual current
    }
    for (const id of [...this.specs.keys()]) {
      if (seen.has(id)) continue;
      this.specs.delete(id);
      this.animator.remove(id);
      this.wasHidden.delete(id);
      this.backend.removeToken(id);
    }
  }

  /** Advance both independent per-frame facilities: `animator.tick` (transform tweens — polyline
   * walks and sample playback) pushes each changed id's latest transform to the backend, and
   * `backend.tickTokenAnimations` (frame-index playback for a token's `kind:"animated"` visual —
   * see `computeAnimatedFrame`) advances independently of any transform
   * tween, so an animated-sprite token still cycles frames while stationary.
   * @param dtMs Elapsed render-frame time in ms since the last tick.
   * @example
   * ```ts
   * import { TokenView, MockBackend } from "@shadowcat/render";
   * import { AssetResolver, type ReadableDocuments } from "@shadowcat/core";
   *
   * declare const store: ReadableDocuments;
   * const view = new TokenView(store, new AssetResolver(), new MockBackend());
   * view.tick(16);
   * ```
   */
  tick(dtMs: number): void {
    for (const id of this.animator.tick(dtMs)) this.push(id);
    this.backend.tickTokenAnimations(dtMs);
  }

  /** Push a token to the backend with its latest visual + current (tweened) transform.
   * INVARIANT: removeToken is called exactly once per visible→hidden transition (gap entry),
   * not every tick. setToken is called on every visible tick. wasHidden tracks the prior-call
   * state to detect transitions without querying backend state.
   * Coupling: reconcile respects isHidden through this path; wasHidden is cleared on token
   * removal so a re-created token starts from a clean visible state.
   * @param id The token id to push.
   * @example
   * ```
   * // private method; not part of the public API
   * this.push("token-1");
   * ```
   */
  private push(id: string): void {
    const spec = this.specs.get(id);
    const t = this.animator.get(id);
    if (!spec) return;
    const hidden = this.animator.isHidden(id);
    if (hidden) {
      if (!this.wasHidden.has(id)) {
        // Transition: visible → hidden. Remove from backend once at gap entry.
        this.backend.removeToken(id);
        this.wasHidden.add(id);
      }
      return;
    }
    // Visible: clear wasHidden (handles gap-exit transition implicitly) and update backend.
    this.wasHidden.delete(id);
    if (t) this.backend.setToken(id, { ...spec, x: t.x, y: t.y, rotation: t.rotation });
  }

  /** Resolve an actor/token-declared `AnimatedSource` (raw asset ids) into a `ResolvedAnimatedSource`
   * (already-URL'd) via `this.assets`, mirroring the `visual.kind === "image"` branch's
   * `assets.url(...)` call in `toSpec` — the backend never resolves asset ids itself.
   * @param source The animated-visual source (a frame-URL list, or a sprite sheet), pre-resolution.
   * @returns The same shape with every asset id replaced by its resolved serve URL.
   * @example
   * ```
   * // private method; not part of the public API
   * this.resolveSource({ type: "frames", frames: ["00000000-0000-0000-0000-000000000001"] });
   * ```
   */
  private resolveSource(source: AnimatedSource): ResolvedAnimatedSource {
    return source.type === "frames"
      ? { type: "frames", urls: source.frames.map((id) => this.assets.url(id)) }
      // `count` is required-nullable on the generated `AnimatedSource` (`number | null`);
      // `ResolvedAnimatedSource.count` is optional (`number | undefined`) — coalesce null to
      // undefined rather than assuming presence implies non-null.
      : { type: "sheet", url: this.assets.url(source.asset), rows: source.rows, cols: source.cols, count: source.count ?? undefined };
  }

  /** Project a `token` doc into a renderable `TokenNodeSpec`: resolves the effective actor
   * (`resolveTokenActor`), the visual (`resolveTokenVisual`, image or animated, URL-resolved via
   * `resolveSource`), the faction border color (via the world `faction-registry` doc; `null` when
   * the effective actor has no faction or the faction has no registered color), condition badges
   * (`resolveConditions`), and the footprint box/shape (`resolveTokenBox`). Fails closed to `null`
   * when the doc has no `engine` body or `resolveTokenVisual` cannot resolve a visual; `reconcile`
   * treats a `null` result as "this token is absent" and tears down any tracked state for its id.
   * @param doc The `token` document to project.
   * @returns The resolved spec, or `null` when the doc cannot be rendered.
   * @example
   * ```
   * // private method; not part of the public API
   * declare const doc: WireDocument;
   * this.toSpec(doc);
   * ```
   */
  private toSpec(doc: WireDocument): TokenNodeSpec | null {
    const s = doc.engine as TokenEngine | undefined;
    if (!s) return null;
    const eff = resolveTokenActor(doc, this.store);
    const visual = resolveTokenVisual(doc, this.store, eff);
    if (!visual) return null;
    const resolvedVisual: TokenNodeSpec["visual"] =
      visual.kind === "image"
        ? { kind: "image", url: this.assets.url(visual.asset) }
        : { kind: "animated", source: this.resolveSource(visual.source), fps: visual.fps, loop: visual.loop };
    // Faction border color resolves through the world faction registry; null = no border.
    let borderColor: number | null = null;
    if (eff?.faction) {
      const reg = this.store.query("faction-registry")[0]?.engine as FactionRegistryEngine | undefined;
      const hex = reg?.factions?.[eff.faction]?.color;
      if (hex) borderColor = parseColor(hex);
    }
    // Condition badges: resolve the actor's condition ids to registry icon glyphs.
    const badges = resolveConditions(doc, this.store).map((c) => c.icon);
    const box = resolveTokenBox(doc, this.store, eff);
    return {
      x: box.x, y: box.y, w: box.w, h: box.h, rotation: s.rotation ?? 0,
      visual: resolvedVisual,
      borderColor,
      badges,
      shape: box.shape,
    };
  }
}
