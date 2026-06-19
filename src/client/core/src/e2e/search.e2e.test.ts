// Node<->Rust end-to-end: the real client searches the real server and must
// never receive a document it cannot read. The fixture seeds a player-owned
// "Player Dragon" (readable) and a GM-only "Secret Dragon" (default None); both
// match "dragon", but a player's results must contain only the readable one.
import { afterAll, beforeAll, expect, test } from "vitest";
import WebSocket from "ws";
import { WsClient } from "../ws-client";
import type { Transport, TransportHandlers } from "../transport";
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

test("search excludes documents the player cannot read", async () => {
  const cookie = await login(server.baseUrl, "pl", "pw");
  const { world } = server.fixture;
  const client = new WsClient({
    connect: nodeConnect(server.wsUrl, world, cookie),
    handlers: { onCommand: () => {} },
  });
  await client.start();
  await sleep(400); // settle Welcome + initial resync

  const page = await client.search("dragon", { limit: 20 });
  // The player-owned "Player Dragon" matches; the GM-only "Secret Dragon" must not.
  expect(page.hits.length).toBeGreaterThanOrEqual(1);
  const blob = JSON.stringify(page.hits);
  expect(blob.includes("Secret Dragon")).toBe(false);
  expect(blob.includes("Player Dragon")).toBe(true);

  client.stop();
});
