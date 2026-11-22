import { test, expect, vi, beforeEach } from "vitest";
import * as api from "@shadowcat/core";
import { getFavorites, toggleFavorite, getViewMode, setViewMode } from "./favorites";

beforeEach(() => vi.restoreAllMocks());

/** A `UiState` carrying `slice` as the asset-browser module's stored settings. */
function stateWith(slice: Record<string, unknown> | undefined): api.UiState {
  return {
    global: { locale: "en", lastWorld: null },
    worlds: {},
    modules: slice ? { "asset-browser": slice } : {},
  };
}

test("getFavorites reads the module slice, and is empty when none is stored", async () => {
  vi.spyOn(api, "getUiState").mockResolvedValueOnce(stateWith({ favorites: ["a1", "a2"] }));
  expect(await getFavorites()).toEqual(["a1", "a2"]);
  vi.spyOn(api, "getUiState").mockResolvedValueOnce(stateWith(undefined));
  expect(await getFavorites()).toEqual([]);
});

test("toggleFavorite adds an absent id to the SERVER's list and writes the whole array", async () => {
  vi.spyOn(api, "getUiState").mockResolvedValue(stateWith({ favorites: ["a1"] }));
  const put = vi.spyOn(api, "putUiState").mockResolvedValue();
  expect(await toggleFavorite("a2")).toEqual(["a1", "a2"]);
  expect(put).toHaveBeenCalledWith({ modules: { "asset-browser": { favorites: ["a1", "a2"] } } });
});

test("toggleFavorite removes a present id", async () => {
  vi.spyOn(api, "getUiState").mockResolvedValue(stateWith({ favorites: ["a1", "a2"] }));
  const put = vi.spyOn(api, "putUiState").mockResolvedValue();
  expect(await toggleFavorite("a1")).toEqual(["a2"]);
  expect(put).toHaveBeenCalledWith({ modules: { "asset-browser": { favorites: ["a2"] } } });
});

test("the view mode defaults to grid, reads list when stored, and writes only its own key", async () => {
  vi.spyOn(api, "getUiState").mockResolvedValueOnce(stateWith({ viewMode: "bogus" }));
  expect(await getViewMode()).toBe("grid");
  vi.spyOn(api, "getUiState").mockResolvedValueOnce(stateWith({ viewMode: "list" }));
  expect(await getViewMode()).toBe("list");
  const put = vi.spyOn(api, "putUiState").mockResolvedValue();
  await setViewMode("list");
  expect(put).toHaveBeenCalledWith({ modules: { "asset-browser": { viewMode: "list" } } });
});
