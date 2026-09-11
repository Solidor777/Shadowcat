// Node<->Rust end-to-end: the trigger-maintained `assets_fts` index over the
// real server, exercised through the real `GET /api/worlds/{world}/assets?q=`
// route -- an uploaded asset is findable by a word of its name and, once
// tagged, by that tag; an unrelated query yields no match.
import { afterAll, beforeAll, expect, test } from "vitest";
import type { Asset } from "@shadowcat/types";
import { startTestServer, login, type TestServer } from "./server-process";

let server: TestServer;
beforeAll(async () => {
  server = await startTestServer();
});
afterAll(() => server?.stop());

/** A 1x1 transparent PNG's raw bytes -- small, real image bytes so the
 * server's asset-ingest content-sniffing accepts it. */
const PNG_1X1 = Buffer.from(
  "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII=",
  "base64",
);

/** Uploads `PNG_1X1` to `world` as multipart under `filename`, authenticated
 * with `cookie`.
 * @param baseUrl The test server's base URL.
 * @param world The world id to upload into.
 * @param cookie The uploader's session cookie (GM-only endpoint).
 * @param filename The uploaded file's name (becomes `original_name`).
 * @returns The created asset record.
 */
async function uploadPng(baseUrl: string, world: string, cookie: string, filename: string): Promise<Asset> {
  const form = new FormData();
  form.append("file", new Blob([PNG_1X1], { type: "image/png" }), filename);
  const res = await fetch(`${baseUrl}/api/worlds/${world}/assets`, {
    method: "POST",
    headers: { cookie },
    body: form,
  });
  if (!res.ok) throw new Error(`upload failed: ${res.status}`);
  return (await res.json()) as Asset;
}

/** Tags `assetId` with `tag` via `PATCH /api/assets/{id}`.
 * @param baseUrl The test server's base URL.
 * @param assetId The asset to tag.
 * @param cookie The GM's session cookie.
 * @param tag The explicit tag to set.
 */
async function tagAsset(baseUrl: string, assetId: string, cookie: string, tag: string): Promise<void> {
  const res = await fetch(`${baseUrl}/api/assets/${assetId}`, {
    method: "PATCH",
    headers: { cookie, "content-type": "application/json" },
    body: JSON.stringify({ tags: [tag] }),
  });
  if (!res.ok) throw new Error(`tag failed: ${res.status}`);
}

/** Queries `?q=<query>` and returns the page's item ids.
 * @param baseUrl The test server's base URL.
 * @param world The world id.
 * @param cookie The caller's session cookie.
 * @param query The full-text query string.
 * @returns The matched asset ids, in page order.
 */
async function queryIds(baseUrl: string, world: string, cookie: string, query: string): Promise<string[]> {
  const res = await fetch(`${baseUrl}/api/worlds/${world}/assets?q=${encodeURIComponent(query)}&limit=10`, {
    headers: { cookie },
  });
  if (!res.ok) throw new Error(`query failed: ${res.status}`);
  const page = (await res.json()) as { items: Asset[] };
  return page.items.map((a) => a.id);
}

test("q finds an uploaded asset by name and by tag, over the real server", async () => {
  const gmCookie = await login(server.baseUrl, "gm", "pw");
  const world = server.fixture.world;

  const asset = await uploadPng(server.baseUrl, world, gmCookie, "wyvern-nest.png");
  await tagAsset(server.baseUrl, asset.id, gmCookie, "roost-marker");

  expect(await queryIds(server.baseUrl, world, gmCookie, "wyvern")).toEqual([asset.id]);
  expect(await queryIds(server.baseUrl, world, gmCookie, "roost-marker")).toEqual([asset.id]);
  expect(await queryIds(server.baseUrl, world, gmCookie, "zzzzznomatch")).toEqual([]);
});
