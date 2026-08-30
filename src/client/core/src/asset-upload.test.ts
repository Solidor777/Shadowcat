import { test, expect, vi, afterEach } from "vitest";
import { startChunkedUpload, CHUNK_THRESHOLD_BYTES, ChunkedUploadError } from "./asset-upload";
import * as rest from "./asset-rest";

afterEach(() => vi.restoreAllMocks());

const ASSET = { id: "a1", world_id: "w1", version: 1 };

/** A recording `fetch` whose responses are scripted per (method, url prefix). */
function scriptedFetch(
  script: (method: string, url: string, call: number) => Response | Error,
) {
  const calls: { method: string; url: string; body: unknown }[] = [];
  const impl = vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
    const url = String(input);
    const method = init?.method ?? "GET";
    calls.push({ method, url, body: init?.body });
    const out = script(method, url, calls.length);
    if (out instanceof Error) throw out;
    return out;
  });
  return { impl: impl as unknown as typeof fetch, calls };
}

function json(status: number, body: unknown): Response {
  return new Response(JSON.stringify(body), { status });
}

function bigFile(bytes: number): File {
  return new File([new Uint8Array(bytes)], "big.bin", { type: "application/octet-stream" });
}

test("a small file goes single-shot and applies placement via patchAsset", async () => {
  const upload = vi.spyOn(rest, "uploadAsset").mockResolvedValue(ASSET as never);
  const patch = vi.spyOn(rest, "patchAsset").mockResolvedValue({ ...ASSET, tags: ["map"] } as never);
  const progress: [number, number][] = [];
  const file = new File([new Uint8Array([1, 2, 3])], "x.png", { type: "image/png" });
  const out = await startChunkedUpload("w1", file, {
    tags: ["map"],
    onProgress: (s, t) => progress.push([s, t]),
  });
  expect(upload).toHaveBeenCalledWith("w1", file);
  expect(patch).toHaveBeenCalledWith("a1", { folder_id: null, tags: ["map"] });
  expect(out.tags).toEqual(["map"]);
  expect(progress).toEqual([[3, 3]]);
});

test("a small file with no placement skips the patch", async () => {
  vi.spyOn(rest, "uploadAsset").mockResolvedValue(ASSET as never);
  const patch = vi.spyOn(rest, "patchAsset");
  await startChunkedUpload("w1", new File([new Uint8Array([1])], "x.png"));
  expect(patch).not.toHaveBeenCalled();
});

test("a large file opens a session, PUTs chunks at 0/8/16, then completes", async () => {
  const { impl, calls } = scriptedFetch((method, url) => {
    if (method === "POST" && url.endsWith("/assets/uploads"))
      return json(201, { upload_id: "s1", chunk_size: 8 });
    if (method === "PUT") return new Response(null, { status: 204 });
    if (method === "POST" && url.endsWith("/complete")) return json(200, ASSET);
    return new Response(null, { status: 500 });
  });
  const progress: number[] = [];
  const file = bigFile(CHUNK_THRESHOLD_BYTES + 1);
  // The server dictates the chunk size; a tiny one keeps the test cheap.
  const out = await startChunkedUpload("w1", file, {
    fetchImpl: impl,
    folderId: "f1",
    tags: ["big"],
    onProgress: (s) => progress.push(s),
  });
  expect(out).toEqual(ASSET);
  const create = calls[0];
  expect(create.url).toBe("/api/worlds/w1/assets/uploads");
  const createBody = JSON.parse(create.body as string);
  expect(createBody).toMatchObject({
    name: "big.bin",
    content_type: "application/octet-stream",
    byte_size: file.size,
    folder_id: "f1",
    tags: ["big"],
  });
  const puts = calls.filter((c) => c.method === "PUT").map((c) => c.url);
  expect(puts.slice(0, 3)).toEqual([
    "/api/assets/uploads/s1/0",
    "/api/assets/uploads/s1/8",
    "/api/assets/uploads/s1/16",
  ]);
  expect(puts).toHaveLength(Math.ceil(file.size / 8));
  expect(calls.at(-1)?.url).toBe("/api/assets/uploads/s1/complete");
  expect(progress.at(-1)).toBe(file.size);
  expect(progress).toEqual([...progress].sort((a, b) => a - b));
});

test("a chunk that fails on the wire is retried at the same offset", async () => {
  let putAttempts = 0;
  const { impl, calls } = scriptedFetch((method, url) => {
    if (method === "POST" && url.endsWith("/assets/uploads"))
      return json(201, { upload_id: "s1", chunk_size: CHUNK_THRESHOLD_BYTES });
    if (method === "PUT") {
      putAttempts++;
      if (putAttempts === 2) return new Error("connection reset");
      if (putAttempts === 3) return new Response(null, { status: 503 });
      return new Response(null, { status: 204 });
    }
    if (method === "POST" && url.endsWith("/complete")) return json(200, ASSET);
    return new Response(null, { status: 500 });
  });
  const file = bigFile(CHUNK_THRESHOLD_BYTES + 1);
  await startChunkedUpload("w1", file, { fetchImpl: impl });
  const puts = calls.filter((c) => c.method === "PUT").map((c) => c.url);
  // Chunk 0 accepted; chunk at 8 MiB failed twice (network, 503) then succeeded.
  expect(puts).toEqual([
    "/api/assets/uploads/s1/0",
    `/api/assets/uploads/s1/${CHUNK_THRESHOLD_BYTES}`,
    `/api/assets/uploads/s1/${CHUNK_THRESHOLD_BYTES}`,
    `/api/assets/uploads/s1/${CHUNK_THRESHOLD_BYTES}`,
  ]);
});

test("a 409 (offset desync) is not retried: the session is aborted and the upload rejects", async () => {
  const { impl, calls } = scriptedFetch((method, url) => {
    if (method === "POST" && url.endsWith("/assets/uploads"))
      return json(201, { upload_id: "s1", chunk_size: CHUNK_THRESHOLD_BYTES });
    if (method === "PUT") return json(409, { error: "offset 0 is not the next byte (8)" });
    if (method === "DELETE") return new Response(null, { status: 204 });
    return new Response(null, { status: 500 });
  });
  const file = bigFile(CHUNK_THRESHOLD_BYTES + 1);
  await expect(startChunkedUpload("w1", file, { fetchImpl: impl, retries: 3 })).rejects.toThrow(
    ChunkedUploadError,
  );
  expect(calls.filter((c) => c.method === "PUT")).toHaveLength(1);
  expect(calls.at(-1)).toMatchObject({ method: "DELETE", url: "/api/assets/uploads/s1" });
});

test("an abort signal aborts the server session and rejects", async () => {
  const controller = new AbortController();
  const { impl, calls } = scriptedFetch((method, url) => {
    if (method === "POST" && url.endsWith("/assets/uploads"))
      return json(201, { upload_id: "s1", chunk_size: 8 });
    if (method === "PUT") {
      // Abort mid-stream, after the first chunk lands.
      controller.abort(new Error("user cancelled"));
      return new Response(null, { status: 204 });
    }
    if (method === "DELETE") return new Response(null, { status: 204 });
    return new Response(null, { status: 500 });
  });
  const file = bigFile(CHUNK_THRESHOLD_BYTES + 1);
  await expect(
    startChunkedUpload("w1", file, { fetchImpl: impl, signal: controller.signal }),
  ).rejects.toThrow("user cancelled");
  expect(calls.filter((c) => c.method === "PUT")).toHaveLength(1);
  expect(calls.at(-1)).toMatchObject({ method: "DELETE", url: "/api/assets/uploads/s1" });
});

test("a refused session create surfaces the server's error and status", async () => {
  const { impl } = scriptedFetch(() => json(413, { error: "file exceeds 1024 bytes" }));
  const file = bigFile(CHUNK_THRESHOLD_BYTES + 1);
  await expect(startChunkedUpload("w1", file, { fetchImpl: impl })).rejects.toMatchObject({
    name: "ChunkedUploadError",
    status: 413,
    message: expect.stringContaining("file exceeds 1024 bytes"),
  });
});
