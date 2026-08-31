import { describe, expect, it, vi } from "vitest";
import { BUILTIN_THEMES, DEFAULT_THEME_ID, THEME_TOKEN_NAMES } from "./theme";
import { ThemeController, activeTheme, theme } from "./theme.svelte";

describe("ThemeController", () => {
  it("starts on the default theme", () => {
    const controller = new ThemeController();
    expect(controller.active).toBe(DEFAULT_THEME_ID);
    expect(controller.resolved.id).toBe(DEFAULT_THEME_ID);
    expect(controller.customThemes).toEqual({});
  });

  it("setActive switches theme and notifies subscribers", () => {
    const controller = new ThemeController();
    const listener = vi.fn();
    controller.subscribe(listener);
    controller.setActive("slate-light");
    expect(controller.active).toBe("slate-light");
    expect(controller.resolved.colorScheme).toBe("light");
    expect(listener).toHaveBeenCalledTimes(1);
  });

  it("setActive with the current id is a no-op", () => {
    const controller = new ThemeController();
    const listener = vi.fn();
    controller.subscribe(listener);
    controller.setActive(DEFAULT_THEME_ID);
    expect(listener).not.toHaveBeenCalled();
  });

  it("setActive with an unresolvable id selects the default theme", () => {
    const controller = new ThemeController();
    controller.setActive("no-such-theme");
    expect(controller.active).toBe(DEFAULT_THEME_ID);
    controller.setActive("custom:missing");
    expect(controller.active).toBe(DEFAULT_THEME_ID);
  });

  it("saveCustom stores a validated theme resolvable via custom:<id>", () => {
    const controller = new ThemeController();
    controller.saveCustom("mine", {
      label: "Mine",
      base: "slate-light",
      tokens: { accent: "#123456", bogus: "red" } as never,
    });
    controller.setActive("custom:mine");
    expect(controller.active).toBe("custom:mine");
    expect(controller.resolved.tokens.accent).toBe("#123456");
    expect(controller.resolved.colorScheme).toBe("light");
    expect(controller.customThemes.mine!.tokens).toEqual({ accent: "#123456" });
  });

  it("deleteCustom removes the theme and falls back to the default when active", () => {
    const controller = new ThemeController();
    controller.saveCustom("mine", { label: "Mine", base: "slate-dark", tokens: {} });
    controller.setActive("custom:mine");
    controller.deleteCustom("mine");
    expect(controller.customThemes).toEqual({});
    expect(controller.active).toBe(DEFAULT_THEME_ID);
    controller.deleteCustom("mine");
    expect(controller.active).toBe(DEFAULT_THEME_ID);
  });

  it("load tolerates garbage", () => {
    const controller = new ThemeController();
    controller.load(undefined);
    expect(controller.active).toBe(DEFAULT_THEME_ID);
    controller.load({ active: "no-such-theme", custom: { bad: "nope" } as never });
    expect(controller.active).toBe(DEFAULT_THEME_ID);
    expect(controller.customThemes).toEqual({});
  });

  it("load applies a valid custom state", () => {
    const controller = new ThemeController();
    controller.load({
      active: "custom:mine",
      custom: { mine: { label: "Mine", base: "slate-dark", tokens: { accent: "#123456" } } },
    });
    expect(controller.active).toBe("custom:mine");
    expect(controller.resolved.tokens.accent).toBe("#123456");
  });

  it("load drops an active custom selector whose theme is absent", () => {
    const controller = new ThemeController();
    controller.load({ active: "custom:gone", custom: {} });
    expect(controller.active).toBe(DEFAULT_THEME_ID);
  });

  it("serialize round-trips through load", () => {
    const controller = new ThemeController();
    controller.saveCustom("mine", { label: "Mine", base: "slate-dark", tokens: { accent: "#123456" } });
    controller.setActive("custom:mine");
    const restored = new ThemeController();
    restored.load(controller.serialize());
    expect(restored.serialize()).toEqual(controller.serialize());
    expect(restored.resolved.tokens.accent).toBe("#123456");
  });

  it("applyTo writes every token and the color scheme inline", () => {
    const controller = new ThemeController();
    controller.setActive("contrast-dark");
    const doc = document.implementation.createHTMLDocument();
    controller.applyTo(doc);
    const style = doc.documentElement.style;
    for (const name of THEME_TOKEN_NAMES) {
      expect(style.getPropertyValue(`--${name}`)).not.toBe("");
    }
    expect(style.getPropertyValue("color-scheme")).toBe("dark");
    expect(style.getPropertyValue("--surface-base")).toBe(
      BUILTIN_THEMES.find((t) => t.id === "contrast-dark")!.tokens["surface-base"],
    );
  });

  it("registerDocument applies immediately and on later changes until unregistered", () => {
    const controller = new ThemeController();
    const doc = document.implementation.createHTMLDocument();
    const unregister = controller.registerDocument(doc);
    const style = doc.documentElement.style;
    expect(style.getPropertyValue("--surface-base")).toBe(
      BUILTIN_THEMES.find((t) => t.id === DEFAULT_THEME_ID)!.tokens["surface-base"],
    );
    controller.setActive("slate-light");
    expect(style.getPropertyValue("--surface-base")).toBe(
      BUILTIN_THEMES.find((t) => t.id === "slate-light")!.tokens["surface-base"],
    );
    unregister();
    controller.setActive("contrast-dark");
    expect(style.getPropertyValue("--surface-base")).toBe(
      BUILTIN_THEMES.find((t) => t.id === "slate-light")!.tokens["surface-base"],
    );
  });

  it("unsubscribe stops notifications", () => {
    const controller = new ThemeController();
    const listener = vi.fn();
    const unsubscribe = controller.subscribe(listener);
    unsubscribe();
    controller.setActive("slate-light");
    expect(listener).not.toHaveBeenCalled();
  });
});

describe("theme singleton adapter", () => {
  it("activeTheme reads the singleton's resolved theme", () => {
    expect(activeTheme()).toEqual(theme.resolved);
  });
});
