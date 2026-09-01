import { describe, it, expect, vi } from "vitest";
import { render, screen, fireEvent, within } from "@testing-library/svelte";
import { tick } from "svelte";
import { setAppContextForTest } from "@shadowcat/ui-kit/test";
import { DocumentStore, buildActorDoc, buildItemDoc, buildTokenFromActor, type WireDocument, type WireOperation } from "@shadowcat/core";
import { TokenSelection } from "@shadowcat/ui-kit";
import ActorsPanel from "./ActorsPanel.svelte";

// Suppress listAssets fetch: ActorsPanel calls listAssets($effect) which hits /api/... in jsdom.
vi.mock("@shadowcat/core", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@shadowcat/core")>();
  return {
    ...actual,
    listAssets: vi.fn().mockResolvedValue([]),
  };
});

const cmd = (ops: WireOperation[]) => ({ seq: 1, world_id: "w1", author: "a", ts: 0, ops });
function storeWith(...docs: WireDocument[]): DocumentStore {
  const s = new DocumentStore();
  s.applyCommand(cmd(docs.map((doc) => ({ op: "create" as const, doc }))));
  return s;
}

describe("ActorsPanel — shape + size", () => {
  it("renders a Shape select with square/circle options in the create form", async () => {
    render(ActorsPanel, {
      context: setAppContextForTest({
        role: "gm",
        world: "w1",
        documents: new DocumentStore(),
        dispatchIntent: vi.fn(),
      }),
    });

    // The create-form shape select (aria-label = "actors.shape")
    const shapeSelect = screen.getByLabelText("actors.shape");
    expect(shapeSelect.tagName).toBe("SELECT");
    expect((shapeSelect as HTMLSelectElement).value).toBe("square");
  });

  it("create form reflects chosen shape (circle) and size (2x2) values", async () => {
    render(ActorsPanel, {
      context: setAppContextForTest({
        role: "gm",
        world: "w1",
        documents: new DocumentStore(),
        dispatchIntent: vi.fn(),
      }),
    });

    const shapeSelect = screen.getByLabelText("actors.shape");
    const widthInput = screen.getByLabelText("actors.width");
    const heightInput = screen.getByLabelText("actors.height");

    await fireEvent.change(shapeSelect, { target: { value: "circle" } });
    await fireEvent.input(widthInput, { target: { value: "2" } });
    await fireEvent.input(heightInput, { target: { value: "2" } });

    expect((shapeSelect as HTMLSelectElement).value).toBe("circle");
    expect((widthInput as HTMLInputElement).value).toBe("2");
    expect((heightInput as HTMLInputElement).value).toBe("2");
  });

  it("create dispatches an actor with the chosen shape and size", async () => {
    const dispatchIntent = vi.fn();
    const { listAssets } = await import("@shadowcat/core");
    // Provide a fake asset so the picker has something to select
    vi.mocked(listAssets).mockResolvedValue([
      { id: "asset-1", world_id: "w1", original_name: "hero.png", content_type: "image/png", byte_size: 100n, created_by: "u-self", created_at: 0n, storage_key: "k1", version: 1n, folder_id: null, tags: [], derived_tags: [], width: null, height: null, has_alpha: false, animated: false, original_content_type: "image/png", original_byte_size: 100n, original_retained: false, conversion_note: null },
    ]);

    render(ActorsPanel, {
      context: setAppContextForTest({
        role: "gm",
        world: "w1",
        documents: new DocumentStore(),
        dispatchIntent,
        assets: { url: (id: string) => `/assets/${id}`, reconcile: () => {} } as never,
      }),
    });

    // Wait for the asset list to populate via the $effect
    await vi.waitFor(() => expect(screen.queryAllByRole("button", { name: "hero.png" }).length).toBeGreaterThan(0));

    // Fill in name
    const nameInput = screen.getByPlaceholderText("actors.name");
    await fireEvent.input(nameInput, { target: { value: "Ogre" } });
    await fireEvent.change(nameInput, { target: { value: "Ogre" } });

    // Pick the asset (enables the create button)
    const assetBtn = screen.getByRole("button", { name: "hero.png" });
    await fireEvent.click(assetBtn);

    // Choose circle shape
    const shapeSelect = screen.getByLabelText("actors.shape");
    await fireEvent.change(shapeSelect, { target: { value: "circle" } });

    // Set size 2x2 — bind:value on number inputs updates on the input event in Svelte 5.
    const widthInput = screen.getByLabelText("actors.width");
    const heightInput = screen.getByLabelText("actors.height");
    await fireEvent.input(widthInput, { target: { value: "2" } });
    await fireEvent.input(heightInput, { target: { value: "2" } });

    // Submit
    const submitBtn = screen.getByText("actors.create");
    await fireEvent.click(submitBtn);

    expect(dispatchIntent).toHaveBeenCalledTimes(1);
    const ops = dispatchIntent.mock.calls[0][0] as WireOperation[];
    const op = ops[0] as { op: string; doc: WireDocument };
    expect(op.op).toBe("create");
    const sys = op.doc.engine as { shape: string; size: { w: number; h: number } };
    expect(sys.shape).toBe("circle");
    expect(sys.size).toEqual({ w: 2, h: 2 });
  });

  it("per-row GM shape edit dispatches update to /system/shape", async () => {
    const dispatchIntent = vi.fn();
    const actor = buildActorDoc(
      "w1",
      "Troll",
      { displayName: "Troll", visual: { kind: "image", asset: "a1" }, size: { w: 1, h: 1 }, shape: "square", faction: null, conditions: [], prototype: false, vision: null, aura: null, sound: null, vfx: null },
      "act1",
    );
    const store = storeWith(actor);

    render(ActorsPanel, {
      context: setAppContextForTest({
        role: "gm",
        world: "w1",
        documents: store,
        dispatchIntent,
      }),
    });

    // Scope to the list item so we get the per-row control, not the create-form control.
    const listItem = screen.getByRole("listitem");
    const rowSelect = within(listItem).getByLabelText("actors.shape");

    await fireEvent.change(rowSelect, { target: { value: "circle" } });

    expect(dispatchIntent).toHaveBeenCalledTimes(1);
    const ops = dispatchIntent.mock.calls[0][0] as WireOperation[];
    expect(ops[0]).toMatchObject({
      op: "update",
      doc_id: "act1",
      changes: [{ path: "/engine/shape", old: "square", new: "circle" }],
    });
  });

  it("per-row GM width edit dispatches update to /system/size", async () => {
    const dispatchIntent = vi.fn();
    const actor = buildActorDoc(
      "w1",
      "Troll",
      { displayName: "Troll", visual: { kind: "image", asset: "a1" }, size: { w: 1, h: 1 }, shape: "square", faction: null, conditions: [], prototype: false, vision: null, aura: null, sound: null, vfx: null },
      "act1",
    );
    const store = storeWith(actor);

    render(ActorsPanel, {
      context: setAppContextForTest({
        role: "gm",
        world: "w1",
        documents: store,
        dispatchIntent,
      }),
    });

    // Scope to the list item so we get the per-row control, not the create-form control.
    const listItem = screen.getByRole("listitem");
    const rowWidthInput = within(listItem).getByLabelText("actors.width");

    await fireEvent.change(rowWidthInput, { target: { value: "3" } });

    expect(dispatchIntent).toHaveBeenCalledTimes(1);
    const ops = dispatchIntent.mock.calls[0][0] as WireOperation[];
    expect(ops[0]).toMatchObject({
      op: "update",
      doc_id: "act1",
      changes: [{ path: "/engine/size", old: { w: 1, h: 1 }, new: { w: 3, h: 1 } }],
    });
  });

  it("per-row GM height edit dispatches update to /system/size preserving width", async () => {
    const dispatchIntent = vi.fn();
    const actor = buildActorDoc(
      "w1",
      "Troll",
      { displayName: "Troll", visual: { kind: "image", asset: "a1" }, size: { w: 2, h: 1 }, shape: "square", faction: null, conditions: [], prototype: false, vision: null, aura: null, sound: null, vfx: null },
      "act1",
    );
    const store = storeWith(actor);

    render(ActorsPanel, {
      context: setAppContextForTest({
        role: "gm",
        world: "w1",
        documents: store,
        dispatchIntent,
      }),
    });

    // Scope to the list item so we get the per-row control, not the create-form control.
    const listItem = screen.getByRole("listitem");
    const rowHeightInput = within(listItem).getByLabelText("actors.height");

    await fireEvent.change(rowHeightInput, { target: { value: "3" } });

    expect(dispatchIntent).toHaveBeenCalledTimes(1);
    const ops = dispatchIntent.mock.calls[0][0] as WireOperation[];
    expect(ops[0]).toMatchObject({
      op: "update",
      doc_id: "act1",
      changes: [{ path: "/engine/size", old: { w: 2, h: 1 }, new: { w: 2, h: 3 } }],
    });
  });
});

describe("ActorsPanel — darkvision authoring", () => {
  it("create includes darkvision vision when a range is entered", async () => {
    const dispatchIntent = vi.fn();
    const { listAssets } = await import("@shadowcat/core");
    vi.mocked(listAssets).mockResolvedValue([
      { id: "asset-1", world_id: "w1", original_name: "hero.png", content_type: "image/png" } as never,
    ]);
    render(ActorsPanel, {
      context: setAppContextForTest({ role: "gm", world: "w1", documents: new DocumentStore(), dispatchIntent, assets: { url: (id: string) => `/assets/${id}`, reconcile: () => {} } as never }),
    });
    await vi.waitFor(() => expect(screen.queryAllByRole("button", { name: "hero.png" }).length).toBeGreaterThan(0));
    await fireEvent.input(screen.getByPlaceholderText("actors.name"), { target: { value: "Drow" } });
    await fireEvent.click(screen.getByRole("button", { name: "hero.png" }));
    await fireEvent.change(screen.getByLabelText("actors.darkvision"), { target: { value: "12" } });
    await fireEvent.click(screen.getByText("actors.create"));

    const ops = dispatchIntent.mock.calls[0][0];
    expect(ops[0].doc.engine).toMatchObject({ vision: [{ mode: "darkvision", range: 12 }] });
  });

  it("create omits vision when darkvision range is 0", async () => {
    const dispatchIntent = vi.fn();
    const { listAssets } = await import("@shadowcat/core");
    vi.mocked(listAssets).mockResolvedValue([
      { id: "asset-1", world_id: "w1", original_name: "hero.png", content_type: "image/png" } as never,
    ]);
    render(ActorsPanel, {
      context: setAppContextForTest({ role: "gm", world: "w1", documents: new DocumentStore(), dispatchIntent, assets: { url: (id: string) => `/assets/${id}`, reconcile: () => {} } as never }),
    });
    await vi.waitFor(() => expect(screen.queryAllByRole("button", { name: "hero.png" }).length).toBeGreaterThan(0));
    await fireEvent.input(screen.getByPlaceholderText("actors.name"), { target: { value: "Human" } });
    await fireEvent.click(screen.getByRole("button", { name: "hero.png" }));
    await fireEvent.click(screen.getByText("actors.create"));
    // `vision` is required-nullable on the generated `ActorEngine` — omitted becomes an
    // explicit `null` (never `undefined`, and never a genuinely-absent key).
    expect(dispatchIntent.mock.calls[0][0][0].doc.engine.vision).toBeNull();
  });

  it("create dispatches the emission editor's pending emissions (and null when untouched)", async () => {
    const dispatchIntent = vi.fn();
    const { listAssets } = await import("@shadowcat/core");
    vi.mocked(listAssets).mockResolvedValue([
      { id: "asset-1", world_id: "w1", original_name: "hero.png", content_type: "image/png" } as never,
    ]);
    render(ActorsPanel, {
      context: setAppContextForTest({ role: "gm", world: "w1", documents: new DocumentStore(), dispatchIntent, assets: { url: (id: string) => `/assets/${id}`, reconcile: () => {} } as never }),
    });
    await vi.waitFor(() => expect(screen.queryAllByRole("button", { name: "hero.png" }).length).toBeGreaterThan(0));
    await fireEvent.input(screen.getByPlaceholderText("actors.name"), { target: { value: "Wisp" } });
    await fireEvent.click(screen.getByRole("button", { name: "hero.png" }));
    // Toggle the aura section on and set its radius; sound/vfx stay null.
    await fireEvent.click(screen.getByLabelText("actors.aura"));
    await fireEvent.change(screen.getByLabelText("actors.auraRadius"), { target: { value: "3" } });
    await fireEvent.click(screen.getByText("actors.create"));
    const engine = dispatchIntent.mock.calls[0][0][0].doc.engine;
    expect(engine.aura).toEqual({ color: "#ffcc66", opacity: 0.4, radius: 3, enabled: true });
    expect(engine.sound).toBeNull();
    expect(engine.vfx).toBeNull();
    // The form reset after create clears the pending emissions too.
    expect((screen.getByLabelText("actors.aura") as HTMLInputElement).checked).toBe(false);
  });

  it("per-row darkvision input shows 0 for a vision assignment carrying range: null", async () => {
    // `VisionAssignment.range` is `number | null` on the wire (an omitted/null range inherits the
    // mode's own default) — the row reads it via `?? 0`, guarding against exactly this case.
    const actor = buildActorDoc(
      "w1",
      "Troll",
      {
        displayName: "Troll",
        visual: { kind: "image", asset: "a1" },
        size: { w: 1, h: 1 },
        shape: "square",
        faction: null,
        conditions: [],
        prototype: false,
        vision: [{ mode: "darkvision", range: null }],
        aura: null,
        sound: null,
        vfx: null,
      },
      "act1",
    );
    const store = storeWith(actor);

    render(ActorsPanel, {
      context: setAppContextForTest({
        role: "gm",
        world: "w1",
        documents: store,
        dispatchIntent: vi.fn(),
      }),
    });

    const listItem = screen.getByRole("listitem");
    const rowDarkvisionInput = within(listItem).getByLabelText("actors.darkvision");
    expect((rowDarkvisionInput as HTMLInputElement).value).toBe("0");
  });
});

describe("ActorsPanel — visual kind editor", () => {
  it("defaults to the image kind and creates an image visual as before", async () => {
    const dispatchIntent = vi.fn();
    const { listAssets } = await import("@shadowcat/core");
    vi.mocked(listAssets).mockResolvedValue([
      { id: "asset-1", world_id: "w1", original_name: "hero.png", content_type: "image/png" } as never,
    ]);
    render(ActorsPanel, {
      context: setAppContextForTest({ role: "gm", world: "w1", documents: new DocumentStore(), dispatchIntent, assets: { url: (id: string) => `/assets/${id}`, reconcile: () => {} } as never }),
    });
    await vi.waitFor(() => expect(screen.queryAllByRole("button", { name: "hero.png" }).length).toBeGreaterThan(0));
    await fireEvent.input(screen.getByPlaceholderText("actors.name"), { target: { value: "Ogre" } });
    await fireEvent.click(screen.getByRole("button", { name: "hero.png" }));
    await fireEvent.click(screen.getByText("actors.create"));
    const ops = dispatchIntent.mock.calls[0][0] as WireOperation[];
    const op = ops[0] as { doc: WireDocument };
    expect(op.doc.engine).toMatchObject({ visual: { kind: "image", asset: "asset-1" } });
  });

  it("switching to the animated kind and choosing frames + fps creates an animated visual", async () => {
    const dispatchIntent = vi.fn();
    const { listAssets } = await import("@shadowcat/core");
    vi.mocked(listAssets).mockResolvedValue([
      { id: "f1", world_id: "w1", original_name: "f1.png", content_type: "image/png" } as never,
      { id: "f2", world_id: "w1", original_name: "f2.png", content_type: "image/png" } as never,
    ]);
    render(ActorsPanel, {
      context: setAppContextForTest({ role: "gm", world: "w1", documents: new DocumentStore(), dispatchIntent, assets: { url: (id: string) => `/assets/${id}`, reconcile: () => {} } as never }),
    });
    await vi.waitFor(() => expect(screen.queryAllByRole("button", { name: "f1.png" }).length).toBeGreaterThan(0));
    await fireEvent.input(screen.getByPlaceholderText("actors.name"), { target: { value: "Wisp" } });
    await fireEvent.change(screen.getByLabelText("actors.visualKind"), { target: { value: "animated" } });
    await fireEvent.click(screen.getByRole("button", { name: "f1.png" }));
    await fireEvent.click(screen.getByRole("button", { name: "f2.png" }));
    await fireEvent.change(screen.getByLabelText("actors.animFps"), { target: { value: "10" } });
    await fireEvent.click(screen.getByText("actors.create"));
    const ops = dispatchIntent.mock.calls[0][0] as WireOperation[];
    const op = ops[0] as { doc: WireDocument };
    expect(op.doc.engine).toMatchObject({ visual: { kind: "animated", source: { type: "frames", frames: ["f1", "f2"] }, fps: 10, loop: true } });
  });

  it("switching to the faces kind with two image faces + a default creates a faces visual", async () => {
    const dispatchIntent = vi.fn();
    const { listAssets } = await import("@shadowcat/core");
    vi.mocked(listAssets).mockResolvedValue([
      { id: "n1", world_id: "w1", original_name: "normal.png", content_type: "image/png" } as never,
      { id: "b1", world_id: "w1", original_name: "bloodied.png", content_type: "image/png" } as never,
    ]);
    const { container } = render(ActorsPanel, {
      context: setAppContextForTest({ role: "gm", world: "w1", documents: new DocumentStore(), dispatchIntent, assets: { url: (id: string) => `/assets/${id}`, reconcile: () => {} } as never }),
    });
    await vi.waitFor(() => expect(screen.queryAllByRole("button", { name: "normal.png" }).length).toBeGreaterThan(0));
    await fireEvent.input(screen.getByPlaceholderText("actors.name"), { target: { value: "Goblin" } });
    await fireEvent.change(screen.getByLabelText("actors.visualKind"), { target: { value: "faces" } });
    await fireEvent.click(screen.getByText("actors.faceAdd"));
    await fireEvent.click(screen.getByText("actors.faceAdd"));
    const nameInputs = screen.getAllByLabelText("actors.faceName");
    await fireEvent.input(nameInputs[0], { target: { value: "normal" } });
    await fireEvent.input(nameInputs[1], { target: { value: "bloodied" } });
    // Each face row renders its own asset-picker instance, so scope the pick to that
    // row's DOM subtree (`.face-row`) rather than a global getAllByRole index — a
    // global index picks the first occurrence across ALL rows' pickers, not "this row's".
    const faceRowEls = container.querySelectorAll(".face-row");
    const normalPickBtn = within(faceRowEls[0] as HTMLElement).getByRole("button", { name: "normal.png" });
    await fireEvent.click(normalPickBtn);
    const bloodiedPickBtn = within(faceRowEls[1] as HTMLElement).getByRole("button", { name: "bloodied.png" });
    await fireEvent.click(bloodiedPickBtn);
    await fireEvent.change(screen.getByLabelText("actors.faceDefault"), { target: { value: "normal" } });
    await fireEvent.click(screen.getByText("actors.create"));
    const ops = dispatchIntent.mock.calls[0][0] as WireOperation[];
    const op = ops[0] as { doc: WireDocument };
    expect(op.doc.engine).toMatchObject({
      visual: { kind: "faces", faces: { normal: { kind: "image", asset: "n1" }, bloodied: { kind: "image", asset: "b1" } }, default: "normal" },
    });
  });

  it("an incomplete face row (kind image, no asset picked) keeps the create button disabled", async () => {
    const dispatchIntent = vi.fn();
    const { listAssets } = await import("@shadowcat/core");
    vi.mocked(listAssets).mockResolvedValue([
      { id: "n1", world_id: "w1", original_name: "normal.png", content_type: "image/png" } as never,
    ]);
    render(ActorsPanel, {
      context: setAppContextForTest({ role: "gm", world: "w1", documents: new DocumentStore(), dispatchIntent, assets: { url: (id: string) => `/assets/${id}`, reconcile: () => {} } as never }),
    });
    await vi.waitFor(() => expect(screen.queryAllByRole("button", { name: "normal.png" }).length).toBeGreaterThan(0));
    await fireEvent.input(screen.getByPlaceholderText("actors.name"), { target: { value: "Goblin" } });
    await fireEvent.change(screen.getByLabelText("actors.visualKind"), { target: { value: "faces" } });
    await fireEvent.click(screen.getByText("actors.faceAdd"));
    const nameInputs = screen.getAllByLabelText("actors.faceName");
    await fireEvent.input(nameInputs[0], { target: { value: "normal" } });
    // Leave the row on kind "image" with no asset picked, then set defaultFace.
    await fireEvent.change(screen.getByLabelText("actors.faceDefault"), { target: { value: "normal" } });

    const submitBtn = screen.getByText("actors.create");
    expect((submitBtn as HTMLButtonElement).disabled).toBe(true);
    await fireEvent.click(submitBtn);
    expect(dispatchIntent).not.toHaveBeenCalled();
  });

  it("duplicate face-row names keep the create button disabled", async () => {
    const dispatchIntent = vi.fn();
    const { listAssets } = await import("@shadowcat/core");
    vi.mocked(listAssets).mockResolvedValue([
      { id: "n1", world_id: "w1", original_name: "normal.png", content_type: "image/png" } as never,
      { id: "b1", world_id: "w1", original_name: "bloodied.png", content_type: "image/png" } as never,
    ]);
    const { container } = render(ActorsPanel, {
      context: setAppContextForTest({ role: "gm", world: "w1", documents: new DocumentStore(), dispatchIntent, assets: { url: (id: string) => `/assets/${id}`, reconcile: () => {} } as never }),
    });
    await vi.waitFor(() => expect(screen.queryAllByRole("button", { name: "normal.png" }).length).toBeGreaterThan(0));
    await fireEvent.input(screen.getByPlaceholderText("actors.name"), { target: { value: "Goblin" } });
    await fireEvent.change(screen.getByLabelText("actors.visualKind"), { target: { value: "faces" } });
    await fireEvent.click(screen.getByText("actors.faceAdd"));
    await fireEvent.click(screen.getByText("actors.faceAdd"));
    const nameInputs = screen.getAllByLabelText("actors.faceName");
    // Both rows get the SAME name — this must be rejected, not silently collapsed.
    await fireEvent.input(nameInputs[0], { target: { value: "normal" } });
    await fireEvent.input(nameInputs[1], { target: { value: "normal" } });
    const faceRowEls = container.querySelectorAll(".face-row");
    const normalPickBtn = within(faceRowEls[0] as HTMLElement).getByRole("button", { name: "normal.png" });
    await fireEvent.click(normalPickBtn);
    const bloodiedPickBtn = within(faceRowEls[1] as HTMLElement).getByRole("button", { name: "bloodied.png" });
    await fireEvent.click(bloodiedPickBtn);
    await fireEvent.change(screen.getByLabelText("actors.faceDefault"), { target: { value: "normal" } });

    const submitBtn = screen.getByText("actors.create");
    expect((submitBtn as HTMLButtonElement).disabled).toBe(true);
    await fireEvent.click(submitBtn);
    expect(dispatchIntent).not.toHaveBeenCalled();
  });
});

describe("ActorsPanel — per-token face swap", () => {
  function facesActor(): WireDocument {
    return buildActorDoc(
      "w1",
      "Goblin",
      { displayName: "Goblin", visual: { kind: "faces", faces: { normal: { kind: "image", asset: "n1" }, bloodied: { kind: "image", asset: "b1" } }, default: "normal", faceMap: null }, size: { w: 1, h: 1 }, shape: "square", faction: null, conditions: [], prototype: false, vision: null, aura: null, sound: null, vfx: null },
      "act1",
    );
  }

  it("shows no face palette when no token is selected", async () => {
    render(ActorsPanel, {
      context: setAppContextForTest({ role: "gm", world: "w1", documents: storeWith(facesActor()), dispatchIntent: vi.fn(), tokenSelection: new TokenSelection(), canEdit: () => true }),
    });
    expect(screen.queryByText("actors.faceSwapHint")).toBeNull();
  });

  it("shows the face palette for a selected token whose visual is 'faces', not for a plain image token", async () => {
    const actor = facesActor();
    const token = buildTokenFromActor("w1", "scene1", actor, "link", { x: 0, y: 0 }, { w: 100, h: 100 }, "tok1");
    const store = storeWith(actor, token);
    const tokenSelection = new TokenSelection();
    tokenSelection.set(["tok1"]);
    render(ActorsPanel, {
      context: setAppContextForTest({ role: "gm", world: "w1", documents: store, dispatchIntent: vi.fn(), tokenSelection, canEdit: () => true }),
    });
    expect(screen.getByText("actors.faceSwapHint")).toBeTruthy();
    expect(screen.getByRole("button", { name: "normal" })).toBeTruthy();
    expect(screen.getByRole("button", { name: "bloodied" })).toBeTruthy();
  });

  it("clicking a face dispatches a /system/face update reading the raw stored value for `old`", async () => {
    const dispatchIntent = vi.fn();
    const actor = facesActor();
    const token = buildTokenFromActor("w1", "scene1", actor, "link", { x: 0, y: 0 }, { w: 100, h: 100 }, "tok1");
    const store = storeWith(actor, token);
    const tokenSelection = new TokenSelection();
    tokenSelection.set(["tok1"]);
    render(ActorsPanel, {
      context: setAppContextForTest({ role: "gm", world: "w1", documents: store, dispatchIntent, tokenSelection, canEdit: () => true }),
    });
    await fireEvent.click(screen.getByRole("button", { name: "bloodied" }));
    expect(dispatchIntent).toHaveBeenCalledWith([
      { op: "update", doc_id: "tok1", changes: [{ path: "/engine/face", old: null, new: "bloodied" }] },
    ]);
  });
});

describe("ActorsPanel — live search + open sheet", () => {
  // Real (not identity) `t` for this describe block: the "Open sheet" assertion below matches
  // the actual rendered label text, not the raw i18n key (the identity default the other
  // `describe` blocks rely on has no space between "open" and "Sheet").
  const realT = (k: string): string => ({ "actors.openSheet": "Open sheet", "actors.search": "Search actors" })[k as "actors.openSheet" | "actors.search"] ?? k;

  function actorDoc(id: string, name = "Goblin"): WireDocument {
    return buildActorDoc(
      "w1",
      name,
      { displayName: name, visual: { kind: "image", asset: "a1" }, size: { w: 1, h: 1 }, shape: "square", faction: null, conditions: [], prototype: false, vision: null, aura: null, sound: null, vfx: null },
      id,
    );
  }

  // A search hit's `document` is a full `WireDocument` clone, permissions envelope included —
  // the panel reads `permissions.property_overrides` off every row it renders, search-sourced
  // or store-resolved alike. Fixtures go through the real builders so they cannot describe a
  // document shape the wire never produces.
  const hit = (document: WireDocument) => ({ document, score: 1, snippet: "" });

  it("opens a sheet for an actor row via ctx.openDocument", async () => {
    const opened: unknown[] = [];
    const store = storeWith(actorDoc("a1"));
    const { getByRole } = render(ActorsPanel, {
      context: setAppContextForTest({
        role: "gm",
        world: "w1",
        documents: store,
        store,
        dispatchIntent: vi.fn(),
        openDocument: (ref) => opened.push(ref),
        t: realT,
      }),
    });
    await fireEvent.click(getByRole("button", { name: /open sheet/i }));
    expect(opened).toEqual([{ docId: "a1" }]);
  });

  it("runs a live search on a non-empty query and lists only actor hits", async () => {
    let capturedOnUpdate: ((hits: unknown[]) => void) | null = null;
    const emptyStore = new DocumentStore();
    const { getByLabelText, findByText } = render(ActorsPanel, {
      context: setAppContextForTest({
        role: "gm",
        world: "w1",
        documents: emptyStore,
        store: emptyStore,
        dispatchIntent: vi.fn(),
        searchDocuments: (_q, _o, onUpdate) => {
          capturedOnUpdate = onUpdate as (hits: unknown[]) => void;
          return Promise.resolve({ unsubscribe() {} });
        },
      }),
    });
    await fireEvent.input(getByLabelText(/search/i), { target: { value: "gob" } });
    capturedOnUpdate!([hit(actorDoc("a9")), hit(buildItemDoc("w1", "Gob-stopper", {}, "i9"))]);
    await findByText("Goblin");
    expect(screen.queryByText("Gob-stopper")).toBeNull();
  });

  it("ignores a stale query's onUpdate firing after a newer query's subscription is active", async () => {
    // Regression: WsClient.subscribeSearch's initial page fires `onUpdate` SYNCHRONOUSLY inside
    // the pending-resolve handler, before `resolve({unsubscribe})` — so it beats the search
    // effect's own `.then()`-based cancellation. Capture both queries' `onUpdate` callbacks and
    // invoke the FIRST (stale) one only after the SECOND (current) query's subscription is
    // already live and has already delivered results; the rendered list must reflect the
    // second query, never the first's late-arriving stale hits.
    const capturedOnUpdate: Array<{ q: string; onUpdate: (hits: unknown[]) => void }> = [];
    const emptyStore = new DocumentStore();
    render(ActorsPanel, {
      context: setAppContextForTest({
        role: "gm",
        world: "w1",
        documents: emptyStore,
        store: emptyStore,
        dispatchIntent: vi.fn(),
        searchDocuments: (q, _o, onUpdate) => {
          capturedOnUpdate.push({ q, onUpdate: onUpdate as (hits: unknown[]) => void });
          return Promise.resolve({ unsubscribe() {} });
        },
      }),
    });

    const searchInput = screen.getByLabelText(/search/i);
    await fireEvent.input(searchInput, { target: { value: "g" } });
    await fireEvent.input(searchInput, { target: { value: "go" } });

    expect(capturedOnUpdate).toHaveLength(2);
    const [first, second] = capturedOnUpdate;
    expect(first.q).toBe("g");
    expect(second.q).toBe("go");

    // Second (current) query's results arrive first.
    second.onUpdate([hit(actorDoc("a-go", "Goliath"))]);
    await screen.findByText("Goliath");

    // First (stale, abandoned) query's results arrive late. Flush reactivity via `tick()` after
    // the call: without it, a still-in-flight DOM update could mask a re-introduced
    // `searchHits`-overwritten-but-not-yet-rendered bug and pass this assertion for the wrong
    // reason.
    first.onUpdate([hit(actorDoc("a-g", "Ghoul"))]);
    await tick();

    expect(screen.queryByText("Ghoul")).toBeNull();
    expect(screen.getByText("Goliath")).toBeTruthy();
  });
});
