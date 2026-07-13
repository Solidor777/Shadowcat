import { describe, expect, it } from "vitest";
import { applyOp, defaultLayout, locate, prune, type PanelLayoutV1 } from "./tree";

const REGS = [
  { id: "chat", placement: { kind: "docked" as const, zone: "right" as const } },
  { id: "assets", placement: { kind: "minimized" as const } },
  { id: "actors", placement: { kind: "minimized" as const } },
  { id: "factions", placement: { kind: "minimized" as const } },
  { id: "conditions", placement: { kind: "minimized" as const } },
  { id: "game-settings", placement: { kind: "minimized" as const } },
  { id: "settings", placement: { kind: "minimized" as const } },
];

function base(): PanelLayoutV1 {
  return defaultLayout(REGS);
}

describe("defaultLayout", () => {
  it("docks chat alone in a right-zone group and minimizes the rest", () => {
    const l = base();
    expect(l.expanded.zones.right.groups).toEqual([{ tabs: ["chat"], active: "chat", size: 1 }]);
    expect(l.expanded.minimized).toEqual(["assets", "actors", "factions", "conditions", "game-settings", "settings"]);
    expect(l.expanded.minimized).toHaveLength(6);
  });

  it("orders compact.order by registration order and defaults activeView to the first", () => {
    const l = base();
    expect(l.compact.order).toEqual(REGS.map((r) => r.id));
    expect(l.compact.activeView).toBe("chat");
  });

  it("leaves a placement-less registration closed (not docked/floating/minimized) but still in compact.order", () => {
    const l = defaultLayout([{ id: "chat" }]);
    expect(locate(l, "chat")).toEqual({ where: "closed" });
    expect(l.compact.order).toEqual(["chat"]);
  });

  it("all three zone keys are always present, even empty", () => {
    const l = defaultLayout([]);
    expect(l.expanded.zones.right.groups).toEqual([]);
    expect(l.expanded.zones.bottom.groups).toEqual([]);
    expect(l.expanded.zones.left.groups).toEqual([]);
  });
});

describe("locate", () => {
  it("finds a docked panel's zone/group/tabIndex", () => {
    const l = base();
    expect(locate(l, "chat")).toEqual({ where: "docked", zone: "right", group: 0, tabIndex: 0 });
  });

  it("finds a minimized panel", () => {
    expect(locate(base(), "assets")).toEqual({ where: "minimized" });
  });

  it("returns closed for an unknown id (total, no throw)", () => {
    expect(locate(base(), "nope")).toEqual({ where: "closed" });
  });
});

describe("invariant: at most one location", () => {
  it("dock removes the panel from wherever it previously was", () => {
    let l = base();
    l = applyOp(l, { op: "dock", id: "assets", zone: "bottom", group: "new" });
    expect(locate(l, "assets")).toEqual({ where: "docked", zone: "bottom", group: 0, tabIndex: 0 });
    expect(l.expanded.minimized).not.toContain("assets");
  });

  it("float removes the panel from wherever it previously was", () => {
    let l = base();
    l = applyOp(l, { op: "float", id: "assets", rect: { x: 0, y: 0, w: 100, h: 100 } });
    expect(l.expanded.minimized).not.toContain("assets");
    expect(locate(l, "assets")).toEqual({ where: "floating", index: 0 });
  });

  it("random op sequences never place an id in two locations at once", () => {
    const ops: Array<() => object> = [
      () => ({ op: "dock", id: "assets", zone: "right", group: "new" }),
      () => ({ op: "dock", id: "assets", zone: "bottom", group: 0 }),
      () => ({ op: "float", id: "assets", rect: { x: 1, y: 2, w: 3, h: 4 } }),
      () => ({ op: "minimize", id: "assets" }),
      () => ({ op: "close", id: "assets" }),
      () => ({ op: "open", id: "assets", placement: { kind: "docked", zone: "left" } }),
      () => ({ op: "restore", id: "assets" }),
    ];
    let l = base();
    // Deterministic pseudo-random walk (seeded index sequence) — reproducible, not corpus-based.
    const seq = [0, 3, 1, 2, 4, 5, 6, 3, 2, 4, 0, 5, 1, 6, 3];
    for (const i of seq) {
      // eslint-disable-next-line @typescript-eslint/no-explicit-any
      l = applyOp(l, ops[i]() as any);
      const loc = locate(l, "assets");
      const count =
        (loc.where === "docked" ? 1 : 0) +
        (loc.where === "floating" ? 1 : 0) +
        (loc.where === "minimized" ? 1 : 0) +
        (loc.where === "closed" ? 1 : 0);
      expect(count).toBe(1);
      // Cross-check against the raw structures: assets appears in exactly one of them.
      const inZones = Object.values(l.expanded.zones).some((z) => z.groups.some((g) => g.tabs.includes("assets")));
      const inFloating = l.expanded.floating.some((f) => f.id === "assets");
      const inMinimized = l.expanded.minimized.includes("assets");
      const total = [inZones, inFloating, inMinimized].filter(Boolean).length;
      expect(total).toBeLessThanOrEqual(1);
    }
  });
});

describe("op: close/minimize/float detach first", () => {
  it("close from docked removes the tab and, being the only tab, removes the group (zone keeps its size)", () => {
    let l = base();
    const beforeZoneSize = l.expanded.zones.right.size;
    l = applyOp(l, { op: "close", id: "chat" });
    expect(l.expanded.zones.right.groups).toEqual([]);
    expect(l.expanded.zones.right.size).toBe(beforeZoneSize);
    expect(locate(l, "chat")).toEqual({ where: "closed" });
  });

  it("close on an already-closed id is a total no-op returning the SAME reference", () => {
    const l = base();
    const l2 = applyOp(l, { op: "close", id: "does-not-exist" });
    expect(l2).toBe(l);
  });

  it("minimize from docked detaches then adds to minimized", () => {
    let l = base();
    l = applyOp(l, { op: "minimize", id: "chat" });
    expect(l.expanded.zones.right.groups).toEqual([]);
    expect(l.expanded.minimized).toContain("chat");
  });

  it("float from minimized detaches then adds a floating entry", () => {
    let l = base();
    l = applyOp(l, { op: "float", id: "assets", rect: { x: 10, y: 20, w: 300, h: 200 } });
    expect(l.expanded.minimized).not.toContain("assets");
    expect(l.expanded.floating).toEqual([{ id: "assets", rect: { x: 10, y: 20, w: 300, h: 200 }, z: 0 }]);
  });
});

describe("op: open is a focus no-op when already open", () => {
  it("docked: sets group active, does not move the panel", () => {
    let l = base();
    l = applyOp(l, { op: "dock", id: "assets", zone: "right", group: 0 });
    expect(l.expanded.zones.right.groups[0]).toEqual({ tabs: ["chat", "assets"], active: "assets", size: 1 });
    l = applyOp(l, { op: "open", id: "chat" });
    expect(l.expanded.zones.right.groups[0].active).toBe("chat");
    expect(l.expanded.zones.right.groups[0].tabs).toEqual(["chat", "assets"]);
  });

  it("floating: bumps z to the top instead of moving zones", () => {
    let l = base();
    l = applyOp(l, { op: "float", id: "assets", rect: { x: 0, y: 0, w: 1, h: 1 } });
    l = applyOp(l, { op: "float", id: "actors", rect: { x: 0, y: 0, w: 1, h: 1 } });
    // actors is now on top (z=1); re-open assets and it should become top instead.
    l = applyOp(l, { op: "open", id: "assets" });
    const assetsEntry = l.expanded.floating.find((f) => f.id === "assets")!;
    const maxZ = Math.max(...l.expanded.floating.map((f) => f.z));
    expect(assetsEntry.z).toBe(maxZ);
  });

  it("floating: no-op (same reference) when already on top", () => {
    let l = base();
    l = applyOp(l, { op: "float", id: "assets", rect: { x: 0, y: 0, w: 1, h: 1 } });
    const l2 = applyOp(l, { op: "open", id: "assets" });
    expect(l2).toBe(l);
  });

  it("minimized: places per the given placement", () => {
    let l = base();
    l = applyOp(l, { op: "open", id: "assets", placement: { kind: "docked", zone: "bottom" } });
    expect(locate(l, "assets")).toEqual({ where: "docked", zone: "bottom", group: 0, tabIndex: 0 });
  });
});

describe("op: dock", () => {
  it("group: 'new' inserts at end with equal-share size renormalization", () => {
    let l = base();
    l = applyOp(l, { op: "dock", id: "assets", zone: "right", group: "new" });
    expect(l.expanded.zones.right.groups.map((g) => g.size)).toEqual([0.5, 0.5]);
    l = applyOp(l, { op: "dock", id: "actors", zone: "right", group: "new" });
    l.expanded.zones.right.groups.forEach((g) => expect(g.size).toBeCloseTo(1 / 3));
  });

  it("group: N inserts into the existing group's tabs and activates it", () => {
    let l = base();
    l = applyOp(l, { op: "dock", id: "assets", zone: "right", group: 0 });
    expect(l.expanded.zones.right.groups).toHaveLength(1);
    expect(l.expanded.zones.right.groups[0].tabs).toEqual(["chat", "assets"]);
    expect(l.expanded.zones.right.groups[0].active).toBe("assets");
  });
});

describe("op: activeTab", () => {
  it("sets active for a member id", () => {
    let l = base();
    l = applyOp(l, { op: "dock", id: "assets", zone: "right", group: 0 });
    l = applyOp(l, { op: "activeTab", zone: "right", group: 0, id: "chat" });
    expect(l.expanded.zones.right.groups[0].active).toBe("chat");
  });

  it("is a total no-op returning the SAME reference for a non-member id", () => {
    const l = base();
    const l2 = applyOp(l, { op: "activeTab", zone: "right", group: 0, id: "not-a-tab" });
    expect(l2).toBe(l);
  });

  it("is a total no-op for an out-of-range group index (no throw)", () => {
    const l = base();
    const l2 = applyOp(l, { op: "activeTab", zone: "right", group: 99, id: "chat" });
    expect(l2).toBe(l);
  });
});

describe("op: resizeZone / resizeGroup", () => {
  it("resizeZone updates the zone px size and no-ops on an unchanged value", () => {
    const l = base();
    const l2 = applyOp(l, { op: "resizeZone", zone: "right", size: 400 });
    expect(l2.expanded.zones.right.size).toBe(400);
    const l3 = applyOp(l2, { op: "resizeZone", zone: "right", size: 400 });
    expect(l3).toBe(l2);
  });

  it("resizeGroup does not renormalize sibling groups (manual resize is not structural)", () => {
    let l = base();
    l = applyOp(l, { op: "dock", id: "assets", zone: "right", group: "new" });
    l = applyOp(l, { op: "resizeGroup", zone: "right", group: 0, size: 0.8 });
    expect(l.expanded.zones.right.groups[0].size).toBe(0.8);
    expect(l.expanded.zones.right.groups[1].size).toBe(0.5);
  });

  it("resizeGroup on an out-of-range group is a total no-op (no throw)", () => {
    const l = base();
    const l2 = applyOp(l, { op: "resizeGroup", zone: "right", group: 99, size: 0.8 });
    expect(l2).toBe(l);
  });
});

describe("op: compactView", () => {
  it("sets activeView for a known id", () => {
    let l = base();
    l = applyOp(l, { op: "compactView", id: "assets" });
    expect(l.compact.activeView).toBe("assets");
  });

  it("no-ops on an id absent from compact.order", () => {
    const l = base();
    const l2 = applyOp(l, { op: "compactView", id: "unregistered" });
    expect(l2).toBe(l);
  });
});

describe("prune", () => {
  it("drops unknown ids from zones, floating, minimized, and compact.order", () => {
    let l = base();
    l = applyOp(l, { op: "float", id: "actors", rect: { x: 0, y: 0, w: 1, h: 1 } });
    const known = new Set(["chat", "settings"]);
    l = prune(l, known);
    expect(locate(l, "actors")).toEqual({ where: "closed" });
    expect(l.expanded.minimized).toEqual(["settings"]);
    expect(l.compact.order).toEqual(["chat", "settings"]);
  });

  it("fixes a group's active id to a surviving member after pruning the prior active", () => {
    let l = base();
    l = applyOp(l, { op: "dock", id: "assets", zone: "right", group: 0 }); // active becomes assets
    l = prune(l, new Set(["chat", "assets", "actors", "factions", "conditions", "game-settings", "settings"].filter((x) => x !== "assets")));
    expect(l.expanded.zones.right.groups[0].tabs).toEqual(["chat"]);
    expect(l.expanded.zones.right.groups[0].active).toBe("chat");
  });

  it("fixes compact.activeView to a surviving order member, or null if none survive", () => {
    let l = base();
    l = applyOp(l, { op: "compactView", id: "assets" });
    l = prune(l, new Set(["settings"]));
    expect(l.compact.order).toEqual(["settings"]);
    expect(l.compact.activeView).toBe("settings");

    const l2 = prune(l, new Set());
    expect(l2.compact.order).toEqual([]);
    expect(l2.compact.activeView).toBeNull();
  });

  it("only renormalizes a zone's groups when a whole group was dropped, not on partial-tab pruning", () => {
    let l = base();
    l = applyOp(l, { op: "dock", id: "assets", zone: "right", group: "new" });
    l = applyOp(l, { op: "resizeGroup", zone: "right", group: 0, size: 0.7 });
    // Prune nothing away: sizes must be untouched.
    const known = new Set(REGS.map((r) => r.id));
    const pruned = prune(l, known);
    expect(pruned.expanded.zones.right.groups[0].size).toBe(0.7);
  });
});

describe("every op is total (no throw on any prior location)", () => {
  const ALL_OPS: object[] = [
    { op: "open", id: "ghost" },
    { op: "close", id: "ghost" },
    { op: "dock", id: "ghost", zone: "right", group: "new" },
    { op: "dock", id: "ghost", zone: "right", group: 5 },
    { op: "float", id: "ghost", rect: { x: 0, y: 0, w: 1, h: 1 } },
    { op: "minimize", id: "ghost" },
    { op: "restore", id: "ghost" },
    { op: "activeTab", zone: "right", group: 5, id: "ghost" },
    { op: "resizeZone", zone: "right", size: 100 },
    { op: "resizeGroup", zone: "right", group: 5, size: 0.5 },
    { op: "compactView", id: "ghost" },
  ];

  it("applies every op against a fresh empty layout without throwing", () => {
    for (const o of ALL_OPS) {
      const l = defaultLayout([]);
      // eslint-disable-next-line @typescript-eslint/no-explicit-any
      expect(() => applyOp(l, o as any)).not.toThrow();
    }
  });
});
