import { describe, it, expect } from "vitest";
import { DocumentStore, AssetResolver, buildSceneDoc, buildTokenDoc, parseBars, EMPTY_BARS, type TokenEngine, type WireDocument, type BarsLookup } from "@shadowcat/core";
import { MockBackend, TokenView, type TokenBarsHost } from "./index";

function storeWith(...docs: WireDocument[]): DocumentStore {
  const s = new DocumentStore();
  s.applyCommand({ seq: 1, world_id: "w1", author: "a", ts: 0, ops: docs.map((doc) => ({ op: "create" as const, doc })) });
  return s;
}

function token(id: string, x: number): WireDocument {
  const engine: TokenEngine = {
    x, y: 0, w: 100, h: 100, rotation: 0, visual: null, actor_id: null, overrides: null, face: null,
    elevation: null, lock_rotation: false, identity_hidden: false,
  };
  return buildTokenDoc("w1", "scene1", engine, id);
}

const bar = (mode: string, current = 5, max = 10) => ({ label: "HP", current, max, color: "#ff0000", mode });

function lookup(tokens: Record<string, { nameplate: string | null; nameplate_mode: string | null; bars: unknown[] }>): BarsLookup {
  return parseBars({ scenes: { scene1: { tokens } } });
}

interface Rig {
  backend: MockBackend;
  view: TokenView;
  host: { bars: BarsLookup; enabled: boolean; selected: Set<string> };
}

function rig(bars: BarsLookup): Rig {
  const backend = new MockBackend();
  const host = { bars, enabled: true, selected: new Set<string>() };
  const barsHost: TokenBarsHost = { lookup: () => host.bars, enabled: () => host.enabled, fallbackLabel: () => "Unknown" };
  const store = storeWith(buildSceneDoc("w1", {}, "scene1"), token("tok1", 0), token("tok2", 500));
  const view = new TokenView(store, new AssetResolver(), backend, () => "scene1", () => null, undefined, undefined, () => host.selected, undefined, undefined, undefined, undefined, barsHost);
  view.reconcile();
  return { backend, view, host };
}

describe("TokenView nameplates and bars", () => {
  it("draws nothing for a host that delivers nothing", () => {
    const { backend } = rig(EMPTY_BARS);
    expect(backend.tokens.get("tok1")!.nameplate).toBeUndefined();
    expect(backend.tokens.get("tok1")!.bars).toBeUndefined();
  });

  it("draws an always nameplate and bar continuously, with the fill fraction and packed colour", () => {
    const { backend } = rig(lookup({ tok1: { nameplate: "Goblin", nameplate_mode: "always", bars: [bar("always", 4, 10)] } }));
    const spec = backend.tokens.get("tok1")!;
    expect(spec.nameplate).toBe("Goblin");
    expect(spec.bars).toEqual([{ label: "HP", fraction: 0.4, color: 0xff0000 }]);
    expect(backend.tokens.get("tok2")!.nameplate).toBeUndefined();
  });

  it("clamps the fill fraction and reads a non-positive max as empty", () => {
    const { backend } = rig(lookup({ tok1: { nameplate: null, nameplate_mode: null, bars: [bar("always", 30, 10), bar("always", 3, 0), bar("always", -2, 10)] } }));
    expect(backend.tokens.get("tok1")!.bars!.map((b) => b.fraction)).toEqual([1, 0, 0]);
  });

  it("holds a hover value back until the token is hovered, then hides it again", () => {
    const { backend, view } = rig(lookup({ tok1: { nameplate: "Goblin", nameplate_mode: "hover", bars: [bar("hover")] } }));
    expect(backend.tokens.get("tok1")!.nameplate).toBeUndefined();
    expect(backend.tokens.get("tok1")!.bars).toBeUndefined();
    view.setHovered("tok1");
    expect(backend.tokens.get("tok1")!.nameplate).toBe("Goblin");
    expect(backend.tokens.get("tok1")!.bars).toHaveLength(1);
    view.setHovered(null);
    expect(backend.tokens.get("tok1")!.nameplate).toBeUndefined();
    expect(backend.tokens.get("tok1")!.bars).toBeUndefined();
  });

  it("moving the hover between tokens re-projects both", () => {
    const { backend, view } = rig(lookup({
      tok1: { nameplate: "A", nameplate_mode: "hover", bars: [] },
      tok2: { nameplate: "B", nameplate_mode: "hover", bars: [] },
    }));
    view.setHovered("tok1");
    view.setHovered("tok2");
    expect(backend.tokens.get("tok1")!.nameplate).toBeUndefined();
    expect(backend.tokens.get("tok2")!.nameplate).toBe("B");
  });

  it("a selected token reveals its hover values", () => {
    const { backend, view, host } = rig(lookup({ tok1: { nameplate: "Goblin", nameplate_mode: "hover", bars: [bar("hover")] } }));
    host.selected = new Set(["tok1"]);
    view.reconcile();
    expect(backend.tokens.get("tok1")!.nameplate).toBe("Goblin");
    expect(backend.tokens.get("tok1")!.bars).toHaveLength(1);
  });

  it("an owner-mode value draws without hover, and a hover bar beside it stays hidden", () => {
    const { backend } = rig(lookup({ tok1: { nameplate: null, nameplate_mode: null, bars: [bar("owner"), bar("hover")] } }));
    expect(backend.tokens.get("tok1")!.bars).toHaveLength(1);
  });

  it("a delivered nameplate that names nothing draws the host's fallback label", () => {
    const { backend } = rig(lookup({ tok1: { nameplate: null, nameplate_mode: "always", bars: [] } }));
    expect(backend.tokens.get("tok1")!.nameplate).toBe("Unknown");
  });

  it("a nameplate the server did not deliver draws nothing, not the fallback", () => {
    const { backend } = rig(lookup({ tok1: { nameplate: null, nameplate_mode: null, bars: [bar("always")] } }));
    expect(backend.tokens.get("tok1")!.nameplate).toBeUndefined();
    expect(backend.tokens.get("tok1")!.bars).toHaveLength(1);
  });

  it("the budget flag off drops every nameplate and bar", () => {
    const { backend, view, host } = rig(lookup({ tok1: { nameplate: "Goblin", nameplate_mode: "always", bars: [bar("always")] } }));
    host.enabled = false;
    view.reconcile();
    expect(backend.tokens.get("tok1")!.nameplate).toBeUndefined();
    expect(backend.tokens.get("tok1")!.bars).toBeUndefined();
  });

  it("tokenAt picks the topmost tracked token under a point and null elsewhere", () => {
    const { view } = rig(EMPTY_BARS);
    expect(view.tokenAt({ x: 10, y: 10 })).toBe("tok1");
    expect(view.tokenAt({ x: 510, y: 10 })).toBe("tok2");
    expect(view.tokenAt({ x: 300, y: 300 })).toBeNull();
  });
});
