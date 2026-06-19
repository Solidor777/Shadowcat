import { render, screen } from "@testing-library/svelte";
import { test, expect } from "vitest";
import { ContributionRegistry } from "@shadowcat/core";
import Harness from "./__fixtures__/SurfaceHarness.svelte";
import Probe from "./__fixtures__/Probe.svelte";

test("renders contributions for the contract, sorted by order", () => {
  const registry = new ContributionRegistry();
  registry.contribute({ id: "b", contract: "s:bar", order: 2, component: Probe, props: { label: "B" } });
  registry.contribute({ id: "a", contract: "s:bar", order: 1, component: Probe, props: { label: "A" } });
  registry.contribute({ id: "other", contract: "s:elsewhere", component: Probe, props: { label: "X" } });

  render(Harness, { props: { registry, contract: "s:bar" } });

  const probes = screen.getAllByTestId("probe").map((n) => n.textContent);
  expect(probes).toEqual(["A", "B"]); // order 1 before 2; the other contract excluded
});

test("an empty surface renders nothing", () => {
  const registry = new ContributionRegistry();
  render(Harness, { props: { registry, contract: "s:empty" } });
  expect(screen.queryByTestId("probe")).toBeNull();
});

test("updates reactively when a contribution is added then disposed", async () => {
  const registry = new ContributionRegistry();
  render(Harness, { props: { registry, contract: "s:live" } });
  expect(screen.queryByTestId("probe")).toBeNull();

  const dispose = registry.contribute({ id: "p", contract: "s:live", component: Probe, props: { label: "live" } });
  // Svelte flushes reactive DOM updates on a microtask; findBy awaits it.
  expect((await screen.findByTestId("probe")).textContent).toBe("live");

  dispose();
  await Promise.resolve();
  expect(screen.queryByTestId("probe")).toBeNull();
});
