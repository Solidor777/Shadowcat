import type { WireDieKind } from "@shadowcat/core";

/** The six standard convex-hull shapes rendered: tetrahedron, cube, octahedron,
 * pentagonal trapezohedron, dodecahedron, icosahedron. A d100 is two `"d10"` dice (tens +
 * ones), never a shape of its own. */
export type DieShapeId = "d4" | "d6" | "d8" | "d10" | "d12" | "d20";

/** One standard shape's id paired with its real geometric face count (a named
 * interface rather than an inline object-literal type so both properties can be
 * documented). */
interface StandardFaceCount {
  /** The shape id. */
  shape: DieShapeId;
  /** The shape's real geometric face count. */
  faces: number;
}

/** Every standard shape's real geometric face count, ascending. */
const STANDARD_FACE_COUNTS: readonly StandardFaceCount[] = [
  { shape: "d4", faces: 4 },
  { shape: "d6", faces: 6 },
  { shape: "d8", faces: 8 },
  { shape: "d10", faces: 10 },
  { shape: "d12", faces: 12 },
  { shape: "d20", faces: 20 },
];

/** A die's resolved physical shape: which standard geometry it renders on, and how many of
 * that geometry's faces carry a real label — the rest render blank. Any `DieKind` uses the
 * standard shape whose face count is >= the kind's face count, with unused faces blank. A
 * `realFaceCount` above 20 (outside the d100 tens/ones split, which the caller handles as two
 * independent `realFaceCount: 10` calls) renders on the `"d20"` body with `sameLabel: true`:
 * EVERY physical face carries the die's final value, so no face can ever show a wrong value
 * chip label; labels are never wrapped. */
export interface ResolvedShape {
  /** Which standard geometry to render. */
  shape: DieShapeId;
  /** That geometry's own face count (>= `realFaceCount`, or 20 for an over-large kind). */
  physicalFaceCount: number;
  /** The die's own face count. When `sameLabel` is false, that many of `physicalFaceCount`'s
   * faces carry a real label and the rest render blank. */
  realFaceCount: number;
  /** `realFaceCount > physicalFaceCount`: every physical face shows the single value label
   * and the remap target is face 0 (a bijection over identical labels). */
  sameLabel: boolean;
}

/**
 * Resolves a die's real label count to the physical shape it renders on.
 * @param realFaceCount The die's own face count (`DieKind::Numeric`'s `max - min + 1`, or
 * `DieKind::Faces.faces.length`).
 * @returns The resolved standard shape, its physical face count, and the real face count.
 * @example
 * ```ts
 * import { shapeFor } from "@shadowcat/module-dice-3d";
 *
 * shapeFor(3); // { shape: "d6", physicalFaceCount: 6, realFaceCount: 3, sameLabel: false }
 * ```
 */
export function shapeFor(realFaceCount: number): ResolvedShape {
  const clamped = Math.max(1, realFaceCount);
  const match = STANDARD_FACE_COUNTS.find((s) => s.faces >= clamped);
  if (match) return { shape: match.shape, physicalFaceCount: match.faces, realFaceCount: clamped, sameLabel: false };
  // Over-large kind: the d20 body as a "value chip" — one label on every face.
  return { shape: "d20", physicalFaceCount: 20, realFaceCount: clamped, sameLabel: true };
}

/** A die's real face count, from its `WireDieKind` (mirrors `dice::spec::DieKind`'s two
 * variants — `Numeric`'s inclusive range, `Faces`'s explicit list length).
 * @param kind The die's face space.
 * @returns The number of real, distinct faces this die has.
 * @example
 * ```ts
 * import { realFaceCountOf } from "@shadowcat/module-dice-3d";
 *
 * realFaceCountOf({ Numeric: { min: 1, max: 20 } }); // 20
 * ```
 */
export function realFaceCountOf(kind: WireDieKind): number {
  return "Numeric" in kind
    ? kind.Numeric.max - kind.Numeric.min + 1
    : kind.Faces.faces.length;
}
