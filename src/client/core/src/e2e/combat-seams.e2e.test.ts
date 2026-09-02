// Node<->Rust end-to-end: the combat client seams driven against the real Rust
// test_server. Exercises the full stack in one scenario: a GM authors a scene/resource
// registry/combat/combatants through raw intents, starts and advances the clock through the
// correlated combat_result/combat_error reply, and both a GM and a player connection derive
// identical combat:* hook events from their own per-recipient document stream while the
// player's "combat" channel frame and pathfind reply reflect the server's visibility and
// movement-budget rules.
import { afterAll, beforeAll, expect, test } from "vitest";
import WebSocket from "ws";
import { WsClient, type WireWelcome } from "../ws-client";
import type { Transport, TransportHandlers } from "../transport";
import { DocumentStore } from "../store";
import type { WireDocument, WireCommand, CombatsView } from "../wire";
import { parseCombats } from "../wire";
import { commandTouchesCombat, deriveCombatHookEvents, type CombatHookEvent } from "../combat-hooks";
import { buildSceneDoc, buildTokenDoc, buildCombatDoc, buildCombatantDoc, newCombatEngine } from "../scene-docs";
import type { CombatEngine, CombatantEngine } from "../scene-docs";
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

/** Waits for `predicate` to become true, polling every 100ms up to `tries` times. */
async function waitFor(predicate: () => boolean, tries = 50): Promise<void> {
  for (let i = 0; i < tries && !predicate(); i++) await sleep(100);
  expect(predicate()).toBe(true);
}

/** One connected client's harness: its own authoritative `DocumentStore` (the per-recipient
 * optimistic view has no place in a raw-WsClient e2e test) plus the `combat:*` events derived
 * from every applied command, in arrival order. */
function harness(): { store: DocumentStore; events: CombatHookEvent[] } {
  const store = new DocumentStore();
  const events: CombatHookEvent[] = [];
  return { store, events };
}

function onCommandFor(h: { store: DocumentStore; events: CombatHookEvent[] }) {
  return (cmd: WireCommand): void => {
    const touches = commandTouchesCombat(cmd, h.store);
    const before = new Map<string, WireDocument | undefined>();
    if (touches) {
      for (const op of cmd.ops) {
        const id = op.op === "update" ? op.doc_id : op.doc.id;
        before.set(id, h.store.get(id));
      }
    }
    h.store.applyCommand(cmd);
    if (touches) h.events.push(...deriveCombatHookEvents((id) => before.get(id), cmd, h.store));
  };
}

test("combat seams: correlation, identical hook derivation, per-recipient channel, budget preview", async () => {
  const { world, player } = server.fixture;
  const gmCookie = await login(server.baseUrl, "gm", "pw");
  const playerCookie = await login(server.baseUrl, "pl", "pw");

  const gmH = harness();
  const playerH = harness();

  let gmWelcome: WireWelcome | null = null;
  const gmRejects: unknown[] = [];
  const gmClient = new WsClient({
    world,
    connect: nodeConnect(server.wsUrl, world, gmCookie),
    handlers: { onCommand: onCommandFor(gmH), onReject: (id, reason) => gmRejects.push({ id, reason }), onWelcome: (w) => { gmWelcome = w; } },
  });
  let playerWelcome: WireWelcome | null = null;
  const playerClient = new WsClient({
    world,
    connect: nodeConnect(server.wsUrl, world, playerCookie),
    handlers: { onCommand: onCommandFor(playerH), onReject: () => {}, onWelcome: (w) => { playerWelcome = w; } },
  });

  await gmClient.start();
  await playerClient.start();
  await waitFor(() => gmWelcome !== null);
  await waitFor(() => playerWelcome !== null);

  // The server seeds every world with an empty `resource-registry` singleton at first touch
  // (`data::world_seed::missing_config_ops`), so this scenario UPDATES that existing document
  // to add its `movement` resource rather than creating a second one (which the singleton
  // ingress gate would reject as a conflict).
  const snapshotRes = await fetch(`${server.baseUrl}/api/worlds/${world}/snapshot`, { headers: { cookie: gmCookie } });
  const snapshot = (await snapshotRes.json()) as { documents: WireDocument[]; seq: number };
  const existingRegistry = snapshot.documents.find((d) => d.doc_type === "resource-registry")!;
  const registryId = existingRegistry.id;

  const sceneId = "73b7c3e2-6842-4fe5-932f-751a767c9dea";
  const combatId = "64ee6577-3116-4e21-b8bf-4ed5fb5123b9";
  const playerTokenId = "9e38eeab-d9cb-48a9-94c9-c4a6e93f1594";
  const playerCombatantId = "eb3dc7fc-ab41-4346-9778-cf63a32a107b";
  const gmCombatantId = "5f993130-e7d1-4db9-a753-3e33be67dd81";
  const hiddenCombatantId = "f593d50f-789c-48da-bd3e-c9d4bb52abf1";

  // `combat_start` re-resolves `movement`/`turn_control`/etc. from the defaults chain (scene >
  // world > system) rather than trusting whatever a Create stamped on the combat document
  // directly (those fields are a SNAPSHOT of that chain, not authored state) — see
  // `CombatEngine.movement`'s own doc. The scene-level override is the most specific layer, so
  // it is what actually reaches the started combat's `movement.enforcement`.
  const sceneDoc = buildSceneDoc(
    world,
    {
      grid: { kind: "square", size: 100, distance: { perCell: 5, unit: "ft" } },
      // The 100x100-unit fail-safe default bounds is exactly one cell — too small for the
      // player's token (already 100x100) to have anywhere reachable to step to.
      bounds: { width: 1000, height: 1000 },
      // Unrestricted: no vision/lighting is authored, so a player's default `Visible` movement
      // restriction would see (and therefore route through) nothing at all.
      vision: { losRestriction: null, fog: null, observerVision: null, movementRestriction: "unrestricted", movementModel: null },
      combat: { movementResource: "movement", interpretation: "per_cell", enforcement: "warn" },
    },
    sceneId,
  );
  const registryUpdate = {
    op: "update" as const,
    doc_id: registryId,
    changes: [
      {
        path: "/engine/resources",
        old: (existingRegistry.engine as { resources: unknown }).resources,
        new: { movement: { name: "Movement", order: 0, binding: { kind: "tracked", max: 30, recover: { turn_start: 0, turn_end: 0, round_start: 0, round_end: 0 } } } },
      },
    ],
  };
  const tokenDoc = buildTokenDoc(world, sceneId, { x: 0, y: 0, w: 100, h: 100, rotation: 0, visual: { kind: "image", asset: "a" }, actor_id: null, overrides: null, face: null }, playerTokenId);
  tokenDoc.owner = player;

  const combatEngine: CombatEngine = {
    ...newCombatEngine(sceneId),
    movement: { resource: "movement", interpretation: "per_cell", enforcement: "warn" },
    order: [playerCombatantId, gmCombatantId],
  };
  const combatDoc = buildCombatDoc(world, combatEngine, combatId);

  const playerCombatantEngine: CombatantEngine = {
    kind: { type: "actor", token_id: playerTokenId, actor_id: null },
    initiative: 10,
    tiebreak: 0,
    resources: { movement: { current: 30 } },
  };
  const playerCombatantDoc = buildCombatantDoc(world, combatId, playerCombatantEngine, { owner: player, id: playerCombatantId });

  const gmCombatantEngine: CombatantEngine = {
    kind: { type: "event", lifespan: null, message: null },
    initiative: 5,
    tiebreak: 0,
    resources: {},
  };
  const gmCombatantDoc = buildCombatantDoc(world, combatId, gmCombatantEngine, { id: gmCombatantId, name: "lair" });

  const hiddenCombatantEngine: CombatantEngine = {
    kind: { type: "event", lifespan: null, message: null },
    initiative: 1,
    tiebreak: 0,
    resources: {},
  };
  const hiddenCombatantDoc = buildCombatantDoc(world, combatId, hiddenCombatantEngine, { id: hiddenCombatantId, hidden: true, name: "secret" });

  // Scene + registry update first: the registry write must be accepted before the combatants
  // that reference `movement` are created, so a genuine rejection surfaces immediately.
  gmRejects.length = 0;
  gmClient.send({ type: "intent", intent_id: crypto.randomUUID(), ops: [{ op: "create", doc: sceneDoc }, registryUpdate] });
  await waitFor(() => gmH.store.get(sceneId) !== undefined || gmRejects.length > 0);
  if (gmRejects.length > 0) throw new Error(`scene/registry rejected: ${JSON.stringify(gmRejects)}`);

  const creates: WireDocument[] = [tokenDoc, combatDoc, playerCombatantDoc, gmCombatantDoc, hiddenCombatantDoc];
  gmRejects.length = 0;
  gmClient.send({ type: "intent", intent_id: crypto.randomUUID(), ops: creates.map((doc) => ({ op: "create" as const, doc })) });
  await waitFor(() => gmH.store.get(combatId) !== undefined || gmRejects.length > 0);
  if (gmRejects.length > 0) throw new Error(`combat setup rejected: ${JSON.stringify(gmRejects)}`);
  await waitFor(() => playerH.store.get(combatId) !== undefined);

  // (d): the hidden combatant reaches the GM's store, never the player's.
  expect(gmH.store.get(hiddenCombatantId)).toBeDefined();
  expect(playerH.store.get(hiddenCombatantId)).toBeUndefined();

  // Subscribe both clients to the "combat" channel before starting the clock.
  let gmCombatFrame: CombatsView | null = null;
  let playerCombatFrame: CombatsView | null = null;
  await gmClient.subscribeScene("combat", (f) => { gmCombatFrame = parseCombats(f.payload); });
  await playerClient.subscribeScene("combat", (f) => { playerCombatFrame = parseCombats(f.payload); });

  // (a): CombatResult correlation — `combat()` resolves only once the broadcast event applies.
  await gmClient.combat({ type: "combat_start", request_id: "bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb", combat_id: combatId });
  await waitFor(() => (gmH.store.get(combatId)?.engine as CombatEngine).active === true);
  await waitFor(() => (playerH.store.get(combatId)?.engine as CombatEngine).active === true);

  await gmClient.combat({ type: "combat_advance", request_id: "cccccccc-cccc-cccc-cccc-cccccccccccc", combat_id: combatId });
  await waitFor(() => (gmH.store.get(combatId)?.engine as CombatEngine).round === 2);
  await waitFor(() => (playerH.store.get(combatId)?.engine as CombatEngine).round === 2);

  // (b): both clients derived identical combat:* events for the visible transition (start
  // through the first advance) — the hidden combatant never enters `order`/`turn`, so nothing
  // in the derivation depends on a document only the GM's store holds.
  expect(playerH.events).toEqual(gmH.events);
  expect(gmH.events.map((e) => e.name)).toContain("combat:start");
  expect(gmH.events.map((e) => e.name)).toContain("combat:turn-start");

  // (c): the player's channel frame carries their own combatant's numbers and `null` for the
  // GM's (default-visibility, not owner-or-GM) combatant.
  await waitFor(() => playerCombatFrame !== null && playerCombatFrame.combats.length > 0);
  const playerView = (playerCombatFrame as unknown as CombatsView).combats.find((c) => c.id === combatId)!;
  const ownCc = playerView.combatants.find((cc) => cc.id === playerCombatantId)!;
  const gmCc = playerView.combatants.find((cc) => cc.id === gmCombatantId)!;
  expect(ownCc.resources).not.toBeNull();
  expect(gmCc.resources).toBeNull();
  // (d), channel half: the hidden combatant never appears in the player's frame at all,
  // though the GM's own frame carries it.
  expect(playerView.combatants.some((cc) => cc.id === hiddenCombatantId)).toBe(false);
  await waitFor(() => gmCombatFrame !== null && gmCombatFrame.combats.length > 0);
  const gmView = (gmCombatFrame as unknown as CombatsView).combats.find((c) => c.id === combatId)!;
  expect(gmView.combatants.some((cc) => cc.id === hiddenCombatantId)).toBe(true);

  // (e): the player's own pathfind reply carries `budgetCells` under Warn enforcement —
  // current(30) / perCell(5) = 6.
  const result = await playerClient.pathfind(sceneId, [0, 0], [[100, 0]], 0, playerTokenId);
  expect(result.budgetCells).toBe(6);

  gmClient.stop();
  playerClient.stop();
}, 30_000);
