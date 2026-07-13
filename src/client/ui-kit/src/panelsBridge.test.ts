import { test, expect, vi, afterEach } from "vitest";
import { PanelsBridge, type PanelsApi } from "./panelsBridge";

afterEach(() => {
  vi.restoreAllMocks();
});

function fakeImpl(): PanelsApi & { calls: string[] } {
  const calls: string[] = [];
  return {
    calls,
    open: (id) => calls.push(`open:${id}`),
    close: (id) => calls.push(`close:${id}`),
    focus: (id) => calls.push(`focus:${id}`),
    toggle: (id) => calls.push(`toggle:${id}`),
  };
}

test("PanelsBridge.open before bind warns once and no-throws", () => {
  const warn = vi.spyOn(console, "warn").mockImplementation(() => {});
  const bridge = new PanelsBridge();

  expect(() => bridge.open("a")).not.toThrow();
  expect(() => bridge.close("a")).not.toThrow();
  expect(() => bridge.focus("a")).not.toThrow();
  expect(() => bridge.toggle("a")).not.toThrow();

  expect(warn).toHaveBeenCalledTimes(1); // warns once, not per call
});

test("PanelsBridge delegates to the bound implementation", () => {
  const warn = vi.spyOn(console, "warn").mockImplementation(() => {});
  const bridge = new PanelsBridge();
  const impl = fakeImpl();

  bridge.bind(impl);
  bridge.open("a");
  bridge.close("b");
  bridge.focus("c");
  bridge.toggle("d");

  expect(impl.calls).toEqual(["open:a", "close:b", "focus:c", "toggle:d"]);
  expect(warn).not.toHaveBeenCalled();
});
