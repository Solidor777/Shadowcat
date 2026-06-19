import { render, screen } from "@testing-library/svelte";
import { test, expect } from "vitest";
import Probe from "./__fixtures__/Probe.svelte";

test("the Svelte test harness renders a component", () => {
  render(Probe, { props: { label: "hello" } });
  expect(screen.getByTestId("probe").textContent).toBe("hello");
});
