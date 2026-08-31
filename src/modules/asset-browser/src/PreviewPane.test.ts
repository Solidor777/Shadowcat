import { render, screen, fireEvent, waitFor } from "@testing-library/svelte";
import { test, expect, vi, beforeEach } from "vitest";
import { setAppContextForTest } from "@shadowcat/ui-kit/test";
import * as api from "@shadowcat/core";
import PreviewPane from "./PreviewPane.svelte";
import type { Asset } from "@shadowcat/types";

beforeEach(() => vi.restoreAllMocks());

function asset(over: Record<string, unknown> = {}): Asset {
  return {
    id: "a1",
    world_id: "w1",
    storage_key: "",
    original_name: "map.png",
    content_type: "image/webp",
    byte_size: 10,
    created_by: null,
    created_at: 0,
    version: 3,
    folder_id: null,
    tags: ["hero"],
    derived_tags: ["image", "webp"],
    width: 32,
    height: 32,
    has_alpha: false,
    animated: false,
    original_content_type: "image/png",
    original_byte_size: 20,
    original_retained: true,
    conversion_note: null,
    ...over,
  } as unknown as Asset;
}

function pane(a: Asset, onChanged = vi.fn()) {
  render(PreviewPane, {
    props: { asset: a, mutable: true, onChanged },
    context: setAppContextForTest(),
  });
  return onChanged;
}

test("adding a tag patches the FULL replacement explicit set", async () => {
  const patch = vi.spyOn(api, "patchAsset").mockResolvedValue(asset() as never);
  pane(asset());
  const input = screen.getByTestId("preview-tag-input");
  await fireEvent.input(input, { target: { value: "boss" } });
  await fireEvent.keyDown(input, { key: "Enter" });
  await waitFor(() => expect(patch).toHaveBeenCalledWith("a1", { tags: ["hero", "boss"] }));
});

test("removing a tag patches the remaining set; derived tags carry no remover", async () => {
  const patch = vi.spyOn(api, "patchAsset").mockResolvedValue(asset() as never);
  pane(asset());
  await fireEvent.click(screen.getByTestId("preview-tag-remove-hero"));
  await waitFor(() => expect(patch).toHaveBeenCalledWith("a1", { tags: [] }));
  expect(screen.queryByTestId("preview-tag-remove-image")).toBeNull();
  expect(screen.getByTestId("preview-derived-image")).toBeTruthy();
});

test("original affordances render only when the original is retained", () => {
  pane(asset());
  expect(screen.getByTestId("preview-download-original")).toBeTruthy();
  expect(
    (screen.getByTestId("preview-reconvert") as HTMLButtonElement).disabled,
  ).toBe(false);
});

test("without a retained original there is no download link and reconvert is disabled", () => {
  pane(asset({ original_retained: false }));
  expect(screen.queryByTestId("preview-download-original")).toBeNull();
  expect(
    (screen.getByTestId("preview-reconvert") as HTMLButtonElement).disabled,
  ).toBe(true);
});

test("delete asks for confirmation before calling deleteAsset", async () => {
  const del = vi.spyOn(api, "deleteAsset").mockResolvedValue();
  const changed = pane(asset());
  await fireEvent.click(screen.getByTestId("preview-delete"));
  expect(del).not.toHaveBeenCalled();
  await fireEvent.click(screen.getByTestId("preview-delete-confirm"));
  await waitFor(() => expect(del).toHaveBeenCalledWith("a1"));
  expect(changed).toHaveBeenCalled();
});

test("rename patches the display name", async () => {
  const patch = vi.spyOn(api, "patchAsset").mockResolvedValue(asset() as never);
  pane(asset());
  await fireEvent.click(screen.getByTestId("preview-rename"));
  const input = screen.getByTestId("preview-rename-input");
  await fireEvent.input(input, { target: { value: "dragon.png" } });
  await fireEvent.keyDown(input, { key: "Enter" });
  await waitFor(() => expect(patch).toHaveBeenCalledWith("a1", { name: "dragon.png" }));
});
