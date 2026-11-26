import { render, screen, waitFor } from "@testing-library/svelte";
import { test, expect } from "vitest";
import { ContributionRegistry } from "@shadowcat/core";
import Harness from "./__fixtures__/SurfaceHarness.svelte";
import Probe from "./__fixtures__/Probe.svelte";
import { findTourAnchor } from "./tourAnchors";

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
  await waitFor(() => expect(screen.queryByTestId("probe")).toBeNull());
});

test("a contribution with a tourAnchor renders inside a data-tour wrapper; one without is unwrapped", () => {
  const registry = new ContributionRegistry();
  registry.contribute({ id: "anch", contract: "s:bar", component: Probe, props: { label: "T" }, tourAnchor: "example.anchor" });
  registry.contribute({ id: "plain", contract: "s:bar", component: Probe, props: { label: "P" } });

  render(Harness, { props: { registry, contract: "s:bar" } });

  const wrapper = screen.getByText("T").closest("[data-tour]");
  expect(wrapper?.getAttribute("data-tour")).toBe("example.anchor");
  expect(screen.getByText("P").closest("[data-tour]")).toBeNull();
  expect(findTourAnchor(document, "example.anchor")).toBe(screen.getByText("T"));
});

test("a tourAnchor on an isolated contribution wraps the isolation wrapper", () => {
  const registry = new ContributionRegistry();
  registry.contribute({ id: "both", contract: "s:bar", component: Probe, props: { label: "B" }, styling: "isolated", tourAnchor: "example.both" });

  render(Harness, { props: { registry, contract: "s:bar" } });

  const anchor = findTourAnchor(document, "example.both");
  expect(anchor?.classList.contains("sc-theme-isolate")).toBe(true);
});

test("findTourAnchor returns null for an unmounted or illegal anchor", () => {
  expect(findTourAnchor(document, "example.absent")).toBeNull();
  expect(findTourAnchor(document, '"] x')).toBeNull();
});

test("an isolated contribution renders wrapped in the theme-isolation class; a default one is not wrapped", () => {
  const registry = new ContributionRegistry();
  registry.contribute({ id: "iso", contract: "s:bar", component: Probe, props: { label: "I" }, styling: "isolated" });
  registry.contribute({ id: "plain", contract: "s:bar", component: Probe, props: { label: "P" } });

  render(Harness, { props: { registry, contract: "s:bar" } });

  expect(screen.getByText("I").closest(".sc-theme-isolate")).toBeTruthy();
  expect(screen.getByText("P").closest(".sc-theme-isolate")).toBeNull();
});
