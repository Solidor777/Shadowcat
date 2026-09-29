// @vitest-environment node
import { describe, it, expect } from "vitest";
import { RemoteRulerView, RULER_STALE_MS, rulerColor } from "./remote-ruler-view";

describe("RemoteRulerView", () => {
  it("keys rulers by sender: concurrent senders are independent", () => {
    const v = new RemoteRulerView();
    v.set("a", "o1", [[0, 0], [10, 0]]);
    v.set("b", "o2", [[0, 5], [10, 5]]);
    const live = v.tick(0);
    expect(live.map((r) => r.user)).toEqual(["a", "b"]);
    expect(live[0].points).toEqual([[0, 0], [10, 0]]);
    expect(live[1].points).toEqual([[0, 5], [10, 5]]);
  });

  it("two connections of ONE user render as two rulers, and stopping one leaves the other", () => {
    const v = new RemoteRulerView();
    v.set("a", "tablet", [[0, 0], [10, 0]]);
    v.set("a", "laptop", [[0, 9], [10, 9]]);
    let live = v.tick(0);
    expect(live.map((r) => r.origin)).toEqual(["tablet", "laptop"]);
    expect(new Set(live.map((r) => r.color)).size).toBe(1);

    v.clear("a", "tablet");
    live = v.tick(0);
    expect(live.map((r) => r.origin)).toEqual(["laptop"]);
    expect(live[0].points).toEqual([[0, 9], [10, 9]]);

    v.set("a", "laptop", []);
    expect(v.tick(0)).toEqual([]);
  });

  it("a forged origin cannot touch another user's ruler: the sender is part of the key", () => {
    const v = new RemoteRulerView();
    v.set("victim", "shared-token", [[0, 0], [10, 0]]);
    v.set("forger", "shared-token", [[5, 5], [6, 6]]);
    v.clear("forger", "shared-token");
    const live = v.tick(0);
    expect(live.map((r) => r.user)).toEqual(["victim"]);
    expect(live[0].points).toEqual([[0, 0], [10, 0]]);
  });

  it("set replaces only that connection's ruler", () => {
    const v = new RemoteRulerView();
    v.set("a", "o1", [[0, 0]]);
    v.set("b", "o2", [[9, 9]]);
    v.set("a", "o1", [[1, 1], [2, 2]]);
    const live = v.tick(0);
    expect(live.find((r) => r.user === "a")!.points).toEqual([[1, 1], [2, 2]]);
    expect(live.find((r) => r.user === "b")!.points).toEqual([[9, 9]]);
  });

  it("clear removes exactly that connection's ruler", () => {
    const v = new RemoteRulerView();
    v.set("a", "o1", [[0, 0]]);
    v.set("b", "o2", [[9, 9]]);
    v.clear("a", "o1");
    expect(v.tick(0).map((r) => r.user)).toEqual(["b"]);
  });

  it("an empty point list is the stop signal for that connection only", () => {
    const v = new RemoteRulerView();
    v.set("a", "o1", [[0, 0]]);
    v.set("b", "o2", [[9, 9]]);
    v.set("a", "o1", []);
    expect(v.tick(0).map((r) => r.user)).toEqual(["b"]);
  });

  it("drops a ruler not updated within the staleness window, keeps one that was refreshed", () => {
    const v = new RemoteRulerView();
    v.set("stale", "o1", [[0, 0]]);
    v.set("fresh", "o2", [[1, 1]]);
    v.tick(RULER_STALE_MS - 1);
    v.set("fresh", "o2", [[1, 1]]); // keepalive resets only fresh's age
    const live = v.tick(2);
    expect(live.map((r) => r.user)).toEqual(["fresh"]);
    expect(v.tick(RULER_STALE_MS).length).toBe(0);
  });

  it("clearAll removes every ruler", () => {
    const v = new RemoteRulerView();
    v.set("a", "o1", [[0, 0]]);
    v.set("b", "o2", [[1, 1]]);
    v.clearAll();
    expect(v.tick(0)).toEqual([]);
  });

  it("assigns each sender a stable color that tick reports", () => {
    const v = new RemoteRulerView();
    v.set("a", "o1", [[0, 0]]);
    expect(v.tick(0)[0].color).toBe(rulerColor("a"));
    expect(rulerColor("a")).toBe(rulerColor("a"));
    // The palette spreads distinct ids over more than one color.
    const colors = new Set(["u1", "u2", "u3", "u4", "u5", "u6", "u7", "u8"].map(rulerColor));
    expect(colors.size).toBeGreaterThan(1);
  });
});
