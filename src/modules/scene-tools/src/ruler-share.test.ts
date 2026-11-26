// @vitest-environment node
// Exercises plain state and pure functions: no component render and no DOM API use.
import { describe, it, expect } from "vitest";
import { RULER_STALE_MS } from "@shadowcat/render";
import { RULER_LIMITS } from "@shadowcat/core";
import { countHypot } from "./__fixtures__/work-counter";
import {
  RulerSharer,
  MAX_RULER_POINTS,
  RULER_SEND_INTERVAL_MS,
  RULER_KEEPALIVE_MS,
} from "./ruler-share";
import type { Polyline } from "./polyline-simplify";

/** A logical clock plus timer queue: `advance` moves time and runs every due timer in order. */
function fakeClock() {
  let t = 0;
  const timers: Array<{ id: number; at: number; fn: () => void }> = [];
  let next = 1;
  return {
    now: () => t,
    schedule: (fn: () => void, ms: number): unknown => {
      const id = next++;
      timers.push({ id, at: t + ms, fn });
      return id;
    },
    cancel: (h: unknown): void => {
      const i = timers.findIndex((x) => x.id === h);
      if (i >= 0) timers.splice(i, 1);
    },
    advance(ms: number): void {
      const target = t + ms;
      for (;;) {
        timers.sort((a, b) => a.at - b.at);
        const due = timers[0];
        if (!due || due.at > target) break;
        timers.shift();
        t = due.at;
        due.fn();
      }
      t = target;
    },
    pending: () => timers.length,
    /** Moves the wall clock back by `ms` WITHOUT moving any timer's due time in real time: a
     * real timer is relative and unaffected by a `Date.now` step. */
    stepBack(ms: number): void {
      t -= ms;
      for (const x of timers) x.at -= ms;
    },
  };
}

interface Sent {
  scene: string;
  points: Polyline;
  active: boolean;
}

function makeSharer(enabled: { on: boolean } = { on: true }, viewed: { scene: string | null } = { scene: "s1" }) {
  const clock = fakeClock();
  const sent: Sent[] = [];
  const sharer = new RulerSharer({
    enabled: () => enabled.on,
    viewedScene: () => viewed.scene,
    send: (scene, points, active) => sent.push({ scene, points, active }),
    now: clock.now,
    schedule: clock.schedule,
    cancel: clock.cancel,
  });
  return { sharer, clock, sent, enabled, viewed };
}

describe("RulerSharer", () => {
  it("caps at the server's per-frame cap", () => {
    expect(MAX_RULER_POINTS).toBe(RULER_LIMITS.maxPoints);
  });

  it("keeps the keepalive well inside the receiver's staleness window", () => {
    expect(RULER_KEEPALIVE_MS * 2).toBeLessThan(RULER_STALE_MS);
    expect(RULER_SEND_INTERVAL_MS).toBeLessThan(RULER_KEEPALIVE_MS);
  });

  it("sends the first update at once and coalesces a burst into one trailing send of the LATEST route", () => {
    const { sharer, clock, sent } = makeSharer();
    sharer.update("s1", [[0, 0], [1, 1]]);
    expect(sent).toEqual([{ scene: "s1", points: [[0, 0], [1, 1]], active: true }]);

    // Three more updates inside the window send nothing yet.
    clock.advance(50);
    sharer.update("s1", [[0, 0], [2, 2]]);
    clock.advance(50);
    sharer.update("s1", [[0, 0], [3, 3]]);
    clock.advance(50);
    sharer.update("s1", [[0, 0], [4, 4]]);
    expect(sent).toHaveLength(1);

    // The window ends: exactly one send, carrying the newest route (not the first or second).
    clock.advance(RULER_SEND_INTERVAL_MS);
    expect(sent).toHaveLength(2);
    expect(sent[1].points).toEqual([[0, 0], [4, 4]]);
  });

  it("re-sends an unchanged ruler on the keepalive until stopped", () => {
    const { sharer, clock, sent } = makeSharer();
    sharer.update("s1", [[5, 5], [6, 6]]);
    clock.advance(RULER_KEEPALIVE_MS);
    clock.advance(RULER_KEEPALIVE_MS);
    expect(sent.filter((s) => s.active)).toHaveLength(3);
    expect(sent.every((s) => JSON.stringify(s.points) === "[[5,5],[6,6]]")).toBe(true);
  });

  it("stop sends a terminating frame on the scene the ruler was sent on, then goes silent", () => {
    const { sharer, clock, sent } = makeSharer();
    sharer.update("s1", [[0, 0], [1, 1]]);
    sharer.stop();
    expect(sent.at(-1)).toEqual({ scene: "s1", points: [], active: false });
    const count = sent.length;
    clock.advance(RULER_KEEPALIVE_MS * 3);
    expect(sent).toHaveLength(count);
    expect(clock.pending()).toBe(0);
  });

  it("never sends a stop for a ruler that was never sent", () => {
    const { sharer, sent } = makeSharer();
    sharer.stop();
    expect(sent).toEqual([]);
  });

  it("switching the shared switch off mid-drag ends the ruler at once", () => {
    const { sharer, sent, enabled } = makeSharer();
    sharer.update("s1", [[0, 0], [1, 1]]);
    enabled.on = false;
    sharer.update("s1", [[0, 0], [2, 2]]);
    expect(sent.map((s) => s.active)).toEqual([true, false]);
  });

  it("never sends while disabled", () => {
    const { sharer, clock, sent } = makeSharer({ on: false });
    sharer.update("s1", [[0, 0], [1, 1]]);
    clock.advance(RULER_KEEPALIVE_MS * 3);
    sharer.stop();
    expect(sent).toEqual([]);
  });

  it("a change of scene ends the ruler on the old scene at once, then shares on the new one after the send interval", () => {
    const { sharer, sent, clock, viewed } = makeSharer();
    sharer.update("s1", [[0, 0], [1, 1]]);
    viewed.scene = "s2";
    sharer.update("s2", [[7, 7], [8, 8]]);
    expect(sent).toEqual([
      { scene: "s1", points: [[0, 0], [1, 1]], active: true },
      { scene: "s1", points: [], active: false },
    ]);
    clock.advance(RULER_SEND_INTERVAL_MS);
    expect(sent.at(-1)).toEqual({ scene: "s2", points: [[7, 7], [8, 8]], active: true });
  });

  it("switching sharing off with the pointer held still ends the ruler on the next keepalive, sending exactly one stop and then nothing", () => {
    const { sharer, clock, sent, enabled } = makeSharer();
    sharer.update("s1", [[0, 0], [1, 1]]);
    enabled.on = false;
    clock.advance(RULER_KEEPALIVE_MS);
    expect(sent.map((s) => s.active)).toEqual([true, false]);
    expect(clock.pending()).toBe(0);
    clock.advance(RULER_KEEPALIVE_MS * 5);
    expect(sent).toHaveLength(2);
  });

  it("switching sharing off before the first frame went out sends nothing at all", () => {
    const { sharer, clock, sent, enabled } = makeSharer();
    sharer.update("s1", [[0, 0], [1, 1]]);
    sharer.stop();
    sharer.update("s1", [[0, 0], [2, 2]]); // deferred: inside the interval of the stop
    enabled.on = false;
    clock.advance(RULER_KEEPALIVE_MS * 3);
    expect(sent.map((s) => s.active)).toEqual([true, false]);
  });

  it("a viewed-scene change with the pointer held still ends the ruler on the old scene at the next timer, then goes silent", () => {
    const { sharer, clock, sent, viewed } = makeSharer();
    sharer.update("s1", [[0, 0], [1, 1]]);
    viewed.scene = "s2";
    clock.advance(RULER_KEEPALIVE_MS);
    expect(sent).toEqual([
      { scene: "s1", points: [[0, 0], [1, 1]], active: true },
      { scene: "s1", points: [], active: false },
    ]);
    expect(clock.pending()).toBe(0);
    clock.advance(RULER_KEEPALIVE_MS * 5);
    expect(sent).toHaveLength(2);
  });

  it("thins the route only when a frame is sent, never on an update that is deferred or coalesced", () => {
    const { sharer, clock, sent } = makeSharer();
    // Thinning evaluates geometry (`Math.hypot`); copying and validating the route does not.
    const big = (n: number): Polyline => Array.from({ length: n }, (_, i) => [i, i % 2 === 0 ? 0 : 5]);
    const first = countHypot(() => sharer.update("s1", big(2000)));
    const coalesced = countHypot(() => {
      for (let i = 0; i < 20; i++) {
        clock.advance(5);
        sharer.update("s1", big(2000));
      }
    });
    const trailing = countHypot(() => clock.advance(RULER_SEND_INTERVAL_MS));
    expect(first.calls, "the first update sends at once, so it must thin").toBeGreaterThanOrEqual(2000);
    expect(coalesced.calls).toBe(0); // twenty coalesced updates thinned nothing
    expect(trailing.calls, "the one trailing send must thin the newest route").toBeGreaterThanOrEqual(2000);
    expect(sent).toHaveLength(2);
    expect(sent.at(-1)!.points).toHaveLength(MAX_RULER_POINTS);
  });

  it("a clock stepping backwards never delays the next send beyond the send interval or drops the keepalive", () => {
    const { sharer, clock, sent } = makeSharer();
    clock.advance(10 * 60_000);
    sharer.update("s1", [[0, 0], [1, 1]]);
    expect(sent).toHaveLength(1);
    clock.stepBack(60 * 60_000); // an hour backwards
    sharer.update("s1", [[0, 0], [2, 2]]);
    clock.advance(RULER_SEND_INTERVAL_MS);
    expect(sent.map((x) => JSON.stringify(x.points))).toEqual(["[[0,0],[1,1]]", "[[0,0],[2,2]]"]);
    // The keepalive still fires on schedule afterwards.
    clock.advance(RULER_KEEPALIVE_MS);
    expect(sent).toHaveLength(3);
  });

  it("a continuous drag right after a backwards clock step still sends within a few intervals instead of starving", () => {
    const { sharer, clock, sent } = makeSharer();
    clock.advance(10 * 60_000);
    sharer.update("s1", [[0, 0], [1, 1]]);
    clock.stepBack(60 * 60_000);
    // A pointer event every 50 ms: each update re-arms the trailing timer, so a wait that did
    // not shrink as time passes would never fire while the drag continues.
    for (let i = 0; i < 12; i++) {
      sharer.update("s1", [[0, 0], [i, i]]);
      clock.advance(50);
    }
    expect(sent.length).toBeGreaterThanOrEqual(2);
  });

  it("never sends a stop for a route that was only deferred and never went out", () => {
    const { sharer, sent, clock } = makeSharer();
    sharer.update("s1", [[0, 0], [1, 1]]);
    sharer.stop(); // stop #1: the route did go out
    sharer.update("s1", [[0, 0], [2, 2]]); // deferred inside the interval
    sharer.stop(); // nothing shown: no frame
    clock.advance(RULER_KEEPALIVE_MS * 3);
    expect(sent.map((s) => s.active)).toEqual([true, false]);
  });

  it("rapid measuring stays inside the server's per-user budget and never drops a stop", () => {
    // Worst pattern: gestures far faster than the send interval, each an update then its stop a
    // millisecond later (20 ms per gesture). Run for 3 minutes of logical time.
    const clock = fakeClock();
    const stamps: number[] = [];
    const wrapped: Sent[] = [];
    const tracked = new RulerSharer({
      enabled: () => true,
      viewedScene: () => "s1",
      send: (scene, points, active) => {
        stamps.push(clock.now());
        wrapped.push({ scene, points, active });
      },
      now: clock.now,
      schedule: clock.schedule,
      cancel: clock.cancel,
    });
    for (let i = 0; i < 9000; i++) {
      tracked.update("s1", [[0, 0], [1, i]]);
      clock.advance(1);
      tracked.stop();
      clock.advance(19);
    }
    let worst = 0;
    for (let i = 0, j = 0; i < stamps.length; i++) {
      while (stamps[i] - stamps[j] >= 60_000) j++;
      worst = Math.max(worst, i - j + 1);
    }
    expect(worst).toBeLessThanOrEqual(RULER_LIMITS.ratePerMin);
    // Non-vacuous: the pattern really does run at the 10-frames-a-second worst case.
    expect(worst).toBeGreaterThan(RULER_LIMITS.ratePerMin / 2);
    // Every shown ruler ended with a stop: actives and stops alternate and the last frame is a stop.
    expect(wrapped.map((w) => w.active).slice(0, 4)).toEqual([true, false, true, false]);
    expect(wrapped.filter((w) => w.active).length).toBeLessThan(9000 / 5); // deferred updates that never went out owe no stop
    expect(wrapped.at(-1)!.active).toBe(false);
    expect(wrapped.filter((w) => w.active)).toHaveLength(wrapped.filter((w) => !w.active).length);
  });

  it("copies the caller's route: mutating or reusing the array after update changes nothing that is sent", () => {
    const { sharer, clock, sent } = makeSharer();
    const route: Polyline = [[0, 0], [10, 0]];
    sharer.update("s1", route); // sent at once
    clock.advance(10);
    const deferred: Polyline = [[0, 0], [20, 0]];
    sharer.update("s1", deferred); // deferred to the trailing send
    deferred[1][0] = 999;
    deferred.length = 0;
    clock.advance(RULER_SEND_INTERVAL_MS);
    expect(sent.at(-1)!.points).toEqual([[0, 0], [20, 0]]);
    expect(sent[0].points).toEqual([[0, 0], [10, 0]]);
  });

  const badValues = [Number.NaN, Number.POSITIVE_INFINITY, Number.NEGATIVE_INFINITY, RULER_LIMITS.maxCoord * 2, -RULER_LIMITS.maxCoord * 2];
  for (const axis of [0, 1]) {
    for (const vertex of [0, 2]) {
      it(`never sends a route with a non-finite or over-bound coordinate on axis ${axis} of vertex ${vertex}, and leaves no timer`, () => {
        for (const bad of badValues) {
          const { sharer, clock, sent } = makeSharer();
          const route: Polyline = [[0, 0], [5, 5], [9, 9]];
          route[vertex][axis] = bad;
          sharer.update("s1", route);
          expect(sent, `nothing is sent for ${bad}`).toHaveLength(0);
          expect(clock.pending(), `no timer or keepalive remains for ${bad}`).toBe(0);
          clock.advance(10 * RULER_KEEPALIVE_MS);
          expect(sent).toHaveLength(0);
        }
      });
    }
  }

  it("accepts a coordinate exactly at the bound", () => {
    const { sharer, sent } = makeSharer();
    sharer.update("s1", [[-RULER_LIMITS.maxCoord, RULER_LIMITS.maxCoord], [0, 0]]);
    expect(sent).toHaveLength(1);
  });

  it("a bad route ends a shown ruler with one stop and leaves no timer", () => {
    const { sharer, clock, sent } = makeSharer();
    sharer.update("s1", [[0, 0], [5, 5]]);
    expect(sent).toHaveLength(1);
    clock.advance(RULER_SEND_INTERVAL_MS);
    sharer.update("s1", [[0, 0], [Number.NaN, 5]]);
    expect(sent.at(-1)).toEqual({ scene: "s1", points: [], active: false });
    expect(clock.pending()).toBe(0);
    sharer.update("s1", [[0, 0], [Number.POSITIVE_INFINITY, 5]]);
    expect(sent).toHaveLength(2);
  });

  it("an empty route ends the ruler", () => {
    const { sharer, sent } = makeSharer();
    sharer.update("s1", [[0, 0], [1, 1]]);
    sharer.update("s1", []);
    expect(sent.at(-1)).toEqual({ scene: "s1", points: [], active: false });
  });

  it("caps an over-long route before sending", () => {
    const { sharer, sent } = makeSharer();
    sharer.update("s1", Array.from({ length: MAX_RULER_POINTS * 4 }, (_, i) => [i, i % 2 === 0 ? 0 : 5]));
    expect(sent[0].points).toHaveLength(MAX_RULER_POINTS);
  });
});
