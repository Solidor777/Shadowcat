// @vitest-environment node
import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { SYNC_TICK_MS } from "./engine";
import { setMediaElementFactory } from "./track-player";
import { entry, rig, state } from "./__fixtures__/engineHarness";
import { stubMediaElement } from "./__fixtures__/stubContext";

beforeEach(() => {
  setMediaElementFactory(() => stubMediaElement());
});
afterEach(() => vi.restoreAllMocks());

describe("AudioEngine synchronisation tick", () => {
  it("schedules nothing before unlock and one SYNC_TICK_MS period after it", async () => {
    const { engine, timer } = rig();
    expect(timer.periodMs()).toBeNull();
    await engine.unlock();
    expect(timer.periodMs()).toBe(SYNC_TICK_MS);
  });

  it("re-applies the latest state on every tick: a position change that arrives only through the tick is applied", async () => {
    const { engine, clock, timer, elements } = rig();
    await engine.unlock();
    engine.applyState(state(entry("e1")));
    expect(elements).toHaveLength(1);
    expect(elements[0].currentTime).toBe(0);
    // The server clock advances 10 s with no audio-state change and the element did not move.
    clock.ms = 10_000;
    timer.fire();
    expect(elements[0].currentTime).toBeCloseTo(10, 5);
  });

  it("a tick starts no second player for an entry the engine already holds", async () => {
    const { engine, timer, elements } = rig();
    await engine.unlock();
    engine.applyState(state(entry("e1")));
    timer.fire();
    timer.fire();
    expect(elements).toHaveLength(1);
  });

  it("replays the latest state at unlock through the same source the tick uses", async () => {
    const { engine, clock, timer, elements } = rig();
    engine.applyState(state(entry("e1")));
    engine.applyState(state(entry("e1"), entry("e2", { asset: "a2" })));
    expect(elements).toHaveLength(0);
    clock.ms = 3_000;
    await engine.unlock();
    expect(elements).toHaveLength(2);
    expect(elements[0].currentTime).toBeCloseTo(3, 5);
    timer.fire();
    expect(elements).toHaveLength(2);
  });

  it("a page returning to the foreground re-applies the latest state; going hidden does not", async () => {
    const { engine, clock, visibility, elements } = rig();
    await engine.unlock();
    engine.applyState(state(entry("e1")));
    clock.ms = 20_000;
    visibility.set(false);
    expect(elements[0].currentTime).toBe(0);
    visibility.set(true);
    expect(elements[0].currentTime).toBeCloseTo(20, 5);
  });

  it("after dispose the timer is cleared, the observer is removed and nothing re-applies", async () => {
    const { engine, clock, timer, visibility, elements } = rig();
    await engine.unlock();
    engine.applyState(state(entry("e1")));
    engine.dispose();
    expect(timer.cleared()).toBe(1);
    expect(timer.periodMs()).toBeNull();
    expect(visibility.observers()).toBe(0);
    clock.ms = 30_000;
    timer.fire();
    visibility.set(true);
    expect(elements[0].currentTime).toBe(0);
  });

  it("the degraded no-Web-Audio engine ticks too", async () => {
    const { engine, clock, timer, elements } = rig({
      createContext: () => {
        throw new Error("no web audio");
      },
    });
    await engine.unlock();
    engine.applyState(state(entry("e1")));
    expect(timer.periodMs()).toBe(SYNC_TICK_MS);
    clock.ms = 12_000;
    timer.fire();
    expect(elements[0].currentTime).toBeCloseTo(12, 5);
  });
});

describe("AudioEngine synchronisation tick keeps a streaming element in step over a long run", () => {
  /** Plays 300 simulated seconds: the element's clock advances at its own `playbackRate` each
   * second; `ticks` decides whether the engine's tick runs. The server position starts 80 ms
   * ahead of the element (a not-yet-playing element at its first sync).
   * @returns The final signed drift, element minus server, seconds. */
  async function run(ticks: boolean): Promise<number> {
    const { engine, clock, timer, elements } = rig();
    await engine.unlock();
    clock.ms = 80;
    engine.applyState(state(entry("e1")));
    const el = elements[0];
    for (let second = 1; second <= 300; second++) {
      clock.ms += 1000;
      el.currentTime += el.playbackRate;
      if (ticks) timer.fire();
    }
    return el.currentTime - clock.ms / 1000;
  }

  it("without a tick the nudge never returns to 1 and the drift grows past a second", async () => {
    expect(Math.abs(await run(false))).toBeGreaterThan(1);
  });

  it("with the tick the drift stays inside the engage bound plus one tick's nudge", async () => {
    expect(Math.abs(await run(true))).toBeLessThan(0.05);
  });
});
