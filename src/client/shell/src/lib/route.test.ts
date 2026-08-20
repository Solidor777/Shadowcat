import { test, expect, afterEach } from "vitest";
import { parseHash, navigate, currentRoute } from "./route.svelte";

afterEach(() => {
  location.hash = "";
  window.dispatchEvent(new HashChangeEvent("hashchange"));
});

test("parses the known routes", () => {
  expect(parseHash("#/login")).toEqual({ name: "login" });
  expect(parseHash("#/setup")).toEqual({ name: "setup" });
  expect(parseHash("#/worlds")).toEqual({ name: "worlds" });
  expect(parseHash("#/world/abc-123")).toEqual({ name: "world", id: "abc-123" });
  expect(parseHash("")).toEqual({ name: "unknown" });
  expect(parseHash("#/nonsense")).toEqual({ name: "unknown" });
});

test("navigate() updates currentRoute() synchronously, with no hashchange event needed", () => {
  location.hash = "#/world/abc";
  window.dispatchEvent(new HashChangeEvent("hashchange"));
  expect(currentRoute()).toEqual({ name: "world", id: "abc" });

  navigate({ name: "login" });
  // No manual hashchange dispatch here — jsdom never fires one on a bare `location.hash`
  // assignment, and a real browser's own dispatch is asynchronous either way. If navigate()
  // relied solely on that event, currentRoute() would still read the stale "world" route here.
  expect(currentRoute()).toEqual({ name: "login" });
  expect(location.hash).toBe("#/login");
});
