import { test, expect } from "vitest";
import type { Logger } from "@shadowcat/core";
import { PanelsBridge, type PanelsApi } from "./panelsBridge";

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

function capturingLogger(): Logger & { warnings: string[] } {
  const warnings: string[] = [];
  return {
    warnings,
    debug: () => {},
    warn: (msg) => warnings.push(msg),
    error: () => {},
  };
}

test("PanelsBridge.open before bind warns once through the injected logger, not console", () => {
  const logger = capturingLogger();
  const bridge = new PanelsBridge(logger);

  expect(() => bridge.open("a")).not.toThrow();
  expect(() => bridge.close("a")).not.toThrow();
  expect(() => bridge.focus("a")).not.toThrow();
  expect(() => bridge.toggle("a")).not.toThrow();

  expect(logger.warnings).toHaveLength(1); // warns once, not per call
});

test("PanelsBridge delegates to the bound implementation", () => {
  const logger = capturingLogger();
  const bridge = new PanelsBridge(logger);
  const impl = fakeImpl();

  bridge.bind(impl);
  bridge.open("a");
  bridge.close("b");
  bridge.focus("c");
  bridge.toggle("d");

  expect(impl.calls).toEqual(["open:a", "close:b", "focus:c", "toggle:d"]);
  expect(logger.warnings).toHaveLength(0);
});
