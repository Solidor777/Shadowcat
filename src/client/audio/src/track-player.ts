import type { AssetResolver, AudioUrls, PlayingTrack } from "@shadowcat/core";
import type {
  AudioBufferLike,
  AudioContextLike,
  BufferSourceNodeLike,
  GainNodeLike,
  MediaElementLike,
  MediaElementSourceNodeLike,
} from "./context";
import type { OneShotPlayer } from "./one-shot-player";

/** Position drift beyond which `TrackPlayer.sync` performs a hard seek rather than a rate
 * nudge, seconds. */
export const SYNC_SEEK_THRESHOLD_SECS = 0.25;
/** Playback-rate nudge applied inside the seek threshold to close small drift inaudibly. */
export const SYNC_RATE_NUDGE = 0.02;

/** Pick the `<audio>`-element source for a streaming track: the first candidate the element
 * reports playable (Ogg derivative, then WebM, then the native original).
 * @param el The media element (its `canPlayType` is the device truth).
 * @param urls The asset's playback URL set.
 * @returns The URL to assign to `el.src`.
 * @example
 * ```ts
 * // private seam — exercised through `TrackPlayer`'s constructor
 * ```
 */
function pickStreamSrc(el: MediaElementLike, urls: AudioUrls): string {
  if (el.canPlayType(urls.oggType) !== "") return urls.ogg;
  if (el.canPlayType(urls.webmType) !== "") return urls.webm;
  return urls.fallback;
}

/** Position anchor for the buffered mode: the unwrapped buffer position at `serverNow`, plus
 * the rate it is advancing at. `positionAt` is pure arithmetic over the anchor — no reads of
 * the live source node, so a rate nudge mid-flight stays tracked exactly. */
interface PositionAnchor {
  /** Unwrapped position, seconds, at `serverNow`. */
  offset: number;
  /** The calibrated server-clock ms the anchor was taken at. */
  serverNow: number;
  /** Advance rate (the source's `playbackRate`). */
  rate: number;
}

/** Streams one playlist track, or — when the entry loops — plays it from a decoded buffer with
 * a sample-accurate full-buffer loop (the seamless-looping rule: `loop` is a playback-time
 * flag, so a looping track never streams; it decodes through `OneShotPlayer`'s shared LRU,
 * preferring the Ogg derivative). The streaming path routes an `<audio>` element through a
 * `MediaElementAudioSourceNode` (never decoded to RAM — appropriate for long music). Both
 * modes keep in sync with the server-authoritative `PlayingTrack.startedAt`/`pausedAt` via
 * `sync`, called on every `audio-state` change and on a periodic drift-correction tick. */
export class TrackPlayer {
  /** The mixer graph's `AudioContextLike`. */
  #context: AudioContextLike;
  /** The shared decode/LRU (buffered mode). */
  #oneShot: OneShotPlayer;
  /** The asset being played. */
  #asset: string;
  /** Whether this entry loops (selects buffered mode). */
  #loop: boolean;
  /** The per-track gain node (routes into the channel bus). */
  #gain: GainNodeLike;
  /** Streaming-mode element (null in buffered mode). */
  #el: MediaElementLike | null = null;
  /** Streaming-mode graph source (null in buffered mode). */
  #source: MediaElementSourceNodeLike | null = null;
  /** The live buffered source, when started (null in streaming mode and while paused). */
  #bufferSource: BufferSourceNodeLike | null = null;
  /** The decoded loop buffer (buffered mode, once decoded). */
  #buffer: AudioBufferLike | null = null;
  /** The buffered position anchor (buffered mode, while playing). */
  #anchor: PositionAnchor | null = null;
  /** The frozen unwrapped position while paused (buffered mode). */
  #frozen: number | null = null;
  /** The latest `sync` arguments, applied once the buffer finishes decoding. */
  #pending: {
    /** The stashed entry. */
    entry: PlayingTrack;
    /** The stashed calibrated server-clock ms. */
    serverNow: number;
  } | null = null;

  /** Construct a player for `entry`, choosing the mode from `entry.loop` (buffered gapless
   * loop vs streaming element).
   * @param context The mixer graph's `AudioContextLike`.
   * @param resolver Resolves asset ids to playback URL sets (streaming mode).
   * @param oneShot The shared decode/LRU (buffered mode).
   * @param entry The server-authoritative entry to play.
   * @param dest The channel gain node to route through.
   * @example
   * ```
   * // constructed by `AudioEngine.applyState` — exercised through this package's tests
   * ```
   */
  constructor(
    context: AudioContextLike,
    resolver: AssetResolver,
    oneShot: OneShotPlayer,
    entry: PlayingTrack,
    dest: GainNodeLike,
  ) {
    this.#context = context;
    this.#oneShot = oneShot;
    this.#asset = entry.asset;
    this.#loop = entry.loop;
    this.#gain = context.createGain();
    this.#gain.gain.value = entry.gain;
    this.#gain.connect(dest);
    if (!this.#loop) {
      this.#el = createMediaElement();
      const urls = resolver.audioUrl(entry.asset);
      this.#el.src = pickStreamSrc(this.#el, urls);
      this.#source = context.createMediaElementSource(this.#el);
      this.#source.connect(this.#gain);
    }
  }

  /** The buffered loop's current unwrapped position at `serverNow`, seconds.
   * @param serverNow The calibrated server-clock ms to evaluate at.
   * @returns The unwrapped position, seconds.
   * @example
   * ```
   * // private helper; exercised through the buffered-loop sync tests
   * ```
   */
  #positionAt(serverNow: number): number {
    const anchor = this.#anchor;
    if (!anchor) return 0;
    return anchor.offset + ((serverNow - anchor.serverNow) / 1000) * anchor.rate;
  }

  /** (Re)start the buffered source at unwrapped position `target`, rate `rate`.
   * @param target The unwrapped position to start from, seconds (mod the buffer duration).
   * @param rate The source's playback rate (`1` = unity).
   * @param serverNow The calibrated server-clock ms at this (re)start.
   * @example
   * ```
   * // private helper; exercised through the buffered-loop sync tests
   * ```
   */
  #startBufferedAt(target: number, rate: number, serverNow: number): void {
    const buffer = this.#buffer;
    if (!buffer) return;
    this.#bufferSource?.stop();
    const source = this.#context.createBufferSource();
    source.buffer = buffer;
    source.loop = true;
    source.loopStart = 0;
    source.loopEnd = buffer.duration;
    source.playbackRate = rate;
    source.connect(this.#gain);
    source.start(0, target % buffer.duration);
    this.#bufferSource = source;
    this.#anchor = { offset: target, serverNow, rate };
  }

  /** Decode the loop buffer on first use, then apply any stashed sync.
   * @example
   * ```
   * // private helper; exercised through the buffered-loop sync tests
   * ```
   */
  async #ensureBuffer(): Promise<void> {
    if (this.#buffer) return;
    this.#buffer = await this.#oneShot.getBuffer(this.#asset, "loop");
    const pending = this.#pending;
    this.#pending = null;
    if (pending) this.sync(pending.entry, pending.serverNow);
  }

  /** Reconcile playback position/rate against the server-authoritative entry. `serverNow` is
   * the CALLER's calibrated clock (`WsClient.serverNow()`), never `Date.now()` directly.
   * @param entry The current `PlayingTrack` state.
   * @param serverNow The calibrated server-clock ms at the moment of this call.
   * @example
   * ```
   * // exercised through `track-player.test.ts`'s streaming and buffered-loop sync cases
   * ```
   */
  sync(entry: PlayingTrack, serverNow: number): void {
    this.#gain.gain.value = entry.gain;
    const targetSecs =
      entry.pausedAt != null
        ? (entry.pausedAt - entry.startedAt) / 1000
        : (serverNow - entry.startedAt) / 1000;

    if (this.#loop) {
      if (!this.#buffer) {
        this.#pending = { entry, serverNow };
        void this.#ensureBuffer();
        return;
      }
      const playing = this.#bufferSource !== null;
      if (entry.pausedAt != null) {
        if (playing) {
          this.#frozen = this.#positionAt(serverNow);
          this.#bufferSource?.stop();
          this.#bufferSource = null;
          this.#anchor = null;
        }
        return;
      }
      if (!playing) {
        this.#startBufferedAt(this.#frozen ?? targetSecs, 1, serverNow);
        this.#frozen = null;
        return;
      }
      const drift = Math.abs(this.#positionAt(serverNow) - targetSecs);
      if (drift > SYNC_SEEK_THRESHOLD_SECS) {
        this.#startBufferedAt(targetSecs, 1, serverNow);
      } else if (drift > 0) {
        const behind = this.#positionAt(serverNow) < targetSecs;
        const rate = behind ? 1 + SYNC_RATE_NUDGE : 1 - SYNC_RATE_NUDGE;
        if (this.#bufferSource) this.#bufferSource.playbackRate = rate;
        this.#anchor = { offset: this.#positionAt(serverNow), serverNow, rate };
      } else if (this.#anchor?.rate !== 1) {
        if (this.#bufferSource) this.#bufferSource.playbackRate = 1;
        this.#anchor = { offset: this.#positionAt(serverNow), serverNow, rate: 1 };
      }
      return;
    }

    // Streaming mode.
    const el = this.#el;
    if (!el) return;
    if (entry.pausedAt != null) {
      el.pause();
    }
    const drift = Math.abs(el.currentTime - targetSecs);
    if (drift > SYNC_SEEK_THRESHOLD_SECS) {
      el.currentTime = targetSecs;
      el.playbackRate = 1;
    } else if (drift > 0) {
      el.playbackRate = el.currentTime < targetSecs ? 1 + SYNC_RATE_NUDGE : 1 - SYNC_RATE_NUDGE;
    } else {
      el.playbackRate = 1;
    }
    if (entry.pausedAt == null) {
      void el.play();
    }
  }

  /** Stop and detach from the graph.
   * @example
   * ```
   * // exercised through `track-player.test.ts`'s dispose calls
   * ```
   */
  dispose(): void {
    this.#el?.pause();
    this.#source?.disconnect();
    this.#bufferSource?.stop();
    this.#gain.disconnect();
  }
}

/** The installed element factory — throws until `setMediaElementFactory` runs (the shell
 * installs the real `<audio>` factory at app start; tests install `stubMediaElement`).
 * @example
 * ```
 * // private module state; exercised through `setMediaElementFactory` + `TrackPlayer`
 * ```
 */
let mediaElementFactory: () => MediaElementLike = () => {
  throw new Error("no MediaElementLike factory configured — call setMediaElementFactory first");
};

/** Production/shell entry point: install the real `<audio>`-element factory once, at app start.
 * @param factory Returns a fresh `MediaElementLike` each call.
 * @example
 * ```ts
 * import { setMediaElementFactory } from "@shadowcat/audio";
 * setMediaElementFactory(() => document.createElement("audio"));
 * ```
 */
export function setMediaElementFactory(factory: () => MediaElementLike): void {
  mediaElementFactory = factory;
}

/** Construct a fresh media element through the installed factory.
 * @returns The new element.
 * @example
 * ```
 * // private helper; exercised through `TrackPlayer`'s streaming-mode construction
 * ```
 */
function createMediaElement(): MediaElementLike {
  return mediaElementFactory();
}
