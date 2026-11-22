import { describe, it, expect, vi } from "vitest";
import { EscapeCoordinator } from "./escapeCoordinator";

/** Builds a coordinator on a private target so tests never touch the shared singleton. */
function setup(): { coordinator: EscapeCoordinator; press: (key?: string, target?: EventTarget) => KeyboardEvent } {
  const root = new EventTarget();
  const coordinator = new EscapeCoordinator(root);
  return {
    coordinator,
    press: (key = "Escape", target = root) => {
      const event = new KeyboardEvent("keydown", { key, cancelable: true, bubbles: true });
      target.dispatchEvent(event);
      return event;
    },
  };
}

describe("EscapeCoordinator", () => {
  it("fires only the highest-priority registration", () => {
    const { coordinator, press } = setup();
    const low = vi.fn();
    const high = vi.fn();
    coordinator.register(10, low);
    coordinator.register(20, high);
    press();
    expect(high).toHaveBeenCalledOnce();
    expect(low).not.toHaveBeenCalled();
  });

  it("a higher priority wins regardless of registration order", () => {
    const { coordinator, press } = setup();
    const high = vi.fn();
    const low = vi.fn();
    coordinator.register(20, high);
    coordinator.register(10, low);
    press();
    expect(high).toHaveBeenCalledOnce();
    expect(low).not.toHaveBeenCalled();
  });

  it("ties resolve to the most recently registered", () => {
    const { coordinator, press } = setup();
    const first = vi.fn();
    const second = vi.fn();
    coordinator.register(10, first);
    coordinator.register(10, second);
    press();
    expect(second).toHaveBeenCalledOnce();
    expect(first).not.toHaveBeenCalled();
  });

  it("a disposed registration no longer competes, and disposing twice is harmless", () => {
    const { coordinator, press } = setup();
    const low = vi.fn();
    const high = vi.fn();
    coordinator.register(10, low);
    const dispose = coordinator.register(20, high);
    dispose();
    dispose();
    press();
    expect(low).toHaveBeenCalledOnce();
    expect(high).not.toHaveBeenCalled();
  });

  it("consumes the native event when a registration wins", () => {
    const { coordinator, press } = setup();
    coordinator.register(10, () => {});
    const event = press();
    expect(event.defaultPrevented).toBe(true);
  });

  it("stops propagation before invoking the winner", () => {
    const host = document.createElement("div");
    const child = document.createElement("button");
    host.appendChild(child);
    const coordinator = new EscapeCoordinator(host);
    const downstream = vi.fn();
    child.addEventListener("keydown", downstream);
    coordinator.register(10, () => {});
    // The coordinator's capture listener on the host runs before the child's own listener.
    child.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", cancelable: true, bubbles: true }));
    expect(downstream).not.toHaveBeenCalled();
  });

  it("does nothing, and leaves the event untouched, with no registrations", () => {
    const { press } = setup();
    const event = press();
    expect(event.defaultPrevented).toBe(false);
  });

  it("ignores keys other than Escape", () => {
    const { coordinator, press } = setup();
    const cb = vi.fn();
    coordinator.register(10, cb);
    const event = press("Enter");
    expect(cb).not.toHaveBeenCalled();
    expect(event.defaultPrevented).toBe(false);
  });

  it("skips a registration whose appliesTo rejects the event, falling to the next", () => {
    const { coordinator, press } = setup();
    const scoped = vi.fn();
    const general = vi.fn();
    coordinator.register(10, general);
    coordinator.register(20, scoped, () => false);
    press();
    expect(scoped).not.toHaveBeenCalled();
    expect(general).toHaveBeenCalledOnce();
  });

  it("leaves the event unconsumed when the only registration's appliesTo rejects it", () => {
    const { coordinator, press } = setup();
    coordinator.register(20, () => {}, () => false);
    expect(press().defaultPrevented).toBe(false);
  });
});
