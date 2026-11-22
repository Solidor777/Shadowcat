// Persistent per-module settings behind `ModuleContext.settings`. Two stores, each reusing an
// existing primitive rather than adding a third:
//  - world-scoped: one slice per module id in the `system` band of the world's singleton
//    `module-settings` document, written through the ordinary field-path Update (OCC, permission
//    filtering and broadcast come with it);
//  - account-scoped: one slice per module id under `ui_state.modules`, written through the
//    merge-patch route (`putUiState`).
// Modules are admin-trusted, so isolation between module slices is by construction of the
// `ModuleContext` (the id is stamped, never passed), not by a server-side ACL.
import { putUiState, type UiState } from "./user-rest";
import { buildUpdate, type FieldEdit } from "./update-op";
import type { ReadableDocuments } from "./store";
import type { WireOperation } from "./wire";

/** Doc_type of the world's singleton module-settings document. Mirrors the server's
 * `MODULE_SETTINGS_DOC_TYPE`. */
export const MODULE_SETTINGS_DOC_TYPE = "module-settings";

/** Escapes one JSON-pointer reference token (RFC 6901: `~` then `/`).
 * @param token The raw key.
 * @returns The token safe to join into a pointer.
 * @example
 * ```ts
 * import { pointerToken } from "@shadowcat/core";
 *
 * pointerToken("a/b"); // "a~1b"
 * ```
 */
export function pointerToken(token: string): string {
  return token.replace(/~/g, "~0").replace(/\//g, "~1");
}

/** The module's slice of the world's `module-settings` document, read through `docs`.
 * @param docs The document view to read (the optimistic view, so a pending write is visible).
 * @param moduleId The module whose slice to read.
 * @returns The slice, or `{}` when the document or the slice is absent.
 * @example
 * ```ts
 * import { OptimisticClient, readWorldModuleSettings } from "@shadowcat/core";
 *
 * const client = new OptimisticClient("00000000-0000-0000-0000-000000000001");
 * readWorldModuleSettings(client, "example-module"); // {}
 * ```
 */
export function readWorldModuleSettings(
  docs: ReadableDocuments,
  moduleId: string,
): Record<string, unknown> {
  const system = docs.query(MODULE_SETTINGS_DOC_TYPE)[0]?.system as Record<string, unknown> | null | undefined;
  const slice = system?.[moduleId];
  return slice !== null && typeof slice === "object" ? (slice as Record<string, unknown>) : {};
}

/** Builds the ONE Update that merges `patch` into the module's world slice: one `FieldChange` per
 * top-level patch key at `/system/<moduleId>/<key>`, each carrying its own OCC pre-image. A
 * field-path Update touches only the leaves it names, so sibling keys stay untouched; one change
 * replacing `/system/<moduleId>` would drop them.
 * @param docs The document view to read the singleton and the pre-images from.
 * @param moduleId The module whose slice is written.
 * @param patch The keys to merge.
 * @returns The Update operation.
 * @throws {Error} When the world has no `module-settings` document in `docs`.
 * @example
 * ```ts
 * import { OptimisticClient, buildWorldModuleSettingsUpdate } from "@shadowcat/core";
 *
 * const client = new OptimisticClient("00000000-0000-0000-0000-000000000001");
 * try {
 *   buildWorldModuleSettingsUpdate(client, "example-module", { volume: 3 });
 * } catch {
 *   // no module-settings document yet
 * }
 * ```
 */
export function buildWorldModuleSettingsUpdate(
  docs: ReadableDocuments,
  moduleId: string,
  patch: Record<string, unknown>,
): WireOperation {
  const doc = docs.query(MODULE_SETTINGS_DOC_TYPE)[0];
  if (!doc) throw new Error("the world has no module-settings document");
  const current = readWorldModuleSettings(docs, moduleId);
  const edits: FieldEdit[] = Object.entries(patch).map(([key, value]) => ({
    path: `/system/${pointerToken(moduleId)}/${pointerToken(key)}`,
    old: current[key],
    value,
  }));
  return buildUpdate(doc.id, edits);
}

/** The account-scoped half of `ModuleContext.settings`: a snapshot of `ui_state.modules` plus a
 * write-through merge. The snapshot is seeded from the loaded UI state and mutated in place on
 * every write, so a later reader holding the same object sees it. Reads are a plain snapshot: the
 * UI-state route is fetched, never pushed. */
export class UserModuleSettings {
  /** Builds the store over an already-loaded `modules` map.
   * @param modules The `UiState.modules` object; mutated in place by `patch`.
   * @param put The UI-state write (defaults to `putUiState`).
   * @example
   * ```ts
   * import { UserModuleSettings } from "@shadowcat/core";
   *
   * const settings = new UserModuleSettings({});
   * settings.get("example-module"); // {}
   * ```
   */
  constructor(
    private readonly modules: UiState["modules"],
    private readonly put: typeof putUiState = putUiState,
  ) {}

  /** The module's slice.
   * @param moduleId The module whose slice to read.
   * @returns The slice, or `{}` when unset.
   * @example
   * ```ts
   * import { UserModuleSettings } from "@shadowcat/core";
   *
   * new UserModuleSettings({ "example-module": { a: 1 } }).get("example-module"); // { a: 1 }
   * ```
   */
  get(moduleId: string): Record<string, unknown> {
    return this.modules[moduleId] ?? {};
  }

  /** Writes `patch` to the server, then merges it into the snapshot (a `null` value removes that
   * key, matching the server's merge). A rejected write leaves the snapshot unchanged.
   * @param moduleId The module whose slice to merge into.
   * @param patch The keys to merge; a `null` value removes that key.
   * @returns Resolves when the server accepted the patch.
   * @example
   * ```ts
   * import { UserModuleSettings } from "@shadowcat/core";
   *
   * const settings = new UserModuleSettings({}, async () => {});
   * await settings.patch("example-module", { volume: 3 });
   * ```
   */
  async patch(moduleId: string, patch: Record<string, unknown>): Promise<void> {
    await this.put({ modules: { [moduleId]: patch } });
    const slice = (this.modules[moduleId] ??= {});
    for (const [key, value] of Object.entries(patch)) {
      if (value === null) delete slice[key];
      else slice[key] = value;
    }
  }
}
