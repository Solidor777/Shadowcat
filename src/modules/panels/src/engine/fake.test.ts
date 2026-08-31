import { test, expect } from "vitest";
import { FakeEngine } from "./fake";
import { applyOp, defaultLayout } from "../layout/tree";

function makeSlots(ids: string[]): (id: string) => HTMLElement {
  const map = new Map<string, HTMLElement>();
  for (const id of ids) {
    const el = document.createElement("div");
    el.dataset.panel = id;
    map.set(id, el);
  }
  return (id: string) => map.get(id) ?? document.createElement("div");
}

// jsdom has no layout engine, so this can't assert computed pixel heights —
// it asserts the CONTRACT: `init()` must give both
// `host` and the adopted center-well container a definite size chain (flex
// context + `flex: 1`/`min-height: 0`), or the adopted `.stage` element's
// `height: 100%` resolves against an auto-height ancestor and collapses.
test("FakeEngine.init establishes a definite size chain on host and centerEl", () => {
  const engine = new FakeEngine();
  const host = document.createElement("div");
  const stageEl = document.createElement("div");
  engine.init(host, () => document.createElement("div"), stageEl);

  expect(host.style.display).toBe("flex");
  expect(host.style.flexDirection).toBe("column");
  expect(host.style.height).toBe("100%");
  expect(host.style.minHeight).toBe("0px");

  const centerEl = engine.centerEl();
  expect(centerEl).toBeTruthy();
  expect(centerEl!.style.flex).toBe("1 1 0%");
  expect(centerEl!.style.minHeight).toBe("0px");
});

test("poppedOut degrades to a floating window (bespoke-fallback engine has no cross-window popout)", () => {
  const host = document.createElement("div");
  const slotFor = makeSlots(["chat"]);
  const eng = new FakeEngine();
  eng.init(host, slotFor, document.createElement("div"));

  let l = defaultLayout([{ id: "chat" }]);
  l = applyOp(l, { op: "dock", id: "chat", zone: "right", group: "new" });
  l = applyOp(l, { op: "popOut", id: "chat", key: "w-chat", rect: null });
  eng.apply(l.expanded, new Map());

  // Rendered as a float window, so the slot stays adopted (never lost).
  const floatEl = eng.floatEl("chat");
  expect(floatEl).not.toBeNull();
  expect(floatEl?.contains(slotFor("chat"))).toBe(true);
  eng.destroy();
});

// `FakeEngine.apply` reads `ZoneNode.size` (the zone's own px basis, already
// tracked by the reducer and driven by dockview's real splitter) to give each
// docked zone's container a fixed px cross-size (contained, overflow-managed),
// once ANY groups are docked — without it, a docked zone's container carries
// no width/height constraint of its own and stretches to the full `host`
// cross-size (flex `align-items: stretch` default) instead of staying
// columned, regardless of group count.
test("FakeEngine constrains a zone's cross-size to ZoneNode.size once it has docked groups, past 2 groups", () => {
  const host = document.createElement("div");
  const slotFor = makeSlots(["a", "b", "c"]);
  const eng = new FakeEngine();
  eng.init(host, slotFor, document.createElement("div"));

  let l = defaultLayout([{ id: "a" }, { id: "b" }, { id: "c" }]);
  l = applyOp(l, { op: "dock", id: "a", zone: "right", group: "new" });
  l = applyOp(l, { op: "dock", id: "b", zone: "right", group: "new" });
  eng.apply(l.expanded, new Map());

  const zoneAfterTwo = eng.zoneEl("right")!;
  expect(zoneAfterTwo.style.width).toBe(`${l.expanded.zones.right.size}px`);
  expect(zoneAfterTwo.style.flex).toBe("0 0 auto");

  l = applyOp(l, { op: "dock", id: "c", zone: "right", group: "new" });
  eng.apply(l.expanded, new Map());

  // A THIRD docked group must not widen the zone past its own px basis.
  const zoneAfterThree = eng.zoneEl("right")!;
  expect(zoneAfterThree.style.width).toBe(`${l.expanded.zones.right.size}px`);
  expect(zoneAfterThree.style.flex).toBe("0 0 auto");
  expect(zoneAfterThree.style.overflow).not.toBe("");
  eng.destroy();
});

// A fixed (unoffset) fallback rect would stack every popped-out id fully
// overlapping at the identical position — this asserts two simultaneously
// popped-out ids render at distinct rects and z-indices under this
// bespoke-fallback engine (mirrors the cascade tests at the other degraded/
// rehydrated-position sites: `layout/tree`'s own test suite, `PanelsController`'s
// own test suite).
test("two simultaneously popped-out ids cascade to distinct floating rects under FakeEngine", () => {
  const host = document.createElement("div");
  const slotFor = makeSlots(["chat", "assets"]);
  const eng = new FakeEngine();
  eng.init(host, slotFor, document.createElement("div"));

  let l = defaultLayout([{ id: "chat" }, { id: "assets" }]);
  l = applyOp(l, { op: "dock", id: "chat", zone: "right", group: "new" });
  l = applyOp(l, { op: "popOut", id: "chat", key: "w-chat", rect: null });
  l = applyOp(l, { op: "dock", id: "assets", zone: "right", group: "new" });
  l = applyOp(l, { op: "popOut", id: "assets", key: "w-assets", rect: null });
  eng.apply(l.expanded, new Map());

  const chatEl = eng.floatEl("chat")!;
  const assetsEl = eng.floatEl("assets")!;
  expect(chatEl).not.toBeNull();
  expect(assetsEl).not.toBeNull();
  expect({ left: chatEl.style.left, top: chatEl.style.top }).not.toEqual({
    left: assetsEl.style.left,
    top: assetsEl.style.top,
  });
  expect(chatEl.style.zIndex).not.toBe(assetsEl.style.zIndex);
  eng.destroy();
});

// THIRD copy of the cascade constants: `POPOUT_FALLBACK_BASE`/`STEP` inside
// `apply()`, whose own comment asserts it mirrors `layout/tree`'s
// SHEET_CASCADE_BASE/STEP and `PanelsController`'s own REHYDRATE_FLOAT_BASE/STEP.
// The test above asserts only that two fallback rects DIFFER from each other,
// which stays green if this copy drifts away from the other two; the
// "cascade parity at index %i: a floating placement and a rehydrated popout
// land on the identical rect" parity test covers the other two but not this
// one. This
// closes the third leg: with no pre-existing floating panels, the i-th
// popped-out id's degraded rect must equal what the layout tree would place the
// i-th floating panel at. That test's index choice mirrors it — 3 and 5
// are the ones that pin the `% 6` modulus rather than merely the step.
test.each([0, 1, 3, 5, 7])(
  "cascade parity at index %i: FakeEngine's popout fallback matches the layout tree's floating placement",
  (index) => {
    const ids = Array.from({ length: index + 1 }, (_, i) => `p${i}`);
    const probe = ids[index];

    // Layout-tree side: place `index` panels floating, then the probe.
    let treeLayout = defaultLayout([]);
    for (const id of ids) {
      treeLayout = applyOp(treeLayout, { op: "open", id, placement: { kind: "floating" } });
    }
    const treeRect = treeLayout.expanded.floating.find((f) => f.id === probe)!.rect;

    // FakeEngine side: all ids popped out, degraded to floating by `apply()`.
    const host = document.createElement("div");
    const eng = new FakeEngine();
    eng.init(host, makeSlots(ids), document.createElement("div"));
    let popped = defaultLayout([]);
    for (const id of ids) {
      popped = applyOp(popped, { op: "open", id, placement: { kind: "docked", zone: "right" } });
      popped = applyOp(popped, { op: "popOut", id, key: `w-${id}`, rect: null });
    }
    eng.apply(popped.expanded, new Map());
    const el = eng.floatEl(probe)!;

    expect({ x: el.style.left, y: el.style.top, w: el.style.width, h: el.style.height }).toEqual({
      x: `${treeRect.x}px`,
      y: `${treeRect.y}px`,
      w: `${treeRect.w}px`,
      h: `${treeRect.h}px`,
    });
    eng.destroy();
  },
);
