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
   * @param dest The downstream node.
   * @returns The downstream node (DOM `AudioNode.connect`); ignored by every caller. */
  connect(dest: AudioNodeLike): unknown;
  /** Detach this node from the graph. */
  disconnect(): void;
}

/** The subset of `StereoPannerNode` a spatial emitter needs. */
export interface PannerNodeLike {
  /** The node's pan parameter (`-1` full left .. `1` full right). */
  readonly pan: AudioParamLike;
  /** Connect this node's output to `dest`.
   * @param dest The downstream node.
   * @returns The downstream node (DOM `AudioNode.connect`); ignored by every caller. */
  connect(dest: AudioNodeLike): unknown;
  /** Detach this node from the graph. */
  disconnect(): void;
}

/** The graph's terminal node: the real `AudioDestinationNode` in production, or a stub's
 * branded object in tests (the brand is what a spy distinguishes it by). */
export type AudioDestinationLike =
  | {
      /** Brand marking the stub destination. */
      readonly __destination?: true;
    }
  | AudioDestinationNode;

/** Anything a `GainNode`/`PannerNode`/`AudioContext.destination` can be — the connect-graph
 * leaf type every node's `connect` accepts. Includes the DOM `AudioNode` itself so the REAL
 * Web Audio classes satisfy the `*Like` interfaces structurally (their `connect` takes an
 * `AudioNode`; method-parameter bivariance then admits both the stubs and the real nodes). */
export type AudioNodeLike = GainNodeLike | PannerNodeLike | AudioDestinationLike | AudioNode;

/** The subset of `AudioBuffer` the players need: the decoded-PCM container a
 * `BufferSourceNode` plays. A real `AudioBuffer` satisfies this verbatim (`duration`/
 * `sampleRate`/`copyToChannel` are its own members); the WASM decode path constructs one
 * through `AudioContextLike.createBuffer` and fills it channel-by-channel. */
export interface AudioBufferLike {
  /** Buffer duration, seconds (drives gapless `loopEnd` and buffered-loop position math). */
  readonly duration: number;
  /** Sample rate, Hz. */
  readonly sampleRate: number;
  /** Channel count (drives the LRU cache's byte estimate: channels × frames × 4 for f32 PCM). */
  readonly numberOfChannels: number;
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
  /** Playback-rate parameter (`TrackPlayer`'s small-drift nudge writes `.value`; `1` =
   * unity). A parameter, not a plain number — the DOM's `AudioBufferSourceNode.playbackRate`
   * is an `AudioParam`. */
  readonly playbackRate: AudioParamLike;
  /** Connect this node's output to `dest`.
   * @param dest The downstream node.
   * @returns The downstream node (DOM `AudioNode.connect`); ignored by every caller. */
  connect(dest: AudioNodeLike): unknown;
  /** Disconnect every outgoing connection (DOM `AudioNode.disconnect`) — `OneShotPlayer.play`'s
   * `onended` handler calls this so a completed one-shot's source is not left permanently
   * connected into the graph. */
  disconnect(): void;
  /** Start playback at `when` (context time), from `offset` seconds into the buffer.
   * @param when The context time to start at (`undefined` = now).
   * @param offset The buffer position to start from, seconds. */
  start(when?: number, offset?: number): void;
  /** Stop playback at `when` (context time).
   * @param when The context time to stop at (`undefined` = now). */
  stop(when?: number): void;
  /** End-of-playback callback (natural end, never a `stop()`).
   * @param ev The DOM `Event` (unused by every current handler). */
  onended: ((ev: Event) => void) | null;
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
  /** Output volume, 0..=1 — only meaningful to `FallbackTrackPlayer`, whose bare element has
   * no mixer graph to route through (the channel/master buses land on this one knob). */
  volume: number;
  /** Whether the element loops. Graph-routed playback never sets this (looping is the
   * buffered mode's job — `TrackPlayer`'s sample-accurate full-buffer loop); the no-Web-Audio
   * `FallbackTrackPlayer` sets it for a looping entry, accepting the element's seam hiccup. */
  loop: boolean;
  /** Begin playback.
   * @returns Resolves when playback has started. */
  play(): Promise<void>;
  /** Pause playback in place. */
  pause(): void;
  /** Natural-end callback (playback reached the resource's end — never fired by `pause()`).
   * `TrackPlayer` uses it for the client-observed track-end report (`AudioOp::TrackEnded`).
   * The parameter is the DOM `Event` (typed so both the DOM handler signature and a bare
   * stub closure assign).
   * @param ev The DOM `Event` (unused by every current handler). */
  onended: ((ev: Event) => void) | null;
  /** The device's own support answer for a MIME string.
   * @param type The MIME string to test.
   * @returns `""`, `"maybe"`, or `"probably"`. */
  canPlayType(type: string): "" | "maybe" | "probably";
}

/** The subset of `MediaElementAudioSourceNode` `TrackPlayer` needs. */
export interface MediaElementSourceNodeLike {
  /** Connect this node's output to `dest`.
   * @param dest The downstream node.
   * @returns The downstream node (DOM `AudioNode.connect`); ignored by every caller. */
  connect(dest: AudioNodeLike): unknown;
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
  readonly state: "suspended" | "running" | "closed" | "interrupted";
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
