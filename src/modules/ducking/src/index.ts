import { consoleLogger, SETTINGS_SECTION_CONTRACT, type Module } from "@shadowcat/core";
import DuckingSettings from "./DuckingSettings.svelte";
import DuckingRuntime from "./DuckingRuntime.svelte";
import { DuckSourcesController } from "./controller";
import { readDuckingMirror } from "./duckingMirror";

export { KeySource, NULL_SINK, DEFAULT_KEY, type DuckSink } from "./keySource";
export {
  MicVadSource,
  VadEngine,
  VAD_PROCESSOR_NAME,
  DEFAULT_SENSITIVITY,
  VAD_FRAME_MS,
  type MicVadDeps,
  type MicVadDenialReason,
} from "./micVad";
export { OsMonitorSource, DEFAULT_THRESHOLD, type OsMonitorStatus } from "./osMonitor";
export { DuckSourcesController } from "./controller";
export {
  readDuckingMirror,
  writeDuckingMirror,
  DEFAULT_DUCKING_PREFERENCES,
  DUCKING_MIRROR_STORAGE_KEY,
  type DuckingPreferences,
} from "./duckingMirror";
export { default as DuckingSettings } from "./DuckingSettings.svelte";
export { default as DuckingRuntime } from "./DuckingRuntime.svelte";

/** The active controller, set by `register` and torn down by `unregister` — module-scoped
 * rather than closure-captured per call because `unregister` needs it and the `Module`
 * interface gives no other channel between the two. Safe as a singleton because exactly one
 * `WorldSession` (and therefore one active `ducking` registration) exists per browser tab
 * (`App.svelte`'s single `session` state). */
let currentController: DuckSourcesController | null = null;

/** Voice ducking: a mic voice-activity source, an OS audio-session monitor source (the
 * `shadowcat audio-monitor` subcommand), and a push-to-duck key — three `DuckSource`s behind
 * the audio engine's `DuckController` contract. `KeySource`/`OsMonitorSource` are constructed
 * here (both need only `window`/`WebSocket`, running from `register()` onward regardless of
 * whether Settings is ever opened); the actual wiring to `ctx.audio.duck` happens in
 * `DuckingRuntime` (contributed into `shadowcat.surface:overlay`, always mounted for the
 * session) rather than here, because `register(ctx)`'s `ModuleContext` carries no `audio`
 * member — `AudioApi` is reachable only through the Svelte-side `AppContext` (`getAppContext()`),
 * mirroring every other `ctx.audio` consumer in this codebase (`AudioPanel`, `StatusBar`,
 * `PlaylistSheet`). */
export const ducking: Module = {
  manifest: {
    id: "ducking",
    version: "0.1.0",
    dependencies: {},
    requires: [SETTINGS_SECTION_CONTRACT, "shadowcat.surface:overlay"],
    provides: [],
  },
  register(ctx) {
    const prefs = readDuckingMirror(localStorage);
    const controller = new DuckSourcesController(prefs, consoleLogger());
    currentController = controller;

    ctx.contributions.contribute({
      id: "ducking:settings",
      contract: SETTINGS_SECTION_CONTRACT,
      component: DuckingSettings,
      props: { controller },
      settingsSection: { labelKey: "ducking.sectionTitle" },
    });
    ctx.contributions.contribute({
      id: "ducking:runtime",
      contract: "shadowcat.surface:overlay",
      component: DuckingRuntime,
      props: { controller },
    });
  },
  unregister() {
    currentController?.dispose();
    currentController = null;
  },
};
