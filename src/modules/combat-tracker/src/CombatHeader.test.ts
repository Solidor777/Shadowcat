import { describe, it, expect, vi, afterEach } from "vitest";
import { render, fireEvent, cleanup } from "@testing-library/svelte";
import { setAppContextForTest } from "@shadowcat/ui-kit/test";
import { DocumentStore, buildCombatDoc, newCombatEngine, buildChannelRegistryDoc, type WireDocument } from "@shadowcat/core";
import CombatHeader from "./CombatHeader.svelte";
import { fakeCombatApi } from "./__fixtures__/fakeCombatApi";
import type { Row } from "./model";

afterEach(() => cleanup());

function renderHeader(opts: {
  combat: WireDocument;
  store: DocumentStore;
  role?: "gm" | "player";
  selfId?: string;
  busy?: boolean;
  canAct?: Parameters<ReturnType<typeof fakeCombatApi>["setCanAct"]>[0];
  rows?: Row[];
  panelsOpen?: (id: string) => void;
  notify?: (m: string, level?: string) => void;
}) {
  const combatApi = fakeCombatApi(opts.store, { role: opts.role ?? "gm", selfId: opts.selfId });
  if (opts.canAct) combatApi.setCanAct(opts.canAct);
  const utils = render(CombatHeader, {
    props: {
      combat: opts.combat,
      rows: opts.rows ?? [],
      busy: opts.busy ?? false,
      run: async (fn: () => Promise<void>) => {
        await fn();
      },
      notation: "1d20",
    },
    context: setAppContextForTest({
      store: opts.store,
      documents: opts.store,
      combat: combatApi,
      role: opts.role ?? "gm",
      selfId: opts.selfId ?? "u-self",
      notify: opts.notify as never,
      panels: opts.panelsOpen
        ? ({ open: opts.panelsOpen, close: () => {}, focus: () => {}, toggle: () => {}, minimized: [], metaMap: new Map(), restore: () => {} } as never)
        : undefined,
    }),
  });
  return { ...utils, combatApi };
}

function combat(id = "c1", active = false): WireDocument {
  const engine = newCombatEngine("scene-1");
  engine.active = active;
  return buildCombatDoc("w1", engine, id);
}

describe("CombatHeader control visibility", () => {
  it("GM sees start/pause/end/advance/rewind/sort/settings, and delete when inactive", () => {
    const store = new DocumentStore();
    const c = combat("c1", false);
    store.applyCommand({ seq: 1, world_id: "w1", author: "a", ts: 0, ops: [{ op: "create", doc: c }] });
    const { queryByText } = renderHeader({ combat: c, store, role: "gm", canAct: { rewind: false } });
    expect(queryByText("combatTracker.start")).toBeTruthy();
    expect(queryByText("combatTracker.pause")).toBeTruthy();
    expect(queryByText("combatTracker.advance")).toBeTruthy();
    expect(queryByText("combatTracker.rewind")).toBeNull(); // CombatApi.canAct gates rewind on round > 0
    expect(queryByText("combatTracker.sort")).toBeTruthy();
    expect(queryByText("combatTracker.settings")).toBeTruthy();
    expect(queryByText("combatTracker.delete")).toBeTruthy();
  });

  it("a non-owner player sees no advance/end-my-turn control", () => {
    const store = new DocumentStore();
    const c = combat("c1", true);
    store.applyCommand({ seq: 1, world_id: "w1", author: "a", ts: 0, ops: [{ op: "create", doc: c }] });
    const { queryByTestId, queryByText } = renderHeader({
      combat: c, store, role: "player", canAct: { advance: false, start: false, pause: false, end: false, rewind: false, sort: false, edit: false },
    });
    expect(queryByTestId("combat-tracker:end-my-turn")).toBeNull();
    expect(queryByText("combatTracker.settings")).toBeNull();
    expect(queryByText("combatTracker.delete")).toBeNull();
  });

  it("an owner-on-turn player under owner_may_end sees End my turn, not Advance", () => {
    const store = new DocumentStore();
    const c = combat("c1", true);
    store.applyCommand({ seq: 1, world_id: "w1", author: "a", ts: 0, ops: [{ op: "create", doc: c }] });
    const { queryByTestId, queryByText } = renderHeader({
      combat: c, store, role: "player", canAct: { advance: true },
    });
    expect(queryByTestId("combat-tracker:end-my-turn")).toBeTruthy();
    expect(queryByText("combatTracker.advance")).toBeNull();
  });
});

describe("CombatHeader controls dispatch through CombatApi", () => {
  it("each clock control calls the matching CombatApi method with the combat id", async () => {
    const store = new DocumentStore();
    const c = combat("c1", false);
    store.applyCommand({ seq: 1, world_id: "w1", author: "a", ts: 0, ops: [{ op: "create", doc: c }] });
    const { getByText, combatApi } = renderHeader({ combat: c, store, role: "gm" });
    await fireEvent.click(getByText("combatTracker.start"));
    expect(combatApi.calls.start).toEqual([["c1"]]);
    await fireEvent.click(getByText("combatTracker.pause"));
    expect(combatApi.calls.pause).toEqual([["c1"]]);
    await fireEvent.click(getByText("combatTracker.advance"));
    expect(combatApi.calls.advance).toEqual([["c1"]]);
    await fireEvent.click(getByText("combatTracker.sort"));
    expect(combatApi.calls.sort).toEqual([["c1"]]);
  });

  it("End requires two clicks within the confirm window", async () => {
    const store = new DocumentStore();
    const c = combat("c1", true);
    store.applyCommand({ seq: 1, world_id: "w1", author: "a", ts: 0, ops: [{ op: "create", doc: c }] });
    const { getByTestId, combatApi } = renderHeader({ combat: c, store, role: "gm" });
    const button = getByTestId("combat-tracker:end");
    await fireEvent.click(button);
    expect(combatApi.calls.end).toBeUndefined();
    expect(button.textContent).toBe("combatTracker.endConfirm");
    await fireEvent.click(button);
    expect(combatApi.calls.end).toEqual([["c1"]]);
  });

  it("Delete calls deleteCombat", async () => {
    const store = new DocumentStore();
    const c = combat("c1", false);
    store.applyCommand({ seq: 1, world_id: "w1", author: "a", ts: 0, ops: [{ op: "create", doc: c }] });
    const { getByText, combatApi } = renderHeader({ combat: c, store, role: "gm" });
    await fireEvent.click(getByText("combatTracker.delete"));
    expect(combatApi.calls.deleteCombat).toEqual([["c1"]]);
  });

  it("Settings… opens the game-settings panel", async () => {
    const store = new DocumentStore();
    const c = combat("c1", false);
    store.applyCommand({ seq: 1, world_id: "w1", author: "a", ts: 0, ops: [{ op: "create", doc: c }] });
    const panelsOpen = vi.fn();
    const { getByText } = renderHeader({ combat: c, store, role: "gm", panelsOpen });
    await fireEvent.click(getByText("combatTracker.settings"));
    expect(panelsOpen).toHaveBeenCalledWith("game-settings:panel");
  });

  it("busy disables every button", () => {
    const store = new DocumentStore();
    const c = combat("c1", false);
    store.applyCommand({ seq: 1, world_id: "w1", author: "a", ts: 0, ops: [{ op: "create", doc: c }] });
    const { container } = renderHeader({ combat: c, store, role: "gm", busy: true });
    const buttons = Array.from(container.querySelectorAll("button")) as HTMLButtonElement[];
    // Settings… is not gated by busy (it only navigates the panel host, no intent).
    const gated = buttons.filter((b) => b.textContent !== "combatTracker.settings");
    expect(gated.length).toBeGreaterThan(0);
    for (const b of gated) expect(b.disabled).toBe(true);
  });

  it("Roll all builds one roll entry per target with the notation and the registry's first channel", async () => {
    const store = new DocumentStore();
    const c = combat("c1", false);
    store.applyCommand({
      seq: 1, world_id: "w1", author: "a", ts: 0,
      ops: [{ op: "create", doc: c }, { op: "create", doc: buildChannelRegistryDoc("w1", { general: { name: "General" } }) }],
    });
    const row: Row = {
      doc: { ...c, id: "combatant-1", doc_type: "combatant", engine: { kind: { type: "actor", token_id: null, actor_id: null }, initiative: null, tiebreak: 0, resources: {} }, owner: "u-self" },
      kind: "actor", view: null, name: "Combatant 1", art: {},
    };
    const { getByTestId, combatApi } = renderHeader({ combat: c, store, role: "gm", rows: [row] });
    await fireEvent.click(getByTestId("combat-tracker:roll-all"));
    expect(combatApi.calls.roll).toEqual([["c1", "general", [{ combatant_id: "combatant-1", notation: "1d20" }]]]);
  });

  it("Roll all notifies instead of rolling when no channel is configured", async () => {
    const store = new DocumentStore();
    const c = combat("c1", false);
    store.applyCommand({ seq: 1, world_id: "w1", author: "a", ts: 0, ops: [{ op: "create", doc: c }] });
    const notify = vi.fn();
    const { getByTestId, combatApi } = renderHeader({ combat: c, store, role: "gm", notify });
    await fireEvent.click(getByTestId("combat-tracker:roll-all"));
    expect(combatApi.calls.roll).toBeUndefined();
    expect(notify).toHaveBeenCalledWith("combatTracker.noChannel", "warning");
  });
});
