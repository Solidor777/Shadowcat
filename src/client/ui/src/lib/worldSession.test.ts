import { test, expect, vi } from "vitest";
import { ContributionRegistry, silentLogger, type Connect } from "@shadowcat/core";
import { WorldSession } from "./worldSession.svelte";

// `MockServer` is internal core test code (not barrel-exported), so use a minimal
// inline Connect that delivers one valid Welcome frame on connect and ignores
// sends. The frame must satisfy parseServerMsg (all welcome fields present).
const welcomeFrame = {
  type: "welcome",
  world: "w1",
  current_seq: 0,
  server_time: 0,
  world_default_grants: { by_role: {}, by_user: {} },
  actor_role: "player",
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
    coreUiModule: coreUiStub,
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
    coreUiModule: coreUiStub,
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
    coreUiModule: coreUiStub,
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

test("subscribeScene sends scene_subscribe and re-establishes on a reconnect Welcome", async () => {
  let push!: (frame: unknown) => void;
  const sent: Array<Record<string, unknown>> = [];
  const connect: Connect = (handlers) => {
    push = (frame) => handlers.onMessage(JSON.stringify(frame));
    queueMicrotask(() => push(welcomeFrame));
    return Promise.resolve({ send: (d) => sent.push(JSON.parse(d)), close: () => handlers.onClose() });
  };
  const session = new WorldSession({ selfId: "u1", connect, coreUiModule: coreUiStub, logger: silentLogger });
  await session.enter("w1");
  await vi.waitFor(() => expect(session.role).toBe("player"));

  const frames: unknown[] = [];
  session.subscribeScene("identity", (f) => frames.push(f));
  await vi.waitFor(() => expect(sent.filter((m) => m.type === "scene_subscribe")).toHaveLength(1));
  const req = sent.find((m) => m.type === "scene_subscribe")!;
  // First frame resolves the underlying ws subscription + fires onUpdate.
  push({ type: "scene_derived", request_id: req.request_id, channel: "identity", computed_at_seq: 0, payload: {} });
  await vi.waitFor(() => expect(frames).toHaveLength(1));

  // A second Welcome (reconnect) must re-establish the subscription.
  push(welcomeFrame);
  await vi.waitFor(() => expect(sent.filter((m) => m.type === "scene_subscribe")).toHaveLength(2));
});
