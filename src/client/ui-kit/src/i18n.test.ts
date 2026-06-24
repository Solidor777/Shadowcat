import { render, screen, waitFor } from "@testing-library/svelte";
import { test, expect, afterEach } from "vitest";
import Probe from "./__fixtures__/I18nProbe.svelte";
import { i18n, t } from "./i18n.svelte";

afterEach(() => i18n.setLocale("en"));

test("the Svelte t adapter renders the en string", () => {
  render(Probe);
  expect(screen.getByTestId("msg").textContent).toBe("Log in");
});

test("switching locale re-renders components using t", async () => {
  render(Probe);
  expect(screen.getByTestId("msg").textContent).toBe("Log in");
  // No catalog for "zz" → t falls back to the key; the re-render proves reactivity.
  i18n.setLocale("zz");
  await waitFor(() => expect(screen.getByTestId("msg").textContent).toBe("login.submit"));
});

test("t interpolates params", () => {
  expect(t("settings.role", { role: "gm" })).toBe("Role: gm");
});
