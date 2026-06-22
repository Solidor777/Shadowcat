import type { DocumentStore, AssetResolver, WireDocument } from "@shadowcat/core";
import type { DisplayBackend } from "./backend";
import type { TokenNodeSpec } from "./types";
import { TokenAnimator } from "./token-animator";

/** Engine-reserved token system fields (M8 §4.2; client-owned). `(x,y)` = center. */
interface TokenSystem {
  x: number;
  y: number;
  w: number;
  h: number;
  rotation?: number;
  visual: { kind: string; asset: string };
}

/** Renders `doc_type:"token"` docs as backend token nodes, tweening transforms via a
 * TokenAnimator. The visual (size + image) applies immediately; the transform tweens. */
export class TokenView {
  private readonly animator = new TokenAnimator();
  private readonly specs = new Map<string, TokenNodeSpec>();

  constructor(
    private readonly store: DocumentStore,
    private readonly assets: AssetResolver,
    private readonly backend: DisplayBackend,
  ) {}

  reconcile(): void {
    const seen = new Set<string>();
    for (const doc of this.store.query("token")) {
      const spec = this.toSpec(doc);
      if (!spec) continue;
      seen.add(doc.id);
      this.specs.set(doc.id, spec);
      this.animator.setTarget(doc.id, { x: spec.x, y: spec.y, rotation: spec.rotation });
      this.push(doc.id); // immediate: new tokens snapped, visual current
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
    if (!s || s.visual?.kind !== "image") return null; // only image tokens render in M8d-1
    return {
      x: s.x, y: s.y, w: s.w, h: s.h, rotation: s.rotation ?? 0,
      url: this.assets.url(s.visual.asset),
    };
  }
}
