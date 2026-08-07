import { consoleLogger } from "@shadowcat/core";
import { getUiState, putUiState, type UiState, type UiStatePatch } from "./api";
import { i18n } from "@shadowcat/ui-kit";

const logger = consoleLogger();
let state: UiState = { global: { locale: "en", lastWorld: null }, worlds: {} };
let loaded = false;
let observing = false;

// Leading-edge debounce with a trailing catch-up (ui_state changes are user-paced;
// the leading edge persists promptly, the trailing flush captures a change made
// during the cooldown). See [[debounce-leading-edge-not-trailing-rearm]].
const COOLDOWN_MS = 500;
let timer: ReturnType<typeof setTimeout> | null = null;
let pendingDuringCooldown = false;

/** A dirty-trackable key of `UiState.global` (`"locale"` or `"lastWorld"`). */
type GlobalField = keyof UiState["global"];
/** A dirty-trackable key of one `UiState.worlds[id]` entry (`"panelLayout"` or
 * `"chatRead"`). */
type WorldKey = keyof UiState["worlds"][string];

/** Leaf-key dirty tracking: persist() sends ONLY the individual fields/keys
 * marked since the last successful write — the client-side half of the
 * per-key granularity stated once on `UiStatePatch`. Write granularity is the
 * concurrency control: concurrent sessions/mutators of one account contend
 * only on the individual leaves both actually touch, so a session can never
 * revert a leaf it did not touch (the server merges per top-level key, and
 * one level deeper inside `worlds.<id>` / `global`). */
const dirty = {
  global: new Set<GlobalField>(),
  worlds: new Map<string, Set<WorldKey>>(),
};

/** A structural copy of `dirty`, taken before clearing it for a persist
 * attempt so a failure can restore exactly what was lost. */
type DirtySnapshot = {
  /** Copy of the dirty global-field set at snapshot time. */
  global: Set<GlobalField>;
  /** Copy of the dirty per-world key sets at snapshot time, keyed by world id. */
  worlds: Map<string, Set<WorldKey>>;
};

/** Deep-copies the live `dirty` tracking into new `Set`/`Map` instances, not
 * references, so a subsequent `clearDirty()` cannot mutate the copy out from
 * under an in-flight persist attempt.
 * @returns A structural copy of `dirty`.
 * @example
 * ```
 * const snap = snapshotDirty();
 * ```
 */
function snapshotDirty(): DirtySnapshot {
  return {
    global: new Set(dirty.global),
    worlds: new Map([...dirty.worlds].map(([id, keys]) => [id, new Set(keys)])),
  };
}

/** Clears all dirty tracking: the global field set and every world's key set.
 * @example
 * ```
 * clearDirty();
 * ```
 */
function clearDirty(): void {
  dirty.global.clear();
  dirty.worlds.clear();
}

/** Re-adds every field/key from `snap` into the live dirty tracking (deep
 * re-add) — used when a persist attempt fails, so the next one retries
 * exactly what was lost, merged with anything marked in the meantime.
 * @param snap - A `snapshotDirty` snapshot to merge back into `dirty`.
 * @example
 * ```
 * declare const snap: DirtySnapshot;
 * remarkDirty(snap);
 * ```
 */
function remarkDirty(snap: DirtySnapshot): void {
  for (const field of snap.global) dirty.global.add(field);
  for (const [id, keys] of snap.worlds) {
    const set = dirty.worlds.get(id) ?? new Set<WorldKey>();
    for (const key of keys) set.add(key);
    dirty.worlds.set(id, set);
  }
}

/** Marks a single per-world key dirty, creating that world's key set on
 * first use.
 * @param world - World id.
 * @param key - The `UiState.worlds[string]` key that changed
 *   (`"panelLayout"` or `"chatRead"`).
 * @example
 * ```
 * markWorldDirty("w1", "panelLayout");
 * ```
 */
function markWorldDirty(world: string, key: WorldKey): void {
  const set = dirty.worlds.get(world) ?? new Set<WorldKey>();
  set.add(key);
  dirty.worlds.set(world, set);
}

/** Builds the `global` slice of a `UiStatePatch` from the currently dirty
 * global fields, reading current values from `state.global`.
 * @returns The dirty fields only, or `undefined` if no global field is dirty.
 * @example
 * ```
 * const patch = buildGlobalPatch();
 * ```
 */
function buildGlobalPatch(): Partial<UiState["global"]> | undefined {
  if (dirty.global.size === 0) return undefined;
  const patch: Partial<UiState["global"]> = {};
  if (dirty.global.has("locale")) patch.locale = state.global.locale;
  if (dirty.global.has("lastWorld")) patch.lastWorld = state.global.lastWorld;
  return patch;
}

/** Builds one world's slice of a `UiStatePatch` from a set of dirty keys,
 * reading current values from `state.worlds[id]`.
 * @param id - World id.
 * @param keys - The dirty keys for this world.
 * @returns The dirty keys only, or `undefined` if the world has no entry in
 *   `state` or no key is dirty.
 * @example
 * ```
 * const slice = buildWorldPatch("w1", new Set(["panelLayout"]));
 * ```
 */
function buildWorldPatch(
  id: string,
  keys: Set<WorldKey>,
): Partial<UiState["worlds"][string]> | undefined {
  const w = state.worlds[id];
  if (!w || keys.size === 0) return undefined;
  const slice: Partial<UiState["worlds"][string]> = {};
  if (keys.has("panelLayout")) slice.panelLayout = w.panelLayout;
  if (keys.has("chatRead")) slice.chatRead = w.chatRead;
  return slice;
}

/** Builds a full `UiStatePatch` from the current `dirty` tracking: a
 * `global` slice via `buildGlobalPatch`, plus one `worlds[id]` slice per
 * world whose dirty keys still resolve through `buildWorldPatch`. Omits
 * `global`/`worlds` entirely when there is nothing to send on that side.
 * @returns The patch to PUT — may have neither `global` nor `worlds` set if
 *   nothing is dirty.
 * @example
 * ```
 * const patch = buildPatch();
 * ```
 */
function buildPatch(): UiStatePatch {
  const patch: UiStatePatch = {};
  const global = buildGlobalPatch();
  if (global) patch.global = global;
  if (dirty.worlds.size > 0) {
    const worlds: Record<string, Partial<UiState["worlds"][string]>> = {};
    for (const [id, keys] of dirty.worlds) {
      const slice = buildWorldPatch(id, keys);
      if (slice) worlds[id] = slice;
    }
    if (Object.keys(worlds).length > 0) patch.worlds = worlds;
  }
  return patch;
}

/** Builds a patch from the currently dirty fields/keys, clears the dirty
 * tracking, and PUTs it. On failure, re-marks exactly the fields/keys this
 * attempt captured — merged with anything marked since, via `remarkDirty` —
 * so the next scheduled persist retries them; on success the clear is
 * permanent. A no-op (no PUT; dirty tracking stays cleared) when the current
 * dirty state builds an empty patch.
 * @example
 * ```
 * await persist();
 * ```
 */
async function persist(): Promise<void> {
  const snap = snapshotDirty();
  const patch = buildPatch();
  clearDirty();
  if (patch.global === undefined && patch.worlds === undefined) return;
  try {
    await putUiState(patch);
  } catch (e) {
    // Re-mark the lost fields/keys so the next scheduled persist retries them.
    remarkDirty(snap);
    logger.warn("ui_state persist failed", e);
  }
}

/** Leading-edge debounced trigger for `persist()`: the first call after
 * `loaded` becomes true persists immediately and starts a `COOLDOWN_MS`
 * cooldown; calls arriving during the cooldown only set
 * `pendingDuringCooldown`. On cooldown expiry, a pending call re-triggers
 * `schedulePersist()` — persisting again immediately and starting a fresh
 * cooldown — exactly once, rather than resetting the cooldown on every call.
 * @example
 * ```
 * schedulePersist();
 * ```
 */
function schedulePersist(): void {
  if (!loaded) return; // don't write back during the initial restore
  if (timer === null) {
    void persist(); // leading edge
    timer = setTimeout(() => {
      timer = null;
      if (pendingDuringCooldown) {
        pendingDuringCooldown = false;
        schedulePersist();
      }
    }, COOLDOWN_MS);
  } else {
    pendingDuringCooldown = true;
  }
}

/** Fetches the UI-state blob, applies its saved locale, marks the module
 * loaded, and — once per process lifetime — starts observing future locale
 * changes to persist them. Clears any dirty tracking left over from a prior
 * session first, so a stale marker from a previous account never bleeds into
 * the freshly loaded state. Safe to call again on re-login: the `observing`
 * latch stops a second call from stacking a second locale-change listener.
 * @returns The freshly loaded `UiState`.
 * @example
 * ```
 * const ui = await loadSessionState();
 * ```
 */
export async function loadSessionState(): Promise<UiState> {
  // Re-login hygiene: a stale dirty marker from a previous session/account
  // must never bleed into the freshly loaded one.
  clearDirty();
  state = await getUiState();
  // Apply locale before marking loaded so the initial apply does not persist.
  if (i18n.locale !== state.global.locale) i18n.setLocale(state.global.locale);
  loaded = true;
  // Observe future locale changes (switcher, etc.) and persist them — once for the
  // process lifetime (load runs again on re-login; the singleton must not stack
  // listeners).
  if (!observing) {
    observing = true;
    i18n.subscribe(() => {
      if (state.global.locale !== i18n.locale) {
        state.global.locale = i18n.locale;
        dirty.global.add("locale");
        schedulePersist();
      }
    });
  }
  return state;
}

/** The in-memory `UiState`, as last loaded/mutated. Not reactive — `state`
 * is a plain module variable, not a rune — so a caller that needs to react
 * to a change re-reads after the mutating call (`setLastWorld`,
 * `setPanelLayout`, `setChatRead`) returns.
 * @returns The current session state.
 * @example
 * ```
 * const ui = getSessionState();
 * ```
 */
export function getSessionState(): UiState {
  return state;
}

/** Sets `global.lastWorld`, marks it dirty, and schedules a persist. A no-op
 * (no dirty mark, no persist) if `id` already equals the current value.
 * @param id - The world id to remember, or `null` to clear it.
 * @example
 * ```
 * setLastWorld("w1");
 * ```
 */
export function setLastWorld(id: string | null): void {
  if (state.global.lastWorld === id) return;
  state.global.lastWorld = id;
  dirty.global.add("lastWorld");
  schedulePersist();
}

/** Reads a world's persisted panel-layout blob. The shell never inspects its
 * shape — ownership belongs to `@shadowcat/module-panels`' persistence codec.
 * @param world - World id.
 * @returns The stored blob, or `null` if the world has none.
 * @example
 * ```
 * const layout = getPanelLayout("w1");
 * ```
 */
export function getPanelLayout(world: string): unknown | null {
  return state.worlds[world]?.panelLayout ?? null;
}

/** Stores a world's panel-layout blob, marks it dirty, and schedules a
 * persist. Creates the world's entry in `state.worlds` if it does not exist
 * yet.
 * @param world - World id.
 * @param blob - The opaque layout blob (see `getPanelLayout`).
 * @example
 * ```
 * declare const encodedLayout: unknown;
 * setPanelLayout("w1", encodedLayout);
 * ```
 */
export function setPanelLayout(world: string, blob: unknown): void {
  const w = (state.worlds[world] ??= {});
  w.panelLayout = blob;
  markWorldDirty(world, "panelLayout");
  schedulePersist();
}

/** Reads a world's persisted chat last-read-marker blob. The shell never
 * inspects its shape — ownership belongs to the chat module.
 * @param world - World id.
 * @returns The stored blob, or `null` if the world has none.
 * @example
 * ```
 * const read = getChatRead("w1");
 * ```
 */
export function getChatRead(world: string): unknown | null {
  return state.worlds[world]?.chatRead ?? null;
}

/** Stores a world's chat last-read-marker blob, marks it dirty, and
 * schedules a persist. Creates the world's entry in `state.worlds` if it
 * does not exist yet.
 * @param world - World id.
 * @param blob - The opaque last-read-marker blob (see `getChatRead`).
 * @example
 * ```
 * declare const marker: unknown;
 * setChatRead("w1", marker);
 * ```
 */
export function setChatRead(world: string, blob: unknown): void {
  const w = (state.worlds[world] ??= {});
  w.chatRead = blob;
  markWorldDirty(world, "chatRead");
  schedulePersist();
}

/** Force any pending persist to run now (test/teardown helper). Cancels the
 * cooldown timer and the pending-during-cooldown flag, then calls
 * `persist()` directly — whatever is dirty at call time is sent immediately.
 * @example
 * ```
 * await flushSessionState();
 * ```
 */
export async function flushSessionState(): Promise<void> {
  if (timer) {
    clearTimeout(timer);
    timer = null;
  }
  pendingDuringCooldown = false;
  await persist();
}

/** Best-effort flush on page hide/unload: a change made during the cooldown is
 * otherwise only written by the trailing timer, which never fires if the tab
 * closes first. `keepalive` lets the PUT survive the unload. Uses the same
 * snapshot/clear/re-mark-on-failure shape as `persist()` — a rejected
 * keepalive PUT (the request can still fail even though the tab is closing,
 * e.g. mid-navigation) must not silently drop the fields/keys it carried.
 * @example
 * ```
 * window.addEventListener("pagehide", flushOnUnload);
 * ```
 */
export function flushOnUnload(): void {
  if (!loaded || (dirty.global.size === 0 && dirty.worlds.size === 0)) return;
  const snap = snapshotDirty();
  const patch = buildPatch();
  clearDirty();
  pendingDuringCooldown = false;
  void putUiState(patch, { keepalive: true }).catch((e) => {
    remarkDirty(snap);
    logger.warn("ui_state unload flush failed", e);
  });
}
