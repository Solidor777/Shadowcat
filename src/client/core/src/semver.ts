// Internal semver matcher for module dependency ranges and hook versions.
// Deliberately tiny (exact / ^ / ~ / *) to avoid a runtime dependency; swap for
// the `semver` package only if richer ranges become a real requirement.
type V = [number, number, number];

function parse(v: string): V {
  const m = /^(\d+)\.(\d+)\.(\d+)$/.exec(v.trim());
  if (!m) throw new Error(`invalid semver: ${v}`);
  return [Number(m[1]), Number(m[2]), Number(m[3])];
}

function gte(a: V, b: V): boolean {
  for (let i = 0; i < 3; i++) {
    if (a[i] > b[i]) return true;
    if (a[i] < b[i]) return false;
  }
  return true;
}

export function satisfies(version: string, range: string): boolean {
  const r = range.trim();
  const v = parse(version);
  if (r === "*") return true;
  if (r.startsWith("^")) {
    const b = parse(r.slice(1));
    return v[0] === b[0] && gte(v, b);
  }
  if (r.startsWith("~")) {
    const b = parse(r.slice(1));
    return v[0] === b[0] && v[1] === b[1] && gte(v, b);
  }
  const b = parse(r);
  return v[0] === b[0] && v[1] === b[1] && v[2] === b[2];
}
