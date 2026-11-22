import { test, expect, afterEach } from "vitest";
import { render, screen, cleanup } from "@testing-library/svelte";
import { tick } from "svelte";
import { SvelteMap } from "svelte/reactivity";
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

test("a member with no presence entry is unknown: no state class, no state suffix", () => {
  const members = new Map([["u1", "Ada"]]);
  render(Presence, { context: setAppContextForTest({ members }) });
  const dot = screen.getByTestId("presence-dot-u1");
  expect(dot.classList.contains("online")).toBe(false);
  expect(dot.classList.contains("offline")).toBe(false);
  expect(screen.getByTestId("presence-u1").getAttribute("aria-label")).toBe("Ada");
});

test("the dot follows live presence transitions and the label names the state", async () => {
  const members = new Map([["u1", "Ada"]]);
  const presence = new SvelteMap<string, boolean>();
  render(Presence, { context: setAppContextForTest({ members, presence }) });

  presence.set("u1", true);
  await tick();
  expect(screen.getByTestId("presence-dot-u1").classList.contains("online")).toBe(true);
  expect(screen.getByTestId("presence-u1").getAttribute("aria-label")).toBe("Ada (topbar.online)");

  presence.set("u1", false);
  await tick();
  const dot = screen.getByTestId("presence-dot-u1");
  expect(dot.classList.contains("online")).toBe(false);
  expect(dot.classList.contains("offline")).toBe(true);
  expect(screen.getByTestId("presence-u1").getAttribute("aria-label")).toBe("Ada (topbar.offline)");
});

test("renders an empty roster group when there are no members", () => {
  render(Presence, { context: setAppContextForTest({ members: new Map() }) });
  expect(screen.getByTestId("presence").children.length).toBe(0);
});

test("tracks in-place mutations of a reactive SvelteMap (join + leave)", async () => {
  const members = new SvelteMap([
    ["u1", "Ada"],
    ["u2", "Bo"],
  ]);
  render(Presence, { context: setAppContextForTest({ role: "player", members }) });

  expect(screen.getByTestId("presence-u1")).toBeTruthy();
  expect(screen.getByTestId("presence-u2")).toBeTruthy();

  // Mutate the SAME Map instance in place post-render: a `roster` computed once
  // at init (rather than via `$derived` over `ctx.members`) would never see this.
  members.set("u3", "Cy");
  members.delete("u1");
  await tick();

  expect(screen.queryByTestId("presence-u1")).toBeNull();
  expect(screen.getByTestId("presence-u2")).toBeTruthy();
  expect(screen.getByTestId("presence-u3")).toBeTruthy();
  expect(screen.getByTestId("presence-u3").textContent?.trim()).toBe("C");
});
