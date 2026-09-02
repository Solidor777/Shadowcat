// Node<->Rust end-to-end: a `[[asset:<uuid>|alt]]` span in a chat message
// resolves server-side to an `image` segment carrying that asset_id/alt,
// visible to every recipient; a span referencing a nonexistent asset is
// refused with `RollError::UnknownAsset`'s player-presentable text, surfaced
// (per `build_roll_error_notice`'s existing pattern for every other roll/scan
// failure) as a whispered `MessageKind::System` notice to the sender only —
// never as a hard `ChatError` rejection of the send itself.
import { afterAll, beforeAll, expect, test } from "vitest";
import WebSocket from "ws";
import { WsClient } from "../ws-client";
import type { Transport, TransportHandlers } from "../transport";
import type { ClientMsg, WireCommand } from "../wire";
import type { Asset } from "@shadowcat/types";
import { startTestServer, login, type TestServer } from "./server-process";

let server: TestServer;
beforeAll(async () => {
  server = await startTestServer();
});
afterAll(() => server?.stop());

function nodeConnect(wsUrl: string, world: string, cookie: string) {
  return (handlers: TransportHandlers): Promise<Transport> =>
    new Promise((resolve, reject) => {
      const sock = new WebSocket(`${wsUrl}?world=${world}`, { headers: { cookie } });
      sock.on("open", () =>
        resolve({ send: (d: string) => sock.send(d), close: () => sock.close() }),
      );
      sock.on("message", (d) => handlers.onMessage(d.toString()));
      sock.on("close", () => handlers.onClose());
      sock.on("error", reject);
    });
}

const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

/** A 1x1 transparent PNG's raw bytes — small, real image bytes so the
 * server's asset-ingest content-sniffing accepts it. */
const PNG_1X1 = Buffer.from(
  "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII=",
  "base64",
);

/** Fetches the world's already-seeded `chat-settings` singleton
 * (`world_seed::missing_config_ops`, run at world-creation time, pre-seeds
 * one alongside `channel_registry`/etc. for every world) via
 * `GET /api/worlds/{world}/documents?type=chat-settings`, then writes its
 * `images` field to `true` over the wire via the same JSON-pointer `Update`
 * intent shape `GameSettingsPanel` itself sends -- using the doc's REAL
 * pre-image as `old`, never an assumed one.
 * @param baseUrl The test server's base URL.
 * @param world The world id.
 * @param cookie The GM's session cookie.
 * @param ws The GM's already-connected client to send the enabling intent on.
 */
async function enableImages(baseUrl: string, world: string, cookie: string, ws: WsClient): Promise<void> {
  const res = await fetch(`${baseUrl}/api/worlds/${world}/documents?type=chat-settings`, {
    headers: { cookie },
  });
  if (!res.ok) throw new Error(`chat-settings query failed: ${res.status}`);
  const docs = (await res.json()) as { id: string; engine: { images: boolean | null } }[];
  if (docs.length === 0) throw new Error("chat-settings singleton not found");
  const doc = docs[0];
  const intent: ClientMsg = {
    type: "intent",
    intent_id: "22222222-2222-2222-2222-222222222222",
    ops: [{ op: "update", doc_id: doc.id, changes: [{ path: "/engine/images", old: doc.engine.images, new: true }] }],
  };
  ws.send(intent);
}

/** Uploads `PNG_1X1` to `world` as multipart, authenticated with `cookie` --
 * `uploadAsset` assumes a browser origin (relative `fetch`), so this posts
 * the `FormData` directly against `baseUrl` instead.
 * @param baseUrl The test server's base URL.
 * @param world The world id to upload into.
 * @param cookie The uploader's session cookie (GM-only endpoint).
 * @returns The created asset record.
 */
async function uploadPng(baseUrl: string, world: string, cookie: string): Promise<Asset> {
  const form = new FormData();
  form.append("file", new Blob([PNG_1X1], { type: "image/png" }), "pixel.png");
  const res = await fetch(`${baseUrl}/api/worlds/${world}/assets`, {
    method: "POST",
    headers: { cookie },
    body: form,
  });
  if (!res.ok) throw new Error(`upload failed: ${res.status}`);
  return (await res.json()) as Asset;
}

test("[[asset:<id>|alt]] resolves to an image segment every recipient receives", async () => {
  const gmCookie = await login(server.baseUrl, "gm", "pw");
  const plCookie = await login(server.baseUrl, "pl", "pw");
  const world = server.fixture.world;

  const gm = new WsClient({ world, connect: nodeConnect(server.wsUrl, world, gmCookie), handlers: { onCommand: () => {} } });
  await gm.start();
  await sleep(300);

  await enableImages(server.baseUrl, world, gmCookie, gm);
  await sleep(300);

  const asset = await uploadPng(server.baseUrl, world, gmCookie);

  let playerSawImage: { asset_id: string; alt: string } | null = null;
  const playerWatch = new WsClient({
    world,
    connect: nodeConnect(server.wsUrl, world, plCookie),
    handlers: {
      onCommand: (cmd: WireCommand) => {
        for (const op of cmd.ops) {
          if (op.op === "create" && op.doc.doc_type === "message") {
            const content = (op.doc.engine as { content: { kind: string; asset_id?: string; alt?: string }[] }).content;
            const img = content.find((s) => s.kind === "image");
            if (img?.asset_id) playerSawImage = { asset_id: img.asset_id, alt: img.alt ?? "" };
          }
        }
      },
    },
  });
  await playerWatch.start();
  await sleep(300);

  await gm.sendChatMessage({ channel: "general", content: `[[asset:${asset.id}|a map]]` });
  await sleep(500);

  expect(playerSawImage).not.toBeNull();
  expect((playerSawImage as unknown as { asset_id: string; alt: string }).asset_id).toBe(asset.id);
  expect((playerSawImage as unknown as { asset_id: string; alt: string }).alt).toBe("a map");

  gm.stop();
  playerWatch.stop();
});

test("[[asset:<nonexistent-id>]] is refused with UnknownAsset's text, as a whisper-to-sender System notice, never a ChatError", async () => {
  const gmCookie = await login(server.baseUrl, "gm", "pw");
  const world = server.fixture.world;

  let sawSystemNotice: string | null = null;
  const gm = new WsClient({
    world,
    connect: nodeConnect(server.wsUrl, world, gmCookie),
    handlers: {
      onCommand: (cmd: WireCommand) => {
        for (const op of cmd.ops) {
          if (op.op === "create" && op.doc.doc_type === "message") {
            const sys = op.doc.engine as { kind: string; content: { kind: string; text?: string }[] };
            if (sys.kind === "system") {
              const text = sys.content.find((s) => s.kind === "text")?.text;
              if (text) sawSystemNotice = text;
            }
          }
        }
      },
    },
  });
  await gm.start();
  await sleep(300);

  await enableImages(server.baseUrl, world, gmCookie, gm);
  await sleep(300);

  // `sendChatMessage` resolves success-assumed after its full silence window
  // (~15s, `CHAT_ERROR_WINDOW_MS`) when no `chat_error` arrives -- matching
  // this suite's existing `recalc-roll.e2e.test.ts` precedent of awaiting the
  // send directly rather than racing a shorter timeout.
  let sawChatError = false;
  const nonexistentId = "44444444-4444-4444-4444-444444444444";
  try {
    await gm.sendChatMessage({ channel: "general", content: `[[asset:${nonexistentId}]]` });
  } catch {
    sawChatError = true;
  }

  expect(sawChatError).toBe(false);
  expect(sawSystemNotice).not.toBeNull();
  expect(sawSystemNotice).toBe("that image could not be found");

  gm.stop();
});
