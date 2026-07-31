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

// Dirty-slice tracking: persist() sends ONLY the slices marked since the last
// successful write. Write granularity is the concurrency control — concurrent
// sessions of one account contend only on slices both actually write, so a
// session can never revert a slice it did not touch (the server merges per
// top-level key / per world id).
const dirty = { global: false, worlds: new Set<string>() };

function buildPatch(): UiStatePatch {
  const patch: UiStatePatch = {};
  if (dirty.global) patch.global = state.global;
  if (dirty.worlds.size > 0) {
    patch.worlds = {};
    for (const id of dirty.worlds) {
      const w = state.worlds[id];
      if (w) patch.worlds[id] = w;
    }
  }
  return patch;
}

async function persist(): Promise<void> {
  const hadGlobal = dirty.global;
  const hadWorlds = [...dirty.worlds];
  const patch = buildPatch();
  dirty.global = false;
  dirty.worlds.clear();
  if (patch.global === undefined && patch.worlds === undefined) return;
  try {
    await putUiState(patch);
  } catch (e) {
    // Re-mark the lost slices so the next scheduled persist retries them.
    if (hadGlobal) dirty.global = true;
    for (const id of hadWorlds) dirty.worlds.add(id);
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
        dirty.global = true;
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
  dirty.global = true;
  schedulePersist();
}

export function getPanelLayout(world: string): unknown | null {
  return state.worlds[world]?.panelLayout ?? null;
}

export function setPanelLayout(world: string, blob: unknown): void {
  const w = (state.worlds[world] ??= {});
  w.panelLayout = blob;
  dirty.worlds.add(world);
  schedulePersist();
}

export function getChatRead(world: string): unknown | null {
  return state.worlds[world]?.chatRead ?? null;
}

export function setChatRead(world: string, blob: unknown): void {
  const w = (state.worlds[world] ??= {});
  w.chatRead = blob;
  dirty.worlds.add(world);
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
 * closes first. `keepalive` lets the PUT survive the unload. */
export function flushOnUnload(): void {
  if (!loaded || (!dirty.global && dirty.worlds.size === 0)) return;
  const patch = buildPatch();
  dirty.global = false;
  dirty.worlds.clear();
  pendingDuringCooldown = false;
  void putUiState(patch, { keepalive: true }).catch((e) => logger.warn("ui_state unload flush failed", e));
}
