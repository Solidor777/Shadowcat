import { describe, it, expect, vi } from "vitest";
import { render, screen, fireEvent, within } from "@testing-library/svelte";
import { setAppContextForTest } from "@shadowcat/ui-kit/test";
import { DocumentStore, buildActorDoc, type WireDocument, type WireOperation } from "@shadowcat/core";
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
      { id: "asset-1", world_id: "w1", original_name: "hero.png", content_type: "image/png", byte_size: 100n, created_by: "u-self", created_at: 0n, storage_key: "k1", version: 1n },
    ]);

    render(ActorsPanel, {
      context: setAppContextForTest({
        role: "gm",
        world: "w1",
        documents: new DocumentStore(),
        dispatchIntent,
        assets: { url: (id: string) => `/assets/${id}` } as never,
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
    const sys = op.doc.system as { shape: string; size: { w: number; h: number } };
    expect(sys.shape).toBe("circle");
    expect(sys.size).toEqual({ w: 2, h: 2 });
  });

  it("per-row GM shape edit dispatches update to /system/shape", async () => {
    const dispatchIntent = vi.fn();
    const actor = buildActorDoc(
      "w1",
      { name: "Troll", displayName: "Troll", visual: { kind: "image", asset: "a1" }, size: { w: 1, h: 1 }, shape: "square", faction: null, conditions: [], prototype: false },
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
      changes: [{ path: "/system/shape", old: "square", new: "circle" }],
    });
  });

  it("per-row GM width edit dispatches update to /system/size", async () => {
    const dispatchIntent = vi.fn();
    const actor = buildActorDoc(
      "w1",
      { name: "Troll", displayName: "Troll", visual: { kind: "image", asset: "a1" }, size: { w: 1, h: 1 }, shape: "square", faction: null, conditions: [], prototype: false },
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
      changes: [{ path: "/system/size", old: { w: 1, h: 1 }, new: { w: 3, h: 1 } }],
    });
  });

  it("per-row GM height edit dispatches update to /system/size preserving width", async () => {
    const dispatchIntent = vi.fn();
    const actor = buildActorDoc(
      "w1",
      { name: "Troll", displayName: "Troll", visual: { kind: "image", asset: "a1" }, size: { w: 2, h: 1 }, shape: "square", faction: null, conditions: [], prototype: false },
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
      changes: [{ path: "/system/size", old: { w: 2, h: 1 }, new: { w: 2, h: 3 } }],
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
      context: setAppContextForTest({ role: "gm", world: "w1", documents: new DocumentStore(), dispatchIntent, assets: { url: (id: string) => `/assets/${id}` } as never }),
    });
    await vi.waitFor(() => expect(screen.queryAllByRole("button", { name: "hero.png" }).length).toBeGreaterThan(0));
    await fireEvent.input(screen.getByPlaceholderText("actors.name"), { target: { value: "Drow" } });
    await fireEvent.click(screen.getByRole("button", { name: "hero.png" }));
    await fireEvent.change(screen.getByLabelText("actors.darkvision"), { target: { value: "12" } });
    await fireEvent.click(screen.getByText("actors.create"));

    const ops = dispatchIntent.mock.calls[0][0];
    expect(ops[0].doc.system).toMatchObject({ vision: [{ mode: "darkvision", range: 12 }] });
  });

  it("create omits vision when darkvision range is 0", async () => {
    const dispatchIntent = vi.fn();
    const { listAssets } = await import("@shadowcat/core");
    vi.mocked(listAssets).mockResolvedValue([
      { id: "asset-1", world_id: "w1", original_name: "hero.png", content_type: "image/png" } as never,
    ]);
    render(ActorsPanel, {
      context: setAppContextForTest({ role: "gm", world: "w1", documents: new DocumentStore(), dispatchIntent, assets: { url: (id: string) => `/assets/${id}` } as never }),
    });
    await vi.waitFor(() => expect(screen.queryAllByRole("button", { name: "hero.png" }).length).toBeGreaterThan(0));
    await fireEvent.input(screen.getByPlaceholderText("actors.name"), { target: { value: "Human" } });
    await fireEvent.click(screen.getByRole("button", { name: "hero.png" }));
    await fireEvent.click(screen.getByText("actors.create"));
    expect(dispatchIntent.mock.calls[0][0][0].doc.system.vision).toBeUndefined();
  });
});
