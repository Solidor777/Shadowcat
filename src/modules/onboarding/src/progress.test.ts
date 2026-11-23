import { describe, expect, it, vi } from "vitest";
import { memorySettings, tour } from "./__fixtures__/harness";
import { TourProgress, doneKey } from "./progress";

describe("doneKey", () => {
  it("keys a global tour by id and a world tour by world and id", () => {
    expect(doneKey({ id: "a.b", scope: "global" }, "w1")).toBe("g:a.b");
    expect(doneKey({ id: "a.b", scope: "world" }, "w1")).toBe("w:w1:a.b");
    expect(doneKey({ id: "a.b", scope: "world" }, "w2")).not.toBe(doneKey({ id: "a.b", scope: "world" }, "w1"));
  });
});

describe("TourProgress", () => {
  it("reports 0 for a tour never finished", () => {
    const progress = new TourProgress(memorySettings(), () => {});
    expect(progress.doneVersion(tour(), "w1")).toBe(0);
  });

  it("writes one flat key per finished tour through setUser", () => {
    const settings = memorySettings();
    new TourProgress(settings, () => {}).markDone(tour({ version: 3 }), "w1");
    expect(settings.writes).toEqual([{ "g:example.intro": 3 }]);
  });

  it("reads back a previous session's stored version", () => {
    const progress = new TourProgress(memorySettings({ "g:example.intro": 2 }), () => {});
    expect(progress.doneVersion(tour(), "w1")).toBe(2);
  });

  it("sees its own write immediately, before the server accepted it", () => {
    const settings = { getUser: () => ({}), setUser: () => new Promise<void>(() => {}) };
    const progress = new TourProgress(settings, () => {});
    progress.markDone(tour({ version: 1 }), "w1");
    expect(progress.doneVersion(tour(), "w1")).toBe(1);
  });

  it("keeps a world-scoped tour's state separate per world", () => {
    const progress = new TourProgress(memorySettings(), () => {});
    const scoped = tour({ scope: "world" });
    progress.markDone(scoped, "w1");
    expect(progress.doneVersion(scoped, "w1")).toBe(1);
    expect(progress.doneVersion(scoped, "w2")).toBe(0);
  });

  it("records the disabled switch", () => {
    const settings = memorySettings();
    const progress = new TourProgress(settings, () => {});
    expect(progress.isDisabled()).toBe(false);
    progress.setDisabled();
    expect(progress.isDisabled()).toBe(true);
    expect(settings.writes).toEqual([{ disabled: true }]);
  });

  it("reports a rejected write and keeps the value for the session", async () => {
    const onError = vi.fn();
    const failure = new Error("offline");
    const progress = new TourProgress({ getUser: () => ({}), setUser: () => Promise.reject(failure) }, onError);
    progress.markDone(tour(), "w1");
    await Promise.resolve();
    await Promise.resolve();
    expect(onError).toHaveBeenCalledWith(failure);
    expect(progress.doneVersion(tour(), "w1")).toBe(1);
  });
});
