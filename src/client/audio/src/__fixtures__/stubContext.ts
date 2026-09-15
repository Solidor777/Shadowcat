import { vi } from "vitest";
import type {
  AudioBufferLike,
  AudioContextLike,
  BufferSourceNodeLike,
  GainNodeLike,
  MediaElementLike,
  MediaElementSourceNodeLike,
  PannerNodeLike,
  WasmOpusDecoderLike,
} from "../context";

/** A no-op `GainNodeLike` (unit `setTargetAtTime` writes through to `value` so a test can
 * read the last target back).
 * @returns The stub node.
 * @example
 * ```ts
 * import { stubAudioContext } from "../__fixtures__/stubContext";
 * stubAudioContext().createGain().gain.value; // 1
 * ```
 */
function stubGain(): GainNodeLike {
  return {
    gain: {
      value: 1,
      setTargetAtTime(target: number): void {
        this.value = target;
      },
    },
    connect: () => {},
    disconnect: () => {},
  };
}

/** A no-op `PannerNodeLike` (same write-through param shape as `stubGain`).
 * @returns The stub node.
 * @example
 * ```ts
 * import { stubAudioContext } from "../__fixtures__/stubContext";
 * stubAudioContext().createStereoPanner().pan.value; // 0
 * ```
 */
function stubPanner(): PannerNodeLike {
  return {
    pan: {
      value: 0,
      setTargetAtTime(target: number): void {
        this.value = target;
      },
    },
    connect: () => {},
    disconnect: () => {},
  };
}

/** A minimal `AudioBufferLike` for the stub context: carries the duration/sampleRate the
 * buffered-loop math reads; `copyToChannel` is a no-op (nothing renders in Node).
 * @param durationSecs The buffer's duration, seconds.
 * @param channels The buffer's channel count (drives the LRU byte estimate).
 * @param sampleRate The buffer's sample rate, Hz.
 * @returns The stub buffer.
 * @example
 * ```ts
 * import { stubBuffer } from "../__fixtures__/stubContext";
 * stubBuffer(1).duration; // 1
 * ```
 */
export function stubBuffer(durationSecs: number, sampleRate = 48_000, channels = 1): AudioBufferLike {
  return {
    duration: durationSecs,
    sampleRate,
    numberOfChannels: channels,
    /** No-op channel write (nothing renders in Node).
     * @example
     * ```
     * // stub member; nothing to call
     * ```
     */
    copyToChannel: () => {},
  };
}

/** `stubAudioContext`'s return shape: the stub context plus its captured nodes. */
export interface StubAudioContext extends AudioContextLike {
  /** Every buffer source the stub created, in creation order (spy targets). */
  sources: BufferSourceNodeLike[];
  /** Every gain node the stub created, in creation order (spy targets). */
  gains: GainNodeLike[];
  /** Every media-element source the stub created, in creation order (spy targets). */
  mediaSources: MediaElementSourceNodeLike[];
}

/** A fully in-memory `AudioContextLike` for Node-environment tests — no real Web Audio, no
 * jsdom. `decodeAudioData` returns a 1-second `stubBuffer` (never actually decoded);
 * `createBuffer` likewise. Every created buffer source is captured in `sources` so a test can
 * spy on `start`/`stop`/`loop`/`playbackRate`.
 * @returns The stub context plus its `sources` spy list.
 * @example
 * ```ts
 * import { stubAudioContext } from "../__fixtures__/stubContext";
 * stubAudioContext().state; // "suspended"
 * ```
 */
export function stubAudioContext(): StubAudioContext {
  /** The stub's current running state. */
  let state: "suspended" | "running" | "closed" = "suspended";
  const sources: BufferSourceNodeLike[] = [];
  const gains: GainNodeLike[] = [];
  const mediaSources: MediaElementSourceNodeLike[] = [];
  return {
    sources,
    gains,
    mediaSources,
    /** The stub's running state (flips to `"running"` on `resume`).
     * @returns The state.
     * @example
     * ```
     * // stub member; read by engine tests
     * ```
     */
    get state() {
      return state;
    },
    currentTime: 0,
    destination: { __destination: true },
    createGain: () => {
      const node = stubGain();
      gains.push(node);
      return node;
    },
    createStereoPanner: stubPanner,
    createBufferSource: (): BufferSourceNodeLike => {
      const source: BufferSourceNodeLike = {
        buffer: null,
        loop: false,
        loopStart: 0,
        loopEnd: 0,
        playbackRate: 1,
        connect: () => {},
        start: vi.fn(),
        stop: vi.fn(),
        onended: null,
      };
      sources.push(source);
      return source;
    },
    createBuffer: (channels: number, frames: number, sampleRate: number) =>
      stubBuffer(frames / sampleRate, sampleRate, channels),
    createMediaElementSource: (): MediaElementSourceNodeLike => {
      const source: MediaElementSourceNodeLike = {
        connect: () => {},
        disconnect: vi.fn(),
      };
      mediaSources.push(source);
      return source;
    },
    decodeAudioData: async () => stubBuffer(1),
    resume: async () => {
      state = "running";
    },
  };
}

/** A minimal `MediaElementLike` stub for `TrackPlayer` tests.
 * @returns The stub element (`canPlayType` answers `"probably"` to everything).
 * @example
 * ```ts
 * import { stubMediaElement } from "../__fixtures__/stubContext";
 * stubMediaElement().canPlayType("audio/ogg"); // "probably"
 * ```
 */
export function stubMediaElement(): MediaElementLike {
  return {
    src: "",
    currentTime: 0,
    playbackRate: 1,
    loop: false,
    play: async () => {},
    pause: () => {},
    onended: null,
    canPlayType: () => "probably",
  };
}

/** A minimal `WasmOpusDecoderLike` stub: reports one second of stereo PCM at 48 kHz.
 * @returns The stub decoder.
 * @example
 * ```ts
 * import { stubWasmDecoder } from "../__fixtures__/stubContext";
 * const pcm = await (await stubWasmDecoder()).decodeFile(new Uint8Array(0));
 * pcm.channelData.length; // 2
 * ```
 */
export function stubWasmDecoder(): WasmOpusDecoderLike {
  return {
    decodeFile: async () => ({
      channelData: [new Float32Array(48_000), new Float32Array(48_000)],
      sampleRate: 48_000,
    }),
    free: () => {},
  };
}

/** Bytes starting with the Ogg page magic, for the WASM-fallback path.
 * @returns Four OggS magic bytes plus padding.
 * @example
 * ```ts
 * import { oggBytes } from "../__fixtures__/stubContext";
 * new Uint8Array(oggBytes())[0]; // 0x4f ("O")
 * ```
 */
export function oggBytes(): ArrayBuffer {
  const bytes = new Uint8Array([0x4f, 0x67, 0x67, 0x53, 1, 2, 3, 4]);
  return bytes.buffer;
}

/** Bytes that are NOT Ogg, for the native-decode path.
 * @returns Four RIFF magic bytes plus padding.
 * @example
 * ```ts
 * import { wavBytes } from "../__fixtures__/stubContext";
 * new Uint8Array(wavBytes())[0]; // 0x52 ("R")
 * ```
 */
export function wavBytes(): ArrayBuffer {
  const bytes = new Uint8Array([0x52, 0x49, 0x46, 0x46, 1, 2, 3, 4]);
  return bytes.buffer;
}
