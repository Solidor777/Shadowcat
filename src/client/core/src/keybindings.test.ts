// @vitest-environment node
import { afterEach, describe, expect, it, vi } from "vitest";
import { HookBus } from "./hooks";
import { silentLogger } from "./logger";
import {
  KeybindingsController,
  canonicalizeCombo,
  comboKey,
  describeCombo,
  isEditableTarget,
  isModifierOnly,
  normalizeCombo,
} from "./keybindings";

/** Modifier and code fields of a synthetic keydown. */
interface KeySpec {
  /** `KeyboardEvent.code`. */
  code: string;
  /** Layout-dependent character; present only to prove `normalizeCombo` ignores it. */
  key?: string;
  /** Ctrl held. */
  ctrlKey?: boolean;
  /** Shift held. */
  shiftKey?: boolean;
  /** Alt held. */
  altKey?: boolean;
  /** Meta held. */
  metaKey?: boolean;
  /** Auto-repeat flag. */
  repeat?: boolean;
  /** Overrides the event's `target`. */
  target?: unknown;
}

/** Builds a cancelable synthetic keydown carrying `spec`'s KeyboardEvent fields. */
function keydown(spec: KeySpec): KeyboardEvent {
  const event = new Event("keydown", { cancelable: true });
  const { target, ...fields } = spec;
  Object.assign(event, { ctrlKey: false, shiftKey: false, altKey: false, metaKey: false, repeat: false, ...fields });
  if (target !== undefined) Object.defineProperty(event, "target", { value: target });
  return event as unknown as KeyboardEvent;
}

/** A controller over a fresh bus listening on a private target the test dispatches on. */
function setup() {
  const target = new EventTarget();
  const hooks = new HookBus(silentLogger);
  const controller = new KeybindingsController(hooks, target);
  return { target, hooks, controller };
}

afterEach(() => {
  vi.unstubAllGlobals();
});

describe("normalizeCombo", () => {
  it("orders modifiers canonically regardless of how the event reports them", () => {
    const a = normalizeCombo({ code: "Backquote", ctrlKey: true, shiftKey: true, altKey: false, metaKey: false });
    const b = normalizeCombo({ code: "Backquote", shiftKey: true, ctrlKey: true, metaKey: false, altKey: false });
    expect(a).toBe("ctrl+shift+backquote");
    expect(b).toBe(a);
  });

  it("reads event.code, not event.key", () => {
    // A US-layout `key` on a non-US physical position: the combo names the position.
    const event = { code: "KeyZ", key: "y", ctrlKey: false, shiftKey: false, altKey: false, metaKey: false };
    expect(normalizeCombo(event)).toBe("keyz");
  });
});

describe("canonicalizeCombo / comboKey / isModifierOnly", () => {
  it("lowercases and reorders a hand-written combo", () => {
    expect(canonicalizeCombo("Shift+Ctrl+KeyK")).toBe("ctrl+shift+keyk");
    expect(comboKey("ctrl+shift+keyk")).toBe("keyk");
  });

  it("flags modifier keys pressed alone", () => {
    expect(isModifierOnly({ code: "ShiftLeft" })).toBe(true);
    expect(isModifierOnly({ code: "KeyK" })).toBe(false);
  });
});

describe("describeCombo", () => {
  /** A catalog lookup standing in for `i18n.t`: unknown keys echo back like the real fallback. */
  const catalog: Record<string, string> = {
    "keybindings.mod.ctrl": "Strg",
    "keybindings.comboSeparator": " + ",
    "keybindings.key.keyz": "Z (de)",
  };
  const t = (key: string): string => catalog[key] ?? key;

  it("resolves modifiers, named keys and the separator through the catalog", () => {
    expect(describeCombo("ctrl+keyz", t)).toBe("Strg + Z (de)");
  });

  it("never shows a raw code token for a key without a catalog entry", () => {
    expect(describeCombo("ctrl+keyv", t)).toBe("Strg + V");
    expect(describeCombo("backquote", t)).toBe("Backquote");
    expect(describeCombo("digit1", t)).toBe("1");
  });
});

describe("isEditableTarget", () => {
  /** Minimal element double: `instanceof HTMLElement` holds, with the two fields the guard reads. */
  class FakeElement {
    /** Whether the element is `contenteditable`. */
    isContentEditable = false;
    /** The element's tag name. */
    tagName = "DIV";
  }

  it("is false when the runtime has no DOM", () => {
    expect(isEditableTarget({} as EventTarget)).toBe(false);
  });

  it("is true for input, textarea and contenteditable elements only", () => {
    vi.stubGlobal("HTMLElement", FakeElement);
    const make = (patch: Partial<FakeElement>) => Object.assign(new FakeElement(), patch) as unknown as EventTarget;
    expect(isEditableTarget(make({ tagName: "INPUT" }))).toBe(true);
    expect(isEditableTarget(make({ tagName: "TEXTAREA" }))).toBe(true);
    expect(isEditableTarget(make({ isContentEditable: true }))).toBe(true);
    expect(isEditableTarget(make({}))).toBe(false);
    expect(isEditableTarget(null)).toBe(false);
  });
});

describe("KeybindingsController dispatch", () => {
  it("fires the action for the matching combo and not for a near miss", () => {
    const { target, controller } = setup();
    const action = vi.fn();
    controller.register("m", "b", { defaultCombo: "Ctrl+KeyK", labelKey: "m.b", action });
    target.dispatchEvent(keydown({ code: "KeyK" }));
    expect(action).not.toHaveBeenCalled();
    target.dispatchEvent(keydown({ code: "KeyK", ctrlKey: true }));
    expect(action).toHaveBeenCalledTimes(1);
  });

  it("prevents the default action of a claimed keystroke only", () => {
    const { target, controller } = setup();
    controller.register("m", "b", { defaultCombo: "KeyK", labelKey: "m.b", action: () => {} });
    const claimed = keydown({ code: "KeyK" });
    const unclaimed = keydown({ code: "KeyJ" });
    target.dispatchEvent(claimed);
    target.dispatchEvent(unclaimed);
    expect(claimed.defaultPrevented).toBe(true);
    expect(unclaimed.defaultPrevented).toBe(false);
  });

  it("a rebind changes which combo triggers the action without re-registering", () => {
    const { target, controller } = setup();
    const action = vi.fn();
    controller.register("m", "b", { defaultCombo: "KeyK", labelKey: "m.b", action });
    controller.rebind("m", "b", "KeyL");
    target.dispatchEvent(keydown({ code: "KeyK" }));
    expect(action).not.toHaveBeenCalled();
    target.dispatchEvent(keydown({ code: "KeyL" }));
    expect(action).toHaveBeenCalledTimes(1);
  });

  it("an unbound binding fires on nothing", () => {
    const { target, controller } = setup();
    const action = vi.fn();
    controller.register("m", "b", { defaultCombo: "KeyK", labelKey: "m.b", action });
    controller.rebind("m", "b", null);
    target.dispatchEvent(keydown({ code: "KeyK" }));
    expect(action).not.toHaveBeenCalled();
    expect(controller.allBindings()[0].combo).toBeNull();
  });

  it("two modules on one combo: the first registered fires, the second reports the conflict", () => {
    const { target, controller } = setup();
    const first = vi.fn();
    const second = vi.fn();
    const a = controller.register("a", "x", { defaultCombo: "KeyK", labelKey: "a.x", action: first });
    const b = controller.register("b", "x", { defaultCombo: "KeyK", labelKey: "b.x", action: second });
    expect(a.conflictsWith).toBeNull();
    expect(b.conflictsWith).toBe("a");
    target.dispatchEvent(keydown({ code: "KeyK" }));
    expect(first).toHaveBeenCalledTimes(1);
    expect(second).not.toHaveBeenCalled();
  });

  it("ignores keystrokes aimed at an editable element", () => {
    class FakeInput {
      /** Whether the element is `contenteditable`. */
      isContentEditable = false;
      /** The element's tag name. */
      tagName = "INPUT";
    }
    vi.stubGlobal("HTMLElement", FakeInput);
    const { target, controller } = setup();
    const action = vi.fn();
    controller.register("m", "b", { defaultCombo: "KeyK", labelKey: "m.b", action });
    target.dispatchEvent(keydown({ code: "KeyK", target: new FakeInput() }));
    expect(action).not.toHaveBeenCalled();
    target.dispatchEvent(keydown({ code: "KeyK" }));
    expect(action).toHaveBeenCalledTimes(1);
  });

  it("does not re-fire an action on key auto-repeat", () => {
    const { target, controller } = setup();
    const action = vi.fn();
    controller.register("m", "b", { defaultCombo: "KeyK", labelKey: "m.b", action });
    target.dispatchEvent(keydown({ code: "KeyK" }));
    target.dispatchEvent(keydown({ code: "KeyK", repeat: true }));
    expect(action).toHaveBeenCalledTimes(1);
  });

  it("stops dispatch to later core:keydown listeners once a binding claims the keystroke", async () => {
    const { target, hooks, controller } = setup();
    const later = vi.fn();
    hooks.on("core:keydown", later);
    controller.register("m", "b", { defaultCombo: "KeyK", labelKey: "m.b", action: () => {} });
    target.dispatchEvent(keydown({ code: "KeyK" }));
    await Promise.resolve();
    expect(later).not.toHaveBeenCalled();
    target.dispatchEvent(keydown({ code: "KeyJ" }));
    await Promise.resolve();
    expect(later).toHaveBeenCalledTimes(1);
  });

  it("dispose stops the window listener", () => {
    const { target, controller } = setup();
    const action = vi.fn();
    controller.register("m", "b", { defaultCombo: "KeyK", labelKey: "m.b", action });
    controller.dispose();
    target.dispatchEvent(keydown({ code: "KeyK" }));
    expect(action).not.toHaveBeenCalled();
  });
});

describe("KeybindingsController bookkeeping", () => {
  it("removeModule and dispose drop bindings and notify subscribers", () => {
    const { controller } = setup();
    const changed = vi.fn();
    controller.subscribe(changed);
    const reg = controller.register("m", "a", { defaultCombo: "KeyA", labelKey: "m.a", action: () => {} });
    controller.register("m", "b", { defaultCombo: "KeyB", labelKey: "m.b", action: () => {} });
    controller.register("n", "c", { defaultCombo: "KeyC", labelKey: "n.c", action: () => {} });
    reg.dispose();
    expect(controller.bindingsFor("m").map((b) => b.bindingId)).toEqual(["b"]);
    controller.removeModule("m");
    expect(controller.allBindings().map((b) => b.moduleId)).toEqual(["n"]);
    expect(changed).toHaveBeenCalledTimes(5);
  });

  it("a stale dispose does not remove a replacement registered under the same ids", () => {
    const { controller } = setup();
    const old = controller.register("m", "a", { defaultCombo: "KeyA", labelKey: "m.a", action: () => {} });
    controller.register("m", "a", { defaultCombo: "KeyB", labelKey: "m.a", action: () => {} });
    old.dispose();
    expect(controller.bindingsFor("m")).toHaveLength(1);
  });

  it("round-trips overrides and applies a saved override to a binding registered later", () => {
    const { target, controller } = setup();
    controller.loadOverrides({ "m:b": "Ctrl+KeyL", "m:gone": null });
    const action = vi.fn();
    controller.register("m", "b", { defaultCombo: "KeyK", labelKey: "m.b", action });
    target.dispatchEvent(keydown({ code: "KeyK" }));
    expect(action).not.toHaveBeenCalled();
    target.dispatchEvent(keydown({ code: "KeyL", ctrlKey: true }));
    expect(action).toHaveBeenCalledTimes(1);
    expect(controller.serializeOverrides()).toEqual({ "m:b": "ctrl+keyl", "m:gone": null });
  });

  it("rebinding to the default clears the override", () => {
    const { controller } = setup();
    controller.register("m", "b", { defaultCombo: "KeyK", labelKey: "m.b", action: () => {} });
    controller.rebind("m", "b", "KeyL");
    expect(controller.serializeOverrides()).toEqual({ "m:b": "keyl" });
    controller.rebind("m", "b", "KeyK");
    expect(controller.serializeOverrides()).toEqual({});
  });
});
