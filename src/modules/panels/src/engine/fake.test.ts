import { test, expect } from "vitest";
import { FakeEngine } from "./fake";

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
