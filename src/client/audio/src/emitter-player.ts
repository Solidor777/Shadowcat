import type { AudibleEmitter } from "@shadowcat/core";
import type {
  AudioContextLike,
  BiquadFilterNodeLike,
  BufferSourceNodeLike,
  GainNodeLike,
  PannerNodeLike,
} from "./context";
import type { OneShotPlayer } from "./one-shot-player";

/** `AudioParam.setTargetAtTime`'s time constant for a live gain/pan update, seconds — smooths a
 * server-driven audibility push (arrives at most a few times per second) into a continuous
 * ramp rather than an audible step. */
export const AUDIBILITY_RAMP_TAU_SECS = 0.15;

/** Lowpass cutoff, Hz, of a muffled emitter. Attenuates speech and music clarity above it while
 * leaving bass audible, the usual cue for sound heard through a wall or door. */
export const MUFFLED_LOWPASS_HZ = 800;

/** One carried `SoundEmission`'s live playback: a looping (or one-shot, per the emission's own
 * `loop` flag) `AudioBufferSourceNode` through a per-emitter gain → stereo-pan pair into the
 * `"sfx"` channel (through a lowpass stage while the emitter is `muffled`). Carried emitters
 * are, by `AudioChannel::Sfx`'s own doc ("sound effects, one-shots, AND SPATIAL EMITTERS"), an
 * `sfx`-channel concern, never a fourth bus. One instance
 * per token id (`AudioEngine.applyAudibility`'s own diff key); restarted only when `asset`
 * changes, so a same-asset gain/pan update never restarts playback. Buffer decode is delegated
 * to the shared `OneShotPlayer.getBuffer` so a carried emitter and a one-shot draw from the SAME
 * LRU cache/eviction budget. */
export class EmitterPlayer {
  /** The mixer graph's `AudioContextLike`. */
  #context: AudioContextLike;
  /** The shared decode/LRU (the same cache one-shots draw from). */
  #oneShot: OneShotPlayer;
  /** The per-emitter gain node (the server's resolved `gain` lands here). */
  #gain: GainNodeLike;
  /** The per-emitter stereo panner (the server's resolved `pan` lands here, or 0 under the
   * local spatial override). */
  #panner: PannerNodeLike;
  /** The `"sfx"` channel gain node the panner (or the lowpass) feeds. */
  #dest: GainNodeLike;
  /** The lowpass stage; connected between the panner and `#dest` only while `#muffled`. */
  #lowpass: BiquadFilterNodeLike;
  /** Whether the lowpass stage is currently in the graph. Reconnection happens only when this
   * changes, so a steady frame never tears down a live connection (an audible click). */
  #muffled = false;
  /** The live source, once the first `sync` decode has landed and started it. */
  #source: BufferSourceNodeLike | null = null;
  /** The asset the live source plays (`null` before the first `sync`), the restart key. */
  #asset: string | null = null;
  /** Set on `dispose()`; the async decode continuation checks it before touching the graph
   * (mirrors `TrackPlayer`/`FallbackTrackPlayer`'s own `#disposed` guard). */
  #disposed = false;
  /** Bumped on every asset-changing `sync` call; a decode continuation only (re)starts the
   * source when its own token still matches on resolution — an earlier call's decode
   * resolving after a later one is a stale write and is dropped. Mirrors
   * `TrackPlayer#decodeStarted`/`#pending`'s latest-sync-wins discipline, generalized for a
   * player whose asset can change many times over its lifetime rather than exactly once. */
  #decodeToken = 0;

  /** Construct a player routed `gain → panner → dest` (the lowpass is created
   * unconnected; no source yet — the first `sync` decodes and starts it).
   * @param context The mixer graph's `AudioContextLike`.
   * @param oneShot The shared decode/LRU.
   * @param dest The `"sfx"` channel gain node to route through.
   * @example
   * ```
   * // constructed by `AudioEngine.applyAudibility` — exercised through this package's tests
   * ```
   */
  constructor(context: AudioContextLike, oneShot: OneShotPlayer, dest: GainNodeLike) {
    this.#context = context;
    this.#oneShot = oneShot;
    this.#gain = context.createGain();
    this.#panner = context.createStereoPanner();
    this.#dest = dest;
    this.#lowpass = context.createBiquadFilter();
    this.#lowpass.type = "lowpass";
    this.#lowpass.frequency.value = MUFFLED_LOWPASS_HZ;
    this.#gain.connect(this.#panner);
    this.#panner.connect(dest);
  }

  /** Apply a resolved `AudibleEmitter` frame: (re)decode and (re)start only when `asset`
   * changed; every call ramps gain/pan toward the frame's values. `spatialOverride === false`
   * forces pan to `0` regardless of the frame's own `pan` (a local device-performance opt-out —
   * see `AudioEngine.applyAudibility`'s own doc for why this affects panning only, never gain).
   * A looping emitter decodes with the loop preference (the Ogg derivative — the container
   * every measured engine trims sample-exactly), a one-shot with the native-first preference.
   * @param emitter The resolved emitter frame. A change of `muffled` inserts or removes the
   * lowpass between the panner and the channel.
   * @param spatialOverride Whether this device renders panning; `false` centers every emitter.
   * @returns Resolves once any (re)start has been issued and the ramps are set.
   * @example
   * ```
   * // exercised through `emitter-player.test.ts`'s ramp-vs-restart cases
   * ```
   */
  async sync(emitter: AudibleEmitter, spatialOverride: boolean): Promise<void> {
    if (this.#asset !== emitter.asset) {
      this.#asset = emitter.asset;
      const token = ++this.#decodeToken;
      const buffer = await this.#oneShot.getBuffer(emitter.asset, emitter.loop ? "loop" : "oneshot");
      // A later `sync` call's decode may have resolved first, or `dispose()` may have run
      // meanwhile — either way, a stale continuation must never touch the graph (the former
      // would reconnect a source over a newer one; the latter would leak a source onto an
      // already-disconnected gain).
      if (this.#disposed || token !== this.#decodeToken) return;
      this.#source?.stop();
      const source = this.#context.createBufferSource();
      source.buffer = buffer;
      source.loop = emitter.loop;
      source.connect(this.#gain);
      source.start();
      this.#source = source;
    }
    if (emitter.muffled !== this.#muffled) {
      this.#panner.disconnect();
      if (emitter.muffled) {
        this.#panner.connect(this.#lowpass);
        this.#lowpass.connect(this.#dest);
      } else {
        this.#lowpass.disconnect();
        this.#panner.connect(this.#dest);
      }
      this.#muffled = emitter.muffled;
    }
    const now = this.#context.currentTime;
    // Clamped: the server validates only finiteness (`0..=1` is a convention, not enforced),
    // and a negative value THROWS against a real `AudioParam`.
    const gain = Math.max(0, Math.min(1, emitter.gain));
    this.#gain.gain.setTargetAtTime(gain, now, AUDIBILITY_RAMP_TAU_SECS);
    this.#panner.pan.setTargetAtTime(spatialOverride ? emitter.pan : 0, now, AUDIBILITY_RAMP_TAU_SECS);
  }

  /** Stop and detach from the graph.
   * @example
   * ```
   * // exercised through `emitter-player.test.ts`'s dispose case
   * ```
   */
  dispose(): void {
    this.#disposed = true;
    this.#source?.stop();
    this.#gain.disconnect();
    this.#panner.disconnect();
    this.#lowpass.disconnect();
  }
}
