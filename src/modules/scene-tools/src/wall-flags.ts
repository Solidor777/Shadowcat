import type { WallEngine } from "@shadowcat/core";

/** The authored "blocks sound" value of a wall as the server resolves it: an explicit
 * `blocksSound` wins, and an absent one inherits the wall's authored `blocksSight`. Mirrors the
 * `blocks_sound` line of the server's `WallEngine::effective_flags`, minus the door step: an
 * editor shows what the wall is authored to do, so an open door does not blank the box.
 * @param wall The wall's engine body.
 * @returns `true` when the wall occludes sound.
 * @example
 * ```
 * declare const wall: WallEngine;
 * wallBlocksSound(wall); // true for a sight-blocking wall that never authored `blocksSound`
 * ```
 */
export function wallBlocksSound(wall: WallEngine): boolean {
  return wall.blocksSound ?? wall.blocksSight === true;
}
