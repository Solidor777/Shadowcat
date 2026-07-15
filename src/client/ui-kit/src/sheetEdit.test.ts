import { describe, it, expect } from "vitest";
import { setField } from "./sheetEdit";
import type { AppContext } from "./appContext";

function makeCtx(): { ctx: AppContext; calls: unknown[] } {
  const calls: unknown[] = [];
  const c = { dispatchIntent: (ops: unknown) => calls.push(ops) } as unknown as AppContext;
  return { ctx: c, calls };
}

describe("setField", () => {
  it("dispatches one update op carrying the REAL pre-image as old", () => {
    const { ctx, calls } = makeCtx();
    setField(ctx, "d1", "/system/name", "Goblin", "Hobgoblin");
    expect(calls).toEqual([[{ op: "update", doc_id: "d1", changes: [{ path: "/system/name", old: "Goblin", new: "Hobgoblin" }] }]]);
  });

  it("passes old: null ONLY when the pre-image is genuinely absent (undefined)", () => {
    const { ctx, calls } = makeCtx();
    setField(ctx, "d1", "/system/newField", undefined, 3);
    expect((calls[0] as { changes: { old: unknown }[] }[])[0].changes[0].old).toBeNull();
  });

  it("preserves a falsy real pre-image (0 / false / '') as old, not null", () => {
    const { ctx, calls } = makeCtx();
    setField(ctx, "d1", "/system/hp", 0, 5);
    setField(ctx, "d1", "/system/flag", false, true);
    const olds = (calls as { changes: { old: unknown }[] }[][]).map((c) => c[0].changes[0].old);
    expect(olds).toEqual([0, false]);
  });
});
