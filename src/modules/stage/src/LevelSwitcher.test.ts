// @vitest-environment jsdom
import { test, expect, vi } from "vitest";
import { render, fireEvent } from "@testing-library/svelte";
import LevelSwitcher from "./LevelSwitcher.svelte";
import { setAppContextForTest } from "@shadowcat/ui-kit/test";
import type { SceneLevel } from "@shadowcat/core";

const levels: SceneLevel[] = [
  { id: "l1", name: "Ground", bottom: 0, top: 10, background: null },
  { id: "l2", name: "Upper", bottom: 10, top: 20, background: null },
];

test("renders nothing for an empty levels array", () => {
  const context = setAppContextForTest();
  const { container } = render(LevelSwitcher, { props: { levels: [], active: null, onSelect: () => {} }, context });
  expect(container.querySelector('[data-testid="level-switcher"]')).toBeNull();
});

test("renders one button per level with the active one aria-pressed", () => {
  const context = setAppContextForTest();
  const { container } = render(LevelSwitcher, { props: { levels, active: "l2", onSelect: () => {} }, context });
  const l1 = container.querySelector('[data-testid="level-l1"]') as HTMLButtonElement;
  const l2 = container.querySelector('[data-testid="level-l2"]') as HTMLButtonElement;
  expect(l1).toBeTruthy();
  expect(l2).toBeTruthy();
  expect(l1.textContent?.trim()).toBe("Ground");
  expect(l2.textContent?.trim()).toBe("Upper");
  expect(l1.getAttribute("aria-pressed")).toBe("false");
  expect(l2.getAttribute("aria-pressed")).toBe("true");
});

test("clicking a button calls onSelect with that level's id", async () => {
  const onSelect = vi.fn();
  const context = setAppContextForTest();
  const { container } = render(LevelSwitcher, { props: { levels, active: "l1", onSelect }, context });
  const l2 = container.querySelector('[data-testid="level-l2"]') as HTMLButtonElement;
  await fireEvent.click(l2);
  expect(onSelect).toHaveBeenCalledWith("l2");
});
