// @vitest-environment node
import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { AssetResolver, type AudioStateEngine, type PlayingTrack } from "@shadowcat/core";
import { AudioEngine, type AudioEngineOpts } from "./engine";
import { setMediaElementFactory } from "./track-player";
import { stubAudioContext, stubMediaElement, stubWasmDecoder, wavBytes } from "./__fixtures__/stubContext";

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

function state(...playing: PlayingTrack[]): AudioStateEngine {
  return { playing, shuffleSeed: 0 };
}

/** A manually pumped raf/caf pair (production passes `requestAnimationFrame`). */
function pumpHarness() {
  let queued: ((now: number) => void) | null = null;
  return {
    raf: (cb: (now: number) => void): number => {
      queued = cb;
      return 1;
    },
    caf: (): void => {
      queued = null;
    },
    pump(now: number): void {
      const cb = queued;
      queued = null;
      cb?.(now);
    },
    hasQueued: (): boolean => queued !== null,
  };
}

function makeOpts(over: Partial<AudioEngineOpts> = {}): AudioEngineOpts {
  const pump = pumpHarness();
  return {
    resolver: new AssetResolver(),
    serverNow: () => 0,
    transport: vi.fn(),
    createContext: () => stubAudioContext(),
    createOggOpusDecoder: async () => stubWasmDecoder(),
    raf: pump.raf,
    caf: pump.caf,
    ...over,
  };
}

beforeEach(() => {
  setMediaElementFactory(() => stubMediaElement());
});
afterEach(() => vi.restoreAllMocks());

describe("AudioEngine", () => {
  it("tracks pending state before unlock and applies it once unlock resolves", async () => {
    let elements = 0;
    setMediaElementFactory(() => {
      elements++;
      return stubMediaElement();
    });
    const engine = new AudioEngine(makeOpts());
    engine.applyState(state(entry("e1")));
    // No context yet: NO player exists — a streaming entry would have needed an element.
    expect(elements).toBe(0);
    await engine.unlock();
    expect(elements).toBe(1); // the queued state produced exactly one live player
    engine.applyState(state());
  });

  it("diffs applyState across successive calls: create, keep, remove", async () => {
    const engine = new AudioEngine(makeOpts());
    await engine.unlock();
    engine.applyState(state(entry("a"), entry("b")));
    engine.applyState(state(entry("b"), entry("c")));
    engine.applyState(state());
  });

  it("setChannel writes the live channel GainNode and clamps gain to 0..=1", async () => {
    const opts = makeOpts();
    const ctx = stubAudioContext();
    opts.createContext = () => ctx;
    const engine = new AudioEngine(opts);
    await engine.unlock();
    // Channel node order in unlock(): master, duck, then master/music/ambience/sfx/ui.
    const sfxNode = ctx.gains[5];
    engine.setChannel("sfx", { gain: 0.4 });
    expect(sfxNode.gain.value).toBe(0.4);
    engine.setChannel("sfx", { muted: true });
    expect(sfxNode.gain.value).toBe(0);
    engine.setChannel("sfx", { muted: false });
    expect(sfxNode.gain.value).toBe(0.4);
    engine.setChannel("sfx", { gain: 5 });
    expect(engine.channels.sfx.gain).toBe(1);
    expect(sfxNode.gain.value).toBe(1);
  });

  it("playOneShot before unlock is a silent no-op (never throws)", () => {
    const engine = new AudioEngine(makeOpts());
    expect(() => engine.playOneShot("a1")).not.toThrow();
  });

  it("reports the constructor's duckDepth before unlock", () => {
    const engine = new AudioEngine(makeOpts({ duckDepth: 0.2 }));
    expect(engine.duck.depth).toBe(0.2);
  });

  it("the duck loop drives the duck GainNode toward 1 - depth and back", async () => {
    const pump = pumpHarness();
    const opts = makeOpts({ raf: pump.raf, caf: pump.caf });
    const engine = new AudioEngine(opts);
    await engine.unlock();
    expect(pump.hasQueued()).toBe(true);
    engine.duck.addSource("mic").set(1);
    pump.pump(1_000);
    pump.pump(1_100);
    const ducked = engine.duck.gain;
    expect(ducked).toBeLessThan(1);
    expect(ducked).toBeGreaterThan(1 - engine.duck.depth);
    engine.duck.addSource("mic").set(0);
    for (let t = 1_200; t < 6_000; t += 200) pump.pump(t);
    expect(engine.duck.gain).toBeGreaterThan(ducked);
    engine.dispose();
    expect(pump.hasQueued()).toBe(false);
  });

  it("dispose cancels the duck loop's raf handle and disconnects every player node", async () => {
    const pump = pumpHarness();
    const opts = makeOpts({ raf: pump.raf, caf: pump.caf });
    const ctx = stubAudioContext();
    opts.createContext = () => ctx;
    const caf = vi.spyOn(opts, "caf");
    const engine = new AudioEngine(opts);
    await engine.unlock();
    engine.applyState(state(entry("e1")));
    engine.dispose();
    expect(caf).toHaveBeenCalledTimes(1);
    expect(ctx.mediaSources[0].disconnect).toHaveBeenCalledTimes(1);
    pump.pump(9_999); // no re-arm after dispose
    expect(pump.hasQueued()).toBe(false);
  });

  it("crossfades a replaced entry over the playlist's fadeMs instead of a hard cut", async () => {
    const opts = makeOpts({ fadeMsFor: (pid) => (pid === "pl1" ? 5 : 0) });
    const ctx = stubAudioContext();
    opts.createContext = () => ctx;
    const engine = new AudioEngine(opts);
    await engine.unlock();
    engine.applyState(state(entry("a", { playlist: "pl1" })));
    const oldSource = ctx.mediaSources[0];
    // The advance: same playlist, fresh id.
    engine.applyState(state(entry("b", { playlist: "pl1" })));
    // The outgoing player RAMPED to 0 (crossfade) rather than disconnecting immediately...
    const oldPlayerGains = ctx.gains.filter((g) => g.gain.value === 0);
    expect(oldPlayerGains.length).toBeGreaterThan(0);
    expect(oldSource.disconnect).not.toHaveBeenCalled();
    // ...and the incoming player exists (second element created).
    expect(ctx.mediaSources.length).toBe(2);
    // The deferred dispose lands after fadeMs.
    await new Promise((r) => setTimeout(r, 30));
    expect(oldSource.disconnect).toHaveBeenCalledTimes(1);
  });

  it("a buffered-loop entry creates a player without a media element", async () => {
    vi.spyOn(globalThis, "fetch").mockImplementation(async () => new Response(wavBytes()));
    const engine = new AudioEngine(makeOpts());
    await engine.unlock();
    engine.applyState(state(entry("e1", { loop: true })));
    engine.applyState(state());
  });
});
