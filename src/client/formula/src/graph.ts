import { MAX_GRAPH_VISITS, type FormulaValue } from "./types";
import { validateResolverOutput } from "./internal";

/** Internal control-flow signal, never surfaced to a caller — thrown by the
 * `get` closure when a dependency is neither memoized nor on the active path.
 * Caught only by the iterative driver in `resolveKey`; NOT a `FormulaValue`
 * error and never stored in `memo`. */
class NeedsDependency {
  constructor(readonly key: string) {}
}

/** Memoized lazy resolution over named nodes. Dependencies are discovered
 * dynamically: evalNode calls get(depKey) and cycles are detected via the
 * in-progress stack. Every node on a cycle resolves to {error:"cycle"}.
 * INVARIANT: result is independent of key iteration order (consumers rely on
 * this for the Nightfox permutation invariant, spec D3/D12).
 *
 * Recursion bound: O(1) JS call-stack frames regardless of graph depth or
 * chain length. `evalNode` is a consumer-supplied synchronous callback that
 * calls `get` inline to fetch a dependency's value, so a plain iterative loop
 * that calls `evalNode` itself cannot be made stack-safe on its own —
 * `evalNode`'s own call frame must still exist while it waits on a nested
 * `get` to return a value it uses locally. Instead this uses a restart-based
 * trampoline: `get` throws `NeedsDependency` the first time it meets a key
 * that is not yet memoized and not on the active path, unwinding the current
 * (necessarily partial) `evalNode` call without growing the JS stack further.
 * The iterative driver below catches that signal on an explicit
 * heap-allocated `stack` array (not the JS call stack), resolves the missing
 * dependency, and RE-INVOKES `evalNode` for the same key from scratch.
 * Because resolved dependencies are memoized, a retried `evalNode` call redoes
 * only cheap `Map` lookups for each already-memoized `get` call it makes
 * before reaching the next (or no) unresolved dependency — each node runs to
 * completion after at most (its count of distinct dependencies) restarts,
 * all driven by the same `while` loop. `MAX_GRAPH_VISITS` is the only bound
 * that can trip: it is charged once per newly discovered key, at first
 * attempt. */
export function resolveAll(
  keys: readonly string[],
  evalNode: (key: string, get: (dep: string) => FormulaValue) => FormulaValue,
): Map<string, FormulaValue> {
  const memo = new Map<string, FormulaValue>();
  const visiting = new Set<string>();
  let visits = 0;

  const get = (key: string): FormulaValue => {
    if (memo.has(key)) return memo.get(key)!;
    if (visiting.has(key)) {
      return { error: "cycle", detail: `reference cycle involving '${key}'` };
    }
    throw new NeedsDependency(key);
  };

  // Iteratively resolves `root` and every transitive dependency it needs,
  // using `stack` (a plain array) in place of JS call-stack recursion.
  const resolveKey = (root: string): void => {
    const stack: string[] = [root];
    while (stack.length > 0) {
      const key = stack[stack.length - 1];
      if (memo.has(key)) {
        stack.pop();
        continue;
      }
      if (!visiting.has(key)) {
        // First attempt at this key: charge it against the visit cap once.
        visiting.add(key);
        visits += 1;
        if (visits > MAX_GRAPH_VISITS) {
          memo.set(key, { error: "cap", detail: "graph resolution exceeded visit cap" });
          visiting.delete(key);
          stack.pop();
          continue;
        }
      }
      try {
        const raw = evalNode(key, get);
        memo.set(key, validateResolverOutput(raw));
        visiting.delete(key);
        stack.pop();
      } catch (e) {
        if (e instanceof NeedsDependency) {
          // Leave `key` on the stack (and `visiting`) — retry it once
          // `e.key` is resolved, at which point `get(e.key)` is an O(1)
          // memo hit and evalNode can proceed past where it left off.
          stack.push(e.key);
          continue;
        }
        // evalNode threw something other than our internal signal: a
        // consumer-callback fault, never allowed to propagate past the
        // library boundary (never throw, per spec §3.2).
        memo.set(key, {
          error: "resolver-error",
          detail: `evalNode threw: ${e instanceof Error ? e.message : String(e)}`,
        });
        visiting.delete(key);
        stack.pop();
      }
    }
  };

  for (const key of keys) {
    if (!memo.has(key)) resolveKey(key);
  }

  return memo;
}
