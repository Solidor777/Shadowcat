import { render, screen, fireEvent } from "@testing-library/svelte";
import { test, expect, vi } from "vitest";
import { setAppContextForTest } from "@shadowcat/ui-kit/test";
import AssetGrid from "./AssetGrid.svelte";
import type { Asset } from "@shadowcat/types";

function asset(id: string): Asset {
  return {
    id,
    world_id: "w1",
    storage_key: "",
    original_name: `${id}.png`,
    content_type: "image/webp",
    byte_size: 1,
    created_by: null,
    created_at: 0,
    version: 1,
    folder_id: null,
    tags: [],
    derived_tags: [],
    width: null,
    height: null,
    has_alpha: false,
    animated: false,
    original_content_type: "image/png",
    original_byte_size: 1,
    original_retained: false,
    conversion_note: null,
  } as unknown as Asset;
}

const ITEMS = ["a1", "a2", "a3", "a4"].map(asset);

function grid(onSelectionChange = vi.fn(), selected: string[] = []) {
  render(AssetGrid, {
    props: { items: ITEMS, selected, onSelectionChange },
    context: setAppContextForTest(),
  });
  return onSelectionChange;
}

test("a plain click selects exactly that asset", async () => {
  const cb = grid();
  const tiles = screen.getAllByTestId("asset-tile");
  await fireEvent.click(tiles[1]);
  expect(cb).toHaveBeenCalledWith(["a2"]);
});

test("ctrl-click toggles membership without clearing the rest", async () => {
  const cb = grid(vi.fn(), ["a1"]);
  const tiles = screen.getAllByTestId("asset-tile");
  await fireEvent.click(tiles[2], { ctrlKey: true });
  expect(cb).toHaveBeenCalledWith(["a1", "a3"]);
});

test("shift-click extends a contiguous range from the last anchor", async () => {
  const cb = vi.fn();
  render(AssetGrid, {
    props: { items: ITEMS, selected: [], onSelectionChange: cb },
    context: setAppContextForTest(),
  });
  const tiles = screen.getAllByTestId("asset-tile");
  await fireEvent.click(tiles[0]);
  await fireEvent.click(tiles[2], { shiftKey: true });
  expect(cb).toHaveBeenLastCalledWith(["a1", "a2", "a3"]);
});

test("a tile's star reports its asset, marks favorites, and does not select the tile", async () => {
  const onToggleFavorite = vi.fn();
  const onSelectionChange = vi.fn();
  render(AssetGrid, {
    props: { items: ITEMS, selected: [], favorites: ["a2"], onToggleFavorite, onSelectionChange },
    context: setAppContextForTest(),
  });
  const stars = screen.getAllByTestId("asset-favorite");
  expect(stars[1].getAttribute("aria-pressed")).toBe("true");
  expect(stars[0].getAttribute("aria-pressed")).toBe("false");
  await fireEvent.click(stars[0]);
  expect(onToggleFavorite).toHaveBeenCalledWith("a1");
  expect(onSelectionChange).not.toHaveBeenCalled();
});

test("no star is drawn without a toggle handler", () => {
  grid();
  expect(screen.queryByTestId("asset-favorite")).toBeNull();
});

/** A drop target occupying the 100x100 square at `left`. */
function dropTarget(id: string, left: number) {
  const rect = { left, top: 0, width: 100, height: 100, right: left + 100, bottom: 100, x: left, y: 0, toJSON: () => ({}) } as DOMRect;
  return { id, rect: () => rect };
}

/** A grid with two drop targets side by side: "f1" at x 0..100 and "all-assets" at x 200..300. */
function draggableGrid(selected: string[] = []) {
  const onDropAssets = vi.fn();
  const onDragHover = vi.fn();
  render(AssetGrid, {
    props: {
      items: ITEMS,
      selected,
      onSelectionChange: vi.fn(),
      getDropTargets: () => [dropTarget("f1", 0), dropTarget("all-assets", 200)],
      onDropAssets,
      onDragHover,
    },
    context: setAppContextForTest(),
  });
  return { onDropAssets, onDragHover, tiles: screen.getAllByTestId("asset-tile") };
}

test("dragging an unselected tile onto a target drops just that asset", async () => {
  const { onDropAssets, tiles } = draggableGrid(["a3"]);
  await fireEvent.pointerDown(tiles[0], { clientX: 5, clientY: 5 });
  await fireEvent(window, new PointerEvent("pointerup", { clientX: 50, clientY: 50 }));
  expect(onDropAssets).toHaveBeenCalledWith(["a1"], "f1");
});

test("dragging a selected tile drops the whole selection, onto the all-assets target", async () => {
  const { onDropAssets, tiles } = draggableGrid(["a2", "a3"]);
  await fireEvent.pointerDown(tiles[1], { clientX: 5, clientY: 5 });
  await fireEvent(window, new PointerEvent("pointerup", { clientX: 250, clientY: 50 }));
  expect(onDropAssets).toHaveBeenCalledWith(["a2", "a3"], "all-assets");
});

test("releasing outside every target drops nothing", async () => {
  const { onDropAssets, tiles } = draggableGrid();
  await fireEvent.pointerDown(tiles[0], { clientX: 5, clientY: 5 });
  await fireEvent(window, new PointerEvent("pointerup", { clientX: 150, clientY: 50 }));
  expect(onDropAssets).not.toHaveBeenCalled();
});

test("the live drop resolution is reported while dragging and cleared on release", async () => {
  const { onDragHover, tiles } = draggableGrid();
  await fireEvent.pointerDown(tiles[0], { clientX: 5, clientY: 5 });
  await fireEvent(window, new PointerEvent("pointermove", { clientX: 50, clientY: 50 }));
  expect(onDragHover).toHaveBeenLastCalledWith({ dragged: "a1", over: "f1", placement: "into" });
  await fireEvent(window, new PointerEvent("pointerup", { clientX: 50, clientY: 50 }));
  expect(onDragHover).toHaveBeenLastCalledWith(null);
});

test("a touch tap released before the long-press hold never drops", async () => {
  const { onDropAssets, tiles } = draggableGrid();
  await fireEvent.pointerDown(tiles[0], { clientX: 5, clientY: 5, pointerType: "touch" });
  await fireEvent(window, new PointerEvent("pointerup", { clientX: 50, clientY: 50, pointerType: "touch" }));
  expect(onDropAssets).not.toHaveBeenCalled();
});

test("without drop targets a tile press starts no drag", async () => {
  const onDragHover = vi.fn();
  render(AssetGrid, {
    props: { items: ITEMS, selected: [], onSelectionChange: vi.fn(), onDragHover },
    context: setAppContextForTest(),
  });
  await fireEvent.pointerDown(screen.getAllByTestId("asset-tile")[0], { clientX: 5, clientY: 5 });
  await fireEvent(window, new PointerEvent("pointermove", { clientX: 50, clientY: 50 }));
  expect(onDragHover).not.toHaveBeenCalled();
});
