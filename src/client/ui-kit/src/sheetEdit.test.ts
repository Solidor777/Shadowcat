// @vitest-environment node
// Exercises plain state and pure functions: no component render and no DOM API use, so
// the package-default jsdom environment would be constructed per file and never touched.
import { describe, it, expect } from "vitest";
import { setField, setFields, unsetField } from "./sheetEdit";
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

describe("setField / unsetField are thin wrappers over setFields", () => {
  it("setField produces exactly what setFields produces for the single-edit case", () => {
    const a = makeCtx();
    const b = makeCtx();
    setField(a.ctx, "d1", "/system/hp", 8, 10);
    setFields(b.ctx, "d1", [{ path: "/system/hp", old: 8, value: 10 }]);
    expect(a.calls).toEqual(b.calls);
  });

  it("unsetField produces exactly what setFields produces for the single-edit removal case", () => {
    const a = makeCtx();
    const b = makeCtx();
    unsetField(a.ctx, "d1", "/system/tempFlag", true);
    setFields(b.ctx, "d1", [{ path: "/system/tempFlag", old: true, remove: true }]);
    expect(a.calls).toEqual(b.calls);
  });
});

describe("setFields", () => {
  it("dispatches ONE update op carrying every field's change, in order", () => {
    const { ctx, calls } = makeCtx();
    setFields(ctx, "d1", [
      { path: "/engine/draw", old: { kind: "weighted" }, value: { kind: "formula", notation: "1d20" } },
      { path: "/engine/rows", old: [], value: [] },
    ]);
    expect(calls).toEqual([[{
      op: "update", doc_id: "d1",
      changes: [
        { path: "/engine/draw", old: { kind: "weighted" }, new: { kind: "formula", notation: "1d20" } },
        { path: "/engine/rows", old: [], new: [] },
      ],
    }]]);
  });

  it("passes old: null ONLY when a field's pre-image is genuinely absent (undefined)", () => {
    const { ctx, calls } = makeCtx();
    setFields(ctx, "d1", [{ path: "/system/newField", old: undefined, value: 3 }]);
    expect((calls[0] as { changes: { old: unknown }[] }[])[0].changes[0].old).toBeNull();
  });

  it("preserves a falsy real pre-image (0 / false) as old, not null", () => {
    const { ctx, calls } = makeCtx();
    setFields(ctx, "d1", [
      { path: "/system/hp", old: 0, value: 5 },
      { path: "/system/flag", old: false, value: true },
    ]);
    const olds = (calls[0] as { changes: { old: unknown }[] }[])[0].changes.map((c) => c.old);
    expect(olds).toEqual([0, false]);
  });
});
