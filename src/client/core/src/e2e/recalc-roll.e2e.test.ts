// Node<->Rust end-to-end: a GM's `/roll` produces a RollEmbed with spec/raw
// hidden from a non-GM player but visible to the GM; a GM recalc mutates the
// stored outcome, appends a visible-to-everyone recalc_history entry, and the
// player's redacted view still never receives spec/raw (including inside the
// new history entry's previous_raw).
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

test("GM recalc: spec/raw stay GM-only, recalc_history is visible to everyone and never leaks previous_raw to a player", async () => {
  const gmCookie = await login(server.baseUrl, "gm", "pw");
  const plCookie = await login(server.baseUrl, "pl", "pw");
  const world = server.fixture.world;

  const gm = new WsClient({ world, connect: nodeConnect(server.wsUrl, world, gmCookie), handlers: { onCommand: () => {} } });
  await gm.start();
  await sleep(300);

  let messageId = "";
  let rollId = "";
  let sawGmRaw = false;
  const gmWatch = new WsClient({
    world,
    connect: nodeConnect(server.wsUrl, world, gmCookie),
    handlers: {
      onCommand: (cmd: WireCommand) => {
        for (const op of cmd.ops) {
          if (op.op === "create" && op.doc.doc_type === "message") {
            const content = (op.doc.engine as { content: { kind: string; roll_id?: string; raw?: unknown }[] }).content;
            const embed = content.find((s) => s.kind === "roll_embed");
            if (embed?.roll_id) {
              messageId = op.doc.id;
              rollId = embed.roll_id;
              if (embed.raw) sawGmRaw = true;
            }
          }
        }
      },
    },
  });
  await gmWatch.start();
  await sleep(300);

  let playerSawRaw: unknown = "unset";
  const playerWatch = new WsClient({
    world,
    connect: nodeConnect(server.wsUrl, world, plCookie),
    handlers: {
      onCommand: (cmd: WireCommand) => {
        for (const op of cmd.ops) {
          if (op.op === "create" && op.doc.doc_type === "message") {
            const content = (op.doc.engine as { content: { kind: string; raw?: unknown }[] }).content;
            const embed = content.find((s) => s.kind === "roll_embed");
            if (embed) playerSawRaw = embed.raw ?? null;
          }
        }
      },
    },
  });
  await playerWatch.start();
  await sleep(300);

  await gm.sendChatMessage({ channel: "all", content: "/roll 1d6" });
  await sleep(500);

  expect(messageId).not.toBe("");
  expect(rollId).not.toBe("");
  expect(sawGmRaw).toBe(true);
  expect(playerSawRaw).toBeNull();

  // Recalc as GM.
  let playerSawRecalcHistory: unknown[] | null = null;
  let playerSawRecalcHistoryRaw: unknown = "unset";
  const playerWatch2 = new WsClient({
    world,
    connect: nodeConnect(server.wsUrl, world, plCookie),
    handlers: {
      onCommand: (cmd: WireCommand) => {
        for (const op of cmd.ops) {
          if (op.op === "update" && op.doc_id === messageId) {
            const engineChange = op.changes.find((c) => c.path === "/engine");
            if (engineChange && typeof engineChange.new === "object" && engineChange.new !== null) {
              const content = (engineChange.new as { content: { kind: string; recalc_history?: { previous_raw?: unknown }[] }[] }).content;
              const embed = content.find((s) => s.kind === "roll_embed");
              if (embed?.recalc_history) {
                playerSawRecalcHistory = embed.recalc_history;
                playerSawRecalcHistoryRaw = embed.recalc_history[0]?.previous_raw ?? null;
              }
            }
          }
        }
      },
    },
  });
  await playerWatch2.start();
  await sleep(300);

  await gm.recalcRoll(messageId, rollId, []);
  await sleep(500);

  expect(playerSawRecalcHistory).not.toBeNull();
  expect((playerSawRecalcHistory as unknown[]).length).toBe(1);
  expect(playerSawRecalcHistoryRaw).toBeNull();

  gm.stop();
  gmWatch.stop();
  playerWatch.stop();
  playerWatch2.stop();
});
