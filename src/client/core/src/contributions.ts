// The framework-neutral UI contribution registry: modules contribute opaque
// component handles into named string-contract "surfaces"; a host (e.g. the
// Svelte <Surface> adapter) renders them. Same subscribe/snapshot reactivity as
// DocumentStore — no framework runtime here; `component` is opaque to core.

/** One provider or many for a surface contract. */
export type Cardinality = "singleton" | "multi";

export interface Contribution {
  id: string;
  contract: string;
  /** Ascending sort key within a contract; default 0. */
  order?: number;
  props?: Record<string, unknown>;
  /** Opaque host-rendered component handle. */
  component: unknown;
}

interface Entry {
  c: Contribution;
  module?: string;
  seq: number;
}

export type Listener = () => void;

export class ContributionRegistry {
  private entries: Entry[] = [];
  private listeners = new Set<Listener>();
  private seqCounter = 0;

  /** Register a contribution; returns a dispose that removes exactly it. */
  contribute(c: Contribution, opts: { module?: string } = {}): () => void {
    const entry: Entry = { c, module: opts.module, seq: this.seqCounter++ };
    this.entries.push(entry);
    this.emit();
    return () => {
      const i = this.entries.indexOf(entry);
      if (i >= 0) {
        this.entries.splice(i, 1);
        this.emit();
      }
    };
  }

  /** Contributions for a contract, sorted by `order` (default 0) then insertion. */
  contributionsFor(contract: string): readonly Contribution[] {
    return this.entries
      .filter((e) => e.c.contract === contract)
      .sort((a, b) => (a.c.order ?? 0) - (b.c.order ?? 0) || a.seq - b.seq)
      .map((e) => e.c);
  }

  subscribe(listener: Listener): () => void {
    this.listeners.add(listener);
    return () => this.listeners.delete(listener);
  }

  /** Drop every contribution tagged with `moduleId` (module unload teardown). */
  removeModule(moduleId: string): void {
    const before = this.entries.length;
    this.entries = this.entries.filter((e) => e.module !== moduleId);
    if (this.entries.length !== before) this.emit();
  }

  private emit(): void {
    for (const fn of this.listeners) fn();
  }
}
