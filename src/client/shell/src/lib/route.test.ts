import { test, expect } from "vitest";
import { parseHash } from "./route.svelte";

test("parses the known routes", () => {
  expect(parseHash("#/login")).toEqual({ name: "login" });
  expect(parseHash("#/setup")).toEqual({ name: "setup" });
  expect(parseHash("#/worlds")).toEqual({ name: "worlds" });
  expect(parseHash("#/world/abc-123")).toEqual({ name: "world", id: "abc-123" });
  expect(parseHash("")).toEqual({ name: "unknown" });
  expect(parseHash("#/nonsense")).toEqual({ name: "unknown" });
});
