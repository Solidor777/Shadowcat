import { describe, it, expect, vi } from "vitest";
import { render, screen, fireEvent, within } from "@testing-library/svelte";
import { setAppContextForTest } from "@shadowcat/ui-kit/test";
import { DocumentStore, buildActorDoc, type BarAttribute, type WireDocument } from "@shadowcat/core";
import ActorsPanel from "./ActorsPanel.svelte";

// Suppress listAssets fetch: EmissionEditor calls listAssets in an effect, which would hit the
// network in jsdom.
vi.mock("@shadowcat/core", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@shadowcat/core")>();
  return { ...actual, listAssets: vi.fn().mockResolvedValue([]) };
});

const hp: BarAttribute = { label: "HP", current: "hp.value", max: 10, color: "#cc2222", mode: "always" };
const base = { displayName: "Troll", visual: { kind: "image" as const, asset: "a1" }, size: { w: 1, h: 1 }, shape: "square" as const, faction: null, conditions: [], prototype: false, vision: null, light: null, movement: [], aura: null, sound: null, vfx: null };

function storeWith(...docs: WireDocument[]): DocumentStore {
  const s = new DocumentStore();
  s.applyCommand({ seq: 1, world_id: "w1", author: "a", ts: 0, ops: docs.map((doc) => ({ op: "create" as const, doc })) });
  return s;
}

describe("ActorsPanel — bars and nameplate", () => {
  it("per-row bar edit commits the whole /engine/bars list with the raw stored list as old", async () => {
    const dispatchIntent = vi.fn();
    const actor = buildActorDoc("w1", "Troll", { ...base, bars: [hp] }, "act1");
    render(ActorsPanel, { context: setAppContextForTest({ role: "gm", world: "w1", documents: storeWith(actor), dispatchIntent }) });
    const row = screen.getByRole("listitem");
    await fireEvent.change(within(row).getByTestId("bar-color"), { target: { value: "#00ff00" } });
    expect(dispatchIntent).toHaveBeenCalledWith([
      { op: "update", doc_id: "act1", changes: [{ path: "/engine/bars", old: [hp], new: [{ ...hp, color: "#00ff00" }] }] },
    ]);
  });

  it("removing the last bar normalizes the write to null", async () => {
    const dispatchIntent = vi.fn();
    const actor = buildActorDoc("w1", "Troll", { ...base, bars: [hp] }, "act1");
    render(ActorsPanel, { context: setAppContextForTest({ role: "gm", world: "w1", documents: storeWith(actor), dispatchIntent }) });
    await fireEvent.click(within(screen.getByRole("listitem")).getByTestId("bar-remove"));
    expect(dispatchIntent).toHaveBeenCalledWith([
      { op: "update", doc_id: "act1", changes: [{ path: "/engine/bars", old: [hp], new: null }] },
    ]);
  });

  it("per-row nameplate mode commits /engine/nameplate_mode with a null old when absent", async () => {
    const dispatchIntent = vi.fn();
    const actor = buildActorDoc("w1", "Troll", base, "act1");
    render(ActorsPanel, { context: setAppContextForTest({ role: "gm", world: "w1", documents: storeWith(actor), dispatchIntent }) });
    const select = within(screen.getByRole("listitem")).getByTestId("actor-nameplate-mode") as HTMLSelectElement;
    expect(select.value).toBe("hover");
    await fireEvent.change(select, { target: { value: "always" } });
    expect(dispatchIntent).toHaveBeenCalledWith([
      { op: "update", doc_id: "act1", changes: [{ path: "/engine/nameplate_mode", old: null, new: "always" }] },
    ]);
  });

  it("create carries the form's bars and nameplate mode, and omits both when untouched", async () => {
    const dispatchIntent = vi.fn();
    const { listAssets } = await import("@shadowcat/core");
    vi.mocked(listAssets).mockResolvedValue([
      { id: "asset-1", world_id: "w1", original_name: "hero.png", content_type: "image/png" } as never,
    ]);
    const pickAsset = vi.fn().mockResolvedValue("asset-1");
    render(ActorsPanel, {
      context: setAppContextForTest({ role: "gm", world: "w1", documents: new DocumentStore(), dispatchIntent, pickAsset: pickAsset as never, assets: { url: (id: string) => `/assets/${id}`, reconcile: () => {} } as never }),
    });
    await fireEvent.input(screen.getByPlaceholderText("actors.name"), { target: { value: "Wraith" } });
    await fireEvent.click(screen.getByTestId("visual-pick"));
    await vi.waitFor(() => expect(pickAsset).toHaveBeenCalled());
    await fireEvent.click(screen.getByText("actors.create"));
    const untouched = dispatchIntent.mock.calls[0][0][0].doc.engine;
    expect("bars" in untouched).toBe(false);
    expect("nameplate_mode" in untouched).toBe(false);

    await fireEvent.input(screen.getByPlaceholderText("actors.name"), { target: { value: "Wraith" } });
    await fireEvent.click(screen.getByTestId("visual-pick"));
    await vi.waitFor(() => expect(pickAsset).toHaveBeenCalledTimes(2));
    await fireEvent.click(screen.getByTestId("bar-add"));
    await fireEvent.change(screen.getByTestId("create-nameplate-mode"), { target: { value: "always" } });
    await fireEvent.click(screen.getByText("actors.create"));
    expect(dispatchIntent.mock.calls[1][0][0].doc.engine).toMatchObject({
      bars: [{ label: "actors.barDefaultLabel", current: 0, max: 1, color: "#cc2222", mode: "hover" }],
      nameplate_mode: "always",
    });
  });
});
