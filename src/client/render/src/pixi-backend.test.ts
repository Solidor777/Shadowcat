import { describe, test, expect, vi } from "vitest";
import { Container } from "pixi.js";
import type { Application } from "pixi.js";
import { PixiBackend } from "./pixi-backend";
import type { LightingFrame } from "./lighting";
import type { TokenNodeSpec, VisibilityInput } from "./types";

vi.mock("pixi.js", async (importActual) => {
  const actual = await importActual<typeof import("pixi.js")>();
  /** Headless stand-in for the real `ColorMatrixFilter` (whose constructor compiles shader
   * programs and so needs a GL context): records the assigned matrix verbatim and latches
   * `_destroyed` on `destroy`, so `PixiBackend.updateTokenFx`'s plumbing is testable here. The
   * matrix math itself is the REAL `composeTokenFxMatrix` — only filter construction is stubbed. */
  class StubColorMatrixFilter {
    /** The last assigned `matrix`, recorded verbatim. */
    matrix: number[] = [];
    /** Latch set by `destroy` — mirrors `Shader.destroy`'s private `_destroyed`. */
    _destroyed = false;
    /** Record destruction (see `_destroyed`). */
    destroy(): void {
      this._destroyed = true;
    }
  }
  return { ...actual, ColorMatrixFilter: StubColorMatrixFilter };
});

/** A minimal stand-in satisfying `PixiBackend`'s constructor: it reads only `app.stage.addChild`.
 * `Container` is GL-free (a pure scene-graph node), so this constructs a real `PixiBackend`
 * without a WebGL context or `Application.init()`.
 * @returns A `PixiBackend` constructed over a stub `Application`.
 */
function headlessBackend(): PixiBackend {
  const fakeApp = { stage: new Container() } as unknown as Application;
  return new PixiBackend(fakeApp);
}

/** A headless backend whose `app.renderer.render` is a counting stub instead of a real GPU
 * call — enough for `setVisibilityBlend`'s `captureFog` path to run (it only needs
 * `app.screen`/`app.renderer.resolution`/`app.renderer.render`/`RenderTexture.create`, none of
 * which need a live GL context) while exposing how many full render-to-texture passes it
 * performed.
 * @returns The backend plus a `renderCalls` counter incremented once per `app.renderer.render`.
 */
function headlessBackendWithRenderCounter(): { backend: PixiBackend; renderCalls: { count: number } } {
  const renderCalls = { count: 0 };
  const fakeApp = {
    stage: new Container(),
    screen: { width: 800, height: 600 },
    renderer: {
      resolution: 1,
      render: () => {
        renderCalls.count++;
      },
    },
  } as unknown as Application;
  return { backend: new PixiBackend(fakeApp), renderCalls };
}

interface GraphicsInstructionLog {
  context: {
    instructions: { data: { path: { instructions: { action: string; data: unknown[] }[] } } }[];
  };
}

/** Extracts the flat point arrays passed to every `poly(...)` draw call recorded on one of the
 * backend's lighting-layer Graphics — the same instruction log `Graphics` builds without a GL
 * context, so this reads what `setLighting` actually drew.
 * @param backend A `PixiBackend`; only the named private Graphics field is read.
 * @param field Which lighting Graphics to read: the per-cell fills, the darkness sheet, or
 * the lit-cell holes cut from it.
 * @returns One flat `[x0,y0,x1,y1,…]` array per `poly(...)` call, in draw order.
 */
function polyDraws(backend: PixiBackend, field: "lightingGraphics" | "darknessGraphics" | "litHoles" = "lightingGraphics"): number[][] {
  // Reads a private field to inspect the recorded draw instructions — no public accessor
  // exists, and this test's whole point is pinning what the backend actually draws.
  const graphics = (backend as unknown as Record<typeof field, GraphicsInstructionLog>)[field];
  return graphics.context.instructions
    .map((i) => i.data.path.instructions.find((pi) => pi.action === "poly"))
    .filter((pi): pi is { action: string; data: unknown[] } => pi !== undefined)
    .map((pi) => pi.data[0] as number[]);
}

describe("PixiBackend.setLighting", () => {
  test("with no lit cell the darkness sheet paints whole and UNMASKED; with one it is inverse-masked by the holes", () => {
    const backend = headlessBackend();
    const priv = backend as unknown as { darknessGraphics: Container; litHoles: Container };
    const dark = { points: [0, 0, 300, 0, 300, 300, 0, 300] };
    backend.setLighting({ cell: 100, cells: [], darkness: [dark] });
    expect(polyDraws(backend, "darknessGraphics")).toEqual([dark.points]);
    // Pixi reports a cleared mask as `undefined` (no mask effect attached), never a Graphics.
    expect(priv.darknessGraphics.mask ?? null).toBeNull();
    const corners = [{ x: 0, y: 0 }, { x: 100, y: 0 }, { x: 100, y: 100 }, { x: 0, y: 100 }];
    backend.setLighting({ cell: 100, cells: [{ i: 0, j: 0, alpha: 0, tint: 0, tintAlpha: 0, desaturate: false, corners }], darkness: [dark] });
    expect(priv.darknessGraphics.mask).toBe(priv.litHoles);
    // A degenerate cell cuts no hole, so it counts as "nothing lit" too.
    backend.setLighting({ cell: 100, cells: [{ i: 0, j: 0, alpha: 0, tint: 0, tintAlpha: 0, desaturate: false, corners: corners.slice(0, 2) }], darkness: [dark] });
    expect(priv.darknessGraphics.mask ?? null).toBeNull();
  });

  test("draws each cell's own poly geometry from LitDrawCell.corners, not an index*cellSize rect", () => {
    const backend = headlessBackend();
    // i=5, j=3 at cell=70 would rect-anchor at (350,210) under an index*cellSize scheme; these
    // corners are deliberately offset from that so the two paint strategies produce different
    // draw calls.
    const corners = [{ x: 111, y: 222 }, { x: 333, y: 222 }, { x: 333, y: 444 }];
    const frame: LightingFrame = {
      cell: 70,
      cells: [{ i: 5, j: 3, alpha: 0.4, tint: 0x112233, tintAlpha: 0.25, desaturate: false, corners }],
      darkness: [],
    };
    backend.setLighting(frame);
    const draws = polyDraws(backend);
    expect(draws.length).toBeGreaterThan(0);
    const expected = corners.flatMap((p) => [p.x, p.y]);
    for (const d of draws) expect(d).toEqual(expected);
    expect(polyDraws(backend, "darknessGraphics")).toEqual([]);
    expect(polyDraws(backend, "litHoles")).toEqual([expected]);
  });

  test("paints the darkness regions as a sheet with every lit cell cut out as a hole", () => {
    const backend = headlessBackend();
    const corners = [{ x: 0, y: 0 }, { x: 70, y: 0 }, { x: 70, y: 70 }, { x: 0, y: 70 }];
    const los = [0, 0, 700, 0, 700, 700, 0, 700];
    backend.setLighting({
      cell: 70,
      cells: [
        { i: 0, j: 0, alpha: 0, tint: 0, tintAlpha: 0, desaturate: false, corners },
        { i: 9, j: 9, alpha: 0, tint: 0, tintAlpha: 0, desaturate: false, corners: [{ x: 1, y: 1 }] }, // degenerate: no hole
      ],
      darkness: [{ points: los }, { points: [5, 5, 6, 6] }], // the 2-vertex region is skipped
    });
    expect(polyDraws(backend, "darknessGraphics")).toEqual([los]);
    expect(polyDraws(backend, "litHoles")).toEqual([corners.flatMap((p) => [p.x, p.y])]);
    const sheet = (backend as unknown as { darknessGraphics: { mask: unknown }; litHoles: unknown });
    expect(sheet.darknessGraphics.mask).toBe(sheet.litHoles);
    // A later frame with no darkness clears the sheet and the holes.
    backend.setLighting({ cell: 70, cells: [], darkness: [] });
    expect(polyDraws(backend, "darknessGraphics")).toEqual([]);
    expect(polyDraws(backend, "litHoles")).toEqual([]);
  });
});

test("setClearColor writes the renderer's background clear color", () => {
  const fakeApp = {
    stage: new Container(),
    renderer: { background: { color: 0x000000 } },
  } as unknown as Application;
  const backend = new PixiBackend(fakeApp);
  backend.setClearColor(0x123456);
  expect(fakeApp.renderer.background.color).toBe(0x123456);
});

interface TokenNodeLike {
  /** Mirror of `TokenNode.container` — read to assert the aura's child order. */
  container: Container;
  /** Mirror of `TokenNode.visualContainer` — read to assert the frame's child order + fx slot. */
  visualContainer: Container;
  /** Mirror of `TokenNode.visual` — read to assert mask assignment. */
  visual: { mask: unknown };
  /** Mirror of `TokenNode.generated` — read to assert frame lifetime. */
  generated: { background: GraphicsInstructionLog; mask: GraphicsInstructionLog; ring: GraphicsInstructionLog } | null;
  /** Mirror of `TokenNode.aura` — read to assert the disc's draw + lifetime. */
  aura: (GraphicsInstructionLog & { destroyed: boolean }) | null;
  /** Mirror of `TokenNode.auraKey` — read to assert the redraw memo. */
  auraKey: string;
  /** Mirror of `TokenNode.fx` — read to assert the filter's matrix + lifetime (`_destroyed` is
   * `Shader.destroy`'s private latch; `Filter` extends `Shader` and exposes no public getter). */
  fx: { matrix: number[]; _destroyed: boolean } | null;
  /** Mirror of `TokenNode.fxKey` — read to assert the rebuild memo. */
  fxKey: string;
}

interface PixiBackendTokenInternals {
  /** Mirror of the private `PixiBackend.createTokenNode`. */
  createTokenNode(id: string): TokenNodeLike;
  /** Mirror of the private `PixiBackend.updateTokenGeneratedFrame`. */
  updateTokenGeneratedFrame(node: TokenNodeLike, spec: TokenNodeSpec): void;
  /** Mirror of the private `PixiBackend.updateTokenAura`. */
  updateTokenAura(node: TokenNodeLike, spec: TokenNodeSpec): void;
  /** Mirror of the private `PixiBackend.updateTokenFx`. */
  updateTokenFx(node: TokenNodeLike, spec: TokenNodeSpec): void;
}

/** Reads the flat draw-call action list recorded on a Graphics' context — the same instruction
 * log `polyDraws` reads for `setLighting`, widened to every path action (`ellipse`/`rect`/…).
 * @param g A Graphics (via its instruction-log mirror); only its private `context` is read.
 * @returns One action name per path instruction, in draw order.
 */
function pathActions(g: GraphicsInstructionLog): string[] {
  return g.context.instructions.flatMap((i) => i.data.path.instructions.map((pi) => pi.action));
}

/** Reads the style recorded on a Graphics' fill/stroke context instruction of the given action.
 * @param g A Graphics (via its instruction-log mirror); only its private `context` is read.
 * @param action The instruction action to find (`"fill"` or `"stroke"`).
 * @returns The instruction's converted style, or `null` when no such instruction exists.
 */
function styleOf(g: GraphicsInstructionLog, action: "fill" | "stroke"): { width?: number; color?: number; alpha?: number } | null {
  const instr = g.context.instructions.find((i) => (i as unknown as { action?: string }).action === action);
  return instr ? (instr.data as unknown as { style: { width?: number; color?: number; alpha?: number } }).style : null;
}

describe("PixiBackend.setVisibilityBlend", () => {
  test("reuses both captured RenderTextures across many ticks holding the same (from, to) sample pair — only the blend factor moves", () => {
    // Mirrors RenderEngine.applyVisionSweep's per-tick call shape: a sweep holds the same
    // (from, to) sample pair for many consecutive ticks, only `factor` advancing.
    const { backend, renderCalls } = headlessBackendWithRenderCounter();
    const from: VisibilityInput = { mode: "masked", visible: [{ points: [0, 0, 10, 0, 10, 10, 0, 10] }], explored: [], perceived: [] };
    const to: VisibilityInput = { mode: "masked", visible: [{ points: [0, 0, 20, 0, 20, 20, 0, 20] }], explored: [], perceived: [] };
    for (let i = 0; i < 30; i++) backend.setVisibilityBlend(from, to, i / 30);
    // One GPU render-to-texture pass per endpoint (from, to) — never one pair per tick.
    expect(renderCalls.count).toBeLessThan(5);
  });

  test("captures a fresh pair whenever the (from, to) content actually changes — render count scales with DISTINCT pairs, not tick count", () => {
    // A cache that stops discriminating (e.g. a key collapsed to a constant) would pass the
    // "holds the same pair" test above and show an even LARGER render-count reduction here,
    // while silently reusing a stale rasterized texture for content that genuinely changed —
    // exactly the failure `visibilityInputKey`'s own discrimination tests guard against.
    const { backend, renderCalls } = headlessBackendWithRenderCounter();
    const endpoints: VisibilityInput[] = Array.from({ length: 6 }, (_, i) => ({
      mode: "masked",
      visible: [{ points: [0, 0, (i + 1) * 10, 0, (i + 1) * 10, (i + 1) * 10, 0, (i + 1) * 10] }],
      explored: [],
      perceived: [],
    }));
    const ticksPerPair = 6;
    const pairCount = endpoints.length - 1;
    for (let pair = 0; pair < pairCount; pair++) {
      for (let tick = 0; tick < ticksPerPair; tick++) {
        backend.setVisibilityBlend(endpoints[pair], endpoints[pair + 1], tick / ticksPerPair);
      }
    }
    // Exactly one fresh capture per endpoint (2 per pair transition), never per tick: a
    // content-blind cache would report far fewer than 2*pairCount; a never-caching one would
    // report 2*pairCount*ticksPerPair (60).
    expect(renderCalls.count).toBe(2 * pairCount);
  });
});

describe("PixiBackend.updateTokenGeneratedFrame", () => {
  const base: Omit<TokenNodeSpec, "visual"> = { x: 0, y: 0, w: 100, h: 50, rotation: 0, borderColor: null, badges: [], shape: "square", perceived: false };

  test("composes background → masked art → ring inside visualContainer, under the faction border", () => {
    const backend = headlessBackend() as unknown as PixiBackendTokenInternals;
    const node = backend.createTokenNode("t1");
    backend.updateTokenGeneratedFrame(node, {
      ...base,
      visual: { kind: "generated", art: { kind: "image", url: "u" }, crop: "circle", border: { color: 0xff8800, width: 0.1 }, background: { color: 0x102030 } },
    });
    const frame = node.generated!;
    expect(frame).not.toBeNull();
    // Child order: background under the art, mask + ring above it, faction border topmost.
    expect(node.visualContainer.children[0]).toBe(frame.background);
    expect(node.visualContainer.children[1]).toBe(node.visual);
    expect(node.visualContainer.children[2]).toBe(frame.mask);
    expect(node.visualContainer.children[3]).toBe(frame.ring);
    expect(node.visual.mask).toBe(frame.mask);
    // Circle crop: every frame shape is the inscribed ellipse of the extent.
    expect(pathActions(frame.mask)).toContain("ellipse");
    expect(pathActions(frame.background)).toContain("ellipse");
    expect(pathActions(frame.ring)).toContain("ellipse");
    // Background fill color passes through; ring width scales the authored fraction by the
    // token's smaller extent (0.1 × min(100, 50)).
    expect(styleOf(frame.background, "fill")?.color).toBe(0x102030);
    expect(styleOf(frame.ring, "stroke")?.width).toBeCloseTo(5);
    expect(styleOf(frame.ring, "stroke")?.color).toBe(0xff8800);
  });

  test("square crop draws the extent rect; omitted border/background leave their Graphics empty", () => {
    const backend = headlessBackend() as unknown as PixiBackendTokenInternals;
    const node = backend.createTokenNode("t1");
    backend.updateTokenGeneratedFrame(node, {
      ...base,
      visual: { kind: "generated", art: { kind: "image", url: "u" }, crop: "square" },
    });
    const frame = node.generated!;
    expect(pathActions(frame.mask)).toContain("rect");
    expect(pathActions(frame.mask)).not.toContain("ellipse");
    expect(styleOf(frame.background, "fill")).toBeNull();
    expect(styleOf(frame.ring, "stroke")).toBeNull();
  });

  test("redraws against the current extent on a size-only re-push", () => {
    const backend = headlessBackend() as unknown as PixiBackendTokenInternals;
    const node = backend.createTokenNode("t1");
    const visual: TokenNodeSpec["visual"] = { kind: "generated", art: { kind: "image", url: "u" }, crop: "circle", border: { color: 0xff8800, width: 0.1 } };
    backend.updateTokenGeneratedFrame(node, { ...base, visual });
    backend.updateTokenGeneratedFrame(node, { ...base, w: 200, h: 200, visual });
    expect(styleOf(node.generated!.ring, "stroke")?.width).toBeCloseTo(20);
  });

  test("swapping to a non-generated visual drops the mask and destroys the frame", () => {
    const backend = headlessBackend() as unknown as PixiBackendTokenInternals;
    const node = backend.createTokenNode("t1");
    backend.updateTokenGeneratedFrame(node, {
      ...base,
      visual: { kind: "generated", art: { kind: "image", url: "u" }, crop: "circle" },
    });
    const frame = node.generated!;
    backend.updateTokenGeneratedFrame(node, { ...base, visual: { kind: "image", url: "u" } });
    expect(node.generated).toBeNull();
    // Pixi's mask getter returns `undefined` (not `null`) once the mask effect is removed.
    expect(node.visual.mask).toBeUndefined();
    expect((frame.mask as unknown as { destroyed: boolean }).destroyed).toBe(true);
  });
});

describe("PixiBackend.updateTokenAura", () => {
  const base: Omit<TokenNodeSpec, "visual"> = { x: 0, y: 0, w: 100, h: 50, rotation: 0, borderColor: null, badges: [], shape: "square", perceived: false };
  const imageVisual: TokenNodeSpec["visual"] = { kind: "image", url: "u" };

  test("draws the disc as the container's bottom-most child with the given color + opacity", () => {
    const backend = headlessBackend() as unknown as PixiBackendTokenInternals;
    const node = backend.createTokenNode("t1");
    backend.updateTokenAura(node, { ...base, visual: imageVisual, aura: { color: 0xffcc66, opacity: 0.4, radius: 140 } });
    const aura = node.aura!;
    expect(aura).not.toBeNull();
    // Child order: aura first (the art's visualContainer draws over it; badges stay on top).
    expect(node.container.children[0]).toBe(aura);
    expect(pathActions(aura)).toContain("ellipse");
    const fill = styleOf(aura, "fill");
    expect(fill?.color).toBe(0xffcc66);
    expect(fill?.alpha).toBeCloseTo(0.4);
  });

  test("an unchanged aura key skips the redraw; a changed one redraws in place", () => {
    const backend = headlessBackend() as unknown as PixiBackendTokenInternals;
    const node = backend.createTokenNode("t1");
    backend.updateTokenAura(node, { ...base, visual: imageVisual, aura: { color: 0xffcc66, opacity: 0.4, radius: 140 } });
    const aura = node.aura!;
    // Memoized: a same-key re-push leaves the recorded instructions untouched (still one pass).
    backend.updateTokenAura(node, { ...base, visual: imageVisual, aura: { color: 0xffcc66, opacity: 0.4, radius: 140 } });
    expect(node.aura).toBe(aura);
    expect(aura.context.instructions.length).toBe(1);
    // A changed key redraws on the SAME Graphics (clear + re-fill — still one fill instruction set).
    backend.updateTokenAura(node, { ...base, visual: imageVisual, aura: { color: 0x0000ff, opacity: 0.4, radius: 140 } });
    expect(node.aura).toBe(aura);
    expect(styleOf(aura, "fill")?.color).toBe(0x0000ff);
    expect(node.auraKey).toBe(`${0x0000ff}:0.4:140`);
  });

  test("an absent aura destroys the disc and drops the reference", () => {
    const backend = headlessBackend() as unknown as PixiBackendTokenInternals;
    const node = backend.createTokenNode("t1");
    backend.updateTokenAura(node, { ...base, visual: imageVisual, aura: { color: 0xffcc66, opacity: 0.4, radius: 140 } });
    const aura = node.aura!;
    backend.updateTokenAura(node, { ...base, visual: imageVisual });
    expect(node.aura).toBeNull();
    expect(aura.destroyed).toBe(true);
    // And a token that never had an aura leaves no Graphics behind.
    const node2 = backend.createTokenNode("t2");
    backend.updateTokenAura(node2, { ...base, visual: imageVisual });
    expect(node2.aura).toBeNull();
    expect(node2.container.children.length).toBe(1); // visualContainer only
  });
});

describe("PixiBackend.updateTokenFx", () => {
  const base: Omit<TokenNodeSpec, "visual"> = { x: 0, y: 0, w: 100, h: 50, rotation: 0, borderColor: null, badges: [], shape: "square", perceived: false };
  const imageVisual: TokenNodeSpec["visual"] = { kind: "image", url: "u" };

  test("composes the fx list into one ColorMatrixFilter on the visualContainer", () => {
    const backend = headlessBackend() as unknown as PixiBackendTokenInternals;
    const node = backend.createTokenNode("t1");
    backend.updateTokenFx(node, { ...base, visual: imageVisual, fx: [{ kind: "tint", color: 0xff0000, strength: 0.5 }] });
    const fx = node.fx!;
    expect(fx).not.toBeNull();
    expect(node.visualContainer.filters).toEqual([fx]);
    // A half-strength red tint: the red row stays identity, the green/blue diagonals drop to 0.5.
    expect(fx.matrix[0]).toBeCloseTo(1);
    expect(fx.matrix[6]).toBeCloseTo(0.5);
    expect(fx.matrix[12]).toBeCloseTo(0.5);
  });

  test("desaturate collapses every RGB output row to the equal-thirds mean; highlight scales and offsets", () => {
    const backend = headlessBackend() as unknown as PixiBackendTokenInternals;
    const node = backend.createTokenNode("t1");
    backend.updateTokenFx(node, { ...base, visual: imageVisual, fx: [{ kind: "desaturate" }] });
    const m = node.fx!.matrix;
    for (const row of [0, 1, 2]) {
      expect(m[row * 5]).toBeCloseTo(1 / 3);
      expect(m[row * 5 + 1]).toBeCloseTo(1 / 3);
      expect(m[row * 5 + 2]).toBeCloseTo(1 / 3);
    }
    backend.updateTokenFx(node, { ...base, visual: imageVisual, fx: [{ kind: "highlight", color: 0xffd400, strength: 0.4 }] });
    const h = node.fx!.matrix;
    expect(h[0]).toBeCloseTo(0.6); // scale: 1 - strength
    expect(h[4]).toBeCloseTo(0.4 * (0xff / 255)); // offset: strength × red channel
    expect(h[9]).toBeCloseTo(0.4 * (0xd4 / 255));
    expect(h[14]).toBeCloseTo(0); // blue channel of 0xffd400
  });

  test("later entries transform the output of earlier ones (array order)", () => {
    const backend = headlessBackend() as unknown as PixiBackendTokenInternals;
    const node = backend.createTokenNode("t1");
    // Desaturate first, then a full-strength green tint: the composed green/blue diagonals
    // read (1/3, 0) — the tint scaling the desaturated mean — not the tint's own (1, 0).
    backend.updateTokenFx(node, { ...base, visual: imageVisual, fx: [{ kind: "desaturate" }, { kind: "tint", color: 0x00ff00, strength: 1 }] });
    const m = node.fx!.matrix;
    expect(m[0]).toBeCloseTo(0); // red row scaled to 0 by the full green tint
    expect(m[6]).toBeCloseTo(1 / 3); // green row keeps the desaturated mean of green
    expect(m[7]).toBeCloseTo(1 / 3); // green row keeps the desaturated mean of blue
    expect(m[12]).toBeCloseTo(0); // blue row scaled to 0
  });

  test("an unchanged fx key skips the rebuild; a changed one rebuilds the SAME filter instance in place", () => {
    const backend = headlessBackend() as unknown as PixiBackendTokenInternals;
    const node = backend.createTokenNode("t1");
    const spec = { ...base, visual: imageVisual, fx: [{ kind: "tint" as const, color: 0xff0000, strength: 0.5 }] };
    backend.updateTokenFx(node, spec);
    const fx = node.fx!;
    backend.updateTokenFx(node, { ...spec, fx: [{ kind: "tint" as const, color: 0xff0000, strength: 0.5 }] });
    expect(node.fx).toBe(fx); // memoized: same instance, no rebuild
    backend.updateTokenFx(node, { ...spec, fx: [{ kind: "tint" as const, color: 0x00ff00, strength: 0.5 }] });
    expect(node.fx).toBe(fx); // rebuilt in place, not swapped
    expect(fx.matrix[6]).toBeCloseTo(1);
    expect(fx.matrix[0]).toBeCloseTo(0.5);
  });

  test("an absent fx list destroys the filter, clears the slot, and drops the reference", () => {
    const backend = headlessBackend() as unknown as PixiBackendTokenInternals;
    const node = backend.createTokenNode("t1");
    backend.updateTokenFx(node, { ...base, visual: imageVisual, fx: [{ kind: "desaturate" }] });
    const fx = node.fx!;
    backend.updateTokenFx(node, { ...base, visual: imageVisual });
    expect(node.fx).toBeNull();
    expect(fx._destroyed).toBe(true);
    expect(node.visualContainer.filters).toEqual([]);
    // And a token that never had fx leaves no filter behind (the filters slot is never touched —
    // a fresh Container's `filters` getter reads `undefined`).
    const node2 = backend.createTokenNode("t2");
    backend.updateTokenFx(node2, { ...base, visual: imageVisual });
    expect(node2.fx).toBeNull();
    expect(node2.visualContainer.filters ?? []).toEqual([]);
  });

  test("removeToken destroys the fx filter (it is not a display-list child)", () => {
    const backend = headlessBackend();
    const internals = backend as unknown as PixiBackendTokenInternals;
    const node = internals.createTokenNode("t1");
    internals.updateTokenFx(node, { ...base, visual: imageVisual, fx: [{ kind: "desaturate" }] });
    const fx = node.fx!;
    backend.removeToken("t1");
    expect(fx._destroyed).toBe(true);
  });
});
