// Owns the lifecycle of document (sheet) panels. Each open document is a runtime
// `Contribution` under `shadowcat.panel` with id `sheet:<docId>` — the panel host mounts
// it via `{#each visibleRegs}` and the layout reducer floats/docks/minimizes it like any
// panel. This controller is generic host glue (constructed by the shell alongside
// `PanelsBridge`); it imports no module. Dedup + disposer tracking live here; placement,
// cascade, focus, and prune live in the panel manager.
import { PANEL_CONTRACT, pickSheet, resolveDocRef, type ContributionRegistry, type Logger, type ReadableDocuments, type SheetRef } from "@shadowcat/core";
import type { PanelsApi } from "./panelsBridge.svelte";
import SheetHost from "./SheetHost.svelte";

/** The controller's collaborators, supplied once at construction. */
export interface SheetsControllerDeps {
  /** Registry `#register` contributes each `sheet:<docId>` panel into. */
  contributions: ContributionRegistry;
  /** Optimistic document view `resolveDocRef` resolves a `SheetRef` against. */
  documents: ReadableDocuments;
  /** Imperative panel host used to open/focus/close a sheet's panel. */
  panels: PanelsApi;
  /** Sink for the warnings logged on a dangling/unresolvable open attempt. */
  logger: Logger;
}

/**
 * Owns the lifecycle of document (sheet) panels, backing `AppContext.openDocument`.
 * Each open document is a runtime `Contribution` under `shadowcat.panel` with id
 * `sheet:<docId>` — the panel host mounts it via `{#each visibleRegs}` and the layout
 * reducer floats/docks/minimizes it like any panel. This controller is generic host glue
 * (constructed by the shell alongside `PanelsBridge`); it imports no module. Dedup + disposer
 * tracking live here; placement, cascade, focus, and prune live in the panel manager.
 */
export class SheetsController {
  /** The controller's collaborators, fixed at construction. */
  #deps: SheetsControllerDeps;
  /** panelId -> the contribution disposer, for every sheet this controller has registered. */
  #open = new Map<string, () => void>();

  /** Build a controller wired to its collaborators; starts with no sheets open.
   * @param deps - The controller's collaborators (contributions/documents/panels/logger).
   * @example new SheetsController({ contributions, documents, panels, logger });
   */
  constructor(deps: SheetsControllerDeps) {
    this.#deps = deps;
  }

  /** Resolve `ref` -> doc + write-site (fail-closed), pick the sheet component, and
   * register+float `sheet:<docId>`. A re-open of an already-registered document focuses
   * the existing panel (never a duplicate — keep-mounted state survives).
   * @param ref - The document (optionally embedded-path/token) reference to open.
   * @example sheetsController.openDocument({ docId });
   */
  openDocument(ref: SheetRef): void {
    const target = resolveDocRef(ref, this.#deps.documents);
    if (!target) {
      this.#deps.logger.warn("openDocument: reference did not resolve (dangling/raw); opening nothing");
      return;
    }
    if (this.#open.has(target.panelId)) {
      this.#deps.panels.focus(target.panelId);
      return;
    }
    const component = pickSheet(this.#deps.contributions, target.doc);
    if (!component) {
      this.#deps.logger.warn(`openDocument: no sheet provider (not even a fallback) for doc_type "${target.doc.doc_type}"`);
      return;
    }
    this.#register(target.panelId, component, target.writeDocId, target.writePrefix);
    this.#deps.panels.open(target.panelId);
  }

  /** Full disposal: removes the panel from the layout AND drops the contribution (the
   * sheet component unmounts). The header Close control routes here.
   * @param panelId - The `sheet:<docId>`-shaped panel id to close.
   * @example sheetsController.closeDocument("sheet:doc1");
   */
  closeDocument(panelId: string): void {
    this.#deps.panels.close(panelId);
    const dispose = this.#open.get(panelId);
    if (dispose) {
      dispose();
      this.#open.delete(panelId);
    }
  }

  /** Boot restore (§7): re-registers every `sheet:<docId>` id found anywhere in the
   * persisted panel blob whose document currently resolves — the panel manager's own
   * late-registration path (`placeNewRegistrations` -> `placeFromPersistedLocation`) then
   * restores each to its persisted floating/minimized spot, so this NEVER calls `open()`
   * (which would re-cascade). Idempotent (dedup via `#open`) + generic (scans for the
   * `sheet:` id shape, not the blob's exact schema). An unresolvable id is left pruned.
   * @param blob - The opaque persisted panel-layout blob to scan for `sheet:*` ids.
   * @example sheetsController.restoreFromPersisted(panelLayoutBlob);
   */
  restoreFromPersisted(blob: unknown): void {
    for (const panelId of collectSheetIds(blob)) {
      if (this.#open.has(panelId)) continue;
      const docId = panelId.slice("sheet:".length).split("/")[0];
      const embeddedPath = panelId.includes("/embedded/") ? panelId.slice(panelId.indexOf("/embedded/")) : undefined;
      const ref: SheetRef = embeddedPath ? { docId, embeddedPath } : { docId };
      const target = resolveDocRef(ref, this.#deps.documents);
      if (!target || target.panelId !== panelId) continue;
      const component = pickSheet(this.#deps.contributions, target.doc);
      if (!component) continue;
      this.#register(target.panelId, component, target.writeDocId, target.writePrefix);
    }
  }

  /** Register `panelId` as a `shadowcat.panel` contribution wrapping `component` in
   * `SheetHost`, and track its disposer in `#open` for later `closeDocument`.
   * @param panelId - The `sheet:<docId>`-shaped panel id to register.
   * @param component - The resolved sheet component (from `pickSheet`) to host.
   * @param docId - The write-target document id `SheetHost` passes through to `component`.
   * @param systemPrefix - The write-target's system-body prefix `SheetHost` passes through.
   * @example this.#register(panelId, component, docId, "/system");
   */
  #register(panelId: string, component: unknown, docId: string, systemPrefix: string): void {
    const dispose = this.#deps.contributions.contribute(
      {
        id: panelId,
        contract: PANEL_CONTRACT,
        component: SheetHost,
        props: { docId, systemPrefix, close: () => this.closeDocument(panelId), inner: component },
        panel: { icon: "\u{1F4C4}", labelKey: "sheets.title", defaultPlacement: { kind: "floating" } },
      },
      { module: "sheets" },
    );
    this.#open.set(panelId, dispose);
  }
}

/** Every distinct `sheet:*` string anywhere in `blob` (deep walk — robust to the panel
 * persistence schema evolving; it only assumes sheet ids are strings prefixed `sheet:`).
 * @param blob - The opaque persisted panel-layout blob to walk.
 * @returns Every distinct `sheet:*`-prefixed string found, in first-encountered order.
 * @example collectSheetIds({ floating: ["sheet:d1"] }); // ["sheet:d1"]
 */
function collectSheetIds(blob: unknown): string[] {
  const found = new Set<string>();
  const walk = (v: unknown): void => {
    if (typeof v === "string") {
      if (v.startsWith("sheet:")) found.add(v);
    } else if (Array.isArray(v)) {
      for (const x of v) walk(x);
    } else if (v !== null && typeof v === "object") {
      for (const x of Object.values(v as Record<string, unknown>)) walk(x);
    }
  };
  walk(blob);
  return [...found];
}
