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
