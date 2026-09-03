import { describe, it, expect, vi } from "vitest";
import { render, screen, fireEvent } from "@testing-library/svelte";
import { setAppContextForTest } from "@shadowcat/ui-kit/test";
import { DocumentStore, type TokenVisual } from "@shadowcat/core";
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

  it("emits a complete generated visual (image art, circle crop, no border/background)", async () => {
    const onBuild = vi.fn();
    const pickAsset = vi.fn().mockResolvedValue("art-1");
    render(VisualKindEditor, {
      context: setAppContextForTest({ role: "gm", world: "w1", documents: new DocumentStore(), dispatchIntent: vi.fn(), pickAsset: pickAsset as never, assets: { url: (id: string) => `/assets/${id}`, reconcile: () => {} } as never }),
      props: { conditionOptions: [], onBuild },
    });

    await fireEvent.change(screen.getByLabelText("actors.visualKind"), { target: { value: "generated" } });
    // No art picked yet: the generated kind is incomplete, like every other kind.
    expect(onBuild).toHaveBeenLastCalledWith(null);

    await fireEvent.click(screen.getByTestId("visual-pick"));
    expect(pickAsset).toHaveBeenCalledWith({ kind: "image" });
    await vi.waitFor(() =>
      expect(onBuild).toHaveBeenLastCalledWith({
        kind: "generated",
        art: { kind: "image", asset: "art-1" },
        crop: "circle",
        border: null,
        background: null,
      }),
    );
  });

  it("generated kind: an enabled border/background rides along, and a non-positive border width nulls the build", async () => {
    const onBuild = vi.fn();
    const pickAsset = vi.fn().mockResolvedValue("art-1");
    render(VisualKindEditor, {
      context: setAppContextForTest({ role: "gm", world: "w1", documents: new DocumentStore(), dispatchIntent: vi.fn(), pickAsset: pickAsset as never, assets: { url: (id: string) => `/assets/${id}`, reconcile: () => {} } as never }),
      props: { conditionOptions: [], onBuild },
    });

    await fireEvent.change(screen.getByLabelText("actors.visualKind"), { target: { value: "generated" } });
    await fireEvent.click(screen.getByTestId("visual-pick"));
    await vi.waitFor(() => expect(onBuild).toHaveBeenLastCalledWith(expect.objectContaining({ kind: "generated" })));

    // Enable the border: the defaults alone already satisfy the renderer's acceptance rule.
    await fireEvent.click(screen.getByLabelText("actors.genBorder"));
    await vi.waitFor(() =>
      expect(onBuild).toHaveBeenLastCalledWith(
        expect.objectContaining({ border: { color: "#ff8800", width: 0.06 } }),
      ),
    );

    // A zero width fails the finite-positive rule `resolveTokenVisual` enforces at the render
    // boundary — the whole visual nulls rather than emitting a visual the renderer would reject.
    await fireEvent.change(screen.getByLabelText("actors.genBorderWidth"), { target: { value: "0" } });
    await vi.waitFor(() => expect(onBuild).toHaveBeenLastCalledWith(null));

    await fireEvent.change(screen.getByLabelText("actors.genBorderWidth"), { target: { value: "0.12" } });
    await fireEvent.click(screen.getByLabelText("actors.genBackground"));
    await vi.waitFor(() =>
      expect(onBuild).toHaveBeenLastCalledWith({
        kind: "generated",
        art: { kind: "image", asset: "art-1" },
        crop: "circle",
        border: { color: "#ff8800", width: 0.12 },
        background: { color: "#102030" },
      }),
    );
  });

  it("generated kind with animated art defers to the animated source's completeness", async () => {
    const onBuild = vi.fn();
    const pickAsset = vi.fn().mockResolvedValue(["f1", "f2"]);
    render(VisualKindEditor, {
      context: setAppContextForTest({ role: "gm", world: "w1", documents: new DocumentStore(), dispatchIntent: vi.fn(), pickAsset: pickAsset as never, assets: { url: (id: string) => `/assets/${id}`, reconcile: () => {} } as never }),
      props: { conditionOptions: [], onBuild },
    });

    await fireEvent.change(screen.getByLabelText("actors.visualKind"), { target: { value: "generated" } });
    await fireEvent.change(screen.getByLabelText("actors.genArt"), { target: { value: "animated" } });
    // No frames picked: the animated art is incomplete, so the whole generated visual is null.
    expect(onBuild).toHaveBeenLastCalledWith(null);

    await fireEvent.click(screen.getByTestId("visual-pick-frames"));
    await vi.waitFor(() =>
      expect(onBuild).toHaveBeenLastCalledWith(
        expect.objectContaining({
          kind: "generated",
          art: { kind: "animated", source: { type: "frames", frames: ["f1", "f2"] }, fps: 8, loop: true },
        }),
      ),
    );
  });

  it("initializes kind + asset from an `initial` image visual", async () => {
    const onBuild = vi.fn();
    render(VisualKindEditor, {
      context: setAppContextForTest({ role: "gm", world: "w1", documents: new DocumentStore(), dispatchIntent: vi.fn(), assets: { url: (id: string) => `/assets/${id}`, reconcile: () => {} } as never }),
      props: { conditionOptions: [], onBuild, initial: { kind: "image", asset: "a1" } },
    });

    expect((screen.getByLabelText("actors.visualKind") as HTMLSelectElement).value).toBe("image");
    await vi.waitFor(() => expect(onBuild).toHaveBeenLastCalledWith({ kind: "image", asset: "a1" }));
  });

  it("initializes kind + all animated fields from an `initial` animated visual", async () => {
    const onBuild = vi.fn();
    const initial: TokenVisual = { kind: "animated", source: { type: "sheet", asset: "sh1", rows: 2, cols: 3, count: 5 }, fps: 12, loop: false };
    render(VisualKindEditor, {
      context: setAppContextForTest({ role: "gm", world: "w1", documents: new DocumentStore(), dispatchIntent: vi.fn(), assets: { url: (id: string) => `/assets/${id}`, reconcile: () => {} } as never }),
      props: { conditionOptions: [], onBuild, initial },
    });

    expect((screen.getByLabelText("actors.visualKind") as HTMLSelectElement).value).toBe("animated");
    await vi.waitFor(() => expect(onBuild).toHaveBeenLastCalledWith(initial));
  });

  it("initializes rows + default + faceMap from an `initial` faces visual", async () => {
    const onBuild = vi.fn();
    const initial: TokenVisual = {
      kind: "faces",
      faces: { normal: { kind: "image", asset: "n1" }, tired: { kind: "animated", source: { type: "frames", frames: ["t1"] }, fps: 4, loop: true } },
      default: "normal",
      faceMap: { prone: "tired" },
    };
    render(VisualKindEditor, {
      context: setAppContextForTest({ role: "gm", world: "w1", documents: new DocumentStore(), dispatchIntent: vi.fn(), assets: { url: (id: string) => `/assets/${id}`, reconcile: () => {} } as never }),
      props: { conditionOptions: [["prone", { name: "Prone", icon: "⬇️" }]], onBuild, initial },
    });

    expect((screen.getByLabelText("actors.visualKind") as HTMLSelectElement).value).toBe("faces");
    const nameInputs = screen.getAllByLabelText("actors.faceName") as HTMLInputElement[];
    expect(nameInputs.map((i) => i.value)).toEqual(["normal", "tired"]);
    expect((screen.getByLabelText("actors.faceDefault") as HTMLSelectElement).value).toBe("normal");
    // The build round-trips the initialized state unchanged.
    await vi.waitFor(() => expect(onBuild).toHaveBeenLastCalledWith(initial));
  });

  it("initializes art/crop/border/background from an `initial` generated visual", async () => {
    const onBuild = vi.fn();
    const initial: TokenVisual = {
      kind: "generated",
      art: { kind: "image", asset: "p1" },
      crop: "square",
      border: { color: "#ff8800", width: 0.1 },
      background: { color: "#102030" },
    };
    render(VisualKindEditor, {
      context: setAppContextForTest({ role: "gm", world: "w1", documents: new DocumentStore(), dispatchIntent: vi.fn(), assets: { url: (id: string) => `/assets/${id}`, reconcile: () => {} } as never }),
      props: { conditionOptions: [], onBuild, initial },
    });

    expect((screen.getByLabelText("actors.visualKind") as HTMLSelectElement).value).toBe("generated");
    expect((screen.getByLabelText("actors.genCrop") as HTMLSelectElement).value).toBe("square");
    expect((screen.getByLabelText("actors.genBorder") as HTMLInputElement).checked).toBe(true);
    expect((screen.getByLabelText("actors.genBorderWidth") as HTMLInputElement).value).toBe("0.1");
    expect((screen.getByLabelText("actors.genBackground") as HTMLInputElement).checked).toBe(true);
    await vi.waitFor(() => expect(onBuild).toHaveBeenLastCalledWith(initial));
  });
});
