// @vitest-environment node
import { describe, it, expect, vi } from "vitest";
import { Dice3DBridge, type Dice3DHost } from "./dice3dInteraction";

describe("Dice3DBridge", () => {
  it("no-ops before attach", () => {
    const bridge = new Dice3DBridge();
    expect(() => bridge.roll({} as never, "r1")).not.toThrow();
    expect(() => bridge.clear()).not.toThrow();
  });

  it("forwards to the attached host", () => {
    const bridge = new Dice3DBridge();
    const host: Dice3DHost = { roll: vi.fn(), clear: vi.fn() };
    bridge.attach(host);
    bridge.roll({} as never, "r1");
    bridge.clear();
    expect(host.roll).toHaveBeenCalledWith({}, "r1");
    expect(host.clear).toHaveBeenCalledOnce();
  });

  it("a stale detach after re-attach is a no-op", () => {
    const bridge = new Dice3DBridge();
    const first: Dice3DHost = { roll: vi.fn(), clear: vi.fn() };
    const second: Dice3DHost = { roll: vi.fn(), clear: vi.fn() };
    const detachFirst = bridge.attach(first);
    bridge.attach(second);
    detachFirst();
    bridge.roll({} as never, "r1");
    expect(first.roll).not.toHaveBeenCalled();
    expect(second.roll).toHaveBeenCalledWith({}, "r1");
  });

  it("no-ops after detach", () => {
    const bridge = new Dice3DBridge();
    const host: Dice3DHost = { roll: vi.fn(), clear: vi.fn() };
    const detach = bridge.attach(host);
    detach();
    bridge.roll({} as never, "r1");
    expect(host.roll).not.toHaveBeenCalled();
  });
});
