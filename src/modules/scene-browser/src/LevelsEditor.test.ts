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
});
