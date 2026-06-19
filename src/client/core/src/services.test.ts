import { expect, test } from "vitest";
import { ServiceRegistry } from "./services";

test("provide then get returns the impl and version", () => {
  const r = new ServiceRegistry();
  r.provide("dice", { roll: () => 4 }, { module: "core", version: "1.0.0" });
  expect(r.has("dice")).toBe(true);
  expect(r.get<{ roll: () => number }>("dice")!.roll()).toBe(4);
  expect(r.versionOf("dice")).toBe("1.0.0");
});

test("duplicate provide is an error", () => {
  const r = new ServiceRegistry();
  r.provide("x", {}, { version: "1.0.0" });
  expect(() => r.provide("x", {}, { version: "1.0.0" })).toThrow();
});

test("removeModule drops that module's services", () => {
  const r = new ServiceRegistry();
  r.provide("x", {}, { module: "m1", version: "1.0.0" });
  r.removeModule("m1");
  expect(r.has("x")).toBe(false);
});
