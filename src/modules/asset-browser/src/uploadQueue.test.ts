import { describe, it, expect, vi, beforeEach } from "vitest";
import * as api from "@shadowcat/core";
import { ChunkedUploadError } from "@shadowcat/core";
import { UploadQueue } from "./uploadQueueModel.svelte";

beforeEach(() => vi.restoreAllMocks());

function file(name: string): File {
  return new File([new Uint8Array([1, 2, 3])], name);
}

/** Waits until `cond` holds (microtask-driven queues settle fast). */
async function until(cond: () => boolean): Promise<void> {
  for (let i = 0; i < 200 && !cond(); i++) await new Promise((r) => setTimeout(r, 5));
  expect(cond()).toBe(true);
}

describe("UploadQueue", () => {
  it("runs sequentially and mirrors per-chunk progress", async () => {
    const seen: string[] = [];
    vi.spyOn(api, "startChunkedUpload").mockImplementation(async (_w, f, opts) => {
      seen.push(f.name);
      opts?.onProgress?.(1, 3);
      opts?.onProgress?.(3, 3);
      return { id: "a-" + f.name } as never;
    });
    const done = vi.fn();
    const q = new UploadQueue("w1", done);
    q.enqueue([file("one.png"), file("two.png")], "folder-1");
    await until(() => q.entries.every((e) => e.status === "done"));
    expect(seen).toEqual(["one.png", "two.png"]);
    expect(q.entries[0].sent).toBe(3);
    expect(q.entries[0].total).toBe(3);
    expect(done).toHaveBeenCalledTimes(2);
  });

  it("passes the target folder and marks a failure retryable", async () => {
    const start = vi
      .spyOn(api, "startChunkedUpload")
      .mockRejectedValueOnce(new Error("network down"))
      .mockResolvedValueOnce({ id: "a1" } as never);
    const q = new UploadQueue("w1", vi.fn());
    q.enqueue([file("x.png")], "folder-9");
    await until(() => q.entries[0].status === "error");
    expect(q.entries[0].error).toContain("network down");
    expect(start).toHaveBeenCalledWith("w1", expect.anything(), expect.objectContaining({ folderId: "folder-9" }));

    q.retry(0);
    await until(() => q.entries[0].status === "done");
  });

  it("surfaces a partial-placement failure as done-with-warning, never re-uploading", async () => {
    vi.spyOn(api, "startChunkedUpload").mockRejectedValue(
      new ChunkedUploadError("placement failed after upload", 503, { id: "a1" } as never),
    );
    const done = vi.fn();
    const q = new UploadQueue("w1", done);
    q.enqueue([file("x.png")], null);
    await until(() => q.entries[0].status === "done");
    expect(q.entries[0].partial).toBeTruthy();
    expect(q.entries[0].error).toContain("placement failed");
    // The asset exists server-side; the listing refresh must fire.
    expect(done).toHaveBeenCalled();
  });

  it("cancelling a queued entry removes it; cancelling an active one aborts", async () => {
    let abortSignal: AbortSignal | undefined;
    vi.spyOn(api, "startChunkedUpload").mockImplementation(
      (_w, _f, opts) =>
        new Promise((_res, rej) => {
          abortSignal = opts?.signal;
          opts?.signal?.addEventListener("abort", () => rej(new Error("aborted")));
        }),
    );
    const q = new UploadQueue("w1", vi.fn());
    q.enqueue([file("active.png"), file("waiting.png")], null);
    await until(() => q.entries[0].status === "uploading");
    // Remove the queued one outright.
    q.cancel(1);
    expect(q.entries).toHaveLength(1);
    // Abort the active one.
    q.cancel(0);
    await until(() => q.entries[0].status === "error");
    expect(abortSignal?.aborted).toBe(true);
  });
});
