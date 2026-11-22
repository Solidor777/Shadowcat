import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { adminBackup } from "./backup-rest";

/** A fetch response with a JSON body. */
function json(status: number, body: unknown): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { "content-type": "application/json" },
  });
}

describe("backup-rest", () => {
  const fetchMock = vi.fn();
  beforeEach(() => {
    fetchMock.mockReset();
    vi.stubGlobal("fetch", fetchMock);
  });
  afterEach(() => vi.unstubAllGlobals());

  it("adminBackup POSTs the backup route and returns the manifest", async () => {
    const manifest = {
      shadowcat_version: "0.1.0",
      created_at_unix_ms: 1,
      source_db: "shadowcat.db",
      source_assets_dir: "assets",
      asset_file_count: 3,
      db_bytes: 4096,
      kind: "manual",
      note: null,
    };
    fetchMock.mockResolvedValue(json(200, manifest));
    expect(await adminBackup()).toEqual(manifest);
    expect(fetchMock.mock.calls[0][0]).toBe("/api/admin/backup");
    expect(fetchMock.mock.calls[0][1]).toEqual({ method: "POST" });
  });

  it("adminBackup surfaces the server's error text, else the fallback", async () => {
    fetchMock.mockResolvedValueOnce(json(403, { error: "forbidden" }));
    await expect(adminBackup()).rejects.toThrow("forbidden");
    fetchMock.mockResolvedValueOnce(new Response("", { status: 500 }));
    await expect(adminBackup()).rejects.toThrow();
  });
});
