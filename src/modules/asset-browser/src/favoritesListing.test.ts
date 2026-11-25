import { test, expect, vi, beforeEach } from "vitest";
import * as api from "@shadowcat/core";
import { queryFavoriteAssets } from "./favoritesListing";

beforeEach(() => vi.restoreAllMocks());

/** A minimal asset row. */
function asset(id: string, name: string, size: number, created: number) {
  return { id, original_name: name, byte_size: size, created_at: created };
}

test("no ids sends no request", async () => {
  const q = vi.spyOn(api, "queryAssets");
  expect(await queryFavoriteAssets("w1", {}, [])).toEqual([]);
  expect(q).not.toHaveBeenCalled();
});

test("ids beyond the per-request cap go in separate requests and merge in the requested order", async () => {
  const ids = Array.from({ length: api.MAX_QUERY_IDS + 1 }, (_, i) => `id-${String(i).padStart(3, "0")}`);
  const q = vi.spyOn(api, "queryAssets").mockImplementation(
    async (_world, query) =>
      ({
        // Each chunk answers in its own name order; the last id carries the smallest name.
        items: (query.ids ?? []).map((id) =>
          asset(id, id === ids[ids.length - 1] ? "Aardvark" : `n-${id}`, 1, 1),
        ),
        next_cursor: null,
      }) as never,
  );
  const out = await queryFavoriteAssets("w1", { sort: "name", cursor: "stale", limit: 5 }, ids);
  expect(q).toHaveBeenCalledTimes(2);
  expect(q.mock.calls[0][1].ids).toHaveLength(api.MAX_QUERY_IDS);
  expect(q.mock.calls[1][1].ids).toEqual([ids[ids.length - 1]]);
  expect(q.mock.calls.every((c) => c[1].cursor === undefined)).toBe(true);
  expect(out).toHaveLength(ids.length);
  expect(out[0].original_name).toBe("Aardvark");
});

test("size and created sorts order numerically with the id as tiebreak", async () => {
  vi.spyOn(api, "queryAssets").mockResolvedValue({
    items: [asset("b", "x", 10, 1), asset("a", "y", 10, 2), asset("c", "z", 9, 3)],
    next_cursor: null,
  } as never);
  expect((await queryFavoriteAssets("w1", { sort: "size" }, ["a"])).map((a) => a.id)).toEqual(["c", "a", "b"]);
  expect((await queryFavoriteAssets("w1", { sort: "created" }, ["a"])).map((a) => a.id)).toEqual(["b", "a", "c"]);
});
