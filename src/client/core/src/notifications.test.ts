import { describe, it, expect, vi } from "vitest";
import { NotificationCenter, MAX_VISIBLE_NOTIFICATIONS } from "./notifications";

describe("NotificationCenter", () => {
  it("push adds a notification, items reflects it, and subscribe fires", () => {
    const center = new NotificationCenter();
    const listener = vi.fn();
    center.subscribe(listener);
    const id = center.push("warning", "Some targets were skipped.");
    expect(center.items).toHaveLength(1);
    expect(center.items[0]).toEqual({ id, level: "warning", message: "Some targets were skipped." });
    expect(listener).toHaveBeenCalledTimes(1);
  });

  it("dismiss removes the notification by id and fires subscribe; an unknown id is a no-op", () => {
    const center = new NotificationCenter();
    const listener = vi.fn();
    const id = center.push("info", "Saved.");
    center.subscribe(listener);
    center.dismiss("does-not-exist");
    expect(center.items).toHaveLength(1);
    expect(listener).not.toHaveBeenCalled();
    center.dismiss(id);
    expect(center.items).toHaveLength(0);
    expect(listener).toHaveBeenCalledTimes(1);
  });

  it("multiple pushes preserve insertion order in items", () => {
    const center = new NotificationCenter();
    center.push("info", "first");
    center.push("warning", "second");
    center.push("error", "third");
    expect(center.items.map((n) => n.message)).toEqual(["first", "second", "third"]);
  });

  it("push carries an optional action; omitting it stays backward compatible", () => {
    const center = new NotificationCenter();
    const run = vi.fn();
    const id = center.push("info", "Restorable.", { label: "Reopen windows", run });
    expect(center.items[0]).toEqual({
      id,
      level: "info",
      message: "Restorable.",
      action: { label: "Reopen windows", run },
    });
    const plain = center.push("info", "Plain.");
    expect(center.items[1]).toEqual({ id: plain, level: "info", message: "Plain." });
    expect(center.items[1].action).toBeUndefined();
  });
});

describe("NotificationCenter visible-concurrency cap", () => {
  it("queues a push beyond the visible cap, and items exposes only the visible set", () => {
    const center = new NotificationCenter();
    const ids = Array.from({ length: MAX_VISIBLE_NOTIFICATIONS + 2 }, (_, i) =>
      center.push("info", `n${i}`),
    );
    expect(center.items).toHaveLength(MAX_VISIBLE_NOTIFICATIONS);
    expect(center.items.map((n) => n.id)).toEqual(ids.slice(0, MAX_VISIBLE_NOTIFICATIONS));
  });

  it("dismissing a visible notification promotes the oldest queued one, FIFO", () => {
    const center = new NotificationCenter();
    const ids = Array.from({ length: MAX_VISIBLE_NOTIFICATIONS + 2 }, (_, i) =>
      center.push("info", `n${i}`),
    );
    center.dismiss(ids[0]);
    expect(center.items.map((n) => n.id)).toEqual([
      ...ids.slice(1, MAX_VISIBLE_NOTIFICATIONS),
      ids[MAX_VISIBLE_NOTIFICATIONS],
    ]);
    center.dismiss(ids[1]);
    expect(center.items.map((n) => n.id)).toEqual([
      ...ids.slice(2, MAX_VISIBLE_NOTIFICATIONS),
      ids[MAX_VISIBLE_NOTIFICATIONS],
      ids[MAX_VISIBLE_NOTIFICATIONS + 1],
    ]);
  });

  it("dismissing a queued (not yet visible) notification removes it from the queue without affecting items", () => {
    const center = new NotificationCenter();
    const ids = Array.from({ length: MAX_VISIBLE_NOTIFICATIONS + 2 }, (_, i) =>
      center.push("info", `n${i}`),
    );
    const visibleBefore = center.items.map((n) => n.id);
    center.dismiss(ids[MAX_VISIBLE_NOTIFICATIONS]); // queued, not visible
    expect(center.items.map((n) => n.id)).toEqual(visibleBefore);
    // The remaining queued entry still promotes correctly once a slot frees up.
    center.dismiss(ids[0]);
    expect(center.items.map((n) => n.id)).toEqual([
      ...ids.slice(1, MAX_VISIBLE_NOTIFICATIONS),
      ids[MAX_VISIBLE_NOTIFICATIONS + 1],
    ]);
  });

  it("dismissing an unknown id when a queue exists is a no-op (no promotion, no notify)", () => {
    const center = new NotificationCenter();
    Array.from({ length: MAX_VISIBLE_NOTIFICATIONS + 1 }, (_, i) => center.push("info", `n${i}`));
    const listener = vi.fn();
    center.subscribe(listener);
    center.dismiss("does-not-exist");
    expect(listener).not.toHaveBeenCalled();
    expect(center.items).toHaveLength(MAX_VISIBLE_NOTIFICATIONS);
  });
});

describe("NotificationCenter items identity", () => {
  it("push exposes a NEW items array, so a reference-equality reactive read observes it (dismiss already does)", () => {
    const center = new NotificationCenter();
    const before = center.items;
    center.push("info", "Saved.");
    expect(center.items).not.toBe(before);
    const afterPush = center.items;
    center.dismiss(afterPush[0].id);
    expect(center.items).not.toBe(afterPush);
  });
});

describe("NotificationCenter progress", () => {
  it("push stores a determinate and an indeterminate progress; omitting it leaves the field absent", () => {
    const center = new NotificationCenter();
    const a = center.push("info", "Uploading", undefined, { value: 0, max: 10 });
    const b = center.push("info", "Working", undefined, { value: null, max: 0 });
    const c = center.push("info", "Plain");
    expect(center.items.find((n) => n.id === a)?.progress).toEqual({ value: 0, max: 10 });
    expect(center.items.find((n) => n.id === b)?.progress).toEqual({ value: null, max: 0 });
    expect("progress" in center.items.find((n) => n.id === c)!).toBe(false);
  });

  it("update mutates in place: keeps id and position, replaces the items array, and notifies", () => {
    const center = new NotificationCenter();
    center.push("info", "first");
    const id = center.push("info", "Uploading", undefined, { value: 0, max: 10 });
    center.push("info", "last");
    const before = center.items;
    const listener = vi.fn();
    center.subscribe(listener);
    center.update(id, { progress: { value: 4, max: 10 }, message: "Uploading 40%" });
    expect(center.items).not.toBe(before);
    expect(center.items.map((n) => n.message)).toEqual(["first", "Uploading 40%", "last"]);
    expect(center.items[1]).toMatchObject({ id, progress: { value: 4, max: 10 } });
    expect(listener).toHaveBeenCalledTimes(1);
  });

  it("update on an unknown id is a no-op and does not notify", () => {
    const center = new NotificationCenter();
    center.push("info", "x");
    const listener = vi.fn();
    center.subscribe(listener);
    center.update("does-not-exist", { message: "y" });
    expect(listener).not.toHaveBeenCalled();
    expect(center.items[0].message).toBe("x");
  });

  it("update on a queued notification changes the queued entry, leaves the visible array alone, and the entry surfaces updated", () => {
    const center = new NotificationCenter();
    const ids = Array.from({ length: MAX_VISIBLE_NOTIFICATIONS }, (_, i) => center.push("info", `n${i}`));
    const queued = center.push("info", "queued", undefined, { value: 0, max: 5 });
    const visibleBefore = center.items;
    center.update(queued, { progress: { value: 3, max: 5 } });
    expect(center.items).toBe(visibleBefore);
    center.dismiss(ids[0]);
    expect(center.items.at(-1)).toMatchObject({ id: queued, progress: { value: 3, max: 5 } });
  });
});
