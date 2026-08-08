// Internal semver matcher for module dependency ranges and hook versions.
// Deliberately tiny (exact / ^ / ~ / *) to avoid a runtime dependency; swap for
// the `semver` package only if richer ranges become a real requirement.
/** A parsed `[major, minor, patch]` version triple. */
type V = [number, number, number];

/** Parses a strict `major.minor.patch` version string (no pre-release/build metadata).
 * @param v The version string.
 * @returns The parsed `[major, minor, patch]` tuple.
 * @example
 * ```
 * parse("1.2.3");
 * ```
 */
function parse(v: string): V {
  const m = /^(\d+)\.(\d+)\.(\d+)$/.exec(v.trim());
  if (!m) throw new Error(`invalid semver: ${v}`);
  return [Number(m[1]), Number(m[2]), Number(m[3])];
}

/** Lexicographic `[major, minor, patch]` comparison.
 * @param a The candidate version.
 * @param b The floor version.
 * @returns `true` if `a` is greater than or equal to `b`.
 * @example
 * ```
 * gte([1, 2, 3], [1, 2, 0]);
 * ```
 */
function gte(a: V, b: V): boolean {
  for (let i = 0; i < 3; i++) {
    if (a[i] > b[i]) return true;
    if (a[i] < b[i]) return false;
  }
  return true;
}

/** Tests a strict `major.minor.patch` version against a range: an exact version,
 * `*` (any), `^` (caret, npm-semver leftmost-non-zero-component semantics — see
 * the module note), or `~` (tilde, same major+minor, patch >= the range's patch).
 * Not exported from `@shadowcat/core`'s public surface — internal to module
 * engine-compat checks (`checkEngineCompat`, `HookBus.on`, `ModuleRegistry.depsSatisfied`).
 * @param version The version being tested.
 * @param range The range to test against.
 * @returns `true` if `version` satisfies `range`.
 * @example
 * ```
 * satisfies("1.4.0", "^1.2.0");
 * ```
 */
export function satisfies(version: string, range: string): boolean {
  const r = range.trim();
  const v = parse(version);
  if (r === "*") return true;
  if (r.startsWith("^")) {
    const b = parse(r.slice(1));
    // Caret's upper bound is set by the LEFTMOST non-zero component
    // (npm-semver semantics): major>0 -> next major is breaking;
    // major===0 with minor>0 -> next minor is breaking (0.x.y line);
    // major===0 and minor===0 -> next patch is breaking (0.0.y line).
    if (b[0] > 0) return v[0] === b[0] && gte(v, b);
    if (b[1] > 0) return v[0] === b[0] && v[1] === b[1] && gte(v, b);
    return v[0] === b[0] && v[1] === b[1] && v[2] === b[2];
  }
  if (r.startsWith("~")) {
    const b = parse(r.slice(1));
    return v[0] === b[0] && v[1] === b[1] && gte(v, b);
  }
  const b = parse(r);
  return v[0] === b[0] && v[1] === b[1] && v[2] === b[2];
}
