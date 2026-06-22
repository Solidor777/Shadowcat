import { render, screen, waitFor, fireEvent } from "@testing-library/svelte";
import { test, expect, vi, beforeEach } from "vitest";
import Harness from "./__fixtures__/AssetsHarness.svelte";
import * as api from "../../../lib/api";

beforeEach(() => vi.restoreAllMocks());

test("renders a thumbnail grid from listAssets", async () => {
  vi.spyOn(api, "listAssets").mockResolvedValue([
    {
      id: "a1",
      world_id: "w1",
      storage_key: "",
      original_name: "map.png",
      content_type: "image/png",
      byte_size: 1,
      created_by: "u",
      created_at: 0,
      version: 1,
    },
  ] as never);
  render(Harness);
  const tile = await screen.findByTestId("asset-tile");
  expect(tile).toBeTruthy();
  expect(screen.getByText("map.png")).toBeTruthy();
});

test("uploading a file calls uploadAsset then reloads", async () => {
  vi.spyOn(api, "listAssets").mockResolvedValue([] as never);
  const upload = vi.spyOn(api, "uploadAsset").mockResolvedValue({ id: "a1" } as never);
  render(Harness);
  const input = await screen.findByTestId("asset-upload");
  const file = new File([new Uint8Array([1])], "x.png", { type: "image/png" });
  await fireEvent.change(input, { target: { files: [file] } });
  await waitFor(() => expect(upload).toHaveBeenCalledWith("w1", file));
});

test("deleting an asset calls deleteAsset and removes the tile", async () => {
  const asset = {
    id: "a1",
    world_id: "w1",
    storage_key: "",
    original_name: "map.png",
    content_type: "image/png",
    byte_size: 1,
    created_by: "u",
    created_at: 0,
    version: 1,
  };
  // Mount reload returns the asset; the post-delete reload returns empty.
  vi.spyOn(api, "listAssets")
    .mockResolvedValueOnce([asset] as never)
    .mockResolvedValueOnce([] as never);
  const del = vi.spyOn(api, "deleteAsset").mockResolvedValue(undefined);
  render(Harness);
  await screen.findByTestId("asset-tile");
  await fireEvent.click(screen.getByRole("button", { name: "Delete" }));
  await waitFor(() => expect(del).toHaveBeenCalledWith("a1"));
  await waitFor(() => expect(screen.queryByTestId("asset-tile")).toBeNull());
});

test("an asset_changed notice triggers a reload", async () => {
  const list = vi.spyOn(api, "listAssets").mockResolvedValue([] as never);
  let fire: (m: { uuid: string; op: "replaced" | "deleted" }) => void = () => {};
  render(Harness, {
    props: {
      onAssetChanged: (cb: typeof fire) => {
        fire = cb;
        return () => {};
      },
    },
  });
  await waitFor(() => expect(list).toHaveBeenCalledTimes(1));
  fire({ uuid: "a1", op: "deleted" });
  await waitFor(() => expect(list).toHaveBeenCalledTimes(2));
});
