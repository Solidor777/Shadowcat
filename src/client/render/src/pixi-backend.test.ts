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
