import { resolveTokenActor, resolveConditions, resolveTokenBox, resolveTokenVisual } from "@shadowcat/core";
import type { ReadableDocuments, AssetResolver, WireDocument, FactionRegistryEngine, TokenEngine, AnimatedSource } from "@shadowcat/core";
import type { DisplayBackend } from "./backend";
import type { TokenNodeSpec, ResolvedAnimatedSource } from "./types";
import { parseColor } from "./geometry";
import { TokenAnimator, type MoveSample } from "./token-animator";
import type { EasingMode } from "./easing";
import { sceneScopedDocs } from "./scene-scope";

/** Renders `doc_type:"token"` docs as backend token nodes, tweening transforms via a
 * TokenAnimator. The visual (size + image) applies immediately; the transform tweens. */
export class TokenView {
  private readonly animator = new TokenAnimator();
  private readonly specs = new Map<string, TokenNodeSpec>();
  /** Tracks tokens that were hidden on the previous push call, to detect visible↔hidden
   * transitions and call removeToken only once per gap entry (not every tick). */
  private readonly wasHidden = new Set<string>();
  /** A locally-dragged token id snaps to its target each reconcile (no tween lag);
   * remote tokens still tween. Set by the move tool via the engine. */
  private dragging: string | null = null;

  // Animation config fields; kept in sync with the animator via pushAnimConfig().
  private cellSize = 100;
  private animSpeed = 6;
  private animEasing: EasingMode = "easeInOut";

  constructor(
    private readonly store: ReadableDocuments,
    private readonly assets: AssetResolver,
    private readonly backend: DisplayBackend,
    private readonly viewedSceneId: () => string | null = () => null,
  ) {}

  setDragging(id: string | null): void {
    this.dragging = id;
  }

  /** Update the pixel-per-cell value used to compute tween durations. */
  setCellSize(px: number): void {
    this.cellSize = px;
    this.pushAnimConfig();
  }

  /** Update the speed + easing used to compute tween durations. */
  setAnimationConfig(cfg: { speedCellsPerSec: number; easing: EasingMode }): void {
    this.animSpeed = cfg.speedCellsPerSec;
    this.animEasing = cfg.easing;
    this.pushAnimConfig();
  }

  /** Merge the stored speed/easing/cellSize into a single AnimationConfig and forward it to the
   * animator. Coupling: both setCellSize and setAnimationConfig must call this so the animator's
   * config is always the product of the latest values of all three fields. */
  private pushAnimConfig(): void {
    this.animator.setConfig({ speedCellsPerSec: this.animSpeed, easing: this.animEasing, cellSize: this.cellSize });
  }

  /** Drive a smooth local walk along a route's scene-coord waypoints. Rotation is held (a route
   * move does not rotate the token). The prompt authoritative commit catches up via reconcile()'s
   * setTarget, which the animator recognizes as expected progress. */
  animateAlongPath(id: string, path: [number, number][]): void {
    const rotation = this.specs.get(id)?.rotation ?? 0;
    this.animator.animateAlongPath(id, path, rotation);
    this.push(id);
  }

  /** Drive server-broadcast sample-based playback. Interpolates position between adjacent samples
   * by tMs; hides the token across occlusion gaps (server-clipped visibility spans). Catch-up: if
   * `serverNow()` is ahead of `startServerMs`, playback begins from the matching elapsed offset. */
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

  tick(dtMs: number): void {
    for (const id of this.animator.tick(dtMs)) this.push(id);
    this.backend.tickTokenAnimations(dtMs);
  }

  /** Push a token to the backend with its latest visual + current (tweened) transform.
   * INVARIANT: removeToken is called exactly once per visible→hidden transition (gap entry),
   * not every tick. setToken is called on every visible tick. wasHidden tracks the prior-call
   * state to detect transitions without querying backend state.
   * Coupling: reconcile respects isHidden through this path; wasHidden is cleared on token
   * removal so a re-created token starts from a clean visible state. */
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

  private resolveSource(source: AnimatedSource): ResolvedAnimatedSource {
    return source.type === "frames"
      ? { type: "frames", urls: source.frames.map((id) => this.assets.url(id)) }
      // `count` is required-nullable on the generated `AnimatedSource` (`number | null`);
      // `ResolvedAnimatedSource.count` is optional (`number | undefined`) — coalesce null to
      // undefined rather than assuming presence implies non-null.
      : { type: "sheet", url: this.assets.url(source.asset), rows: source.rows, cols: source.cols, count: source.count ?? undefined };
  }

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
