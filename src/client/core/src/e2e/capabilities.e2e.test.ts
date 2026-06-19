// Node<->Rust end-to-end: the real @shadowcat/core WS client driven against the
// real Rust test_server. Verifies the security-critical capability enforcement
// end to end — a player is rejected writing a GM-gated path, the extended
// Welcome reaches the client, and the projected grants do not leak other users.
import { afterAll, beforeAll, expect, test } from "vitest";
import WebSocket from "ws";
import { WsClient, type WireWelcome } from "../ws-client";
import type { Transport, TransportHandlers } from "../transport";
import type { ClientMsg } from "../wire";
import type { RejectReason } from "@shadowcat/types";
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
        resolve({
          send: (d: string) => sock.send(d),
          close: () => sock.close(),
        }),
      );
      sock.on("message", (d) => handlers.onMessage(d.toString()));
      sock.on("close", () => handlers.onClose());
      sock.on("error", reject);
    });
}

const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

test("player is rejected writing a GM-gated path; Welcome carries projected caps", async () => {
  const cookie = await login(server.baseUrl, "pl", "pw");
  const { world, doc, player } = server.fixture;

  let rejected: RejectReason | null = null;
  let welcome: WireWelcome | null = null;
  const client = new WsClient({
    connect: nodeConnect(server.wsUrl, world, cookie),
    handlers: {
      onCommand: () => {},
      onReject: (_id, reason) => {
        rejected = reason;
      },
      onWelcome: (w) => {
        welcome = w;
      },
    },
  });
  await client.start();

  // Wait for the Welcome (and the initial resync to settle).
  for (let i = 0; i < 50 && welcome === null; i++) await sleep(100);
  expect(welcome).not.toBeNull();
  const w = welcome as unknown as WireWelcome;
  expect(w.actor_role).toBe("player");
  expect(w.capability_requirements.some((r) => r.path_prefix === "/system/vision")).toBe(true);
  // Projection: the grant map must not leak other users' ids. by_user is keyed
  // by uuid; the only permissible key is the connecting player.
  for (const id of Object.keys(w.world_default_grants.by_user)) {
    expect(id).toBe(player);
  }

  // Attempt the GM-gated write directly over the wire.
  const intent: ClientMsg = {
    type: "intent",
    intent_id: "11111111-1111-1111-1111-111111111111",
    ops: [
      {
        op: "update",
        doc_id: doc,
        changes: [{ path: "/system/vision/range", old: 30, new: 60 }],
      },
    ],
  };
  client.send(intent);

  for (let i = 0; i < 50 && rejected === null; i++) await sleep(100);
  expect(rejected).toBe("forbidden");

  client.stop();
});
