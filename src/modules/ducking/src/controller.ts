import type { Logger } from "@shadowcat/core";
import { KeySource, NULL_SINK, type DuckSink } from "./keySource";
import { OsMonitorSource } from "./osMonitor";
import type { DuckingPreferences } from "./duckingMirror";

/**
 * Owns the two sources that need no `AudioContext` (`KeySource`, `OsMonitorSource`) and
 * applies a `DuckingPreferences` snapshot to their running state. Constructed once in
 * `register(ctx)` and shared across every mount/unmount of the contributed settings section
 * — the sources must keep running while Settings is closed, since ducking is a background
 * effect, not a settings-panel-only feature. The mic source needs the engine's shared
 * `AudioContext`, so it is owned separately and wired only once `AudioApi.context()` exists.
 */
export class DuckSourcesController {
  /** The push-to-duck key source. */
  readonly key: KeySource;
  /** The OS audio-session monitor source. */
  readonly osMonitor: OsMonitorSource;

  /**
   * Constructs a controller owning the key and OS-monitor sources.
   * @param prefs The initial preferences snapshot (from `readDuckingMirror`).
   * @param logger Diagnostic sink, forwarded to `OsMonitorSource`.
   * @example
   * ```
   * const controller = new DuckSourcesController(({} as DuckingPreferences), { debug() {}, warn() {}, error() {} });
   * ```
   */
  constructor(prefs: DuckingPreferences, logger: Logger) {
    this.key = new KeySource(NULL_SINK, prefs.keyBinding);
    this.osMonitor = new OsMonitorSource({ port: prefs.osPort, watch: prefs.watchList, logger });
    this.osMonitor.setThreshold(prefs.osThreshold);
    this.applyEnablement(prefs);
  }

  /**
   * Applies a preferences snapshot's binding/threshold/watch-list/enablement fields to the
   * running sources. Called on every settings change, not just construction.
   * @param prefs The preferences snapshot to apply.
   * @example
   * ```
   * const controller = new DuckSourcesController(({} as DuckingPreferences), { debug() {}, warn() {}, error() {} });
   * controller.applyPreferences(({} as DuckingPreferences));
   * ```
   */
  applyPreferences(prefs: DuckingPreferences): void {
    this.key.setKey(prefs.keyBinding);
    this.osMonitor.setThreshold(prefs.osThreshold);
    this.osMonitor.setWatch(prefs.watchList);
    this.applyEnablement(prefs);
  }

  /**
   * Wires both sources' demand output to real `DuckSource` sinks (this plan's integration
   * task, once `ctx.audio.duck` exists).
   * @param keySink The sink for the key source's demand.
   * @param osSink The sink for the OS monitor source's demand.
   * @example
   * ```
   * const controller = new DuckSourcesController(({} as DuckingPreferences), { debug() {}, warn() {}, error() {} });
   * controller.wireToAudioDuck(NULL_SINK, NULL_SINK);
   * ```
   */
  wireToAudioDuck(keySink: DuckSink, osSink: DuckSink): void {
    this.key.setSink(keySink);
    this.osMonitor.setSink(osSink);
  }

  /**
   * Tears down both sources (called from the module's `unregister()`).
   * @example
   * ```
   * const controller = new DuckSourcesController(({} as DuckingPreferences), { debug() {}, warn() {}, error() {} });
   * controller.dispose();
   * ```
   */
  dispose(): void {
    this.key.stop();
    this.osMonitor.stop();
  }

  /**
   * Starts/stops each source per `masterEnabled` AND its own per-source flag.
   * @param prefs The preferences snapshot to derive enablement from.
   * @example
   * ```
   * // private method; not part of the public API — called from the constructor and applyPreferences
   * this.applyEnablement(({} as DuckingPreferences));
   * ```
   */
  private applyEnablement(prefs: DuckingPreferences): void {
    if (prefs.masterEnabled && prefs.keyEnabled) this.key.start();
    else this.key.stop();
    if (prefs.masterEnabled && prefs.osEnabled) this.osMonitor.start();
    else this.osMonitor.stop();
  }
}
