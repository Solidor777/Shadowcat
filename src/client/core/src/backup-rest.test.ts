import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import {
  adminBackup,
  listBackups,
  deleteBackup,
  restoreBackup,
  pruneBackups,
  dismissRestoreFailed,
} from "./backup-rest";

/** A fetch response with a JSON body. */
function json(status: number, body: unknown): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { "content-type": "application/json" },
  });
}

/** A manifest as the server's JSON carries it. */
const MANIFEST = {
  shadowcat_version: "0.1.0",
  created_at_unix_ms: 1_789_000_000_000,
  source_db: "shadowcat.db",
  source_assets_dir: "assets",
  asset_file_count: 3,
  db_bytes: 4096,
  kind: "manual",
  note: null,
};

describe("backup-rest", () => {
  const fetchMock = vi.fn();
  beforeEach(() => {
    fetchMock.mockReset();
    vi.stubGlobal("fetch", fetchMock);
  });
  afterEach(() => vi.unstubAllGlobals());

  it("adminBackup POSTs the backup route and returns the manifest", async () => {
    fetchMock.mockResolvedValue(json(200, MANIFEST));
    expect(await adminBackup()).toEqual(MANIFEST);
    expect(fetchMock.mock.calls[0][0]).toBe("/api/admin/backup");
    expect(fetchMock.mock.calls[0][1]).toEqual({ method: "POST" });
  });

  it("adminBackup surfaces the server's error text, else the fallback", async () => {
    fetchMock.mockResolvedValueOnce(json(403, { error: "forbidden" }));
    await expect(adminBackup()).rejects.toThrow("forbidden");
    fetchMock.mockResolvedValueOnce(new Response("", { status: 500 }));
    await expect(adminBackup()).rejects.toThrow();
  });

  it("listBackups GETs the list and converts every u64 field to a number", async () => {
    // The fake body carries `bigint` values, the shape the generated types declare; the
    // converter must hand back numbers for both that and the plain numbers real JSON holds.
    const body = {
      entries: [
        {
          id: "backup-1",
          manifest: {
            ...MANIFEST,
            created_at_unix_ms: 1_789_000_000_000n,
            asset_file_count: 3n,
            db_bytes: 4096n,
          },
          total_bytes: 9000n,
        },
      ],
      policy: { keep_last: 3, keep_days: null },
      root: "backups",
      restore_failed: { id: "backup-0", requested_at_ms: 5n, error: "boom" },
    };
    fetchMock.mockResolvedValue({ ok: true, json: async () => body } as unknown as Response);
    const listing = await listBackups();
    expect(fetchMock.mock.calls[0][0]).toBe("/api/admin/backups");
    expect(listing.root).toBe("backups");
    expect(listing.policy).toEqual({ keep_last: 3, keep_days: null });
    expect(listing.entries[0].id).toBe("backup-1");
    expect(typeof listing.entries[0].total_bytes).toBe("number");
    expect(typeof listing.entries[0].manifest.db_bytes).toBe("number");
    expect(typeof listing.entries[0].manifest.created_at_unix_ms).toBe("number");
    expect(typeof listing.entries[0].manifest.asset_file_count).toBe("number");
    expect(listing.restore_failed).toEqual({ id: "backup-0", requested_at_ms: 5, error: "boom" });
  });

  it("listBackups keeps a null restore_failed null", async () => {
    fetchMock.mockResolvedValue(
      json(200, { entries: [], policy: { keep_last: null, keep_days: null }, root: "r", restore_failed: null }),
    );
    expect((await listBackups()).restore_failed).toBeNull();
  });

  it("listBackups rejects with the server's error text", async () => {
    fetchMock.mockResolvedValue(json(403, { error: "forbidden" }));
    await expect(listBackups()).rejects.toThrow("forbidden");
  });

  it("deleteBackup DELETEs the encoded id and rejects on failure", async () => {
    fetchMock.mockResolvedValueOnce(new Response(null, { status: 204 }));
    await deleteBackup("backup-1");
    expect(fetchMock.mock.calls[0][0]).toBe("/api/admin/backups/backup-1");
    expect(fetchMock.mock.calls[0][1]).toEqual({ method: "DELETE" });
    fetchMock.mockResolvedValueOnce(json(404, { error: "not found" }));
    await expect(deleteBackup("backup-2")).rejects.toThrow("not found");
  });

  it("dismissRestoreFailed DELETEs the marker route and rejects on failure", async () => {
    fetchMock.mockResolvedValueOnce(new Response(null, { status: 204 }));
    await dismissRestoreFailed();
    expect(fetchMock.mock.calls[0][0]).toBe("/api/admin/backups/restore-failed");
    expect(fetchMock.mock.calls[0][1]).toEqual({ method: "DELETE" });
    fetchMock.mockResolvedValueOnce(json(403, { error: "forbidden" }));
    await expect(dismissRestoreFailed()).rejects.toThrow("forbidden");
  });

  it("restoreBackup POSTs the restore route and returns the staged marker", async () => {
    fetchMock.mockResolvedValue(json(200, { staged: true, pre_restore_id: "backup-9" }));
    expect(await restoreBackup("backup-1")).toEqual({ staged: true, pre_restore_id: "backup-9" });
    expect(fetchMock.mock.calls[0][0]).toBe("/api/admin/backups/backup-1/restore");
    expect(fetchMock.mock.calls[0][1]).toEqual({ method: "POST" });
    fetchMock.mockResolvedValueOnce(json(404, { error: "not found" }));
    await expect(restoreBackup("nope")).rejects.toThrow("not found");
  });

  it("pruneBackups POSTs the prune route and returns the removed ids", async () => {
    fetchMock.mockResolvedValue(json(200, { removed: ["backup-1"] }));
    expect(await pruneBackups()).toEqual({ removed: ["backup-1"] });
    expect(fetchMock.mock.calls[0][0]).toBe("/api/admin/backups/prune");
    expect(fetchMock.mock.calls[0][1]).toEqual({ method: "POST" });
  });
});
