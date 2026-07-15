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
// it asserts the CONTRACT (buddy-check finding 2): `init()` must give both
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

test("poppedOut degrades to a floating window (bespoke-fallback, spec §10)", () => {
  const host = document.createElement("div");
  const slotFor = makeSlots(["chat"]);
  const eng = new FakeEngine();
  eng.init(host, slotFor, document.createElement("div"));

  let l = defaultLayout([{ id: "chat" }]);
  l = applyOp(l, { op: "dock", id: "chat", zone: "right", group: "new" });
  l = applyOp(l, { op: "popOut", id: "chat" });
  eng.apply(l.expanded, new Map());

  // Rendered as a float window, so the slot stays adopted (never lost).
  const floatEl = eng.floatEl("chat");
  expect(floatEl).not.toBeNull();
  expect(floatEl?.contains(slotFor("chat"))).toBe(true);
  eng.destroy();
});

// Regression: `docs/OPEN_BUGS.md` "[Panels] The bespoke-fallback engine ...
// loses width containment once a THIRD docked group is added". `ZoneNode.size`
// (the zone's own px basis, already tracked by the reducer and driven by
// dockview's real splitter) was never read by `FakeEngine.apply` — a docked
// zone's container carried no width/height constraint at all, so it stretched
// to the full `host` cross-size (flex `align-items: stretch` default) instead
// of staying columned, regardless of group count. This asserts the zone
// container now carries a fixed px cross-size (contained, overflow-managed)
// once ANY groups are docked, unaffected by how many groups pile up.
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
// rehydrated-position sites: tree.test.ts, controller.test.ts).
test("two simultaneously popped-out ids cascade to distinct floating rects under FakeEngine", () => {
  const host = document.createElement("div");
  const slotFor = makeSlots(["chat", "assets"]);
  const eng = new FakeEngine();
  eng.init(host, slotFor, document.createElement("div"));

  let l = defaultLayout([{ id: "chat" }, { id: "assets" }]);
  l = applyOp(l, { op: "dock", id: "chat", zone: "right", group: "new" });
  l = applyOp(l, { op: "popOut", id: "chat" });
  l = applyOp(l, { op: "dock", id: "assets", zone: "right", group: "new" });
  l = applyOp(l, { op: "popOut", id: "assets" });
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
