/** The engine-owned canvas z-order (§6.1). Module layers splice between these by
 * fractional `order`; core ids are reserved. Index = the core order key. */
export type CoreLayerId =
  | "background" | "grid" | "tiles" | "drawings" | "walls"
  | "tokens" | "templates" | "lighting" | "mask" | "overlays";

export const CORE_LAYERS: readonly CoreLayerId[] = [
  "background", "grid", "tiles", "drawings", "walls",
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

  /** All layer ids in ascending z-order (core indices + module fractional orders). */
  orderedIds(): string[] {
    const all: { id: string; order: number }[] = [
      ...CORE_LAYERS.map((id, i) => ({ id, order: i })),
      ...this.modules,
    ];
    all.sort((a, b) => a.order - b.order);
    return all.map((l) => l.id);
  }

  /** Register a module layer; returns a dispose removing exactly it. */
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
