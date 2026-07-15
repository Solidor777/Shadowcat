import { test, expect, afterEach } from "vitest";
import { render, screen, cleanup } from "@testing-library/svelte";
import { setAppContextForTest } from "@shadowcat/ui-kit/test";
import Presence from "./Presence.svelte";

afterEach(() => cleanup());

test("renders a badge per world member (available to every role)", () => {
  const members = new Map([
    ["u1", "Ada"],
    ["u2", "Bo"],
  ]);
  render(Presence, { context: setAppContextForTest({ role: "player", members }) });

  const roster = screen.getByTestId("presence");
  expect(roster.getAttribute("role")).toBe("group");

  const a = screen.getByTestId("presence-u1");
  expect(a.getAttribute("title")).toBe("Ada");
  expect(a.getAttribute("aria-label")).toBe("Ada");
  expect(a.textContent?.trim()).toBe("A");

  expect(screen.getByTestId("presence-u2").getAttribute("title")).toBe("Bo");
});

test("renders an empty roster group when there are no members", () => {
  render(Presence, { context: setAppContextForTest({ members: new Map() }) });
  expect(screen.getByTestId("presence").children.length).toBe(0);
});
