// @vitest-environment node
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { AppContext } from "@shadowcat/ui-kit";
import { fxToolState } from "./fxToolState.svelte";
import { onFxSceneClick } from "./onSceneClick";

/** A minimal AppContext stand-in carrying only the four members the handler reads. */
function fakeCtx(over: Partial<Pick<AppContext, "viewedSceneId" | "pickAsset" | "vfx">> = {}): {
  ctx: AppContext;
  played: Array<Record<string, unknown>>;
  pickAsset: ReturnType<typeof vi.fn>;
} {
  const played: Array<Record<string, unknown>> = [];
  const pickAsset = vi.fn().mockResolvedValue("picked-1");
  const ctx = {
    viewedSceneId: "scene1",
    pickAsset,
    vfx: { play: (req: Record<string, unknown>) => played.push(req), onVfx: () => () => {} },
    ...over,
  } as unknown as AppContext;
  return { ctx, played, pickAsset };
}

beforeEach(() => {
  fxToolState.assetId = null;
  fxToolState.scale = 1;
  fxToolState.soundId = null;
});

describe("onFxSceneClick", () => {
  it("prompts for an asset on first click, then plays with the picked asset and config", async () => {
    const { ctx, played, pickAsset } = fakeCtx();
    fxToolState.scale = 2;
    fxToolState.soundId = "snd1";
    await onFxSceneClick(ctx, 11, 22);
    expect(pickAsset).toHaveBeenCalledWith({ kind: "image", tags: ["vfx"] });
    expect(fxToolState.assetId).toBe("picked-1");
    expect(played).toEqual([
      { scene: "scene1", asset: "picked-1", x: 11, y: 22, scale: 2, sound: "snd1" },
    ]);
  });

  it("reuses the stored asset on later clicks without prompting again", async () => {
    const { ctx, played, pickAsset } = fakeCtx();
    fxToolState.assetId = "already-1";
    await onFxSceneClick(ctx, 1, 2);
    expect(pickAsset).not.toHaveBeenCalled();
    expect(played).toEqual([
      { scene: "scene1", asset: "already-1", x: 1, y: 2, scale: 1, sound: undefined },
    ]);
  });

  it("is a no-op when no scene is viewed", async () => {
    const { ctx, played, pickAsset } = fakeCtx({ viewedSceneId: null });
    await onFxSceneClick(ctx, 1, 2);
    expect(pickAsset).not.toHaveBeenCalled();
    expect(played).toEqual([]);
  });

  it("is a no-op when the inline pick is cancelled", async () => {
    const { ctx, played } = fakeCtx({ pickAsset: vi.fn().mockResolvedValue(null) as never });
    await onFxSceneClick(ctx, 1, 2);
    expect(fxToolState.assetId).toBeNull();
    expect(played).toEqual([]);
  });
});
