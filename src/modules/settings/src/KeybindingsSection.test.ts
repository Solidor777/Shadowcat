import { describe, it, expect, afterEach } from "vitest";
import { tick } from "svelte";
import { render, screen, fireEvent } from "@testing-library/svelte";
import { HookBus, KeybindingsController, silentLogger } from "@shadowcat/core";
import { setAppContextForTest } from "@shadowcat/ui-kit/test";
import KeybindingsSection from "./KeybindingsSection.svelte";

describe("KeybindingsSection", () => {
  let controller: KeybindingsController | undefined;

  /** Renders the section over a real controller with one binding on `KeyK`. */
  function renderSection(): { fired: { count: number } } {
    const target = new EventTarget();
    controller = new KeybindingsController(new HookBus(silentLogger), target);
    const fired = { count: 0 };
    controller.register("example", "ping", {
      defaultCombo: "KeyK",
      labelKey: "example.ping",
      action: () => {
        fired.count += 1;
      },
    });
    const keybindings = {
      register: () => ({ dispose() {}, conflictsWith: null }),
      rebind: (m: string, b: string, c: string | null) => controller!.rebind(m, b, c),
      bindingsFor: (m: string) => controller!.bindingsFor(m),
      allBindings: () => controller!.allBindings(),
      subscribe: (l: () => void) => controller!.subscribe(l),
    };
    render(KeybindingsSection, { props: { keybindings }, context: setAppContextForTest({}) });
    return { fired };
  }

  /** Dispatches a keydown on `window` the way the browser does for a real keystroke. */
  function press(code: string, init: KeyboardEventInit = {}): KeyboardEvent {
    const event = new KeyboardEvent("keydown", { code, key: code, bubbles: true, cancelable: true, ...init });
    window.dispatchEvent(event);
    return event;
  }

  afterEach(() => {
    controller?.dispose();
    controller = undefined;
  });

  it("lists every registered binding with its label, owner and current combo", () => {
    renderSection();
    expect(screen.getByText("example.ping")).toBeTruthy();
    expect(screen.getByText("example")).toBeTruthy();
    expect(screen.getByText("K")).toBeTruthy();
  });

  it("a capture keypress rebinds to the normalized combo, with modifiers", async () => {
    renderSection();
    await fireEvent.click(screen.getByRole("button", { name: "settings.keybindings.rebind" }));
    expect(screen.getByText("settings.keybindings.prompt")).toBeTruthy();
    press("ShiftLeft", { shiftKey: true });
    expect(controller!.allBindings()[0].combo).toBe("keyk");
    press("KeyL", { ctrlKey: true });
    expect(controller!.allBindings()[0].combo).toBe("ctrl+keyl");
    await tick();
    expect(screen.queryByText("settings.keybindings.prompt")).toBeNull();
  });

  it("clearing a binding unbinds it and its action no longer fires on the old default", async () => {
    const { fired } = renderSection();
    await fireEvent.click(screen.getByRole("button", { name: "settings.keybindings.clear" }));
    expect(controller!.allBindings()[0].combo).toBeNull();
    expect(screen.getByText("settings.keybindings.unbound")).toBeTruthy();
    expect(fired.count).toBe(0);
  });

  it("Escape exits an armed capture without rebinding", async () => {
    renderSection();
    await fireEvent.click(screen.getByRole("button", { name: "settings.keybindings.rebind" }));
    press("Escape", { key: "Escape" });
    await tick();
    expect(screen.queryByText("settings.keybindings.prompt")).toBeNull();
    expect(controller!.allBindings()[0].combo).toBe("keyk");
    // Capture is over: a following keystroke is not recorded.
    press("KeyL");
    expect(controller!.allBindings()[0].combo).toBe("keyk");
  });

  it("the Cancel button exits an armed capture without rebinding", async () => {
    renderSection();
    await fireEvent.click(screen.getByRole("button", { name: "settings.keybindings.rebind" }));
    await fireEvent.click(screen.getByRole("button", { name: "settings.keybindings.cancel" }));
    expect(screen.queryByText("settings.keybindings.prompt")).toBeNull();
    expect(controller!.allBindings()[0].combo).toBe("keyk");
  });
});
