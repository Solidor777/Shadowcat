// @vitest-environment node
import { describe, it, expect } from "vitest";
import { VadEngine } from "./micVad";

describe("VadEngine", () => {
  it("stays silent under a constant quiet floor", () => {
    const engine = new VadEngine({ sensitivity: 2.5 });
    let demand = 0;
    for (let i = 0; i < 50; i += 1) demand = engine.pushFrame(0.01);
    expect(demand).toBe(0);
  });

  it("requires 3 consecutive above-threshold frames to start speech", () => {
    const engine = new VadEngine({ sensitivity: 2 });
    for (let i = 0; i < 60; i += 1) engine.pushFrame(0.01); // establish a low floor
    expect(engine.pushFrame(1)).toBe(0);
    expect(engine.pushFrame(1)).toBe(0);
    expect(engine.pushFrame(1)).toBe(1); // third consecutive frame starts speech
  });

  it("a single loud frame among quiet ones resets the onset counter", () => {
    const engine = new VadEngine({ sensitivity: 2 });
    for (let i = 0; i < 60; i += 1) engine.pushFrame(0.01);
    engine.pushFrame(1);
    engine.pushFrame(1);
    expect(engine.pushFrame(0.01)).toBe(0); // quiet frame breaks the streak
    expect(engine.pushFrame(1)).toBe(0); // only 1 consecutive frame again
  });

  it("holds demand through the 400ms hangover after speech drops below threshold", () => {
    const engine = new VadEngine({ sensitivity: 2, frameMs: 20 });
    for (let i = 0; i < 60; i += 1) engine.pushFrame(0.01);
    engine.pushFrame(1);
    engine.pushFrame(1);
    expect(engine.pushFrame(1)).toBe(1); // speech starts
    // 400ms / 20ms = 20 hangover frames; the 20th quiet frame after onset still reports 1.
    let demand = 1;
    for (let i = 0; i < 19; i += 1) demand = engine.pushFrame(0.01);
    expect(demand).toBe(1);
    expect(engine.pushFrame(0.01)).toBe(0); // the 20th quiet frame ends the hangover
  });

  it("an adaptive floor rejects a rms level that would have triggered against a lower floor", () => {
    const quiet = new VadEngine({ sensitivity: 2 });
    const loud = new VadEngine({ sensitivity: 2 });
    for (let i = 0; i < 60; i += 1) quiet.pushFrame(0.01);
    for (let i = 0; i < 60; i += 1) loud.pushFrame(0.2); // a noisier room raises the floor
    // The SAME rms trips the quiet-room engine but not the noisier-room one.
    expect(quiet.pushFrame(0.05)).toBe(0); // still below 3-frame onset on frame 1
    expect(quiet.pushFrame(0.05)).toBe(0);
    expect(quiet.pushFrame(0.05)).toBe(1);
    expect(loud.pushFrame(0.05)).toBe(0);
    expect(loud.pushFrame(0.05)).toBe(0);
    expect(loud.pushFrame(0.05)).toBe(0); // 0.05 never clears the noisier floor's threshold
  });
});

import { MicVadSource, type MicVadDeps } from "./micVad";

function stubDeps(overrides: Partial<MicVadDeps> = {}): MicVadDeps {
  return {
    getUserMedia: async () => ({ getTracks: () => [] }) as unknown as MediaStream,
    addWorkletModule: async () => {},
    createWorkletNode: () =>
      ({ port: { onmessage: null, close: () => {} }, connect: () => {}, disconnect: () => {} }) as unknown as AudioWorkletNode,
    createMediaStreamSource: () => ({ connect: () => {}, disconnect: () => {} }) as unknown as MediaStreamAudioSourceNode,
    ...overrides,
  };
}

describe("MicVadSource", () => {
  it("enable() resolves null and forwards worklet messages to the sink", async () => {
    // A mutable field on a `const` holder, not a reassigned `let` — TypeScript's control-flow
    // narrowing cannot track a `let` reassigned only inside a nested closure back to its
    // declared union type at the read site below, so it narrows to `never` instead.
    const captured: {
      port: { onmessage: ((e: MessageEvent<boolean>) => void) | null; close: () => void } | null;
    } = { port: null };
    const deps = stubDeps({
      createWorkletNode: () => {
        // The port object is shared by reference between `captured.port` and the returned
        // node — MicVadSource.enable() assigns `node.port.onmessage`, and this test reads
        // it back through `captured.port` to invoke it; a spread copy here would give the
        // two references distinct port objects and the assignment would never be observed.
        const port = { onmessage: null as ((e: MessageEvent<boolean>) => void) | null, close: () => {} };
        captured.port = port;
        return { port, connect: () => {}, disconnect: () => {} } as unknown as AudioWorkletNode;
      },
    });
    const calls: number[] = [];
    const source = new MicVadSource({
      audioContext: {} as AudioContext,
      workletUrl: "vad.worklet.js",
      logger: { debug() {}, warn() {}, error() {} },
      deps,
    });
    source.setSink({ set: (v) => calls.push(v) });
    const denial = await source.enable();
    expect(denial).toBeNull();
    expect(source.isEnabled()).toBe(true);
    captured.port?.onmessage?.({ data: true } as MessageEvent<boolean>);
    expect(calls).toEqual([1]);
  });

  it("enable() reports permission-denied without throwing", async () => {
    const deps = stubDeps({
      getUserMedia: async () => {
        throw new DOMException("denied", "NotAllowedError");
      },
    });
    const source = new MicVadSource({
      audioContext: {} as AudioContext,
      workletUrl: "vad.worklet.js",
      logger: { debug() {}, warn() {}, error() {} },
      deps,
    });
    const denial = await source.enable();
    expect(denial).toBe("permission-denied");
    expect(source.isEnabled()).toBe(false);
  });

  it("disable() resets demand to 0 and stops the tracks", async () => {
    const stopped: boolean[] = [];
    const deps = stubDeps({
      getUserMedia: async () => ({ getTracks: () => [{ stop: () => stopped.push(true) }] }) as unknown as MediaStream,
    });
    const calls: number[] = [];
    const source = new MicVadSource({
      audioContext: {} as AudioContext,
      workletUrl: "vad.worklet.js",
      logger: { debug() {}, warn() {}, error() {} },
      deps,
    });
    source.setSink({ set: (v) => calls.push(v) });
    await source.enable();
    source.disable();
    expect(stopped).toEqual([true]);
    expect(calls.at(-1)).toBe(0);
    expect(source.isEnabled()).toBe(false);
  });
});
