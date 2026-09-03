import { describe, expect, it, test } from "vitest";
import { decodeLayout, encodeLayout } from "./persist";
import { applyOp, defaultLayout, type PanelLayoutV1 } from "./tree";

const REGS = [
  { id: "chat", placement: { kind: "docked" as const, zone: "right" as const } },
  { id: "assets", placement: { kind: "minimized" as const } },
];

function base(): PanelLayoutV1 {
  return defaultLayout(REGS);
}

function fallback(): PanelLayoutV1 {
  return defaultLayout(REGS);
}

const KNOWN = new Set(["chat", "assets"]);

describe("encodeLayout / decodeLayout round-trip", () => {
  it("decodes exactly what was encoded (identity)", () => {
    const l = base();
    const encoded = encodeLayout(l);
    const { layout, reset } = decodeLayout(encoded, KNOWN, fallback);
    expect(reset).toBe(false);
    expect(layout).toEqual(l);
  });
});

describe("decodeLayout source (pre-prune blob for PanelsController)", () => {
  it("returns the untouched pre-prune blob as source when valid, even with a partial known set", () => {
    const l = base(); // records both "chat" and "assets"
    const encoded = encodeLayout(l);
    // `known` reflects only what has registered SO FAR (the boot race) — narrower than
    // what the blob actually records.
    const partiallyKnown = new Set(["chat"]);
    const { layout, source } = decodeLayout(encoded, partiallyKnown, fallback);
    // The returned `layout` is pruned against the partial set (assets dropped)...
    expect(layout.compact.order).toEqual(["chat"]);
    // ...but `source` still records "assets" — nothing here has been pruned.
    expect(source).toEqual(l);
    expect(source?.compact.order).toEqual(["chat", "assets"]);
  });

  it("returns null source on reset", () => {
    const { source, reset } = decodeLayout("not an object", KNOWN, fallback);
    expect(reset).toBe(true);
    expect(source).toBeNull();
  });
});

describe("decodeLayout structural validation", () => {
  it("resets on non-object garbage", () => {
    const fb = fallback();
    const { layout, reset } = decodeLayout("not an object", KNOWN, () => fb);
    expect(reset).toBe(true);
    expect(layout).toBe(fb);
  });

  it("resets on null", () => {
    const fb = fallback();
    const { layout, reset } = decodeLayout(null, KNOWN, () => fb);
    expect(reset).toBe(true);
    expect(layout).toBe(fb);
  });

  it("resets on the wrong version", () => {
    const encoded = encodeLayout(base()) as Record<string, unknown>;
    const bad = { ...encoded, version: 2 };
    const fb = fallback();
    const { layout, reset } = decodeLayout(bad, KNOWN, () => fb);
    expect(reset).toBe(true);
    expect(layout).toBe(fb);
  });

  it("resets on a truncated (missing compact) blob", () => {
    const encoded = encodeLayout(base()) as Record<string, unknown>;
    const { compact: _compact, ...truncated } = encoded;
    const fb = fallback();
    const { layout, reset } = decodeLayout(truncated, KNOWN, () => fb);
    expect(reset).toBe(true);
    expect(layout).toBe(fb);
  });

  it("resets when a panel id is a non-string", () => {
    const encoded = encodeLayout(base()) as PanelLayoutV1 & Record<string, unknown>;
    const bad = {
      ...encoded,
      expanded: {
        ...encoded.expanded,
        minimized: [123],
      },
    };
    const fb = fallback();
    const { layout, reset } = decodeLayout(bad, KNOWN, () => fb);
    expect(reset).toBe(true);
    expect(layout).toBe(fb);
  });
});

describe("decodeLayout numeric guards (finite/non-negative)", () => {
  it("resets when a Rect coordinate is NaN", () => {
    const encoded = structuredClone(encodeLayout(base())) as PanelLayoutV1 & Record<string, unknown>;
    const firstFloating = { id: "chat", rect: { x: NaN, y: 0, w: 10, h: 10 }, z: 0 };
    const bad = { ...encoded, expanded: { ...encoded.expanded, floating: [firstFloating] } };
    const fb = fallback();
    const { layout, reset } = decodeLayout(bad, KNOWN, () => fb);
    expect(reset).toBe(true);
    expect(layout).toBe(fb);
  });

  it("resets when a Rect w is negative", () => {
    const encoded = structuredClone(encodeLayout(base())) as PanelLayoutV1 & Record<string, unknown>;
    const badFloating = { id: "chat", rect: { x: 0, y: 0, w: -50, h: 10 }, z: 0 };
    const bad = { ...encoded, expanded: { ...encoded.expanded, floating: [badFloating] } };
    const fb = fallback();
    const { layout, reset } = decodeLayout(bad, KNOWN, () => fb);
    expect(reset).toBe(true);
    expect(layout).toBe(fb);
  });

  it("resets when a GroupNode size is Infinity", () => {
    const encoded = structuredClone(encodeLayout(base())) as PanelLayoutV1 & Record<string, unknown>;
    const zones = encoded.expanded.zones as Record<string, unknown>;
    const right = zones.right as { groups: Array<Record<string, unknown>>; size: number };
    const bad = {
      ...encoded,
      expanded: {
        ...encoded.expanded,
        zones: { ...zones, right: { ...right, groups: [{ tabs: ["chat"], active: "chat", size: Infinity }] } },
      },
    };
    const fb = fallback();
    const { layout, reset } = decodeLayout(bad, KNOWN, () => fb);
    expect(reset).toBe(true);
    expect(layout).toBe(fb);
  });

  it("resets when a ZoneNode size is NaN", () => {
    const encoded = structuredClone(encodeLayout(base())) as PanelLayoutV1 & Record<string, unknown>;
    const zones = encoded.expanded.zones as Record<string, unknown>;
    const right = zones.right as Record<string, unknown>;
    const bad = { ...encoded, expanded: { ...encoded.expanded, zones: { ...zones, right: { ...right, size: NaN } } } };
    const fb = fallback();
    const { layout, reset } = decodeLayout(bad, KNOWN, () => fb);
    expect(reset).toBe(true);
    expect(layout).toBe(fb);
  });

  it("resets when a floating item's z is NaN", () => {
    const encoded = structuredClone(encodeLayout(base())) as PanelLayoutV1 & Record<string, unknown>;
    const badFloating = { id: "chat", rect: { x: 0, y: 0, w: 10, h: 10 }, z: NaN };
    const bad = { ...encoded, expanded: { ...encoded.expanded, floating: [badFloating] } };
    const fb = fallback();
    const { layout, reset } = decodeLayout(bad, KNOWN, () => fb);
    expect(reset).toBe(true);
    expect(layout).toBe(fb);
  });
});

describe("decodeLayout per-guard-branch malformed blobs", () => {
  it("resets when a Rect is missing a field", () => {
    const encoded = structuredClone(encodeLayout(base())) as PanelLayoutV1 & Record<string, unknown>;
    const badFloating: Record<string, unknown> = { id: "chat", rect: { x: 0, y: 0, w: 10 }, z: 0 };
    const bad = { ...encoded, expanded: { ...encoded.expanded, floating: [badFloating] } };
    const fb = fallback();
    const { layout, reset } = decodeLayout(bad, KNOWN, () => fb);
    expect(reset).toBe(true);
    expect(layout).toBe(fb);
  });

  it("resets when a GroupNode is missing tabs", () => {
    const encoded = structuredClone(encodeLayout(base())) as PanelLayoutV1 & Record<string, unknown>;
    const zones = encoded.expanded.zones as Record<string, unknown>;
    const right = zones.right as Record<string, unknown>;
    const bad = {
      ...encoded,
      expanded: {
        ...encoded.expanded,
        zones: { ...zones, right: { ...right, groups: [{ active: "chat", size: 1 }] } },
      },
    };
    const fb = fallback();
    const { layout, reset } = decodeLayout(bad, KNOWN, () => fb);
    expect(reset).toBe(true);
    expect(layout).toBe(fb);
  });

  it("resets when a GroupNode's tabs is not an array", () => {
    const encoded = structuredClone(encodeLayout(base())) as PanelLayoutV1 & Record<string, unknown>;
    const zones = encoded.expanded.zones as Record<string, unknown>;
    const right = zones.right as Record<string, unknown>;
    const bad = {
      ...encoded,
      expanded: {
        ...encoded.expanded,
        zones: { ...zones, right: { ...right, groups: [{ tabs: "chat", active: "chat", size: 1 }] } },
      },
    };
    const fb = fallback();
    const { layout, reset } = decodeLayout(bad, KNOWN, () => fb);
    expect(reset).toBe(true);
    expect(layout).toBe(fb);
  });

  it("resets when a GroupNode is missing size", () => {
    const encoded = structuredClone(encodeLayout(base())) as PanelLayoutV1 & Record<string, unknown>;
    const zones = encoded.expanded.zones as Record<string, unknown>;
    const right = zones.right as Record<string, unknown>;
    const bad = {
      ...encoded,
      expanded: {
        ...encoded.expanded,
        zones: { ...zones, right: { ...right, groups: [{ tabs: ["chat"], active: "chat" }] } },
      },
    };
    const fb = fallback();
    const { layout, reset } = decodeLayout(bad, KNOWN, () => fb);
    expect(reset).toBe(true);
    expect(layout).toBe(fb);
  });

  it("resets when a ZoneNode's groups is not an array", () => {
    const encoded = structuredClone(encodeLayout(base())) as PanelLayoutV1 & Record<string, unknown>;
    const zones = encoded.expanded.zones as Record<string, unknown>;
    const right = zones.right as Record<string, unknown>;
    const bad = { ...encoded, expanded: { ...encoded.expanded, zones: { ...zones, right: { ...right, groups: "nope" } } } };
    const fb = fallback();
    const { layout, reset } = decodeLayout(bad, KNOWN, () => fb);
    expect(reset).toBe(true);
    expect(layout).toBe(fb);
  });

  it("resets when a floating item is missing rect", () => {
    const encoded = structuredClone(encodeLayout(base())) as PanelLayoutV1 & Record<string, unknown>;
    const badFloating: Record<string, unknown> = { id: "chat", z: 0 };
    const bad = { ...encoded, expanded: { ...encoded.expanded, floating: [badFloating] } };
    const fb = fallback();
    const { layout, reset } = decodeLayout(bad, KNOWN, () => fb);
    expect(reset).toBe(true);
    expect(layout).toBe(fb);
  });

  it("resets when a floating item's id is not a string", () => {
    const encoded = structuredClone(encodeLayout(base())) as PanelLayoutV1 & Record<string, unknown>;
    const badFloating = { id: 42, rect: { x: 0, y: 0, w: 10, h: 10 }, z: 0 };
    const bad = { ...encoded, expanded: { ...encoded.expanded, floating: [badFloating] } };
    const fb = fallback();
    const { layout, reset } = decodeLayout(bad, KNOWN, () => fb);
    expect(reset).toBe(true);
    expect(layout).toBe(fb);
  });

  it("resets when compact.order is not an array", () => {
    const encoded = structuredClone(encodeLayout(base())) as PanelLayoutV1 & Record<string, unknown>;
    const bad = { ...encoded, compact: { ...encoded.compact, order: "chat" } };
    const fb = fallback();
    const { layout, reset } = decodeLayout(bad, KNOWN, () => fb);
    expect(reset).toBe(true);
    expect(layout).toBe(fb);
  });

  it("resets when compact.activeView is neither null nor a string", () => {
    const encoded = structuredClone(encodeLayout(base())) as PanelLayoutV1 & Record<string, unknown>;
    const bad = { ...encoded, compact: { ...encoded.compact, activeView: 7 } };
    const fb = fallback();
    const { layout, reset } = decodeLayout(bad, KNOWN, () => fb);
    expect(reset).toBe(true);
    expect(layout).toBe(fb);
  });

  it("resets when a zone key is missing", () => {
    const encoded = structuredClone(encodeLayout(base())) as PanelLayoutV1 & Record<string, unknown>;
    const zones = encoded.expanded.zones as Record<string, unknown>;
    const { right: _right, ...zonesWithoutRight } = zones;
    const bad = { ...encoded, expanded: { ...encoded.expanded, zones: zonesWithoutRight } };
    const fb = fallback();
    const { layout, reset } = decodeLayout(bad, KNOWN, () => fb);
    expect(reset).toBe(true);
    expect(layout).toBe(fb);
  });
});

describe("decodeLayout referential consistency", () => {
  it("resets when a group's active is not in its own tabs", () => {
    const encoded = structuredClone(encodeLayout(base())) as PanelLayoutV1 & Record<string, unknown>;
    const zones = encoded.expanded.zones as Record<string, unknown>;
    const right = zones.right as Record<string, unknown>;
    const bad = {
      ...encoded,
      expanded: {
        ...encoded.expanded,
        zones: { ...zones, right: { ...right, groups: [{ tabs: ["chat"], active: "assets", size: 1 }] } },
      },
    };
    const fb = fallback();
    const { layout, reset } = decodeLayout(bad, KNOWN, () => fb);
    expect(reset).toBe(true);
    expect(layout).toBe(fb);
  });

  it("resets when compact.activeView is not in compact.order", () => {
    const encoded = structuredClone(encodeLayout(base())) as PanelLayoutV1 & Record<string, unknown>;
    const bad = { ...encoded, compact: { activeView: "not-in-order", order: encoded.compact.order } };
    const fb = fallback();
    const { layout, reset } = decodeLayout(bad, KNOWN, () => fb);
    expect(reset).toBe(true);
    expect(layout).toBe(fb);
  });

  it("resets when one panel id occupies two locations (docked and popped out)", () => {
    // A crafted blob can place the same id in a zone's tabs AND in a popout
    // window's panels; every structural guard passes, so the uniqueness arm of
    // the referential guard is what rejects it.
    const encoded = structuredClone(encodeLayout(base())) as PanelLayoutV1 & Record<string, unknown>;
    const zones = encoded.expanded.zones as Record<string, unknown>;
    const right = zones.right as Record<string, unknown>;
    const bad = {
      ...encoded,
      expanded: {
        ...encoded.expanded,
        zones: { ...zones, right: { ...right, groups: [{ tabs: ["chat"], active: "chat", size: 1 }] } },
        popouts: [{ key: "w-chat", panels: ["chat"], rect: null }],
      },
    };
    const fb = fallback();
    const { layout, reset } = decodeLayout(bad, KNOWN, () => fb);
    expect(reset).toBe(true);
    expect(layout).toBe(fb);
  });

  it("resets when one panel id appears in two popout windows", () => {
    const encoded = structuredClone(encodeLayout(base())) as PanelLayoutV1 & Record<string, unknown>;
    const bad = {
      ...encoded,
      expanded: {
        ...encoded.expanded,
        popouts: [
          { key: "w-1", panels: ["chat"], rect: null },
          { key: "w-2", panels: ["chat"], rect: null },
        ],
      },
    };
    const fb = fallback();
    const { layout, reset } = decodeLayout(bad, KNOWN, () => fb);
    expect(reset).toBe(true);
    expect(layout).toBe(fb);
  });

  it("resets when a legacy poppedOut id is also docked", () => {
    const encoded = structuredClone(encodeLayout(base())) as PanelLayoutV1 & Record<string, unknown>;
    const zones = encoded.expanded.zones as Record<string, unknown>;
    const right = zones.right as Record<string, unknown>;
    const expanded = { ...encoded.expanded, zones: { ...zones, right: { ...right, groups: [{ tabs: ["chat"], active: "chat", size: 1 }] } } } as Record<string, unknown>;
    delete expanded.popouts;
    const bad = { ...encoded, expanded: { ...expanded, poppedOut: ["chat"] } };
    const fb = fallback();
    const { layout, reset } = decodeLayout(bad, KNOWN, () => fb);
    expect(reset).toBe(true);
    expect(layout).toBe(fb);
  });
});

test("decode round-trips popouts windows", () => {
  let l = defaultLayout([{ id: "chat" }]);
  l = applyOp(l, { op: "dock", id: "chat", zone: "right", group: "new" });
  l = applyOp(l, { op: "popOut", id: "chat", key: "w-chat", rect: null });
  const { layout, reset } = decodeLayout(l, new Set(["chat"]), () => defaultLayout([]));
  expect(reset).toBe(false);
  expect(layout.expanded.popouts).toEqual([{ key: "w-chat", panels: ["chat"], rect: null }]);
});

// Back-compat: a blob saved before the tree tracked window grouping carries the
// legacy `poppedOut` id array. Migration is deterministic (pure decode — no
// uuid minting), one single-panel window per id, keyed `legacy-<id>`.
test("a legacy poppedOut blob migrates to one single-panel window per id", () => {
  const legacy = {
    version: 1,
    expanded: { zones: { right: { groups: [], size: 320 }, bottom: { groups: [], size: 240 }, left: { groups: [], size: 320 } }, floating: [], minimized: [], poppedOut: ["chat", "assets"] },
    compact: { activeView: null, order: [] },
  };
  const { layout, reset } = decodeLayout(legacy, new Set(["chat", "assets"]), () => defaultLayout([]));
  expect(reset).toBe(false);
  expect(layout.expanded.popouts).toEqual([
    { key: "legacy-chat", panels: ["chat"], rect: null },
    { key: "legacy-assets", panels: ["assets"], rect: null },
  ]);
});

test("decode of a blob predating both popout fields normalizes to []", () => {
  const legacy = {
    version: 1,
    expanded: { zones: { right: { groups: [], size: 320 }, bottom: { groups: [], size: 240 }, left: { groups: [], size: 320 } }, floating: [], minimized: [] },
    compact: { activeView: null, order: [] },
  };
  const { layout, reset } = decodeLayout(legacy, new Set(), () => defaultLayout([]));
  expect(reset).toBe(false);
  expect(layout.expanded.popouts).toEqual([]);
});

test("decode rejects a non-string-array legacy poppedOut", () => {
  const bad = {
    version: 1,
    expanded: { zones: { right: { groups: [], size: 320 }, bottom: { groups: [], size: 240 }, left: { groups: [], size: 320 } }, floating: [], minimized: [], poppedOut: [1, 2] },
    compact: { activeView: null, order: [] },
  };
  const { reset } = decodeLayout(bad, new Set(), () => defaultLayout([]));
  expect(reset).toBe(true);
});

// A present `popouts` field is the canonical shape and is validated strictly:
// any malformed entry fails the WHOLE blob, same as every other field.
describe("decodeLayout popouts validation", () => {
  const zones = { right: { groups: [], size: 320 }, bottom: { groups: [], size: 240 }, left: { groups: [], size: 320 } };
  const blob = (popouts: unknown) => ({
    version: 1,
    expanded: { zones, floating: [], minimized: [], popouts },
    compact: { activeView: null, order: [] },
  });
  const decode = (popouts: unknown) => decodeLayout(blob(popouts), new Set(["chat"]), () => defaultLayout([]));

  test("rejects a non-array popouts", () => {
    expect(decode("chat").reset).toBe(true);
  });

  test("rejects an entry with a non-string key", () => {
    expect(decode([{ key: 1, panels: ["chat"], rect: null }]).reset).toBe(true);
  });

  test("rejects an entry with an empty panels list (emptied windows are dropped, never persisted)", () => {
    expect(decode([{ key: "w", panels: [], rect: null }]).reset).toBe(true);
  });

  test("rejects an entry missing its rect field (must be null or a ScreenRect)", () => {
    expect(decode([{ key: "w", panels: ["chat"] }]).reset).toBe(true);
  });

  test("rejects a rect with a non-finite coordinate", () => {
    expect(decode([{ key: "w", panels: ["chat"], rect: { left: NaN, top: 0, width: 100, height: 100 } }]).reset).toBe(true);
  });

  test("rejects a rect whose width/height is not positive", () => {
    expect(decode([{ key: "w", panels: ["chat"], rect: { left: 0, top: 0, width: 0, height: 100 } }]).reset).toBe(true);
    expect(decode([{ key: "w", panels: ["chat"], rect: { left: 0, top: 0, width: 100, height: -5 } }]).reset).toBe(true);
  });

  test("rejects a non-boolean dormant marker", () => {
    expect(decode([{ key: "w", panels: ["chat"], rect: null, dormant: "yes" }]).reset).toBe(true);
  });

  test("round-trips a window's rect and dormant marker", () => {
    const rect = { left: -1400, top: 120, width: 800, height: 600 };
    const { layout, reset } = decode([{ key: "w", panels: ["chat"], rect, dormant: true }]);
    expect(reset).toBe(false);
    expect(layout.expanded.popouts).toEqual([{ key: "w", panels: ["chat"], rect, dormant: true }]);
  });

  test("a present popouts takes precedence over a legacy poppedOut carried alongside it", () => {
    const both = {
      version: 1,
      expanded: { zones, floating: [], minimized: [], popouts: [], poppedOut: ["chat"] },
      compact: { activeView: null, order: [] },
    };
    const { layout, reset } = decodeLayout(both, new Set(["chat"]), () => defaultLayout([]));
    expect(reset).toBe(false);
    expect(layout.expanded.popouts).toEqual([]);
  });
});

describe("decodeLayout pruning", () => {
  it("prunes unknown ids without resetting", () => {
    const l = base();
    const encoded = encodeLayout(l);
    const knownMinusAssets = new Set(["chat"]);
    const { layout, reset } = decodeLayout(encoded, knownMinusAssets, fallback);
    expect(reset).toBe(false);
    expect(layout.expanded.minimized).toEqual([]);
    expect(layout.compact.order).toEqual(["chat"]);
  });
});
