import { describe, test, expect } from "vitest";
import { Container } from "pixi.js";
import type { Application } from "pixi.js";
import { PixiBackend } from "./pixi-backend";
import type { LightingFrame } from "./lighting";

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
