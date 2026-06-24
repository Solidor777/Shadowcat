import { test, expect, vi } from "vitest";
import { ContributionRegistry, silentLogger, buildTokenDoc, buildActorDoc, type Connect, type WireDocument } from "@shadowcat/core";
import { WorldSession } from "./worldSession.svelte";
import { listWorldMembers } from "./api";

// The members fetch hits the network; stub it (default empty for the player/GM
// tests that don't care, overridable per test).
vi.mock("./api", async (importActual) => {
  const actual = await importActual<typeof import("./api")>();
  return { ...actual, listWorldMembers: vi.fn().mockResolvedValue([]) };
});

// `MockServer` is internal core test code (not barrel-exported), so use a minimal
// inline Connect that delivers one valid Welcome frame on connect and ignores
// sends. The frame must satisfy parseServerMsg (all welcome fields present).
const welcomeFrame = {
  type: "welcome",
  world: "w1",
  current_seq: 0,
  server_time: 0,
  world_default_grants: { by_role: {}, by_user: {} },
  user_role: "player",
  capability_requirements: [],
  contract_declarations: [],
};

// Deliver the Welcome `count` times to exercise reconnect-idempotency (the
// server re-sends Welcome on every (re)connect).
function mockConnect(count = 1): Connect {
  return (handlers) => {
    queueMicrotask(() => {
      for (let i = 0; i < count; i++) handlers.onMessage(JSON.stringify(welcomeFrame));
    });
    return Promise.resolve({ send: () => {}, close: () => handlers.onClose() });
  };
}

// A core-ui stand-in: provides the root surface so activation is exercised without
// pulling the real (Svelte-importing) module into this unit test.
const coreUiStub = {
  manifest: {
    id: "core-ui",
    version: "0.1.0",
    dependencies: {},
    provides: [{ contract: "shadowcat.surface:root", cardinality: "singleton" as const }],
  },
  register: vi.fn(),
};

test("enter starts the socket, captures role from Welcome, activates core-ui", async () => {
  const session = new WorldSession({
    selfId: "u1",
    connect: mockConnect(),
    modules: [coreUiStub],
    logger: silentLogger,
  });

  await session.enter("w1");
  // Welcome arrives on a microtask after connect; poll until handled.
  await vi.waitFor(() => expect(session.role).toBe("player"));
  await vi.waitFor(() => expect(coreUiStub.register).toHaveBeenCalledOnce());
  expect(session.contributions).toBeInstanceOf(ContributionRegistry);

  session.leave();
  expect(session.state).toBe("closed");
});

test("a repeated Welcome (reconnect) does not re-add core-ui or throw", async () => {
  coreUiStub.register.mockClear();
  const session = new WorldSession({
    selfId: "u1",
    connect: mockConnect(2),
    modules: [coreUiStub],
    logger: silentLogger,
  });

  await session.enter("w1");
  await vi.waitFor(() => expect(session.role).toBe("player"));
  // Idempotent: the module is added/activated exactly once across two Welcomes.
  await vi.waitFor(() => expect(coreUiStub.register).toHaveBeenCalledTimes(1));
  // Give the second Welcome a tick to (not) double-add.
  await Promise.resolve();
  expect(coreUiStub.register).toHaveBeenCalledTimes(1);
});

test("applies asset_changed to the resolver and notifies subscribers", async () => {
  // A connect that delivers Welcome and lets the test push later frames.
  let push!: (frame: unknown) => void;
  const connect: Connect = (handlers) => {
    push = (frame) => handlers.onMessage(JSON.stringify(frame));
    queueMicrotask(() => handlers.onMessage(JSON.stringify(welcomeFrame)));
    return Promise.resolve({ send: () => {}, close: () => handlers.onClose() });
  };
  const session = new WorldSession({
    selfId: "u1",
    connect,
    modules: [coreUiStub],
    logger: silentLogger,
  });
  const got: Array<{ uuid: string; op: string }> = [];
  session.onAssetChanged((m) => got.push(m));
  await session.enter("w1");

  const before = session.assets.url("a1"); // "/api/assets/a1"
  push({ type: "asset_changed", uuid: "a1", op: "replaced" });
  await vi.waitFor(() => expect(got).toHaveLength(1));
  // Resolver cache-busts on replace, and subscribers are notified.
  expect(session.assets.url("a1")).not.toBe(before);
  expect(got).toEqual([{ uuid: "a1", op: "replaced" }]);
});

function sceneCreates(sent: Array<Record<string, unknown>>): unknown[] {
  return sent.filter(
    (m) =>
      m.type === "intent" &&
      Array.isArray(m.ops) &&
      (m.ops as Array<{ op: string; doc?: { doc_type?: string } }>).some(
        (o) => o.op === "create" && o.doc?.doc_type === "scene",
      ),
  );
}

// A connect whose Welcome frames are pushed by the test AFTER enter() resolves —
// matching reality (Welcome arrives over an established socket, so intents the
// session dispatches while handling Welcome are actually transmitted).
function pushConnect(sent: Array<Record<string, unknown>>): { connect: Connect; push: (f: unknown) => void } {
  let push!: (f: unknown) => void;
  const connect: Connect = (handlers) => {
    push = (f) => handlers.onMessage(JSON.stringify(f));
    return Promise.resolve({ send: (d) => sent.push(JSON.parse(d)), close: () => handlers.onClose() });
  };
  return { connect, push: (f: unknown) => push(f) };
}

test("auto-creates a default scene on GM entry, exactly once across reconnect Welcomes", async () => {
  const sent: Array<Record<string, unknown>> = [];
  const { connect, push } = pushConnect(sent);
  const gmFrame = { ...welcomeFrame, user_role: "gm" };
  const session = new WorldSession({ selfId: "u1", connect, modules: [coreUiStub], logger: silentLogger });
  await session.enter("w1");
  push(gmFrame);
  await vi.waitFor(() => expect(sceneCreates(sent).length).toBe(1));
  // A reconnect re-fires Welcome; the optimistic-view guard must not double-create.
  push(gmFrame);
  await new Promise((r) => setTimeout(r, 20));
  expect(sceneCreates(sent)).toHaveLength(1);
});

test("sendPing transmits a scene_ping for the active scene; onPing fires on an inbound ping", async () => {
  const sent: Array<Record<string, unknown>> = [];
  const { connect, push } = pushConnect(sent);
  const gmFrame = { ...welcomeFrame, user_role: "gm" };
  const session = new WorldSession({ selfId: "u1", connect, modules: [coreUiStub], logger: silentLogger });
  await session.enter("w1");
  push(gmFrame); // GM → auto-creates a scene (the ping's parent)
  await vi.waitFor(() => expect(sceneCreates(sent).length).toBe(1));

  session.sendPing(12, 34);
  const ping = sent.find((m) => m.type === "scene_ping");
  expect(ping).toBeTruthy();
  expect(ping!.x).toBe(12);
  expect(typeof ping!.scene).toBe("string");

  const got: Array<{ user: string }> = [];
  session.onPing((m) => got.push(m));
  push({ type: "scene_ping", scene: "s1", x: 1, y: 2, user: "u9" });
  await vi.waitFor(() => expect(got).toHaveLength(1));
  expect(got[0].user).toBe("u9");
});

test("does not auto-create a scene for a non-GM actor", async () => {
  const sent: Array<Record<string, unknown>> = [];
  const { connect, push } = pushConnect(sent);
  const session = new WorldSession({ selfId: "u1", connect, modules: [coreUiStub], logger: silentLogger });
  await session.enter("w1");
  push(welcomeFrame); // user_role: "player"
  await vi.waitFor(() => expect(session.role).toBe("player"));
  await new Promise((r) => setTimeout(r, 20));
  expect(sceneCreates(sent)).toHaveLength(0);
});

test("dispatchIntent predicts via ctx.client and sends one correlated intent frame", async () => {
  // A core-ui stand-in whose register captures ctx.client (the optimistic view
  // modules read), so the prediction is observable.
  let capturedClient: { get(id: string): unknown } | null = null;
  const stub = {
    manifest: {
      id: "core-ui",
      version: "0.1.0",
      dependencies: {},
      provides: [{ contract: "shadowcat.surface:root", cardinality: "singleton" as const }],
    },
    register: (ctx: { client: { get(id: string): unknown } }) => {
      capturedClient = ctx.client;
    },
  };
  const sent: Array<Record<string, unknown>> = [];
  const connect: Connect = (handlers) => {
    queueMicrotask(() => handlers.onMessage(JSON.stringify(welcomeFrame)));
    return Promise.resolve({ send: (d) => sent.push(JSON.parse(d)), close: () => handlers.onClose() });
  };
  const session = new WorldSession({ selfId: "u1", connect, modules: [stub], logger: silentLogger });
  await session.enter("w1");
  await vi.waitFor(() => expect(capturedClient).not.toBeNull());

  const doc = buildTokenDoc("w1", "s1", { x: 0, y: 0, w: 100, h: 100, rotation: 0, visual: { kind: "image", asset: "a" } }, "tok-1");
  session.dispatchIntent([{ op: "create", doc }]);

  // Prediction: the optimistic view (ctx.client) shows the new doc immediately.
  expect(capturedClient!.get("tok-1")).toBeTruthy();
  // Send: exactly one intent frame, with a generated id and the same ops.
  const intents = sent.filter((m) => m.type === "intent");
  expect(intents).toHaveLength(1);
  expect(typeof intents[0].intent_id).toBe("string");
  expect((intents[0].intent_id as string).length).toBeGreaterThan(0);
  expect(intents[0].ops).toEqual([{ op: "create", doc }]);
});

test("dispatchIntent while disconnected drops the action (no orphaned prediction)", async () => {
  let capturedClient: { get(id: string): unknown } | null = null;
  const stub = {
    manifest: { id: "core-ui", version: "0.1.0", dependencies: {}, provides: [{ contract: "shadowcat.surface:root", cardinality: "singleton" as const }] },
    register: (ctx: { client: { get(id: string): unknown } }) => { capturedClient = ctx.client; },
  };
  const sent: Array<Record<string, unknown>> = [];
  const { connect, push } = pushConnect(sent);
  const session = new WorldSession({ selfId: "u1", connect, modules: [stub], logger: silentLogger });
  await session.enter("w1");
  push(welcomeFrame);
  await vi.waitFor(() => expect(capturedClient).not.toBeNull());

  session.leave(); // tears down the socket → no transport
  const doc = buildTokenDoc("w1", "s1", { x: 0, y: 0, w: 100, h: 100, rotation: 0, visual: { kind: "image", asset: "a" } }, "tok-x");
  session.dispatchIntent([{ op: "create", doc }]);

  // Neither predicted (no orphaned pending to mis-correlate) nor transmitted.
  expect(capturedClient!.get("tok-x")).toBeUndefined();
  expect(sent.filter((m) => m.type === "intent")).toHaveLength(0);
});

test("a GM Welcome populates members in place (stable reference) for see-as labels", async () => {
  vi.mocked(listWorldMembers).mockResolvedValueOnce([
    { user: "u9", username: "Zara", role: "player" },
  ]);
  const sent: Array<Record<string, unknown>> = [];
  const { connect, push } = pushConnect(sent);
  const session = new WorldSession({ selfId: "u1", connect, modules: [coreUiStub], logger: silentLogger });
  // AppContext captures this reference at mount; it must stay valid as members load.
  const captured = session.members;
  await session.enter("w1");
  push({ ...welcomeFrame, user_role: "gm" });
  await vi.waitFor(() => expect(session.members.get("u9")).toBe("Zara"));
  // Mutated in place, never reassigned — the captured snapshot sees the update.
  expect(session.members).toBe(captured);
  expect(captured.get("u9")).toBe("Zara");
});

test("an intent dispatched while reconnecting is predicted, queued, and flushed after resync", async () => {
  let capturedClient: { get(id: string): unknown } | null = null;
  const stub = {
    manifest: { id: "core-ui", version: "0.1.0", dependencies: {}, provides: [{ contract: "shadowcat.surface:root", cardinality: "singleton" as const }] },
    register: (ctx: { client: { get(id: string): unknown } }) => { capturedClient = ctx.client; },
  };
  const sent: Array<Record<string, unknown>> = [];
  let handlers!: { onMessage: (d: string) => void; onClose: () => void };
  let connectCount = 0;
  const connect: Connect = (h) => {
    connectCount++;
    handlers = h;
    return Promise.resolve({ send: (d) => sent.push(JSON.parse(d)), close: () => h.onClose() });
  };
  const session = new WorldSession({ selfId: "u1", connect, modules: [stub], logger: silentLogger });
  await session.enter("w1");
  handlers.onMessage(JSON.stringify(welcomeFrame));
  await vi.waitFor(() => expect(capturedClient).not.toBeNull());

  // Transport drops but the client stays running → reconnecting.
  handlers.onClose();
  const doc = buildTokenDoc("w1", "s1", { x: 0, y: 0, w: 100, h: 100, rotation: 0, visual: { kind: "image", asset: "a" } }, "tok-off");
  session.dispatchIntent([{ op: "create", doc }]);
  // Predicted immediately, but NOT transmitted while offline.
  expect(capturedClient!.get("tok-off")).toBeTruthy();
  const offSent = (): Array<Record<string, unknown>> =>
    sent.filter((m) => m.type === "intent" && JSON.stringify(m.ops).includes("tok-off"));
  expect(offSent()).toHaveLength(0);

  // Reconnect fires (backoff), then a fresh Welcome → resync completes → flush.
  await vi.waitFor(() => expect(connectCount).toBe(2), { timeout: 2000 });
  handlers.onMessage(JSON.stringify(welcomeFrame));
  await vi.waitFor(() => expect(offSent()).toHaveLength(1));
});

function actorWith(perms: Partial<WireDocument["permissions"]>): WireDocument {
  const d = buildActorDoc("w1", { name: "G", displayName: "G", visual: { kind: "image", asset: "a" }, size: { w: 1, h: 1 }, shape: "square", faction: null, conditions: [], prototype: false }, "act1");
  d.permissions = { ...d.permissions, ...perms };
  return d;
}

test("canEdit: a non-GM owner may write /system/conditions; a non-owner may not; selfId is exposed", async () => {
  const { connect, push } = pushConnect([]);
  const session = new WorldSession({ selfId: "u-self", connect, modules: [coreUiStub], logger: silentLogger });
  await session.enter("w1");
  push(welcomeFrame); // user_role: "player", empty grants/requirements
  await vi.waitFor(() => expect(session.role).toBe("player"));

  expect(session.selfId).toBe("u-self");
  // DocRole "owner" floor grants core:write_fields → may write the /system subtree.
  const owned = actorWith({ users: { "u-self": "owner" } });
  expect(session.canEdit(owned, "/system/conditions")).toBe(true);
  // Default observer (read-only) → no write_fields.
  const other = actorWith({ default: "observer" });
  expect(session.canEdit(other, "/system/conditions")).toBe(false);
});

test("canEdit: a GM bypasses the capability check", async () => {
  const { connect, push } = pushConnect([]);
  const session = new WorldSession({ selfId: "u-self", connect, modules: [coreUiStub], logger: silentLogger });
  await session.enter("w1");
  push({ ...welcomeFrame, user_role: "gm" });
  await vi.waitFor(() => expect(session.role).toBe("gm"));
  const locked = actorWith({ default: "observer" });
  expect(session.canEdit(locked, "/system/conditions")).toBe(true);
});

test("subscribeScene sends scene_subscribe and re-establishes on a reconnect Welcome", async () => {
  let push!: (frame: unknown) => void;
  const sent: Array<Record<string, unknown>> = [];
  const connect: Connect = (handlers) => {
    push = (frame) => handlers.onMessage(JSON.stringify(frame));
    queueMicrotask(() => push(welcomeFrame));
    return Promise.resolve({ send: (d) => sent.push(JSON.parse(d)), close: () => handlers.onClose() });
  };
  const session = new WorldSession({ selfId: "u1", connect, modules: [coreUiStub], logger: silentLogger });
  await session.enter("w1");
  await vi.waitFor(() => expect(session.role).toBe("player"));

  const frames: unknown[] = [];
  session.subscribeScene("identity", (f) => frames.push(f));
  await vi.waitFor(() => expect(sent.filter((m) => m.type === "scene_subscribe")).toHaveLength(1));
  const req = sent.find((m) => m.type === "scene_subscribe")!;
  // First frame resolves the underlying ws subscription + fires onUpdate.
  push({ type: "scene_derived", request_id: req.request_id, channel: "identity", computed_at_seq: 0, payload: {} });
  await vi.waitFor(() => expect(frames).toHaveLength(1));

  // A second Welcome (reconnect) must re-establish the subscription — and tear down
  // the prior one, so no duplicate server subscription leaks.
  push(welcomeFrame);
  await vi.waitFor(() => expect(sent.filter((m) => m.type === "scene_subscribe")).toHaveLength(2));
  await vi.waitFor(() =>
    expect(sent.some((m) => m.type === "scene_unsubscribe" && m.request_id === req.request_id)).toBe(true),
  );
});
