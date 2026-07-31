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
  | { kind: "minimized" }
  | { kind: "floating" };

/** Live tab-badge count seam (e.g. chat unread). A `PanelMeta` object is registered
 * ONCE at module install and is otherwise static — this is the one field on it whose
 * value changes over a session, independent of any panel-manager `apply()` cycle, so
 * it carries its own subscribe/read pair rather than requiring the whole meta map to
 * be rebuilt whenever the count changes. */
export interface PanelBadge {
  /** Current count; 0 (or omitted) renders no badge. */
  get(): number;
  /** Notifies on every count change; returns an unsubscribe. */
  subscribe(cb: () => void): () => void;
}

/** Panel metadata for the `shadowcat.panel` contract (M12a panel-manager host).
 * Plain data — framework-neutral. `labelKey` is an i18n key the HOST resolves at
 * render (locale-reactive); `gmOnly` panels are hidden from non-GM users by the host. */
export interface PanelMeta {
  icon: string;
  labelKey: string;
  /** Advisory UI filter only; the host is responsible for hiding gmOnly panels. */
  gmOnly?: boolean;
  defaultPlacement?: DefaultPlacement;
  /** Optional live unread/notification count rendered in the tab chrome. */
  badge?: PanelBadge;
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

/** Registers contributions into named "surface" contracts (e.g. `shadowcat.panel`,
 * `shadowcat.sheet:<doc_type>`) and notifies subscribers on every add/remove.
 * Framework-neutral — `component` is an opaque handle a host renders (the Svelte
 * `<Surface>` adapter, `PanelHost`, `pickSheet`); this class has no rendering opinion. */
export class ContributionRegistry {
  private entries: Entry[] = [];
  private listeners = new Set<Listener>();
  private seqCounter = 0;

  /** Register a contribution; returns a dispose that removes exactly it.
   * @param c The contribution to register.
   * @param opts Registration options.
   * @param opts.module The registering module's id, recorded for `removeModule` teardown
   * and `entriesFor`'s module-id tie-break; omitted for a host-registered (non-module)
   * contribution.
   * @returns A dispose function that removes this contribution and notifies subscribers.
   * @example
   * ```ts
   * import { ContributionRegistry } from "@shadowcat/core";
   *
   * const registry = new ContributionRegistry();
   * const dispose = registry.contribute(
   *   { id: "example", contract: "shadowcat.panel", component: {} },
   *   { module: "example-module" },
   * );
   * dispose();
   * ```
   */
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

  /** Contributions for a contract, sorted by `order` (default 0) then insertion.
   * @param contract The contract id to look up.
   * @returns The matching contributions, in render order.
   * @example
   * ```ts
   * import { ContributionRegistry } from "@shadowcat/core";
   *
   * const registry = new ContributionRegistry();
   * const panels = registry.contributionsFor("shadowcat.panel");
   * ```
   */
  contributionsFor(contract: string): readonly Contribution[] {
    return this.entries
      .filter((e) => e.c.contract === contract)
      .sort((a, b) => (a.c.order ?? 0) - (b.c.order ?? 0) || a.seq - b.seq)
      .map((e) => e.c);
  }

  /** Contributions for a contract paired with the module id that registered each,
   * in `order` (default 0) then insertion sequence — the sheet registry needs the
   * module id for its deterministic lowest-module-id tie-break.
   * @param contract The contract id to look up.
   * @returns The matching contributions, each paired with its registering module id
   * (undefined for a host-registered contribution).
   * @example
   * ```ts
   * import { ContributionRegistry } from "@shadowcat/core";
   *
   * const registry = new ContributionRegistry();
   * const entries = registry.entriesFor("shadowcat.sheet:actor");
   * ```
   */
  entriesFor(contract: string): readonly { contribution: Contribution; module?: string }[] {
    return this.entries
      .filter((e) => e.c.contract === contract)
      .sort((a, b) => (a.c.order ?? 0) - (b.c.order ?? 0) || a.seq - b.seq)
      .map((e) => ({ contribution: e.c, module: e.module }));
  }

  /** Notifies `listener` on every contribution add/remove (`contribute`'s dispose,
   * or `removeModule`).
   * @param listener Called with no arguments after a change.
   * @returns An unsubscribe function.
   * @example
   * ```ts
   * import { ContributionRegistry } from "@shadowcat/core";
   *
   * const registry = new ContributionRegistry();
   * const unsubscribe = registry.subscribe(() => {});
   * unsubscribe();
   * ```
   */
  subscribe(listener: Listener): () => void {
    this.listeners.add(listener);
    return () => this.listeners.delete(listener);
  }

  /** Drop every contribution tagged with `moduleId` (module unload teardown).
   * @param moduleId The module id whose contributions should be removed.
   * @example
   * ```ts
   * import { ContributionRegistry } from "@shadowcat/core";
   *
   * const registry = new ContributionRegistry();
   * registry.removeModule("example-module");
   * ```
   */
  removeModule(moduleId: string): void {
    const before = this.entries.length;
    this.entries = this.entries.filter((e) => e.module !== moduleId);
    if (this.entries.length !== before) this.emit();
  }

  /** Notifies every subscriber that the contribution set changed.
   * @example
   * ```
   * // internal helper; not part of the public API
   * this.emit();
   * ```
   */
  private emit(): void {
    for (const fn of this.listeners) fn();
  }
}
