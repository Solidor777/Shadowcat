import { test, expect, afterEach, beforeEach } from "vitest";
import { render, screen, fireEvent, cleanup } from "@testing-library/svelte";
import { notifications } from "./notifications.svelte";
import NotificationHost from "./NotificationHost.svelte";

beforeEach(() => {
  // Drain any notifications left by a prior test — `notifications` is a module-level singleton.
  for (const n of [...notifications.items]) notifications.dismiss(n.id);
});
afterEach(() => cleanup());

test("renders one entry per active notification, showing its message text", () => {
  notifications.push("warning", "Some targets were skipped.");
  notifications.push("info", "Saved.");
  render(NotificationHost);
  expect(screen.getByText("Some targets were skipped.")).toBeTruthy();
  expect(screen.getByText("Saved.")).toBeTruthy();
});

test("clicking an item's dismiss button calls notifications.dismiss with that item's id", async () => {
  const id = notifications.push("warning", "Some targets were skipped.");
  render(NotificationHost);
  const dismissButtons = screen.getAllByRole("button");
  await fireEvent.click(dismissButtons[0]);
  expect(notifications.items.find((n) => n.id === id)).toBeUndefined();
  expect(screen.queryByText("Some targets were skipped.")).toBeNull();
});

test("the host container carries aria-live=\"polite\"", () => {
  notifications.push("info", "Saved.");
  const { container } = render(NotificationHost);
  const host = container.querySelector("[aria-live]");
  expect(host?.getAttribute("aria-live")).toBe("polite");
});
