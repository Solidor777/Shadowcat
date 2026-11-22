import { describe, it, expect, vi } from "vitest";
import { render, screen, fireEvent } from "@testing-library/svelte";
import TagChips from "./TagChips.svelte";

/** Renders the chips with a change spy under the `tag` test id prefix. */
function setup(tags: string[]) {
  const onChange = vi.fn();
  render(TagChips, {
    props: {
      tags,
      onChange,
      placeholder: "Add tag",
      removeLabel: (tag: string) => `Remove ${tag}`,
      testIdPrefix: "tag",
    },
  });
  return onChange;
}

describe("TagChips", () => {
  it("removes a tag by committing the list without it", async () => {
    const onChange = setup(["a", "b"]);
    await fireEvent.click(screen.getByTestId("tag-remove-a"));
    expect(onChange).toHaveBeenCalledWith(["b"]);
  });

  it("names each remove button through removeLabel", () => {
    setup(["a"]);
    expect(screen.getByTestId("tag-remove-a").getAttribute("aria-label")).toBe("Remove a");
  });

  it("commits a trimmed new tag on Enter and clears the input", async () => {
    const onChange = setup(["a"]);
    const input = screen.getByTestId("tag-input") as HTMLInputElement;
    await fireEvent.input(input, { target: { value: "  b  " } });
    await fireEvent.keyDown(input, { key: "Enter" });
    expect(onChange).toHaveBeenCalledWith(["a", "b"]);
    expect(input.value).toBe("");
  });

  it("refuses an empty or duplicate draft", async () => {
    const onChange = setup(["a"]);
    const input = screen.getByTestId("tag-input");
    await fireEvent.keyDown(input, { key: "Enter" });
    await fireEvent.input(input, { target: { value: "a" } });
    await fireEvent.keyDown(input, { key: "Enter" });
    expect(onChange).not.toHaveBeenCalled();
  });
});
