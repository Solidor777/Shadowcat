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

test("a member with a colour renders that badge background and a legible glyph colour", () => {
  const members = new Map([
    ["u1", "Ada"],
    ["u2", "Bo"],
  ]);
  const memberProfiles = new Map([
    ["u1", { colour: "#ffee00", pronouns: null, avatar: null }],
    ["u2", { colour: "#101040", pronouns: null, avatar: null }],
  ]);
  render(Presence, { context: setAppContextForTest({ members, memberProfiles }) });
  const light = screen.getByTestId("presence-u1");
  expect(light.style.background).toBe("rgb(255, 238, 0)");
  expect(light.style.color).toBe("rgb(0, 0, 0)");
  const dark = screen.getByTestId("presence-u2");
  expect(dark.style.background).toBe("rgb(16, 16, 64)");
  expect(dark.style.color).toBe("rgb(255, 255, 255)");
});

test("a member with an avatar renders the image instead of the initial glyph", () => {
  const members = new Map([
    ["u1", "Ada"],
    ["u2", "Bo"],
  ]);
  const memberProfiles = new Map([["u1", { colour: null, pronouns: null, avatar: "/api/a.png" }]]);
  render(Presence, { context: setAppContextForTest({ members, memberProfiles }) });
  const img = screen.getByTestId("presence-avatar-u1");
  expect(img.getAttribute("src")).toBe("/api/a.png");
  expect(screen.getByTestId("presence-u1").textContent?.trim()).toBe("");
  // A member with no profile keeps the initial glyph and no inline colour.
  expect(screen.queryByTestId("presence-avatar-u2")).toBeNull();
  expect(screen.getByTestId("presence-u2").textContent?.trim()).toBe("B");
  expect(screen.getByTestId("presence-u2").style.background).toBe("");
});

test("a member's pronouns follow the name in the badge title and label, ahead of the presence state", async () => {
  const members = new Map([
    ["u1", "Ada"],
    ["u2", "Bo"],
  ]);
  const presence = new SvelteMap<string, boolean>([["u1", true]]);
  const memberProfiles = new SvelteMap([["u1", { colour: null, pronouns: "she/her", avatar: null }]]);
  render(Presence, { context: setAppContextForTest({ members, presence, memberProfiles }) });
  const badge = screen.getByTestId("presence-u1");
  expect(badge.getAttribute("title")).toBe("Ada (she/her)");
  expect(badge.getAttribute("aria-label")).toBe("Ada (she/her) (topbar.online)");
  // A member with no pronouns keeps the bare name.
  expect(screen.getByTestId("presence-u2").getAttribute("title")).toBe("Bo");

  memberProfiles.set("u1", { colour: null, pronouns: "they/them", avatar: null });
  await tick();
  expect(screen.getByTestId("presence-u1").getAttribute("title")).toBe("Ada (they/them)");
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
