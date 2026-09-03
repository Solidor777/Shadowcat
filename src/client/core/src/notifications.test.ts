import { describe, it, expect, vi } from "vitest";
import { NotificationCenter } from "./notifications";

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
