import type { AudioUrls } from "@shadowcat/core";
import type { AudioBufferLike, AudioContextLike, WasmOpusDecoderLike } from "./context";

/** Which buffered playback use a decode serves: the candidate order derives from it (a loop
 * prefers the Ogg derivative for gapless deterministic decode; a one-shot prefers the native
 * original container). Streaming playback (`TrackPlayer`'s `<audio>`-element mode) is NOT a
 * decode use — it selects through `canPlayType` instead. */
export type DecodePreference = "loop" | "oneshot";

/** The candidate URLs for `preference`, most-preferred first: a loop tries the `.opus.ogg`
 * derivative, then `.opus.webm`, then the original; a one-shot tries the original (native
 * container), then `.opus.webm` (native everywhere per the canPlayType measurement), then
 * `.opus.ogg` (the WASM fallback's input).
 * @param urls The asset's playback URL set (`AssetResolver.audioUrl`).
 * @param preference The buffered playback use.
 * @returns The candidate URLs in try order.
 * @example
 * ```ts
 * import { decodeCandidates } from "@shadowcat/audio";
 *
 * decodeCandidates({ ogg: "o", webm: "w", fallback: "f", oggType: "", webmType: "" }, "loop");
 * // ["o", "w", "f"]
 * ```
 */
export function decodeCandidates(urls: AudioUrls, preference: DecodePreference): string[] {
  return preference === "loop" ? [urls.ogg, urls.webm, urls.fallback] : [urls.fallback, urls.webm, urls.ogg];
}

/** Whether `bytes` starts with the Ogg page magic — the container sniff the WASM fallback keys
 * on (never a MIME label, the same posture the server's transcode probe takes).
 * @param bytes The candidate's fetched bytes.
 * @returns `true` when the leading four bytes are `OggS`.
 * @example
 * ```
 * // private helper; exercised through `decodeAudioCandidate`'s WASM-fallback tests
 * ```
 */
function isOgg(bytes: ArrayBuffer): boolean {
  const head = new Uint8Array(bytes, 0, 4);
  return head[0] === 0x4f && head[1] === 0x67 && head[2] === 0x67 && head[3] === 0x53;
}

/** Fetch `url`'s bytes, rejecting on any non-2xx (a derivative the import never emitted is a
 * 404, and the caller advances to the next candidate).
 * @param url The candidate URL to fetch.
 * @returns The response body.
 * @example
 * ```
 * // private helper; exercised through `decodeAudioCandidate`'s 404-advance tests
 * ```
 */
async function fetchBytes(url: string): Promise<ArrayBuffer> {
  const res = await fetch(url);
  if (!res.ok) throw new Error(`fetch ${url} failed: ${res.status}`);
  return res.arrayBuffer();
}

/** Decode an audio asset to a buffer, trying each candidate in `preference` order: fetch
 * failure advances to the next candidate; a native `decodeAudioData` failure on OGG bytes
 * falls back to the WASM opus decoder (the WebKit path — its `canPlayType` for Ogg/Opus is
 * empty) before advancing. This is the ONE decode seam every player (`TrackPlayer`'s buffered
 * loop, `OneShotPlayer`, `EmitterPlayer`) goes through, so the native/WASM decision can never
 * drift between call sites.
 * @param context The mixer graph's `AudioContextLike`.
 * @param urls The asset's playback URL set.
 * @param preference The buffered playback use (candidate order).
 * @param createWasmDecoder The injected WASM decoder factory (production: `wasm.ts`'s;
 * tests: a stub).
 * @returns The decoded buffer.
 * @example
 * ```ts
 * // private seam — exercised through `OneShotPlayer.getBuffer`/`TrackPlayer` instead
 * ```
 */
export async function decodeAudioCandidate(
  context: AudioContextLike,
  urls: AudioUrls,
  preference: DecodePreference,
  createWasmDecoder: () => Promise<WasmOpusDecoderLike>,
): Promise<AudioBufferLike> {
  let lastError: unknown;
  for (const url of decodeCandidates(urls, preference)) {
    let bytes: ArrayBuffer;
    try {
      bytes = await fetchBytes(url);
    } catch (e) {
      lastError = e;
      continue;
    }
    try {
      return await context.decodeAudioData(bytes);
    } catch (e) {
      lastError = e;
      if (!isOgg(bytes)) continue;
      try {
        const decoder = await createWasmDecoder();
        const pcm = await decoder.decodeFile(new Uint8Array(bytes));
        const frames = pcm.channelData[0]?.length ?? 0;
        if (pcm.channelData.length === 0 || frames === 0) {
          throw new Error("wasm decode produced no samples");
        }
        const buffer = context.createBuffer(pcm.channelData.length, frames, pcm.sampleRate);
        pcm.channelData.forEach((data, ch) => buffer.copyToChannel(data, ch));
        return buffer;
      } catch (wasmError) {
        lastError = wasmError;
      }
    }
  }
  throw lastError instanceof Error ? lastError : new Error(String(lastError));
}
