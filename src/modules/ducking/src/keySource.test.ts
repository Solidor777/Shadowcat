import { describe, it, expect, afterEach } from "vitest";
import { KeySource, NULL_SINK } from "./keySource";

function press(type: "keydown" | "keyup", code: string, target: EventTarget = window): void {
  const event = new KeyboardEvent(type, { code, bubbles: true });
  Object.defineProperty(event, "target", { value: target, configurable: true });
  target.dispatchEvent(event);
}

describe("KeySource", () => {
  afterEach(() => {
    document.body.innerHTML = "";
  });

  it("sets demand 1 on keydown and 0 on keyup for the bound key", () => {
    const calls: number[] = [];
    const source = new KeySource({ set: (v) => calls.push(v) }, "Backquote");
    source.start();
    press("keydown", "Backquote");
    press("keyup", "Backquote");
    expect(calls).toEqual([1, 0]);
    source.stop();
  });

  it("ignores a different key", () => {
    const calls: number[] = [];
    const source = new KeySource({ set: (v) => calls.push(v) }, "Backquote");
    source.start();
    press("keydown", "KeyA");
    expect(calls).toEqual([]);
    source.stop();
  });

  it("ignores keydown while the target is an editable element", () => {
    const calls: number[] = [];
    const input = document.createElement("input");
    document.body.appendChild(input);
    const source = new KeySource({ set: (v) => calls.push(v) }, "Backquote");
    source.start();
    press("keydown", "Backquote", input);
    expect(calls).toEqual([]);
    source.stop();
  });

  it("stop() resets demand to 0 and detaches listeners", () => {
    const calls: number[] = [];
    const source = new KeySource({ set: (v) => calls.push(v) }, "Backquote");
    source.start();
    source.stop();
    expect(calls).toEqual([0]);
    press("keydown", "Backquote");
    expect(calls).toEqual([0]); // no further calls after stop()
  });

  it("setSink replaces the demand target", () => {
    const first: number[] = [];
    const second: number[] = [];
    const source = new KeySource({ set: (v) => first.push(v) }, "Backquote");
    source.start();
    source.setSink({ set: (v) => second.push(v) });
    press("keydown", "Backquote");
    expect(first).toEqual([]);
    expect(second).toEqual([1]);
    source.stop();
  });

  it("the default sink discards every demand change", () => {
    expect(() => NULL_SINK.set(1)).not.toThrow();
  });
});
