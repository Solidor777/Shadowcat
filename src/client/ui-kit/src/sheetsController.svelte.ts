// Owns the lifecycle of document (sheet) panels (M12c). Each open document is a runtime
// `Contribution` under `shadowcat.panel` with id `sheet:<docId>` — the panel host mounts
// it via `{#each visibleRegs}` and the layout reducer floats/docks/minimizes it like any
// panel. This controller is generic host glue (constructed by the shell alongside
// `PanelsBridge`); it imports no module. Dedup + disposer tracking live here; placement,
// cascade, focus, and prune live in the panel manager.
import { PANEL_CONTRACT, pickSheet, resolveDocRef, type ContributionRegistry, type Logger, type ReadableDocuments, type SheetRef } from "@shadowcat/core";
import type { PanelsApi } from "./panelsBridge.svelte";

export interface SheetsControllerDeps {
  contributions: ContributionRegistry;
  documents: ReadableDocuments;
  panels: PanelsApi;
  logger: Logger;
}

export class SheetsController {
  #deps: SheetsControllerDeps;
  /** panelId -> the contribution disposer, for every sheet this controller has registered. */
  #open = new Map<string, () => void>();

  constructor(deps: SheetsControllerDeps) {
    this.#deps = deps;
  }

  /** Resolve `ref` -> doc + write-site (fail-closed), pick the sheet component, and
   * register+float `sheet:<docId>`. A re-open of an already-registered document focuses
   * the existing panel (never a duplicate — keep-mounted state survives). */
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
   * sheet component unmounts). The header Close control routes here. */
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
   * `sheet:` id shape, not the blob's exact schema). An unresolvable id is left pruned. */
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

  #register(panelId: string, component: unknown, docId: string, systemPrefix: string): void {
    const dispose = this.#deps.contributions.contribute(
      {
        id: panelId,
        contract: PANEL_CONTRACT,
        component,
        props: { docId, systemPrefix, close: () => this.closeDocument(panelId) },
        panel: { icon: "\u{1F4C4}", labelKey: "sheets.title", defaultPlacement: { kind: "floating" } },
      },
      { module: "sheets" },
    );
    this.#open.set(panelId, dispose);
  }
}

/** Every distinct `sheet:*` string anywhere in `blob` (deep walk — robust to the panel
 * persistence schema evolving; it only assumes sheet ids are strings prefixed `sheet:`). */
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
