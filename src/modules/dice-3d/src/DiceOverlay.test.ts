import { describe, it, expect, vi } from "vitest";
import { render } from "@testing-library/svelte";
import { setAppContextForTest } from "@shadowcat/ui-kit/test";
import { Dice3DBridge } from "@shadowcat/ui-kit";
import { DocumentStore, type WireDocument } from "@shadowcat/core";
import DiceOverlay from "./DiceOverlay.svelte";

vi.mock("./DiceEngine", () => ({
  DiceEngine: vi.fn().mockImplementation(() => ({
    init: vi.fn().mockResolvedValue(undefined),
    throwDice: vi.fn().mockResolvedValue([]),
    resize: vi.fn(),
    dispose: vi.fn(),
  })),
}));

function messageDoc(id: string, rollId: string): WireDocument {
  return {
    id, scope: { kind: "world", world_id: "w1" }, doc_type: "message", schema_version: 1,
    name: null, source: null, owner: "u1",
    permissions: { default: "observer", users: {} } as WireDocument["permissions"],
    embedded: {}, parent_id: null,
    engine: {
      channel: "general", user_owner: "u1", kind: "roll", audience: { kind: "public" },
      content: [{ kind: "roll_embed", formula: "1d20", roll_id: rollId, outcome: {
        total: 1, records: [{ value: 1, natural: 1, kept: true, exploded: false, crit_success: false, crit_fail: false, expertise: 0, group_index: 0, symbols: [], kind: { Numeric: { min: 1, max: 20 } } }],
        crit_successes: 0, crit_fails: 0, positive_counter: 0, negative_counter: 0, symbol_counts: {}, labeled_consts: [],
      } }],
    },
    system: {}, created_at: 1, updated_at: 1,
  };
}

describe("DiceOverlay", () => {
  it("registers a data-dice3d-state host attribute, idle at mount with no rolls", () => {
    const { container } = render(DiceOverlay, { context: setAppContextForTest({}) });
    const el = container.querySelector(".dice3d-overlay");
    expect(el?.getAttribute("data-dice3d-state")).toBe("idle");
  });

  it("a message present at mount never plays (stays idle)", async () => {
    const store = new DocumentStore();
    store.applyCommand({ seq: 1, world_id: "w1", author: "a", ts: 0, ops: [{ op: "create", doc: messageDoc("m1", "r1") }] });
    const { container } = render(DiceOverlay, { context: setAppContextForTest({ documents: store }) });
    await Promise.resolve();
    expect(container.querySelector(".dice3d-overlay")?.getAttribute("data-dice3d-state")).toBe("idle");
  });

  it("a message arriving after mount transitions to tumbling then settled", async () => {
    const store = new DocumentStore();
    const { container } = render(DiceOverlay, { context: setAppContextForTest({ documents: store }) });
    store.applyCommand({ seq: 1, world_id: "w1", author: "a", ts: 0, ops: [{ op: "create", doc: messageDoc("m1", "r1") }] });
    await new Promise((r) => setTimeout(r, 0));
    const el = container.querySelector(".dice3d-overlay");
    expect(["tumbling", "settled"]).toContain(el?.getAttribute("data-dice3d-state"));
  });

  it("AppContext.dice3d.roll attaches to this overlay instance", async () => {
    const bridge = new Dice3DBridge();
    render(DiceOverlay, { context: setAppContextForTest({ dice3d: bridge }) });
    await Promise.resolve();
    expect(() => bridge.roll({
      total: 1, records: [], crit_successes: 0, crit_fails: 0, positive_counter: 0,
      negative_counter: 0, symbol_counts: {}, labeled_consts: [],
    }, "r-ext")).not.toThrow();
  });
});
