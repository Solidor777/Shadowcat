import { describe, it, expect, test, vi, afterEach } from "vitest";
import { render } from "@testing-library/svelte";
import { silentLogger, type Connect, type SheetRef } from "@shadowcat/core";
import type { AppContext } from "@shadowcat/ui-kit";
import * as api from "./api";
import * as route from "./route.svelte";
import * as sessionState from "./sessionState.svelte";
import { WorldSession } from "./worldSession.svelte";
import Table from "./Table.svelte";
import AppContextCapture from "./__fixtures__/AppContextCapture.svelte";

// The AppContext type must carry `openDocument`; a compile-time surface check that the
// seam exists with the right shape (runtime wiring is exercised by the ui-kit
// SheetsController tests + the panels e2e).
describe("AppContext.openDocument seam", () => {
  it("accepts docId and tokenId refs", () => {
    const refs: SheetRef[] = [{ docId: "d1" }, { docId: "d1", embeddedPath: "/embedded/item/0" }, { tokenId: "t1" }];
    const fn: AppContext["openDocument"] = () => {};
    for (const r of refs) fn(r);
    expect(refs).toHaveLength(3);
  });
});

// The members fetch and external-module discovery fetches hit the network; stub them
// (mirrors worldSession.test.ts's own mock, needed here for the same reason).
vi.mock("@shadowcat/core", async (importActual) => {
  const actual = await importActual<typeof import("@shadowcat/core")>();
  return {
    ...actual,
    listWorldMembers: vi.fn().mockResolvedValue([]),
    listInstalledModules: vi.fn().mockResolvedValue([]),
    getEnabledModules: vi.fn().mockResolvedValue([]),
  };
});

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

function mockConnect(): Connect {
  return (handlers) => {
    queueMicrotask(() => handlers.onMessage(JSON.stringify(welcomeFrame)));
    return Promise.resolve({ send: () => {}, close: () => handlers.onClose() });
  };
}

afterEach(() => vi.restoreAllMocks());

test("the logout handler logs out, resets session state, then navigates in that order", async () => {
  const session = new WorldSession({
    selfId: "u1",
    connect: mockConnect(),
    modules: [],
    logger: silentLogger,
  });
  await session.enter("w1");
  await vi.waitFor(() => expect(session.role).toBe("player"));

  let ctx!: AppContext;
  session.contributions.contribute({
    id: "app-context-capture",
    contract: "shadowcat.surface:root",
    component: AppContextCapture,
    props: { onContext: (c: AppContext) => { ctx = c; } },
  });

  render(Table, { props: { session, leaveWorld: vi.fn(), serverRole: "user" } });
  await vi.waitFor(() => expect(ctx).toBeTruthy());

  const order: string[] = [];
  const logoutSpy = vi.spyOn(api, "logout").mockImplementation(async () => {
    order.push("logout");
  });
  const resetSpy = vi.spyOn(sessionState, "resetSessionState").mockImplementation(() => {
    order.push("reset");
  });
  const navigateSpy = vi.spyOn(route, "navigate").mockImplementation(() => {
    order.push("navigate");
  });

  await ctx.logout();

  expect(logoutSpy).toHaveBeenCalledOnce();
  expect(resetSpy).toHaveBeenCalledOnce();
  expect(navigateSpy).toHaveBeenCalledWith({ name: "login" });
  // Order matters: the server-side session cookie is invalidated (logout) before
  // sessionState is reset, and reset happens before navigation so no stray mutation
  // slips in during the transition.
  expect(order).toEqual(["logout", "reset", "navigate"]);

  session.leave();
});
