// The framework-neutral UI contribution registry: modules contribute opaque
// component handles into named string-contract "surfaces"; a host (e.g. the
// Svelte <Surface> adapter) renders them. Same subscribe/snapshot reactivity as
// DocumentStore — no framework runtime here; `component` is opaque to core.

import type { WireDocument } from "./wire";

/** One provider or many for a surface contract. */
export type Cardinality = "singleton" | "multi";

/** Dock zone a panel targets under the panel-manager host. */
export type ZoneId = "right" | "bottom" | "left";

/** Where a panel starts when its module first contributes it.
 * Absent `defaultPlacement` on a `PanelMeta` means launcher-only (closed). */
export type DefaultPlacement =
  | { kind: "docked"; zone: ZoneId; order?: number }
  | { kind: "minimized" };

/** Panel metadata for the `shadowcat.panel` contract (M12a panel-manager host).
 * Plain data — framework-neutral. `labelKey` is an i18n key the HOST resolves at
 * render (locale-reactive); `gmOnly` panels are hidden from non-GM users by the host. */
export interface PanelMeta {
  icon: string;
  labelKey: string;
  /** Advisory UI filter only; the host is responsible for hiding gmOnly panels. */
  gmOnly?: boolean;
  defaultPlacement?: DefaultPlacement;
}

/** Contract id panel modules contribute under for the panel-manager host. */
export const PANEL_CONTRACT = "shadowcat.panel";

/** Provider metadata for the `shadowcat.sheet:<doc_type>` contract family (M12c).
 * `priority` selects among competing providers (higher wins; the always-registered
 * generic fallback registers at `-Infinity`). `match` is an optional per-document
 * predicate — a provider that returns `false` is not a candidate for that doc. */
export interface SheetMeta {
  priority: number;
  match?: (doc: WireDocument) => boolean;
}

export interface Contribution {
  id: string;
  contract: string;
  /** Ascending sort key within a contract; default 0. */
  order?: number;
  props?: Record<string, unknown>;
  /** Opaque host-rendered component handle. */
  component: unknown;
  panel?: PanelMeta;
  sheet?: SheetMeta;
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

  /** Contributions for a contract paired with the module id that registered each,
   * in `order` (default 0) then insertion sequence — the sheet registry needs the
   * module id for its deterministic lowest-module-id tie-break. */
  entriesFor(contract: string): readonly { contribution: Contribution; module?: string }[] {
    return this.entries
      .filter((e) => e.c.contract === contract)
      .sort((a, b) => (a.c.order ?? 0) - (b.c.order ?? 0) || a.seq - b.seq)
      .map((e) => ({ contribution: e.c, module: e.module }));
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
