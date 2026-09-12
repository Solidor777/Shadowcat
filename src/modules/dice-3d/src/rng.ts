/**
 * Deterministic 32-bit string hash (FNV-1a) — turns a roll id (a UUID string) into a seed
 * for `mulberry32`, so every client throws the SAME initial velocities for the same roll:
 * the server's authority is over the VALUE, not the animation.
 * @param rollId The roll's stable id.
 * @returns A 32-bit unsigned seed.
 * @example
 * ```ts
 * import { seedFromRollId } from "@shadowcat/module-dice-3d";
 *
 * seedFromRollId("11111111-1111-1111-1111-111111111111");
 * ```
 */
export function seedFromRollId(rollId: string): number {
  let h = 0x811c9dc5;
  for (let i = 0; i < rollId.length; i++) {
    h ^= rollId.charCodeAt(i);
    h = Math.imul(h, 0x01000193);
  }
  return h >>> 0;
}

/**
 * mulberry32: a small, fast, seeded PRNG (public-domain algorithm; clean-room
 * implementation from the published bit-mixing formula, no vendored source). Deterministic
 * for a given seed — the SAME seed always produces the SAME sequence of floats in `[0, 1)`.
 * @param seed A 32-bit unsigned seed (e.g. from `seedFromRollId`).
 * @returns A generator function; each call advances the sequence and returns the next float.
 * @example
 * ```ts
 * import { mulberry32 } from "@shadowcat/module-dice-3d";
 *
 * const next = mulberry32(42);
 * next(); // a float in [0, 1)
 * ```
 */
export function mulberry32(seed: number): () => number {
  let a = seed >>> 0;
  return function next(): number {
    a = (a + 0x6d2b79f5) | 0;
    let t = Math.imul(a ^ (a >>> 15), 1 | a);
    t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t;
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}
