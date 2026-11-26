// The browser's per-account preferences, stored in the account's `ui_state.modules` slice for
// this module (never per world, never visible to another account). The server merge-patches a
// module slice per key, so the favorites array and the view mode are written independently; a
// written array replaces the previous array whole.
import { getUiState, putUiState } from "@shadowcat/core";

/** The module id keying this module's `ui_state.modules` slice. */
const MODULE_ID = "asset-browser";

/** How the browser lays out the listing. */
export type ViewMode = "grid" | "list";

/**
 * The account's favorite asset ids.
 * @returns The stored ids, or `[]` when none are stored.
 * @example
 * ```ts
 * import { getFavorites } from "./favorites";
 *
 * const ids = await getFavorites();
 * ```
 */
export async function getFavorites(): Promise<string[]> {
  const stored = (await getUiState()).modules[MODULE_ID]?.favorites;
  return Array.isArray(stored) ? stored.filter((x): x is string => typeof x === "string") : [];
}

/** Tail of the toggle chain: each toggle waits for the previous one's write, so two quick toggles
 * cannot both read the same stored list and drop one flip. */
let toggleChain: Promise<unknown> = Promise.resolve();

/**
 * Flips one asset's favorite state against the server's current list (so a toggle from a second
 * tab does not drop the first tab's additions), and stores the result.
 * @param id - The asset id to add when absent or remove when present.
 * @returns The stored list after the flip.
 * @example
 * ```ts
 * import { toggleFavorite } from "./favorites";
 *
 * const next = await toggleFavorite("00000000-0000-0000-0000-000000000001");
 * ```
 */
export function toggleFavorite(id: string): Promise<string[]> {
  const run = async (): Promise<string[]> => {
    const current = await getFavorites();
    const next = current.includes(id) ? current.filter((x) => x !== id) : [...current, id];
    await putUiState({ modules: { [MODULE_ID]: { favorites: next } } });
    return next;
  };
  const result = toggleChain.then(run, run);
  toggleChain = result.catch(() => {});
  return result;
}

/**
 * The account's stored layout choice.
 * @returns `"list"` when stored, else `"grid"`.
 * @example
 * ```ts
 * import { getViewMode } from "./favorites";
 *
 * const mode = await getViewMode();
 * ```
 */
export async function getViewMode(): Promise<ViewMode> {
  const stored = (await getUiState()).modules[MODULE_ID]?.viewMode;
  return stored === "list" ? "list" : "grid";
}

/**
 * Stores the account's layout choice.
 * @param mode - The layout to store.
 * @returns Resolves when the server accepted the write.
 * @example
 * ```ts
 * import { setViewMode } from "./favorites";
 *
 * await setViewMode("list");
 * ```
 */
export async function setViewMode(mode: ViewMode): Promise<void> {
  await putUiState({ modules: { [MODULE_ID]: { viewMode: mode } } });
}
