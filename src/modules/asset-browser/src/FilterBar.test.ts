import { render, screen, fireEvent } from "@testing-library/svelte";
import { test, expect, vi } from "vitest";
import { setAppContextForTest } from "@shadowcat/ui-kit/test";
import FilterBar from "./FilterBar.svelte";
import type { FilterState } from "./filterState";

const BASE: FilterState = { name: "", nameIsRegex: false, tags: [], kind: undefined, sort: "created" };

test("typing a name emits the updated filter state", async () => {
  const onChange = vi.fn();
  render(FilterBar, { props: { filter: BASE, onChange }, context: setAppContextForTest() });
  await fireEvent.input(screen.getByTestId("filter-name"), { target: { value: "drag" } });
  expect(onChange).toHaveBeenCalledWith(expect.objectContaining({ name: "drag" }));
});

test("the regex toggle flips nameIsRegex", async () => {
  const onChange = vi.fn();
  render(FilterBar, { props: { filter: BASE, onChange }, context: setAppContextForTest() });
  await fireEvent.click(screen.getByTestId("filter-regex-toggle"));
  expect(onChange).toHaveBeenCalledWith(expect.objectContaining({ nameIsRegex: true }));
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
