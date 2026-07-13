import { test, expect, vi, afterEach } from "vitest";
import { render, screen, fireEvent, cleanup } from "@testing-library/svelte";
import { setAppContextForTest } from "@shadowcat/ui-kit/test";
import { ContributionRegistry, PANEL_CONTRACT } from "@shadowcat/core";

/** Minimal fake MediaQueryList (mirrors ui-kit's sizeClass.test.ts) so
 * PanelHost's sizeClass()-driven presentation switch is deterministic under
 * jsdom, which has no real `matchMedia`. Must be stubbed before the
 * file-scoped dynamic import below, since `sizeClass.svelte.ts` reads
 * `matchMedia` once at module load. */
class FakeMediaQueryList {
  matches: boolean;
  #listeners: Array<() => void> = [];
  constructor(matches: boolean) {
    this.matches = matches;
  }
  addEventListener(_type: string, cb: () => void): void {
    this.#listeners.push(cb);
  }
  removeEventListener(_type: string, cb: () => void): void {
    this.#listeners = this.#listeners.filter((l) => l !== cb);
  }
  fire(matches: boolean): void {
    this.matches = matches;
    for (const cb of this.#listeners) cb();
  }
}
const mql = new FakeMediaQueryList(true); // start expanded
vi.stubGlobal("matchMedia", () => mql);

const { default: PanelHost } = await import("./PanelHost.svelte");
const { FakeEngine } = await import("./engine/fake");
const { default: CountingPanel } = await import("./__fixtures__/CountingPanel.svelte");
const { default: ThrowingPanel } = await import("./__fixtures__/ThrowingPanel.svelte");

afterEach(() => {
  cleanup();
  mql.fire(true); // reset to expanded between tests
});

test("mount-counter: a docked panel's component mounts exactly once across the full op lifecycle", async () => {
  let mounts = 0;
  const registry = new ContributionRegistry();
  registry.contribute({
    id: "chat:panel",
    contract: PANEL_CONTRACT,
    component: CountingPanel,
    props: {
      onMountFn: () => {
        mounts++;
      },
    },
    panel: { icon: "c", labelKey: "chat.tab", defaultPlacement: { kind: "docked", zone: "right" } },
  });
  const engine = new FakeEngine();
  const context = setAppContextForTest({ contributions: registry, role: "gm" });
  render(PanelHost, { props: { engine }, context });

  expect(mounts).toBe(1);

  engine.emitOp({ op: "open", id: "chat:panel" });
  await Promise.resolve();
  expect(mounts).toBe(1);

  engine.emitOp({ op: "dock", id: "chat:panel", zone: "bottom", group: "new" });
  await Promise.resolve();
  expect(mounts).toBe(1);

  engine.emitOp({ op: "minimize", id: "chat:panel" });
  await Promise.resolve();
  expect(mounts).toBe(1);

  engine.emitOp({ op: "restore", id: "chat:panel" });
  await Promise.resolve();
  expect(mounts).toBe(1);

  engine.emitOp({ op: "float", id: "chat:panel", rect: { x: 0, y: 0, w: 200, h: 200 } });
  await Promise.resolve();
  expect(mounts).toBe(1);

  mql.fire(false); // compact
  await Promise.resolve();
  expect(mounts).toBe(1);

  mql.fire(true); // expanded
  await Promise.resolve();
  expect(mounts).toBe(1);
});

test("gmOnly: a gmOnly registration is absent from the compact switcher and dock chips when role is not gm", async () => {
  const registry = new ContributionRegistry();
  registry.contribute({
    id: "chat:panel",
    contract: PANEL_CONTRACT,
    component: CountingPanel,
    props: { onMountFn: () => {} },
    panel: { icon: "c", labelKey: "chat.tab", defaultPlacement: { kind: "docked", zone: "right" } },
  });
  registry.contribute({
    id: "game-settings:panel",
    contract: PANEL_CONTRACT,
    component: CountingPanel,
    props: { onMountFn: () => {} },
    panel: { icon: "g", labelKey: "gameSettings.tab", gmOnly: true, defaultPlacement: { kind: "minimized" } },
  });
  const context = setAppContextForTest({ contributions: registry, role: "player" });
  render(PanelHost, { context });

  // Dock chips (expanded, default presentation): the gmOnly panel is
  // minimized by default placement but never rendered as a restore chip.
  expect(screen.queryByTestId("chip-game-settings:panel")).toBeNull();

  // Compact switcher: the gmOnly panel never reaches `compact.order` either.
  mql.fire(false);
  await Promise.resolve();
  expect(screen.queryByTestId("compact-switch-game-settings:panel")).toBeNull();
  expect(screen.getByTestId("compact-switch-chat:panel")).toBeTruthy();
});

test("boundary: a panel that throws on an event shows the reload affordance and leaves siblings mounted", async () => {
  let siblingMounts = 0;
  const registry = new ContributionRegistry();
  registry.contribute({
    id: "throwing:panel",
    contract: PANEL_CONTRACT,
    component: ThrowingPanel,
    panel: { icon: "t", labelKey: "chat.tab", defaultPlacement: { kind: "docked", zone: "right" } },
  });
  registry.contribute({
    id: "sibling:panel",
    contract: PANEL_CONTRACT,
    component: CountingPanel,
    props: {
      onMountFn: () => {
        siblingMounts++;
      },
    },
    panel: { icon: "s", labelKey: "chat.tab", defaultPlacement: { kind: "docked", zone: "bottom" } },
  });
  const context = setAppContextForTest({ contributions: registry, role: "gm" });
  render(PanelHost, { context });

  expect(siblingMounts).toBe(1);
  expect(screen.getByTestId("counting-panel")).toBeTruthy();

  await fireEvent.click(screen.getByTestId("boom-btn"));

  expect(screen.getByTestId("crashed-throwing:panel")).toBeTruthy();
  // Sibling stays alive, untouched by the crash.
  expect(siblingMounts).toBe(1);
  expect(screen.getByTestId("counting-panel")).toBeTruthy();

  await fireEvent.click(screen.getByTestId("reload-throwing:panel"));
  expect(screen.queryByTestId("crashed-throwing:panel")).toBeNull();
  expect(screen.getByTestId("boom-btn")).toBeTruthy();
});

test("adoption: after apply, a docked panel's slot element is adopted into the FakeEngine's group container", async () => {
  const registry = new ContributionRegistry();
  registry.contribute({
    id: "chat:panel",
    contract: PANEL_CONTRACT,
    component: CountingPanel,
    props: { onMountFn: () => {} },
    panel: { icon: "c", labelKey: "chat.tab", defaultPlacement: { kind: "docked", zone: "right" } },
  });
  const engine = new FakeEngine();
  const context = setAppContextForTest({ contributions: registry, role: "gm" });
  render(PanelHost, { props: { engine }, context });
  await Promise.resolve();

  const slotEl = screen.getByTestId("counting-panel").closest('[data-panel="chat:panel"]');
  const groupEl = engine.groupEl("right", 0);
  expect(slotEl).toBeTruthy();
  expect(groupEl).toBeTruthy();
  expect(slotEl!.parentElement!.isSameNode(groupEl)).toBe(true);
});
