import { render, screen, fireEvent, waitFor } from "@testing-library/svelte";
import { test, expect, vi, beforeEach } from "vitest";
import { setAppContextForTest } from "@shadowcat/ui-kit/test";
import { DocumentStore } from "@shadowcat/core";
import * as api from "@shadowcat/core";
import BulkBar from "./BulkBar.svelte";
import { buildFolderDoc } from "./folderOps";

beforeEach(() => vi.restoreAllMocks());

function bulkCtx() {
  const store = new DocumentStore();
  store.seedDocuments([{ ...buildFolderDoc("w1", "alpha", null), id: "fa" }]);
  return setAppContextForTest({ store, documents: store } as never);
}

const SELECTED = ["a1", "a2"];

test("bulk move-to-folder carries every selected id", async () => {
  const bulk = vi.spyOn(api, "bulkPatchAssets").mockResolvedValue([] as never);
  render(BulkBar, {
    props: { selected: SELECTED, onChanged: vi.fn() },
    context: bulkCtx(),
  });
  await fireEvent.click(screen.getByTestId("bulk-move"));
  await fireEvent.click(screen.getByTestId("bulk-move-target-fa"));
  await waitFor(() =>
    expect(bulk).toHaveBeenCalledWith("w1", {
      ids: SELECTED,
      folder_id: "fa",
      add_tags: [],
      remove_tags: [],
    }),
  );
});

test("bulk tag add/remove sends the tag deltas for every selected id", async () => {
  const bulk = vi.spyOn(api, "bulkPatchAssets").mockResolvedValue([] as never);
  render(BulkBar, {
    props: { selected: SELECTED, onChanged: vi.fn() },
    context: bulkCtx(),
  });
  const input = screen.getByTestId("bulk-tag-input");
  await fireEvent.input(input, { target: { value: "night" } });
  await fireEvent.click(screen.getByTestId("bulk-tag-add"));
  await waitFor(() =>
    expect(bulk).toHaveBeenCalledWith("w1", {
      ids: SELECTED,
      add_tags: ["night"],
      remove_tags: [],
    }),
  );

  await fireEvent.input(input, { target: { value: "night" } });
  await fireEvent.click(screen.getByTestId("bulk-tag-remove"));
  await waitFor(() =>
    expect(bulk).toHaveBeenCalledWith("w1", {
      ids: SELECTED,
      add_tags: [],
      remove_tags: ["night"],
    }),
  );
});

test("bulk delete confirms, then deletes every selected id", async () => {
  const del = vi.spyOn(api, "deleteAsset").mockResolvedValue();
  const changed = vi.fn();
  render(BulkBar, {
    props: { selected: SELECTED, onChanged: changed },
    context: bulkCtx(),
  });
  await fireEvent.click(screen.getByTestId("bulk-delete"));
  expect(del).not.toHaveBeenCalled();
  await fireEvent.click(screen.getByTestId("bulk-delete-confirm"));
  await waitFor(() => expect(del).toHaveBeenCalledTimes(2));
  expect(del).toHaveBeenCalledWith("a1");
  expect(del).toHaveBeenCalledWith("a2");
  expect(changed).toHaveBeenCalled();
});
