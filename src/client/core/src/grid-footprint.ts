// The single resolved-footprint geometry every token-decoration consumer derives from: given a
// token's shape/size and the scene's grid kind, produces the drawn-box dimensions and the
// bounding-disc collision radius, both in GRID UNITS (multiply by the grid's cell size for scene
// pixels — for hex that size is the circumradius, never `worldUnitsPerCell`, mirroring the
// pathfinder's "footprint radius keeps the indexing scale" convention). Server mirror:
// `resolved_footprint_radius_cells` in the Rust `scene` module, kept in parity by
// `footprint_radius_mirrors_the_client_formula`.

/** A scene's grid kind, as carried on the wire (`Grid.kind`, kept a plain string in v1). Anything
 * other than `"hex"` resolves as square — mirrors the server's `grid_kind_from`'s exact-match
 * fail-safe default. */
export type GridKindLike = string;

/** The resolved footprint: the drawn-box dimensions and the bounding-disc collision radius, both
 * in GRID UNITS. */
export interface ResolvedFootprint {
  /** Drawn box width, in grid units. */
  boxW: number;
  /** Drawn box height, in grid units. */
  boxH: number;
  /** Bounding-disc collision radius, in grid units. */
  radius: number;
}

/**
 * The one definition both the drawn box (`resolveTokenBox`) and the bounding-disc collision
 * radius (`footprintRadius`) derive from — never a second, independently-computed expression.
 *
 * Square (byte-identical to the pre-hex formula): the drawn box is the authored `w × h` block
 * itself; the collision radius is the block's own circumradius (a circle uses `max(w,h)/2`, any
 * other shape its half-diagonal `hypot(w,h)/2` — conservative enclosure).
 *
 * Hex: a token's authored size counts HEXES, not a square block (owner ruling), so `shape` is
 * inert here — a hex tessellation has no "square"/"circle" footprint distinction. `n = max(w,h)`
 * hexes; the drawn box is n hexes' own bounding box (`n·√3` wide, `n·2` tall — a single
 * pointy-top hex's own outer width/height, scaled linearly by `n`); the collision radius is `n`
 * (a single hex's conservative enclosure is its own circumradius, i.e. `1.0` — extending
 * `footprintRadius`'s pre-existing "conservative enclosure" convention to hex, derived from that
 * convention rather than a re-ask).
 * @param shape The token's render/hit-test shape (`"square"`|`"circle"`); inert on a hex grid.
 * @param size The authored footprint size, in grid units (cells on square, hexes on hex).
 * @param gridKind The scene's grid kind; anything but `"hex"` resolves as square.
 * @returns The resolved drawn-box dimensions + collision radius, all in grid units.
 * @example
 * ```ts
 * import { resolveFootprintGeometry } from "@shadowcat/core";
 *
 * resolveFootprintGeometry("square", { w: 1, h: 1 }, "square"); // { boxW: 1, boxH: 1, radius: ~0.707 }
 * resolveFootprintGeometry("square", { w: 1, h: 1 }, "hex");     // { boxW: √3, boxH: 2, radius: 1 }
 * ```
 */
export function resolveFootprintGeometry(
  shape: "square" | "circle",
  size: { /** Authored width, grid units. */ w: number; /** Authored height, grid units. */ h: number },
  gridKind: GridKindLike,
): ResolvedFootprint {
  const { w, h } = size;
  if (gridKind === "hex") {
    const n = Math.max(w, h);
    return { boxW: n * Math.sqrt(3), boxH: n * 2, radius: n };
  }
  return shape === "circle"
    ? { boxW: w, boxH: h, radius: Math.max(w, h) / 2 }
    : { boxW: w, boxH: h, radius: Math.hypot(w, h) / 2 };
}
