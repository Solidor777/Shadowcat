// @vitest-environment node
import { test, expect } from "vitest";
import { noticeOncePerGesture } from "./gesture-notice";

test("the first notice of a gesture raises and every later one is dropped", () => {
  const gesture = {};
  const shown: string[] = [];
  expect(noticeOncePerGesture(gesture, () => shown.push("a"))).toBe(true);
  expect(noticeOncePerGesture(gesture, () => shown.push("b"))).toBe(false);
  expect(shown).toEqual(["a"]);
});

test("a different gesture raises its own notice", () => {
  const shown: string[] = [];
  noticeOncePerGesture({}, () => shown.push("a"));
  noticeOncePerGesture({}, () => shown.push("b"));
  expect(shown).toEqual(["a", "b"]);
});

test("gestureless notices never suppress each other or a gesture's", () => {
  const gesture = {};
  const shown: string[] = [];
  noticeOncePerGesture(undefined, () => shown.push("x"));
  noticeOncePerGesture(undefined, () => shown.push("y"));
  noticeOncePerGesture(gesture, () => shown.push("g"));
  noticeOncePerGesture(undefined, () => shown.push("z"));
  noticeOncePerGesture(gesture, () => shown.push("dropped"));
  expect(shown).toEqual(["x", "y", "g", "z"]);
});
