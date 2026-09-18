import type { DieShapeId } from "./shapes";

/** One physical face of a standard die shape. */
export interface ShapeFace {
  /** Ordered polygon vertex indices into `ShapeGeometry.vertices` (each triple of
   * consecutive floats there is one `[x, y, z]` vertex), wound counter-clockwise seen
   * from outside the die. */
  indices: number[];
  /** The face's outward unit normal. */
  normal: [number, number, number];
}

/** A standard die shape's convex geometry: the vertex cloud (shared with the physics
 * collider) plus its physical faces, derived from ONE construction per shape so the
 * visual mesh, the collider, and the up-face normal table can never drift apart. The
 * five Platonic solids derive their faces by grouping the vertex cloud onto the planes
 * of their dual solid's vertex directions; the d10 (a pentagonal trapezohedron, not
 * Platonic) lists its ten kite faces explicitly from the same ring construction its
 * vertices come from. */
export interface ShapeGeometry {
  /** Flat `[x, y, z, ...]` vertex positions — the same point cloud handed to the
   * physics collider's convex-hull computation. */
  vertices: Float32Array;
  /** The physical faces, in the index order the per-face label materials assume. */
  faces: ShapeFace[];
}

/** Component-wise 3-vector used across this module's constructions. */
type Vec3 = [number, number, number];

/** The golden ratio, parameterizing the icosahedron/dodecahedron family and the d10's
 * apex height. */
const PHI = (1 + Math.sqrt(5)) / 2;

/** Subtracts two vectors component-wise.
 * @param a Left operand.
 * @param b Right operand.
 * @returns `a - b`.
 * @example
 * ```ts
 * sub([1, 2, 3], [0, 1, 0]); // [1, 1, 3]
 * ```
 */
function sub(a: Vec3, b: Vec3): Vec3 {
  return [a[0] - b[0], a[1] - b[1], a[2] - b[2]];
}

/** Dot product.
 * @param a Left operand.
 * @param b Right operand.
 * @returns `a . b`.
 * @example
 * ```ts
 * dot([1, 0, 0], [0, 1, 0]); // 0
 * ```
 */
function dot(a: Vec3, b: Vec3): number {
  return a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
}

/** Cross product.
 * @param a Left operand.
 * @param b Right operand.
 * @returns `a x b`.
 * @example
 * ```ts
 * cross([1, 0, 0], [0, 1, 0]); // [0, 0, 1]
 * ```
 */
function cross(a: Vec3, b: Vec3): Vec3 {
  return [
    a[1] * b[2] - a[2] * b[1],
    a[2] * b[0] - a[0] * b[2],
    a[0] * b[1] - a[1] * b[0],
  ];
}

/** Scales a vector to unit length (a zero vector stays zero; every input here is
 * non-zero by construction).
 * @param v The vector to normalize.
 * @returns `v / |v|`.
 * @example
 * ```ts
 * normalize([3, 0, 4]); // [0.6, 0, 0.8]
 * ```
 */
function normalize(v: Vec3): Vec3 {
  const len = Math.hypot(v[0], v[1], v[2]) || 1;
  return [v[0] / len, v[1] / len, v[2] / len];
}

/** Arithmetic mean of a vertex list.
 * @param verts The vertices to average.
 * @returns The centroid.
 * @example
 * ```ts
 * centroidOf([[0, 0, 0], [2, 0, 0]]); // [1, 0, 0]
 * ```
 */
function centroidOf(verts: Vec3[]): Vec3 {
  const sum = verts.reduce<Vec3>((acc, v) => [acc[0] + v[0], acc[1] + v[1], acc[2] + v[2]], [0, 0, 0]);
  return [sum[0] / verts.length, sum[1] / verts.length, sum[2] / verts.length];
}

/** Newell's method: a robust polygon normal tolerant of tiny floating-point
 * non-planarity, used both to orient every face outward and as the d10's face-normal
 * source.
 * @param polygon The polygon's vertices, in winding order.
 * @returns The (unnormalized) Newell normal.
 * @example
 * ```ts
 * newellNormal([[0, 0, 0], [1, 0, 0], [1, 1, 0], [0, 1, 0]]); // [0, 0, 1]
 * ```
 */
function newellNormal(polygon: Vec3[]): Vec3 {
  let nx = 0;
  let ny = 0;
  let nz = 0;
  for (let i = 0; i < polygon.length; i++) {
    const cur = polygon[i];
    const next = polygon[(i + 1) % polygon.length];
    nx += (cur[1] - next[1]) * (cur[2] + next[2]);
    ny += (cur[2] - next[2]) * (cur[0] + next[0]);
    nz += (cur[0] - next[0]) * (cur[1] + next[1]);
  }
  return [nx, ny, nz];
}

/** Plane-membership tolerance for grouping a vertex cloud onto a face plane. The
 * irrational coordinates of the Platonic family agree to ~1e-16; genuine non-members
 * sit O(0.1) away. */
const PLANE_EPSILON = 1e-6;

/** Builds one oriented {@link ShapeFace} from an unordered index list known to lie on
 * one plane: sorts the vertices angularly around the face centroid (an in-plane
 * angular sort of a convex polygon yields its boundary order) and flips the winding
 * when Newell says it points inward.
 * @param vertices The shape's full vertex cloud.
 * @param indices The indices of the coplanar vertices, in any order.
 * @param outward Reference direction the finished normal must agree with (the face
 * plane's own normal direction, or the face centroid for an explicitly built face).
 * @returns The oriented face.
 * @example
 * ```ts
 * orientFace([[1, 1, 1], [1, 1, -1], [1, -1, 1], [1, -1, -1]], [3, 1, 2, 0], [1, 0, 0]);
 * ```
 */
function orientFace(vertices: Vec3[], indices: number[], outward: Vec3): ShapeFace {
  const points = indices.map((i) => vertices[i]);
  const centroid = centroidOf(points);
  const normalGuess = normalize(outward);
  const e1 = normalize(sub(points[0], centroid));
  const e2 = cross(normalGuess, e1);
  const ordered = indices
    .map((index, k) => {
      const rel = sub(points[k], centroid);
      return { index, angle: Math.atan2(dot(rel, e2), dot(rel, e1)) };
    })
    .sort((a, b) => a.angle - b.angle)
    .map((o) => o.index);
  const newell = newellNormal(ordered.map((i) => vertices[i]));
  const wound = dot(newell, outward) < 0 ? [...ordered].reverse() : ordered;
  return { indices: wound, normal: normalize(newellNormal(wound.map((i) => vertices[i]))) };
}

/** Derives a Platonic solid's faces from its vertex cloud and its dual solid's vertex
 * directions (a regular polyhedron's face centers ARE the dual's vertices — a standard
 * identity, so the dodecahedron's 12 face directions are the icosahedron's 12 vertex
 * directions and vice versa): each direction's plane carries the vertices whose dot
 * with it attains the cloud's maximum.
 * @param vertices The shape's vertex cloud.
 * @param directions One outward direction per face, in the order the finished
 * `faces` array (and therefore the per-face label materials) keeps.
 * @returns The derived faces, one per direction.
 * @example
 * ```ts
 * facesFromDirections([[1, 0, 0], [-1, 0, 0], [0, 1, 0], [0, 0, 1]], [[1, 1, 1]]);
 * ```
 */
function facesFromDirections(vertices: Vec3[], directions: Vec3[]): ShapeFace[] {
  return directions.map((dir) => {
    const n = normalize(dir);
    const dots = vertices.map((v) => dot(v, n));
    const h = Math.max(...dots);
    const onPlane = vertices.map((_, i) => i).filter((i) => h - dots[i] < PLANE_EPSILON);
    return orientFace(vertices, onPlane, n);
  });
}

/** One standard shape's analytic construction: its vertex cloud plus either its
 * face-direction table (Platonic solids) or an explicit face list (the d10). */
interface ShapeConstruction {
  /** The vertex cloud. */
  vertices: Vec3[];
  /** Dual direction table for the Platonic solids; `null` for the d10, which fills
   * `explicitFaces` instead. */
  directions: Vec3[] | null;
  /** Explicit per-face index lists (d10 only), in any winding — `orientFace` orders
   * and outward-orients them. */
  explicitFaces: number[][] | null;
}

/** The d10's upper ring radius, in the same arbitrary unit as every other shape's
 * vertex clouds (dice are unit-scaled into the tray at spawn). */
const D10_RING_RADIUS = 1;

/** The d10 ring half-height. */
const D10_RING_HEIGHT = 0.25;

/** The d10 apex height for PLANAR kite faces. A kite is the top apex, two adjacent
 * upper-ring vertices, and the lower-ring vertex angularly between them; coplanarity
 * of those four points with equal ring radii reduces to `A/H = (2 + phi) / (2 - phi)`,
 * with `phi` the golden ratio. */
const D10_APEX = (D10_RING_HEIGHT * (2 + PHI)) / (2 - PHI);

/** The d10 (pentagonal trapezohedron) construction: top apex, a 5-vertex upper ring,
 * a 5-vertex lower ring rotated 36 degrees, bottom apex. Index layout: 0 = top apex,
 * 1..5 = upper ring, 6..10 = lower ring, 11 = bottom apex. Top kite `i` is
 * `{ 0, U_i, U_{i+1}, L_i }` (the lower-ring vertex between the two uppers); bottom
 * kite `i` is `{ 11, L_i, L_{i+1}, U_{i+1} }` by the same construction mirrored.
 * @returns The d10's vertex cloud and explicit kite faces.
 * @example
 * ```ts
 * d10Construction().explicitFaces; // ten kites
 * ```
 */
function d10Construction(): ShapeConstruction {
  const vertices: Vec3[] = [[0, D10_APEX, 0]];
  for (let i = 0; i < 5; i++) {
    const a = (i / 5) * Math.PI * 2;
    vertices.push([Math.cos(a) * D10_RING_RADIUS, D10_RING_HEIGHT, Math.sin(a) * D10_RING_RADIUS]);
  }
  for (let i = 0; i < 5; i++) {
    const a = ((i + 0.5) / 5) * Math.PI * 2;
    vertices.push([Math.cos(a) * D10_RING_RADIUS, -D10_RING_HEIGHT, Math.sin(a) * D10_RING_RADIUS]);
  }
  vertices.push([0, -D10_APEX, 0]);
  const explicitFaces: number[][] = [];
  for (let i = 0; i < 5; i++) explicitFaces.push([0, 1 + i, 1 + ((i + 1) % 5), 6 + i]);
  for (let i = 0; i < 5; i++) explicitFaces.push([11, 6 + i, 6 + ((i + 1) % 5), 1 + ((i + 1) % 5)]);
  return { vertices, directions: null, explicitFaces };
}

/** Every standard shape's construction, keyed by shape id. The Platonic vertex
 * clouds are the solids' well-known coordinates (tetrahedron as alternating cube
 * corners; cube; octahedron as the axis points; dodecahedron and icosahedron in the
 * standard golden-ratio parametrization) — public mathematical constructions, no
 * vendored geometry data.
 * @param shape Which standard shape to describe.
 * @returns The shape's vertex cloud plus its face-direction table or explicit faces.
 * @example
 * ```ts
 * constructionFor("d6").vertices.length; // 8
 * ```
 */
function constructionFor(shape: DieShapeId): ShapeConstruction {
  switch (shape) {
    case "d4":
      return {
        vertices: [[1, 1, 1], [-1, -1, 1], [-1, 1, -1], [1, -1, -1]],
        directions: [[-1, -1, -1], [1, 1, -1], [1, -1, 1], [-1, 1, 1]],
        explicitFaces: null,
      };
    case "d6":
      return {
        vertices: [
          [1, 1, 1], [1, 1, -1], [1, -1, 1], [1, -1, -1],
          [-1, 1, 1], [-1, 1, -1], [-1, -1, 1], [-1, -1, -1],
        ],
        directions: [[1, 0, 0], [-1, 0, 0], [0, 1, 0], [0, -1, 0], [0, 0, 1], [0, 0, -1]],
        explicitFaces: null,
      };
    case "d8": {
      const vertices: Vec3[] = [];
      for (const s of [1, -1]) for (const axis of [0, 1, 2]) {
        const v: Vec3 = [0, 0, 0];
        v[axis] = s;
        vertices.push(v);
      }
      const directions: Vec3[] = [];
      for (const sx of [1, -1]) for (const sy of [1, -1]) for (const sz of [1, -1]) {
        directions.push([sx, sy, sz]);
      }
      return { vertices, directions, explicitFaces: null };
    }
    case "d10":
      return d10Construction();
    case "d12": {
      const phi = PHI;
      const vertices: Vec3[] = [];
      for (const s1 of [1, -1]) for (const s2 of [1, -1]) for (const s3 of [1, -1]) vertices.push([s1, s2, s3]);
      // Dodecahedron vertex directions equal the icosahedron's face normals (the dual
      // identity this construction relies on): the non-cube third scaled by `phi` on
      // the axis the cube-block's sign pattern leaves as the "major" one and by `1/phi`
      // on the other, verified against a brute-force convex-hull face count (every
      // direction below gathers exactly 5 coplanar vertices, never fewer).
      for (const s1 of [1, -1]) for (const s2 of [1, -1]) vertices.push([0, s1 * phi, s2 / phi]);
      for (const s1 of [1, -1]) for (const s2 of [1, -1]) vertices.push([s1 * phi, s2 / phi, 0]);
      for (const s1 of [1, -1]) for (const s2 of [1, -1]) vertices.push([s1 / phi, 0, s2 * phi]);
      const directions: Vec3[] = [];
      for (const s1 of [1, -1]) for (const s2 of [1, -1]) directions.push([0, s1, s2 * phi]);
      for (const s1 of [1, -1]) for (const s2 of [1, -1]) directions.push([s1, s2 * phi, 0]);
      for (const s1 of [1, -1]) for (const s2 of [1, -1]) directions.push([s2 * phi, 0, s1]);
      return { vertices, directions, explicitFaces: null };
    }
    case "d20": {
      const phi = PHI;
      const vertices: Vec3[] = [];
      for (const s1 of [1, -1]) for (const s2 of [1, -1]) vertices.push([0, s1, s2 * phi]);
      for (const s1 of [1, -1]) for (const s2 of [1, -1]) vertices.push([s1, s2 * phi, 0]);
      for (const s1 of [1, -1]) for (const s2 of [1, -1]) vertices.push([s1 * phi, 0, s2]);
      const directions: Vec3[] = [];
      for (const s1 of [1, -1]) for (const s2 of [1, -1]) for (const s3 of [1, -1]) directions.push([s1, s2, s3]);
      // Icosahedron face normals equal the dodecahedron's vertex directions (the same
      // dual identity as the d12 block above, mirrored): see that block's comment.
      for (const s1 of [1, -1]) for (const s2 of [1, -1]) directions.push([0, s1 * phi, s2 / phi]);
      for (const s1 of [1, -1]) for (const s2 of [1, -1]) directions.push([s1 * phi, s2 / phi, 0]);
      for (const s1 of [1, -1]) for (const s2 of [1, -1]) directions.push([s2 / phi, 0, s1 * phi]);
      return { vertices, directions, explicitFaces: null };
    }
  }
}

/** Per-shape memo of built geometries; a construction is a few dozen floats, built
 * once per shape id per session. */
const GEOMETRY_CACHE = new Map<DieShapeId, ShapeGeometry>();

/**
 * Returns a standard shape's convex geometry (vertices plus oriented physical faces),
 * built once per shape id and memoized.
 * @param shape Which standard shape to build.
 * @returns The shape's geometry; the same object on every call for a given `shape`.
 * @example
 * ```ts
 * import { shapeGeometry } from "@shadowcat/module-dice-3d";
 *
 * shapeGeometry("d6").faces.length; // 6
 * ```
 */
export function shapeGeometry(shape: DieShapeId): ShapeGeometry {
  const cached = GEOMETRY_CACHE.get(shape);
  if (cached) return cached;
  const { vertices, directions, explicitFaces } = constructionFor(shape);
  let faces: ShapeFace[];
  if (directions) {
    faces = facesFromDirections(vertices, directions);
  } else if (explicitFaces) {
    faces = explicitFaces.map((indices) =>
      orientFace(vertices, indices, centroidOf(indices.map((i) => vertices[i]))));
  } else {
    // Unreachable by construction: every `constructionFor` arm fills exactly one of
    // the two face sources.
    throw new Error(`shape ${shape} declares no face source`);
  }
  const flat = new Float32Array(vertices.length * 3);
  vertices.forEach((v, i) => {
    flat[i * 3] = v[0];
    flat[i * 3 + 1] = v[1];
    flat[i * 3 + 2] = v[2];
  });
  const built: ShapeGeometry = { vertices: flat, faces };
  GEOMETRY_CACHE.set(shape, built);
  return built;
}
