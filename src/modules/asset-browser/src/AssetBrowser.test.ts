import { render, screen, waitFor, fireEvent } from "@testing-library/svelte";
import { test, expect, vi, beforeEach } from "vitest";
import { setAppContextForTest } from "@shadowcat/ui-kit/test";
import * as api from "@shadowcat/core";
import AssetBrowser from "./AssetBrowser.svelte";
import { buildFolderDoc } from "./folderOps";

beforeEach(() => vi.restoreAllMocks());

test("the audio container selector appears only once a queued file is audio", async () => {
  vi.spyOn(api, "queryAssets").mockResolvedValue({ items: [], next_cursor: null } as never);
  const start = vi.spyOn(api, "startChunkedUpload").mockResolvedValue({ id: "a1" } as never);
  render(AssetBrowser, {
    props: { mode: "manage" },
    context: setAppContextForTest(),
  });
  await screen.findByTestId("asset-browser-empty");
  // Nothing queued: no audio in flight, no selector.
  expect(screen.queryByTestId("audio-containers")).toBeNull();

  const { fireEvent } = await import("@testing-library/svelte");
  const input = screen.getByTestId("asset-upload-input");
  const audioFile = new File([new Uint8Array([1, 2, 3])], "loop.wav", { type: "audio/wav" });
  await fireEvent.change(input, { target: { files: [audioFile] } });
  const select = (await screen.findByTestId("audio-containers")) as HTMLSelectElement;
  expect(select.value).toBe("both");
  expect(Array.from(select.options).map((o) => o.value)).toEqual(["both", "ogg", "webm"]);

  // The default selection reaches the upload options.
  const { waitFor } = await import("@testing-library/svelte");
  await waitFor(() =>
    expect(start).toHaveBeenCalledWith(
      "w1",
      expect.anything(),
      expect.objectContaining({ audioContainers: "both" }),
    ),
  );
});

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

test("dragging a tile onto a folder row files it there, and onto the all-assets row moves it to the root", async () => {
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
        folder_id: "f1",
        tags: [],
        derived_tags: ["image"],
      },
    ],
    next_cursor: null,
  } as never);
  const patch = vi.spyOn(api, "patchAsset").mockResolvedValue({} as never);
  const store = new api.DocumentStore();
  store.seedDocuments([{ ...buildFolderDoc("w1", "maps", null), id: "f1" }]);
  render(AssetBrowser, {
    props: { mode: "manage" },
    context: setAppContextForTest({ store, documents: store }),
  });
  const tile = await screen.findByTestId("asset-tile");
  const rectAt = (top: number) => () => ({ left: 0, right: 100, top, bottom: top + 40, width: 100, height: 40, x: 0, y: top, toJSON: () => ({}) }) as DOMRect;
  const all = screen.getByTestId("folder-node-all");
  all.getBoundingClientRect = rectAt(0);
  (screen.getByTestId("folder-node-f1").closest(".row") as HTMLElement).getBoundingClientRect = rectAt(100);

  await fireEvent.pointerDown(tile, { clientX: 500, clientY: 500 });
  await fireEvent(window, new PointerEvent("pointerup", { clientX: 50, clientY: 120 }));
  await waitFor(() => expect(patch).toHaveBeenCalledWith("a1", { folder_id: "f1" }));

  await fireEvent.pointerDown(tile, { clientX: 500, clientY: 500 });
  await fireEvent(window, new PointerEvent("pointerup", { clientX: 50, clientY: 20 }));
  await waitFor(() => expect(patch).toHaveBeenCalledWith("a1", { folder_id: null }));
});

test("filter changes map 1:1 onto queryAssets params, q vs nameRegex exclusive", async () => {
  const q = vi
    .spyOn(api, "queryAssets")
    .mockResolvedValue({ items: [], next_cursor: null } as never);
  render(AssetBrowser, { props: { mode: "manage" }, context: setAppContextForTest() });
  await screen.findByTestId("asset-browser-empty");

  const { fireEvent } = await import("@testing-library/svelte");
  await fireEvent.input(screen.getByTestId("filter-query"), { target: { value: "drag" } });
  await waitFor(() =>
    expect(q).toHaveBeenLastCalledWith(
      "w1",
      expect.objectContaining({ q: "drag", nameRegex: undefined }),
    ),
  );

  await fireEvent.click(screen.getByTestId("filter-regex-toggle"));
  await waitFor(() =>
    expect(q).toHaveBeenLastCalledWith(
      "w1",
      expect.objectContaining({ nameRegex: "drag", q: undefined }),
    ),
  );
});

test("an asset notice refetches the listing", async () => {
  const q = vi
    .spyOn(api, "queryAssets")
    .mockResolvedValue({ items: [], next_cursor: null } as never);
  let notify: ((m: unknown) => void) | undefined;
  render(AssetBrowser, {
    props: { mode: "manage" },
    context: setAppContextForTest({
      onAssetChanged: (cb) => {
        notify = cb as (m: unknown) => void;
        return () => {};
      },
    }),
  });
  await screen.findByTestId("asset-browser-empty");
  const before = q.mock.calls.length;
  notify?.({ uuid: "a9", op: "created", version: 1 });
  await waitFor(() => expect(q.mock.calls.length).toBeGreaterThan(before));
});
