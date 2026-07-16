// Node<->Rust end-to-end: the envelope `/name` privacy override. A `/name` =
// owner_or_gm override redacts to `null` for a non-owner reader — never
// sent-then-hidden — while the owner and GM keep it, and the visibility-
// partitioned FTS index must not let a non-owner find the document by
// searching the hidden name.
import { afterAll, beforeAll, expect, test } from "vitest";
import WebSocket from "ws";
import { WsClient } from "../ws-client";
import type { Transport, TransportHandlers } from "../transport";
import type { ClientMsg, WireCommand } from "../wire";
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

test("/name owner_or_gm override: owner+GM see the name, a non-owner player gets null, and FTS misses the hidden name for that player", async () => {
  const gmCookie = await login(server.baseUrl, "gm", "pw");
  const plCookie = await login(server.baseUrl, "pl", "pw");
  const { world, gm: gmId } = server.fixture;

  // The player joins first so it observes the live Create.
  let observedName: string | null | undefined;
  const playerWatch = new WsClient({
    connect: nodeConnect(server.wsUrl, world, plCookie),
    handlers: {
      onCommand: (cmd: WireCommand) => {
        for (const op of cmd.ops) {
          if (op.op === "create" && op.doc.doc_type === "actor" && op.doc.name === null) {
            observedName = op.doc.name;
          }
        }
      },
    },
  });
  await playerWatch.start();
  await sleep(300);

  const gm = new WsClient({
    connect: nodeConnect(server.wsUrl, world, gmCookie),
    handlers: { onCommand: () => {} },
  });
  await gm.start();
  await sleep(300);

  const docId = "55555555-5555-5555-5555-555555555555";
  const createIntent: ClientMsg = {
    type: "intent",
    intent_id: "66666666-6666-6666-6666-666666666666",
    ops: [
      {
        op: "create",
        doc: {
          id: docId,
          scope: { kind: "world", world_id: world },
          doc_type: "actor",
          schema_version: 1,
          name: "Nightingale",
          source: null,
          owner: gmId, // GM-owned; the fixture's "pl" player is the non-owner reader.
          permissions: {
            default: "observer", // the player may read the document at all.
            users: {},
            property_overrides: { "/name": "owner_or_gm" },
            capabilities: { by_role: {}, by_user: {} },
            gm_role: null,
          },
          embedded: {},
          parent_id: null,
          // `displayName` deliberately differs from the envelope `name` so the
          // FTS assertions below isolate the `/name` redaction specifically —
          // `engine.displayName` carries no override and stays searchable.
          engine: {
            displayName: "Unrelated Actor",
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
  gm.send(createIntent);
  await sleep(500);

  expect(observedName).toBe(null);
  playerWatch.stop();

  // The GM (owner) still sees the name via search.
  const gmHit = await gm.search("Nightingale", { limit: 20 });
  expect(JSON.stringify(gmHit.hits).includes("Nightingale")).toBe(true);
  gm.stop();

  // The non-owner player cannot find it by the hidden name (the public FTS
  // partition never indexed the redacted /name).
  const plSearch = new WsClient({
    connect: nodeConnect(server.wsUrl, world, plCookie),
    handlers: { onCommand: () => {} },
  });
  await plSearch.start();
  await sleep(300);
  const plHit = await plSearch.search("Nightingale", { limit: 20 });
  expect(JSON.stringify(plHit.hits).includes("Nightingale")).toBe(false);
  plSearch.stop();
});
