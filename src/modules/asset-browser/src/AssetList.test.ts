import { render, screen, fireEvent } from "@testing-library/svelte";
import { test, expect, vi } from "vitest";
import { setAppContextForTest } from "@shadowcat/ui-kit/test";
import AssetList from "./AssetList.svelte";
import type { Asset } from "@shadowcat/types";

function asset(id: string, contentType = "image/webp"): Asset {
  return {
    id,
    original_name: `${id}.png`,
    content_type: contentType,
    byte_size: 2048,
    tags: ["map"],
    derived_tags: ["image"],
  } as unknown as Asset;
}

const ITEMS = ["a1", "a2", "a3", "a4"].map((id) => asset(id));

function list(props: Record<string, unknown> = {}) {
  const onSelectionChange = vi.fn();
  render(AssetList, {
    props: { items: ITEMS, selected: [], onSelectionChange, ...props },
    context: setAppContextForTest(),
  });
  return onSelectionChange;
}

test("renders one row per item with its name, size and tags", () => {
  list();
  const rows = screen.getAllByTestId("asset-row");
  expect(rows).toHaveLength(4);
  expect(rows[0].textContent).toContain("a1.png");
  expect(rows[0].textContent).toContain("2.0 KiB");
  expect(rows[0].textContent).toContain("map, image");
});

test("a plain click selects exactly that row; ctrl-click toggles; shift-click ranges", async () => {
  const cb = list({ selected: ["a1"] });
  const rows = screen.getAllByTestId("asset-row");
  await fireEvent.click(rows[1]);
  expect(cb).toHaveBeenLastCalledWith(["a2"]);
  await fireEvent.click(rows[2], { ctrlKey: true });
  expect(cb).toHaveBeenLastCalledWith(["a1", "a3"]);
  await fireEvent.click(rows[0]);
  await fireEvent.click(rows[2], { shiftKey: true });
  expect(cb).toHaveBeenLastCalledWith(["a1", "a2", "a3"]);
});

test("the star toggles favorites without selecting the row", async () => {
  const onToggleFavorite = vi.fn();
  const cb = list({ onToggleFavorite, favorites: ["a2"] });
  const stars = screen.getAllByTestId("asset-favorite");
  expect(stars[1].getAttribute("aria-pressed")).toBe("true");
  expect(stars[0].getAttribute("aria-pressed")).toBe("false");
  await fireEvent.click(stars[0]);
  expect(onToggleFavorite).toHaveBeenCalledWith("a1");
  expect(cb).not.toHaveBeenCalled();
});

test("no star is drawn without a toggle handler", () => {
  list();
  expect(screen.queryByTestId("asset-favorite")).toBeNull();
});

test("an empty listing shows the empty state", () => {
  list({ items: [] });
  expect(screen.getByTestId("asset-browser-empty")).toBeTruthy();
});
