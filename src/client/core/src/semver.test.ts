import { expect, test } from "vitest";
import { satisfies } from "./semver";

test("wildcard matches anything", () => {
  expect(satisfies("9.9.9", "*")).toBe(true);
});
test("exact match", () => {
  expect(satisfies("1.2.3", "1.2.3")).toBe(true);
  expect(satisfies("1.2.4", "1.2.3")).toBe(false);
});
test("caret allows same-major, >= patch/minor", () => {
  expect(satisfies("1.4.0", "^1.2.3")).toBe(true);
  expect(satisfies("1.2.2", "^1.2.3")).toBe(false);
  expect(satisfies("2.0.0", "^1.2.3")).toBe(false);
});
test("tilde allows same-major.minor, >= patch", () => {
  expect(satisfies("1.2.9", "~1.2.3")).toBe(true);
  expect(satisfies("1.3.0", "~1.2.3")).toBe(false);
});
test("invalid version throws", () => {
  expect(() => satisfies("not-a-version", "*")).toThrow();
});
