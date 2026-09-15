import type { WasmOpusDecoderLike } from "./context";

/** The `ogg-opus-decoder` package's own decoder class shape (only the members this module
 * uses — the package's `.d.ts` is the source of truth the typecheck validates against). */
interface OggOpusDecoderPackage {
  /** Resolves when the WASM binary is compiled and ready. */
  ready: Promise<unknown>;
  /** Decode a whole file.
   * @param data The file bytes.
   * @returns The decoded PCM plus metadata. */
  decodeFile(data: Uint8Array): Promise<{
    /** Per-channel PCM. */
    channelData: Float32Array[];
    /** Sample rate, Hz. */
    sampleRate: number;
    /** Total decoded frames. */
    samplesDecoded: number;
    /** Recoverable decode errors. */
    errors: unknown[];
  }>;
  /** Release the instance's WASM memory.
   * @returns Resolves when released. */
  free(): Promise<void>;
}

/** The memoized shared decoder promise (lives for the page's lifetime).
 * @example
 * ```
 * // private module state; exercised through `createOggOpusDecoder`
 * ```
 */
let cached: Promise<WasmOpusDecoderLike> | null = null;

/** The production `WasmOpusDecoderLike` factory: lazy-imports `ogg-opus-decoder` on FIRST use
 * (never at module load — the package carries a WASM binary whose fetch/compile must not run
 * on a device that never plays Ogg on WebKit), memoized so every later call shares the one
 * instance. The instance is never `free`d: it lives as long as the page, exactly like the
 * `AudioContext` it feeds.
 * @returns The shared decoder instance.
 * @example
 * ```ts
 * // production entry point — `AudioEngineOpts.createOggOpusDecoder` defaults to this
 * const decoder = await createOggOpusDecoder();
 * const pcm = await decoder.decodeFile(new Uint8Array([0x4f, 0x67, 0x67, 0x53]));
 * ```
 */
export function createOggOpusDecoder(): Promise<WasmOpusDecoderLike> {
  cached ??= (async () => {
    const pkg = (await import("ogg-opus-decoder")) as unknown as {
      /** The package's decoder class. */
      OggOpusDecoder: new () => OggOpusDecoderPackage;
    };
    const decoder = new pkg.OggOpusDecoder();
    await decoder.ready;
    return {
      /** Decode a whole Ogg/Opus file to per-channel PCM.
       * @param data The file bytes.
       * @returns The decoded PCM.
       * @example
       * ```
       * // wrapper member; forwards to the package decoder
       * ```
       */
      decodeFile: async (data: Uint8Array) => {
        const result = await decoder.decodeFile(data);
        return { channelData: result.channelData, sampleRate: result.sampleRate };
      },
      /** Release the decoder (never called — the shared instance lives with the page).
       * @returns Resolves when released.
       * @example
       * ```
       * // wrapper member; forwards to the package decoder
       * ```
       */
      free: () => decoder.free(),
    };
  })();
  return cached;
}
