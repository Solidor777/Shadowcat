import type { AppContext } from "@shadowcat/ui-kit";
import { fxToolState } from "./fxToolState.svelte";

/** The FX scene tool's click handler: plays `fxToolState`'s current config at the clicked
 * scene point on the currently-viewed scene. A player who never opened the config panel
 * (`fxToolState.assetId` still `null`) is prompted inline for an asset before the first
 * placement — every subsequent click in the same tool activation reuses that pick, matching
 * the built-in `place` tool's `selectedAsset`-persists-across-clicks behavior. A world with
 * no active scene (`ctx.viewedSceneId` is `null`) makes this a no-op.
 * @param ctx The live AppContext.
 * @param x The click's scene x-coordinate.
 * @param y The click's scene y-coordinate.
 * @example
 * ```ts
 * import { onFxSceneClick } from "@shadowcat/module-vfx";
 * declare const ctx: AppContext;
 * void onFxSceneClick(ctx, 0, 0);
 * ```
 */
export async function onFxSceneClick(ctx: AppContext, x: number, y: number): Promise<void> {
  const scene = ctx.viewedSceneId;
  if (!scene) return;
  let asset = fxToolState.assetId;
  if (!asset) {
    const picked = await ctx.pickAsset({ kind: "image", tags: ["vfx"] });
    if (!picked) return;
    fxToolState.assetId = picked;
    asset = picked;
  }
  ctx.vfx.play({
    scene,
    asset,
    x,
    y,
    scale: fxToolState.scale,
    sound: fxToolState.soundId ?? undefined,
  });
}
