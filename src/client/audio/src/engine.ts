import type {
  AssetResolver,
  AudioApi,
  AudioChannelId,
  AudioChannelPatch,
  AudioChannelState,
  AudioStateEngine,
  DuckController,
  OneShotOptions,
  PlayingTrack,
  SceneAudibility,
  WireAudioOp,
} from "@shadowcat/core";
import type { AudioContextLike, GainNodeLike, MediaElementLike, WasmOpusDecoderLike } from "./context";
import { DEFAULT_DUCK_DEPTH, DuckControllerImpl } from "./duck-controller";
import { EmitterPlayer } from "./emitter-player";
import { FallbackTrackPlayer } from "./fallback-player";
import { OneShotPlayer } from "./one-shot-player";
import { PlaybackGate } from "./playback-gate";
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

/** Period of the engine's synchronisation tick, ms: every tick re-applies the latest
 * `audio-state` so each player's position is re-measured against the server clock. */
export const SYNC_TICK_MS = 1000;

/** Longest the engine waits for a context to reach `"running"` after a `resume()` request,
 * ms. `resume()` can stay pending forever (WebKit before a gesture), so "not running after
 * this wait" is itself the signal that a user gesture is needed. */
export const RESUME_WAIT_MS = 2000;

/** The timer seam `AudioEngine` drives its synchronisation tick and its bounded resume wait
 * through (production passes the window's timers; tests drive a manual set). */
export interface AudioTimer {
  /** Schedule `cb` once after `ms` milliseconds.
   * @param cb The callback.
   * @param ms The delay, milliseconds.
   * @returns A handle for `clearTimeout`. */
  setTimeout(cb: () => void, ms: number): number;
  /** Cancel a schedule made by `setTimeout`.
   * @param handle The handle `setTimeout` returned. */
  clearTimeout(handle: number): void;
  /** Schedule `cb` every `ms` milliseconds.
   * @param cb The callback to run each period.
   * @param ms The period, milliseconds.
   * @returns A handle for `clearInterval`. */
  setInterval(cb: () => void, ms: number): number;
  /** Cancel a schedule made by `setInterval`.
   * @param handle The handle `setInterval` returned. */
  clearInterval(handle: number): void;
}

/** The page-visibility seam `AudioEngine` observes (production wraps `document.hidden` and the
 * `visibilitychange` event; the engine itself has no direct DOM access). */
export interface VisibilitySource {
  /** Whether the page is currently visible.
   * @returns `true` when visible. */
  isVisible(): boolean;
  /** Observe every visibility change.
   * @param cb Called after each change.
   * @returns An unsubscribe function. */
  subscribe(cb: () => void): () => void;
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
  /** The periodic source of the synchronisation tick (`SYNC_TICK_MS`). */
  timer: AudioTimer;
  /** The page-visibility source: a page returning to the foreground re-applies the latest
   * state immediately, because a background page's timers are throttled. */
  visibility: VisibilitySource;
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
  /** `subscribe` listeners, fired on every device-state mutation (`setChannel`, and paths
   * that mutate device state outside it via `notifyAudioChanged`). */
  #listeners = new Set<() => void>();
  /** The shared one-shot decode/LRU player (`null` until `unlock()`). */
  #oneShot: OneShotPlayer | null = null;
  /** Live track players, by `PlayingTrack.id`. */
  #trackPlayers = new Map<string, TrackPlayer>();
  /** Live carried-emitter players, by emitter token id (`applyAudibility`'s diff key). */
  #emitterPlayers = new Map<string, EmitterPlayer>();
  /** Pending audibility slice to apply once `unlock()` completes — same rationale as
   * `#latestState`. */
  #pendingAudibility: SceneAudibility | null = null;
  /** True once `unlock()` found no Web Audio API at all — every playback path then degrades
   * to bare `<audio>` elements (`FallbackTrackPlayer` / a fire-and-forget one-shot element). */
  #noWebAudio = false;
  /** Live degraded players (`#noWebAudio` mode only), by `PlayingTrack.id`. */
  #fallbackPlayers = new Map<string, FallbackTrackPlayer>();
  /** Live degraded carried emitters (`#noWebAudio` mode only), by emitter token id. */
  #fallbackEmitters = new Map<string, FallbackEmitter>();
  /** The latest `AudioStateEngine` every `applyState` call received — the one source the
   * `unlock()` replay (Web Audio node creation before a context exists is impossible), the
   * synchronisation tick and the visibility trigger all re-apply. */
  #latestState: AudioStateEngine | null = null;
  /** Counts `dispose()` calls. A resume wait captures it before it awaits and abandons its
   * continuation when it changed, so a wait that outlives `dispose()` installs and notifies
   * nothing. */
  #disposals = 0;
  /** Whether the device needs a user gesture before it will play: set when the browser refuses
   * a `play()` for want of one, read through `needsGesture`. */
  #needsGesture = false;
  /** The one place every media element this engine owns is started (`PlaybackGate.play`). */
  #playGate = new PlaybackGate(() => this.#setNeedsGesture(true));
  /** The synchronisation tick's timer handle, or `null` before it starts / after `dispose()`. */
  #tickHandle: number | null = null;
  /** Unsubscribes the visibility observer, or `null` before it is installed / after `dispose()`. */
  #visibilityOff: (() => void) | null = null;
  /** The `raf` handle for the running duck-gain driver loop, or `null` before `unlock()`/after
   * `dispose()`. */
  #duckLoopHandle: number | null = null;
  /** The last duck gain applied to `#duckNode` — the driver writes `setTargetAtTime` only when
   * `DuckControllerImpl.gain` differs from this, avoiding a redundant ramp restart every frame. */
  #lastAppliedDuckGain: number | null = null;
  /** The single in-flight preload (`preload`'s bound of at most one track), or `null`. The
   * element is never connected to the graph and never played — its ONLY purpose is to start
   * the browser fetching the resource ahead of an anticipated playlist advance. */
  #preload: {
    /** The asset being preloaded. */
    asset: string;
    /** The element fetching it (never played, never graph-connected). */
    el: MediaElementLike;
  } | null = null;

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

  /** Whether the device needs a user gesture before audio can play (a refused `play()`).
   * @returns `true` while a gesture is required. */
  get needsGesture(): boolean {
    return this.#needsGesture;
  }

  /** Record a `needsGesture` change and notify subscribers when it changed.
   * @param value The new state.
   * @example
   * ```
   * // private; exercised through the `AudioEngine interruption and suspension recovery` and
   * // `AudioEngine start-up through the gate` describe blocks
   * ```
   */
  #setNeedsGesture(value: boolean): void {
    if (this.#needsGesture === value) return;
    this.#needsGesture = value;
    this.notifyAudioChanged();
  }

  /** Per-channel device gain + mute state, keyed by `AudioChannelId` (all five buses).
   * @returns The live channel state record. */
  get channels(): Record<AudioChannelId, AudioChannelState> {
    return this.#channelState;
  }

  /** The shared engine `AudioContext` (thin accessor over `#context`).
   * @returns The real `AudioContext`, or `null` before `unlock()` resolves.
   * @example
   * ```ts
   * // implements `AudioApi.context` — see that interface's own doc
   * ```
   */
  context(): AudioContext | null {
    // `#context` is typed `AudioContextLike` (the injected abstraction every other member of
    // this class uses), but `AudioEngineOpts.createContext`'s production implementation
    // constructs a real DOM `AudioContext`, which structurally satisfies `AudioContextLike`
    // (see `AudioNodeLike`'s own doc for the same real-class-satisfies-the-`*Like`-interface
    // precedent). A consumer needing `AudioWorkletNode` construction (`MicVadSource`) needs the
    // real type the abstraction does not model; a test double is never routed through this
    // accessor by production code.
    return this.#context as unknown as AudioContext | null;
  }

  /** The shared ducking bus.
   * @returns The duck controller. */
  get duck(): DuckController {
    return this.#duckImpl;
  }

  /** Subscribe to device-state mutations (channel gain/mute, duck depth) — the shell's
   * reactivity bridge (`createSubscriber`) hooks here so `AppContext.audio` reads re-render
   * on change, the same pattern `DocumentStore.subscribe` provides for documents.
   * @param fn Called synchronously after each mutation.
   * @returns An unsubscribe function.
   * @example
   * ```ts
   * // consumed by the shell's `AppContext.audio` wrapper — exercised through
   * // `engine.test.ts`'s subscribe case
   * ```
   */
  subscribe(fn: () => void): () => void {
    this.#listeners.add(fn);
    return () => this.#listeners.delete(fn);
  }

  /** Notify `subscribe` listeners of a device-state change made outside `setChannel` (the
   * shell's duck-depth persist path mutates `DuckControllerImpl` directly).
   * @example
   * ```
   * // exercised through `engine.test.ts`'s subscribe case
   * ```
   */
  notifyAudioChanged(): void {
    for (const fn of this.#listeners) fn();
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
   * @example
   * ```ts
   * // implements `AudioApi.setChannel` — see that interface's own doc
   * ```
   */
  setChannel(id: AudioChannelId, patch: AudioChannelPatch): void {
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
    this.notifyAudioChanged();
  }

  /** Unlock the device's `AudioContext` — constructs the mixer graph on first call, resumes
   * the context inside the calling gesture, replays any pending transport state, and starts
   * the duck-gain driver loop. Idempotent. When the device has NO Web Audio API at all
   * (`createContext` throws), the engine degrades instead of rejecting: every playback path
   * falls back to bare `<audio>` elements (`FallbackTrackPlayer`, loops included via the
   * element's own `loop`), so streaming playback still works — just without the graph's
   * sample-accurate loops, per-bus gains, ducking, and crossfades.
   * @returns Resolves once the context is running, once the bounded resume wait
   * (`RESUME_WAIT_MS`) elapsed with `needsGesture` set, or once the degraded mode is armed.
   * @example
   * ```ts
   * // implements `AudioApi.unlock` — see that interface's own doc
   * ```
   */
  async unlock(): Promise<void> {
    // A user gesture lifts every earlier autoplay refusal: the elements are asked again.
    this.#playGate.release();
    const disposals = this.#disposals;
    if (this.#noWebAudio) {
      this.#setNeedsGesture(false);
      this.#restartDegraded();
      return;
    }
    if (this.#context !== null) {
      if (this.#context.state !== "running") await this.#resumeContext(this.#context);
      else this.#setNeedsGesture(false);
      if (disposals !== this.#disposals) return;
      if (this.#latestState) this.applyState(this.#latestState);
      return;
    }
    let context: AudioContextLike;
    try {
      context = this.#opts.createContext();
    } catch {
      // No Web Audio API at all: arm the degraded mode and replay any pending state through
      // it. RESOLVING (not rejecting) is the contract — streaming playback is available.
      this.#noWebAudio = true;
      this.#startSync();
      this.#restartDegraded();
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
    context.onstatechange = () => this.#onContextStateChange();
    await this.#resumeContext(context);
    if (disposals !== this.#disposals) return;
    if (this.#latestState) this.applyState(this.#latestState);
    if (this.#pendingAudibility) {
      this.applyAudibility(this.#pendingAudibility);
      this.#pendingAudibility = null;
    }
    this.#startDuckLoop();
    this.#startSync();
  }

  /** Ask `context` to resume and wait at most `RESUME_WAIT_MS` for it to run. The wait ends
   * on a settled `resume()` or on the timeout; afterwards `needsGesture` is `true` when the
   * request was rejected or the context is still not `"running"`, `false` otherwise. Never
   * loops: one request per call. A wait that finishes after `dispose()` changes nothing.
   * @param context The context to resume.
   * @example
   * ```
   * // private; exercised through `engine-recovery.test.ts`
   * ```
   */
  async #resumeContext(context: AudioContextLike): Promise<void> {
    const disposals = this.#disposals;
    let timeout: number | null = null;
    const elapsed = new Promise<void>((resolve) => {
      timeout = this.#opts.timer.setTimeout(resolve, RESUME_WAIT_MS);
    });
    try {
      await Promise.race([context.resume(), elapsed]);
    } catch {
      if (disposals === this.#disposals) this.#setNeedsGesture(true);
      return;
    } finally {
      if (timeout !== null) this.#opts.timer.clearTimeout(timeout);
    }
    if (disposals === this.#disposals) this.#setNeedsGesture(context.state !== "running");
  }

  /** Recovery from a context state change: a context that ran again re-applies the latest
   * state (and asks the gated elements again); a context that left `"running"` for anything
   * but `"closed"` (suspension, a WebKit interruption) is asked to resume once. No player is
   * created, started or stopped here — `applyState` is the only path that touches players.
   * @example
   * ```
   * // private; exercised through `engine-recovery.test.ts`
   * ```
   */
  #onContextStateChange(): void {
    const context = this.#context;
    if (!context) return;
    if (context.state === "running") {
      this.#setNeedsGesture(false);
      this.#playGate.release();
      if (this.#latestState) this.applyState(this.#latestState);
    } else if (context.state !== "closed") {
      void this.#resumeContext(context);
    }
  }

  /** The degraded (no Web Audio) restart after a gesture: re-apply the latest state and ask
   * every carried emitter's element to play again.
   * @example
   * ```
   * // private; exercised through `playback-gate.test.ts`
   * ```
   */
  #restartDegraded(): void {
    if (this.#latestState) this.applyState(this.#latestState);
    for (const entry of this.#fallbackEmitters.values()) this.#playGate.play(entry.el);
  }

  /** Start the synchronisation tick and the visibility trigger, once. Both run the same path:
   * `applyState` over `#latestState`, which is idempotent per entry id (an existing player is
   * only re-synced), so a trigger never starts a second player.
   * @example
   * ```
   * // private; exercised through `engine-sync.test.ts`
   * ```
   */
  #startSync(): void {
    if (this.#tickHandle !== null) return;
    const resync = (): void => {
      if (this.#latestState) this.applyState(this.#latestState);
    };
    this.#tickHandle = this.#opts.timer.setInterval(resync, SYNC_TICK_MS);
    this.#visibilityOff = this.#opts.visibility.subscribe(() => {
      if (!this.#opts.visibility.isVisible()) return;
      const context = this.#context;
      if (context && context.state !== "running" && context.state !== "closed") {
        void this.#resumeContext(context);
      }
      resync();
    });
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
        // `duck.depth` already notifies on every mutation; `duck.gain` only changes here, so a
        // reactive consumer needs this fired from the same only-on-real-change branch.
        this.notifyAudioChanged();
      }
      this.#duckLoopHandle = this.#opts.raf(frame);
    };
    this.#duckLoopHandle = this.#opts.raf(frame);
  }

  /** Play a one-shot sound effect; a no-op until `unlock()` has run (one-shots are never
   * queued — a missed UI cue is inconsequential).
   * @param asset Asset id of the sound to play.
   * @param opts Optional channel override and gain multiplier.
   * @example
   * ```ts
   * // implements `AudioApi.playOneShot` — see that interface's own doc
   * ```
   */
  playOneShot(asset: string, opts?: OneShotOptions): void {
    if (this.#noWebAudio) {
      // Degraded mode: a detached fire-and-forget element plays the cue straight to the
      // device (no graph, no LRU — nothing tracks the element after `play()`).
      const el = createMediaElement();
      el.src = pickStreamSrc(el, this.#opts.resolver.audioUrl(asset));
      el.volume = this.#degradedVolume(opts?.channel ?? "sfx", opts?.gain ?? 1);
      this.#playGate.play(el);
      return;
    }
    if (!this.#oneShot) return; // not yet unlocked: one-shots are never queued
    void this.#oneShot.play(asset, opts);
  }

  /** Diff `state.playing` by id against the live `TrackPlayer` set: create players for new
   * entries, `sync` existing ones, dispose removed ones. Called from the shell's `audio-state`
   * document-store subscription on every change, from the engine's own synchronisation tick
   * (`SYNC_TICK_MS`, over the latest state) and when the page becomes visible.
   * No-ops (tracks the state for replay) until `unlock()` has run.
   * @param state The world's current `AudioStateEngine`.
   * @example
   * ```
   * // exercised through `engine.test.ts`'s applyState diff cases
   * ```
   */
  applyState(state: AudioStateEngine): void {
    this.#latestState = state;
    this.#releasePreloadIfNowPlaying(state);
    if (this.#noWebAudio) {
      this.#applyStateFallback(state);
      return;
    }
    if (!this.#context) return;
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
          this.#playGate,
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
          this.#playGate,
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
      this.#playGate.play(entry.el);
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
    return channel.muted || master.muted ? 0 : Math.max(0, Math.min(1, gain * channel.gain * master.gain));
  }

  /** Begin prefetching `asset`'s stream ahead of an anticipated playlist advance (see
   * `resolveNextTrackAsset`), so the advance itself has no audible gap while the browser
   * fetches/decodes. Bounded to at most one preloaded track: a second call releases the first.
   * The preload element is never connected to the mixer graph and never played — `load()` (never
   * `play()`, which would make it audible) is the fetch trigger: assigning `src` alone queues a
   * source but does not itself start fetching, so a detached `<audio>` element with no other
   * live reference may never fetch at all on iOS Safari without an explicit `load()` call. A
   * no-op before `unlock()` (nothing to preload into before a device gesture) — unlike transport
   * state, a missed preload window is not queued, since the caller re-issues it on every
   * relevant `audio-state`/`playlist` change.
   * @param asset Asset id to prefetch.
   * @example
   * ```ts
   * // implements `AudioApi.preload` — see that interface's own doc
   * ```
   */
  preload(asset: string): void {
    if (!this.#context && !this.#noWebAudio) return;
    if (this.#preload?.asset === asset) return;
    this.#releasePreload();
    const el = createMediaElement();
    el.src = pickStreamSrc(el, this.#opts.resolver.audioUrl(asset));
    el.load(); // triggers the fetch; never play() — this element must stay inaudible
    this.#preload = { asset, el };
  }

  /** Release the in-flight preload, if any (idempotent).
   * @example
   * ```
   * // private helper; exercised through `engine.test.ts`'s preload cases
   * ```
   */
  #releasePreload(): void {
    if (this.#preload) {
      this.#preload.el.pause();
      this.#preload = null;
    }
  }

  /** Release the in-flight preload once its asset appears in a fresh `AudioStateEngine.playing`
   * set — a real `TrackPlayer`/`FallbackTrackPlayer` is about to stream that same asset, so the
   * preload element would otherwise sit forever as an unused duplicate fetch.
   * @param state The world's current `AudioStateEngine`.
   * @example
   * ```
   * // private helper; exercised through `engine.test.ts`'s preload cases
   * ```
   */
  #releasePreloadIfNowPlaying(state: AudioStateEngine): void {
    if (this.#preload && state.playing.some((entry) => entry.asset === this.#preload!.asset)) {
      this.#releasePreload();
    }
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
    this.#disposals++;
    for (const player of this.#trackPlayers.values()) player.dispose();
    this.#trackPlayers.clear();
    for (const player of this.#emitterPlayers.values()) player.dispose();
    this.#emitterPlayers.clear();
    for (const player of this.#fallbackPlayers.values()) player.dispose();
    this.#fallbackPlayers.clear();
    for (const entry of this.#fallbackEmitters.values()) entry.el.pause();
    this.#fallbackEmitters.clear();
    this.#releasePreload();
    if (this.#tickHandle !== null) {
      this.#opts.timer.clearInterval(this.#tickHandle);
      this.#tickHandle = null;
    }
    this.#visibilityOff?.();
    this.#visibilityOff = null;
    if (this.#context) this.#context.onstatechange = null;
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
