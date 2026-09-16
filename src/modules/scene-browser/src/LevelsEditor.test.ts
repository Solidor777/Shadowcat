import { describe, it, expect, vi } from "vitest";
import { render, fireEvent } from "@testing-library/svelte";
import LevelsEditor from "./LevelsEditor.svelte";
import { setAppContextForTest } from "@shadowcat/ui-kit/test";
import type { SceneLevel } from "@shadowcat/core";

const levels: SceneLevel[] = [{ id: "l1", name: "Ground", bottom: 0, top: 10, background: null }];

describe("LevelsEditor", () => {
  it("adding a level commits the whole array including a new row", async () => {
    const onCommit = vi.fn();
    const context = setAppContextForTest();
    const { container } = render(LevelsEditor, { props: { levels, onCommit }, context });
    await fireEvent.click(container.querySelector('[data-testid="level-add"]')!);
    expect(onCommit).toHaveBeenCalledTimes(1);
    const next = onCommit.mock.calls[0][0] as SceneLevel[];
    expect(next).toHaveLength(2);
    expect(next[0]).toEqual(levels[0]); // original row untouched
    expect(next[1]).toMatchObject({ bottom: 0, top: 10, background: null });
  });

  it("removing a level commits the array without it", async () => {
    const onCommit = vi.fn();
    const context = setAppContextForTest();
    const { container } = render(LevelsEditor, { props: { levels, onCommit }, context });
    await fireEvent.click(container.querySelector('[data-testid="level-remove"]')!);
    expect(onCommit).toHaveBeenCalledWith([]);
  });

  it("editing a row's name commits the whole array with that field changed", async () => {
    const onCommit = vi.fn();
    const context = setAppContextForTest();
    const { container } = render(LevelsEditor, { props: { levels, onCommit }, context });
    const input = container.querySelector('[data-testid="level-name"]') as HTMLInputElement;
    await fireEvent.input(input, { target: { value: "Upper Floor" } });
    expect(onCommit).toHaveBeenCalledWith([{ id: "l1", name: "Upper Floor", bottom: 0, top: 10, background: null }]);
  });

  it("the background picker calls ctx.pickAsset and writes the returned id", async () => {
    const onCommit = vi.fn();
    const pickAsset = vi.fn().mockResolvedValue("asset-1");
    const context = setAppContextForTest({ pickAsset });
    const { container } = render(LevelsEditor, { props: { levels, onCommit }, context });
    await fireEvent.click(container.querySelector('[data-testid="level-background"]')!);
    expect(pickAsset).toHaveBeenCalledWith({ kind: "image" });
    await vi.waitFor(() => expect(onCommit).toHaveBeenCalledWith([{ id: "l1", name: "Ground", bottom: 0, top: 10, background: "asset-1" }]));
  });

  it("the root is a <section> with an accessible name, not a bare <div>", () => {
    const context = setAppContextForTest();
    const { container } = render(LevelsEditor, { props: { levels, onCommit: vi.fn() }, context });
    const root = container.querySelector('[data-testid="levels-editor"]')!;
    expect(root.tagName).toBe("SECTION");
    expect(root.getAttribute("aria-label")).toBeTruthy();
  });

  it("an invalid band (bottom >= top) shows inline feedback and marks the inputs aria-invalid; a valid band shows neither", () => {
    const context = setAppContextForTest();
    const invalid: SceneLevel[] = [{ id: "l1", name: "Bad", bottom: 10, top: 10, background: null }];
    const { container, rerender } = render(LevelsEditor, { props: { levels: invalid, onCommit: vi.fn() }, context });
    expect(container.querySelector('[data-testid="level-invalid"]')).not.toBeNull();
    expect(container.querySelector('[data-testid="level-bottom"]')!.getAttribute("aria-invalid")).toBe("true");
    expect(container.querySelector('[data-testid="level-top"]')!.getAttribute("aria-invalid")).toBe("true");

    rerender({ levels, onCommit: vi.fn() });
    expect(container.querySelector('[data-testid="level-invalid"]')).toBeNull();
    expect(container.querySelector('[data-testid="level-bottom"]')!.getAttribute("aria-invalid")).toBe("false");
  });
});
