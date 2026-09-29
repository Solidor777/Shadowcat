// @vitest-environment node
// Property tests of the thinning deviation over deterministic seeded triples: no DOM, no timers.
import { describe, it, expect } from "vitest";
import { chordDeviation } from "./polyline-simplify";

type Pt = [number, number];

/** Deterministic PRNG (mulberry32, Tommy Ettinger's public-domain 32-bit generator): the same seed
 * yields the same triples on every machine, so a failure reproduces exactly. */
function mulberry32(seed: number): () => number {
  let s = seed >>> 0;
  return () => {
    s = (s + 0x6d2b79f5) >>> 0;
    let x = s;
    x = Math.imul(x ^ (x >>> 15), x | 1);
    x ^= x + Math.imul(x ^ (x >>> 7), x | 61);
    return ((x ^ (x >>> 14)) >>> 0) / 4294967296;
  };
}

/** A triple `a`, `b`, `c` in scene units. */
type Triple = { a: Pt; b: Pt; c: Pt; family: string };

/** Generates triples across scales (0.01 to 1000) from families that sit on every geometric
 * boundary the priority must be continuous across: the foot of `b` exactly at `a` (t = 0), exactly
 * at `c` (t = 1), an exact reversal, a needle (`b` far off a short chord), `a` equal to `c`, an
 * interior foot with a modest height (the perpendicular-height regime), and unconstrained
 * triples. */
function triples(count: number, seed: number): Triple[] {
  const rnd = mulberry32(seed);
  const scale = (): number => Math.pow(10, -2 + 5 * rnd());
  const coord = (s: number): number => (rnd() - 0.5) * 2 * s;
  const out: Triple[] = [];
  for (let i = 0; i < count; i++) {
    const s = scale();
    const a: Pt = [coord(s), coord(s)];
    const families = ["free", "foot-at-a", "foot-at-c", "reversal", "needle", "a-equals-c", "in-segment"];
    const family = families[i % families.length];
    let b: Pt;
    let c: Pt;
    const w = s * (0.1 + rnd());
    const ang = rnd() * 2 * Math.PI;
    const dir: Pt = [Math.cos(ang), Math.sin(ang)];
    const perp: Pt = [-dir[1], dir[0]];
    switch (family) {
      case "foot-at-a": {
        c = [a[0] + dir[0] * s, a[1] + dir[1] * s];
        b = [a[0] + perp[0] * w * 10, a[1] + perp[1] * w * 10];
        break;
      }
      case "foot-at-c": {
        c = [a[0] + dir[0] * s, a[1] + dir[1] * s];
        b = [c[0] + perp[0] * w * 10, c[1] + perp[1] * w * 10];
        break;
      }
      case "reversal": {
        const far = s * (1 + rnd());
        b = [a[0] + dir[0] * far, a[1] + dir[1] * far];
        const back = far * (rnd() * 1.6 - 0.6);
        c = [a[0] + dir[0] * back, a[1] + dir[1] * back];
        break;
      }
      case "needle": {
        c = [a[0] + dir[0] * s * 1e-3, a[1] + dir[1] * s * 1e-3];
        b = [a[0] + dir[0] * s * 5e-4 + perp[0] * s * 10, a[1] + dir[1] * s * 5e-4 + perp[1] * s * 10];
        break;
      }
      case "in-segment": {
        c = [a[0] + dir[0] * s, a[1] + dir[1] * s];
        const u = rnd();
        const h = s * rnd() * 0.9;
        b = [a[0] + dir[0] * s * u + perp[0] * h, a[1] + dir[1] * s * u + perp[1] * h];
        break;
      }
      case "a-equals-c": {
        c = [a[0], a[1]];
        b = [a[0] + dir[0] * w, a[1] + dir[1] * w];
        break;
      }
      default: {
        b = [coord(s), coord(s)];
        c = [coord(s), coord(s)];
      }
    }
    out.push({ a, b, c, family });
  }
  return out;
}

/** Samples per polyline leg and along the chord for the brute-force reference. The reference's
 * cost is the product of the two sample counts, so they are the smallest that keep the error
 * bound (`referenceTolerance`) tight against the property tests' margins. */
const LEG_SAMPLES = 60;
const CHORD_SAMPLES = 120;

/** The Hausdorff distance between the polyline `a`-`b`-`c` and the chord `a`-`c` by brute force,
 * in BOTH directions (the farthest any sampled polyline point lies from the chord, and the
 * farthest any sampled chord point lies from the polyline), computed without the closed form.
 * Sampling `LEG_SAMPLES` points per leg and `CHORD_SAMPLES` on the chord bounds the error by half
 * the sample spacing (`referenceTolerance`); `b` is itself a sample. Distances are compared
 * squared and rooted once per direction, so the inner loop makes no function call.
 * @param a The chord's start.
 * @param b The polyline's middle vertex.
 * @param c The chord's end.
 * @returns The sampled Hausdorff distance.
 */
function hausdorffReference(a: Pt, b: Pt, c: Pt): number {
  const chord: Pt[] = [];
  for (let j = 0; j <= CHORD_SAMPLES; j++) {
    const u = j / CHORD_SAMPLES;
    chord.push([a[0] + (c[0] - a[0]) * u, a[1] + (c[1] - a[1]) * u]);
  }
  const poly: Pt[] = [];
  for (const [p, q] of [[a, b], [b, c]] as [Pt, Pt][]) {
    for (let i = 0; i <= LEG_SAMPLES; i++) {
      const u = i / LEG_SAMPLES;
      poly.push([p[0] + (q[0] - p[0]) * u, p[1] + (q[1] - p[1]) * u]);
    }
  }
  const directed = (from: Pt[], to: Pt[]): number => {
    let worst = 0;
    for (const f of from) {
      let nearest = Infinity;
      for (const k of to) {
        const dx = f[0] - k[0];
        const dy = f[1] - k[1];
        const d2 = dx * dx + dy * dy;
        if (d2 < nearest) nearest = d2;
      }
      if (nearest > worst) worst = nearest;
    }
    return Math.sqrt(worst);
  };
  return Math.max(directed(poly, chord), directed(chord, poly));
}

/** The reference's error bound for a triple: the chord sample spacing (polyline to chord) plus
 * the polyline sample spacing (chord to polyline), each halved, with slack.
 * @param a The chord's start.
 * @param b The polyline's middle vertex.
 * @param c The chord's end.
 * @returns The largest amount the sampled distance can differ from the true one.
 */
function referenceTolerance(a: Pt, b: Pt, c: Pt): number {
  const len = Math.hypot(c[0] - a[0], c[1] - a[1]);
  const leg = Math.max(Math.hypot(b[0] - a[0], b[1] - a[1]), Math.hypot(c[0] - b[0], c[1] - b[1]));
  return 1.02 * (len / (2 * CHORD_SAMPLES) + leg / (2 * LEG_SAMPLES)) + 1e-9;
}

const N = 300;
const dump = (t: Triple): string =>
  `${t.family} a=${JSON.stringify(t.a)} b=${JSON.stringify(t.b)} c=${JSON.stringify(t.c)}`;

/** A generated triple with its brute-force reference distance and that reference's error bound. */
interface Referenced {
  /** The triple. */
  t: Triple;
  /** The sampled Hausdorff distance. */
  ref: number;
  /** The reference's error bound. */
  tol: number;
}

let cachedReferences: Referenced[] | null = null;

/** The triples of the reference-based properties with their references, computed once per file
 * so the agreement and ordering properties share one brute-force pass.
 * @returns The referenced triples.
 */
function references(): Referenced[] {
  cachedReferences ??= triples(N, 0xa11ce).map((t) => ({
    t,
    ref: hausdorffReference(t.a, t.b, t.c),
    tol: referenceTolerance(t.a, t.b, t.c),
  }));
  return cachedReferences;
}

describe("chordDeviation properties (seeded)", () => {
  it("is continuous in c: nudging c moves the deviation no more than the nudge, at t=0, t=1 and a=c", () => {
    const rnd = mulberry32(0xc0ffee);
    for (const t of triples(N, 0x5eed)) {
      const p0 = chordDeviation(t.a, t.b, t.c);
      expect(Number.isFinite(p0), dump(t)).toBe(true);
      const len = Math.hypot(t.c[0] - t.a[0], t.c[1] - t.a[1]);
      const eps = 1e-7 * (1 + len + p0);
      const ang = rnd() * 2 * Math.PI;
      for (const sign of [1, -1]) {
        const c2: Pt = [t.c[0] + sign * eps * Math.cos(ang), t.c[1] + sign * eps * Math.sin(ang)];
        // The distance from b to the segment a-c is 1-Lipschitz in c.
        const moved = Math.abs(chordDeviation(t.a, t.b, c2) - p0);
        expect(moved, dump(t)).toBeLessThanOrEqual(eps * 1.001 + 1e-9 * (1 + p0));
      }
    }
  });

  it("is nondecreasing as b moves farther from the segment along the ray from its nearest point", () => {
    const rnd = mulberry32(0xbeef);
    for (const t of triples(N, 0xfeed)) {
      const acx = t.c[0] - t.a[0];
      const acy = t.c[1] - t.a[1];
      const len2 = acx * acx + acy * acy;
      const along = len2 === 0 ? 0 : ((t.b[0] - t.a[0]) * acx + (t.b[1] - t.a[1]) * acy) / len2;
      const f = Math.min(1, Math.max(0, along));
      const q: Pt = [t.a[0] + f * acx, t.a[1] + f * acy];
      let previous = -Infinity;
      for (const k of [0.25, 0.5, 1, 1.5, 2, 4, 8, 8 + rnd()]) {
        const b: Pt = [q[0] + k * (t.b[0] - q[0]), q[1] + k * (t.b[1] - q[1])];
        const p = chordDeviation(t.a, b, t.c);
        expect(p, `${dump(t)} k=${k}`).toBeGreaterThanOrEqual(previous - 1e-9 * (1 + Math.abs(previous)));
        previous = p;
      }
    }
  });

  it("agrees with a brute-force Hausdorff distance between the polyline a-b-c and its chord", () => {
    for (const { t, ref, tol } of references()) {
      expect(Math.abs(chordDeviation(t.a, t.b, t.c) - ref), dump(t)).toBeLessThanOrEqual(tol);
    }
  });

  it("orders any two vertices by drawn deviation: the larger true loss scores higher on any chords", () => {
    const refs = references();
    let compared = 0;
    for (let i = 0; i + 1 < refs.length; i += 1) {
      for (const j of [i + 1, (i * 7 + 3) % refs.length]) {
        if (j === i) continue;
        const x = refs[i];
        const y = refs[j];
        if (Math.abs(x.ref - y.ref) <= x.tol + y.tol) continue;
        compared++;
        const px = chordDeviation(x.t.a, x.t.b, x.t.c);
        const py = chordDeviation(y.t.a, y.t.b, y.t.c);
        expect(Math.sign(px - py), `${dump(x.t)} versus ${dump(y.t)}`).toBe(Math.sign(x.ref - y.ref));
      }
    }
    expect(
      compared,
      "the generated pairs must include clearly separated losses for this property to be exercised",
    ).toBeGreaterThan(300);
  });

  it("orders a 60-unit deviation on a 10-unit chord above a 10-unit deviation on a 1000-unit chord", () => {
    const short = chordDeviation([0, 0], [5, 60], [10, 0]);
    const long = chordDeviation([0, 0], [500, 10], [1000, 0]);
    expect(short).toBeCloseTo(60, 9);
    expect(long).toBeCloseTo(10, 9);
    expect(short).toBeGreaterThan(long);
  });

  it("equals the perpendicular height whenever the foot of b lies on the segment", () => {
    let checked = 0;
    for (const t of triples(N * 2, 0xd00d)) {
      const acx = t.c[0] - t.a[0];
      const acy = t.c[1] - t.a[1];
      const len = Math.hypot(acx, acy);
      if (len === 0) continue;
      const cross = (t.b[0] - t.a[0]) * acy - acx * (t.b[1] - t.a[1]);
      const along = ((t.b[0] - t.a[0]) * acx + (t.b[1] - t.a[1]) * acy) / (len * len);
      if (along < 0 || along > 1) continue;
      checked++;
      const height = Math.abs(cross) / len;
      const error = Math.abs(chordDeviation(t.a, t.b, t.c) - height);
      expect(error, dump(t)).toBeLessThanOrEqual(1e-9 * (1 + height));
    }
    expect(
      checked,
      "the generator must produce in-segment triples for this property to be exercised",
    ).toBeGreaterThan(50);
  });

  it("scores a hairpin and its 1e-9 perturbation alike, and a 1000-unit needle by its height", () => {
    expect(chordDeviation([0, 0], [0, 1000], [1, 0])).toBeCloseTo(1000, 6);
    expect(chordDeviation([0, 0], [0, 1000], [1e-9, -1e-9])).toBeCloseTo(1000, 6);
    expect(chordDeviation([0, 0], [0.5, 1000], [1, 0])).toBeCloseTo(1000, 6);
  });
});
