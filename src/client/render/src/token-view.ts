import { resolveTokenActor, resolveConditions, resolveTokenBox } from "@shadowcat/core";
import type { ReadableDocuments, AssetResolver, WireDocument, FactionRegistrySystem } from "@shadowcat/core";
import type { DisplayBackend } from "./backend";
import type { TokenNodeSpec } from "./types";
import { parseColor } from "./geometry";
import { TokenAnimator } from "./token-animator";

/** Engine-reserved token system fields (M8 §4.2; client-owned). `(x,y)` = center. */
interface TokenSystem {
  x: number;
  y: number;
  w: number;
  h: number;
  rotation?: number;
  visual?: { kind: string; asset: string };
}

/** Renders `doc_type:"token"` docs as backend token nodes, tweening transforms via a
 * TokenAnimator. The visual (size + image) applies immediately; the transform tweens. */
export class TokenView {
  private readonly animator = new TokenAnimator();
  private readonly specs = new Map<string, TokenNodeSpec>();
  /** A locally-dragged token id snaps to its target each reconcile (no tween lag);
   * remote tokens still tween. Set by the move tool via the engine. */
  private dragging: string | null = null;

  constructor(
    private readonly store: ReadableDocuments,
    private readonly assets: AssetResolver,
    private readonly backend: DisplayBackend,
  ) {}

  setDragging(id: string | null): void {
    this.dragging = id;
  }

  reconcile(): void {
    const seen = new Set<string>();
    for (const doc of this.store.query("token")) {
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
      this.backend.removeToken(id);
    }
  }

  tick(dtMs: number): void {
    for (const id of this.animator.tick(dtMs)) this.push(id);
  }

  /** Push a token to the backend with its latest visual + current (tweened) transform. */
  private push(id: string): void {
    const spec = this.specs.get(id);
    const t = this.animator.get(id);
    if (spec && t) this.backend.setToken(id, { ...spec, x: t.x, y: t.y, rotation: t.rotation });
  }

  private toSpec(doc: WireDocument): TokenNodeSpec | null {
    const s = doc.system as TokenSystem | undefined;
    if (!s) return null;
    // Actor-backed tokens resolve their visual via the actor (+ overrides); raw tokens fall
    // back to their own system.visual. Only image visuals render in M10a.
    const eff = resolveTokenActor(doc, this.store);
    const visual = eff?.visual ?? s.visual;
    if (visual?.kind !== "image") return null;
    // Faction border color resolves through the world faction registry; null = no border.
    let borderColor: number | null = null;
    if (eff?.faction) {
      const reg = this.store.query("faction-registry")[0]?.system as FactionRegistrySystem | undefined;
      const hex = reg?.factions?.[eff.faction]?.color;
      if (hex) borderColor = parseColor(hex);
    }
    // Condition badges: resolve the actor's condition ids to registry icon glyphs.
    const badges = resolveConditions(doc, this.store).map((c) => c.icon);
    const box = resolveTokenBox(doc, this.store, eff);
    return {
      x: box.x, y: box.y, w: box.w, h: box.h, rotation: s.rotation ?? 0,
      url: this.assets.url(visual.asset),
      borderColor,
      badges,
      shape: box.shape,
    };
  }
}
