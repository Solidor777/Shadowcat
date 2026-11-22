import { render, screen, fireEvent } from "@testing-library/svelte";
import { test, expect, vi } from "vitest";
import { setAppContextForTest } from "@shadowcat/ui-kit/test";
import FilterBar from "./FilterBar.svelte";
import type { FilterState } from "./filterState";

const BASE: FilterState = { query: "", queryIsRegex: false, tags: [], kind: undefined, sort: "created", favoritesOnly: false };

test("the favorites toggle flips favoritesOnly", async () => {
  const onChange = vi.fn();
  render(FilterBar, { props: { filter: BASE, onChange }, context: setAppContextForTest() });
  await fireEvent.click(screen.getByTestId("filter-favorites"));
  expect(onChange).toHaveBeenCalledWith(expect.objectContaining({ favoritesOnly: true }));
});

test("the view-mode buttons report the chosen layout and mark the active one", async () => {
  const onViewModeChange = vi.fn();
  render(FilterBar, {
    props: { filter: BASE, onChange: vi.fn(), viewMode: "list", onViewModeChange },
    context: setAppContextForTest(),
  });
  expect(screen.getByTestId("view-mode-list").getAttribute("aria-pressed")).toBe("true");
  expect(screen.getByTestId("view-mode-grid").getAttribute("aria-pressed")).toBe("false");
  await fireEvent.click(screen.getByTestId("view-mode-grid"));
  expect(onViewModeChange).toHaveBeenCalledWith("grid");
});

test("typing a query emits the updated filter state", async () => {
  const onChange = vi.fn();
  render(FilterBar, { props: { filter: BASE, onChange }, context: setAppContextForTest() });
  await fireEvent.input(screen.getByTestId("filter-query"), { target: { value: "drag" } });
  expect(onChange).toHaveBeenCalledWith(expect.objectContaining({ query: "drag" }));
});

test("the regex toggle flips queryIsRegex", async () => {
  const onChange = vi.fn();
  render(FilterBar, { props: { filter: BASE, onChange }, context: setAppContextForTest() });
  await fireEvent.click(screen.getByTestId("filter-regex-toggle"));
  expect(onChange).toHaveBeenCalledWith(expect.objectContaining({ queryIsRegex: true }));
});

test("committing the tag input adds a chip; its remove button drops it", async () => {
  const onChange = vi.fn();
  render(FilterBar, {
    props: { filter: { ...BASE, tags: ["map"] }, onChange },
    context: setAppContextForTest(),
  });
  const input = screen.getByTestId("filter-tag-input");
  await fireEvent.input(input, { target: { value: "hero" } });
  await fireEvent.keyDown(input, { key: "Enter" });
  expect(onChange).toHaveBeenCalledWith(expect.objectContaining({ tags: ["map", "hero"] }));

  await fireEvent.click(screen.getByTestId("filter-tag-remove-map"));
  expect(onChange).toHaveBeenCalledWith(expect.objectContaining({ tags: [] }));
});

test("kind and sort selects emit their values", async () => {
  const onChange = vi.fn();
  render(FilterBar, { props: { filter: BASE, onChange }, context: setAppContextForTest() });
  await fireEvent.change(screen.getByTestId("filter-kind"), { target: { value: "image" } });
  expect(onChange).toHaveBeenCalledWith(expect.objectContaining({ kind: "image" }));
  await fireEvent.change(screen.getByTestId("filter-sort"), { target: { value: "size" } });
  expect(onChange).toHaveBeenCalledWith(expect.objectContaining({ sort: "size" }));
});
