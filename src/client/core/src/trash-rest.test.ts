import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { listTrash, emptyTrash, deleteTrashEntry, TrashEntryGoneError } from "./trash-rest";

const ID = "1737490000123-3fa85f64-5717-4562-b3fc-2c963f66afa6";

/** A fetch response with a JSON body. */
function json(status: number, body: unknown): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { "content-type": "application/json" },
  });
}

describe("trash-rest", () => {
  const fetchMock = vi.fn();
  beforeEach(() => {
    fetchMock.mockReset();
    vi.stubGlobal("fetch", fetchMock);
  });
  afterEach(() => vi.unstubAllGlobals());

  it("listTrash GETs the first page without a query string", async () => {
    const entries = [
      { id: ID, original_path: "/data/a.png", kind: "file", trashed_at_ms: 1, size_bytes: 2 },
    ];
    fetchMock.mockResolvedValue(json(200, entries));
    expect(await listTrash()).toEqual(entries);
    expect(fetchMock.mock.calls[0][0]).toBe("/api/admin/trash");
  });

  it("listTrash sends limit and the before cursor", async () => {
    fetchMock.mockResolvedValue(json(200, []));
    await listTrash({ limit: 50, before: ID });
    expect(fetchMock.mock.calls[0][0]).toBe(`/api/admin/trash?limit=50&before=${ID}`);
  });

  it("listTrash surfaces the server's error text, else the status", async () => {
    fetchMock.mockResolvedValueOnce(json(403, { error: "forbidden" }));
    await expect(listTrash()).rejects.toThrow("forbidden");
    fetchMock.mockResolvedValueOnce(new Response("nope", { status: 500 }));
    await expect(listTrash()).rejects.toThrow("list trash failed: 500");
  });

  it("emptyTrash POSTs the empty route and returns the totals", async () => {
    fetchMock.mockResolvedValue(json(200, { entries: 3, bytes: 15 }));
    expect(await emptyTrash()).toEqual({ entries: 3, bytes: 15 });
    expect(fetchMock.mock.calls[0][0]).toBe("/api/admin/trash/empty");
    expect(fetchMock.mock.calls[0][1]).toMatchObject({ method: "POST" });
  });

  it("deleteTrashEntry DELETEs the encoded entry id", async () => {
    fetchMock.mockResolvedValue(new Response(null, { status: 204 }));
    await deleteTrashEntry(ID);
    expect(fetchMock.mock.calls[0][0]).toBe(`/api/admin/trash/${ID}`);
    expect(fetchMock.mock.calls[0][1]).toMatchObject({ method: "DELETE" });
  });

  it("deleteTrashEntry reports a 404 as TrashEntryGoneError and any other failure as Error", async () => {
    fetchMock.mockResolvedValueOnce(json(404, { error: "not found" }));
    await expect(deleteTrashEntry(ID)).rejects.toBeInstanceOf(TrashEntryGoneError);
    fetchMock.mockResolvedValueOnce(json(500, { error: "internal" }));
    const err = await deleteTrashEntry(ID).catch((e: unknown) => e);
    expect(err).toBeInstanceOf(Error);
    expect(err).not.toBeInstanceOf(TrashEntryGoneError);
  });
});
