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

type GlobalField = keyof UiState["global"];
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
  global: Set<GlobalField>;
  worlds: Map<string, Set<WorldKey>>;
};

function snapshotDirty(): DirtySnapshot {
  return {
    global: new Set(dirty.global),
    worlds: new Map([...dirty.worlds].map(([id, keys]) => [id, new Set(keys)])),
  };
}

function clearDirty(): void {
  dirty.global.clear();
  dirty.worlds.clear();
}

/** Re-adds every field/key from `snap` into the live dirty tracking (deep
 * re-add) — used when a persist attempt fails, so the next one retries
 * exactly what was lost, merged with anything marked in the meantime. */
function remarkDirty(snap: DirtySnapshot): void {
  for (const field of snap.global) dirty.global.add(field);
  for (const [id, keys] of snap.worlds) {
    const set = dirty.worlds.get(id) ?? new Set<WorldKey>();
    for (const key of keys) set.add(key);
    dirty.worlds.set(id, set);
  }
}

function markWorldDirty(world: string, key: WorldKey): void {
  const set = dirty.worlds.get(world) ?? new Set<WorldKey>();
  set.add(key);
  dirty.worlds.set(world, set);
}

function buildGlobalPatch(): Partial<UiState["global"]> | undefined {
  if (dirty.global.size === 0) return undefined;
  const patch: Partial<UiState["global"]> = {};
  if (dirty.global.has("locale")) patch.locale = state.global.locale;
  if (dirty.global.has("lastWorld")) patch.lastWorld = state.global.lastWorld;
  return patch;
}

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

/** Fetch the blob, apply the saved locale, and start observing locale changes. */
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

export function getSessionState(): UiState {
  return state;
}

export function setLastWorld(id: string | null): void {
  if (state.global.lastWorld === id) return;
  state.global.lastWorld = id;
  dirty.global.add("lastWorld");
  schedulePersist();
}

export function getPanelLayout(world: string): unknown | null {
  return state.worlds[world]?.panelLayout ?? null;
}

export function setPanelLayout(world: string, blob: unknown): void {
  const w = (state.worlds[world] ??= {});
  w.panelLayout = blob;
  markWorldDirty(world, "panelLayout");
  schedulePersist();
}

export function getChatRead(world: string): unknown | null {
  return state.worlds[world]?.chatRead ?? null;
}

export function setChatRead(world: string, blob: unknown): void {
  const w = (state.worlds[world] ??= {});
  w.chatRead = blob;
  markWorldDirty(world, "chatRead");
  schedulePersist();
}

/** Force any pending persist to run now (test/teardown helper). */
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
 * e.g. mid-navigation) must not silently drop the fields/keys it carried. */
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
