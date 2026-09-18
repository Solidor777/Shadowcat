import { describe, it, expect, vi, beforeEach } from "vitest";

const rigidBodies: unknown[] = [];

function makeRapierMock() {
  const RigidBodyDesc = {
    fixed: () => ({ setTranslation: () => RigidBodyDesc.fixed() }),
    dynamic: () => {
      const desc = {
        setTranslation: () => desc,
        setLinvel: () => desc,
        setAngvel: () => desc,
      };
      return desc;
    },
  };
  const ColliderDesc = {
    cuboid: () => ({}),
    ball: () => ({}),
    convexHull: () => ({}),
  };
  class World {
    createRigidBody(): { linvel: () => { x: number; y: number; z: number }; angvel: () => { x: number; y: number; z: number }; setEnabled: () => void; rotation: () => { x: number; y: number; z: number; w: number } } {
      const body = {
        linvel: () => ({ x: 0, y: 0, z: 0 }),
        angvel: () => ({ x: 0, y: 0, z: 0 }),
        setEnabled: vi.fn(),
        rotation: () => ({ x: 0, y: 0, z: 0, w: 1 }),
      };
      rigidBodies.push(body);
      return body;
    }
    createCollider(): void {}
    step(): void {}
    free(): void {}
  }
  return { RigidBodyDesc, ColliderDesc, World, init: vi.fn().mockResolvedValue(undefined) };
}

// Every mock implementation below is invoked as `new THREE.X(...)` by `DiceEngine`, so each
// must be a real (non-arrow) function — an arrow function is never constructible in JS and
// `new` on one throws `TypeError: ... is not a constructor`.
vi.mock("three", () => ({
  WebGLRenderer: vi.fn().mockImplementation(function () {
    return { setSize: vi.fn(), render: vi.fn(), dispose: vi.fn() };
  }),
  Scene: vi.fn().mockImplementation(function () {
    return { add: vi.fn() };
  }),
  PerspectiveCamera: vi.fn().mockImplementation(function () {
    return { position: { set: vi.fn() }, lookAt: vi.fn(), aspect: 1, updateProjectionMatrix: vi.fn() };
  }),
  Mesh: vi.fn().mockImplementation(function (geometry: unknown, material: unknown) {
    return { geometry, material };
  }),
  MeshStandardMaterial: vi.fn().mockImplementation(function (opts: { map?: unknown }) {
    return { map: opts.map, needsUpdate: false };
  }),
  CanvasTexture: vi.fn().mockImplementation(function (canvas: unknown) {
    return { canvas, needsUpdate: false, dispose: vi.fn() };
  }),
  BufferGeometry: vi.fn().mockImplementation(function () {
    return { setAttribute: vi.fn(), addGroup: vi.fn(), computeVertexNormals: vi.fn() };
  }),
  Float32BufferAttribute: vi.fn().mockImplementation(function (array: unknown, itemSize: number) {
    return { array, itemSize };
  }),
}));
vi.mock("@dimforge/rapier3d-compat", () => makeRapierMock());

import { DiceEngine } from "./DiceEngine";

describe("DiceEngine", () => {
  beforeEach(() => {
    rigidBodies.length = 0;
    vi.useFakeTimers();
  });

  it("resolves throwDice once every body reports zero velocity", async () => {
    const canvas = document.createElement("canvas");
    const engine = new DiceEngine(canvas, { antialias: true, color: "#2d6ee8", labelColor: "#ffffff" });
    await engine.init();
    const settling = engine.throwDice(
      [{ kind: { Numeric: { min: 1, max: 6 } }, labels: ["1", "2", "3", "4", "5", "6"], targetIndex: 3 }],
      "roll-1",
    );
    await vi.advanceTimersByTimeAsync(500);
    const settled = await settling;
    expect(settled).toHaveLength(1);
    // The mock body's rotation is the identity quaternion, so `upFaceIndex` picks the
    // d6 geometry's +Y face — index 2 (`shapeGeometry("d6")`'s direction-table order,
    // pinned in geometry.test.ts) — deterministically, independent of physics.
    // `remapFaces` must then place the requested targetIndex (3) exactly there.
    expect(settled[0].labelOrder[2]).toBe(3);
    engine.dispose();
  });

  it("dispose is idempotent and stops the render loop", async () => {
    const canvas = document.createElement("canvas");
    const engine = new DiceEngine(canvas, { antialias: false, color: "#2d6ee8", labelColor: "#ffffff" });
    await engine.init();
    engine.dispose();
    expect(() => engine.dispose()).not.toThrow();
  });
});
