import type { AssetResolver, AudioChannelId } from "@shadowcat/core";
import type { AudioBufferLike, AudioContextLike, GainNodeLike, WasmOpusDecoderLike } from "./context";
import { decodeAudioCandidate, type DecodePreference } from "./decode";

/** Decoded-buffer cache budget, bytes (32 MiB). Tracked as the SUM of each cached buffer's
 * `byteLength` estimate (channels × frames × 4 bytes for f32 PCM), evicted least-recently-used
 * when a new decode would exceed it. */
export const ONE_SHOT_CACHE_BUDGET_BYTES = 32 * 1024 * 1024;

/** One cached decoded buffer with its byte estimate. */
interface CacheEntry {
  /** The decoded PCM. */
  buffer: AudioBufferLike;
  /** Estimated bytes (frames × 4 for f32 PCM), the LRU budget's unit. */
  bytes: number;
}

/** Decoded-buffer LRU + `AudioBufferSourceNode` playback for short sound effects
 * (`AudioApi.playOneShot`). Never streams — a one-shot is short enough to decode once and
 * replay from memory on every subsequent play. All decoding goes through `decode.ts`'s single
 * candidate-chain seam (`decodeAudioCandidate`), one-shots preferring the native original
 * container. */
export class OneShotPlayer {
  /** The mixer graph's `AudioContextLike`. */
  #context: AudioContextLike;
  /** Resolves asset ids to playback URL sets. */
  #resolver: AssetResolver;
  /** Every bus's live `GainNode`. */
  #channelGains: Record<AudioChannelId, GainNodeLike>;
  /** The injected WASM decoder factory. */
  #createWasmDecoder: () => Promise<WasmOpusDecoderLike>;
  /** The decoded-buffer LRU (insertion order = recency). */
  #cache = new Map<string, CacheEntry>();
  /** In-flight decodes by `preference:asset` — concurrent misses for the same asset share one
   * decode (and one cache-budget charge) instead of decoding twice. */
  #inFlight = new Map<string, Promise<AudioBufferLike>>();
  /** Sum of cached byte estimates. */
  #totalBytes = 0;

  /** Construct the player over an injected graph and decoder factory.
   * @param context The mixer graph's `AudioContextLike`.
   * @param resolver Resolves asset ids to playback URL sets.
   * @param channelGains Every bus's live `GainNode` (one-shots route through one per call).
   * @param createWasmDecoder The injected WASM decoder factory.
   * @example
   * ```
   * // constructed by `AudioEngine.unlock` — exercised through the engine's tests
   * ```
   */
  constructor(
    context: AudioContextLike,
    resolver: AssetResolver,
    channelGains: Record<AudioChannelId, GainNodeLike>,
    createWasmDecoder: () => Promise<WasmOpusDecoderLike>,
  ) {
    this.#context = context;
    this.#resolver = resolver;
    this.#channelGains = channelGains;
    this.#createWasmDecoder = createWasmDecoder;
  }

  /** Play `asset` once through `opts.channel` (default `"sfx"`) at `opts.gain` (default `1`,
   * multiplied against the channel's own gain via a per-call `GainNode`). Decodes and caches on
   * first play; a cache hit replays instantly.
   * @param asset Asset id to play.
   * @param opts Channel override and gain multiplier.
   * @param opts.channel The bus to play through; default `"sfx"`.
   * @param opts.gain Per-call gain multiplier; default `1`.
   * @example
   * ```
   * // exercised through `one-shot-player.test.ts`'s play/cache cases
   * ```
   */
  async play(asset: string, opts: {
    /** The bus to play through; default `"sfx"`. */
    channel?: AudioChannelId;
    /** Per-call gain multiplier; default `1`. */
    gain?: number;
  } = {}): Promise<void> {
    const channel = opts.channel ?? "sfx";
    const gain = opts.gain ?? 1;
    const buffer = await this.getBuffer(asset, "oneshot");
    const source = this.#context.createBufferSource();
    source.buffer = buffer;
    const perCallGain = this.#context.createGain();
    perCallGain.gain.value = gain;
    source.connect(perCallGain);
    perCallGain.connect(this.#channelGains[channel]);
    // Disconnect both per-call nodes once playback completes — otherwise each `play()` call
    // accumulates one permanently-connected `GainNode` in the mixer graph over a session.
    source.onended = () => {
      source.disconnect();
      perCallGain.disconnect();
    };
    source.start();
  }

  /** Get (decoding + caching on miss) the buffer for `asset` — shared by `play`, by
   * `EmitterPlayer`, and by `TrackPlayer`'s buffered-loop mode, so a looping carried-sound
   * emitter and a one-shot draw from the SAME LRU cache/eviction budget rather than each
   * keeping its own.
   * @param asset Asset id to decode.
   * @param preference Candidate order: `"loop"` (Ogg derivative first) for a looped emitter or
   * a looped playlist track, `"oneshot"` (native original first) for a one-shot.
   * @returns The decoded buffer (cache hit or fresh decode).
   * @example
   * ```
   * // exercised through `one-shot-player.test.ts`'s cache/LRU cases
   * ```
   */
  async getBuffer(asset: string, preference: DecodePreference = "oneshot"): Promise<AudioBufferLike> {
    if (this.#cache.has(asset)) return this.#touchAndGet(asset);
    const key = `${preference}:${asset}`;
    let pending = this.#inFlight.get(key);
    if (!pending) {
      pending = this.#decode(asset, preference).finally(() => this.#inFlight.delete(key));
      this.#inFlight.set(key, pending);
    }
    return pending;
  }

  /** Return the cached buffer for `asset`, refreshing its LRU position.
   * @param asset Asset id known to be cached (the caller checked).
   * @returns The cached buffer.
   * @example
   * ```
   * // private helper; exercised through the cache-hit path of `getBuffer`
   * ```
   */
  #touchAndGet(asset: string): AudioBufferLike {
    const entry = this.#cache.get(asset);
    if (!entry) throw new Error("unreachable: caller already checked cache.get");
    // Re-insert to mark as most-recently-used (Map iteration order is insertion order).
    this.#cache.delete(asset);
    this.#cache.set(asset, entry);
    return entry.buffer;
  }

  /** Decode `asset` through the shared candidate seam and insert it into the LRU.
   * @param asset Asset id to decode.
   * @param preference The decode's candidate order.
   * @returns The freshly decoded buffer.
   * @example
   * ```
   * // private helper; exercised through `getBuffer`'s miss path
   * ```
   */
  async #decode(asset: string, preference: DecodePreference): Promise<AudioBufferLike> {
    const urls = this.#resolver.audioUrl(asset);
    const buffer = await decodeAudioCandidate(this.#context, urls, preference, this.#createWasmDecoder);
    // channels × frames × 4 bytes (f32 PCM) — the budget doc's own accounting unit.
    const estimatedBytes = Math.round(buffer.duration * buffer.sampleRate) * buffer.numberOfChannels * 4;
    this.#evictUntilFits(estimatedBytes);
    this.#cache.set(asset, { buffer, bytes: estimatedBytes });
    this.#totalBytes += estimatedBytes;
    return buffer;
  }

  /** Evict least-recently-used entries until `incoming` more bytes fit the budget.
   * @param incoming The estimated byte size of the buffer about to be cached.
   * @example
   * ```
   * // private helper; exercised through the LRU eviction test
   * ```
   */
  #evictUntilFits(incoming: number): void {
    const it = this.#cache.keys();
    while (this.#totalBytes + incoming > ONE_SHOT_CACHE_BUDGET_BYTES && this.#cache.size > 0) {
      const oldest = it.next().value;
      if (oldest === undefined) break;
      const entry = this.#cache.get(oldest);
      if (entry) {
        this.#totalBytes -= entry.bytes;
        this.#cache.delete(oldest);
      }
    }
  }
}
