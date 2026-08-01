import { MAX_GRAPH_VISITS, type FormulaValue } from "./types";
import { validateResolverOutput } from "./internal";

/** Internal control-flow signal, never surfaced to a caller — thrown by the
 * `get` closure when a dependency is neither memoized nor on the active path.
 * Caught only by the iterative driver in `resolveKey`; NOT a `FormulaValue`
 * error and never stored in `memo`. */
class NeedsDependency {
  /**
   * Carries the single dependency key that blocked the in-flight `evalNode` call.
   * @param key The dependency key that `get` needs resolved before the
   * in-flight `evalNode` call can be retried.
   * @example
   * ```
   * // not part of the public `@shadowcat/formula` surface (index.ts does not
   * // re-export this module) — thrown only inside resolveAll's `get` closure.
   * throw new NeedsDependency("hp.max");
   * ```
   */
  constructor(readonly key: string) {}
}

/** Memoized lazy resolution over named nodes. Dependencies are discovered
 * dynamically: evalNode calls get(depKey) and cycles are detected via the
 * in-progress stack. Every node on a cycle resolves to {error:"cycle"}.
 * INVARIANT: the result is a pure function of the key SET — independent of
 * the caller's key order (consumers rely on this for the Nightfox permutation
 * invariant, spec D3/D12). Enforced by sorting the roots before iteration;
 * see the note at the root loop.
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
 * attempt.
 *
 * INVARIANT: `evalNode` implementations MUST NOT wrap their own call(s) to
 * the injected `get` in try/catch. `get` uses an internal thrown signal
 * (`NeedsDependency`) to unwind `evalNode` and drive the restart-based
 * trampoline above; a surrounding try/catch inside `evalNode` intercepts
 * that signal instead of letting it propagate to the driver, so the
 * dependency is never actually resolved and the catch branch's return value
 * silently becomes the memoized result — no error, no crash, no signal that
 * anything went wrong. Nothing needs to be done to defend against `get`
 * itself failing: `get` never throws in the failure sense — division by
 * zero, cycles, unresolvable references, and all other evaluation faults
 * are returned through `get`'s normal return value as `FormulaValue` errors,
 * not thrown. Only this one internal control-flow signal must never be
 * intercepted.
 * @param keys The full set of node keys to resolve. Order does not affect
 * the result (see the order-independence invariant above); a key needed
 * only as a transitive dependency of another need not be listed here.
 * @param evalNode Consumer callback computing one node's value from its
 * dependencies (fetched via the injected `get`). MUST NOT wrap its own
 * call(s) to `get` in try/catch — see the try/catch invariant above.
 * @returns A map from the requested `keys` plus every transitive dependency
 * DISCOVERED while resolving them, to its resolved `FormulaValue`. Dependencies
 * are found dynamically through `get`, so a dependency on a branch `evalNode`
 * short-circuits past is never requested and never appears — "reachable in the
 * dependency graph" and "present in this map" are not the same set. Resolution
 * is also bounded: once `MAX_GRAPH_VISITS` first-attempt visits are charged,
 * each further key is memoized as `{ error: "cap" }` rather than resolved, so a
 * sufficiently large graph yields cap errors for the keys past the cap instead of
 * throwing. (Which keys those are is traversal-dependent; the order-independence
 * invariant above holds for graphs that stay under the cap.)
 * @example
 * ```ts
 * import { resolveAll } from "@shadowcat/formula";
 *
 * const base: Record<string, number> = { "hp.base": 10 };
 * resolveAll(["hp.base"], (key, _get) =>
 *   key in base ? base[key] : { error: "unknown-ref", detail: key },
 * );
 * ```
 */
export function resolveAll(
  keys: readonly string[],
  evalNode: (key: string, get: (dep: string) => FormulaValue) => FormulaValue,
): Map<string, FormulaValue> {
  const memo = new Map<string, FormulaValue>();
  const visiting = new Set<string>();
  // The active dependency path for the resolveKey currently running (a linear
  // chain: each entry depends on the next). Shared across resolveKey calls,
  // which run sequentially (never nested/reentrant), so one array is safe and
  // lets `get` read the path to name a cycle deterministically.
  const stack: string[] = [];
  let visits = 0;

  const get = (key: string): FormulaValue => {
    if (memo.has(key)) return memo.get(key)!;
    if (visiting.has(key)) {
      // A re-entered key on the active path closes a cycle: the path slice from
      // that key to the top IS the cycle's member set. Naming the raw
      // re-entered key would make `detail` depend on traversal/iteration order
      // (which root started, record-key order) — breaking this module's
      // documented order-independence invariant. Name the lexicographically
      // smallest cycle member instead: canonical for a given cycle regardless
      // of where the traversal happened to detect it.
      // INVARIANT: every visiting key is present on `stack` (add/push and
      // delete/pop are strictly paired, and a re-entered key takes this cycle
      // branch instead of a second push). A miss here means that pairing was
      // broken by a refactor; failing loudly beats silently returning a
      // non-canonical single-key detail. The throw is caught by resolveKey's
      // driver and surfaces as a resolver-error value, per the module's
      // never-throw boundary.
      const start = stack.indexOf(key);
      if (start < 0) {
        throw new Error(`visiting key '${key}' absent from the active stack`);
      }
      const cycle = stack.slice(start);
      const canonical = cycle.reduce((min, k) => (k < min ? k : min), cycle[0]);
      return { error: "cycle", detail: `reference cycle involving '${canonical}'` };
    }
    throw new NeedsDependency(key);
  };

  // Iteratively resolves `root` and every transitive dependency it needs,
  // using `stack` (a plain array) in place of JS call-stack recursion.
  const resolveKey = (root: string): void => {
    stack.length = 0;
    stack.push(root);
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
        // library boundary (never throw, per spec §3.2). Never interpolate
        // the caught exception's message: `detail` is player-presentable
        // (types.ts), and a consumer evalNode's thrown message is an
        // internal implementation detail, not for players.
        memo.set(key, {
          error: "resolver-error",
          detail: `evalNode threw for '${key}'`,
        });
        visiting.delete(key);
        stack.pop();
      }
    }
  };

  // Roots iterate in sorted order, NOT caller order: cycle handling is
  // traversal-path-dependent (whether `get(dep)` finds `dep` mid-evaluation
  // on the active path — a structural cycle error — or already memoized can
  // change a cycle-adjacent node's error KIND, not just its detail, when a
  // node's value short-circuits without needing the cycle completed). Fixing
  // the entry order makes the whole traversal — restart sequence, stack
  // states, detection points — a pure function of the key SET, which is what
  // the order-independence invariant above actually promises. Which member of
  // a short-circuitable cycle reports `cycle` vs the propagated error remains
  // traversal-defined, but deterministically so.
  for (const key of [...keys].sort()) {
    if (!memo.has(key)) resolveKey(key);
  }

  return memo;
}
