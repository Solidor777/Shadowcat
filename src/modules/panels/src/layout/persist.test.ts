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

describe("decodeLayout source (B4: pre-prune blob for PanelsController)", () => {
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
});

test("decode round-trips poppedOut ids", () => {
  let l = defaultLayout([{ id: "chat" }]);
  l = applyOp(l, { op: "dock", id: "chat", zone: "right", group: "new" });
  l = applyOp(l, { op: "popOut", id: "chat" });
  const { layout, reset } = decodeLayout(l, new Set(["chat"]), () => defaultLayout([]));
  expect(reset).toBe(false);
  expect(layout.expanded.poppedOut).toEqual(["chat"]);
});

test("decode of a blob predating the poppedOut field normalizes to []", () => {
  const legacy = {
    version: 1,
    expanded: { zones: { right: { groups: [], size: 320 }, bottom: { groups: [], size: 240 }, left: { groups: [], size: 320 } }, floating: [], minimized: [] },
    compact: { activeView: null, order: [] },
  };
  const { layout, reset } = decodeLayout(legacy, new Set(), () => defaultLayout([]));
  expect(reset).toBe(false);
  expect(layout.expanded.poppedOut).toEqual([]);
});

test("decode rejects a non-string-array poppedOut", () => {
  const bad = {
    version: 1,
    expanded: { zones: { right: { groups: [], size: 320 }, bottom: { groups: [], size: 240 }, left: { groups: [], size: 320 } }, floating: [], minimized: [], poppedOut: [1, 2] },
    compact: { activeView: null, order: [] },
  };
  const { reset } = decodeLayout(bad, new Set(), () => defaultLayout([]));
  expect(reset).toBe(true);
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
