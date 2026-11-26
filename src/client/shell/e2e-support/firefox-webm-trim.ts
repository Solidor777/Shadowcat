/** Samples per Opus frame at 48 kHz (20 ms). */
const FRAME_SAMPLES = 960;

/** Samples per second per channel of the Opus derivatives. */
const SAMPLE_RATE = 48_000;

/** The `OpusHead` pre-skip and WebM `CodecDelay` in samples: the Opus encoder's lookahead at 48 kHz. */
export const OPUS_PRE_SKIP_SAMPLES = 312;

/** Microseconds per second, the unit of a Firefox `TimeUnit` once a block time is read. */
const MICROS_PER_SECOND = 1_000_000;

/** Nanoseconds per second, the unit of the WebM `DiscardPadding` element. */
const NANOS_PER_SECOND = 1_000_000_000;

/** Milliseconds per Opus frame, the spacing of the WebM block timestamps. */
const FRAME_MILLIS = 20;

/**
 * The `DiscardPadding` the server writes on the last WebM block, in nanoseconds: the samples the
 * last packet carries beyond the valid ones (`frames * 960 - pre_skip - valid`) rounded UP to a
 * whole nanosecond, which is `samples_to_ns` in the server's WebM muxer.
 * @param validSamples - The decoded sample count the derivative is meant to have.
 * @returns The padding in nanoseconds, 0 when the packets end exactly on the valid samples.
 * @example
 * ```
 * const ns = discardPaddingNanos(48_000);
 * ```
 */
export function discardPaddingNanos(validSamples: number): number {
  const frames = Math.ceil((OPUS_PRE_SKIP_SAMPLES + validSamples) / FRAME_SAMPLES);
  const padding = frames * FRAME_SAMPLES - OPUS_PRE_SKIP_SAMPLES - validSamples;
  return Math.ceil((padding * NANOS_PER_SECOND) / SAMPLE_RATE);
}

/**
 * The number of samples Firefox's `decodeAudioData` returns for the server's WebM derivative, minus
 * the valid sample count: 0 or -1.
 *
 * Firefox trims the last packet through a window quantised to microseconds. Source: Firefox's
 * `WebMDemuxer::GetNextPacket` (the last block's duration is the packet duration minus the
 * `DiscardPadding`, a `TimeUnit` in the 48 kHz base when the padding is a whole number of ticks and
 * otherwise `TimeUnit::FromSeconds`, which rounds), `AudioTrimmer::PrepareTrimmers` and
 * `HandleDecodedResult` (the trim end is the block time plus that duration, a microsecond
 * `TimeUnit`, again through `FromSeconds`'s rounding unless the tick count is a multiple of 6) and
 * `AudioData::SetTrimWindow` (the kept frames are `ToTicksAtRate` of the window, which truncates).
 * One microsecond is 0.048 sample, so a trim end rounded DOWN to the microsecond loses one sample
 * on truncation. With `F` the audible samples of the last packet, the loss is certain for
 * `F % 6` of 4 or 5, absent for 0, 1 and 2, and decided by double-precision rounding of the sum for
 * 3, which depends on the block timestamp and is why this function evaluates the chain in doubles
 * for the file's own timestamp rather than reading a table. The Matroska definition of
 * `DiscardPadding` is met by the server's file; Chromium and Firefox's Ogg path decode the exact
 * count, so only the Firefox WebM cell calls this. Clean-room transcription of the arithmetic
 * above from reading, no Firefox code is copied.
 * @param validSamples - The decoded sample count the derivative is meant to have.
 * @returns 0 when Firefox decodes exactly `validSamples`, -1 when it decodes one sample fewer.
 * @example
 * ```
 * const delta = firefoxWebmDecodeDelta(50 * 960 + 959);
 * ```
 */
export function firefoxWebmDecodeDelta(validSamples: number): number {
  const frames = Math.ceil((OPUS_PRE_SKIP_SAMPLES + validSamples) / FRAME_SAMPLES);
  const paddingNanos = discardPaddingNanos(validSamples);
  if (paddingNanos === 0) return 0;
  const audibleTicks = lastPacketAudibleTicks(paddingNanos);
  const startMicros = (frames - 1) * FRAME_MILLIS * 1000;
  let endMicros: number;
  if ((audibleTicks * MICROS_PER_SECOND) % SAMPLE_RATE === 0) {
    endMicros = startMicros + (audibleTicks * MICROS_PER_SECOND) / SAMPLE_RATE;
  } else {
    endMicros = Math.round((startMicros / MICROS_PER_SECOND + audibleTicks / SAMPLE_RATE) * MICROS_PER_SECOND);
  }
  const kept = Math.trunc(((endMicros - startMicros) * SAMPLE_RATE) / MICROS_PER_SECOND);
  const audible = FRAME_SAMPLES - (frames * FRAME_SAMPLES - OPUS_PRE_SKIP_SAMPLES - validSamples);
  return kept - audible;
}

/**
 * The last packet's audible duration in 48 kHz ticks: the packet duration minus the padding.
 * @param paddingNanos - The `DiscardPadding` in nanoseconds, above 0.
 * @returns The audible ticks, exact when the padding is a whole number of ticks and rounded otherwise.
 * @example
 * ```
 * const ticks = lastPacketAudibleTicks(13_500_000);
 * ```
 */
function lastPacketAudibleTicks(paddingNanos: number): number {
  if ((paddingNanos * SAMPLE_RATE) % NANOS_PER_SECOND === 0) {
    return FRAME_SAMPLES - (paddingNanos * SAMPLE_RATE) / NANOS_PER_SECOND;
  }
  return Math.round((FRAME_SAMPLES / SAMPLE_RATE - paddingNanos / NANOS_PER_SECOND) * SAMPLE_RATE);
}
