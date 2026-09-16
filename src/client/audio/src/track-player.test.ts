// @vitest-environment node
import { describe, it, expect, vi, afterEach, beforeEach } from "vitest";
import { AssetResolver, type PlayingTrack } from "@shadowcat/core";
import type { AudioParamLike, GainNodeLike } from "./context";
import { OneShotPlayer } from "./one-shot-player";
import { setMediaElementFactory, SYNC_SEEK_THRESHOLD_SECS, TrackPlayer } from "./track-player";
import { stubAudioContext, stubMediaElement, stubWasmDecoder, wavBytes } from "./__fixtures__/stubContext";

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
    ...over,
  };
}

function dest(): GainNodeLike {
  return { gain: { value: 1, setTargetAtTime: () => {} }, connect: () => {}, disconnect: () => {} };
}

function oneShotFor(ctx: ReturnType<typeof stubAudioContext>): OneShotPlayer {
  return new OneShotPlayer(
    ctx,
    new AssetResolver(),
    { master: dest(), music: dest(), ambience: dest(), sfx: dest(), ui: dest() },
    async () => stubWasmDecoder(),
  );
}

async function flush(): Promise<void> {
  for (let i = 0; i < 5; i++) await new Promise((r) => setTimeout(r, 0));
}

beforeEach(() => {
  setMediaElementFactory(() => stubMediaElement());
});
afterEach(() => vi.restoreAllMocks());

describe("TrackPlayer — streaming mode", () => {
  it("picks the ogg derivative when playable, webm when not, the original last", () => {
    const ctx = stubAudioContext();
    const el = stubMediaElement();
    el.canPlayType = () => "probably";
    setMediaElementFactory(() => el);
    new TrackPlayer(ctx, new AssetResolver(), oneShotFor(ctx), entry(), dest(), () => {});
    expect(el.src).toBe("/api/assets/a1?variant=opus");

    const el2 = stubMediaElement();
    el2.canPlayType = (t) => (t.includes("ogg") ? "" : "probably");
    setMediaElementFactory(() => el2);
    new TrackPlayer(ctx, new AssetResolver(), oneShotFor(ctx), entry(), dest(), () => {});
    expect(el2.src).toBe("/api/assets/a1?variant=opus-webm");

    const el3 = stubMediaElement();
    el3.canPlayType = () => "";
    setMediaElementFactory(() => el3);
    new TrackPlayer(ctx, new AssetResolver(), oneShotFor(ctx), entry(), dest(), () => {});
    expect(el3.src).toBe("/api/assets/a1");
  });

  it("seeks when drift exceeds the threshold, nudges inside it, and pauses on pausedAt", () => {
    const ctx = stubAudioContext();
    const el = stubMediaElement();
    setMediaElementFactory(() => el);
    const player = new TrackPlayer(ctx, new AssetResolver(), oneShotFor(ctx), entry(), dest(), () => {});

    el.currentTime = 0;
    player.sync(entry(), (SYNC_SEEK_THRESHOLD_SECS + 1) * 1000);
    expect(el.currentTime).toBeCloseTo(SYNC_SEEK_THRESHOLD_SECS + 1, 5);
    expect(el.playbackRate).toBe(1);

    el.currentTime = 10.05;
    player.sync(entry(), 10_000);
    expect(el.playbackRate).toBeLessThan(1);

    el.currentTime = 0;
    player.sync(entry({ pausedAt: 5_000 }), 20_000);
    expect(el.currentTime).toBeCloseTo(5, 5);
  });
});

describe("TrackPlayer — track-end report", () => {
  it("a natural element end reports the entry id once, and never after dispose", () => {
    const ctx = stubAudioContext();
    const el = stubMediaElement();
    setMediaElementFactory(() => el);
    const reports: string[] = [];
    const player = new TrackPlayer(
      ctx,
      new AssetResolver(),
      oneShotFor(ctx),
      entry({ id: "e-end" }),
      dest(),
      (id) => reports.push(id),
    );
    expect(el.onended).not.toBeNull();
    el.onended!(new Event("ended"));
    expect(reports).toEqual(["e-end"]);
    player.dispose();
    expect(el.onended).toBeNull();
  });
});

describe("TrackPlayer — buffered loop mode", () => {
  it("decodes the buffer, then starts a full-buffer loop at the position offset", async () => {
    vi.spyOn(globalThis, "fetch").mockImplementation(async () => new Response(wavBytes()));
    const ctx = stubAudioContext();
    const player = new TrackPlayer(ctx, new AssetResolver(), oneShotFor(ctx), entry({ loop: true }), dest(), () => {});
    player.sync(entry({ loop: true }), 1_500);
    await flush();
    const source = ctx.sources[0];
    expect(source.loop).toBe(true);
    expect(source.loopStart).toBe(0);
    expect(source.loopEnd).toBe(1); // the stub buffer's one-second duration
    // startedAt 0, serverNow 1500 ⇒ position 1.5 s into a 1 s buffer ⇒ offset 0.5.
    expect(source.start).toHaveBeenCalledWith(0, 0.5);
    player.dispose();
  });

  it("pause stops and freezes; resume restarts at the frozen offset", async () => {
    vi.spyOn(globalThis, "fetch").mockImplementation(async () => new Response(wavBytes()));
    const ctx = stubAudioContext();
    const e = entry({ loop: true });
    const player = new TrackPlayer(ctx, new AssetResolver(), oneShotFor(ctx), e, dest(), () => {});
    player.sync(e, 1_000);
    await flush();
    const first = ctx.sources[0];
    expect(first.start).toHaveBeenCalledWith(0, 0);

    player.sync(entry({ loop: true, pausedAt: 2_000 }), 2_000);
    expect(first.stop).toHaveBeenCalledTimes(1);

    player.sync(e, 3_000);
    const second = ctx.sources[1];
    // Frozen at position 2.0 (1.0 s of track time at start plus 1.0 s elapsed at pause) ⇒
    // restart offset 2.0 % 1 = 0.
    expect(second.start).toHaveBeenCalledWith(0, 0);
    player.dispose();
  });

  it("restarts at the corrected offset when drift exceeds the threshold", async () => {
    vi.spyOn(globalThis, "fetch").mockImplementation(async () => new Response(wavBytes()));
    const ctx = stubAudioContext();
    const e = entry({ loop: true, startedAt: 200 });
    const player = new TrackPlayer(ctx, new AssetResolver(), oneShotFor(ctx), e, dest(), () => {});
    player.sync(e, 1_000);
    await flush();
    const first = ctx.sources[0];
    // A server-side seek re-anchors the entry: startedAt -300 ⇒ target (3000 + 300)/1000 = 3.3
    // while the local player tracked only to 2.8 — drift 0.5 s forces a restart.
    player.sync(entry({ loop: true, startedAt: -300 }), 3_000);
    expect(first.stop).toHaveBeenCalledTimes(1);
    const second = ctx.sources[1];
    expect(second.start).toHaveBeenCalledWith(0, expect.closeTo(0.3, 10));
    player.dispose();
  });

  it("a mid-fade sync re-targets the fade's automation rather than snapping the gain directly", () => {
    const ctx = stubAudioContext();
    const e = entry({ id: "e-fade", gain: 0.8 });
    const player = new TrackPlayer(ctx, new AssetResolver(), oneShotFor(ctx), e, dest(), () => {});
    // Swap the player's gain param for a recording one: a direct `.value` write during a fade
    // is exactly the defect under test; an automation call is the correct re-target.
    const directWrites: number[] = [];
    const ramp = vi.fn();
    let current = 0.8;
    (ctx.gains[0] as { gain: AudioParamLike }).gain = {
      get value() {
        return current;
      },
      set value(v: number) {
        current = v;
        directWrites.push(v);
      },
      setTargetAtTime: (target: number, at: number, tau: number) => ramp(target, at, tau),
    };
    player.fadeIn(1_000);
    expect(directWrites, "fadeIn zeroes the gain directly, then automates").toEqual([0]);
    directWrites.length = 0;
    ramp.mockClear();

    player.sync(e, 0); // mid-fade (stub context sits at currentTime 0)
    expect(directWrites, "no direct write may cancel the in-flight fade").toEqual([]);
    expect(ramp, "the automation is re-targeted over the fade's remaining time").toHaveBeenCalledTimes(1);

    // Past the fade's end a sync ramps normally again (small tau — no click, no snap).
    (ctx as { currentTime: number }).currentTime = 2;
    ramp.mockClear();
    player.sync(e, 0);
    expect(directWrites).toEqual([]);
    expect(ramp).toHaveBeenCalledWith(0.8, 2, expect.any(Number));
    player.dispose();
  });
});
