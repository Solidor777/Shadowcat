import type { Logger } from "@shadowcat/core";
import type { DuckSink } from "./keySource";
import { NULL_SINK } from "./keySource";


/** Rolling-window length the floor tracks: 2000ms of history at the configured frame size.
 * The floor is the EMA of the quietest 10% of samples in that window. */
const FLOOR_WINDOW_MS = 2000;
/** Fraction of the rolling window treated as "quiet" when estimating the noise floor. */
const QUIET_FRACTION = 0.1;
/** Consecutive above-threshold frames required to start speech. */
const ONSET_FRAMES = 3;
/** How long speech is held after the last above-threshold frame, in ms. */
const HANGOVER_MS = 400;
/** Smoothing weight applied to each window's quiet-average when updating the floor EMA. */
const FLOOR_EMA_ALPHA = 0.1;
/** Default sensitivity multiplier over the adaptive floor. */
export const DEFAULT_SENSITIVITY = 2.5;
/** Frame duration the engine is fed at, in ms. */
export const VAD_FRAME_MS = 20;

/** Constructor options for {@link VadEngine}. */
export interface VadEngineOptions {
  /** Multiplier applied to the adaptive noise floor to get the speech threshold; higher
   * values require louder speech relative to the floor before triggering (less sensitive). */
  sensitivity: number;
  /** The duration each `pushFrame` call represents, in ms; default `VAD_FRAME_MS`. */
  frameMs?: number;
}

/**
 * Energy-based voice-activity state machine, fed one frame's RMS amplitude at a time.
 * Clean-room design (no reference implementation): an adaptive noise floor tracks the
 * quietest 10% of a rolling 2s window (an EMA of that quiet-percentile average, so one loud
 * frame cannot itself raise the floor); `sensitivity` scales the floor into the speech
 * threshold; 3 consecutive frames above it start speech, held through a 400 ms hangover so
 * short gaps between syllables do not chatter the demand output.
 *
 * Runs INSIDE the AudioWorkletGlobalScope in production (`vad.worklet.ts`'s `VadProcessor`)
 * — the privacy invariant (audio never leaves the worklet; only a boolean per frame is
 * posted) is why this class exists standalone here rather than folded into the processor: it
 * is unit-testable in a plain node environment with synthetic RMS values, without any real
 * Web Audio/worklet environment.
 */
export class VadEngine {
  /** Speech threshold multiplier over the adaptive floor. */
  private readonly sensitivity: number;
  /** Frame duration in ms, used to size the floor window and hangover in frames. */
  private readonly frameMs: number;
  /** Rolling-window capacity in frames. */
  private readonly floorWindowCapacity: number;
  /** Hangover duration in frames. */
  private readonly hangoverFrameCount: number;
  /** Rolling window of recent RMS readings, oldest first, capped at `floorWindowCapacity`. */
  private readonly window: number[] = [];
  /** The adaptive noise floor: an EMA of each window's quietest-`QUIET_FRACTION` average. */
  private floor = 0;
  /** Consecutive frames at/above the current speech threshold. */
  private aboveCount = 0;
  /** Frames remaining in the hangover countdown; 0 means not currently in hangover. */
  private hangoverFrames = 0;
  /** Current speech state (the value `pushFrame` returns). */
  private speaking = false;

  /**
   * Constructs an engine sized for `opts.frameMs`-duration frames.
   * @param opts Sensitivity and frame-duration options.
   * @example
   * ```
   * const engine = new VadEngine({ sensitivity: 2.5 });
   * ```
   */
  constructor(opts: VadEngineOptions) {
    this.sensitivity = opts.sensitivity;
    this.frameMs = opts.frameMs ?? VAD_FRAME_MS;
    this.floorWindowCapacity = Math.max(1, Math.round(FLOOR_WINDOW_MS / this.frameMs));
    this.hangoverFrameCount = Math.max(1, Math.round(HANGOVER_MS / this.frameMs));
  }

  /**
   * Feeds one frame's RMS amplitude and returns the resulting demand.
   * @param rms The frame's root-mean-square amplitude (0..=1 for normalized audio).
   * @returns `1` while speech is active (including hangover), else `0`.
   * @example
   * ```
   * const engine = new VadEngine({ sensitivity: 2.5 });
   * const demand = engine.pushFrame(0.05);
   * ```
   */
  pushFrame(rms: number): 0 | 1 {
    this.updateFloor(rms);
    const threshold = this.floor * this.sensitivity;
    if (rms > threshold) {
      this.aboveCount += 1;
      if (this.aboveCount >= ONSET_FRAMES) {
        this.speaking = true;
        this.hangoverFrames = this.hangoverFrameCount;
      }
    } else {
      this.aboveCount = 0;
      if (this.hangoverFrames > 0) {
        this.hangoverFrames -= 1;
        if (this.hangoverFrames === 0) this.speaking = false;
      } else {
        this.speaking = false;
      }
    }
    return this.speaking ? 1 : 0;
  }

  /**
   * Appends `rms` to the rolling window (capped at `floorWindowCapacity`), then folds the
   * window's quietest `QUIET_FRACTION` average into the floor EMA.
   * @param rms The latest frame's RMS amplitude.
   * @example
   * ```
   * // private method; not part of the public API — called from pushFrame()
   * this.updateFloor(0.02);
   * ```
   */
  private updateFloor(rms: number): void {
    this.window.push(rms);
    if (this.window.length > this.floorWindowCapacity) this.window.shift();
    const sorted = [...this.window].sort((a, b) => a - b);
    const quietCount = Math.max(1, Math.round(sorted.length * QUIET_FRACTION));
    const quietAverage = sorted.slice(0, quietCount).reduce((sum, v) => sum + v, 0) / quietCount;
    this.floor = this.floor === 0 ? quietAverage : this.floor * (1 - FLOOR_EMA_ALPHA) + quietAverage * FLOOR_EMA_ALPHA;
  }
}

/** The registered `AudioWorkletProcessor` name `vad.worklet.ts` registers under. Declared
 * here (not in the worklet file) so `micVad.ts` can reference it without importing
 * `vad.worklet.ts` as a module (it is loaded via `audioWorklet.addModule`, a URL load, never
 * a normal ES import). */
export const VAD_PROCESSOR_NAME = "shadowcat-vad-processor";

/** Minimal shape of the pieces of `AudioContext`/`AudioWorkletNode`/`MediaStream` this
 * source calls — narrowed so unit tests can inject fakes without a real Web Audio
 * implementation (jsdom has none). */
export interface MicVadDeps {
  /** Requests microphone access; defaults to `navigator.mediaDevices.getUserMedia`. */
  getUserMedia: (constraints: MediaStreamConstraints) => Promise<MediaStream>;
  /** Loads the worklet module into `audioContext`; defaults to
   * `audioContext.audioWorklet.addModule`. */
  addWorkletModule: (audioContext: AudioContext, url: string) => Promise<void>;
  /** Constructs the worklet node; defaults to `new AudioWorkletNode(...)`. */
  createWorkletNode: (
    audioContext: AudioContext,
    name: string,
    options: AudioWorkletNodeOptions,
  ) => AudioWorkletNode;
  /** Constructs the media-stream source node; defaults to
   * `audioContext.createMediaStreamSource`. */
  createMediaStreamSource: (audioContext: AudioContext, stream: MediaStream) => MediaStreamAudioSourceNode;
}

/**
 * The real browser dependencies (production default).
 * @returns The production `MicVadDeps` backed by the real Web Audio/`navigator` APIs.
 * @example
 * ```
 * // private function; not part of the public API — the default for MicVadSourceOptions.deps
 * const deps = defaultDeps();
 * ```
 */
function defaultDeps(): MicVadDeps {
  return {
    getUserMedia: (constraints) => navigator.mediaDevices.getUserMedia(constraints),
    addWorkletModule: (ctx, url) => ctx.audioWorklet.addModule(url),
    createWorkletNode: (ctx, name, options) => new AudioWorkletNode(ctx, name, options),
    createMediaStreamSource: (ctx, stream) => ctx.createMediaStreamSource(stream),
  };
}

/** Constructor options for {@link MicVadSource}. */
export interface MicVadSourceOptions {
  /** The shared engine `AudioContext` (`AudioApi.context()`, wired once real integration is
   * complete — never a source-owned context). */
  audioContext: AudioContext;
  /** The worklet module's URL (`new URL("./vad.worklet.ts", import.meta.url)` in
   * production). */
  workletUrl: string | URL;
  /** Initial sensitivity multiplier; default `DEFAULT_SENSITIVITY`. */
  sensitivity?: number;
  /** Diagnostic sink. */
  logger: Logger;
  /** Injectable browser deps (tests supply fakes); defaults to `defaultDeps()`. */
  deps?: MicVadDeps;
}

/** Why `enable()` could not start listening. */
export type MicVadDenialReason = "permission-denied" | "no-microphone" | "unknown";

/**
 * Mic voice-activity `DuckSource`: on `enable()`, requests the microphone
 * (`echoCancellation`/`noiseSuppression` on), registers the shared `VAD_PROCESSOR_NAME`
 * worklet on the engine's `AudioContext`, and forwards each frame's boolean demand to the
 * sink. **Privacy invariant (ironclad — PII): the raw audio buffer never reaches this class**
 * — `vad.worklet.ts`'s processor posts only a boolean per frame; this class only ever reads
 * `MessageEvent.data: boolean` off the worklet's port.
 */
export class MicVadSource {
  /** The shared engine `AudioContext` the worklet/source nodes are constructed against. */
  private readonly audioContext: AudioContext;
  /** The worklet module's URL, loaded once (idempotently) on the first `enable()`. */
  private readonly workletUrl: string | URL;
  /** The sensitivity multiplier the NEXT `enable()` cycle's worklet is constructed with. */
  private sensitivity: number;
  /** Diagnostic sink. */
  private readonly logger: Logger;
  /** Injected browser dependencies (real or fake). */
  private readonly deps: MicVadDeps;
  /** The sink demand is forwarded to; replaceable via `setSink`. */
  private sink: DuckSink = NULL_SINK;
  /** The active microphone stream, or `null` while disabled. */
  private stream: MediaStream | null = null;
  /** The active `MediaStreamAudioSourceNode`, or `null` while disabled. */
  private sourceNode: MediaStreamAudioSourceNode | null = null;
  /** The active worklet node, or `null` while disabled. */
  private workletNode: AudioWorkletNode | null = null;
  /** Whether the worklet module has already been loaded onto `audioContext`. */
  private moduleLoaded = false;
  /** Whether the source is currently listening (the value `isEnabled()` returns). */
  private enabled = false;

  /**
   * Constructs a mic voice-activity source.
   * @param opts Construction options — shared audio context, worklet URL, and diagnostics.
   * @example
   * ```
   * const source = new MicVadSource({
   *   audioContext: {} as AudioContext,
   *   workletUrl: "vad.worklet.js",
   *   logger: { debug() {}, warn() {}, error() {} },
   * });
   * ```
   */
  constructor(opts: MicVadSourceOptions) {
    this.audioContext = opts.audioContext;
    this.workletUrl = opts.workletUrl;
    this.sensitivity = opts.sensitivity ?? DEFAULT_SENSITIVITY;
    this.logger = opts.logger;
    this.deps = opts.deps ?? defaultDeps();
  }

  /**
   * Replaces the sink demand is forwarded to (the integration task wires the real one).
   * @param sink The replacement sink.
   * @example
   * ```
   * const source = new MicVadSource({ audioContext: {} as AudioContext, workletUrl: "vad.worklet.js", logger: { debug() {}, warn() {}, error() {} } });
   * source.setSink(NULL_SINK);
   * ```
   */
  setSink(sink: DuckSink): void {
    this.sink = sink;
  }

  /**
   * Replaces the sensitivity multiplier for frames scored from now on.
   * @param sensitivity The replacement sensitivity multiplier.
   * @example
   * ```
   * const source = new MicVadSource({ audioContext: {} as AudioContext, workletUrl: "vad.worklet.js", logger: { debug() {}, warn() {}, error() {} } });
   * source.setSensitivity(3);
   * ```
   */
  setSensitivity(sensitivity: number): void {
    this.sensitivity = sensitivity;
    if (this.workletNode) {
      // The running processor's own VadEngine instance keeps its already-constructed
      // sensitivity — a live change takes effect on the NEXT `enable()` cycle, matching the
      // worklet's `processorOptions`-only construction seam (no live-parameter channel is
      // wired for this milestone's scope).
    }
  }

  /**
   * Requests the microphone and starts forwarding VAD demand. Resolves once listening has
   * started; resolves to a denial reason (never throws) on a permission refusal or missing
   * device: a denial shows the reason and leaves the source off.
   * @returns `null` on success, else the denial reason.
   * @example
   * ```
   * const source = new MicVadSource({ audioContext: {} as AudioContext, workletUrl: "vad.worklet.js", logger: { debug() {}, warn() {}, error() {} } });
   * const denial = await source.enable();
   * ```
   */
  async enable(): Promise<MicVadDenialReason | null> {
    if (this.enabled) return null;
    let stream: MediaStream;
    try {
      stream = await this.deps.getUserMedia({ audio: { echoCancellation: true, noiseSuppression: true } });
    } catch (e) {
      const name = e instanceof DOMException ? e.name : "";
      this.logger.warn("mic VAD getUserMedia failed", e);
      if (name === "NotAllowedError" || name === "SecurityError") return "permission-denied";
      if (name === "NotFoundError") return "no-microphone";
      return "unknown";
    }
    if (!this.moduleLoaded) {
      await this.deps.addWorkletModule(this.audioContext, String(this.workletUrl));
      this.moduleLoaded = true;
    }
    this.stream = stream;
    this.sourceNode = this.deps.createMediaStreamSource(this.audioContext, stream);
    const node = this.deps.createWorkletNode(this.audioContext, VAD_PROCESSOR_NAME, {
      processorOptions: { sensitivity: this.sensitivity },
    });
    node.port.onmessage = (event: MessageEvent<boolean>) => {
      this.sink.set(event.data ? 1 : 0);
    };
    this.sourceNode.connect(node);
    this.workletNode = node;
    this.enabled = true;
    return null;
  }

  /**
   * Stops listening, releases the microphone, and resets demand to 0.
   * @example
   * ```
   * const source = new MicVadSource({ audioContext: {} as AudioContext, workletUrl: "vad.worklet.js", logger: { debug() {}, warn() {}, error() {} } });
   * source.disable();
   * ```
   */
  disable(): void {
    if (!this.enabled) return;
    this.enabled = false;
    this.workletNode?.port.close();
    this.workletNode?.disconnect();
    this.sourceNode?.disconnect();
    this.stream?.getTracks().forEach((t) => t.stop());
    this.workletNode = null;
    this.sourceNode = null;
    this.stream = null;
    this.sink.set(0);
  }

  /**
   * Whether the source is currently listening.
   * @returns Whether `enable()` has completed without a subsequent `disable()`.
   * @example
   * ```
   * const source = new MicVadSource({ audioContext: {} as AudioContext, workletUrl: "vad.worklet.js", logger: { debug() {}, warn() {}, error() {} } });
   * const listening = source.isEnabled();
   * ```
   */
  isEnabled(): boolean {
    return this.enabled;
  }
}
