import { describe, expect, it } from "vitest";
import { decodeLayout, encodeLayout } from "./persist";
import { defaultLayout, type PanelLayoutV1 } from "./tree";

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
