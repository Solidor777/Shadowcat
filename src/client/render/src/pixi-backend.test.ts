import { describe, test, expect } from "vitest";
import { Container } from "pixi.js";
import type { Application } from "pixi.js";
import { PixiBackend } from "./pixi-backend";
import type { LightingFrame } from "./lighting";
import type { TokenNodeSpec } from "./types";

/** A minimal stand-in satisfying `PixiBackend`'s constructor: it reads only `app.stage.addChild`.
 * `Container` is GL-free (a pure scene-graph node), so this constructs a real `PixiBackend`
 * without a WebGL context or `Application.init()`.
 * @returns A `PixiBackend` constructed over a stub `Application`.
 */
function headlessBackend(): PixiBackend {
  const fakeApp = { stage: new Container() } as unknown as Application;
  return new PixiBackend(fakeApp);
}

interface GraphicsInstructionLog {
  context: {
    instructions: { data: { path: { instructions: { action: string; data: unknown[] }[] } } }[];
  };
}

/** Extracts the flat point arrays passed to every `poly(...)` draw call recorded on a Graphics'
 * context — the same instruction log `Graphics` builds without a GL context, so this reads what
 * `setLighting` actually drew.
 * @param backend A `PixiBackend`; only its private `lightingGraphics` field is read.
 * @returns One flat `[x0,y0,x1,y1,…]` array per `poly(...)` call, in draw order.
 */
function polyDraws(backend: PixiBackend): number[][] {
  // Reads a private field to inspect the recorded draw instructions — no public accessor
  // exists, and this test's whole point is pinning what the backend actually draws.
  const graphics = (backend as unknown as { lightingGraphics: GraphicsInstructionLog }).lightingGraphics;
  return graphics.context.instructions
    .map((i) => i.data.path.instructions.find((pi) => pi.action === "poly"))
    .filter((pi): pi is { action: string; data: unknown[] } => pi !== undefined)
    .map((pi) => pi.data[0] as number[]);
}

describe("PixiBackend.setLighting", () => {
  test("draws each cell's own poly geometry from LitDrawCell.corners, not an index*cellSize rect", () => {
    const backend = headlessBackend();
    // i=5, j=3 at cell=70 would rect-anchor at (350,210) under an index*cellSize scheme; these
    // corners are deliberately offset from that so the two paint strategies produce different
    // draw calls.
    const corners = [{ x: 111, y: 222 }, { x: 333, y: 222 }, { x: 333, y: 444 }];
    const frame: LightingFrame = {
      cell: 70,
      cells: [{ i: 5, j: 3, alpha: 0.4, tint: 0x112233, tintAlpha: 0.25, desaturate: false, corners }],
    };
    backend.setLighting(frame);
    const draws = polyDraws(backend);
    expect(draws.length).toBeGreaterThan(0);
    const expected = corners.flatMap((p) => [p.x, p.y]);
    for (const d of draws) expect(d).toEqual(expected);
  });
});

interface TokenNodeLike {
  /** Mirror of `TokenNode.visualContainer` — read to assert the frame's child order. */
  visualContainer: Container;
  /** Mirror of `TokenNode.visual` — read to assert mask assignment. */
  visual: { mask: unknown };
  /** Mirror of `TokenNode.generated` — read to assert frame lifetime. */
  generated: { background: GraphicsInstructionLog; mask: GraphicsInstructionLog; ring: GraphicsInstructionLog } | null;
}

interface PixiBackendTokenInternals {
  /** Mirror of the private `PixiBackend.createTokenNode`. */
  createTokenNode(id: string): TokenNodeLike;
  /** Mirror of the private `PixiBackend.updateTokenGeneratedFrame`. */
  updateTokenGeneratedFrame(node: TokenNodeLike, spec: TokenNodeSpec): void;
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
function styleOf(g: GraphicsInstructionLog, action: "fill" | "stroke"): { width?: number; color?: number } | null {
  const instr = g.context.instructions.find((i) => (i as unknown as { action?: string }).action === action);
  return instr ? (instr.data as unknown as { style: { width?: number; color?: number } }).style : null;
}

describe("PixiBackend.updateTokenGeneratedFrame", () => {
  const base: Omit<TokenNodeSpec, "visual"> = { x: 0, y: 0, w: 100, h: 50, rotation: 0, borderColor: null, badges: [], shape: "square" };

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
