import { describe, it, expect, vi } from "vitest";
import { I18n } from "./i18n";

const make = () =>
  new I18n("en", {
    en: { greeting: "Hi {name}", plain: "Plain" },
    fr: { greeting: "Salut {name}", plain: "Simple" },
  });

describe("I18n", () => {
  it("looks up and interpolates", () => {
    const i = make();
    expect(i.t("plain")).toBe("Plain");
    expect(i.t("greeting", { name: "Ada" })).toBe("Hi Ada");
  });

  it("returns the key for a missing key, and leaves an unknown {param}", () => {
    const i = make();
    expect(i.t("nope")).toBe("nope");
    expect(i.t("greeting")).toBe("Hi {name}"); // missing param left intact
  });

  it("setLocale switches output and notifies subscribers (once per change)", () => {
    const i = make();
    const listener = vi.fn();
    i.subscribe(listener);
    expect(i.locale).toBe("en");
    i.setLocale("fr");
    expect(i.locale).toBe("fr");
    expect(i.t("plain")).toBe("Simple");
    expect(listener).toHaveBeenCalledTimes(1);
    i.setLocale("fr"); // no-op
    expect(listener).toHaveBeenCalledTimes(1);
  });

  it("exposes the available locales and unsubscribe stops notifications", () => {
    const i = make();
    expect(i.locales.sort()).toEqual(["en", "fr"]);
    const listener = vi.fn();
    const off = i.subscribe(listener);
    off();
    i.setLocale("fr");
    expect(listener).not.toHaveBeenCalled();
  });

  describe("addMessages / removeModule", () => {
    it("merges a new key into an existing locale's catalog", () => {
      const i = make();
      i.addMessages("en", { "mod.hello": "Mod hello" });
      expect(i.t("mod.hello")).toBe("Mod hello");
    });

    it("overwrites an existing key, including one present at construction", () => {
      const i = make();
      i.addMessages("en", { plain: "Overwritten" });
      expect(i.t("plain")).toBe("Overwritten");
    });

    it("does not notify subscribers for a non-active locale, but does for the active one", () => {
      const i = make();
      const listener = vi.fn();
      i.subscribe(listener);
      i.addMessages("fr", { "mod.hello": "Bonjour" }); // fr is not active
      expect(listener).not.toHaveBeenCalled();
      i.addMessages("en", { "mod.hello": "Hello" }); // en is active
      expect(listener).toHaveBeenCalledTimes(1);
    });

    it("removeModule removes exactly a module's own inserted keys, across multiple calls and locales", () => {
      const i = make();
      i.addMessages("en", { "modA.one": "A1" }, { module: "modA" });
      i.addMessages("fr", { "modA.two": "A2" }, { module: "modA" });
      i.addMessages("en", { "modB.one": "B1" }, { module: "modB" });
      i.addMessages("en", { "unowned.one": "U1" }); // no module tag
      i.removeModule("modA");
      expect(i.t("modA.one")).toBe("modA.one"); // fell back to key
      i.setLocale("fr");
      expect(i.t("modA.two")).toBe("modA.two"); // fell back to key
      i.setLocale("en");
      expect(i.t("modB.one")).toBe("B1"); // other module untouched
      expect(i.t("unowned.one")).toBe("U1"); // host-registered untouched
      expect(i.t("plain")).toBe("Plain"); // construction-time key untouched
    });

    it("removeModule on a module that never called addMessages is a no-op", () => {
      const i = make();
      const listener = vi.fn();
      i.subscribe(listener);
      expect(() => i.removeModule("never-registered")).not.toThrow();
      expect(listener).not.toHaveBeenCalled();
    });

    it("removeModule does not restore a value it shadowed — the key falls back again", () => {
      const i = make();
      expect(i.t("plain")).toBe("Plain"); // built-in value, pre-shadow
      i.addMessages("en", { plain: "Shadowed" }, { module: "modA" });
      expect(i.t("plain")).toBe("Shadowed");
      i.removeModule("modA");
      expect(i.t("plain")).toBe("plain"); // not restored to "Plain" — falls back to the raw key
    });
  });
});
