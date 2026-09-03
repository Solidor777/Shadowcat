import { consoleLogger } from "@shadowcat/core";
import { getUiState, putUiState, type UiState, type UiStatePatch } from "./api";
import { i18n, theme, type PersistedTheme } from "@shadowcat/ui-kit";

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

/** The current `persist()` attempt's PUT, while one is unresolved — `persist()`
 * checks this before starting a second overlapping PUT. */
let persistInFlight: Promise<void> | null = null;
/** The single coalesced retry `persist()` schedules once `persistInFlight`
 * settles — every call arriving while a PUT is in flight shares this SAME
 * promise (never one retry per caller) and awaits it, so `persistInFlight`
 * settling always resolves every waiting caller together, once. */
let persistQueued: Promise<void> | null = null;

/** A dirty-trackable key of `UiState.global` (`"locale"`, `"lastWorld"`, or
 * `"theme"`). */
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
  /** World ids whose ENTIRE `worlds.<id>` entry should be removed on the next persist — distinct
   * from `worlds`, which tracks per-key dirty leaves of an entry that still exists. Populated only
   * by `pruneStaleWorlds`. */
  removedWorlds: new Set<string>(),
};

/** A structural copy of `dirty`, taken before clearing it for a persist
 * attempt so a failure can restore exactly what was lost. */
type DirtySnapshot = {
  /** Copy of the dirty global-field set at snapshot time. */
  global: Set<GlobalField>;
  /** Copy of the dirty per-world key sets at snapshot time, keyed by world id. */
  worlds: Map<string, Set<WorldKey>>;
  /** Copy of the dirty removed-world-id set at snapshot time. */
  removedWorlds: Set<string>;
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
    removedWorlds: new Set(dirty.removedWorlds),
  };
}

/** Clears all dirty tracking: the global field set, every world's key set, and the
 * whole-entry-removal set.
 * @example
 * ```
 * clearDirty();
 * ```
 */
function clearDirty(): void {
  dirty.global.clear();
  dirty.worlds.clear();
  dirty.removedWorlds.clear();
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
  for (const id of snap.removedWorlds) dirty.removedWorlds.add(id);
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

/** Copies `field`'s current value from `g` into `patch`, keyed by `field` itself. A `switch`
 * (not an `if` chain) so a `GlobalField` union widened by a new `UiState.global` property fails to
 * compile here (the `default` branch's `field satisfies never` check) rather than silently
 * dropping the new field from every patch.
 * @param patch The in-progress global patch slice to write into.
 * @param g The current `UiState.global` to read from.
 * @param field The dirty field to copy.
 * @example
 * ```ts
 * declare const patch: Partial<UiState["global"]>;
 * declare const g: UiState["global"];
 * copyGlobalField(patch, g, "locale");
 * ```
 */
function copyGlobalField(
  patch: Partial<UiState["global"]>,
  g: UiState["global"],
  field: GlobalField,
): void {
  switch (field) {
    case "locale":
      patch.locale = g.locale;
      return;
    case "lastWorld":
      patch.lastWorld = g.lastWorld;
      return;
    case "theme":
      patch.theme = g.theme;
      return;
    default:
      field satisfies never;
  }
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
  for (const field of dirty.global) copyGlobalField(patch, state.global, field);
  return patch;
}

/** Copies `key`'s current value from `w` into `slice`, keyed by `key` itself. Same exhaustiveness
 * discipline as `copyGlobalField` — see its doc for why a `switch` over the leaf-key union,
 * not an `if` chain.
 * @param slice The in-progress world patch slice to write into.
 * @param w The current `UiState.worlds[id]` entry to read from.
 * @param key The dirty key to copy.
 * @example
 * ```ts
 * declare const slice: Partial<{ panelLayout?: unknown; chatRead?: unknown }>;
 * declare const w: { panelLayout?: unknown; chatRead?: unknown };
 * copyWorldKey(slice, w, "panelLayout");
 * ```
 */
function copyWorldKey(
  slice: Partial<UiState["worlds"][string]>,
  w: UiState["worlds"][string],
  key: WorldKey,
): void {
  switch (key) {
    case "panelLayout":
      slice.panelLayout = w.panelLayout;
      return;
    case "chatRead":
      slice.chatRead = w.chatRead;
      return;
    default:
      key satisfies never;
  }
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
  for (const key of keys) copyWorldKey(slice, w, key);
  return slice;
}

/** Builds a full `UiStatePatch` from the current `dirty` tracking: a
 * `global` slice via `buildGlobalPatch`, one `worlds[id]` slice per
 * world whose dirty keys still resolve through `buildWorldPatch`, and a `null` entry for
 * each world id marked in `dirty.removedWorlds` (see `pruneStaleWorlds`) — written LAST so a
 * removal wins over any leftover per-key slice for the same id, though `pruneStaleWorlds`
 * itself already prevents that id from appearing in both sets by construction. Omits
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
  if (dirty.worlds.size > 0 || dirty.removedWorlds.size > 0) {
    const worlds: Record<string, Partial<UiState["worlds"][string]> | null> = {};
    for (const [id, keys] of dirty.worlds) {
      const slice = buildWorldPatch(id, keys);
      if (slice) worlds[id] = slice;
    }
    for (const id of dirty.removedWorlds) worlds[id] = null;
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
 *
 * In-flight-PUT ordering guard: a call arriving while an earlier call's
 * `putUiState` is still unresolved does NOT start a second overlapping PUT.
 * It shares (creating if absent) `persistQueued`, a single coalesced retry
 * chained onto `persistInFlight` — so every such caller awaits the SAME
 * eventual retry rather than each firing its own, and none of them resolve
 * until that retry (which snapshots/builds/clears dirty tracking at ITS OWN
 * run time, picking up everything dirtied since) has actually completed —
 * `flushSessionState()`'s callers depend on this: a call made while a PUT is
 * in flight still awaits a real, later write, not an immediate no-op return.
 * @returns Nothing; resolves once the (possibly coalesced) PUT this call is waiting on
 * has completed, or immediately if the current dirty state builds an empty patch.
 * @example
 * ```
 * await persist();
 * ```
 */
async function persist(): Promise<void> {
  if (persistInFlight) {
    persistQueued ??= persistInFlight.then(() => {
      persistQueued = null;
      return persist();
    });
    return persistQueued;
  }
  const snap = snapshotDirty();
  const patch = buildPatch();
  clearDirty();
  if (patch.global === undefined && patch.worlds === undefined) return;
  persistInFlight = (async () => {
    try {
      await putUiState(patch);
    } catch (e) {
      // Re-mark the lost fields/keys so the next scheduled persist retries them.
      remarkDirty(snap);
      logger.warn("ui_state persist failed", e);
    }
  })();
  try {
    await persistInFlight;
  } finally {
    persistInFlight = null;
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

/** The single localStorage key holding the theme mirror: the last-loaded
 * `UiState.global.theme` value, applied pre-login by the app entry so the
 * login/world-select screens honor the last-used theme. Deliberately the
 * codebase's only localStorage use — a cosmetic, non-secret preference. */
export const THEME_MIRROR_STORAGE_KEY = "shadowcat.theme";

/** Reads the theme mirror, garbage-tolerantly: an absent key, malformed JSON,
 * or a non-object payload all yield `undefined` (which `ThemeController.load`
 * resolves to the default theme). Deep validation of the parsed shape is
 * `ThemeController.load`'s job — this helper only parses.
 * @param storage The storage to read (injectable for tests; the app entry
 *   passes `localStorage`).
 * @returns The mirrored value, or `undefined` when absent or unreadable.
 * @example
 * ```ts
 * const mirror = readThemeMirror(localStorage);
 * ```
 */
export function readThemeMirror(storage: Pick<Storage, "getItem">): PersistedTheme | undefined {
  const raw = storage.getItem(THEME_MIRROR_STORAGE_KEY);
  if (raw === null) return undefined;
  try {
    const parsed: unknown = JSON.parse(raw);
    if (typeof parsed !== "object" || parsed === null) return undefined;
    return parsed as PersistedTheme;
  } catch {
    return undefined;
  }
}

/** Writes the theme mirror. A throwing storage (quota, privacy mode) is
 * swallowed with a log — the mirror is a pre-login cosmetic convenience and a
 * failed write must never break the theme change that triggered it.
 * @param storage The storage to write (injectable for tests; callers pass
 *   `localStorage`).
 * @param value The canonical `ThemeController.serialize` output to mirror.
 * @example
 * ```ts
 * writeThemeMirror(localStorage, theme.serialize());
 * ```
 */
export function writeThemeMirror(storage: Pick<Storage, "setItem">, value: PersistedTheme): void {
  try {
    storage.setItem(THEME_MIRROR_STORAGE_KEY, JSON.stringify(value));
  } catch (e) {
    logger.warn("theme mirror write failed", e);
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
  // Same for the theme: `ThemeController.load` tolerates garbage (falling back
  // to the default theme), then the in-memory slice is canonicalized to
  // `ThemeController.serialize`'s output so the change subscriber below
  // compares against a canonical baseline. The mirror is overwritten on every
  // load, whatever a previous tab or session left behind.
  theme.load(state.global.theme);
  state.global.theme = theme.serialize();
  if (typeof localStorage !== "undefined") writeThemeMirror(localStorage, state.global.theme);
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
    theme.subscribe(() => {
      // Unlike the locale arm above, an equality check alone cannot tell a
      // `loadSessionState` apply apart from a user change (`ThemeController.load`
      // canonicalizes, so the raw loaded slice legitimately differs from the
      // serialized one) — the `loaded` guard does that instead, and the load
      // path above writes the mirror explicitly.
      if (!loaded) return;
      const serialized = theme.serialize();
      if (JSON.stringify(state.global.theme) !== JSON.stringify(serialized)) {
        state.global.theme = serialized;
        dirty.global.add("theme");
        if (typeof localStorage !== "undefined") writeThemeMirror(localStorage, serialized);
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

/** Removes every local `worlds.<id>` entry whose id is NOT in `memberWorldIds`, marking each for
 * whole-entry removal on the next persist (`merge_one_level`'s null-removes-key semantics
 * server-side) and scheduling that persist. A no-op if nothing is stale. This is how an
 * accumulated-forever `ui_state.worlds` blob (a world the caller left, or whose access was
 * revoked) actually gets pruned — see `App.svelte`'s `boot()` for the call site and why it's tied
 * to an existing `listWorlds()` fetch rather than an added unconditional one.
 * @param memberWorldIds Every world id the caller currently has access to.
 * @example
 * ```ts
 * declare const memberWorldIds: string[];
 * pruneStaleWorlds(memberWorldIds);
 * ```
 */
export function pruneStaleWorlds(memberWorldIds: string[]): void {
  const memberSet = new Set(memberWorldIds);
  const staleIds = Object.keys(state.worlds).filter((id) => !memberSet.has(id));
  if (staleIds.length === 0) return;
  for (const id of staleIds) {
    delete state.worlds[id];
    dirty.worlds.delete(id);
    dirty.removedWorlds.add(id);
  }
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
  if (
    !loaded ||
    (dirty.global.size === 0 && dirty.worlds.size === 0 && dirty.removedWorlds.size === 0)
  )
    return;
  const snap = snapshotDirty();
  const patch = buildPatch();
  clearDirty();
  pendingDuringCooldown = false;
  void putUiState(patch, { keepalive: true }).catch((e) => {
    remarkDirty(snap);
    logger.warn("ui_state unload flush failed", e);
  });
}

/** Resets session-scoped in-memory state on logout: cancels the cooldown timer, clears dirty
 * tracking, and resets `loaded` to `false`. Without this, a mutation landing inside a re-login
 * `loadSessionState()`'s `await getUiState()` window could pass the `loaded` guard and persist a
 * pre-login `state` value under the new session's cookie — `clearDirty()` at load start only
 * covers the dirty-marker half of re-login hygiene, not the guard itself.
 * @example
 * ```
 * resetSessionState();
 * ```
 */
export function resetSessionState(): void {
  if (timer) {
    clearTimeout(timer);
    timer = null;
  }
  pendingDuringCooldown = false;
  clearDirty();
  loaded = false;
}
