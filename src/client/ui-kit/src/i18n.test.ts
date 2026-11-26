import { render, screen, waitFor } from "@testing-library/svelte";
import { test, expect, afterEach, vi } from "vitest";
import Probe from "./__fixtures__/I18nProbe.svelte";
import { i18n, t } from "./i18n.svelte";

// A module tag scopes every test-inserted catalog entry to this file's own cleanup, so a test
// cannot leave a stray "zz" override in the shared module-level `i18n` singleton for a later
// test to (silently) observe.
const TEST_MODULE = "test-i18n";

afterEach(() => {
  i18n.setLocale("en");
  i18n.removeModule(TEST_MODULE);
});

test("the Svelte t adapter renders the en string", () => {
  render(Probe);
  expect(screen.getByTestId("msg").textContent).toBe("Log in");
});

test("switching locale re-renders components using t", async () => {
  render(Probe);
  expect(screen.getByTestId("msg").textContent).toBe("Log in");
  // "zz" defines its OWN override for this key, so `I18n`'s fallback-locale chain (`"en"` for
  // this instance) never masks it — the re-render proves reactivity.
  i18n.addMessages("zz", { "login.submit": "ZZ Log in" }, { module: TEST_MODULE });
  i18n.setLocale("zz");
  await waitFor(() => expect(screen.getByTestId("msg").textContent).toBe("ZZ Log in"));
});

test("a key missing from the active locale falls through to the fallback locale, not the raw key", async () => {
  render(Probe);
  // First transition to a locale with its OWN, visibly different text for this key. Asserting
  // the fallback text directly from the initial "en" render would pass vacuously the instant
  // `setLocale` is called, before Svelte's reactive update ever runs — `waitFor` would see the
  // already-correct initial DOM and never actually wait for anything. Routing through a
  // DIFFERENT intermediate value first forces a genuine reactive transition to be observed.
  i18n.addMessages("zz", { "login.submit": "ZZ Log in" }, { module: TEST_MODULE });
  i18n.setLocale("zz");
  await waitFor(() => expect(screen.getByTestId("msg").textContent).toBe("ZZ Log in"));
  // "yy" has no catalog entry of its own for this key — the instance's fallback locale ("en")
  // must supply it, so the raw key string never reaches the DOM.
  i18n.setLocale("yy");
  await waitFor(() => expect(screen.getByTestId("msg").textContent).toBe("Log in"));
});

test("t interpolates params", () => {
  expect(t("settings.role", { role: "gm" })).toBe("Role: gm");
});

test("i18n's createSubscriber unsubscribes from i18n.subscribe on teardown", () => {
  const unsubscribeSpy = vi.fn();
  const subscribeSpy = vi.spyOn(i18n, "subscribe").mockReturnValue(unsubscribeSpy);

  const { unmount } = render(Probe);
  expect(subscribeSpy).toHaveBeenCalled();

  unmount();
  expect(unsubscribeSpy).toHaveBeenCalled();

  subscribeSpy.mockRestore();
});

test("no player-facing English string names a test or automation tool", async () => {
  const { en } = await import("./locales/en");
  const offenders = Object.entries(en).filter(([, text]) => /playwright|puppeteer|selenium|webdriver/i.test(JSON.stringify(text)));
  expect(offenders).toEqual([]);
});
