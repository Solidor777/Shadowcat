// Ordered next()-style pipelines around core operations. v1 pipelines:
// "intent-submit" (transform/cancel an outgoing optimistic intent before
// OptimisticClient) and "inbound-event" (observe a confirmed event as applied).
// A middleware that omits next() short-circuits the remainder of the chain.
export type PipelineName = "intent-submit" | "inbound-event";
export type Middleware<C> = (ctx: C, next: () => Promise<void>) => Promise<void>;

interface Entry {
  mw: Middleware<unknown>;
  module?: string;
}

export class MiddlewareChain {
  private chains = new Map<PipelineName, Entry[]>();

  use<C>(pipeline: PipelineName, mw: Middleware<C>, opts: { module?: string } = {}): void {
    const arr = this.chains.get(pipeline) ?? [];
    arr.push({ mw: mw as Middleware<unknown>, module: opts.module });
    this.chains.set(pipeline, arr);
  }

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

  removeModule(moduleId: string): void {
    for (const [name, arr] of this.chains) {
      this.chains.set(name, arr.filter((e) => e.module !== moduleId));
    }
  }
}
