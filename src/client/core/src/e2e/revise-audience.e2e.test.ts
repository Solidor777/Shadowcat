// Node<->Rust end-to-end: a GM-only message is invisible to a player until the GM reveals
// it, at which point the player's already-open session receives the whole message with no
// reload; a later conceal removes it from that session again.
import { afterAll, beforeAll, expect, test } from "vitest";
import WebSocket from "ws";
import { WsClient } from "../ws-client";
import type { Transport, TransportHandlers } from "../transport";
import type { WireCommand } from "../wire";
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

test("GM reveal delivers a hidden message to an open player session and conceal removes it, with no reload", async () => {
  const gmCookie = await login(server.baseUrl, "gm", "pw");
  const plCookie = await login(server.baseUrl, "pl", "pw");
  const world = server.fixture.world;

  let messageId = "";
  const gmWatch = new WsClient({
    world,
    connect: nodeConnect(server.wsUrl, world, gmCookie),
    handlers: {
      onCommand: (cmd: WireCommand) => {
        for (const op of cmd.ops) {
          if (op.op === "create" && op.doc.doc_type === "message") messageId = op.doc.id;
        }
      },
    },
  });
  await gmWatch.start();
  await sleep(300);

  const playerEvents: string[] = [];
  const player = new WsClient({
    world,
    connect: nodeConnect(server.wsUrl, world, plCookie),
    handlers: {
      onCommand: (cmd: WireCommand) => {
        for (const op of cmd.ops) {
          if (op.op === "create" && op.doc.doc_type === "message" && op.doc.id === messageId) playerEvents.push("create");
          if (op.op === "delete" && op.doc.id === messageId) playerEvents.push("delete");
        }
      },
    },
  });
  await player.start();
  await sleep(300);

  const gm = new WsClient({ world, connect: nodeConnect(server.wsUrl, world, gmCookie), handlers: { onCommand: () => {} } });
  await gm.start();
  await sleep(300);

  await gm.sendChatMessage({ channel: "general", content: "for the GM only", audience: { kind: "gm_only" } });
  await sleep(500);
  expect(messageId).not.toBe("");
  expect(playerEvents).toEqual([]);

  await gm.reviseAudience(messageId, { kind: "public" });
  await sleep(500);
  expect(playerEvents).toEqual(["create"]);

  await gm.reviseAudience(messageId, { kind: "gm_only" });
  await sleep(500);
  expect(playerEvents).toEqual(["create", "delete"]);

  gm.stop();
  gmWatch.stop();
  player.stop();
});
