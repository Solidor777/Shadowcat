import { test, expect } from "vitest";
import type { Logger, PanelMeta } from "@shadowcat/core";
import { PanelsBridge, type PanelsApi, type PanelsChipsView } from "./panelsBridge";

function fakeImpl(): PanelsApi & PanelsChipsView & { calls: string[] } {
  const calls: string[] = [];
  const meta = new Map<string, PanelMeta>([["a", { icon: "a", labelKey: "a.tab" }]]);
  return {
    calls,
    open: (id) => calls.push(`open:${id}`),
    close: (id) => calls.push(`close:${id}`),
    focus: (id) => calls.push(`focus:${id}`),
    toggle: (id) => calls.push(`toggle:${id}`),
    minimized: ["a"],
    metaMap: meta,
    restore: (id) => calls.push(`restore:${id}`),
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
  expect(() => bridge.restore("a")).not.toThrow();

  expect(logger.warnings).toHaveLength(1); // warns once, not per call
});

test("PanelsBridge.minimized/metaMap are empty before bind", () => {
  const bridge = new PanelsBridge(capturingLogger());
  expect(bridge.minimized).toEqual([]);
  expect(bridge.metaMap.size).toBe(0);
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
  bridge.restore("e");

  expect(impl.calls).toEqual(["open:a", "close:b", "focus:c", "toggle:d", "restore:e"]);
  expect(logger.warnings).toHaveLength(0);
});

test("PanelsBridge.minimized/metaMap read through to the bound implementation", () => {
  const bridge = new PanelsBridge(capturingLogger());
  const impl = fakeImpl();
  bridge.bind(impl);

  expect(bridge.minimized).toEqual(["a"]);
  expect(bridge.metaMap.get("a")).toEqual({ icon: "a", labelKey: "a.tab" });
});
