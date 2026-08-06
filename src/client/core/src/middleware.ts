// Ordered next()-style pipelines around core operations. v1 pipelines:
// "intent-submit" (transform/cancel an outgoing optimistic intent before
// OptimisticClient) and "inbound-event" (observe a confirmed event as applied).
// A middleware that omits next() short-circuits the remainder of the chain.
/** The two v1 pipeline names — see the module note for what each observes. */
export type PipelineName = "intent-submit" | "inbound-event";
/** A next()-style handler; omitting the `next()` call short-circuits the remainder of the chain. */
export type Middleware<C> = (ctx: C, next: () => Promise<void>) => Promise<void>;

/** One registered middleware, tagged with its owning module for bulk teardown. */
interface Entry {
  /** The registered handler, type-erased to `unknown` for uniform storage across pipelines. */
  mw: Middleware<unknown>;
  /** The registering module's id, for `removeModule` teardown; absent for host-registered middleware. */
  module?: string;
}

/** Per-pipeline ordered next()-style middleware chains, keyed by `PipelineName`.
 * @example
 * ```ts
 * import { MiddlewareChain } from "@shadowcat/core";
 *
 * const chain = new MiddlewareChain();
 * chain.use("intent-submit", async (ctx, next) => next());
 * ```
 */
export class MiddlewareChain {
  /** Registered entries per pipeline, in registration order. */
  private chains = new Map<PipelineName, Entry[]>();

  /** Appends a middleware to the end of `pipeline`'s chain.
   * @param pipeline The pipeline to append to.
   * @param mw The middleware; omitting `next()` short-circuits the remainder of the chain.
   * @param opts Registration options.
   * @param opts.module The owning module id, for later `removeModule` teardown.
   * @example
   * ```ts
   * import { MiddlewareChain } from "@shadowcat/core";
   *
   * const chain = new MiddlewareChain();
   * chain.use("inbound-event", async (ctx, next) => next(), { module: "m1" });
   * ```
   */
  use<C>(
    pipeline: PipelineName,
    mw: Middleware<C>,
    opts: { /** The owning module id, for later `removeModule` teardown. */ module?: string } = {},
  ): void {
    const arr = this.chains.get(pipeline) ?? [];
    arr.push({ mw: mw as Middleware<unknown>, module: opts.module });
    this.chains.set(pipeline, arr);
  }

  /** Runs `pipeline`'s chain to completion over `ctx`, in registration order.
   * @param pipeline The pipeline to run.
   * @param ctx The context object passed to every middleware in the chain.
   * @returns Resolves once the chain completes (or a middleware short-circuits it).
   * @example
   * ```ts
   * import { MiddlewareChain } from "@shadowcat/core";
   *
   * const chain = new MiddlewareChain();
   * await chain.run("intent-submit", { ops: [] });
   * ```
   */
  async run<C>(pipeline: PipelineName, ctx: C): Promise<void> {
    const arr = this.chains.get(pipeline) ?? [];
    let called = -1;
    const dispatch = async (i: number): Promise<void> => {
      // A middleware that calls next() twice would re-dispatch the tail; reject
      // that rather than run handlers more than once.
      if (i <= called) throw new Error("middleware called next() multiple times");
      called = i;
      if (i >= arr.length) return;
      await arr[i].mw(ctx, () => dispatch(i + 1));
    };
    await dispatch(0);
  }

  /** Removes every middleware registered by `moduleId`, across all pipelines
   * (module unload teardown).
   * @param moduleId The unloading module's id.
   * @example
   * ```ts
   * import { MiddlewareChain } from "@shadowcat/core";
   *
   * const chain = new MiddlewareChain();
   * chain.removeModule("m1");
   * ```
   */
  removeModule(moduleId: string): void {
    for (const [name, arr] of this.chains) {
      this.chains.set(name, arr.filter((e) => e.module !== moduleId));
    }
  }
}
