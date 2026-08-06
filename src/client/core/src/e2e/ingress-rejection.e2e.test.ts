// Node<->Rust end-to-end: the real client's Create intents against the real
// server's strict `engine` ingress gate (M13-0 S1/S3). The gate runs ahead of
// the create-authorization check (Phase 1 of `SqliteRepository::apply_intent`), so any
// authenticated connection sees the same rejection; the GM login is used only
// to keep the fixture free of unrelated core:create floor considerations.
import { afterAll, beforeAll, expect, test } from "vitest";
import WebSocket from "ws";
import { WsClient } from "../ws-client";
import type { Transport, TransportHandlers } from "../transport";
import type { ClientMsg, WireCommand, WireDocument } from "../wire";
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
        resolve({ send: (d: string) => sock.send(d), close: () => sock.close() }),
      );
      sock.on("message", (d) => handlers.onMessage(d.toString()));
      sock.on("close", () => handlers.onClose());
      sock.on("error", reject);
    });
}

const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

/** Drive one Create intent to completion, resolving with the reject reason (or
 * null if the server instead accepted it, surfaced as a live `event`). */
async function tryCreate(
  world: string,
  wsUrl: string,
  cookie: string,
  doc: WireDocument,
): Promise<RejectReason | null> {
  let rejected: RejectReason | null = null;
  let accepted = false;
  const client = new WsClient({
    connect: nodeConnect(wsUrl, world, cookie),
    handlers: {
      // Resync replays every pre-existing fixture doc as an "event" too — only
      // a Create op carrying THIS doc's id proves our own intent was accepted,
      // not just that some unrelated resync catch-up event arrived first.
      onCommand: (cmd: WireCommand) => {
        for (const op of cmd.ops) {
          if (op.op === "create" && op.doc.id === doc.id) accepted = true;
        }
      },
      onReject: (_id, reason) => {
        rejected = reason;
      },
    },
  });
  await client.start();
  await sleep(400); // settle Welcome + initial resync

  const intent: ClientMsg = {
    type: "intent",
    intent_id: "22222222-2222-2222-2222-222222222222",
    ops: [{ op: "create", doc }],
  };
  client.send(intent);

  for (let i = 0; i < 50 && rejected === null && !accepted; i++) await sleep(100);
  client.stop();
  return rejected;
}

function baseDoc(
  id: string,
  world: string,
  docType: string,
  extra: Record<string, unknown>,
): WireDocument {
  return {
    id,
    scope: { kind: "world", world_id: world },
    doc_type: docType,
    schema_version: 1,
    name: null,
    source: null,
    owner: null,
    permissions: {
      default: "none",
      users: {},
      property_overrides: {},
      capabilities: { by_role: {}, by_user: {} },
      gm_role: null,
    },
    embedded: {},
    parent_id: null,
    system: {},
    created_at: 0,
    updated_at: 0,
    ...extra,
  };
}

test("Create token with an unknown engine field is rejected (invalid)", async () => {
  const cookie = await login(server.baseUrl, "gm", "pw");
  const { world } = server.fixture;

  const doc = baseDoc("33333333-3333-3333-3333-333333333333", world, "token", {
    engine: {
      x: 0,
      y: 0,
      w: 1,
      h: 1,
      rotation: 0,
      // `bogus` is not a TokenEngine field — deny_unknown_fields rejects it.
      bogus: true,
    },
  });

  const reason = await tryCreate(world, server.wsUrl, cookie, doc);
  expect(reason).toBe("invalid");
});

test("Create item with an engine body is rejected (item is not engine-defined)", async () => {
  const cookie = await login(server.baseUrl, "gm", "pw");
  const { world } = server.fixture;

  const doc = baseDoc("44444444-4444-4444-4444-444444444444", world, "item", {
    // "item" is a client-only doc_type (M12c) — the server has no engine
    // struct for it, so ANY populated `engine` body is rejected.
    engine: {},
  });

  const reason = await tryCreate(world, server.wsUrl, cookie, doc);
  expect(reason).toBe("invalid");
});
