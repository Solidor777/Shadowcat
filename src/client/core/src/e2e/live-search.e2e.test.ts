// Node<->Rust end-to-end: a live search subscription updates when a readable
// matching doc is created, and never delivers a GM-only doc to a player.
import { afterAll, beforeAll, expect, test } from "vitest";
import WebSocket from "ws";
import { WsClient } from "../ws-client";
import type { Transport, TransportHandlers } from "../transport";
import type { ClientMsg, WireSearchHit } from "../wire";
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

function createIntent(world: string, id: string, name: string, role: "observer" | "none"): ClientMsg {
  return {
    type: "intent",
    intent_id: id,
    ops: [
      {
        op: "create",
        doc: {
          id,
          scope: { kind: "world", world_id: world },
          doc_type: "actor",
          schema_version: 1,
          source: null,
          owner: null,
          permissions: {
            default: role,
            users: {},
            property_overrides: {},
            capabilities: { by_role: {}, by_user: {} },
          },
          embedded: {},
          system: { name },
          created_at: 0,
          updated_at: 0,
        },
      },
    ],
  };
}

test("a player's live subscription updates on a readable create and never leaks GM-only docs", async () => {
  const plCookie = await login(server.baseUrl, "pl", "pw");
  const gmCookie = await login(server.baseUrl, "gm", "pw");
  const { world } = server.fixture;

  const player = new WsClient({
    connect: nodeConnect(server.wsUrl, world, plCookie),
    handlers: { onCommand: () => {} },
  });
  await player.start();
  await sleep(300);

  const updates: WireSearchHit[][] = [];
  await player.subscribeSearch("griffon", { limit: 20 }, (hits) => updates.push(hits));
  expect(updates.at(-1)).toEqual([]); // initial empty

  // GM connects and creates one readable + one GM-only doc, both matching "griffon".
  const gm = new WsClient({
    connect: nodeConnect(server.wsUrl, world, gmCookie),
    handlers: { onCommand: () => {} },
  });
  await gm.start();
  await sleep(300);
  gm.send(createIntent(world, "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa", "Readable Griffon", "observer"));
  gm.send(createIntent(world, "bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb", "Secret Griffon", "none"));

  // Within the debounce window the player gets an update with ONLY the readable one.
  await sleep(900);
  const last = updates.at(-1)!;
  const blob = JSON.stringify(last);
  expect(blob.includes("Readable Griffon")).toBe(true);
  expect(blob.includes("Secret Griffon")).toBe(false);

  gm.stop();
  player.stop();
});
