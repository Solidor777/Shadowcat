import { MAX_GRAPH_VISITS, type FormulaValue } from "./types";

/** Memoized lazy resolution over named nodes. Dependencies are discovered
 * dynamically: evalNode calls get(depKey) and cycles are detected via the
 * in-progress stack. Every node on a cycle resolves to {error:"cycle"}.
 * INVARIANT: result is independent of key iteration order (consumers rely on
 * this for the Nightfox permutation invariant, spec D3/D12).
 *
 * Recursion depth equals graph depth; bounded in practice by MAX_GRAPH_VISITS
 * (a visit-count cap trips before the JS call stack would overflow). */
export function resolveAll(
  keys: readonly string[],
  evalNode: (key: string, get: (dep: string) => FormulaValue) => FormulaValue,
): Map<string, FormulaValue> {
  const memo = new Map<string, FormulaValue>();
  const visiting = new Set<string>();
  let visits = 0;

  const get = (key: string): FormulaValue => {
    const cached = memo.get(key);
    if (cached !== undefined) return cached;
    if (visiting.has(key)) {
      return { error: "cycle", detail: `reference cycle involving '${key}'` };
    }
    visits += 1;
    if (visits > MAX_GRAPH_VISITS) {
      const capped: FormulaValue = { error: "cap", detail: "graph resolution exceeded visit cap" };
      memo.set(key, capped);
      return capped;
    }
    visiting.add(key);
    const result = evalNode(key, get);
    visiting.delete(key);
    memo.set(key, result);
    return result;
  };

  for (const key of keys) get(key);

  return memo;
}
