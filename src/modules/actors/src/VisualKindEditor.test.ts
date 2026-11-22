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

  const ctxFor = (pickAsset?: unknown) =>
    setAppContextForTest({ role: "gm", world: "w1", documents: new DocumentStore(), dispatchIntent: vi.fn(), pickAsset: pickAsset as never, assets: { url: (id: string) => `/assets/${id}`, reconcile: () => {} } as never });

  it("image kind: authored framing rides along as a transform, a cleared tint drops the key, and a non-positive scale nulls the build", async () => {
    const onBuild = vi.fn();
    render(VisualKindEditor, {
      context: ctxFor(vi.fn().mockResolvedValue("asset-1")),
      props: { conditionOptions: [], onBuild },
    });
    await fireEvent.click(screen.getByTestId("visual-pick"));
    // Untouched framing builds exactly the visual an editor without the disclosure built.
    await vi.waitFor(() => expect(onBuild).toHaveBeenLastCalledWith({ kind: "image", asset: "asset-1" }));

    await fireEvent.change(screen.getByLabelText("actors.transformScale"), { target: { value: "1.5" } });
    await fireEvent.change(screen.getByLabelText("actors.transformAnchorY"), { target: { value: "0.8" } });
    await fireEvent.change(screen.getByLabelText("actors.transformTint"), { target: { value: "#ff8800" } });
    await vi.waitFor(() =>
      expect(onBuild).toHaveBeenLastCalledWith({
        kind: "image",
        asset: "asset-1",
        transform: { anchor: { x: 0.5, y: 0.8 }, scale: 1.5, tint: "#ff8800" },
      }),
    );

    await fireEvent.click(screen.getByText("actors.transformClearTint"));
    await vi.waitFor(() =>
      expect(onBuild).toHaveBeenLastCalledWith({
        kind: "image",
        asset: "asset-1",
        transform: { anchor: { x: 0.5, y: 0.8 }, scale: 1.5 },
      }),
    );

    await fireEvent.change(screen.getByLabelText("actors.transformScale"), { target: { value: "0" } });
    await vi.waitFor(() => expect(onBuild).toHaveBeenLastCalledWith(null));
  });

  it("Art faces: a preset and a typed angle build a normalised facing, and facing 0 drops the key", async () => {
    const onBuild = vi.fn();
    render(VisualKindEditor, { context: ctxFor(vi.fn().mockResolvedValue("asset-1")), props: { conditionOptions: [], onBuild } });
    await fireEvent.click(screen.getByTestId("visual-pick"));

    await fireEvent.change(screen.getByLabelText("actors.transformFacing"), { target: { value: "270" } });
    await vi.waitFor(() =>
      expect(onBuild).toHaveBeenLastCalledWith({ kind: "image", asset: "asset-1", transform: { anchor: { x: 0.5, y: 0.5 }, scale: 1, facing: 270 } }),
    );
    expect((screen.getByLabelText("actors.transformFacingDegrees") as HTMLInputElement).value).toBe("270");

    // A typed angle outside [0, 360) builds its normalised equivalent: the server refuses the raw value.
    await fireEvent.change(screen.getByLabelText("actors.transformFacingDegrees"), { target: { value: "450" } });
    await vi.waitFor(() =>
      expect(onBuild).toHaveBeenLastCalledWith({ kind: "image", asset: "asset-1", transform: { anchor: { x: 0.5, y: 0.5 }, scale: 1, facing: 90 } }),
    );
    await fireEvent.change(screen.getByLabelText("actors.transformFacingDegrees"), { target: { value: "-90" } });
    await vi.waitFor(() =>
      expect(onBuild).toHaveBeenLastCalledWith({ kind: "image", asset: "asset-1", transform: { anchor: { x: 0.5, y: 0.5 }, scale: 1, facing: 270 } }),
    );
    // A non-preset angle selects the "Other angle" entry.
    await fireEvent.change(screen.getByLabelText("actors.transformFacingDegrees"), { target: { value: "45" } });
    await vi.waitFor(() => expect((screen.getByLabelText("actors.transformFacing") as HTMLSelectElement).value).toBe(""));

    await fireEvent.change(screen.getByLabelText("actors.transformFacing"), { target: { value: "0" } });
    await vi.waitFor(() => expect(onBuild).toHaveBeenLastCalledWith({ kind: "image", asset: "asset-1" }));

    await fireEvent.change(screen.getByLabelText("actors.transformFacingDegrees"), { target: { value: "" } });
    await vi.waitFor(() => expect(onBuild).toHaveBeenLastCalledWith({ kind: "image", asset: "asset-1" }));
  });

  it("Art faces: an `initial` visual's facing seeds the controls and re-emits unchanged", async () => {
    const onBuild = vi.fn();
    const initial: TokenVisual = { kind: "image", asset: "p1", transform: { anchor: { x: 0.5, y: 0.5 }, scale: 1, facing: 180 } };
    render(VisualKindEditor, { context: ctxFor(), props: { conditionOptions: [], onBuild, initial } });
    expect((screen.getByLabelText("actors.transformFacing") as HTMLSelectElement).value).toBe("180");
    expect((screen.getByLabelText("actors.transformFacingDegrees") as HTMLInputElement).value).toBe("180");
    await vi.waitFor(() => expect(onBuild).toHaveBeenLastCalledWith(initial));
  });

  it("initializes the framing from an `initial` animated visual and re-emits it unchanged", async () => {
    const onBuild = vi.fn();
    const initial: TokenVisual = {
      kind: "animated",
      source: { type: "frames", frames: ["f1"] },
      fps: 8,
      loop: true,
      transform: { anchor: { x: -0.25, y: 1.5 }, scale: 2, tint: "#102030" },
    };
    render(VisualKindEditor, { context: ctxFor(), props: { conditionOptions: [], onBuild, initial } });

    expect((screen.getByLabelText("actors.transformAnchorX") as HTMLInputElement).value).toBe("-0.25");
    expect((screen.getByLabelText("actors.transformScale") as HTMLInputElement).value).toBe("2");
    await vi.waitFor(() => expect(onBuild).toHaveBeenLastCalledWith(initial));
  });

  it("a generated visual's art and a face row each keep their own framing", async () => {
    const generatedOnBuild = vi.fn();
    const generated: TokenVisual = {
      kind: "generated",
      art: { kind: "image", asset: "p1", transform: { anchor: { x: 0.5, y: 0.5 }, scale: 1.25 } },
      crop: "circle",
      border: null,
      background: null,
    };
    const { unmount } = render(VisualKindEditor, { context: ctxFor(), props: { conditionOptions: [], onBuild: generatedOnBuild, initial: generated } });
    await vi.waitFor(() => expect(generatedOnBuild).toHaveBeenLastCalledWith(generated));
    unmount();

    const facesOnBuild = vi.fn();
    const faces: TokenVisual = {
      kind: "faces",
      faces: {
        normal: { kind: "image", asset: "n1" },
        framed: { kind: "image", asset: "b1", transform: { anchor: { x: 0.5, y: 0.5 }, scale: 3, tint: "#ff0000" } },
      },
      default: "normal",
      faceMap: null,
    };
    render(VisualKindEditor, { context: ctxFor(), props: { conditionOptions: [], onBuild: facesOnBuild, initial: faces } });
    await vi.waitFor(() => expect(facesOnBuild).toHaveBeenLastCalledWith(faces));
  });
});
