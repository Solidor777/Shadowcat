import type {
  AssetResolver,
  AudioChannelId,
  AudioChannelState,
  PlayingTrack,
} from "@shadowcat/core";
import type { MediaElementLike } from "./context";
import {
  createMediaElement,
  pickStreamSrc,
  SYNC_RATE_NUDGE,
  SYNC_SEEK_THRESHOLD_SECS,
} from "./track-player";

/** The no-Web-Audio degraded player: when `AudioEngine.unlock` finds no `AudioContext` at
 * all, every entry — loops included — plays through a BARE `<audio>` element (no mixer graph,
 * no decode, no LRU). Three deliberate degradations versus `TrackPlayer`, all accepted costs
 * of having no graph: (1) a looping entry sets the element's own `loop` — the seam is NOT
 * sample-accurate (a small hiccup per repeat); (2) channel/master bus gain and mute land on
 * the element's one `volume` knob, ducking included as a no-op; (3) crossfades are hard cuts.
 * Position/rate sync against the server-authoritative `PlayingTrack` mirrors `TrackPlayer`'s
 * streaming arm exactly. */
export class FallbackTrackPlayer {
  /** The bare element (the ONLY playback object — there is no graph). */
  #el: MediaElementLike;
  /** The entry's server channel (selects the bus half of the volume product). */
  #channel: AudioChannelId;
  /** This player's entry id (stable — an advance assigns a fresh id, which creates a NEW
   * player, so a player's own report can never name a stale id). */
  #entryId: string;
  /** The latest entry gain (the per-track half of the volume product). */
  #entryGain: number;
  /** Live read of the engine's channel-state record (volume recomputes against it). */
  #channelState: (id: AudioChannelId) => AudioChannelState;
  /** The client-observed track-end reporter (`AudioEngineOpts.onTrackEnded`). */
  #onTrackEnded: (id: string) => void;
  /** Set on `dispose()`; the end handler checks it before reporting. */
  #disposed = false;

  /** Construct a degraded player for `entry`: picks the stream source by the element's own
   * `canPlayType`, sets the element-loop for a looping entry, and installs the natural-end
   * reporter (which a looping element never fires — element loop repeats in place).
   * @param resolver Resolves asset ids to playback URL sets.
   * @param channelState Live read of the engine's per-channel gain/mute record.
   * @param entry The server-authoritative entry to play.
   * @param onTrackEnded The client-observed track-end reporter.
   * @example
   * ```
   * // constructed by `AudioEngine.applyState`'s degraded arm — exercised through
   * // `fallback-player.test.ts`
   * ```
   */
  constructor(
    resolver: AssetResolver,
    channelState: (id: AudioChannelId) => AudioChannelState,
    entry: PlayingTrack,
    onTrackEnded: (id: string) => void,
  ) {
    this.#channelState = channelState;
    this.#channel = entry.channel;
    this.#entryId = entry.id;
    this.#entryGain = entry.gain;
    this.#onTrackEnded = onTrackEnded;
    this.#el = createMediaElement();
    this.#el.src = pickStreamSrc(this.#el, resolver.audioUrl(entry.asset));
    this.#el.loop = entry.loop;
    this.#el.onended = () => {
      if (!this.#disposed) this.#onTrackEnded(this.#entryId);
    };
    this.applyChannelGain();
  }

  /** Recompute the element's `volume` from the latest entry gain × channel bus × master bus
   * (any mute zeroes it), clamped to the element's 0..=1 range. Called on construction, on
   * every `sync`, and directly by `AudioEngine.setChannel` so a volume slider moves the sound
   * immediately rather than on the next sync tick.
   * @example
   * ```
   * // exercised through `fallback-player.test.ts`'s volume case
   * ```
   */
  applyChannelGain(): void {
    const channel = this.#channelState(this.#channel);
    const master = this.#channelState("master");
    this.#el.volume =
      channel.muted || master.muted ? 0 : Math.min(1, this.#entryGain * channel.gain * master.gain);
  }

  /** Reconcile playback position/rate against the server-authoritative entry — the same
   * pause/seek/nudge arithmetic `TrackPlayer`'s streaming arm runs, plus the volume refresh
   * (a degraded player has no gain node for `sync` to write). `serverNow` is the CALLER's
   * calibrated clock (`WsClient.serverNow()`), never `Date.now()` directly.
   * @param entry The current `PlayingTrack` state.
   * @param serverNow The calibrated server-clock ms at the moment of this call.
   * @example
   * ```
   * // exercised through `fallback-player.test.ts`'s sync case
   * ```
   */
  sync(entry: PlayingTrack, serverNow: number): void {
    this.#entryGain = entry.gain;
    this.applyChannelGain();
    // Clamped ≥ 0: a negatively-skewed clock calibration must never reach
    // `el.currentTime = negative` (it throws).
    const targetSecs = Math.max(
      0,
      entry.pausedAt != null
        ? (entry.pausedAt - entry.startedAt) / 1000
        : (serverNow - entry.startedAt) / 1000,
    );
    const el = this.#el;
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

  /** Stop playback and drop the end callback. Idempotent-ish: marks the player dead first, so
   * a late natural end reports nothing.
   * @example
   * ```
   * // exercised through `fallback-player.test.ts`'s dispose case
   * ```
   */
  dispose(): void {
    this.#disposed = true;
    this.#el.pause();
    this.#el.onended = null;
  }
}
