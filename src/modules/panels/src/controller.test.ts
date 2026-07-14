import { test, expect, vi } from "vitest";
import { ContributionRegistry, PANEL_CONTRACT, silentLogger } from "@shadowcat/core";
import type { PanelsApi } from "@shadowcat/ui-kit";
import { PanelsController, regsForRole, type PanelsBridgeLike } from "./controller.svelte";
import { locate } from "./layout/tree";
import { encodeLayout } from "./layout/persist";

function registry(): ContributionRegistry {
  const r = new ContributionRegistry();
  r.contribute({
    id: "a:panel",
    contract: PANEL_CONTRACT,
    component: {},
    panel: { icon: "a", labelKey: "a.tab", defaultPlacement: { kind: "docked", zone: "bottom" } },
  });
  r.contribute({
    id: "b:panel",
    contract: PANEL_CONTRACT,
    component: {},
    panel: { icon: "b", labelKey: "b.tab", defaultPlacement: { kind: "docked", zone: "right" } },
  });
  return r;
}

function fakeBridge(): PanelsBridgeLike & { bind: ReturnType<typeof vi.fn<(impl: PanelsApi) => void>> } {
  return { bind: vi.fn<(impl: PanelsApi) => void>() };
}

test("open on a closed reg uses its defaultPlacement", () => {
  const contributions = registry();
  const setPanelLayout = vi.fn();
  const ctrl = new PanelsController({
    contributions,
    role: "gm",
    getPanelLayout: () => null,
    setPanelLayout,
    bridge: fakeBridge(),
    logger: silentLogger,
  });

  // "a:panel" starts docked (its defaultPlacement); close it, then re-open —
  // the reopen must land back on "bottom" (its OWN defaultPlacement), not the
  // "right" fallback `placeByPlacement` uses when no placement is given.
  ctrl.close("a:panel");
  expect(locate(ctrl.layout, "a:panel").where).toBe("closed");

  ctrl.open("a:panel");
  const loc = locate(ctrl.layout, "a:panel");
  expect(loc.where).toBe("docked");
  expect(loc.where === "docked" && loc.zone).toBe("bottom");
});

test("an op (engine gesture or PanelsApi call) persists the encoded new tree", () => {
  const contributions = registry();
  const setPanelLayout = vi.fn();
  const ctrl = new PanelsController({
    contributions,
    role: "gm",
    getPanelLayout: () => null,
    setPanelLayout,
    bridge: fakeBridge(),
    logger: silentLogger,
  });
  setPanelLayout.mockClear(); // ignore any construction-time persistence

  ctrl.dispatch({ op: "minimize", id: "b:panel" });

  expect(setPanelLayout).toHaveBeenCalledTimes(1);
  expect(setPanelLayout).toHaveBeenCalledWith(encodeLayout(ctrl.layout));
  expect(locate(ctrl.layout, "b:panel").where).toBe("minimized");
});

test("a no-op dispatch (same-reference contract) never re-persists", () => {
  const contributions = registry();
  const setPanelLayout = vi.fn();
  const ctrl = new PanelsController({
    contributions,
    role: "gm",
    getPanelLayout: () => null,
    setPanelLayout,
    bridge: fakeBridge(),
    logger: silentLogger,
  });
  setPanelLayout.mockClear();

  // "a:panel" is already the active (sole) tab in its group: `open` is a focus
  // no-op per `applyOp`'s same-reference contract.
  ctrl.dispatch({ op: "open", id: "a:panel" });

  expect(setPanelLayout).not.toHaveBeenCalled();
});

test("an invalid persisted blob resets to default, fires the reset callback, and persists the default", () => {
  const contributions = registry();
  const setPanelLayout = vi.fn();
  const onReset = vi.fn();
  const ctrl = new PanelsController({
    contributions,
    role: "gm",
    getPanelLayout: () => ({ version: 2, garbage: true }),
    setPanelLayout,
    bridge: fakeBridge(),
    logger: silentLogger,
    onReset,
  });

  expect(onReset).toHaveBeenCalledTimes(1);
  expect(onReset).toHaveBeenCalledWith("panels.layoutReset");
  expect(setPanelLayout).toHaveBeenCalledTimes(1);
  expect(setPanelLayout).toHaveBeenCalledWith(encodeLayout(ctrl.layout));
  // The persisted default places both regs per their own defaultPlacement.
  expect(locate(ctrl.layout, "a:panel").where).toBe("docked");
  expect(locate(ctrl.layout, "b:panel").where).toBe("docked");
});

// Reproduces the real-world gap this fixes: a module-registration order where NOT every
// panel-contract module has contributed yet by the time `PanelHost` constructs its
// controller (`defaultLayout` then only sees a partial `regs` list). Without
// `syncRegistrations` catching up, a late-registering panel is default-placed nowhere —
// no zone, no minimized chip, no compact-switcher tab — for the rest of the session.
test("syncRegistrations default-places a panel that registers AFTER construction", () => {
  const contributions = new ContributionRegistry();
  contributions.contribute({
    id: "a:panel",
    contract: PANEL_CONTRACT,
    component: {},
    panel: { icon: "a", labelKey: "a.tab", defaultPlacement: { kind: "docked", zone: "bottom" } },
  });
  const ctrl = new PanelsController({
    contributions,
    role: "gm",
    getPanelLayout: () => null,
    setPanelLayout: () => {},
    bridge: fakeBridge(),
    logger: silentLogger,
  });

  // "b:panel" was absent from the registry at construction time — it must start
  // reachable nowhere yet (this controller has never seen it).
  expect(locate(ctrl.layout, "b:panel")).toEqual({ where: "closed" });
  expect(ctrl.layout.compact.order).toEqual(["a:panel"]);

  // It registers late (mirrors PanelHost's `visibleRegs` growing after mount).
  contributions.contribute({
    id: "b:panel",
    contract: PANEL_CONTRACT,
    component: {},
    panel: { icon: "b", labelKey: "b.tab", defaultPlacement: { kind: "minimized" } },
  });
  ctrl.syncRegistrations(ctrl.visibleRegs);

  expect(locate(ctrl.layout, "b:panel").where).toBe("minimized");
  expect(ctrl.layout.compact.order).toEqual(["a:panel", "b:panel"]);
});

test("syncRegistrations persists only when something actually changed", () => {
  const contributions = registry();
  const setPanelLayout = vi.fn();
  const ctrl = new PanelsController({
    contributions,
    role: "gm",
    getPanelLayout: () => null,
    setPanelLayout,
    bridge: fakeBridge(),
    logger: silentLogger,
  });
  setPanelLayout.mockClear();

  ctrl.syncRegistrations(ctrl.visibleRegs); // same regs as construction — no-op

  expect(setPanelLayout).not.toHaveBeenCalled();
});

test("regsForRole: a gmOnly registration is invisible to a non-GM role", () => {
  const regs = [
    { id: "chat:panel", contract: PANEL_CONTRACT, component: {}, panel: { icon: "c", labelKey: "chat.tab" } },
    {
      id: "game-settings:panel",
      contract: PANEL_CONTRACT,
      component: {},
      panel: { icon: "g", labelKey: "gameSettings.tab", gmOnly: true },
    },
  ];

  expect(regsForRole(regs, "player").map((r) => r.id)).toEqual(["chat:panel"]);
  expect(regsForRole(regs, "gm").map((r) => r.id)).toEqual(["chat:panel", "game-settings:panel"]);
});

test("the controller binds itself into the supplied bridge at construction", () => {
  const bridge = fakeBridge();
  const ctrl = new PanelsController({
    contributions: registry(),
    role: "gm",
    getPanelLayout: () => null,
    setPanelLayout: () => {},
    bridge,
    logger: silentLogger,
  });

  expect(bridge.bind).toHaveBeenCalledTimes(1);
  expect(bridge.bind).toHaveBeenCalledWith(ctrl);
});
