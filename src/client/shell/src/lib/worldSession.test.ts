import { test, expect, vi } from "vitest";
import {
  ContributionRegistry,
  silentLogger,
  buildTokenDoc,
  buildActorDoc,
  buildWorldSettingsDoc,
  buildSceneDoc,
  DEFAULT_WORLD_SETTINGS,
  type Connect,
  type WireDocument,
} from "@shadowcat/core";
import { WorldSession } from "./worldSession.svelte";
import { listWorldMembers } from "@shadowcat/core";

// The members fetch hits the network; stub it (safe default, overridable per
// test), alongside the external-module discovery fetches (installed set + a
// world's enabled set), which also hit the network — default to "nothing
// enabled" so the 25+ existing Welcome-flow tests below are unaffected (empty
// enabled set → #loadExternalModules returns before ever calling
// loadModules/import()). The server version now arrives on the Welcome
// payload, not a fetch — ensure the mock Welcome message `mockConnect` builds
// includes a `server_version: "0.1.0"` field, since the `WireWelcome` schema
// now requires it.
vi.mock("@shadowcat/core", async (importActual) => {
  const actual = await importActual<typeof import("@shadowcat/core")>();
  return {
    ...actual,
    listWorldMembers: vi.fn().mockResolvedValue([]),
    listInstalledModules: vi.fn().mockResolvedValue([]),
    getEnabledModules: vi.fn().mockResolvedValue([]),
  };
});

// `MockServer` is internal core test code (not barrel-exported), so use a minimal
// inline Connect that delivers one valid Welcome frame on connect and ignores
// sends. The frame must satisfy parseServerMsg (all welcome fields present).
const welcomeFrame = {
  type: "welcome",
  world: "w1",
  current_seq: 0,
  server_time: 0,
  server_version: "0.1.0",
  world_default_grants: { by_role: {}, by_user: {} },
  user_role: "player",
  capability_requirements: [],
  contract_declarations: [],
  schema_declarations: [],
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

// Two modules whose contract declarations are mutually circular (A requires
// B's contract, B requires A's), so `ModuleRegistry.activate()`'s `topoSort`
// throws on the cycle. Used to prove a thrown activation is re-attempted on
// the next Welcome, not latched for the session's life.
const cycleModA = {
  manifest: {
    id: "cycle-a",
    version: "0.1.0",
    dependencies: {},
    provides: [{ contract: "cycle-a:out", cardinality: "singleton" as const }],
    requires: ["cycle-b:out"],
  },
  register: vi.fn(),
};
const cycleModB = {
  manifest: {
    id: "cycle-b",
    version: "0.1.0",
    dependencies: {},
    provides: [{ contract: "cycle-b:out", cardinality: "singleton" as const }],
    requires: ["cycle-a:out"],
  },
  register: vi.fn(),
};

test("a failed first activation is re-attempted on the next Welcome (no permanent latch)", async () => {
  // Sequential (not concurrent) Welcome deliveries: the second is pushed only
  // after the first has fully resolved (including the outer catch/log), which
  // is what "re-attempted on the NEXT Welcome" means — a second Welcome that
  // races the first's still-pending activation is a different, already-covered
  // concurrency invariant (see "a repeated Welcome (reconnect) does not
  // re-add core-ui or throw").
  const { connect, push } = pushConnect([]);
  const errors: unknown[][] = [];
  const logger = { ...silentLogger, error: (...args: unknown[]) => errors.push(args) };
  const session = new WorldSession({
    selfId: "u1",
    connect,
    modules: [cycleModA, cycleModB],
    logger,
  });
  await session.enter("w1");
  push(welcomeFrame);
  // Pins: the bootstrap guard does not latch a failed activation as done, so a second
  // Welcome re-attempts activation and both Welcomes log the failure.
  await vi.waitFor(() => expect(errors).toHaveLength(1));
  expect(errors[0][0]).toBe("module activation failed; Surfaces degrade until a later Welcome retries");

  push(welcomeFrame);
  await vi.waitFor(() => expect(errors).toHaveLength(2));
  expect(errors[1][0]).toBe("module activation failed; Surfaces degrade until a later Welcome retries");
});

test("a failed activation still runs the rest of the Welcome handler", async () => {
  const { connect, push } = pushConnect([]);
  const errors: unknown[][] = [];
  const logger = { ...silentLogger, error: (...args: unknown[]) => errors.push(args) };
  const session = new WorldSession({
    selfId: "u1",
    connect,
    modules: [cycleModA, cycleModB],
    logger,
  });
  // This suite has no `clearMocks` config, so the shared `listWorldMembers` mock's call
  // history otherwise carries over from every earlier test that also calls `session.enter()`;
  // clear it here so the assertion below proves THIS session's failed activation still triggers
  // the member fetch, not residual calls from an earlier test.
  vi.mocked(listWorldMembers).mockClear();
  await session.enter("w1");
  push(welcomeFrame);
  await vi.waitFor(() => expect(errors).toHaveLength(1));
  expect(listWorldMembers).toHaveBeenCalledWith("w1");
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
  const got: Array<{ uuid: string; op: string; version: number }> = [];
  session.onAssetChanged((m) => got.push(m));
  await session.enter("w1");

  const before = session.assets.url("a1"); // "/api/assets/a1"
  push({ type: "asset_changed", uuid: "a1", op: "replaced", version: 2 });
  await vi.waitFor(() => expect(got).toHaveLength(1));
  // Resolver adopts the frame's authoritative version, and subscribers are notified.
  expect(session.assets.url("a1")).not.toBe(before);
  expect(got).toEqual([{ uuid: "a1", op: "replaced", version: 2 }]);
});

test("evicted frame surfaces through onEvicted", async () => {
  let push!: (frame: unknown) => void;
  const connect: Connect = (handlers) => {
    push = (frame) => handlers.onMessage(JSON.stringify(frame));
    queueMicrotask(() => handlers.onMessage(JSON.stringify(welcomeFrame)));
    return Promise.resolve({ send: () => {}, close: () => handlers.onClose() });
  };
  const onEvicted = vi.fn();
  const session = new WorldSession({
    selfId: "u1",
    connect,
    modules: [coreUiStub],
    logger: silentLogger,
    onEvicted,
  });
  await session.enter("w1");
  push({ type: "evicted", user: null });
  await vi.waitFor(() => expect(onEvicted).toHaveBeenCalledOnce());
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
  const sceneId = (
    (sceneCreates(sent)[0] as { ops: Array<{ doc?: { id?: string } }> }).ops.find((o) => o.doc)!.doc!.id
  ) as string;
  push({ type: "scene_ping", scene: sceneId, x: 1, y: 2, user: "u9" });
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

  const doc = buildTokenDoc("w1", "s1", { x: 0, y: 0, w: 100, h: 100, rotation: 0, visual: { kind: "image", asset: "a" }, actor_id: null, overrides: null, face: null }, "tok-1");
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
  const doc = buildTokenDoc("w1", "s1", { x: 0, y: 0, w: 100, h: 100, rotation: 0, visual: { kind: "image", asset: "a" }, actor_id: null, overrides: null, face: null }, "tok-x");
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

test("a player Welcome also populates members (chat resolves author/recipient names for every role)", async () => {
  vi.mocked(listWorldMembers).mockResolvedValueOnce([
    { user: "u9", username: "Zara", role: "player" },
  ]);
  const sent: Array<Record<string, unknown>> = [];
  const { connect, push } = pushConnect(sent);
  const session = new WorldSession({ selfId: "u1", connect, modules: [coreUiStub], logger: silentLogger });
  await session.enter("w1");
  push({ ...welcomeFrame, user_role: "player" });
  await vi.waitFor(() => expect(session.members.get("u9")).toBe("Zara"));
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
  const doc = buildTokenDoc("w1", "s1", { x: 0, y: 0, w: 100, h: 100, rotation: 0, visual: { kind: "image", asset: "a" }, actor_id: null, overrides: null, face: null }, "tok-off");
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
  const d = buildActorDoc("w1", "G", { displayName: "G", visual: { kind: "image", asset: "a" }, size: { w: 1, h: 1 }, shape: "square", faction: null, conditions: [], prototype: false, vision: null }, "act1");
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

test("canEdit: effective token ownership (inherited from the linked actor) unlocks /engine writes", async () => {
  const { connect, push } = pushConnect([]);
  const session = new WorldSession({ selfId: "u-self", connect, modules: [coreUiStub], logger: silentLogger });
  await session.enter("w1");
  push(welcomeFrame); // user_role: "player", empty grants/requirements
  await vi.waitFor(() => expect(session.role).toBe("player"));

  // The actor this player owns, and a linked token carrying NO per-token override
  // and the shipping `buildTokenDoc` permissions default (observer = read-only).
  const actor = actorWith({});
  actor.owner = "u-self";
  const linked = buildTokenDoc(
    "w1",
    "s1",
    { x: 0, y: 0, w: 100, h: 100, rotation: 0, visual: null, actor_id: "act1", overrides: null, face: null },
    "tok-linked",
  );
  session.dispatchIntent([
    { op: "create", doc: actor },
    { op: "create", doc: linked },
  ]);

  expect(session.canEdit(session.documents.get("tok-linked")!, "/engine/x")).toBe(true);

  // Non-vacuity: the SAME token, same path — re-owning the ACTOR to someone else
  // (with no write to the token) revokes it. A mirror that ignored inheritance,
  // or defaulted open, would not flip here.
  session.dispatchIntent([
    { op: "update", doc_id: "act1", changes: [{ path: "/owner", old: "u-self", new: "u-other" }] },
  ]);
  expect(session.canEdit(session.documents.get("tok-linked")!, "/engine/x")).toBe(false);

  // A per-token override hands it back, beating the actor's owner.
  session.dispatchIntent([
    { op: "update", doc_id: "tok-linked", changes: [{ path: "/owner", old: null, new: "u-self" }] },
  ]);
  expect(session.canEdit(session.documents.get("tok-linked")!, "/engine/x")).toBe(true);

  // The owner floor stops at write_fields: an effective owner may not re-assign
  // ownership or edit permissions (both need core:edit_permissions).
  expect(session.canEdit(session.documents.get("tok-linked")!, "/owner")).toBe(false);
  expect(session.canEdit(session.documents.get("tok-linked")!, "/permissions/default")).toBe(false);
});

test("canEdit: effective ownership does not leak to non-token doc_types", async () => {
  const { connect, push } = pushConnect([]);
  const session = new WorldSession({ selfId: "u-self", connect, modules: [coreUiStub], logger: silentLogger });
  await session.enter("w1");
  push(welcomeFrame);
  await vi.waitFor(() => expect(session.role).toBe("player"));

  // An ACTOR owned by this player, permissions read-only: `owner` keeps its
  // provenance-only meaning off tokens, so no write. Mirrors the server.
  const actor = actorWith({ default: "observer" });
  actor.owner = "u-self";
  expect(session.canEdit(actor, "/system/hp")).toBe(false);
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
  await vi.waitFor(() => expect(sent.filter((m) => m.type === "scene_subscribe" && m.channel === "identity")).toHaveLength(1));
  const req = sent.find((m) => m.type === "scene_subscribe" && m.channel === "identity")!;
  // First frame resolves the underlying ws subscription + fires onUpdate.
  push({ type: "scene_derived", request_id: req.request_id, channel: "identity", computed_at_seq: 0, payload: {} });
  await vi.waitFor(() => expect(frames).toHaveLength(1));

  // A second Welcome (reconnect) must re-establish the subscription — and tear down
  // the prior one, so no duplicate server subscription leaks.
  push(welcomeFrame);
  await vi.waitFor(() => expect(sent.filter((m) => m.type === "scene_subscribe" && m.channel === "identity")).toHaveLength(2));
  await vi.waitFor(() =>
    expect(sent.some((m) => m.type === "scene_unsubscribe" && m.request_id === req.request_id)).toBe(true),
  );
});

test("sendChatMessage forwards ChatSendOptions verbatim into a send_message frame", async () => {
  const sent: Array<Record<string, unknown>> = [];
  const connect: Connect = (handlers) => {
    queueMicrotask(() => handlers.onMessage(JSON.stringify(welcomeFrame)));
    return Promise.resolve({ send: (d) => sent.push(JSON.parse(d)), close: () => handlers.onClose() });
  };
  const session = new WorldSession({ selfId: "u1", connect, modules: [coreUiStub], logger: silentLogger });
  await session.enter("w1");
  await vi.waitFor(() => expect(session.role).toBe("player"));

  void session.sendChatMessage({ channel: "general", content: "hi" });
  await vi.waitFor(() =>
    expect(sent.some((m) => m.type === "send_message" && m.channel === "general" && m.content === "hi")).toBe(true),
  );
});

test("the session subscribes to footprints itself and publishes each frame's resolved extents", async () => {
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

  // Nothing computes a footprint before the server states one.
  expect(session.footprints.token("tok1")).toBeNull();

  const req = await vi.waitFor(() => {
    const m = sent.find((f) => f.type === "scene_subscribe" && f.channel === "footprints");
    expect(m).toBeDefined();
    return m!;
  });
  push({
    type: "scene_derived",
    request_id: req.request_id,
    channel: "footprints",
    computed_at_seq: 0,
    payload: {
      scenes: [
        { scene: "scene-1", unit: { w: 173.2, h: 200 }, tokens: [{ token: "tok1", extent: { w: 346.4, h: 400 } }] },
      ],
    },
  });
  await vi.waitFor(() => expect(session.footprints.token("tok1")).toEqual({ w: 346.4, h: 400 }));
  expect(session.footprints.unit("scene-1")).toEqual({ w: 173.2, h: 200 });

  // Leaving drops the extents with the connection they came from.
  session.leave();
  expect(session.footprints.token("tok1")).toBeNull();
});

// Minimal SceneToolHost fake (mirrors @shadowcat/ui-kit's `fakeSceneHost` fixture, not
// exported from the ui-kit barrel — this package only needs the one method under test here).
function fakeMoveHost(): import("@shadowcat/render").SceneToolHost & {
  calls: Array<{ id: string; moverVision: unknown }>;
} {
  const calls: Array<{ id: string; moverVision: unknown }> = [];
  return {
    setActiveTool: () => {},
    snap: (p) => p,
    setSnapEnabled: () => {},
    setDraggingToken: () => {},
    previewOverlay: () => {},
    clearOverlay: () => {},
    gridDistance: () => 0,
    drawMeasure: () => {},
    clearMeasure: () => {},
    addPing: () => {},
    animateAlongPath: () => {},
    animateSamples: (id, _s, _d, _st, _sn, moverVision) => { calls.push({ id, moverVision }); },
    calls,
  };
}

function moveStreamFrame(
  scene: string,
  moverVision: unknown = null,
  truncated: boolean | null = false,
): Record<string, unknown> {
  return {
    type: "move_stream",
    request_id: "r1",
    token_id: "tok1",
    mover: "u1",
    scene,
    start_server_ms: 0,
    duration_ms: 1000,
    stop: [100, 0],
    samples: [{ t_ms: 0, pos: [0, 0] }, { t_ms: 500, pos: [100, 0] }],
    mover_vision: moverVision,
    cost: 2,
    truncated,
  };
}

test("onMoveStream forwards to sceneInteraction (incl. moverVision) for the active scene", async () => {
  const sent: Array<Record<string, unknown>> = [];
  const { connect, push } = pushConnect(sent);
  const gmFrame = { ...welcomeFrame, user_role: "gm" };
  const session = new WorldSession({ selfId: "u1", connect, modules: [coreUiStub], logger: silentLogger });
  await session.enter("w1");
  push(gmFrame); // GM auto-creates the active scene
  await vi.waitFor(() => expect(sceneCreates(sent).length).toBe(1));
  const sceneId = (
    (sceneCreates(sent)[0] as { ops: Array<{ doc?: { id?: string } }> }).ops.find((o) => o.doc)!.doc!.id
  ) as string;

  const host = fakeMoveHost();
  session.sceneInteraction.attach(host);
  const moverVision = [{ t_ms: 0, polygons: [[[0, 0], [20, 0], [20, 20]]] }];
  push(moveStreamFrame(sceneId, moverVision));
  await vi.waitFor(() => expect(host.calls).toHaveLength(1));
  expect(host.calls[0]).toEqual({
    id: "tok1",
    moverVision: [{ tMs: 0, polygons: [[[0, 0], [20, 0], [20, 20]]] }],
  });
});

test("onMoveStream for a non-active scene is ignored (fail-closed cross-scene guard)", async () => {
  const sent: Array<Record<string, unknown>> = [];
  const { connect, push } = pushConnect(sent);
  const gmFrame = { ...welcomeFrame, user_role: "gm" };
  const session = new WorldSession({ selfId: "u1", connect, modules: [coreUiStub], logger: silentLogger });
  await session.enter("w1");
  push(gmFrame);
  await vi.waitFor(() => expect(sceneCreates(sent).length).toBe(1));

  const host = fakeMoveHost();
  session.sceneInteraction.attach(host);
  push(moveStreamFrame("some-other-scene-id"));
  await new Promise((r) => setTimeout(r, 20));
  expect(host.calls).toHaveLength(0);
});

test("viewedSceneId: player follows activeScene, else the first scene", async () => {
  const sent: Array<Record<string, unknown>> = [];
  const { connect, push } = pushConnect(sent);
  const session = new WorldSession({ selfId: "u1", connect, modules: [coreUiStub], logger: silentLogger });
  await session.enter("w1");
  push(welcomeFrame); // player
  await vi.waitFor(() => expect(session.role).toBe("player"));

  // Predict two scenes + a world-settings doc into the optimistic view.
  session.dispatchIntent([{ op: "create", doc: buildSceneDoc("w1", {}, "s0") }]);
  session.dispatchIntent([{ op: "create", doc: buildSceneDoc("w1", {}, "s1") }]);
  expect(session.viewedSceneId).toBe("s0"); // no activeScene yet ⇒ first scene

  session.dispatchIntent([{ op: "create", doc: buildWorldSettingsDoc("w1", { ...structuredClone(DEFAULT_WORLD_SETTINGS), activeScene: "s1" }) }]);
  expect(session.viewedSceneId).toBe("s1"); // follows activeScene
});

test("setGmViewedScene overrides only for a GM; a player call is ignored", async () => {
  const sent: Array<Record<string, unknown>> = [];
  const { connect, push } = pushConnect(sent);
  const session = new WorldSession({ selfId: "u1", connect, modules: [coreUiStub], logger: silentLogger });
  await session.enter("w1");
  push(welcomeFrame); // player
  await vi.waitFor(() => expect(session.role).toBe("player"));
  session.dispatchIntent([{ op: "create", doc: buildSceneDoc("w1", {}, "s0") }]);
  session.dispatchIntent([{ op: "create", doc: buildSceneDoc("w1", {}, "s1") }]);
  session.dispatchIntent([{ op: "create", doc: buildWorldSettingsDoc("w1", { ...structuredClone(DEFAULT_WORLD_SETTINGS), activeScene: "s1" }) }]);

  session.setGmViewedScene("s0"); // player: ignored
  expect(session.viewedSceneId).toBe("s1");
});

test("a GM roams locally with gmViewedScene; clearing it follows activeScene again", async () => {
  const sent: Array<Record<string, unknown>> = [];
  const { connect, push } = pushConnect(sent);
  const session = new WorldSession({ selfId: "u1", connect, modules: [coreUiStub], logger: silentLogger });
  await session.enter("w1");
  push({ ...welcomeFrame, user_role: "gm" }); // GM auto-creates one scene
  await vi.waitFor(() => expect(sceneCreates(sent).length).toBe(1));
  const first = (sceneCreates(sent)[0] as { ops: Array<{ doc?: { id?: string } }> }).ops.find((o) => o.doc)!.doc!.id as string;
  session.dispatchIntent([{ op: "create", doc: buildSceneDoc("w1", {}, "sB") }]);
  session.dispatchIntent([{ op: "create", doc: buildWorldSettingsDoc("w1", { ...structuredClone(DEFAULT_WORLD_SETTINGS), activeScene: first }) }]);

  session.setGmViewedScene("sB");
  expect(session.viewedSceneId).toBe("sB"); // roaming
  session.setGmViewedScene(null);
  expect(session.viewedSceneId).toBe(first); // follows active again
});

test("setGmViewedScene scene-scopes tokenSelection instead of leaking or clearing it", async () => {
  const sent: Array<Record<string, unknown>> = [];
  const { connect, push } = pushConnect(sent);
  const session = new WorldSession({ selfId: "u1", connect, modules: [coreUiStub], logger: silentLogger });
  await session.enter("w1");
  push({ ...welcomeFrame, user_role: "gm" });
  await vi.waitFor(() => expect(sceneCreates(sent).length).toBe(1));
  const first = (sceneCreates(sent)[0] as { ops: Array<{ doc?: { id?: string } }> }).ops.find((o) => o.doc)!.doc!.id as string;
  session.dispatchIntent([{ op: "create", doc: buildSceneDoc("w1", {}, "sB") }]);

  // Select a token while viewing the first (auto-created, active-by-default) scene.
  session.tokenSelection.set(["tokA"]);
  expect(session.tokenSelection.has("tokA")).toBe(true);

  // Roam to sB: the selection must not leak (tokA lives in the first scene, not sB).
  session.setGmViewedScene("sB");
  expect(session.tokenSelection.ids.size).toBe(0);

  // Select a different token while viewing sB.
  session.tokenSelection.set(["tokB"]);

  // Roam back to the first scene: its stashed selection is restored, sB's is stashed.
  session.setGmViewedScene(first);
  expect(session.tokenSelection.has("tokA")).toBe(true);
  expect(session.tokenSelection.has("tokB")).toBe(false);

  // Roam to sB again: restores tokB, not tokA.
  session.setGmViewedScene("sB");
  expect(session.tokenSelection.has("tokB")).toBe(true);
  expect(session.tokenSelection.has("tokA")).toBe(false);

  // Roam to a never-visited third scene: empty, not a leak from either stash.
  session.dispatchIntent([{ op: "create", doc: buildSceneDoc("w1", {}, "sC") }]);
  session.setGmViewedScene("sC");
  expect(session.tokenSelection.ids.size).toBe(0);
});

test("onMoveStream gates on the GM's LOCAL viewed scene, not activeScene", async () => {
  const sent: Array<Record<string, unknown>> = [];
  const { connect, push } = pushConnect(sent);
  const session = new WorldSession({ selfId: "u1", connect, modules: [coreUiStub], logger: silentLogger });
  await session.enter("w1");
  push({ ...welcomeFrame, user_role: "gm" });
  await vi.waitFor(() => expect(sceneCreates(sent).length).toBe(1));
  const active = (sceneCreates(sent)[0] as { ops: Array<{ doc?: { id?: string } }> }).ops.find((o) => o.doc)!.doc!.id as string;
  session.dispatchIntent([{ op: "create", doc: buildSceneDoc("w1", {}, "sB") }]);
  session.dispatchIntent([{ op: "create", doc: buildWorldSettingsDoc("w1", { ...structuredClone(DEFAULT_WORLD_SETTINGS), activeScene: active }) }]);
  session.setGmViewedScene("sB"); // roaming to sB while players stay on `active`

  const host = fakeMoveHost();
  session.sceneInteraction.attach(host);
  push(moveStreamFrame(active)); // the players' scene — must NOT animate in the GM's local view
  await new Promise((r) => setTimeout(r, 20));
  expect(host.calls).toHaveLength(0);
  push(moveStreamFrame("sB")); // the GM's viewed scene — animates
  await vi.waitFor(() => expect(host.calls).toHaveLength(1));
});

test("onMoveOutcome: a move_stream whose stop matches the requested goal fires 'executed'", async () => {
  const sent: Array<Record<string, unknown>> = [];
  const { connect, push } = pushConnect(sent);
  const session = new WorldSession({ selfId: "u1", connect, modules: [coreUiStub], logger: silentLogger });
  await session.enter("w1");
  push(welcomeFrame);
  await vi.waitFor(() => expect(session.role).toBe("player"));

  const got: Array<{ tokenId: string; outcome: string }> = [];
  session.onMoveOutcome((m) => got.push(m));
  const resolved = session.moveRequest("s1", "tok1", [[0, 0], [100, 0]]);
  await vi.waitFor(() => expect(sent.some((m) => m.type === "move_request")).toBe(true));
  const reqId = sent.find((m) => m.type === "move_request")!.request_id as string;
  push({ ...moveStreamFrame("s1"), request_id: reqId, stop: [100, 0] });
  await resolved;
  await vi.waitFor(() => expect(got).toHaveLength(1));
  expect(got[0]).toEqual({ tokenId: "tok1", outcome: "executed" });
});

test("onMoveOutcome: a region arrest landing exactly on the requested goal still fires 'executed'", async () => {
  // `execute_move`: an arrest region stops the walk AT cell entry (stop_index == path.len()-1
  // on a final-step arrest), so `MoveOutcome.truncated = true` server-side even though `stop`
  // equals the requested goal exactly — the token DID reach the goal; only further movement
  // from there is barred. Since `truncated` never crosses the wire, this locks in that the
  // stop-vs-goal comparison intentionally reads this case as "executed", not "truncated" —
  // documented here so a future reader knows it's by design, not an oversight.
  const sent: Array<Record<string, unknown>> = [];
  const { connect, push } = pushConnect(sent);
  const session = new WorldSession({ selfId: "u1", connect, modules: [coreUiStub], logger: silentLogger });
  await session.enter("w1");
  push(welcomeFrame);
  await vi.waitFor(() => expect(session.role).toBe("player"));

  const got: Array<{ tokenId: string; outcome: string }> = [];
  session.onMoveOutcome((m) => got.push(m));
  const resolved = session.moveRequest("s1", "tok1", [[0, 0], [50, 0], [100, 0]]);
  await vi.waitFor(() => expect(sent.some((m) => m.type === "move_request")).toBe(true));
  const reqId = sent.find((m) => m.type === "move_request")!.request_id as string;
  // stop exactly at the requested goal, as an arrest-on-final-cell outcome would report.
  push({ ...moveStreamFrame("s1", null, true), request_id: reqId, stop: [100, 0] });
  await resolved;
  await vi.waitFor(() => expect(got).toHaveLength(1));
  expect(got[0]).toEqual({ tokenId: "tok1", outcome: "executed" });
});

test("onMoveOutcome: a move_stream whose stop falls short of the requested goal fires 'truncated'", async () => {
  const sent: Array<Record<string, unknown>> = [];
  const { connect, push } = pushConnect(sent);
  const session = new WorldSession({ selfId: "u1", connect, modules: [coreUiStub], logger: silentLogger });
  await session.enter("w1");
  push(welcomeFrame);
  await vi.waitFor(() => expect(session.role).toBe("player"));

  const got: Array<{ tokenId: string; outcome: string }> = [];
  session.onMoveOutcome((m) => got.push(m));
  const resolved = session.moveRequest("s1", "tok1", [[0, 0], [100, 0]]);
  await vi.waitFor(() => expect(sent.some((m) => m.type === "move_request")).toBe(true));
  const reqId = sent.find((m) => m.type === "move_request")!.request_id as string;
  // The server stopped short of the requested goal (wall/mask/region gate) — no separate
  // `truncated` flag rides the wire, so the client infers it from the stop mismatch.
  push({ ...moveStreamFrame("s1", null, true), request_id: reqId, stop: [50, 0] });
  await resolved;
  await vi.waitFor(() => expect(got).toHaveLength(1));
  expect(got[0]).toEqual({ tokenId: "tok1", outcome: "truncated" });
});

test("onMoveOutcome: a move_error rejection fires 'rejected'", async () => {
  const sent: Array<Record<string, unknown>> = [];
  const { connect, push } = pushConnect(sent);
  const session = new WorldSession({ selfId: "u1", connect, modules: [coreUiStub], logger: silentLogger });
  await session.enter("w1");
  push(welcomeFrame);
  await vi.waitFor(() => expect(session.role).toBe("player"));

  const got: Array<{ tokenId: string; outcome: string }> = [];
  session.onMoveOutcome((m) => got.push(m));
  const resolved = session.moveRequest("s1", "tok1", [[0, 0], [100, 0]]);
  resolved.catch(() => {}); // rejection is expected; assert via onMoveOutcome, not the promise
  await vi.waitFor(() => expect(sent.some((m) => m.type === "move_request")).toBe(true));
  const reqId = sent.find((m) => m.type === "move_request")!.request_id as string;
  push({ type: "move_error", request_id: reqId, message: "token is moving" });
  await vi.waitFor(() => expect(got).toHaveLength(1));
  expect(got[0]).toEqual({ tokenId: "tok1", outcome: "rejected" });
});

test("sendPing targets the viewed scene", async () => {
  const sent: Array<Record<string, unknown>> = [];
  const { connect, push } = pushConnect(sent);
  const session = new WorldSession({ selfId: "u1", connect, modules: [coreUiStub], logger: silentLogger });
  await session.enter("w1");
  push({ ...welcomeFrame, user_role: "gm" });
  await vi.waitFor(() => expect(sceneCreates(sent).length).toBe(1));
  const active = (sceneCreates(sent)[0] as { ops: Array<{ doc?: { id?: string } }> }).ops.find((o) => o.doc)!.doc!.id as string;
  session.dispatchIntent([{ op: "create", doc: buildSceneDoc("w1", {}, "sB") }]);
  session.dispatchIntent([{ op: "create", doc: buildWorldSettingsDoc("w1", { ...structuredClone(DEFAULT_WORLD_SETTINGS), activeScene: active }) }]);
  session.setGmViewedScene("sB");

  session.sendPing(10, 20);
  const ping = sent.find((m) => m.type === "scene_ping")!;
  expect(ping.scene).toBe("sB");
});

test("role demotion mid-session excludes a stale gmViewedScene the instant role flips", async () => {
  const sent: Array<Record<string, unknown>> = [];
  const { connect, push } = pushConnect(sent);
  const session = new WorldSession({ selfId: "u1", connect, modules: [coreUiStub], logger: silentLogger });
  await session.enter("w1");
  push({ ...welcomeFrame, user_role: "gm" }); // GM auto-creates one scene
  await vi.waitFor(() => expect(sceneCreates(sent).length).toBe(1));
  const first = (sceneCreates(sent)[0] as { ops: Array<{ doc?: { id?: string } }> }).ops.find((o) => o.doc)!.doc!.id as string;
  session.dispatchIntent([{ op: "create", doc: buildSceneDoc("w1", {}, "sB") }]);
  session.setGmViewedScene("sB");
  expect(session.viewedSceneId).toBe("sB"); // roaming as GM

  // A second Welcome demotes the caller to player (e.g. GM role revoked mid-session).
  push({ ...welcomeFrame, user_role: "player" });
  await vi.waitFor(() => expect(session.role).toBe("player"));
  // The stale roam value must be excluded the instant role flips — no activeScene is set,
  // so the player follows the first scene, not the GM's abandoned roam target.
  expect(session.viewedSceneId).toBe(first);
});

test("onScenePing cross-scene guard: a GM roaming scene B sees own pings for B, drops a ping for A", async () => {
  const sent: Array<Record<string, unknown>> = [];
  const { connect, push } = pushConnect(sent);
  const session = new WorldSession({ selfId: "u1", connect, modules: [coreUiStub], logger: silentLogger });
  await session.enter("w1");
  push({ ...welcomeFrame, user_role: "gm" });
  await vi.waitFor(() => expect(sceneCreates(sent).length).toBe(1));
  const sceneA = (sceneCreates(sent)[0] as { ops: Array<{ doc?: { id?: string } }> }).ops.find((o) => o.doc)!.doc!.id as string;
  session.dispatchIntent([{ op: "create", doc: buildSceneDoc("w1", {}, "sceneB") }]);
  session.dispatchIntent([{ op: "create", doc: buildWorldSettingsDoc("w1", { ...structuredClone(DEFAULT_WORLD_SETTINGS), activeScene: sceneA }) }]);
  session.setGmViewedScene("sceneB"); // roaming B while players stay on A

  const got: Array<{ scene: string }> = [];
  session.onPing((m) => got.push(m));

  push({ type: "scene_ping", scene: sceneA, x: 1, y: 2, user: "u9" }); // players' scene — dropped
  await new Promise((r) => setTimeout(r, 20));
  expect(got).toHaveLength(0);

  push({ type: "scene_ping", scene: "sceneB", x: 3, y: 4, user: "u1" }); // the GM's own viewed scene — accepted
  await vi.waitFor(() => expect(got).toHaveLength(1));
  expect(got[0].scene).toBe("sceneB");
});

test("Welcome warns (but still enters the world) when an enabled id is not installed", async () => {
  const core = await import("@shadowcat/core");
  vi.mocked(core.getEnabledModules).mockResolvedValueOnce(["missing-mod"]);
  vi.mocked(core.listInstalledModules).mockResolvedValueOnce([]);
  const warnings: unknown[][] = [];
  const logger = { ...silentLogger, warn: (...args: unknown[]) => warnings.push(args) };
  const session = new WorldSession({
    selfId: "u1",
    connect: mockConnect(),
    modules: [coreUiStub],
    logger,
  });
  await session.enter("w1");
  await vi.waitFor(() => expect(session.role).toBe("player"));
  await vi.waitFor(() =>
    expect(warnings.some((a) => String(a[0]).includes("missing-mod"))).toBe(true),
  );
});

test("resolves an enabled folder id to its installed entry even when the manifest declares a different id", async () => {
  // Regression: the enabled-module set is keyed on the install FOLDER
  // id, never the manifest's author-declared id — the two may legitimately
  // differ. A lookup keyed on `manifest.id` would (wrongly) treat this
  // entry as "not installed" and skip it.
  const core = await import("@shadowcat/core");
  vi.mocked(core.getEnabledModules).mockResolvedValueOnce(["folder-name"]);
  vi.mocked(core.listInstalledModules).mockResolvedValueOnce([
    {
      id: "folder-name",
      manifest: { id: "declared-manifest-id", version: "1.0.0", dependencies: {}, provides: [] },
      entry_url: "/modules/folder-name/index.js",
    },
  ]);
  const warnings: unknown[][] = [];
  const logger = { ...silentLogger, warn: (...args: unknown[]) => warnings.push(args) };
  const session = new WorldSession({
    selfId: "u1",
    connect: mockConnect(),
    modules: [coreUiStub],
    logger,
  });
  await session.enter("w1");
  await vi.waitFor(() => expect(session.role).toBe("player"));
  await new Promise((r) => setTimeout(r, 20));

  // Must NOT have been skipped as "not installed" — it was found (by folder
  // id) and handed to the loader (which then fails on the fake entry URL,
  // a separate/expected warning distinct from the lookup-miss one below).
  expect(
    warnings.some((a) => String(a[0]).includes("folder-name") && String(a[0]).includes("is not installed")),
  ).toBe(false);
});

test("Welcome degrades gracefully when external module discovery fails (network error)", async () => {
  const core = await import("@shadowcat/core");
  vi.mocked(core.getEnabledModules).mockRejectedValueOnce(new Error("network down"));
  const warnings: unknown[][] = [];
  const logger = { ...silentLogger, warn: (...args: unknown[]) => warnings.push(args) };
  const session = new WorldSession({
    selfId: "u1",
    connect: mockConnect(),
    modules: [coreUiStub],
    logger,
  });
  await session.enter("w1");
  // The session still enters the world normally despite the discovery failure.
  await vi.waitFor(() => expect(session.role).toBe("player"));
  await vi.waitFor(() =>
    expect(
      warnings.some((a) => String(a[0]).includes("external module discovery failed")),
    ).toBe(true),
  );
});

test("unsubscribing a scene sub during the Welcome await window is not re-established (no spurious re-subscribe)", async () => {
  let resolveMembers!: () => void;
  vi.mocked(listWorldMembers).mockImplementationOnce(
    () => new Promise((resolve) => { resolveMembers = () => resolve([]); }),
  );
  const sent: Array<Record<string, unknown>> = [];
  const { connect, push } = pushConnect(sent);
  const session = new WorldSession({ selfId: "u1", connect, modules: [coreUiStub], logger: silentLogger });
  await session.enter("w1");
  push(welcomeFrame);
  await vi.waitFor(() => expect(session.role).toBe("player"));

  const frames: unknown[] = [];
  const sub = session.subscribeScene("identity", (f) => frames.push(f));
  await vi.waitFor(() => expect(sent.filter((m) => m.type === "scene_subscribe" && m.channel === "identity")).toHaveLength(1));

  // Reconnect Welcome: bootstrap already ran, so the only remaining await before the
  // scene-subs reconciliation loop is the member fetch, now held pending.
  push(welcomeFrame);
  await Promise.resolve();
  await Promise.resolve();
  // Torn down DURING the Welcome's await window — the reconciliation loop (running
  // against a pre-await snapshot) must not resurrect this entry.
  sub.unsubscribe();
  resolveMembers();
  await new Promise((r) => setTimeout(r, 20));

  // Only the original subscribe; no re-subscribe was sent for the torn-down sub.
  expect(sent.filter((m) => m.type === "scene_subscribe" && m.channel === "identity")).toHaveLength(1);
});

test("a scene sub added during the Welcome await window is established exactly once", async () => {
  let resolveMembers!: () => void;
  vi.mocked(listWorldMembers).mockImplementationOnce(
    () => new Promise((resolve) => { resolveMembers = () => resolve([]); }),
  );
  const sent: Array<Record<string, unknown>> = [];
  const { connect, push } = pushConnect(sent);
  const session = new WorldSession({ selfId: "u1", connect, modules: [coreUiStub], logger: silentLogger });
  await session.enter("w1");
  push(welcomeFrame);
  await vi.waitFor(() => expect(session.role).toBe("player"));

  // Reconnect Welcome; the member fetch is held pending so the reconciliation loop
  // (over the pre-await snapshot) has not yet run.
  push(welcomeFrame);
  await Promise.resolve();
  await Promise.resolve();

  const frames: unknown[] = [];
  session.subscribeScene("identity", (f) => frames.push(f)); // added mid-flight
  resolveMembers();
  await new Promise((r) => setTimeout(r, 20));

  // subscribeScene's own establish call is the only scene_subscribe sent — the
  // reconciliation loop must not double-establish a sub added mid-flight.
  expect(sent.filter((m) => m.type === "scene_subscribe" && m.channel === "identity")).toHaveLength(1);
});

test("an enabled set with nothing to load never calls listInstalledModules a second time on reconnect", async () => {
  const core = await import("@shadowcat/core");
  // This suite has no `clearMocks` config, so the shared `getEnabledModules` mock's
  // call count otherwise carries over from every earlier test that also calls
  // `session.enter()`; clear it here so the assertion below reflects only this
  // session's calls, not order-dependent accumulation.
  vi.mocked(core.getEnabledModules).mockClear();
  const session = new WorldSession({
    selfId: "u1",
    connect: mockConnect(2), // Welcome delivered twice (reconnect)
    modules: [coreUiStub],
    logger: silentLogger,
  });
  await session.enter("w1");
  await vi.waitFor(() => expect(session.role).toBe("player"));
  await vi.waitFor(() => expect(vi.mocked(core.getEnabledModules)).toHaveBeenCalledTimes(1));
  await Promise.resolve();
  // External-module loading is bootstrap-once, exactly like core-ui activation.
  expect(vi.mocked(core.getEnabledModules)).toHaveBeenCalledTimes(1);
});
