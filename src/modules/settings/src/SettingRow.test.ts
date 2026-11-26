import { describe, it, expect, vi } from "vitest";
import { render, screen, fireEvent } from "@testing-library/svelte";
import { setAppContextForTest } from "@shadowcat/ui-kit/test";
import SettingRow from "./SettingRow.svelte";
import type { SettingField } from "./registerSetting";

/** Renders a row for `field` under the identity-echo `t` of the app-context fixture. */
function renderRow(field: SettingField) {
  return render(SettingRow, { props: { field, labelKey: "example.label" }, context: setAppContextForTest({}) });
}

describe("SettingRow", () => {
  it("renders a toggle reflecting get() and calls set on change", async () => {
    let value = true;
    const set = vi.fn((v: boolean) => {
      value = v;
    });
    renderRow({ kind: "toggle", get: () => value, set });
    const box = screen.getByRole("checkbox", { name: "example.label" }) as HTMLInputElement;
    expect(box.checked).toBe(true);
    await fireEvent.click(box);
    expect(set).toHaveBeenCalledWith(false);
    expect(box.checked).toBe(false);
  });

  it("renders a select with translated options, selects get()'s value and calls set on change", async () => {
    let value = "b";
    const set = vi.fn((v: string) => {
      value = v;
    });
    renderRow({
      kind: "select",
      get: () => value,
      set,
      options: [
        { value: "a", labelKey: "example.a" },
        { value: "b", labelKey: "example.b" },
      ],
    });
    const select = screen.getByRole("combobox", { name: "example.label" }) as HTMLSelectElement;
    expect(Array.from(select.options).map((o) => [o.value, o.textContent])).toEqual([
      ["a", "example.a"],
      ["b", "example.b"],
    ]);
    expect(select.value).toBe("b");
    await fireEvent.change(select, { target: { value: "a" } });
    expect(set).toHaveBeenCalledWith("a");
    expect(select.value).toBe("a");
  });

  it("renders a text input showing get(), its placeholder, and calls set on change", async () => {
    let value = "hello";
    const set = vi.fn((v: string) => {
      value = v;
    });
    renderRow({ kind: "text", get: () => value, set, placeholder: "type here" });
    const input = screen.getByRole("textbox", { name: "example.label" }) as HTMLInputElement;
    expect(input.value).toBe("hello");
    expect(input.placeholder).toBe("type here");
    await fireEvent.change(input, { target: { value: "world" } });
    expect(set).toHaveBeenCalledWith("world");
    expect(input.value).toBe("world");
  });
});
