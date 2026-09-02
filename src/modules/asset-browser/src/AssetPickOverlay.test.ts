import { render, screen, fireEvent, waitFor } from "@testing-library/svelte";
import { test, expect, vi, beforeEach } from "vitest";
import { setAppContextForTest } from "@shadowcat/ui-kit/test";
import { AssetPickController } from "@shadowcat/ui-kit";
import * as api from "@shadowcat/core";
import AssetPickOverlay from "./AssetPickOverlay.svelte";

beforeEach(() => vi.restoreAllMocks());

function pageWith(ids: string[]) {
  return {
    items: ids.map((id) => ({
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
    })),
    next_cursor: null,
  };
}

function overlay(assetPick = new AssetPickController()) {
  render(AssetPickOverlay, {
    props: {},
    context: setAppContextForTest({ assetPick } as never),
  });
  return assetPick;
}

test("renders nothing until a pick is pending", () => {
  vi.spyOn(api, "queryAssets").mockResolvedValue(pageWith([]) as never);
  overlay();
  expect(screen.queryByTestId("asset-pick-dialog")).toBeNull();
});

test("a request opens the dialog; Escape settles null", async () => {
  vi.spyOn(api, "queryAssets").mockResolvedValue(pageWith([]) as never);
  const c = overlay();
  const picked = c.request({ kind: "image" });
  const dialog = await screen.findByTestId("asset-pick-dialog");
  expect(dialog.getAttribute("aria-modal")).toBe("true");
  await fireEvent.keyDown(window, { key: "Escape" });
  await expect(picked).resolves.toBeNull();
  await waitFor(() => expect(screen.queryByTestId("asset-pick-dialog")).toBeNull());
});

test("confirm settles the selection in pick order; mutations are hidden", async () => {
  vi.spyOn(api, "queryAssets").mockResolvedValue(pageWith(["a1", "a2", "a3"]) as never);
  const c = overlay();
  const picked = c.request({ multiple: true });
  await screen.findByTestId("asset-pick-dialog");
  const tiles = await screen.findAllByTestId("asset-tile");
  // Pick-multiple: plain clicks append in order.
  await fireEvent.click(tiles[2]);
  await fireEvent.click(tiles[0]);
  // No mutation affordances anywhere in pick mode.
  expect(screen.queryByTestId("asset-upload-input")).toBeNull();
  expect(screen.queryByTestId("folder-create-name")).toBeNull();
  expect(screen.queryByTestId("bulk-bar")).toBeNull();
  await fireEvent.click(screen.getByTestId("pick-confirm"));
  await expect(picked).resolves.toEqual(["a3", "a1"]);
});

test("the request's tag presets seed the filter chips", async () => {
  vi.spyOn(api, "queryAssets").mockResolvedValue(pageWith([]) as never);
  const c = overlay();
  void c.request({ tags: ["map"] });
  await screen.findByTestId("asset-pick-dialog");
  expect(await screen.findByTestId("filter-tag-remove-map")).toBeTruthy();
});

test("double-click confirms a single pick immediately", async () => {
  vi.spyOn(api, "queryAssets").mockResolvedValue(pageWith(["a1"]) as never);
  const c = overlay();
  const picked = c.request();
  await screen.findByTestId("asset-pick-dialog");
  const tile = await screen.findByTestId("asset-tile");
  await fireEvent.dblClick(tile);
  // The controller resolves the raw id array; ctx.pickAsset maps single picks.
  await expect(picked).resolves.toEqual(["a1"]);
});
