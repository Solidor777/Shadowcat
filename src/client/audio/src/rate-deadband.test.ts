// @vitest-environment node
import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { AssetResolver, type PlayingTrack } from "@shadowcat/core";
import type { AudioParamLike, GainNodeLike, MediaElementLike } from "./context";
import { FallbackTrackPlayer } from "./fallback-player";
import { OneShotPlayer } from "./one-shot-player";
import {
  rateForDrift,
  setMediaElementFactory,
  SYNC_RATE_ENGAGE_SECS,
  SYNC_RATE_NUDGE,
  SYNC_RATE_RELEASE_SECS,
  SYNC_SEEK_THRESHOLD_SECS,
  TrackPlayer,
} from "./track-player";
import { quietGate, stubAudioContext, stubBuffer, stubMediaElement, stubWasmDecoder, wavBytes } from "./__fixtures__/stubContext";

const JITTER_SECS = 0.005;
const TICKS = 60;

function entry(over: Partial<PlayingTrack> = {}): PlayingTrack {
  return {
    id: "e1",
    playlist: null,
    trackIndex: 0,
    asset: "a1",
    channel: "sfx",
    gain: 1,
    loop: false,
    startedAt: 0,
    pausedAt: null,
    sessionPaused: false,
    ...over,
  };
}

function dest(): GainNodeLike {
  return { gain: { value: 1, setTargetAtTime: () => {} }, connect: () => {}, disconnect: () => {} };
}

/** Replace the element's `playbackRate` with a recording accessor.
 * @returns Every value written, in order. */
function recordRateWrites(el: MediaElementLike): number[] {
  const writes: number[] = [];
  let rate = el.playbackRate;
  Object.defineProperty(el, "playbackRate", {
    get: () => rate,
    set: (v: number) => {
      rate = v;
      writes.push(v);
    },
  });
  return writes;
}

beforeEach(() => {
  setMediaElementFactory(() => stubMediaElement());
});
afterEach(() => vi.restoreAllMocks());

describe("rateForDrift", () => {
  it("keeps the hysteresis band strictly inside the seek threshold and the engage bound above the release bound", () => {
    expect(SYNC_RATE_RELEASE_SECS).toBeGreaterThan(0);
    expect(SYNC_RATE_ENGAGE_SECS).toBeGreaterThan(SYNC_RATE_RELEASE_SECS);
    expect(SYNC_RATE_ENGAGE_SECS).toBeLessThan(SYNC_SEEK_THRESHOLD_SECS);
    expect(JITTER_SECS).toBeLessThan(SYNC_RATE_RELEASE_SECS);
  });

  it("an idle player engages only above the engage bound, in the direction of the drift", () => {
    expect(rateForDrift(SYNC_RATE_ENGAGE_SECS, 1)).toBe(1);
    expect(rateForDrift(0.015, 1)).toBe(1);
    expect(rateForDrift(SYNC_RATE_ENGAGE_SECS + 0.001, 1)).toBe(1 + SYNC_RATE_NUDGE);
    expect(rateForDrift(-(SYNC_RATE_ENGAGE_SECS + 0.001), 1)).toBe(1 - SYNC_RATE_NUDGE);
  });

  it("an engaged player holds its nudge inside the band and releases below the release bound", () => {
    expect(rateForDrift(0.015, 1 + SYNC_RATE_NUDGE)).toBe(1 + SYNC_RATE_NUDGE);
    expect(rateForDrift(-0.015, 1 + SYNC_RATE_NUDGE)).toBe(1 - SYNC_RATE_NUDGE);
    expect(rateForDrift(SYNC_RATE_RELEASE_SECS - 0.001, 1 + SYNC_RATE_NUDGE)).toBe(1);
    expect(rateForDrift(0, 1 - SYNC_RATE_NUDGE)).toBe(1);
  });
});

describe("rate deadband per arm", () => {
  it("TrackPlayer streaming: jitter across many ticks writes no playbackRate; a real drift engages and converges back to 1", () => {
    const ctx = stubAudioContext();
    const el = stubMediaElement();
    setMediaElementFactory(() => el);
    const player = new TrackPlayer(ctx, new AssetResolver(), new OneShotPlayer(ctx, new AssetResolver(), { master: dest(), music: dest(), ambience: dest(), sfx: dest(), ui: dest() }, async () => stubWasmDecoder()), entry(), dest(), () => {}, quietGate());
    const writes = recordRateWrites(el);
    for (let i = 1; i <= TICKS; i++) {
      el.currentTime = i + (i % 2 === 0 ? JITTER_SECS : -JITTER_SECS);
      player.sync(entry(), i * 1000);
    }
    expect(writes).toEqual([]);

    // 60 ms behind: engage; after a tick that closes it to 5 ms: release.
    el.currentTime = 100 - 0.06;
    player.sync(entry(), 100_000);
    expect(writes).toEqual([1 + SYNC_RATE_NUDGE]);
    el.currentTime = 101 - 0.005;
    player.sync(entry(), 101_000);
    expect(writes).toEqual([1 + SYNC_RATE_NUDGE, 1]);
  });

  it("FallbackTrackPlayer: jitter across many ticks writes no playbackRate", () => {
    const el = stubMediaElement();
    setMediaElementFactory(() => el);
    const player = new FallbackTrackPlayer(
      new AssetResolver(),
      () => ({ gain: 1, muted: false }),
      entry(),
      () => {},
      quietGate(),
    );
    const writes = recordRateWrites(el);
    for (let i = 1; i <= TICKS; i++) {
      el.currentTime = i + (i % 2 === 0 ? JITTER_SECS : -JITTER_SECS);
      player.sync(entry(), i * 1000);
    }
    expect(writes).toEqual([]);
    el.currentTime = 100 - 0.06;
    player.sync(entry(), 100_000);
    expect(writes).toEqual([1 + SYNC_RATE_NUDGE]);
  });

  it("TrackPlayer buffered: jitter on the audio clock writes no source playbackRate", async () => {
    vi.spyOn(globalThis, "fetch").mockImplementation(async () => new Response(wavBytes()));
    const ctx = stubAudioContext();
    ctx.decodeAudioData = async () => stubBuffer(1000);
    const oneShot = new OneShotPlayer(ctx, new AssetResolver(), { master: dest(), music: dest(), ambience: dest(), sfx: dest(), ui: dest() }, async () => stubWasmDecoder());
    const e = entry({ loop: true });
    const player = new TrackPlayer(ctx, new AssetResolver(), oneShot, e, dest(), () => {}, quietGate());
    player.sync(e, 0);
    for (let i = 0; i < 5; i++) await new Promise((r) => setTimeout(r, 0));
    expect(ctx.sources).toHaveLength(1);
    const writes: number[] = [];
    let rate = 1;
    Object.defineProperty(ctx.sources[0], "playbackRate", {
      value: {
        get value() {
          return rate;
        },
        set value(v: number) {
          rate = v;
          writes.push(v);
        },
        setTargetAtTime: () => {},
      } satisfies AudioParamLike,
    });
    for (let i = 1; i <= TICKS; i++) {
      ctx.currentTime = i + (i % 2 === 0 ? JITTER_SECS : -JITTER_SECS);
      player.sync(e, i * 1000);
    }
    expect(writes).toEqual([]);
    expect(ctx.sources).toHaveLength(1);
    ctx.currentTime = 100 - 0.06;
    player.sync(e, 100_000);
    expect(writes).toEqual([1 + SYNC_RATE_NUDGE]);
    player.dispose();
  });
});
