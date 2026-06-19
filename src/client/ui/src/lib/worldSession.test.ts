import { test, expect, vi } from "vitest";
import { ContributionRegistry, type Connect } from "@shadowcat/core";
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

function mockConnect(): Connect {
  return (handlers) => {
    queueMicrotask(() => handlers.onMessage(JSON.stringify(welcomeFrame)));
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
  });

  await session.enter("w1");
  // Welcome arrives on a microtask after connect; poll until handled.
  await vi.waitFor(() => expect(session.role).toBe("player"));
  await vi.waitFor(() => expect(coreUiStub.register).toHaveBeenCalledOnce());
  expect(session.contributions).toBeInstanceOf(ContributionRegistry);

  session.leave();
  expect(session.state).toBe("closed");
});
