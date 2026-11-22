import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent, within } from "@testing-library/svelte";
import { setAppContextForTest } from "@shadowcat/ui-kit/test";
import * as api from "@shadowcat/core";
import { DocumentStore, type TokenVisual } from "@shadowcat/core";
import VisualKindEditor from "./VisualKindEditor.svelte";

beforeEach(() => vi.restoreAllMocks());

/** An app context whose asset picker resolves to `picked`. */
function ctxFor(picked: string | null = "fallback-1") {
  return setAppContextForTest({
    role: "gm",
    world: "w1",
    documents: new DocumentStore(),
    dispatchIntent: vi.fn(),
    pickAsset: vi.fn().mockResolvedValue(picked) as never,
    assets: { url: (id: string) => `/assets/${id}`, reconcile: () => {} } as never,
  });
}

/** Mocks the pool preview query to answer `ids` as the matching asset page. */
function mockPage(ids: string[]) {
  return vi.spyOn(api, "queryAssets").mockResolvedValue({ items: ids.map((id) => ({ id })), next_cursor: null } as never);
}

describe("VisualKindEditor random variant", () => {
  it("builds a pool from the tag filter and the nested fallback visual", async () => {
    mockPage([]);
    const onBuild = vi.fn();
    render(VisualKindEditor, { context: ctxFor(), props: { conditionOptions: [], onBuild } });

    await fireEvent.change(screen.getAllByLabelText("actors.visualKind")[0], { target: { value: "randomVariant" } });
    // The fallback is still unpicked: the pool is incomplete.
    expect(onBuild).toHaveBeenLastCalledWith(null);

    const tagInput = screen.getByTestId("random-variant-tag-input");
    await fireEvent.input(tagInput, { target: { value: "goblin" } });
    await fireEvent.keyDown(tagInput, { key: "Enter" });
    await fireEvent.click(screen.getByTestId("visual-pick"));

    await vi.waitFor(() =>
      expect(onBuild).toHaveBeenLastCalledWith({
        kind: "randomVariant",
        filter: { tags: ["goblin"] },
        fallback: { kind: "image", asset: "fallback-1" },
      }),
    );
  });

  it("carries the chosen folder into the filter", async () => {
    mockPage([]);
    const onBuild = vi.fn();
    const documents = new DocumentStore();
    const context = setAppContextForTest({
      role: "gm",
      world: "w1",
      documents,
      dispatchIntent: vi.fn(),
      pickAsset: vi.fn().mockResolvedValue("fallback-1") as never,
      assets: { url: (id: string) => `/assets/${id}`, reconcile: () => {} } as never,
    });
    documents.seedDocuments([{ id: "folder-1", doc_type: "asset_folder", name: "Goblins", engine: { sort: 0 } } as never]);
    render(VisualKindEditor, { context, props: { conditionOptions: [], onBuild } });

    await fireEvent.change(screen.getAllByLabelText("actors.visualKind")[0], { target: { value: "randomVariant" } });
    await fireEvent.change(screen.getByTestId("random-variant-folder"), { target: { value: "folder-1" } });
    await fireEvent.click(screen.getByTestId("visual-pick"));

    await vi.waitFor(() =>
      expect(onBuild).toHaveBeenLastCalledWith(
        expect.objectContaining({ filter: { folder: "folder-1", tags: [] } }),
      ),
    );
  });

  it("the nested fallback editor does not offer the random-variant kind", async () => {
    mockPage([]);
    render(VisualKindEditor, { context: ctxFor(), props: { conditionOptions: [], onBuild: vi.fn() } });
    const outer = screen.getAllByLabelText("actors.visualKind")[0] as HTMLSelectElement;
    expect(within(outer).queryByText("actors.visualKindRandomVariant")).not.toBeNull();

    await fireEvent.change(outer, { target: { value: "randomVariant" } });
    const selects = screen.getAllByLabelText("actors.visualKind") as HTMLSelectElement[];
    expect(selects).toHaveLength(2);
    expect(within(selects[0]).queryByText("actors.visualKindRandomVariant")).not.toBeNull();
    expect(within(selects[1]).queryByText("actors.visualKindRandomVariant")).toBeNull();
  });

  it("previews the first query-order match and requeries only the first image of the filter", async () => {
    const spy = mockPage(["first", "second"]);
    render(VisualKindEditor, { context: ctxFor(), props: { conditionOptions: [], onBuild: vi.fn() } });
    await fireEvent.change(screen.getAllByLabelText("actors.visualKind")[0], { target: { value: "randomVariant" } });

    const img = (await screen.findByTestId("random-variant-preview")) as HTMLImageElement;
    expect(img.getAttribute("src")).toBe("/assets/first");
    expect(spy).toHaveBeenLastCalledWith("w1", expect.objectContaining({ kind: "image", sort: "created", limit: 1 }));
  });

  it("falls back to the fallback image in the preview when the pool matches nothing", async () => {
    mockPage([]);
    render(VisualKindEditor, { context: ctxFor("fallback-1"), props: { conditionOptions: [], onBuild: vi.fn() } });
    await fireEvent.change(screen.getAllByLabelText("actors.visualKind")[0], { target: { value: "randomVariant" } });
    await fireEvent.click(screen.getByTestId("visual-pick"));

    const img = (await screen.findByTestId("random-variant-preview-fallback")) as HTMLImageElement;
    expect(img.getAttribute("src")).toBe("/assets/fallback-1");
    expect(screen.queryByTestId("random-variant-preview")).toBeNull();
  });

  it("re-seeds the whole pool, filter and fallback, from an initial visual", async () => {
    mockPage([]);
    const onBuild = vi.fn();
    const initial: TokenVisual = {
      kind: "randomVariant",
      filter: { folder: "folder-9", tags: ["orc", "boss"] },
      fallback: { kind: "image", asset: "fb" },
    };
    render(VisualKindEditor, { context: ctxFor(), props: { conditionOptions: [], onBuild, initial } });
    await vi.waitFor(() => expect(onBuild).toHaveBeenLastCalledWith(initial));
  });
});
