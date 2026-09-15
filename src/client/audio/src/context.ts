/** The subset of `AudioParam` the mixer graph needs. */
export interface AudioParamLike {
  /** The current value (direct writes for instant changes, e.g. mute). */
  value: number;
  /** Exponentially ramp toward `target` starting at `startTime` with time constant
   * `timeConstant` (the smoothing primitive every server-driven gain/pan update uses).
   * @param target The value to ramp toward.
   * @param startTime The context time the ramp starts at.
   * @param timeConstant The ramp's time constant, seconds. */
  setTargetAtTime(target: number, startTime: number, timeConstant: number): void;

}

/** The subset of `GainNode` the mixer graph needs. */
export interface GainNodeLike {
  /** The node's gain parameter. */
  readonly gain: AudioParamLike;
  /** Connect this node's output to `dest`.
   * @param dest The downstream node. */
  connect(dest: AudioNodeLike): void;
  /** Detach this node from the graph. */
  disconnect(): void;
}

/** The subset of `StereoPannerNode` a spatial emitter needs. */
export interface PannerNodeLike {
  /** The node's pan parameter (`-1` full left .. `1` full right). */
  readonly pan: AudioParamLike;
  /** Connect this node's output to `dest`.
   * @param dest The downstream node. */
  connect(dest: AudioNodeLike): void;
  /** Detach this node from the graph. */
  disconnect(): void;
}

/** The marker shape of `AudioContext.destination` (the graph's terminal node). */
export interface AudioDestinationLike {
  /** Brand distinguishing the destination from a real connectable node. */
  readonly __destination: true;
}

/** Anything a `GainNode`/`PannerNode`/`AudioContext.destination` can be — the connect-graph
 * leaf type every node's `connect` accepts. */
export type AudioNodeLike = GainNodeLike | PannerNodeLike | AudioDestinationLike;

/** The subset of `AudioBuffer` the players need: the decoded-PCM container a
 * `BufferSourceNode` plays. A real `AudioBuffer` satisfies this verbatim (`duration`/
 * `sampleRate`/`copyToChannel` are its own members); the WASM decode path constructs one
 * through `AudioContextLike.createBuffer` and fills it channel-by-channel. */
export interface AudioBufferLike {
  /** Buffer duration, seconds (drives gapless `loopEnd` and buffered-loop position math). */
  readonly duration: number;
  /** Sample rate, Hz. */
  readonly sampleRate: number;
  /** Write one channel's PCM (the WASM decoder returns per-channel `Float32Array`s).
   * @param data The channel's samples.
   * @param channel The channel index to write. */
  copyToChannel(data: Float32Array, channel: number): void;
}

/** The subset of `AudioBufferSourceNode` `OneShotPlayer`/`EmitterPlayer`/`TrackPlayer` need. */
export interface BufferSourceNodeLike {
  /** The decoded buffer to play. */
  buffer: AudioBufferLike | null;
  /** Whether playback loops (over `loopStart`..`loopEnd`). */
  loop: boolean;
  /** Loop region start, seconds — `0` for a full-buffer loop (the Opus derivative's decoder
   * already trims pre-skip, so the decoded buffer is gapless-aligned at its own edges). */
  loopStart: number;
  /** Loop region end, seconds — the buffer's own `duration` for a full-buffer loop. */
  loopEnd: number;
  /** Playback-rate multiplier (`TrackPlayer`'s small-drift nudge; `1` = unity). */
  playbackRate: number;
  /** Connect this node's output to `dest`.
   * @param dest The downstream node. */
  connect(dest: AudioNodeLike): void;
  /** Start playback at `when` (context time), from `offset` seconds into the buffer.
   * @param when The context time to start at (`undefined` = now).
   * @param offset The buffer position to start from, seconds. */
  start(when?: number, offset?: number): void;
  /** Stop playback at `when` (context time).
   * @param when The context time to stop at (`undefined` = now). */
  stop(when?: number): void;
  /** End-of-playback callback (natural end, never a `stop()`). */
  onended: (() => void) | null;
}

/** The subset of `HTMLMediaElement` (an `<audio>` element) `TrackPlayer` needs — streamed
 * playback for long music, never decoded to RAM. */
export interface MediaElementLike {
  /** The resource URL. */
  src: string;
  /** Playback position, seconds. */
  currentTime: number;
  /** Playback-rate multiplier (the small-drift nudge). */
  playbackRate: number;
  /** Whether the element loops (never set for a playlist track — looping is the buffered
   * mode's job; the server transport owns track repetition). */
  loop: boolean;
  /** Begin playback.
   * @returns Resolves when playback has started. */
  play(): Promise<void>;
  /** Pause playback in place. */
  pause(): void;
  /** The device's own support answer for a MIME string.
   * @param type The MIME string to test.
   * @returns `""`, `"maybe"`, or `"probably"`. */
  canPlayType(type: string): "" | "maybe" | "probably";
}

/** The subset of `MediaElementAudioSourceNode` `TrackPlayer` needs. */
export interface MediaElementSourceNodeLike {
  /** Connect this node's output to `dest`.
   * @param dest The downstream node. */
  connect(dest: AudioNodeLike): void;
  /** Detach this node from the graph. */
  disconnect(): void;
}

/** The WASM Ogg/Opus decoder's result for one whole file — the browser-independent half of
 * the WebKit Ogg path (`canPlayType("audio/ogg; codecs=opus") === ""` there). */
export interface WasmOpusDecodeResult {
  /** Per-channel PCM, normalized float. */
  channelData: Float32Array[];
  /** Sample rate of the decoded PCM, Hz. */
  sampleRate: number;
}

/** The subset of the `ogg-opus-decoder` package this package depends on, injected so tests
 * never touch a real WASM binary (production wires `wasm.ts`'s lazy factory). */
export interface WasmOpusDecoderLike {
  /** Decode a whole Ogg/Opus file to per-channel PCM.
   * @param data The file bytes.
   * @returns The decoded PCM. */
  decodeFile(data: Uint8Array): Promise<WasmOpusDecodeResult>;
  /** Release the decoder instance's WASM memory.
   * @returns Nothing, or a promise of nothing. */
  free(): void | Promise<void>;
}

/** The subset of `AudioContext` the mixer graph needs — injected via `AudioEngineOpts.context`
 * (production callers pass a real `AudioContext`; tests pass a hand-rolled stub satisfying
 * this interface). */
export interface AudioContextLike {
  /** The context's running state. */
  readonly state: "suspended" | "running" | "closed";
  /** The context's audio-thread clock, seconds. */
  readonly currentTime: number;
  /** The graph's terminal node. */
  readonly destination: AudioDestinationLike;
  /** Create a gain node.
   * @returns The new node. */
  createGain(): GainNodeLike;
  /** Create a stereo panner node.
   * @returns The new node. */
  createStereoPanner(): PannerNodeLike;
  /** Create a buffer source node.
   * @returns The new node. */
  createBufferSource(): BufferSourceNodeLike;
  /** Allocate a PCM buffer the WASM decode path fills channel-by-channel.
   * @param channels Channel count.
   * @param frames Frame count (samples per channel).
   * @param sampleRate Sample rate, Hz.
   * @returns The new buffer. */
  createBuffer(channels: number, frames: number, sampleRate: number): AudioBufferLike;
  /** Route a media element into the graph (streaming playback).
   * @param el The element to route.
   * @returns The new source node. */
  createMediaElementSource(el: MediaElementLike): MediaElementSourceNodeLike;
  /** Decode compressed bytes to a PCM buffer (native path; the WASM fallback handles Ogg
   * where this rejects).
   * @param data The encoded file bytes.
   * @returns The decoded buffer. */
  decodeAudioData(data: ArrayBuffer): Promise<AudioBufferLike>;
  /** Resume a suspended context (inside a user gesture).
   * @returns Resolves when running. */
  resume(): Promise<void>;
}
