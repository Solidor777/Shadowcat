import type {
  AssetResolver,
  AudioApi,
  AudioChannelId,
  AudioChannelState,
  AudioStateEngine,
  DuckController,
  PlayingTrack,
  SceneAudibility,
  WireAudioOp,
} from "@shadowcat/core";
import type { AudioContextLike, GainNodeLike, MediaElementLike, WasmOpusDecoderLike } from "./context";
import { DEFAULT_DUCK_DEPTH, DuckControllerImpl } from "./duck-controller";
import { EmitterPlayer } from "./emitter-player";
import { FallbackTrackPlayer } from "./fallback-player";
import { OneShotPlayer } from "./one-shot-player";
import { createMediaElement, pickStreamSrc, TrackPlayer } from "./track-player";
import { createOggOpusDecoder } from "./wasm";

/** A degraded-mode carried emitter's playback state (`AudioEngine.#fallbackEmitters`): the bare
 * element, the asset it currently plays (the restart key), and the server's resolved gain from
 * the latest audibility frame (recomposed with the live channel state on every `setChannel`). */
interface FallbackEmitter {
  /** The bare element (the ONLY playback object — there is no graph). */
  el: MediaElementLike;
  /** The asset the element currently plays. */
  asset: string;
  /** The server's resolved emitter gain from the latest audibility frame. */
  gain: number;
}

/** Channels every `AudioEngine` mixer graph carries (server-known three plus the two
 * client-only buses `AudioApi.channels` exposes device volume/mute for). */
const ALL_CHANNELS: AudioChannelId[] = ["master", "music", "ambience", "sfx", "ui"];
/** Channels the duck bus attenuates by default. */
const DEFAULT_DUCKABLE: AudioChannelId[] = ["music", "ambience"];

/** Constructor options for `AudioEngine`. */
export interface AudioEngineOpts {
  /** Resolves asset ids to serve URLs (the shell's shared `AssetResolver`). */
  resolver: AssetResolver;
  /** The calibrated server clock (`WsClient.serverNow()`), never `Date.now()` directly. */
  serverNow: () => number;
  /** Sends a GM transport op — a thin forwarder to `WsClient.audioTransport(op)`, injected so
   * this framework-neutral package never references `WsClient` directly. */
  transport: (op: WireAudioOp) => void;
  /** Lazily constructs the underlying `AudioContext`-shaped object, called exactly once, from
   * `unlock()` (Web Audio requires a user gesture before a context may run). */
  createContext: () => AudioContextLike;
  /** The WASM Ogg/Opus decoder factory behind the WebKit Ogg path — production defaults to
   * `wasm.ts`'s lazy singleton; tests inject a stub. */
  createOggOpusDecoder?: () => Promise<WasmOpusDecoderLike>;
  /** Called when a non-looping track reaches its natural end on this device — the
   * client-observed half of track-end advance (the server decides; see
   * `AudioOp::TrackEnded`). The shell wires this to `WsClient.audioTransport`. */
  onTrackEnded?: (id: string) => void;
  /** The crossfade duration for a replaced entry, ms, by source playlist id — the playlist
   * document's own `fadeMs`, looked up by the caller (this package never touches the
   * document store). `0`/absent = hard cut. */
  fadeMsFor?: (playlistId: string | null) => number;
  /** Sets (or clears) the connection's spatial-audio listening override — connection state the
   * shell forwards to `WsClient.audioListenAs`, injected for the same reason `transport` is
   * (this framework-neutral package never references `WsClient` directly). Optional: a host
   * with no listen-as surface leaves it unset and `AudioEngine.listenAs` is a no-op. */
  listenAs?: (token: string | null) => void;
  /** Local device override for spatial rendering — the performance-settings seam
   * (`ctx.performance.current.spatialAudio`). Defaults to always-on when omitted, so every
   * environment without that wiring renders full spatial panning unconditionally. */
  spatial?: () => boolean;
  /** Starting duck depth, `0..=1` — seeded from this device's persisted audio mirror by the
   * caller; defaults to `DEFAULT_DUCK_DEPTH` when the caller has no persisted value. */
  duckDepth?: number;
  /** Schedules `cb` for the next animation frame; injected so tests can pump frames manually
   * without a browser event loop (production passes `requestAnimationFrame`). */
  raf: (cb: (now: number) => void) => number;
  /** Cancels a frame scheduled by `raf` (production passes `cancelAnimationFrame`). */
  caf: (handle: number) => void;
}

/** The per-world mixer graph: `master ← duck ← {music, ambience, sfx, ui}` `GainNode`s, one
 * `AudioContext` created lazily on `unlock()`. Implements `AudioApi` — this IS the concrete
 * class the shell wraps in a reactive adapter for `AppContext.audio`. */
export class AudioEngine implements AudioApi {
  /** The constructor options. */
  #opts: AudioEngineOpts;
  /** The lazily-constructed context (`null` until `unlock()`). */
  #context: AudioContextLike | null = null;
  /** The graph's master gain. */
  #master: GainNodeLike | null = null;
  /** The duck bus's gain node. */
  #duckNode: GainNodeLike | null = null;
  /** Every bus's live gain node, by channel id. */
  #channelNodes = new Map<AudioChannelId, GainNodeLike>();
  /** Every bus's device gain/mute state, by channel id. */
  #channelState: Record<AudioChannelId, AudioChannelState>;
  /** The duck bus implementation. */
  #duckImpl: DuckControllerImpl;
  /** The shared one-shot decode/LRU player (`null` until `unlock()`). */
  #oneShot: OneShotPlayer | null = null;
  /** Live track players, by `PlayingTrack.id`. */
  #trackPlayers = new Map<string, TrackPlayer>();
  /** Live carried-emitter players, by emitter token id (`applyAudibility`'s diff key). */
  #emitterPlayers = new Map<string, EmitterPlayer>();
  /** Pending audibility slice to apply once `unlock()` completes — same rationale as
   * `#pendingState`. */
  #pendingAudibility: SceneAudibility | null = null;
  /** True once `unlock()` found no Web Audio API at all — every playback path then degrades
   * to bare `<audio>` elements (`FallbackTrackPlayer` / a fire-and-forget one-shot element). */
  #noWebAudio = false;
  /** Live degraded players (`#noWebAudio` mode only), by `PlayingTrack.id`. */
  #fallbackPlayers = new Map<string, FallbackTrackPlayer>();
  /** Live degraded carried emitters (`#noWebAudio` mode only), by emitter token id. */
  #fallbackEmitters = new Map<string, FallbackEmitter>();
  /** Pending state to apply once `unlock()` completes — Web Audio node creation before a
   * context exists is impossible, so a `PlayingTrack` set arriving before unlock is tracked
   * here and replayed by `unlock()`'s own tail. */
  #pendingState: AudioStateEngine | null = null;
  /** The `raf` handle for the running duck-gain driver loop, or `null` before `unlock()`/after
   * `dispose()`. */
  #duckLoopHandle: number | null = null;
  /** The last duck gain applied to `#duckNode` — the driver writes `setTargetAtTime` only when
   * `DuckControllerImpl.gain` differs from this, avoiding a redundant ramp restart every frame. */
  #lastAppliedDuckGain: number | null = null;

  /** Construct the engine (no graph yet — `unlock()` builds it on the first user gesture).
   * @param opts The engine's dependencies and device hooks.
   * @example
   * ```
   * // constructed by the shell's world session — exercised through `engine.test.ts`
   * ```
   */
  constructor(opts: AudioEngineOpts) {
    this.#opts = opts;
    this.#duckImpl = new DuckControllerImpl(opts.duckDepth ?? DEFAULT_DUCK_DEPTH);
    this.#channelState = Object.fromEntries(
      ALL_CHANNELS.map((c) => [c, { gain: 1, muted: false }]),
    ) as Record<AudioChannelId, AudioChannelState>;
  }

  /** Per-channel device gain + mute state, keyed by `AudioChannelId` (all five buses).
   * @returns The live channel state record. */
  get channels(): Record<AudioChannelId, AudioChannelState> {
    return this.#channelState;
  }

  /** The shared ducking bus.
   * @returns The duck controller. */
  get duck(): DuckController {
    return this.#duckImpl;
  }

  /** The calibrated server clock, ms (thin forwarder to `AudioEngineOpts.serverNow`).
   * @returns The current calibrated server time, ms.
   * @example
   * ```ts
   * // implements `AudioApi.serverNow` — see that interface's own doc
   * ```
   */
  serverNow(): number {
    return this.#opts.serverNow();
  }

  /** Send a GM-only transport op (thin forwarder to `AudioEngineOpts.transport`).
   * @param op The transport operation to apply.
   * @example
   * ```ts
   * // implements `AudioApi.transport` — see that interface's own doc
   * ```
   */
  transport(op: WireAudioOp): void {
    this.#opts.transport(op);
  }

  /** Set (or clear) the spatial-audio listening override (thin forwarder to
   * `AudioEngineOpts.listenAs`; a no-op when the host wired none).
   * @param token The token to listen as, or `null` to clear the override.
   * @example
   * ```ts
   * // implements `AudioApi.listenAs` — see that interface's own doc
   * ```
   */
  listenAs(token: string | null): void {
    this.#opts.listenAs?.(token);
  }

  /** Adjust one channel's device gain and/or mute state; omitted fields are unchanged. A live
   * channel node follows immediately (muted ⇒ gain 0).
   * @param id The channel to adjust.
   * @param patch The fields to change.
   * @param patch.gain The new gain, `0..=1`; omitted = unchanged.
   * @param patch.muted The new mute state; omitted = unchanged.
   * @example
   * ```ts
   * // implements `AudioApi.setChannel` — see that interface's own doc
   * ```
   */
  setChannel(id: AudioChannelId, patch: {
    /** The new gain, `0..=1`; omitted = unchanged. */
    gain?: number;
    /** The new mute state; omitted = unchanged. */
    muted?: boolean;
  }): void {
    const gain = patch.gain === undefined ? undefined : Math.max(0, Math.min(1, patch.gain));
    this.#channelState[id] = { ...this.#channelState[id], ...patch, ...(gain === undefined ? {} : { gain }) };
    const node = this.#channelNodes.get(id);
    if (node) {
      node.gain.value = this.#channelState[id].muted ? 0 : this.#channelState[id].gain;
    }
    if (this.#noWebAudio) {
      // Degraded players and emitters have no gain node — their element volume recomputes now,
      // not on the next sync tick.
      for (const player of this.#fallbackPlayers.values()) player.applyChannelGain();
      for (const entry of this.#fallbackEmitters.values()) {
        entry.el.volume = this.#degradedVolume("sfx", entry.gain);
      }
    }
  }

  /** Unlock the device's `AudioContext` — constructs the mixer graph on first call, resumes
   * the context inside the calling gesture, replays any pending transport state, and starts
   * the duck-gain driver loop. Idempotent. When the device has NO Web Audio API at all
   * (`createContext` throws), the engine degrades instead of rejecting: every playback path
   * falls back to bare `<audio>` elements (`FallbackTrackPlayer`, loops included via the
   * element's own `loop`), so streaming playback still works — just without the graph's
   * sample-accurate loops, per-bus gains, ducking, and crossfades.
   * @returns Resolves once the context is running (or the degraded mode is armed).
   * @example
   * ```ts
   * // implements `AudioApi.unlock` — see that interface's own doc
   * ```
   */
  async unlock(): Promise<void> {
    if (this.#noWebAudio) return;
    if (this.#context !== null) {
      if (this.#context.state !== "running") await this.#context.resume();
      return;
    }
    let context: AudioContextLike;
    try {
      context = this.#opts.createContext();
    } catch {
      // No Web Audio API at all: arm the degraded mode and replay any pending state through
      // it. RESOLVING (not rejecting) is the contract — streaming playback is available.
      this.#noWebAudio = true;
      if (this.#pendingState) {
        const pending = this.#pendingState;
        this.#pendingState = null;
        this.applyState(pending);
      }
      return;
    }
    this.#master = context.createGain();
    this.#master.connect(context.destination);
    this.#duckNode = context.createGain();
    this.#duckNode.connect(this.#master);
    for (const id of ALL_CHANNELS) {
      const node = context.createGain();
      node.gain.value = this.#channelState[id].muted ? 0 : this.#channelState[id].gain;
      // "master"/"ui" are client-only buses with no server-authored playback source — they
      // exist only so `setChannel` has a uniform target; only music/ambience/sfx ever receive
      // a TrackPlayer/OneShotPlayer/EmitterPlayer connection.
      node.connect(DEFAULT_DUCKABLE.includes(id) ? this.#duckNode : this.#master);
      this.#channelNodes.set(id, node);
    }
    this.#context = context;
    this.#oneShot = new OneShotPlayer(
      context,
      this.#opts.resolver,
      {
        master: this.#channelNodes.get("master")!,
        music: this.#channelNodes.get("music")!,
        ambience: this.#channelNodes.get("ambience")!,
        sfx: this.#channelNodes.get("sfx")!,
        ui: this.#channelNodes.get("ui")!,
      },
      this.#opts.createOggOpusDecoder ?? createOggOpusDecoder,
    );
    await context.resume();
    if (this.#pendingState) {
      this.applyState(this.#pendingState);
      this.#pendingState = null;
    }
    if (this.#pendingAudibility) {
      this.applyAudibility(this.#pendingAudibility);
      this.#pendingAudibility = null;
    }
    this.#startDuckLoop();
  }

  /** Drives `DuckControllerImpl.tick` off `#opts.raf` and applies its output to `#duckNode`.
   * Started once by `unlock()`; stopped by `dispose()`.
   * @example
   * ```
   * // private driver; exercised through `engine.test.ts`'s pumped-raf duck cases
   * ```
   */
  #startDuckLoop(): void {
    const frame = (now: number): void => {
      this.#duckImpl.tick(now);
      const gain = this.#duckImpl.gain;
      if (this.#duckNode && this.#context && gain !== this.#lastAppliedDuckGain) {
        this.#duckNode.gain.setTargetAtTime(gain, this.#context.currentTime, 0.02);
        this.#lastAppliedDuckGain = gain;
      }
      this.#duckLoopHandle = this.#opts.raf(frame);
    };
    this.#duckLoopHandle = this.#opts.raf(frame);
  }

  /** Play a one-shot sound effect; a no-op until `unlock()` has run (one-shots are never
   * queued — a missed UI cue is inconsequential).
   * @param asset Asset id of the sound to play.
   * @param opts Optional channel override and gain multiplier.
   * @param opts.channel The bus to play through; default `"sfx"`.
   * @param opts.gain Per-call gain multiplier; default `1`.
   * @example
   * ```ts
   * // implements `AudioApi.playOneShot` — see that interface's own doc
   * ```
   */
  playOneShot(asset: string, opts?: {
    /** The bus to play through; default `"sfx"`. */
    channel?: AudioChannelId;
    /** Per-call gain multiplier; default `1`. */
    gain?: number;
  }): void {
    if (this.#noWebAudio) {
      // Degraded mode: a detached fire-and-forget element plays the cue straight to the
      // device (no graph, no LRU — nothing tracks the element after `play()`).
      const el = createMediaElement();
      el.src = pickStreamSrc(el, this.#opts.resolver.audioUrl(asset));
      el.volume = this.#degradedVolume(opts?.channel ?? "sfx", opts?.gain ?? 1);
      void el.play();
      return;
    }
    if (!this.#oneShot) return; // not yet unlocked: one-shots are never queued
    void this.#oneShot.play(asset, opts);
  }

  /** Diff `state.playing` by id against the live `TrackPlayer` set: create players for new
   * entries, `sync` existing ones, dispose removed ones. Called from the shell's `audio-state`
   * document-store subscription on every change and on a 1 Hz drift-correction tick.
   * No-ops (tracks the state for replay) until `unlock()` has run.
   * @param state The world's current `AudioStateEngine`.
   * @example
   * ```
   * // exercised through `engine.test.ts`'s applyState diff cases
   * ```
   */
  applyState(state: AudioStateEngine): void {
    if (this.#noWebAudio) {
      this.#applyStateFallback(state);
      return;
    }
    if (!this.#context) {
      this.#pendingState = state;
      return;
    }
    const serverNow = this.#opts.serverNow();
    const seen = new Set<string>();
    for (const entry of state.playing) {
      seen.add(entry.id);
      const existing = this.#trackPlayers.get(entry.id);
      if (existing) {
        existing.sync(entry, serverNow);
      } else {
        const dest = this.#channelNodes.get(channelIdOf(entry))!;
        const player = new TrackPlayer(
          this.#context,
          this.#opts.resolver,
          this.#oneShot!,
          entry,
          dest,
          (id) => this.#opts.onTrackEnded?.(id),
        );
        player.sync(entry, serverNow);
        // Crossfade IN when this fresh entry replaces an outgoing one from the same playlist
        // (the server assigns a fresh id on advance — the pair is the playlist id).
        const outgoing = [...this.#trackPlayers.values()].find(
          (p) => p.playlistId !== null && p.playlistId === entry.playlist,
        );
        if (outgoing) {
          const fadeMs = this.#opts.fadeMsFor?.(entry.playlist) ?? 0;
          if (fadeMs > 0) player.fadeIn(fadeMs);
        }
        this.#trackPlayers.set(entry.id, player);
      }
    }
    for (const [id, player] of this.#trackPlayers) {
      if (!seen.has(id)) {
        // A replaced player fades out over the playlist's own `fadeMs`; anything else is a
        // hard cut (a stop, or a playlist with no crossfade authored).
        const replacement = state.playing.find(
          (e) => e.playlist !== null && e.playlist === player.playlistId,
        );
        const fadeMs = replacement ? (this.#opts.fadeMsFor?.(replacement.playlist) ?? 0) : 0;
        if (replacement && fadeMs > 0) {
          player.fadeOut(fadeMs);
        } else {
          player.dispose();
        }
        this.#trackPlayers.delete(id);
      }
    }
  }

  /** The degraded-mode `applyState` arm: the same create/sync/dispose id-diff the graph mode
   * runs, over `FallbackTrackPlayer`s. Crossfades are NOT paired here — a degraded replace is
   * a hard cut (one of the three accepted degradations `FallbackTrackPlayer`'s doc lists).
   * @param state The world's current `AudioStateEngine`.
   * @example
   * ```
   * // private arm; exercised through `engine.test.ts`'s degraded-mode cases
   * ```
   */
  #applyStateFallback(state: AudioStateEngine): void {
    const serverNow = this.#opts.serverNow();
    const seen = new Set<string>();
    for (const entry of state.playing) {
      seen.add(entry.id);
      const existing = this.#fallbackPlayers.get(entry.id);
      if (existing) {
        existing.sync(entry, serverNow);
      } else {
        const player = new FallbackTrackPlayer(
          this.#opts.resolver,
          (id) => this.#channelState[id],
          entry,
          (id) => this.#opts.onTrackEnded?.(id),
        );
        player.sync(entry, serverNow);
        this.#fallbackPlayers.set(entry.id, player);
      }
    }
    for (const [id, player] of this.#fallbackPlayers) {
      if (!seen.has(id)) {
        player.dispose();
        this.#fallbackPlayers.delete(id);
      }
    }
  }

  /** Diff `payload.emitters` by token id against the live `EmitterPlayer` set: create/sync/
   * dispose exactly like `applyState`'s own `TrackPlayer` diff. No-ops (tracks the payload for
   * replay) until `unlock()` has run. Takes ONE scene's already-filtered slice — the caller
   * picks it out of the full multi-scene `AudibilityPayload` via `sceneAudibility`.
   *
   * The LOCAL `spatial()` override (a device-performance opt-out, distinct from the SERVER's
   * own world-level `payload.spatial` overlay) affects panning ONLY, never gain: disabling
   * panning is a CPU/accessibility choice (mono mixdown), while `payload.gain` already folds in
   * the server's own distance falloff as a plain volume decision no client should second-guess
   * by trying to reconstruct an un-attenuated volume it was never sent.
   * @param payload The viewed scene's current `"audibility"` slice.
   * @example
   * ```
   * // exercised through `engine.test.ts`'s applyAudibility diff cases
   * ```
   */
  applyAudibility(payload: SceneAudibility): void {
    if (this.#noWebAudio) {
      this.#applyAudibilityFallback(payload);
      return;
    }
    if (!this.#context) {
      this.#pendingAudibility = payload;
      return;
    }
    const spatialOverride = payload.spatial && (this.#opts.spatial?.() ?? true);
    const seen = new Set<string>();
    for (const emitter of payload.emitters) {
      seen.add(emitter.token);
      let player = this.#emitterPlayers.get(emitter.token);
      if (!player) {
        player = new EmitterPlayer(this.#context, this.#oneShot!, this.#channelNodes.get("sfx")!);
        this.#emitterPlayers.set(emitter.token, player);
      }
      void player.sync(emitter, spatialOverride);
    }
    for (const [token, player] of this.#emitterPlayers) {
      if (!seen.has(token)) {
        player.dispose();
        this.#emitterPlayers.delete(token);
      }
    }
  }

  /** The degraded-mode `applyAudibility` arm: each emitter plays through a bare element —
   * `loop` from the emission (the element-seam hiccup, accepted), the server's resolved gain
   * composed with the live sfx/master buses on the element's `volume` (panning does not exist
   * without a graph, so there is nothing to center).
   * @param payload The viewed scene's current `"audibility"` slice.
   * @example
   * ```
   * // private arm; exercised through `engine.test.ts`'s degraded-mode cases
   * ```
   */
  #applyAudibilityFallback(payload: SceneAudibility): void {
    const seen = new Set<string>();
    for (const emitter of payload.emitters) {
      seen.add(emitter.token);
      let entry = this.#fallbackEmitters.get(emitter.token);
      if (!entry) {
        const el = createMediaElement();
        el.src = pickStreamSrc(el, this.#opts.resolver.audioUrl(emitter.asset));
        el.loop = emitter.loop;
        entry = { el, asset: emitter.asset, gain: emitter.gain };
        this.#fallbackEmitters.set(emitter.token, entry);
      } else if (entry.asset !== emitter.asset) {
        entry.el.src = pickStreamSrc(entry.el, this.#opts.resolver.audioUrl(emitter.asset));
        entry.el.loop = emitter.loop;
        entry.asset = emitter.asset;
      }
      entry.gain = emitter.gain;
      entry.el.volume = this.#degradedVolume("sfx", emitter.gain);
      void entry.el.play();
    }
    for (const [token, entry] of this.#fallbackEmitters) {
      if (!seen.has(token)) {
        entry.el.pause();
        this.#fallbackEmitters.delete(token);
      }
    }
  }

  /** The degraded-mode volume product: `gain` × channel bus × master bus, any mute zeroing it,
   * clamped to the element's 0..=1 range (shared by `FallbackTrackPlayer`'s one-shot path and
   * `#applyAudibilityFallback`).
   * @param id The channel bus to compose.
   * @param gain The per-source gain (entry gain, one-shot gain, or emitter gain).
   * @returns The element volume.
   * @example
   * ```
   * // private helper; exercised through `engine.test.ts`'s degraded-mode cases
   * ```
   */
  #degradedVolume(id: AudioChannelId, gain: number): number {
    const channel = this.#channelState[id];
    const master = this.#channelState.master;
    return channel.muted || master.muted ? 0 : Math.min(1, gain * channel.gain * master.gain);
  }

  /** Release every node, player, and the duck-loop `raf` handle; the `AudioContext` itself is
   * left to the shell (one `AudioEngine` per world session — the context's own lifecycle is the
   * shell's, not this class's, since `unlock()` may be called again on rejoin).
   * @example
   * ```
   * // exercised through `engine.test.ts`'s dispose cases
   * ```
   */
  dispose(): void {
    for (const player of this.#trackPlayers.values()) player.dispose();
    this.#trackPlayers.clear();
    for (const player of this.#emitterPlayers.values()) player.dispose();
    this.#emitterPlayers.clear();
    for (const player of this.#fallbackPlayers.values()) player.dispose();
    this.#fallbackPlayers.clear();
    for (const entry of this.#fallbackEmitters.values()) entry.el.pause();
    this.#fallbackEmitters.clear();
    if (this.#duckLoopHandle !== null) {
      this.#opts.caf(this.#duckLoopHandle);
      this.#duckLoopHandle = null;
    }
  }
}

/** Maps a `PlayingTrack`'s server channel (`"music" | "ambience" | "sfx"`) onto this engine's
 * `AudioChannelId` — a pure passthrough today (the sets already coincide on those three), kept
 * as a named function so a future divergence between the two vocabularies has one seam to
 * change.
 * @param entry The playing entry whose channel maps.
 * @returns The mixer bus id for the entry's channel.
 * @example
 * ```
 * // private helper; exercised through `AudioEngine.applyState`'s player construction
 * ```
 */
function channelIdOf(entry: PlayingTrack): AudioChannelId {
  return entry.channel;
}
