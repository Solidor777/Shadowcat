import { remapFaces } from "./remapFaces";
import { shapeFor, realFaceCountOf, type DieShapeId } from "./shapes";
import { shapeGeometry, type ShapeGeometry } from "./geometry";
import { mulberry32, seedFromRollId } from "./rng";
import type { WireDieKind } from "@shadowcat/core";

/** Fixed physics substep, decoupled from render framerate: the world steps at a fixed
 * 60 Hz substep regardless of the render frame rate. */
const FIXED_DT = 1 / 60;
/** A body's linear+angular speed must stay below this for `SETTLE_HOLD_MS` to count as
 * settled. */
const SETTLE_EPSILON = 0.02;
/** How long a body must stay under `SETTLE_EPSILON` before it is considered settled. */
const SETTLE_HOLD_MS = 300;
/** Hard cap: settle (damped to rest) regardless of velocity past this many ms. */
const SETTLE_TIMEOUT_MS = 4000;

/** One die's request to `DiceEngine.throwDice`. */
export interface DieThrowSpec {
  /** The die's face space (drives shape + real face count). */
  kind: WireDieKind;
  /** The label strings to draw onto the physical shape's real faces, in `DieKind`'s own
   * face order (numeric faces: `String(value)` per face; symbolic faces: the face's own
   * `symbols.join(",")`, or `String(value)` for an ordered numeric-valued Faces die). */
  labels: string[];
  /** The face-list index (0-based) that must end up showing on top once the die settles —
   * resolved by the caller from the die's final `value`/`natural` (see `DiceOverlay`'s
   * per-die target derivation). */
  targetIndex: number;
}

/** One settled die's remap result, consumed by the label-texture swap. */
export interface SettledDie {
  /** Index into the `dice` array passed to `throwDice`. */
  index: number;
  /** The shape id this die rendered on. */
  shape: DieShapeId;
  /** `labelOrder[faceIndex]` — which label index each physical face now shows. */
  labelOrder: number[];
}

/** One live tumble: the physics body, its throw spec, its resolved shape/geometry, and
 * its visual mesh. */
interface LiveDie {
  /** The rapier rigid body. */
  body: import("@dimforge/rapier3d-compat").RigidBody;
  /** The throw request this die was spawned from. */
  spec: DieThrowSpec;
  /** The resolved physical shape (face counts, sameLabel flag). */
  shape: ReturnType<typeof shapeFor>;
  /** The shape's convex geometry (vertex cloud + oriented faces). */
  geom: ShapeGeometry;
  /** The visual mesh (one material per physical face). */
  mesh: import("three").Mesh;
}

/** Construction options for {@link DiceEngine} (a named interface rather than an inline
 * object-literal type so every property can be documented). */
export interface DiceEngineOpts {
  /** Whether the WebGL context multisamples. */
  antialias: boolean;
  /** Resolved die-body color (device override or theme accent). */
  color: string;
  /** Resolved label glyph color (device override or theme primary text). */
  labelColor: string;
}

/**
 * Owns one tray's worth of tumbling dice: lazily-imported `three` scene/renderer and
 * `@dimforge/rapier3d-compat` physics world, stepped at a fixed 60 Hz substep and rendered
 * on `requestAnimationFrame`. Created once per overlay mount; `dispose()` releases both the
 * WebGL context and the physics world (idle disposal is `DiceOverlay`'s concern, not this
 * class's — it owns WHEN to construct/dispose an engine, not the engine's own lifecycle).
 */
export class DiceEngine {
  /** The transparent overlay canvas the renderer draws into. */
  #canvas: HTMLCanvasElement;
  /** Whether the WebGL context multisamples. */
  #antialias: boolean;
  /** Die-body base color for the label texture's background; `readDice3DSettings().color ||
   * deviceAccentColor` at construction time (empty override falls back to the theme accent —
   * `DiceOverlay` resolves the theme token and passes it in, the `Stage.svelte` `readColor`
   * precedent, since `DiceEngine` itself never touches the DOM's computed style). */
  #deviceColor: string;
  /** Label glyph color; `readDice3DSettings().labelColor || devicePrimaryTextColor`. */
  #deviceLabelColor: string;
  /** The lazily-imported `three` module, `null` until {@link init} resolves. */
  #three: typeof import("three") | null = null;
  /** The lazily-imported rapier module, `null` until {@link init} resolves. */
  #rapier: typeof import("@dimforge/rapier3d-compat") | null = null;
  /** The WebGL renderer, `null` until {@link init} and after {@link dispose}. */
  #renderer: import("three").WebGLRenderer | null = null;
  /** The three scene holding the tray's dice meshes. */
  #scene: import("three").Scene | null = null;
  /** The top-down-with-tilt camera the dice read from. */
  #camera: import("three").PerspectiveCamera | null = null;
  /** The physics world (gravity -9.81 on Y). */
  #world: import("@dimforge/rapier3d-compat").World | null = null;
  /** The render loop's current animation-frame handle, `null` when unscheduled. */
  #rafHandle: number | null = null;
  /** Set by {@link dispose}: stops the render loop and makes a racing {@link init} a no-op. */
  #disposed = false;

  /** Creates an engine bound to one overlay canvas; the WebGL context and physics
   * world are not constructed until {@link init} (or the first {@link throwDice}).
   * @param canvas The transparent overlay canvas to render into.
   * @param opts The render options (see {@link DiceEngineOpts}).
   * @example
   * ```ts
   * declare const canvas: HTMLCanvasElement;
   * const engine = new DiceEngine(canvas, { antialias: true, color: "#2d6ee8", labelColor: "#ffffff" });
   * ```
   */
  constructor(canvas: HTMLCanvasElement, opts: DiceEngineOpts) {
    this.#canvas = canvas;
    this.#antialias = opts.antialias;
    this.#deviceColor = opts.color;
    this.#deviceLabelColor = opts.labelColor;
  }

  /**
   * Lazily loads `three` and `@dimforge/rapier3d-compat` (their chunks never load for a
   * device with 3D dice off) and constructs the renderer/scene/camera/physics world. Must
   * resolve before the first {@link throwDice} call.
   * @returns Resolves once both libraries are loaded and the scene is constructed.
   * @example
   * ```ts
   * declare const engine: DiceEngine;
   * await engine.init();
   * ```
   */
  async init(): Promise<void> {
    const [THREE, RAPIER] = await Promise.all([
      import("three"),
      import("@dimforge/rapier3d-compat"),
    ]);
    if (this.#disposed) return; // teardown raced the async import
    await RAPIER.init();
    this.#three = THREE;
    this.#rapier = RAPIER;
    this.#renderer = new THREE.WebGLRenderer({
      canvas: this.#canvas,
      alpha: true,
      antialias: this.#antialias,
      powerPreference: "low-power",
    });
    this.#scene = new THREE.Scene();
    const camera = new THREE.PerspectiveCamera(35, 1, 0.1, 100);
    camera.position.set(0, 12, 6);
    camera.lookAt(0, 0, 0);
    this.#camera = camera;
    this.#world = new RAPIER.World({ x: 0, y: -9.81, z: 0 });
    this.#buildTray(RAPIER, this.#world);
    this.#loop();
  }

  /** Four invisible walls plus a ground plane, sized to the visible tray rect: the
   * visible stage rect projected onto a ground plane with four invisible walls.
   * @param RAPIER The lazily-imported rapier module (the caller already holds it).
   * @param world The physics world to build the tray into.
   * @example
   * ```ts
   * // private; invoked from `init` once the physics world exists
   * declare const RAPIER: typeof import("@dimforge/rapier3d-compat");
   * this.#buildTray(RAPIER, new RAPIER.World({ x: 0, y: -9.81, z: 0 }));
   * ```
   */
  #buildTray(RAPIER: typeof import("@dimforge/rapier3d-compat"), world: import("@dimforge/rapier3d-compat").World): void {
    const groundBody = world.createRigidBody(RAPIER.RigidBodyDesc.fixed());
    world.createCollider(RAPIER.ColliderDesc.cuboid(20, 0.1, 20), groundBody);
    const wallAt = (x: number, z: number, hx: number, hz: number): void => {
      const body = world.createRigidBody(RAPIER.RigidBodyDesc.fixed().setTranslation(x, 1, z));
      world.createCollider(RAPIER.ColliderDesc.cuboid(hx, 1, hz), body);
    };
    wallAt(20, 0, 0.5, 20);
    wallAt(-20, 0, 0.5, 20);
    wallAt(0, 20, 20, 0.5);
    wallAt(0, -20, 20, 0.5);
  }

  /** Resizes the renderer/camera to the overlay's current CSS size (`DiceOverlay`'s
   * `ResizeObserver` calls this).
   * @param width CSS pixel width.
   * @param height CSS pixel height.
   * @example
   * ```ts
   * declare const engine: DiceEngine;
   * engine.resize(800, 600);
   * ```
   */
  resize(width: number, height: number): void {
    if (!this.#renderer || !this.#camera) return;
    this.#renderer.setSize(width, height, false);
    this.#camera.aspect = width / Math.max(1, height);
    this.#camera.updateProjectionMatrix();
  }

  /**
   * Spawns and throws one roll's dice, seeded from `rollId` so every client throws the SAME
   * throw for the same roll. Resolves once every die has settled and its label texture
   * swapped to show `targetIndex` — no motion after the last frame moves.
   * @param dice The dice to throw.
   * @param rollId The roll's stable id (RNG seed source).
   * @returns Resolves with the settled remap result per die, in `dice` order.
   * @example
   * ```ts
   * declare const engine: DiceEngine;
   * await engine.throwDice(
   *   [{ kind: { Numeric: { min: 1, max: 6 } }, labels: ["1", "2", "3", "4", "5", "6"], targetIndex: 3 }],
   *   "roll-1",
   * );
   * ```
   */
  async throwDice(dice: DieThrowSpec[], rollId: string): Promise<SettledDie[]> {
    if (!this.#three || !this.#rapier || !this.#world || !this.#scene) await this.init();
    const THREE = this.#three!;
    const RAPIER = this.#rapier!;
    const world = this.#world!;
    const scene = this.#scene!;
    const next = mulberry32(seedFromRollId(rollId));

    const bodies: LiveDie[] = [];
    dice.forEach((spec, i) => {
      const shape = shapeFor(realFaceCountOf(spec.kind));
      const geom = shapeGeometry(shape.shape);
      const angle = (i / Math.max(1, dice.length)) * Math.PI * 2;
      const body = world.createRigidBody(
        RAPIER.RigidBodyDesc.dynamic()
          .setTranslation(Math.cos(angle) * 15, 3, Math.sin(angle) * 15)
          .setLinvel((next() - 0.5) * 6, 2, (next() - 0.5) * 6)
          .setAngvel({ x: (next() - 0.5) * 10, y: (next() - 0.5) * 10, z: (next() - 0.5) * 10 }),
      );
      world.createCollider(RAPIER.ColliderDesc.convexHull(geom.vertices) ?? RAPIER.ColliderDesc.ball(0.6), body);
      // One MeshStandardMaterial PER PHYSICAL FACE, each mapping a runtime-drawn canvas
      // texture: face labels are drawn at runtime onto a canvas texture per die, so
      // custom face sets need no art. `spec.labels[faceIndex]` is blank ("")
      // for a face beyond the die's own real label count (unused-face padding, `shapes.ts`).
      const geometry = meshGeometry(THREE, geom);
      const materials = Array.from({ length: shape.physicalFaceCount }, (_, faceIndex) =>
        new THREE.MeshStandardMaterial({
          map: buildLabelTexture(THREE, spec.labels[faceIndex] ?? "", this.#deviceColor, this.#deviceLabelColor),
        }),
      );
      const mesh = new THREE.Mesh(geometry, materials);
      scene.add(mesh);
      bodies.push({ body, spec, shape, geom, mesh });
    });

    return this.#stepUntilSettled(THREE, world, bodies);
  }

  /** Fixed-substep physics loop + settle detection; resolves the moment every body has
   * either gone quiet for `SETTLE_HOLD_MS` or hit `SETTLE_TIMEOUT_MS`. Swaps each settled
   * die's per-face label textures to `remapFaces`'s result BEFORE resolving — only the
   * texture is swapped after settling, no motion after the last frame moves.
   * @param THREE The lazily-imported `three` module (the caller already holds it).
   * @param world The physics world to step.
   * @param bodies The live dice to settle.
   * @returns Resolves with the settled remap result per die, in `bodies` order.
   * @example
   * ```ts
   * // private; invoked from `throwDice` after every die is spawned
   * declare const THREE: typeof import("three");
   * declare const world: import("@dimforge/rapier3d-compat").World;
   * this.#stepUntilSettled(THREE, world, []);
   * ```
   */
  #stepUntilSettled(
    THREE: typeof import("three"),
    world: import("@dimforge/rapier3d-compat").World,
    bodies: LiveDie[],
  ): Promise<SettledDie[]> {
    return new Promise((resolve) => {
      const quietSince = new Map<number, number>();
      const start = performance.now();
      const tick = (): void => {
        world.step();
        const now = performance.now();
        const elapsed = now - start;
        let allSettled = true;
        bodies.forEach((b, i) => {
          const v = b.body.linvel();
          const w = b.body.angvel();
          const speed = Math.hypot(v.x, v.y, v.z) + Math.hypot(w.x, w.y, w.z);
          if (speed < SETTLE_EPSILON) {
            if (!quietSince.has(i)) quietSince.set(i, now);
          } else {
            quietSince.delete(i);
          }
          const settled = (quietSince.has(i) && now - quietSince.get(i)! >= SETTLE_HOLD_MS) || elapsed >= SETTLE_TIMEOUT_MS;
          if (!settled) allSettled = false;
        });
        if (allSettled) {
          resolve(bodies.map((b, i) => {
            b.body.setEnabled(false); // freeze — no motion after the last frame moves
            const upIndex = upFaceIndex(b.body, b.geom);
            const labelOrder = remapFaces(upIndex, b.shape.physicalFaceCount, b.spec.targetIndex);
            // Swap each physical face's material to the label its remapped index now shows —
            // the ONLY visual change settle makes; the mesh's transform is frozen above.
            const materials = Array.isArray(b.mesh.material) ? b.mesh.material : [b.mesh.material];
            labelOrder.forEach((labelIndex, faceIndex) => {
              const mat = materials[faceIndex] as import("three").MeshStandardMaterial | undefined;
              if (!mat) return;
              mat.map?.dispose();
              mat.map = buildLabelTexture(THREE, b.spec.labels[labelIndex] ?? "", this.#deviceColor, this.#deviceLabelColor);
              mat.needsUpdate = true;
            });
            return { index: i, shape: b.shape.shape, labelOrder };
          }));
          return;
        }
        setTimeout(tick, FIXED_DT * 1000);
      };
      tick();
    });
  }

  /** Renders the scene on every animation frame until disposed.
   * @example
   * ```ts
   * // private; invoked from `init` and self-reschedules on every animation frame
   * this.#loop();
   * ```
   */
  #loop(): void {
    const render = (): void => {
      if (this.#disposed) return;
      if (this.#renderer && this.#scene && this.#camera) {
        this.#renderer.render(this.#scene, this.#camera);
      }
      this.#rafHandle = requestAnimationFrame(render);
    };
    this.#rafHandle = requestAnimationFrame(render);
  }

  /** Releases the WebGL context and the physics world. Idempotent.
   * @example
   * ```ts
   * declare const engine: DiceEngine;
   * engine.dispose();
   * ```
   */
  dispose(): void {
    this.#disposed = true;
    if (this.#rafHandle !== null) cancelAnimationFrame(this.#rafHandle);
    this.#renderer?.dispose();
    this.#world?.free();
    this.#renderer = null;
    this.#scene = null;
    this.#world = null;
  }
}

/** Builds the visual mesh's geometry from a shape's {@link ShapeGeometry}: a
 * non-indexed triangle soup (vertices duplicated per face, so
 * `computeVertexNormals` yields flat per-face shading) with one `addGroup` entry per
 * physical face — the mechanism three uses to route each triangle range to its own
 * per-face `MeshStandardMaterial` — plus planar-projected UVs so each face's label
 * texture lands centered on the face polygon.
 * @param THREE The lazily-imported `three` module (the caller already holds it).
 * @param geom The shape's convex geometry.
 * @returns A geometry with `geom.faces.length` groups, in face order.
 * @example
 * ```ts
 * declare const THREE: typeof import("three");
 * declare const geom: ShapeGeometry;
 * meshGeometry(THREE, geom);
 * ```
 */
function meshGeometry(
  THREE: typeof import("three"),
  geom: ShapeGeometry,
): import("three").BufferGeometry {
  const positions: number[] = [];
  const uvs: number[] = [];
  const geometry = new THREE.BufferGeometry();
  const vertexAt = (index: number): [number, number, number] => [
    geom.vertices[index * 3],
    geom.vertices[index * 3 + 1],
    geom.vertices[index * 3 + 2],
  ];
  geom.faces.forEach((face, faceIndex) => {
    const start = positions.length / 3;
    const pts = face.indices.map(vertexAt);
    const centroid = pts.reduce<[number, number, number]>(
      (acc, p) => [acc[0] + p[0] / pts.length, acc[1] + p[1] / pts.length, acc[2] + p[2] / pts.length],
      [0, 0, 0],
    );
    // Planar UV projection: an orthonormal in-plane basis, each vertex projected and
    // scaled so the face polygon lands inside the middle of its label texture (the
    // glyph `buildLabelTexture` draws is centered, so it reads centered on the face).
    const rel0: [number, number, number] = [pts[0][0] - centroid[0], pts[0][1] - centroid[1], pts[0][2] - centroid[2]];
    const len0 = Math.hypot(rel0[0], rel0[1], rel0[2]) || 1;
    const e1: [number, number, number] = [rel0[0] / len0, rel0[1] / len0, rel0[2] / len0];
    const e2raw = [
      face.normal[1] * e1[2] - face.normal[2] * e1[1],
      face.normal[2] * e1[0] - face.normal[0] * e1[2],
      face.normal[0] * e1[1] - face.normal[1] * e1[0],
    ];
    const coords = pts.map((p) => {
      const rel = [p[0] - centroid[0], p[1] - centroid[1], p[2] - centroid[2]];
      return [
        rel[0] * e1[0] + rel[1] * e1[1] + rel[2] * e1[2],
        rel[0] * e2raw[0] + rel[1] * e2raw[1] + rel[2] * e2raw[2],
      ];
    });
    const maxR = Math.max(...coords.map(([u, v]) => Math.hypot(u, v))) || 1;
    // Fan triangulation of the (convex, boundary-ordered) face polygon.
    for (let k = 1; k < pts.length - 1; k++) {
      for (const idx of [0, k, k + 1]) {
        positions.push(pts[idx][0], pts[idx][1], pts[idx][2]);
        uvs.push(0.5 + (0.45 * coords[idx][0]) / maxR, 0.5 + (0.45 * coords[idx][1]) / maxR);
      }
    }
    geometry.addGroup(start, positions.length / 3 - start, faceIndex);
  });
  geometry.setAttribute("position", new THREE.Float32BufferAttribute(positions, 3));
  geometry.setAttribute("uv", new THREE.Float32BufferAttribute(uvs, 2));
  geometry.computeVertexNormals();
  return geometry;
}

/** Square size (px) of one runtime-drawn label texture — small enough that a die's full
 * material set costs nothing noticeable, large enough that a label reads clearly up close. */
const LABEL_TEXTURE_SIZE = 128;

/**
 * Draws one face's label onto a 2D canvas and returns it as a `CanvasTexture` — face labels
 * are drawn at runtime onto a canvas texture per die (numbers or `Face` labels/symbols), so
 * custom face sets need no art. An empty `label` (an unused-face-padded slot,
 * `shapes.ts`) draws just the body color with no glyph — a blank face.
 * @param THREE The lazily-imported `three` module (the caller already holds it).
 * @param label The text to draw (a number, a comma-joined symbol list, or `""` for blank).
 * @param bodyColor The die body's background color (a css color string).
 * @param labelColor The glyph color (a css color string).
 * @returns A texture ready to assign to a `MeshStandardMaterial.map`.
 * @example
 * ```ts
 * declare const THREE: typeof import("three");
 * buildLabelTexture(THREE, "7", "#2d6ee8", "#ffffff");
 * ```
 */
function buildLabelTexture(
  THREE: typeof import("three"),
  label: string,
  bodyColor: string,
  labelColor: string,
): import("three").CanvasTexture {
  const canvas = document.createElement("canvas");
  canvas.width = LABEL_TEXTURE_SIZE;
  canvas.height = LABEL_TEXTURE_SIZE;
  // Guard on a null 2D context: this repo's own jsdom test convention stubs
  // `HTMLCanvasElement.prototype.getContext` to always return `null` (see
  // `src/modules/stage/vitest.setup.ts`, copied into this package's own setup),
  // so every DiceEngine unit test constructs a blank-but-valid texture here — real label
  // pixel content is Playwright-covered, never unit-tested.
  const ctx = canvas.getContext("2d");
  if (ctx) {
    ctx.fillStyle = bodyColor;
    ctx.fillRect(0, 0, LABEL_TEXTURE_SIZE, LABEL_TEXTURE_SIZE);
    if (label !== "") {
      ctx.fillStyle = labelColor;
      ctx.font = `${Math.floor(LABEL_TEXTURE_SIZE * 0.4)}px sans-serif`;
      ctx.textAlign = "center";
      ctx.textBaseline = "middle";
      ctx.fillText(label, LABEL_TEXTURE_SIZE / 2, LABEL_TEXTURE_SIZE / 2);
    }
  }
  const texture = new THREE.CanvasTexture(canvas);
  texture.needsUpdate = true;
  return texture;
}

/** Reads which physical face is currently pointing most toward world +Y (this engine's up
 * axis — three's Y-up convention, rather than +Z) from a settled body's rotation:
 * transforms every face's local outward normal (`ShapeGeometry.faces[].normal`) by the
 * body's rotation quaternion and returns the index of the maximum dot product against
 * +Y — the max-dot-of-face-normals-with-the-up-axis test.
 * @param body The settled rigid body.
 * @param geom The shape's geometry, whose face-normal table to read.
 * @returns The physical face index most nearly pointing up.
 * @example
 * ```ts
 * declare const body: import("@dimforge/rapier3d-compat").RigidBody;
 * declare const geom: ShapeGeometry;
 * upFaceIndex(body, geom);
 * ```
 */
function upFaceIndex(body: import("@dimforge/rapier3d-compat").RigidBody, geom: ShapeGeometry): number {
  const q = body.rotation();
  const qv: [number, number, number] = [q.x, q.y, q.z];
  const cross = (a: [number, number, number], b: [number, number, number]): [number, number, number] => [
    a[1] * b[2] - a[2] * b[1],
    a[2] * b[0] - a[0] * b[2],
    a[0] * b[1] - a[1] * b[0],
  ];
  // Standard quaternion-vector rotation: v' = v + 2 * cross(q.xyz, cross(q.xyz, v) + q.w * v).
  const rotateY = (v: [number, number, number]): number => {
    const inner = cross(qv, v);
    const t: [number, number, number] = [inner[0] + q.w * v[0], inner[1] + q.w * v[1], inner[2] + q.w * v[2]];
    const outer = cross(qv, t);
    return v[1] + 2 * outer[1];
  };
  let bestIndex = 0;
  let bestDot = -Infinity;
  geom.faces.forEach((face, i) => {
    const y = rotateY(face.normal);
    if (y > bestDot) {
      bestDot = y;
      bestIndex = i;
    }
  });
  return bestIndex;
}
