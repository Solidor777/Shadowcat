import { render, screen } from "@testing-library/svelte";
import { test, expect, vi, beforeEach } from "vitest";
import { setAppContextForTest } from "@shadowcat/ui-kit/test";
import * as api from "@shadowcat/core";
import AssetBrowser from "./AssetBrowser.svelte";

beforeEach(() => vi.restoreAllMocks());

test("renders the empty state when the world has no assets", async () => {
  vi.spyOn(api, "queryAssets").mockResolvedValue({ items: [], next_cursor: null } as never);
  render(AssetBrowser, {
    props: { mode: "manage" },
    context: setAppContextForTest(),
  });
  expect(await screen.findByTestId("asset-browser-empty")).toBeTruthy();
});

test("renders a thumb tile per queried asset", async () => {
  vi.spyOn(api, "queryAssets").mockResolvedValue({
    items: [
      {
        id: "a1",
        world_id: "w1",
        storage_key: "",
        original_name: "map.png",
        content_type: "image/webp",
        byte_size: 1,
        created_by: null,
        created_at: 0,
        version: 1,
        folder_id: null,
        tags: [],
        derived_tags: ["image"],
      },
    ],
    next_cursor: null,
  } as never);
  render(AssetBrowser, {
    props: { mode: "manage" },
    context: setAppContextForTest(),
  });
  expect(await screen.findByTestId("asset-tile")).toBeTruthy();
});
