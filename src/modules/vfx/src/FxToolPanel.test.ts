import { beforeEach, describe, expect, it, vi } from "vitest";
import { render, screen, fireEvent } from "@testing-library/svelte";
import { ContributionRegistry, SCENE_TOOL_CONTRACT } from "@shadowcat/core";
import { setAppContextForTest } from "@shadowcat/ui-kit/test";
import { fxToolState } from "./fxToolState.svelte";
import FxToolPanel from "./FxToolPanel.svelte";

beforeEach(() => {
  fxToolState.assetId = null;
  fxToolState.scale = 1;
  fxToolState.soundId = null;
});

describe("FxToolPanel", () => {
  it("registers its scene-tool contribution on mount and removes it on unmount", () => {
    const contributions = new ContributionRegistry();
    const { unmount } = render(FxToolPanel, { context: setAppContextForTest({ contributions }) });
    const tools = contributions.contributionsFor(SCENE_TOOL_CONTRACT);
    expect(tools).toHaveLength(1);
    expect(tools[0].sceneTool?.id).toBe("vfx");
    expect(tools[0].sceneTool?.labelKey).toBe("vfx.toolLabel");
    unmount();
    expect(contributions.contributionsFor(SCENE_TOOL_CONTRACT)).toHaveLength(0);
  });

  it("picking an effect stores the pick and renders the preview", async () => {
    const { container } = render(FxToolPanel, {
      context: setAppContextForTest({ pickAsset: (async () => "fx-1") as never }),
    });
    await fireEvent.click(screen.getByTestId("fx-pick-effect"));
    await vi.waitFor(() => expect(fxToolState.assetId).toBe("fx-1"));
    const preview = container.querySelector("[data-testid='fx-preview']") as HTMLImageElement;
    expect(preview).not.toBeNull();
    expect(preview.getAttribute("src")).toBe("/api/assets/fx-1");
  });

  it("the scale input writes the shared state", async () => {
    render(FxToolPanel, { context: setAppContextForTest({}) });
    await fireEvent.input(screen.getByTestId("fx-scale"), { target: { value: "2.5" } });
    expect(fxToolState.scale).toBe(2.5);
  });

  it("picking then clearing a sound toggles the shared state and the clear button", async () => {
    render(FxToolPanel, {
      context: setAppContextForTest({ pickAsset: (async () => "snd-1") as never }),
    });
    expect(screen.queryByTestId("fx-clear-sound")).toBeNull();
    await fireEvent.click(screen.getByTestId("fx-pick-sound"));
    await vi.waitFor(() => expect(fxToolState.soundId).toBe("snd-1"));
    await fireEvent.click(screen.getByTestId("fx-clear-sound"));
    expect(fxToolState.soundId).toBeNull();
    expect(screen.queryByTestId("fx-clear-sound")).toBeNull();
  });
});
