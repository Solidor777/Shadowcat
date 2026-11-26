/** Parameters of a synthetic PCM16 WAV file. */
export interface SilentWavOptions {
  /** Length of the audio, in seconds; the sample count is rounded to a whole number. */
  seconds: number;
  /** Samples per second per channel. */
  sampleRate: number;
  /** Interleaved channel count. */
  channels: number;
}

/** Size in bytes of the canonical 44-byte WAV header (`RIFF`, `fmt ` and `data` chunk headers). */
const WAV_HEADER_BYTES = 44;

/** Bytes per sample of 16-bit PCM. */
const BYTES_PER_SAMPLE = 2;

/** A valid PCM16 WAV of silence, built in the Playwright process (the server is never asked to
 * generate it). The layout is the canonical RIFF header `symphonia`'s WAV reader and every
 * browser's media stack accept: a 16-byte `fmt ` chunk (PCM, format 1) followed by one `data`
 * chunk; `Buffer.alloc` zero-fills, which is the silence.
 * @param options - Length, sample rate and channel count.
 * @returns The file bytes.
 * @example
 * ```
 * const wav = buildSilentWav({ seconds: 0.2, sampleRate: 8000, channels: 1 });
 * ```
 */
export function buildSilentWav(options: SilentWavOptions): Buffer {
  const { seconds, sampleRate, channels } = options;
  const frames = Math.round(sampleRate * seconds);
  const blockAlign = channels * BYTES_PER_SAMPLE;
  const dataSize = frames * blockAlign;
  const buf = Buffer.alloc(WAV_HEADER_BYTES + dataSize);
  buf.write("RIFF", 0, "ascii");
  buf.writeUInt32LE(36 + dataSize, 4);
  buf.write("WAVE", 8, "ascii");
  buf.write("fmt ", 12, "ascii");
  buf.writeUInt32LE(16, 16);
  buf.writeUInt16LE(1, 20);
  buf.writeUInt16LE(channels, 22);
  buf.writeUInt32LE(sampleRate, 24);
  buf.writeUInt32LE(sampleRate * blockAlign, 28);
  buf.writeUInt16LE(blockAlign, 32);
  buf.writeUInt16LE(BYTES_PER_SAMPLE * 8, 34);
  buf.write("data", 36, "ascii");
  buf.writeUInt32LE(dataSize, 40);
  return buf;
}

/** The whole number of seconds of PCM16 audio whose WAV file fits in `maxBytes`.
 * @param maxBytes - The size ceiling, header included.
 * @param sampleRate - Samples per second per channel.
 * @param channels - Interleaved channel count.
 * @returns Whole seconds.
 * @example
 * ```
 * const seconds = wavSecondsForBytes(24 * 1024 * 1024, 48000, 2);
 * ```
 */
export function wavSecondsForBytes(maxBytes: number, sampleRate: number, channels: number): number {
  const bytesPerSecond = sampleRate * channels * BYTES_PER_SAMPLE;
  return Math.floor((maxBytes - WAV_HEADER_BYTES) / bytesPerSecond);
}
