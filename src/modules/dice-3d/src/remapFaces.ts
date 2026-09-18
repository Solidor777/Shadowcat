/**
 * Computes which label index each geometric die face should render so the die's UP face
 * (after physics settles) shows `targetIndex` — the server's authoritative result —
 * regardless of which geometric face physics happened to land on. A pure
 * cyclic shift: bijective over `0..faceCount-1`, so every label index still appears on
 * exactly one geometric face; the die's apparent randomness comes entirely from the physics
 * simulation, never from mislabeling.
 * @param upFaceIndex The geometric face index (0-based) the simulation settled with pointing up.
 * @param faceCount The die's total geometric (physical mesh) face count — may exceed the
 * die's own real label count for an unused-face-padded shape (see `shapes.ts`); a face index
 * at or beyond the real label count renders a blank texture, decided by the caller, not here.
 * @param targetIndex The label index (0-based) that must appear on `upFaceIndex`.
 * @returns `labelOrder`, `faceCount` long: `labelOrder[faceIndex]` is the label index to
 * render on that geometric face. `labelOrder[upFaceIndex] === targetIndex` always holds.
 * @example
 * ```ts
 * import { remapFaces } from "@shadowcat/module-dice-3d";
 *
 * remapFaces(0, 6, 3); // [3, 4, 5, 0, 1, 2] — face 0 (up) shows label 3
 * ```
 */
export function remapFaces(upFaceIndex: number, faceCount: number, targetIndex: number): number[] {
  const order: number[] = new Array(faceCount);
  for (let face = 0; face < faceCount; face++) {
    order[face] = (((face - upFaceIndex + targetIndex) % faceCount) + faceCount) % faceCount;
  }
  return order;
}
