/** The engine-owned canvas z-order (§6.1). Module layers splice between these by
 * fractional `order`; core ids are reserved. Index = the core order key. */
export type CoreLayerId =
  | "background" | "grid" | "tiles" | "regions" | "drawings" | "walls"
  | "tokens" | "templates" | "lighting" | "mask" | "overlays";

export const CORE_LAYERS: readonly CoreLayerId[] = [
  "background", "grid", "tiles", "regions", "drawings", "walls",
  "tokens", "templates", "lighting", "mask", "overlays",
] as const;

interface ModuleLayer {
  id: string;
  order: number;
}

/** Ordered named layer stack — client-only, engine-owned (#6/#7). Core layers are
 * fixed; modules add layers at a fractional `order` relative to core indices. */
export class LayerRegistry {
  private readonly core = new Map<string, number>(
    CORE_LAYERS.map((id, i) => [id, i]),
  );
  private modules: ModuleLayer[] = [];

  /** All layer ids in ascending z-order (core indices + module fractional orders).
   * @returns Layer ids sorted by ascending `order`. `RenderEngine.start()` is the sole
   * production caller — it always requests exactly {@link CORE_LAYERS} today, since no module
   * layer is ever `register()`-ed in production (see that method's doc).
   * @example
   * ```ts
   * import { LayerRegistry, CORE_LAYERS } from "@shadowcat/render";
   *
   * const registry = new LayerRegistry();
   * registry.orderedIds(); // [...CORE_LAYERS] — no module layers registered yet
   * ```
   */
  orderedIds(): string[] {
    const all: { id: string; order: number }[] = [
      ...CORE_LAYERS.map((id, i) => ({ id, order: i })),
      ...this.modules,
    ];
    all.sort((a, b) => a.order - b.order);
    return all.map((l) => l.id);
  }

  /** Register a module layer at a fractional `order` relative to the core indices (e.g. `6.5`
   * splices between `tokens`(6) and `templates`(7)). Throws if `id` collides with a reserved
   * core layer id or an already-registered module layer id. **No production caller today:**
   * `RenderEngine` constructs a private `LayerRegistry` but never calls `register` on it — every
   * `start()` requests exactly {@link CORE_LAYERS} via `orderedIds()` — so no module can splice a
   * layer into the engine z-order through `RenderEngine`'s public API as it stands; this method
   * is exercised only by this package's own tests.
   * @param id The new layer's id; must not collide with a core id or an already-registered one.
   * @param order The layer's position in z-order, relative to the core indices in
   * {@link CORE_LAYERS} (fractional values splice between two core layers).
   * @returns A dispose function that removes exactly this layer when called.
   * @example
   * ```ts
   * import { LayerRegistry } from "@shadowcat/render";
   *
   * const registry = new LayerRegistry();
   * const dispose = registry.register("fx", 6.5); // between tokens(6) and templates(7)
   * dispose(); // removes "fx"
   * ```
   */
  register(id: string, order: number): () => void {
    if (this.core.has(id)) {
      throw new Error(`layer id "${id}" is a reserved core layer`);
    }
    if (this.modules.some((m) => m.id === id)) {
      throw new Error(`layer id "${id}" is already registered`);
    }
    const layer: ModuleLayer = { id, order };
    this.modules.push(layer);
    return () => {
      const i = this.modules.indexOf(layer);
      if (i >= 0) this.modules.splice(i, 1);
    };
  }
}
