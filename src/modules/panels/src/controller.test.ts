import { test, expect, vi } from "vitest";
import { ContributionRegistry, PANEL_CONTRACT, silentLogger } from "@shadowcat/core";
import type { PanelsApi } from "@shadowcat/ui-kit";
import { PanelsController, regsForRole, type PanelsBridgeLike } from "./controller.svelte";
import { applyOp, defaultLayout, locate, type Rect } from "./layout/tree";
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

// `panels` (which requires PANEL_CONTRACT) topologically activates BEFORE any
// panel-contract module, so `PanelHost`'s controller is routinely constructed against an
// EMPTY (or partial) registry — this reproduces that exact condition and asserts a saved,
// customized layout survives it instead of being silently overwritten with defaults.
test("late registrations against an empty-registry construction restore their SAVED positions, not defaults", () => {
  // A saved, customized layout: assets docked left alone, actors floating, chat docked
  // right, settings minimized, factions closed-but-known (never placed).
  const saved = {
    version: 1 as const,
    expanded: {
      zones: {
        right: { groups: [{ tabs: ["chat"], active: "chat", size: 1 }], size: 320 },
        bottom: { groups: [], size: 240 },
        left: { groups: [{ tabs: ["assets"], active: "assets", size: 1 }], size: 320 },
      },
      floating: [{ id: "actors", rect: { x: 40, y: 60, w: 400, h: 300 }, z: 0 }],
      minimized: ["settings"],
    },
    compact: { activeView: "chat", order: ["chat", "assets", "actors", "settings", "factions"] },
  };

  const contributions = new ContributionRegistry(); // EMPTY at construction.
  const setPanelLayout = vi.fn();
  const ctrl = new PanelsController({
    contributions,
    role: "gm",
    getPanelLayout: () => saved,
    setPanelLayout,
    bridge: fakeBridge(),
    logger: silentLogger,
  });

  // Registrations arrive incrementally, in an order that does not match `saved.compact.order`.
  const reg = (id: string, defaultPlacement?: { kind: "docked"; zone: "left" | "right" | "bottom" } | { kind: "minimized" }) =>
    contributions.contribute({
      id,
      contract: PANEL_CONTRACT,
      component: {},
      panel: { icon: id, labelKey: `${id}.tab`, defaultPlacement },
    });

  reg("settings", { kind: "minimized" });
  ctrl.syncRegistrations(ctrl.visibleRegs);
  reg("factions", { kind: "minimized" });
  ctrl.syncRegistrations(ctrl.visibleRegs);
  reg("actors", { kind: "docked", zone: "right" }); // default would be docked right, NOT floating
  ctrl.syncRegistrations(ctrl.visibleRegs);
  reg("chat", { kind: "docked", zone: "right" });
  ctrl.syncRegistrations(ctrl.visibleRegs);
  reg("assets", { kind: "minimized" }); // default would be minimized, NOT docked left
  ctrl.syncRegistrations(ctrl.visibleRegs);

  expect(locate(ctrl.layout, "assets")).toEqual({ where: "docked", zone: "left", group: 0, tabIndex: 0 });
  expect(locate(ctrl.layout, "actors")).toEqual({ where: "floating", index: 0 });
  expect(ctrl.layout.expanded.floating[0]).toEqual({ id: "actors", rect: { x: 40, y: 60, w: 400, h: 300 }, z: 0 });
  expect(locate(ctrl.layout, "chat")).toEqual({ where: "docked", zone: "right", group: 0, tabIndex: 0 });
  expect(locate(ctrl.layout, "settings")).toEqual({ where: "minimized" });
  // factions was closed-but-known in the saved blob — stays closed, never re-defaulted.
  expect(locate(ctrl.layout, "factions")).toEqual({ where: "closed" });
  expect(ctrl.layout.compact.order).toEqual(["chat", "assets", "actors", "settings", "factions"]);

  // No call along the way persisted a defaults-shaped tree that discarded the customization
  // — every persisted snapshot must already carry the restored (non-default) positions.
  for (const call of setPanelLayout.mock.calls) {
    const blob = call[0] as typeof saved;
    const assetsLoc = locate(blob as never, "assets");
    if (assetsLoc.where === "docked") expect(assetsLoc.zone).toBe("left");
  }
});

test("rehydratePoppedOut: a persisted popped-out id comes back as floating + a notice", () => {
  let saved = defaultLayout([{ id: "chat", placement: { kind: "docked", zone: "right" } }]);
  saved = applyOp(saved, { op: "dock", id: "chat", zone: "right", group: "new" });
  saved = applyOp(saved, { op: "popOut", id: "chat", key: "w-chat", rect: { left: 500, top: 100, width: 900, height: 700 } });

  const contributions = new ContributionRegistry();
  contributions.contribute({
    id: "chat",
    contract: PANEL_CONTRACT,
    component: {},
    panel: { icon: "c", labelKey: "chat.tab", defaultPlacement: { kind: "docked", zone: "right" } },
  });

  const notices: string[] = [];
  const setPanelLayout = vi.fn();
  const ctrl = new PanelsController({
    contributions,
    role: "gm",
    getPanelLayout: () => saved,
    setPanelLayout,
    bridge: fakeBridge(),
    logger: silentLogger,
    onNotice: (key) => notices.push(key),
  });

  expect(ctrl.layout.expanded.floating.map((f) => f.id)).toEqual(["chat"]);
  // The window entry is RETAINED, marked dormant (rect and panel set intact) —
  // the arrangement record a later restore gesture re-opens.
  expect(ctrl.layout.expanded.popouts).toEqual([
    { key: "w-chat", panels: ["chat"], rect: { left: 500, top: 100, width: 900, height: 700 }, dormant: true },
  ]);
  // A dormant entry is a record, not a live window: the panel locates floating.
  expect(locate(ctrl.layout, "chat").where).toBe("floating");
  // The notice is QUEUED, not fired, at construction
  // — `deps.onNotice` must not be invoked until a post-mount caller (`PanelHost`'s
  // `$effect`) calls `flushPendingNotice()`. Firing it here, synchronously
  // alongside construction, would set the a11y live region's text before its
  // first paint; a `polite` live region only announces CHANGES.
  expect(notices).toEqual([]);
  // The converted layout must be PERSISTED, not just held in memory — otherwise
  // this notice would fire again on every subsequent page load.
  expect(setPanelLayout).toHaveBeenCalledWith(encodeLayout(ctrl.layout));

  ctrl.flushPendingNotice();
  expect(notices).toEqual(["panels.popoutRestoredFloating"]);

  // Idempotent: a second flush must not re-announce.
  ctrl.flushPendingNotice();
  expect(notices).toEqual(["panels.popoutRestoredFloating"]);
});

// A fixed (unoffset) rehydration rect would stack every rehydrated popout at
// the identical (x,y) — this asserts the cascade offset actually differs
// across two rehydrated ids.
test("rehydratePoppedOut: two persisted popped-out ids cascade to distinct floating rects", () => {
  const contributions = new ContributionRegistry();
  for (const id of ["chat", "assets"]) {
    contributions.contribute({
      id,
      contract: PANEL_CONTRACT,
      component: {},
      panel: { icon: id, labelKey: `${id}.tab`, defaultPlacement: { kind: "docked", zone: "right" } },
    });
  }

  let saved = defaultLayout([
    { id: "chat", placement: { kind: "docked", zone: "right" } },
    { id: "assets", placement: { kind: "docked", zone: "right" } },
  ]);
  saved = applyOp(saved, { op: "dock", id: "chat", zone: "right", group: "new" });
  saved = applyOp(saved, { op: "popOut", id: "chat", key: "w-chat", rect: null });
  saved = applyOp(saved, { op: "dock", id: "assets", zone: "right", group: "new" });
  saved = applyOp(saved, { op: "popOut", id: "assets", key: "w-assets", rect: null });

  const setPanelLayout = vi.fn();
  const ctrl = new PanelsController({
    contributions,
    role: "gm",
    getPanelLayout: () => saved,
    setPanelLayout,
    bridge: fakeBridge(),
    logger: silentLogger,
  });

  expect(ctrl.layout.expanded.popouts).toEqual([
    { key: "w-chat", panels: ["chat"], rect: null, dormant: true },
    { key: "w-assets", panels: ["assets"], rect: null, dormant: true },
  ]);
  const rects = ctrl.layout.expanded.floating.map((f) => ({ x: f.rect.x, y: f.rect.y }));
  expect(rects).toHaveLength(2);
  expect(rects[0]).not.toEqual(rects[1]);
  expect(setPanelLayout).toHaveBeenCalledWith(encodeLayout(ctrl.layout));
});

// Panels that shared ONE saved pop-out window must rehydrate to ADJACENT
// cascade steps (nothing interleaved between them), so the reload's floating
// stack still reads as that window's group. Offsets below are the cascade
// base + step already pinned by the "cascade parity at index %i" gate.
test("rehydratePoppedOut: one saved window's panels cascade adjacently, behind an already-floating panel", () => {
  const contributions = new ContributionRegistry();
  for (const id of ["pre", "chat", "assets", "actors"]) {
    contributions.contribute({
      id,
      contract: PANEL_CONTRACT,
      component: {},
      panel: { icon: id, labelKey: `${id}.tab`, defaultPlacement: { kind: "docked", zone: "right" } },
    });
  }

  let saved = defaultLayout([]);
  saved = applyOp(saved, { op: "open", id: "pre", placement: { kind: "floating" } });
  saved = applyOp(saved, { op: "open", id: "chat", placement: { kind: "docked", zone: "right" } });
  saved = applyOp(saved, { op: "open", id: "assets", placement: { kind: "docked", zone: "right" } });
  saved = applyOp(saved, { op: "open", id: "actors", placement: { kind: "docked", zone: "right" } });
  // One window holding chat+assets (tab order), a second holding actors.
  saved = applyOp(saved, { op: "popOut", id: "chat", key: "w1", rect: null });
  saved = applyOp(saved, { op: "popOutInto", id: "assets", key: "w1" });
  saved = applyOp(saved, { op: "popOut", id: "actors", key: "w2", rect: { left: 40, top: 50, width: 600, height: 500 } });

  const ctrl = new PanelsController({
    contributions,
    role: "gm",
    getPanelLayout: () => saved,
    setPanelLayout: () => {},
    bridge: fakeBridge(),
    logger: silentLogger,
  });

  // "pre" already floated at cascade index 0; rehydration continues from
  // index 1: chat (w1's first panel), assets (w1's second — the ADJACENT
  // step), then actors (w2).
  const rectOf = (id: string) => ctrl.layout.expanded.floating.find((f) => f.id === id)!.rect;
  expect(rectOf("pre")).toEqual({ x: 96, y: 96, w: 420, h: 520 });
  expect(rectOf("chat")).toEqual({ x: 124, y: 124, w: 420, h: 520 });
  expect(rectOf("assets")).toEqual({ x: 152, y: 152, w: 420, h: 520 });
  expect(rectOf("actors")).toEqual({ x: 180, y: 180, w: 420, h: 520 });
  // Both windows retained as dormant arrangement records, rects intact.
  expect(ctrl.layout.expanded.popouts).toEqual([
    { key: "w1", panels: ["chat", "assets"], rect: null, dormant: true },
    { key: "w2", panels: ["actors"], rect: { left: 40, top: 50, width: 600, height: 500 }, dormant: true },
  ]);
});

// A blob saved before the tree tracked window grouping (legacy `poppedOut`
// id array) decodes via the deterministic migration, then rehydrates exactly
// like a current-shape blob: floated panels, retained dormant records.
test("rehydratePoppedOut: a legacy poppedOut blob migrates, floats, and retains dormant windows", () => {
  const contributions = new ContributionRegistry();
  for (const id of ["chat", "assets"]) {
    contributions.contribute({
      id,
      contract: PANEL_CONTRACT,
      component: {},
      panel: { icon: id, labelKey: `${id}.tab`, defaultPlacement: { kind: "docked", zone: "right" } },
    });
  }
  const legacy = {
    version: 1,
    expanded: {
      zones: {
        right: { groups: [], size: 320 },
        bottom: { groups: [], size: 240 },
        left: { groups: [], size: 320 },
      },
      floating: [],
      minimized: [],
      poppedOut: ["chat", "assets"],
    },
    compact: { activeView: null, order: ["chat", "assets"] },
  };

  const ctrl = new PanelsController({
    contributions,
    role: "gm",
    getPanelLayout: () => legacy,
    setPanelLayout: () => {},
    bridge: fakeBridge(),
    logger: silentLogger,
  });

  expect(ctrl.layout.expanded.floating.map((f) => f.id)).toEqual(["chat", "assets"]);
  expect(ctrl.layout.expanded.popouts).toEqual([
    { key: "legacy-chat", panels: ["chat"], rect: null, dormant: true },
    { key: "legacy-assets", panels: ["assets"], rect: null, dormant: true },
  ]);
});

// The restore affordance is keyed on dormant-PRESENCE, not on conversion: a
// second reload's blob carries ONLY dormant entries (the first reload's
// rehydrate already converted and persisted them), so nothing converts — yet
// the arrangement is still restorable, so the notice (with its "Reopen
// windows" action) must still fire.
test("rehydratePoppedOut: a blob carrying only dormant entries converts nothing but still queues the restore notice, with its action", () => {
  const contributions = new ContributionRegistry();
  contributions.contribute({
    id: "chat",
    contract: PANEL_CONTRACT,
    component: {},
    panel: { icon: "c", labelKey: "chat.tab", defaultPlacement: { kind: "docked", zone: "right" } },
  });

  // Session 1's blob: chat popped out in a live window.
  let saved = defaultLayout([{ id: "chat", placement: { kind: "docked", zone: "right" } }]);
  saved = applyOp(saved, { op: "dock", id: "chat", zone: "right", group: "new" });
  saved = applyOp(saved, { op: "popOut", id: "chat", key: "w-chat", rect: { left: 500, top: 100, width: 900, height: 700 } });

  // Session 1's rehydrate converts + persists; its OUTPUT is session 2's blob.
  const first = new PanelsController({
    contributions,
    role: "gm",
    getPanelLayout: () => saved,
    setPanelLayout: () => {},
    bridge: fakeBridge(),
    logger: silentLogger,
  });
  const secondBlob = encodeLayout(first.layout);

  const notices: { key: string; action?: { labelKey: string } }[] = [];
  const setPanelLayout = vi.fn();
  const second = new PanelsController({
    contributions,
    role: "gm",
    getPanelLayout: () => secondBlob,
    setPanelLayout,
    bridge: fakeBridge(),
    logger: silentLogger,
    onNotice: (key, action) => notices.push({ key, ...(action ? { action } : {}) }),
  });

  // Nothing converted (the panel was already floating in the blob), and
  // nothing needed re-persisting.
  expect(second.layout.expanded.floating.map((f) => f.id)).toEqual(["chat"]);
  expect(second.layout.expanded.popouts).toEqual([
    { key: "w-chat", panels: ["chat"], rect: { left: 500, top: 100, width: 900, height: 700 }, dormant: true },
  ]);
  expect(setPanelLayout).not.toHaveBeenCalled();

  second.flushPendingNotice();
  expect(notices).toEqual([{ key: "panels.popoutRestoredFloating", action: { labelKey: "panels.reopenWindows" } }]);
});

// The restore gesture's arrangement record is the retained PRE-PRUNE persisted
// source: a window's panel that registers only after construction (the boot
// registration trickle) is pruned out of the live tree's dormant entry at
// decode/`syncRegistrations` time, but the restore must still know the full
// saved panel set.
test("restorablePopouts returns the full pre-prune arrangement across the boot registration trickle", () => {
  const contributions = new ContributionRegistry();
  contributions.contribute({
    id: "chat",
    contract: PANEL_CONTRACT,
    component: {},
    panel: { icon: "c", labelKey: "chat.tab", defaultPlacement: { kind: "docked", zone: "right" } },
  });

  // chat + assets shared ONE saved window — but only chat is registered at
  // construction time.
  let saved = defaultLayout([{ id: "chat", placement: { kind: "docked", zone: "right" } }]);
  saved = applyOp(saved, { op: "dock", id: "chat", zone: "right", group: "new" });
  saved = applyOp(saved, { op: "popOut", id: "chat", key: "w1", rect: { left: 40, top: 50, width: 600, height: 500 } });
  saved = {
    ...saved,
    expanded: {
      ...saved.expanded,
      popouts: [{ key: "w1", panels: ["chat", "assets"], rect: { left: 40, top: 50, width: 600, height: 500 } }],
    },
  };

  const ctrl = new PanelsController({
    contributions,
    role: "gm",
    getPanelLayout: () => saved,
    setPanelLayout: () => {},
    bridge: fakeBridge(),
    logger: silentLogger,
  });

  // The live tree's dormant record was pruned to the registered panel…
  expect(ctrl.layout.expanded.popouts).toEqual([
    { key: "w1", panels: ["chat"], rect: { left: 40, top: 50, width: 600, height: 500 }, dormant: true },
  ]);
  // …but the restore surface reads the full saved arrangement.
  expect(ctrl.restorablePopouts()).toEqual([
    { key: "w1", panels: ["chat", "assets"], rect: { left: 40, top: 50, width: 600, height: 500 } },
  ]);

  // The late registration floats the panel (persisted popped-out location →
  // floating) and the restore record stays complete.
  contributions.contribute({
    id: "assets",
    contract: PANEL_CONTRACT,
    component: {},
    panel: { icon: "a", labelKey: "assets.tab", defaultPlacement: { kind: "docked", zone: "right" } },
  });
  ctrl.syncRegistrations(regsForRole(contributions.contributionsFor(PANEL_CONTRACT), "gm"));
  expect(locate(ctrl.layout, "assets").where).toBe("floating");
  // The live tree's record heals back to the full saved set, so the next
  // persist no longer carries the trickle-shrunken record.
  expect(ctrl.layout.expanded.popouts).toEqual([
    { key: "w1", panels: ["chat", "assets"], rect: { left: 40, top: 50, width: 600, height: 500 }, dormant: true },
  ]);
  expect(ctrl.restorablePopouts()).toEqual([
    { key: "w1", panels: ["chat", "assets"], rect: { left: 40, top: 50, width: 600, height: 500 } },
  ]);
});

// The notice offers the restore gesture whenever an arrangement was SAVED —
// the boot registration trickle can prune the live tree's records to empty
// before the panel ever registers, so keying the notice on the live tree
// alone would lose the gesture exactly when a registration lags.
test("the restore notice keys on the saved arrangement even when the panel has not registered yet", () => {
  const contributions = new ContributionRegistry(); // nothing registered at construction
  let saved = defaultLayout([{ id: "chat", placement: { kind: "docked", zone: "right" } }]);
  saved = applyOp(saved, { op: "dock", id: "chat", zone: "right", group: "new" });
  saved = applyOp(saved, { op: "popOut", id: "chat", key: "w-chat", rect: { left: 500, top: 100, width: 900, height: 700 } });

  const notices: { key: string; action?: { labelKey: string } }[] = [];
  const ctrl = new PanelsController({
    contributions,
    role: "gm",
    getPanelLayout: () => saved,
    setPanelLayout: () => {},
    bridge: fakeBridge(),
    logger: silentLogger,
    onNotice: (key, action) => notices.push({ key, ...(action ? { action } : {}) }),
  });

  // "chat" is unknown at construction: pruned out of the live tree entirely…
  expect(locate(ctrl.layout, "chat").where).toBe("closed");
  expect(ctrl.layout.expanded.popouts).toEqual([]);
  // …and nothing is restorable while no panel of the window is registered.
  expect(ctrl.restorablePopouts()).toEqual([]);
  // The action notice is WITHHELD (not dropped) while the gesture could
  // restore nothing — the host re-flushes on every registration change.
  ctrl.flushPendingNotice();
  expect(notices).toEqual([]);

  // The panel's late registration (the boot trickle) makes the saved window
  // restorable, and the next flush delivers the notice.
  contributions.contribute({
    id: "chat",
    contract: PANEL_CONTRACT,
    component: {},
    panel: { icon: "c", labelKey: "chat.tab", defaultPlacement: { kind: "docked", zone: "right" } },
  });
  ctrl.syncRegistrations(regsForRole(contributions.contributionsFor(PANEL_CONTRACT), "gm"));
  ctrl.flushPendingNotice();
  expect(notices).toEqual([{ key: "panels.popoutRestoredFloating", action: { labelKey: "panels.reopenWindows" } }]);
});

// No persisted history (or a reset blob) means no arrangement to restore.
test("restorablePopouts is empty with no persisted source", () => {
  const ctrl = new PanelsController({
    contributions: registry(),
    role: "gm",
    getPanelLayout: () => null,
    setPanelLayout: () => {},
    bridge: fakeBridge(),
    logger: silentLogger,
  });
  expect(ctrl.restorablePopouts()).toEqual([]);
});

// Anti-drift gate for a deliberately-forked constant pair. `layout/tree`'s
// SHEET_CASCADE_BASE/STEP and `PanelsController`'s own REHYDRATE_FLOAT_BASE/STEP
// are intentionally NOT a shared import (the pure layout tree stays decoupled
// from the controller), so nothing structural stops one from drifting; both
// comments promise only that they stay numerically identical. Every other
// cascade test — here, in `layout/tree`'s own test suite, and `engine/fake`'s —
// asserts only that
// a given side's own offsets differ FROM EACH OTHER, which stays green if
// either pair changes. This is the one test that fails on divergence: it drives
// both call sites to the same floating index and demands the identical rect.
// Index choice is load-bearing. n=0 pins BASE (offset is 0 whatever STEP is);
// n=1 pins STEP; n=7 exercises the wrap. Those three alone do NOT pin the
// MODULUS: 0,1,7 have identical residues under `% 6`, `% 3` and `% 2`, so a
// one-side edit to `% 3` passes every one of them while diverging at n=2..6.
// n=3 and n=5 disagree across those divisors (3 -> 3/0/1, 5 -> 5/2/1) and are
// what actually gate the `% 6`.
function rectViaPlacement(alreadyFloating: number): Rect {
  let l = defaultLayout([]);
  for (let i = 0; i < alreadyFloating; i++) {
    l = applyOp(l, { op: "open", id: `pre${i}`, placement: { kind: "floating" } });
  }
  l = applyOp(l, { op: "open", id: "probe", placement: { kind: "floating" } });
  return l.expanded.floating.find((f) => f.id === "probe")!.rect;
}

function rectViaRehydration(alreadyFloating: number): Rect {
  const ids = Array.from({ length: alreadyFloating }, (_, i) => `pre${i}`);
  const contributions = new ContributionRegistry();
  for (const id of [...ids, "probe"]) {
    contributions.contribute({
      id,
      contract: PANEL_CONTRACT,
      component: {},
      panel: { icon: id, labelKey: `${id}.tab` },
    });
  }
  let saved = defaultLayout([]);
  for (const id of ids) saved = applyOp(saved, { op: "open", id, placement: { kind: "floating" } });
  saved = applyOp(saved, { op: "open", id: "probe", placement: { kind: "docked", zone: "right" } });
  saved = applyOp(saved, { op: "popOut", id: "probe", key: "w-probe", rect: null });

  const ctrl = new PanelsController({
    contributions,
    role: "gm",
    getPanelLayout: () => saved,
    setPanelLayout: () => {},
    bridge: fakeBridge(),
    logger: silentLogger,
  });
  return ctrl.layout.expanded.floating.find((f) => f.id === "probe")!.rect;
}

test.each([0, 1, 3, 5, 7])(
  "cascade parity at index %i: a floating placement and a rehydrated popout land on the identical rect",
  (alreadyFloating) => {
    expect(rectViaRehydration(alreadyFloating)).toEqual(rectViaPlacement(alreadyFloating));
  },
);

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
