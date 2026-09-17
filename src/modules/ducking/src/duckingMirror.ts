import { DEFAULT_KEY } from "./keySource";
import { DEFAULT_THRESHOLD } from "./osMonitor";
import { DEFAULT_SENSITIVITY } from "./micVad";

/** Persisted per-device ducking preferences, mirrored to `localStorage` under
 * `shadowcat.ducking`. */
export interface DuckingPreferences {
  /** Master enable — off disables all three sources regardless of their own toggles. */
  masterEnabled: boolean;
  /** Push-to-duck key source enable. */
  keyEnabled: boolean;
  /** The bound key's `KeyboardEvent.code`. */
  keyBinding: string;
  /** Mic voice-activity source enable. */
  micEnabled: boolean;
  /** Mic VAD sensitivity multiplier. */
  micSensitivity: number;
  /** OS audio-session monitor source enable. */
  osEnabled: boolean;
  /** The `shadowcat audio-monitor` port to connect to. */
  osPort: number;
  /** Peak threshold above which a watched session counts as "talking". */
  osThreshold: number;
  /** Watched-process substrings, case-insensitive (also the monitor's live `watch` list). */
  watchList: string[];
}

// Duck DEPTH is deliberately absent: it is `ctx.audio.duck.depth`, persisted by the audio
// engine's own `shadowcat.audio` mirror, never a second copy here — one stored value per fact.

/** The single localStorage key holding the ducking preferences mirror — this module's own
 * read/write pair, styled after `sessionState.svelte.ts`'s `readThemeMirror`/
 * `writeThemeMirror` (garbage-tolerant read, best-effort write). */
export const DUCKING_MIRROR_STORAGE_KEY = "shadowcat.ducking";

/** The preferences a fresh device starts with. */
export const DEFAULT_DUCKING_PREFERENCES: DuckingPreferences = {
  masterEnabled: true,
  keyEnabled: false,
  keyBinding: DEFAULT_KEY,
  micEnabled: false,
  micSensitivity: DEFAULT_SENSITIVITY,
  osEnabled: false,
  osPort: 31998,
  osThreshold: DEFAULT_THRESHOLD,
  watchList: ["discord"],
};

/**
 * Reads the ducking mirror, garbage-tolerantly: an absent key, malformed JSON, or a
 * non-object payload all yield the defaults; a partial object fills missing fields from the
 * defaults (forward-compatible with a preferences field added later).
 * @param storage The storage to read (injectable for tests; production passes `localStorage`).
 * @returns The persisted preferences, or the defaults when absent/unreadable.
 * @example
 * ```
 * const prefs = readDuckingMirror(localStorage);
 * ```
 */
export function readDuckingMirror(storage: Pick<Storage, "getItem">): DuckingPreferences {
  const raw = storage.getItem(DUCKING_MIRROR_STORAGE_KEY);
  if (raw === null) return { ...DEFAULT_DUCKING_PREFERENCES };
  try {
    const parsed: unknown = JSON.parse(raw);
    if (typeof parsed !== "object" || parsed === null) return { ...DEFAULT_DUCKING_PREFERENCES };
    return { ...DEFAULT_DUCKING_PREFERENCES, ...(parsed as Partial<DuckingPreferences>) };
  } catch {
    return { ...DEFAULT_DUCKING_PREFERENCES };
  }
}

/**
 * Writes the ducking mirror. A throwing storage (quota, privacy mode) is swallowed — a
 * failed write must never break the settings change that triggered it.
 * @param storage The storage to write (injectable for tests; production passes
 *   `localStorage`).
 * @param value The preferences snapshot to persist.
 * @example
 * ```
 * writeDuckingMirror(localStorage, DEFAULT_DUCKING_PREFERENCES);
 * ```
 */
export function writeDuckingMirror(storage: Pick<Storage, "setItem">, value: DuckingPreferences): void {
  try {
    storage.setItem(DUCKING_MIRROR_STORAGE_KEY, JSON.stringify(value));
  } catch {
    // best-effort; see the function doc above.
  }
}
