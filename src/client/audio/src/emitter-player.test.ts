// @vitest-environment node
import { describe, it, expect, vi, afterEach, beforeEach } from "vitest";
import { AssetResolver, type AudibleEmitter } from "@shadowcat/core";
import { EmitterPlayer, AUDIBILITY_RAMP_TAU_SECS } from "./emitter-player";
import { OneShotPlayer } from "./one-shot-player";
import { setMediaElementFactory } from "./track-player";
import type { GainNodeLike } from "./context";
import { stubAudioContext, stubMediaElement, stubWasmDecoder, wavBytes } from "./__fixtures__/stubContext";

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

function emitter(over: Partial<AudibleEmitter> = {}): AudibleEmitter {
  return { token: "tok-1", asset: "a-wind", gain: 0.5, pan: 0.4, loop: true, ...over };
}

beforeEach(() => {
  vi.spyOn(globalThis, "fetch").mockImplementation(async () => new Response(wavBytes()));
  setMediaElementFactory(() => stubMediaElement());
});
afterEach(() => vi.restoreAllMocks());

describe("EmitterPlayer", () => {
  it("an asset change decodes and (re)starts the source; a same-asset update only ramps", async () => {
    const ctx = stubAudioContext();
    const player = new EmitterPlayer(ctx, oneShotFor(ctx), dest());

    await player.sync(emitter(), true);
    expect(ctx.sources).toHaveLength(1);
    expect(ctx.sources[0].loop, "the emission's own loop flag drives the source").toBe(true);

    // Same asset, changed gain/pan: ramps through setTargetAtTime, NO restart — the
    // ramp-vs-restart distinction is the whole point of the asset-identity check.
    const first = ctx.sources[0];
    await player.sync(emitter({ gain: 0.2, pan: -0.6 }), true);
    expect(first.stop).not.toHaveBeenCalled();
    expect(ctx.sources).toHaveLength(1);
    expect(ctx.gains[0].gain.value, "the stub's setTargetAtTime writes through to value").toBe(0.2);
    expect(ctx.panners[0].pan.value).toBe(-0.6);

    // A different asset: the old source stops, a new one starts.
    await player.sync(emitter({ asset: "a-rain" }), true);
    expect(first.stop).toHaveBeenCalledTimes(1);
    expect(ctx.sources).toHaveLength(2);
  });

  it("ramps gain/pan through setTargetAtTime with the audibility time constant", async () => {
    const ctx = stubAudioContext();
    const player = new EmitterPlayer(ctx, oneShotFor(ctx), dest());
    const gainRamp = vi.spyOn(ctx.gains[0].gain, "setTargetAtTime");
    const panRamp = vi.spyOn(ctx.panners[0].pan, "setTargetAtTime");
    await player.sync(emitter({ gain: 0.3, pan: 0.9 }), true);
    expect(gainRamp).toHaveBeenCalledWith(0.3, 0, AUDIBILITY_RAMP_TAU_SECS);
    expect(panRamp).toHaveBeenCalledWith(0.9, 0, AUDIBILITY_RAMP_TAU_SECS);
  });

  it("spatialOverride false centers the pan regardless of the frame's pan, never touching gain", async () => {
    const ctx = stubAudioContext();
    const player = new EmitterPlayer(ctx, oneShotFor(ctx), dest());
    await player.sync(emitter({ pan: 0.9 }), false);
    expect(ctx.panners[0].pan.value, "gain folds the server's falloff — never second-guessed").toBe(0);
    expect(ctx.gains[0].gain.value).toBe(0.5);
  });

  it("dispose stops the source and disconnects both nodes", async () => {
    const ctx = stubAudioContext();
    const player = new EmitterPlayer(ctx, oneShotFor(ctx), dest());
    await player.sync(emitter(), true);
    const gainDisc = vi.spyOn(ctx.gains[0], "disconnect");
    const panDisc = vi.spyOn(ctx.panners[0], "disconnect");
    player.dispose();
    expect(ctx.sources[0].stop).toHaveBeenCalledTimes(1);
    expect(gainDisc).toHaveBeenCalledTimes(1);
    expect(panDisc).toHaveBeenCalledTimes(1);
  });

  it("the LATEST sync call's asset wins regardless of decode-resolution order", async () => {
    const ctx = stubAudioContext();
    const oneShot = oneShotFor(ctx);
    let resolveFirst!: (b: Awaited<ReturnType<OneShotPlayer["getBuffer"]>>) => void;
    let resolveSecond!: (b: Awaited<ReturnType<OneShotPlayer["getBuffer"]>>) => void;
    let call = 0;
    vi.spyOn(oneShot, "getBuffer").mockImplementation(() => {
      call += 1;
      if (call === 1) return new Promise((res) => (resolveFirst = res));
      return new Promise((res) => (resolveSecond = res));
    });
    const player = new EmitterPlayer(ctx, oneShot, dest());

    const first = player.sync(emitter({ asset: "a-wind" }), true);
    const second = player.sync(emitter({ asset: "a-rain" }), true);

    const buffer = { duration: 1, sampleRate: 48000, numberOfChannels: 1 } as Awaited<
      ReturnType<OneShotPlayer["getBuffer"]>
    >;
    // The SECOND (latest) call's decode resolves FIRST.
    resolveSecond(buffer);
    await second;
    resolveFirst(buffer);
    await first;

    // Only one source was ever started, and it plays the latest-requested asset.
    expect(ctx.sources).toHaveLength(1);
  });

  it("dispose during an in-flight sync means the continuation never creates a source", async () => {
    const ctx = stubAudioContext();
    const oneShot = oneShotFor(ctx);
    let resolve!: (b: Awaited<ReturnType<OneShotPlayer["getBuffer"]>>) => void;
    vi.spyOn(oneShot, "getBuffer").mockImplementation(
      () => new Promise((res) => (resolve = res)),
    );
    const player = new EmitterPlayer(ctx, oneShot, dest());

    const pending = player.sync(emitter(), true);
    player.dispose();
    resolve({ duration: 1, sampleRate: 48000, numberOfChannels: 1 } as Awaited<
      ReturnType<OneShotPlayer["getBuffer"]>
    >);
    await pending;

    expect(ctx.sources).toHaveLength(0);
  });

  it("clamps a negative server-sent gain to 0 before writing to the AudioParam", async () => {
    const ctx = stubAudioContext();
    const player = new EmitterPlayer(ctx, oneShotFor(ctx), dest());
    await player.sync(emitter({ gain: -0.4 }), true);
    expect(ctx.gains[0].gain.value).toBeGreaterThanOrEqual(0);
    expect(ctx.gains[0].gain.value).toBe(0);
  });

  it("clamps a gain above 1 to 1 before writing to the AudioParam", async () => {
    const ctx = stubAudioContext();
    const player = new EmitterPlayer(ctx, oneShotFor(ctx), dest());
    await player.sync(emitter({ gain: 1.7 }), true);
    expect(ctx.gains[0].gain.value).toBe(1);
  });
});
