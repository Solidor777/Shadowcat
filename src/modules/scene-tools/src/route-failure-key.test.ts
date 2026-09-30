// @vitest-environment node
// Pure function under test: no DOM is touched.
import { test, expect } from "vitest";
import { NotConnectedError, RequestTimeoutError, PathfindError } from "@shadowcat/core";
import { routeFailureKey } from "./controller.svelte";

// The rows that carry the same TEXT as a typed rejection but are plain `Error`s are the ones that
// prove the key is chosen by type: an implementation matching `.message` maps them to a specific
// notice instead of the generic one.
const CASES: { name: string; error: () => unknown; key: string }[] = [
  { name: "NotConnectedError", error: () => new NotConnectedError(), key: "tools.moveDisconnected" },
  { name: "NotConnectedError with another message", error: () => new NotConnectedError("x"), key: "tools.moveDisconnected" },
  { name: "RequestTimeoutError", error: () => new RequestTimeoutError("pathfind request timeout"), key: "tools.moveTimeout" },
  { name: "RequestTimeoutError with another message", error: () => new RequestTimeoutError("x"), key: "tools.moveTimeout" },
  { name: "PathfindError unreachable", error: () => new PathfindError("unreachable", "unreachable"), key: "tools.moveNoRoute" },
  { name: "PathfindError invalid with an unrelated message", error: () => new PathfindError("boom", "invalid"), key: "tools.moveInvalid" },
  { name: "PathfindError exceeded with the text of unreachable", error: () => new PathfindError("unreachable", "exceeded"), key: "tools.moveTooComplex" },
  { name: "plain Error carrying the not-connected text", error: () => new Error("not connected"), key: "tools.moveRejected" },
  { name: "plain Error carrying the timeout text", error: () => new Error("pathfind request timeout"), key: "tools.moveRejected" },
  { name: "plain Error carrying the invalid text", error: () => new Error("invalid request"), key: "tools.moveRejected" },
  { name: "plain Error carrying the exceeded text", error: () => new Error("search exceeded"), key: "tools.moveRejected" },
  { name: "plain Error carrying the unreachable text", error: () => new Error("unreachable"), key: "tools.moveRejected" },
  { name: "a non-Error value", error: () => "boom", key: "tools.moveRejected" },
];

test.each(CASES)("routeFailureKey maps $name to $key", ({ error, key }) => {
  expect(routeFailureKey(error())).toBe(key);
});
