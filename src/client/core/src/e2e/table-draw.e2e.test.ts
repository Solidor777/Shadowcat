// Node<->Rust end-to-end: `WsClient.drawTable` against the real server's
// `tables::handle_draw_table`. Covers the one behavior no unit test can:
// a real wire round-trip through `Room::publish`'s redaction, so a nested
// draw's `spec`/`raw` stay GM-only at every depth for a genuine second
// connection, not a mocked `Access`. Also exercises the cycle/permission
// refusals and a `DrawRule::Formula` table's range-matched outcome.
import { afterAll, beforeAll, expect, test } from "vitest";
import WebSocket from "ws";
import { WsClient } from "../ws-client";
import type { Transport, TransportHandlers } from "../transport";
import type { ClientMsg, WireCommand, WireDocument } from "../wire";
import type { TableDrawSegment } from "../chat-docs";
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

/** Builds a `table` document envelope. `defaultRole` gates non-GM READ
 * (`"observer"` for the readable fixtures, `"none"` for the refusal case).
 * @param id The document id.
 * @param world The owning world id.
 * @param draw The `TableEngine.draw` rule.
 * @param rows The table's rows, in `TableEngine.rows` order.
 * @param opts Optional envelope name and default role.
 * @returns A ready-to-send `WireDocument`.
 */
function buildTestTableDoc(
  id: string,
  world: string,
  draw: Record<string, unknown>,
  rows: Record<string, unknown>[],
  opts: { name?: string; defaultRole?: "owner" | "observer" | "none" } = {},
): WireDocument {
  return {
    id,
    scope: { kind: "world", world_id: world },
    doc_type: "table",
    schema_version: 1,
    name: opts.name ?? null,
    source: null,
    owner: null,
    permissions: {
      default: opts.defaultRole ?? "observer",
      users: {},
      property_overrides: {},
      capabilities: { by_role: {}, by_user: {} },
      gm_role: null,
    },
    embedded: {},
    parent_id: null,
    engine: { draw, rows, description: "" },
    system: {},
    created_at: 0,
    updated_at: 0,
  };
}

/** Sends a Create intent for `doc` on `ws` and waits past the server's
 * apply window. Fire-and-settle, mirroring this suite's existing
 * `name-privacy.e2e.test.ts` precedent (GM create, fixed settle sleep,
 * no ack frame to await).
 * @param ws The GM's already-connected client.
 * @param doc The document to create.
 */
async function createDoc(ws: WsClient, doc: WireDocument): Promise<void> {
  const intent: ClientMsg = {
    type: "intent",
    intent_id: crypto.randomUUID(),
    ops: [{ op: "create", doc }],
  };
  ws.send(intent);
  await sleep(400);
}

/** Finds the first `table_draw` segment in a `message` document's engine
 * content, if any.
 * @param doc The message document.
 * @returns The segment, or `undefined` if the doc isn't a matching message.
 */
function tableDrawSegment(doc: WireDocument): TableDrawSegment | undefined {
  if (doc.doc_type !== "message") return undefined;
  const content = (doc.engine as { content?: TableDrawSegment[] }).content;
  return content?.find((s) => s.kind === "table_draw");
}

test("draw_table: a nested Draw fans out to row.nested, redacted spec/raw at every depth for a player but visible to the GM", async () => {
  const gmCookie = await login(server.baseUrl, "gm", "pw");
  const plCookie = await login(server.baseUrl, "pl", "pw");
  const { world } = server.fixture;

  const tableB = "10000000-0000-0000-0000-0000000000b1";
  const tableA = "10000000-0000-0000-0000-0000000000a1";

  const gm = new WsClient({ world, connect: nodeConnect(server.wsUrl, world, gmCookie), handlers: { onCommand: () => {} } });
  await gm.start();
  await sleep(300);

  await createDoc(
    gm,
    buildTestTableDoc(tableB, world, { kind: "weighted" }, [
      { weight: 1, range: null, label: "a gem", results: [] },
      { weight: 1, range: null, label: "a coin", results: [] },
    ]),
  );
  await createDoc(
    gm,
    buildTestTableDoc(tableA, world, { kind: "weighted" }, [
      {
        weight: 1,
        range: null,
        label: "loot chest",
        results: [{ kind: "draw", table_id: tableB, count: 2 }],
      },
    ]),
  );

  let gmSeg: TableDrawSegment | undefined;
  const gmWatch = new WsClient({
    world,
    connect: nodeConnect(server.wsUrl, world, gmCookie),
    handlers: {
      onCommand: (cmd: WireCommand) => {
        for (const op of cmd.ops) {
          if (op.op === "create") {
            const seg = tableDrawSegment(op.doc);
            if (seg?.table_id === tableA) gmSeg = seg;
          }
        }
      },
    },
  });
  await gmWatch.start();
  await sleep(300);

  let plSeg: TableDrawSegment | undefined;
  const plWatch = new WsClient({
    world,
    connect: nodeConnect(server.wsUrl, world, plCookie),
    handlers: {
      onCommand: (cmd: WireCommand) => {
        for (const op of cmd.ops) {
          if (op.op === "create") {
            const seg = tableDrawSegment(op.doc);
            if (seg?.table_id === tableA) plSeg = seg;
          }
        }
      },
    },
  });
  await plWatch.start();
  await sleep(300);

  await gm.drawTable({ tableId: tableA, channel: "general" });
  await sleep(500);

  expect(gmSeg).toBeDefined();
  expect(plSeg).toBeDefined();

  expect(gmSeg?.row?.nested.length).toBe(2);
  expect(plSeg?.row?.nested.length).toBe(2);

  // GM sees the row-selecting roll state at every depth.
  expect(gmSeg?.spec).toBeDefined();
  expect(gmSeg?.raw).toBeTruthy();
  for (const nested of gmSeg?.row?.nested ?? []) {
    expect(nested.spec).toBeDefined();
    expect(nested.raw).toBeTruthy();
  }

  // The player's copy carries neither key, at the top level or any nested draw.
  expect(plSeg && "spec" in plSeg).toBe(false);
  expect(plSeg && "raw" in plSeg).toBe(false);
  for (const nested of plSeg?.row?.nested ?? []) {
    expect("spec" in nested).toBe(false);
    expect("raw" in nested).toBe(false);
  }

  gm.stop();
  gmWatch.stop();
  plWatch.stop();
});

test("draw_table: a player draw against a permissions.default:'none' table is refused with the generic, existence-hiding text", async () => {
  const gmCookie = await login(server.baseUrl, "gm", "pw");
  const plCookie = await login(server.baseUrl, "pl", "pw");
  const { world } = server.fixture;

  const hiddenTable = "10000000-0000-0000-0000-0000000000e1";

  const gm = new WsClient({ world, connect: nodeConnect(server.wsUrl, world, gmCookie), handlers: { onCommand: () => {} } });
  await gm.start();
  await sleep(300);

  await createDoc(
    gm,
    buildTestTableDoc(
      hiddenTable,
      world,
      { kind: "weighted" },
      [{ weight: 1, range: null, label: "secret", results: [] }],
      { defaultRole: "none" },
    ),
  );

  const player = new WsClient({ world, connect: nodeConnect(server.wsUrl, world, plCookie), handlers: { onCommand: () => {} } });
  await player.start();
  await sleep(300);

  let rejection: string | null = null;
  try {
    await player.drawTable({ tableId: hiddenTable, channel: "general" });
  } catch (e) {
    rejection = (e as Error).message;
  }

  expect(rejection).toBe("That table could not be found.");

  gm.stop();
  player.stop();
});

test("draw_table: a table whose row draws itself is refused as a cycle", async () => {
  const gmCookie = await login(server.baseUrl, "gm", "pw");
  const { world } = server.fixture;

  const selfTable = "10000000-0000-0000-0000-0000000000c1";

  const gm = new WsClient({ world, connect: nodeConnect(server.wsUrl, world, gmCookie), handlers: { onCommand: () => {} } });
  await gm.start();
  await sleep(300);

  await createDoc(
    gm,
    buildTestTableDoc(selfTable, world, { kind: "weighted" }, [
      {
        weight: 1,
        range: null,
        label: "loops forever",
        results: [{ kind: "draw", table_id: selfTable, count: 1 }],
      },
    ]),
  );

  let rejection: string | null = null;
  try {
    await gm.drawTable({ tableId: selfTable, channel: "general" });
  } catch (e) {
    rejection = (e as Error).message;
  }

  expect(rejection).toBe("That table refers back to itself.");

  gm.stop();
});

test("draw_table: a DrawRule::Formula table matches the row whose range contains the rolled total", async () => {
  const gmCookie = await login(server.baseUrl, "gm", "pw");
  const { world } = server.fixture;

  const formulaTable = "10000000-0000-0000-0000-0000000000f1";

  const gm = new WsClient({ world, connect: nodeConnect(server.wsUrl, world, gmCookie), handlers: { onCommand: () => {} } });
  await gm.start();
  await sleep(300);

  await createDoc(
    gm,
    buildTestTableDoc(formulaTable, world, { kind: "formula", notation: "2d6" }, [
      { weight: 1, range: { lo: 2, hi: 6 }, label: "low", results: [] },
      { weight: 1, range: { lo: 7, hi: 12 }, label: "high", results: [] },
    ]),
  );

  let seg: TableDrawSegment | undefined;
  const gmWatch = new WsClient({
    world,
    connect: nodeConnect(server.wsUrl, world, gmCookie),
    handlers: {
      onCommand: (cmd: WireCommand) => {
        for (const op of cmd.ops) {
          if (op.op === "create") {
            const found = tableDrawSegment(op.doc);
            if (found?.table_id === formulaTable) seg = found;
          }
        }
      },
    },
  });
  await gmWatch.start();
  await sleep(300);

  await gm.drawTable({ tableId: formulaTable, channel: "general" });
  await sleep(500);

  expect(seg).toBeDefined();
  expect(seg?.row).not.toBeNull();
  const total = seg?.outcome.total ?? null;
  expect(total).not.toBeNull();
  const ranges = [
    { lo: 2, hi: 6 },
    { lo: 7, hi: 12 },
  ];
  const matched = ranges[seg?.row?.index ?? -1];
  expect(matched).toBeDefined();
  expect(total).toBeGreaterThanOrEqual(matched.lo);
  expect(total).toBeLessThanOrEqual(matched.hi);

  gm.stop();
  gmWatch.stop();
});
