import { expect, test, vi } from "vitest";
import { silentLogger, consoleLogger } from "./logger";

test("silentLogger swallows all levels", () => {
  expect(() => {
    silentLogger.debug("d");
    silentLogger.warn("w");
    silentLogger.error("e");
  }).not.toThrow();
});

test("consoleLogger routes warn/error to console methods", () => {
  const warn = vi.spyOn(console, "warn").mockImplementation(() => {});
  consoleLogger().warn("hello", { a: 1 });
  expect(warn).toHaveBeenCalledWith("[shadowcat] hello", { a: 1 });
  warn.mockRestore();
});
