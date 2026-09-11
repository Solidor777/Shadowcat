// Node<->Rust end-to-end: the real client searches the real server and must
// never receive a document it cannot read. The fixture seeds a player-owned
// "Player Dragon" (readable) and a GM-only "Secret Dragon" (default None); both
// match "dragon", but a player's results must contain only the readable one.
import { afterAll, beforeAll, expect, test } from "vitest";
import WebSocket from "ws";
import { WsClient } from "../ws-client";
import type { Transport, TransportHandlers } from "../transport";
import type { ClientMsg } from "../wire";
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
    world: "w1",
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

/** An intent creating one `actor` document with `name` as its display name. */
function createActorIntent(world: string, id: string, name: string): ClientMsg {
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
          name,
          source: null,
          owner: null,
          permissions: {
            default: "observer",
            users: {},
            property_overrides: {},
            capabilities: { by_role: {}, by_user: {} },
            gm_role: null,
          },
          embedded: {},
          parent_id: null,
          engine: {
            displayName: name,
            visual: { kind: "image", asset: "a.png" },
            size: { w: 1, h: 1 },
            shape: "square",
            faction: null,
            conditions: [],
            prototype: true,
          },
          system: {},
          created_at: 0,
          updated_at: 0,
        },
      },
    ],
  };
}

/** An intent creating one `note` document; `source` is plain text, and the
 * server derives `body` from it (the client-sent `body: []` is overwritten
 * server-side by `normalize_engine`'s "note" arm — never trusted as-is). */
function createNoteIntent(world: string, id: string, source: string): ClientMsg {
  return {
    type: "intent",
    intent_id: id,
    ops: [
      {
        op: "create",
        doc: {
          id,
          scope: { kind: "world", world_id: world },
          doc_type: "note",
          schema_version: 1,
          name: null,
          source: null,
          owner: null,
          permissions: {
            default: "observer",
            users: {},
            property_overrides: {},
            capabilities: { by_role: {}, by_user: {} },
            gm_role: null,
          },
          embedded: {},
          parent_id: null,
          engine: { source, body: [], sort: 0 },
          system: {},
          created_at: 0,
          updated_at: 0,
        },
      },
    ],
  };
}

test("doc_types narrows a search to the listed types", async () => {
  const gmCookie = await login(server.baseUrl, "gm", "pw");
  const { world } = server.fixture;
  const gm = new WsClient({
    world: "w1",
    connect: nodeConnect(server.wsUrl, world, gmCookie),
    handlers: { onCommand: () => {} },
  });
  await gm.start();
  await sleep(400);

  const word = "wyverncrest";
  gm.send(createActorIntent(world, "cccccccc-cccc-cccc-cccc-cccccccccccc", `Actor ${word}`));
  gm.send(createNoteIntent(world, "dddddddd-dddd-dddd-dddd-dddddddddddd", `A note about ${word}`));
  await sleep(400); // settle both creates

  const notesOnly = await gm.search(word, { limit: 20, docTypes: ["note"] });
  expect(notesOnly.hits.length).toBe(1);
  expect(notesOnly.hits[0].document.doc_type).toBe("note");

  const everyType = await gm.search(word, { limit: 20, docTypes: [] });
  expect(everyType.hits.length).toBe(2);

  gm.stop();
});
