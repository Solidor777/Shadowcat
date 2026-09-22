// @vitest-environment node
// Exercises plain state and pure functions: no component render and no DOM API use, so the
// package-default jsdom environment would be constructed per file and never touched.
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

  describe("fallback locale", () => {
    it("consults the fallback locale (default 'en') before the raw key", () => {
      const i = new I18n("fr", { en: { onlyEn: "Only in English" }, fr: {} });
      expect(i.t("onlyEn")).toBe("Only in English");
    });

    it("prefers the active locale's own entry over the fallback's", () => {
      const i = new I18n("fr", { en: { greeting: "Hello" }, fr: { greeting: "Bonjour" } });
      expect(i.t("greeting")).toBe("Bonjour");
    });

    it("falls through to the raw key when neither the active nor the fallback locale has it", () => {
      const i = new I18n("fr", { en: {}, fr: {} });
      expect(i.t("nope")).toBe("nope");
    });

    it("accepts an explicit non-default fallback locale", () => {
      const i = new I18n("fr", { en: { onlyEn: "Only in English" }, de: { onlyEn: "Nur auf Deutsch" }, fr: {} }, "de");
      expect(i.t("onlyEn")).toBe("Nur auf Deutsch");
    });

    it("a PluralEntry found via t() is treated as a miss and falls through the chain", () => {
      const i = new I18n("fr", { en: { items: "fallback string" }, fr: { items: { other: "{count} items" } } });
      expect(i.t("items")).toBe("fallback string");
    });
  });

  describe("plural", () => {
    it("selects 'one' vs 'other' in English", () => {
      const i = new I18n("en", { en: { items: { one: "{count} item", other: "{count} items" } } });
      expect(i.plural("items", 1)).toBe("1 item");
      expect(i.plural("items", 5)).toBe("5 items");
      expect(i.plural("items", 0)).toBe("0 items");
    });

    it("selects a locale-specific category set (Polish: one/few/many/other)", () => {
      const i = new I18n("pl", {
        pl: { items: { one: "{count} przedmiot", few: "{count} przedmioty", many: "{count} przedmiotów", other: "{count} przedmiotu" } },
      });
      expect(i.plural("items", 1)).toBe("1 przedmiot");
      expect(i.plural("items", 2)).toBe("2 przedmioty"); // few: 2-4
      expect(i.plural("items", 5)).toBe("5 przedmiotów"); // many: 5-21
    });

    it("falls back to 'other' when the entry lacks the selected category", () => {
      const i = new I18n("en", { en: { items: { other: "{count} items" } } });
      expect(i.plural("items", 1)).toBe("1 items");
    });

    it("falls through the fallback locale, then the raw key, exactly like t()", () => {
      const i = new I18n("fr", { en: { items: { one: "{count} item", other: "{count} items" } }, fr: {} });
      expect(i.plural("items", 1)).toBe("1 item");
      const missing = new I18n("fr", { en: {}, fr: {} });
      expect(missing.plural("nope", 1)).toBe("nope");
    });

    it("merges {count} automatically and lets explicit params win on collision", () => {
      const i = new I18n("en", { en: { withName: { one: "{count} {name}", other: "{count} {name}s" } } });
      expect(i.plural("withName", 2, { name: "cat" })).toBe("2 cats");
    });

    it("selects the category using the FALLBACK locale's plural rules when the entry came from fallback", () => {
      // Intl.PluralRules('fr').select(0) === "one"; Intl.PluralRules('en').select(0) === "other".
      // With fr active and only an en entry, category selection must use en's rule (the locale
      // that supplied the entry), not fr's (the active locale) — else count=0 wrongly selects
      // "one" and renders "0 item" instead of "0 items".
      const i = new I18n("fr", { en: { items: { one: "{count} item", other: "{count} items" } }, fr: {} });
      expect(i.plural("items", 0)).toBe("0 items");
    });
  });

  describe("compare", () => {
    it("orders by primary letter regardless of case, unlike a plain code-point sort", () => {
      const i = new I18n("en", { en: {} });
      // Plain `<`/String sort puts every uppercase letter before every lowercase one
      // (code-point order), so "Zebra" < "apple". A locale-aware collator orders by letter
      // first: "apple" (a) sorts before "Zebra" (z).
      expect(["Zebra", "apple"].sort((x, y) => x < y ? -1 : x > y ? 1 : 0)).toEqual(["Zebra", "apple"]);
      expect(["Zebra", "apple"].sort((x, y) => i.compare(x, y))).toEqual(["apple", "Zebra"]);
    });

    it("rebuilds the collator on setLocale, so accent-letter ordering changes per locale", () => {
      // Documented Intl.Collator behavior (MDN): German treats "ä" as a variant of "a" sorting
      // between "a" and "z"; Swedish treats "ä" as its own letter sorting after "z".
      const i = new I18n("de", { de: {}, sv: {} });
      const words = ["z", "a", "ä"];
      i.setLocale("de");
      expect([...words].sort((a, b) => i.compare(a, b))).toEqual(["a", "ä", "z"]);
      i.setLocale("sv");
      expect([...words].sort((a, b) => i.compare(a, b))).toEqual(["a", "z", "ä"]);
    });
  });
});
