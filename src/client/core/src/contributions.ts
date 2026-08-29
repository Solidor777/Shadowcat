// The framework-neutral UI contribution registry: modules contribute opaque
// component handles into named string-contract "surfaces"; a host (e.g. the
// Svelte <Surface> adapter) renders them. Same subscribe/snapshot reactivity as
// DocumentStore — no framework runtime here; `component` is opaque to core.

import type { WireDocument } from "./wire";

/** One provider or many for a surface contract. `"singleton"` enforcement (a collision demotes
 * every losing claimant rather than aborting it) lives on the consumer, `ContractProvide.cardinality`
 * — see that field's own doc comment for the enforcing citation; not restated here. */
export type Cardinality = "singleton" | "multi";

/** Dock zone a panel targets under the panel-manager host. The three-member set is the
 * enforcement boundary itself: `placeByPlacement`
 * switches on `DefaultPlacement`'s `kind` and, for `"docked"`, places directly into
 * `l.expanded.zones[zone]` — no zone outside this union is representable in the layout tree. The
 * drop-target policy (`classifyDrop`) independently confirms the
 * boundary: it vetoes any container-edge drop targeting `"top"`, because
 * `"top"` is not, and never will be, a `ZoneId`. */
export type ZoneId = "right" | "bottom" | "left";

/** Where a panel starts when its module first contributes it.
 * Absent `defaultPlacement` on a `PanelMeta` means launcher-only (closed). */
export type DefaultPlacement =
  | {
      /** Starts docked into a `ZoneId`. */
      kind: "docked";
      /** The dock zone to start docked into. */
      zone: ZoneId;
      /** Ascending sort key among panels docked in the same zone; default 0. */
      order?: number;
    }
  | {
      /** Starts minimized (chip in the panel-dock strip, not rendered). */
      kind: "minimized";
    }
  | {
      /** Starts floating (its own window/overlay, not docked to a zone). */
      kind: "floating";
    };

/** Live tab-badge count seam (e.g. chat unread). A `PanelMeta` object is registered
 * ONCE at module install and is otherwise static — this is the one field on it whose
 * value changes over a session, independent of any panel-manager `apply()` cycle, so
 * it carries its own subscribe/read pair rather than requiring the whole meta map to
 * be rebuilt whenever the count changes. */
export interface PanelBadge {
  /** Current count; 0 (or omitted) renders no badge.
   * @returns The current badge count. */
  get(): number;
  /** Notifies on every count change; returns an unsubscribe.
   * @param cb Called with no arguments after the count changes.
   * @returns An unsubscribe function. */
  subscribe(cb: () => void): () => void;
}

/** Panel metadata for the `shadowcat.panel` contract (the panel-manager host's
 * contribution shape). Plain data — framework-neutral. `labelKey` is an i18n key the HOST resolves at
 * render (locale-reactive); `gmOnly` panels are hidden from non-GM users by the host. */
export interface PanelMeta {
  /** Icon identifier the host resolves to a rendered icon; opaque to core. */
  icon: string;
  /** i18n key for the panel's tab label, resolved by the host at render (locale-reactive). */
  labelKey: string;
  /** Advisory UI filter only; the host is responsible for hiding gmOnly panels. */
  gmOnly?: boolean;
  /** Where the panel starts when first contributed; absent means launcher-only (closed). */
  defaultPlacement?: DefaultPlacement;
  /** Optional live unread/notification count rendered in the tab chrome. */
  badge?: PanelBadge;
}

/** Contract id panel modules contribute under for the panel-manager host. */
export const PANEL_CONTRACT = "shadowcat.panel";

/** Singleton contract id the active game system's module provides. The registry elects one
 * winner (`ModuleRegistry`'s existing singleton-contract election, `ContractProvide.cardinality:
 * "singleton"`) the same way it does for `PANEL_CONTRACT`; `ModuleRegistry.systemModule()`
 * returns that winner. */
export const SYSTEM_CONTRACT = "shadowcat.system";

/** Provider metadata for the `shadowcat.sheet:<doc_type>` contract family.
 * `priority` selects among competing providers (higher wins; the always-registered
 * generic fallback registers at `-Infinity`). `match` is an optional per-document
 * predicate — a provider that returns `false` is not a candidate for that doc. */
export interface SheetMeta {
  /** Selects among competing providers for the same doc_type; higher wins. The
   * always-registered generic fallback registers at `-Infinity`. */
  priority: number;
  /** Optional per-document predicate; a provider whose `match` returns `false` for `doc` is
   * not a candidate for it. Absent means the provider is a candidate for every document of its
   * contract's doc_type. */
  match?: (doc: WireDocument) => boolean;
}

/** One piece of UI a module contributes into a named surface contract. */
export interface Contribution {
  /** An id the contributing module/host is responsible for keeping unique across whatever
   * contract it registers under; the registry neither checks nor enforces this — `contribute`
   * pushes the entry unconditionally and its returned dispose closes over the `Entry` object,
   * removing it by identity (`this.entries.indexOf(entry)`), never by `id` lookup, so a
   * duplicate would register successfully and dispose correctly. The risk is downstream:
   * consumers that key off `id` assume uniqueness and were not verified against a collision —
   * `PanelsController` keys its persisted layout tree, `open(id)`/`close(id)`, and `metaMap` off
   * it, `PanelHost` keys its rendered
   * slots (`{#each ... (c.id)}`) and crash/reload testids off it, and
   * `pickSheet`'s deterministic ordering falls back to `contribution.id` as the tie-break when
   * `module` is absent (a host-registered contribution). */
  id: string;
  /** The surface contract this contribution targets (e.g. `"shadowcat.panel"`). */
  contract: string;
  /** Ascending sort key within a contract; default 0. */
  order?: number;
  /** Opaque props passed to the rendered `component`; framework-neutral, no shape imposed. */
  props?: Record<string, unknown>;
  /** Opaque host-rendered component handle. */
  component: unknown;
  /** Panel metadata, present iff `contract` is the `shadowcat.panel` family. */
  panel?: PanelMeta;
  /** Sheet metadata, present iff `contract` is a `shadowcat.sheet:<doc_type>` family member. */
  sheet?: SheetMeta;
}

/** Internal registration record: a `Contribution` plus its registering module and insertion
 * order. Not exported. */
interface Entry {
  /** The registered contribution. */
  c: Contribution;
  /** The registering module's id; undefined for a host-registered (non-module) contribution. */
  module?: string;
  /** Monotonic insertion order, the tie-break after `order` in `contributionsFor`/`entriesFor`. */
  seq: number;
}

/** A change-notification callback registered via `ContributionRegistry.subscribe`. */
export type Listener = () => void;

/** Registration options for `ContributionRegistry.contribute`. */
export interface ContributeOptions {
  /** The registering module's id; omitted for a host-registered contribution. */
  module?: string;
}

/** Registers contributions into named "surface" contracts (e.g. `shadowcat.panel`,
 * `shadowcat.sheet:<doc_type>`) and notifies subscribers on every add/remove.
 * Framework-neutral — `component` is an opaque handle a host renders (the Svelte
 * `<Surface>` adapter, `PanelHost`, `pickSheet`); this class has no rendering opinion. */
export class ContributionRegistry {
  /** Every currently registered contribution. */
  private entries: Entry[] = [];
  /** Subscribers notified on every add/remove. */
  private listeners = new Set<Listener>();
  /** Monotonic counter stamped onto each `Entry.seq` at registration time. */
  private seqCounter = 0;

  /** Register a contribution; returns a dispose that removes exactly it.
   * @param c The contribution to register.
   * @param opts Registration options; `module` (recorded for `removeModule` teardown and
   * `entriesFor`'s module-id tie-break) is omitted for a host-registered (non-module)
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
  contribute(c: Contribution, opts: ContributeOptions = {}): () => void {
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
  entriesFor(contract: string): readonly {
    /** The registered contribution. */
    contribution: Contribution;
    /** Its registering module id; undefined for a host-registered contribution. */
    module?: string;
  }[] {
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
