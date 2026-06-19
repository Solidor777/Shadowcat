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
});
