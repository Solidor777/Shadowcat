// @vitest-environment node
import { describe, it, expect } from "vitest";
import { shapeGeometry, type ShapeGeometry } from "./geometry";
import type { DieShapeId } from "./shapes";

type Vec3 = [number, number, number];

function vertexAt(geom: ShapeGeometry, index: number): Vec3 {
  return [geom.vertices[index * 3], geom.vertices[index * 3 + 1], geom.vertices[index * 3 + 2]];
}

function dot(a: Vec3, b: Vec3): number {
  return a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
}

const EXPECTED: Record<DieShapeId, { vertices: number; faces: number; sides: number }> = {
  d4: { vertices: 4, faces: 4, sides: 3 },
  d6: { vertices: 8, faces: 6, sides: 4 },
  d8: { vertices: 6, faces: 8, sides: 3 },
  d10: { vertices: 12, faces: 10, sides: 4 },
  d12: { vertices: 20, faces: 12, sides: 5 },
  d20: { vertices: 12, faces: 20, sides: 3 },
};

const SHAPES = Object.keys(EXPECTED) as DieShapeId[];

describe("shapeGeometry", () => {
  it.each(SHAPES)("%s: the expected vertex and face counts, every face the expected polygon size", (shape) => {
    const geom = shapeGeometry(shape);
    const { vertices, faces, sides } = EXPECTED[shape];
    expect(geom.vertices.length).toBe(vertices * 3);
    expect(geom.faces).toHaveLength(faces);
    for (const face of geom.faces) {
      expect(face.indices).toHaveLength(sides);
      expect(new Set(face.indices).size).toBe(sides);
      for (const i of face.indices) {
        expect(i).toBeGreaterThanOrEqual(0);
        expect(i).toBeLessThan(vertices);
      }
    }
  });

  it.each(SHAPES)("%s: every face is planar and every other vertex lies behind its plane", (shape) => {
    const geom = shapeGeometry(shape);
    const vertexCount = geom.vertices.length / 3;
    for (const face of geom.faces) {
      const heights = face.indices.map((i) => dot(vertexAt(geom, i), face.normal));
      const h = heights[0];
      // Planarity of the face's own polygon — the real check on the d10's
      // golden-ratio apex height, whose kites are explicit (not plane-derived).
      for (const height of heights) expect(Math.abs(height - h)).toBeLessThan(1e-6);
      // Containment: no vertex of the cloud sits OUTSIDE the face plane, so the
      // face is a genuine convex-hull face.
      for (let i = 0; i < vertexCount; i++) {
        expect(dot(vertexAt(geom, i), face.normal)).toBeLessThanOrEqual(h + 1e-6);
      }
    }
  });

  it.each(SHAPES)("%s: every normal is unit length and points outward", (shape) => {
    const geom = shapeGeometry(shape);
    for (const face of geom.faces) {
      expect(Math.hypot(...face.normal)).toBeCloseTo(1, 9);
      const centroid: Vec3 = [0, 0, 0];
      for (const i of face.indices) {
        const v = vertexAt(geom, i);
        centroid[0] += v[0] / face.indices.length;
        centroid[1] += v[1] / face.indices.length;
        centroid[2] += v[2] / face.indices.length;
      }
      expect(dot(centroid, face.normal)).toBeGreaterThan(0);
    }
  });

  it("is memoized per shape id", () => {
    expect(shapeGeometry("d6")).toBe(shapeGeometry("d6"));
  });

  it("d6 face index 2 is the +Y face (the identity-rotation up face)", () => {
    // Pins the d6 direction-table order `upFaceIndex`'s settle remap reads through.
    expect(shapeGeometry("d6").faces[2].normal).toEqual([0, 1, 0]);
  });
});
