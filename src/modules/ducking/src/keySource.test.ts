import { describe, it, expect, afterEach } from "vitest";
import type { KeybindingsController } from "@shadowcat/core";
import { KeySource, NULL_SINK, PUSH_TO_DUCK_BINDING_ID, DUCKING_MODULE_ID } from "./keySource";
import { makeBindings } from "./__fixtures__/bindings";

function press(type: "keydown" | "keyup", code: string, target: EventTarget = window): void {
  const event = new KeyboardEvent(type, { code, bubbles: true, cancelable: true });
  Object.defineProperty(event, "target", { value: target, configurable: true });
  target.dispatchEvent(event);
}

describe("KeySource", () => {
  let controller: KeybindingsController | undefined;

  /** Builds a started-ready source over a real keybindings controller listening on `window`. */
  function build(sink: { set(v: number): void }) {
    const made = makeBindings();
    controller = made.controller;
    return new KeySource(made.bindings, sink);
  }

  afterEach(() => {
    controller?.dispose();
    controller = undefined;
    document.body.innerHTML = "";
  });

  it("registers a rebindable push-to-duck binding on the default key at construction", () => {
    build({ set() {} });
    expect(controller!.bindingsFor(DUCKING_MODULE_ID)).toEqual([
      {
        moduleId: DUCKING_MODULE_ID,
        bindingId: PUSH_TO_DUCK_BINDING_ID,
        labelKey: "ducking.keySource.binding",
        combo: "backquote",
        defaultCombo: "backquote",
      },
    ]);
  });

  it("sets demand 1 on keydown and 0 on keyup for the bound key", () => {
    const calls: number[] = [];
    const source = build({ set: (v) => calls.push(v) });
    source.start();
    press("keydown", "Backquote");
    press("keyup", "Backquote");
    expect(calls).toEqual([1, 0]);
    source.stop();
  });

  it("ignores a different key", () => {
    const calls: number[] = [];
    const source = build({ set: (v) => calls.push(v) });
    source.start();
    press("keydown", "KeyA");
    press("keyup", "KeyA");
    expect(calls).toEqual([]);
    source.stop();
  });

  it("ignores keydown while the target is an editable element", () => {
    const calls: number[] = [];
    const input = document.createElement("input");
    document.body.appendChild(input);
    const source = build({ set: (v) => calls.push(v) });
    source.start();
    press("keydown", "Backquote", input);
    expect(calls).toEqual([]);
    source.stop();
  });

  it("follows a rebind: the new key ducks and releases, the old key does nothing", () => {
    const calls: number[] = [];
    const source = build({ set: (v) => calls.push(v) });
    source.start();
    controller!.rebind(DUCKING_MODULE_ID, PUSH_TO_DUCK_BINDING_ID, "KeyV");
    press("keydown", "Backquote");
    expect(calls).toEqual([]);
    press("keydown", "KeyV");
    press("keyup", "KeyV");
    expect(calls).toEqual([1, 0]);
    source.stop();
  });

  it("does not duck while stopped, though the binding stays registered", () => {
    const calls: number[] = [];
    const source = build({ set: (v) => calls.push(v) });
    press("keydown", "Backquote");
    expect(calls).toEqual([]);
    expect(controller!.bindingsFor(DUCKING_MODULE_ID)).toHaveLength(1);
    source.start();
    source.stop();
    expect(calls).toEqual([0]);
    press("keydown", "Backquote");
    expect(calls).toEqual([0]); // no further demand after stop()
  });

  it("setSink replaces the demand target", () => {
    const first: number[] = [];
    const second: number[] = [];
    const source = build({ set: (v) => first.push(v) });
    source.start();
    source.setSink({ set: (v) => second.push(v) });
    press("keydown", "Backquote");
    expect(first).toEqual([]);
    expect(second).toEqual([1]);
    source.stop();
  });

  it("resets demand to 0 on window blur while the bound key is held", () => {
    const calls: number[] = [];
    const source = build({ set: (v) => calls.push(v) });
    source.start();
    press("keydown", "Backquote");
    expect(calls).toEqual([1]);
    window.dispatchEvent(new Event("blur"));
    expect(calls).toEqual([1, 0]);
    source.stop();
  });

  it("the default sink discards every demand change", () => {
    expect(() => NULL_SINK.set(1)).not.toThrow();
  });
});
