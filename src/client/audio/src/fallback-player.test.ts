// @vitest-environment node
import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import {
  AssetResolver,
  type AudioChannelId,
  type AudioChannelState,
  type PlayingTrack,
} from "@shadowcat/core";
import { FallbackTrackPlayer } from "./fallback-player";
import { setMediaElementFactory, SYNC_SEEK_THRESHOLD_SECS } from "./track-player";
import type { MediaElementLike } from "./context";
import { stubMediaElement } from "./__fixtures__/stubContext";

function entry(id: string, over: Partial<PlayingTrack> = {}): PlayingTrack {
  return {
    id,
    playlist: null,
    trackIndex: 0,
    asset: "a1",
    channel: "sfx",
    gain: 1,
    loop: false,
    startedAt: 0,
    pausedAt: null,
    ...over,
  };
}

/** A channel-state getter over a mutable record (the engine's live `#channelState` mirror). */
function channelStates(over: Partial<Record<AudioChannelId, Partial<AudioChannelState>>> = {}) {
  const record = Object.fromEntries(
    (["master", "music", "ambience", "sfx", "ui"] as AudioChannelId[]).map((id) => [
      id,
      { gain: 1, muted: false, ...over[id] },
    ]),
  ) as Record<AudioChannelId, AudioChannelState>;
  return {
    get: (id: AudioChannelId): AudioChannelState => record[id],
    record,
  };
}

let els: MediaElementLike[];
beforeEach(() => {
  els = [];
  setMediaElementFactory(() => {
    const el = stubMediaElement();
    els.push(el);
    return el;
  });
});
afterEach(() => vi.restoreAllMocks());

describe("FallbackTrackPlayer", () => {
  it("plays a LOOPING entry through the bare element with el.loop set (hiccup accepted)", () => {
    const channels = channelStates();
    const player = new FallbackTrackPlayer(new AssetResolver(), channels.get, entry("e1", { loop: true }), () => {});
    player.sync(entry("e1", { loop: true }), 1000);
    expect(els).toHaveLength(1);
    expect(els[0].loop).toBe(true);
    expect(els[0].src).toContain("variant=opus"); // the stub plays every type; Ogg wins the preference order
    player.dispose();
  });

  it("a non-looping entry streams with el.loop false and reports its natural end", () => {
    const ended: string[] = [];
    const player = new FallbackTrackPlayer(new AssetResolver(), channelStates().get, entry("e1"), (id) => ended.push(id));
    expect(els[0].loop).toBe(false);
    els[0].onended?.(new Event("ended"));
    expect(ended).toEqual(["e1"]);
    player.dispose();
  });

  it("volume combines the entry gain with the channel AND master buses, honoring mute", () => {
    const channels = channelStates({ sfx: { gain: 0.8 } });
    const player = new FallbackTrackPlayer(
      new AssetResolver(),
      channels.get,
      entry("e1", { gain: 0.5 }),
      () => {},
    );
    expect(els[0].volume).toBeCloseTo(0.4);
    channels.record.sfx = { gain: 0.8, muted: true };
    player.applyChannelGain();
    expect(els[0].volume).toBe(0);
    channels.record.sfx = { gain: 0.8, muted: false };
    channels.record.master = { gain: 1, muted: true };
    player.applyChannelGain();
    expect(els[0].volume).toBe(0);
    player.dispose();
  });

  it("clamps a negative server-sent gain to 0 rather than reaching el.volume as negative", () => {
    const channels = channelStates();
    const player = new FallbackTrackPlayer(
      new AssetResolver(),
      channels.get,
      entry("e1", { gain: -0.6 }),
      () => {},
    );
    expect(els[0].volume).toBeGreaterThanOrEqual(0);
    expect(els[0].volume).toBe(0);
    player.dispose();
  });

  it("sync pauses and resumes against the authoritative entry and hard-seeks past the drift threshold", () => {
    const player = new FallbackTrackPlayer(new AssetResolver(), channelStates().get, entry("e1"), () => {});
    const el = els[0];
    const play = vi.spyOn(el, "play");
    const pause = vi.spyOn(el, "pause");
    // Playing, no drift: a play() with unity rate.
    player.sync(entry("e1"), 1000);
    expect(play).toHaveBeenCalledTimes(1);
    expect(el.playbackRate).toBe(1);
    // Pause: pause() and no further play().
    player.sync(entry("e1", { pausedAt: 2000 }), 2000);
    expect(pause).toHaveBeenCalledTimes(1);
    expect(play).toHaveBeenCalledTimes(1);
    // Large drift while playing: a hard seek, not a rate nudge.
    player.sync(entry("e1", { startedAt: 10_000 }), 10_000 + SYNC_SEEK_THRESHOLD_SECS * 1000 + 1000);
    expect(el.currentTime).toBeCloseTo(SYNC_SEEK_THRESHOLD_SECS + 1);
    expect(el.playbackRate).toBe(1);
    player.dispose();
  });

  it("dispose pauses the element and drops the end callback (a late end reports nothing)", () => {
    const ended: string[] = [];
    const player = new FallbackTrackPlayer(new AssetResolver(), channelStates().get, entry("e1"), (id) => ended.push(id));
    const pause = vi.spyOn(els[0], "pause");
    player.dispose();
    expect(pause).toHaveBeenCalled();
    expect(els[0].onended).toBeNull();
    expect(ended).toEqual([]);
  });
});
