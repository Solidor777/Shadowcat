import { describe, it, expect, vi } from "vitest";
import { render, screen, fireEvent } from "@testing-library/svelte";
import { setAppContextForTest } from "@shadowcat/ui-kit/test";
import { DocumentStore } from "@shadowcat/core";
import VisualKindEditor from "./VisualKindEditor.svelte";

describe("VisualKindEditor", () => {
  it("emits a complete image visual via onBuild when an asset is picked (image kind)", async () => {
    const onBuild = vi.fn();
    const pickAsset = vi.fn().mockResolvedValue("asset-1");
    render(VisualKindEditor, {
      context: setAppContextForTest({ role: "gm", world: "w1", documents: new DocumentStore(), dispatchIntent: vi.fn(), pickAsset: pickAsset as never, assets: { url: (id: string) => `/assets/${id}`, reconcile: () => {} } as never }),
      props: { conditionOptions: [], onBuild },
    });

    await fireEvent.click(screen.getByTestId("visual-pick"));
    expect(pickAsset).toHaveBeenCalledWith({ kind: "image" });
    await vi.waitFor(() =>
      expect(onBuild).toHaveBeenLastCalledWith({ kind: "image", asset: "asset-1" }),
    );
  });

  it("the frames pick replaces the ordered frame list wholesale", async () => {
    const onBuild = vi.fn();
    const pickAsset = vi.fn().mockResolvedValue(["f2", "f1"]);
    render(VisualKindEditor, {
      context: setAppContextForTest({ role: "gm", world: "w1", documents: new DocumentStore(), dispatchIntent: vi.fn(), pickAsset: pickAsset as never, assets: { url: (id: string) => `/assets/${id}`, reconcile: () => {} } as never }),
      props: { conditionOptions: [], onBuild },
    });
    await fireEvent.change(screen.getByLabelText("actors.visualKind"), { target: { value: "animated" } });
    await fireEvent.click(screen.getByTestId("visual-pick-frames"));
    expect(pickAsset).toHaveBeenCalledWith({ kind: "image", multiple: true });
    await vi.waitFor(() =>
      expect(onBuild).toHaveBeenLastCalledWith(
        expect.objectContaining({ kind: "animated", source: { type: "frames", frames: ["f2", "f1"] } }),
      ),
    );
  });

  it("a cancelled pick (null) leaves the visual state untouched", async () => {
    const onBuild = vi.fn();
    const pickAsset = vi.fn().mockResolvedValue(null);
    render(VisualKindEditor, {
      context: setAppContextForTest({ role: "gm", world: "w1", documents: new DocumentStore(), dispatchIntent: vi.fn(), pickAsset: pickAsset as never, assets: { url: (id: string) => `/assets/${id}`, reconcile: () => {} } as never }),
      props: { conditionOptions: [], onBuild },
    });
    await fireEvent.click(screen.getByTestId("visual-pick"));
    await vi.waitFor(() => expect(pickAsset).toHaveBeenCalled());
    // No asset applied: the image visual stays incomplete.
    expect(onBuild).toHaveBeenLastCalledWith(null);
  });

  it("emits null via onBuild for an incomplete faces row (no asset picked)", async () => {
    const onBuild = vi.fn();
    render(VisualKindEditor, {
      context: setAppContextForTest({ role: "gm", world: "w1", documents: new DocumentStore(), dispatchIntent: vi.fn(), assets: { url: (id: string) => `/assets/${id}`, reconcile: () => {} } as never }),
      props: { conditionOptions: [], onBuild },
    });

    await fireEvent.change(screen.getByLabelText("actors.visualKind"), { target: { value: "faces" } });
    await fireEvent.click(screen.getByText("actors.faceAdd"));
    await fireEvent.input(screen.getByLabelText("actors.faceName"), { target: { value: "normal" } });
    await fireEvent.change(screen.getByLabelText("actors.faceDefault"), { target: { value: "normal" } });

    // An image face row with no asset picked is incomplete → the whole faces visual is null.
    expect(onBuild).toHaveBeenLastCalledWith(null);
  });
});
