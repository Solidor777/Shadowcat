import { test, expect, vi, afterEach } from "vitest";
import { render, screen, fireEvent, cleanup } from "@testing-library/svelte";
import { setAppContextForTest } from "@shadowcat/ui-kit/test";
import type { AppContext } from "@shadowcat/ui-kit";
import { ContributionRegistry, PANEL_CONTRACT, silentLogger } from "@shadowcat/core";
import type { EngineAdapter } from "./engine/adapter";
import { applyOp, defaultLayout } from "./layout/tree";
import { PanelsController } from "./controller.svelte";
import { STAGE_ID } from "./engine/policy";

/** Minimal fake MediaQueryList (mirrors `@shadowcat/ui-kit`'s own
 * `FakeMediaQueryList`) so
 * PanelHost's sizeClass()-driven presentation switch is deterministic under
 * jsdom, which has no real `matchMedia`. Must be stubbed before the
 * file-scoped dynamic import below, since `sizeClass()`'s module reads
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
const { default: CrashOnceCountingPanel } = await import("./__fixtures__/CrashOnceCountingPanel.svelte");
// Dynamic, alongside the imports above: a static top-level `import { i18n }
// from "@shadowcat/ui-kit"` would pull in the whole ui-kit barrel (including
// `sizeClass()`'s module, which reads `matchMedia` at module load) BEFORE the
// `vi.stubGlobal` call above runs, breaking the ordering that comment warns about.
const { i18n, PanelsBridge } = await import("@shadowcat/ui-kit");
// Same hazard applies transitively: `./engine/dockview` itself statically
// imports `i18n` from `@shadowcat/ui-kit` (for its panel-menu labels), so it
// must be dynamic here too, after the stub above.
const { DockviewEngine } = await import("./engine/dockview");

afterEach(() => {
  cleanup();
  mql.fire(true); // reset to expanded between tests
});

/** Minimal `EngineAdapter` wrapping a `FakeEngine` and adding an `onNotice`
 * source — `FakeEngine` itself intentionally omits `onNotice` (no notice
 * source of its own; see `EngineAdapter`'s doc comment), so exercising
 * PanelHost's `eng.onNotice?.()` subscription + `unsubNotice?.()` teardown
 * needs a test double that actually implements it. */
class NoticeEngine implements EngineAdapter {
  #fake = new FakeEngine();
  #noticeListeners = new Set<(key: string) => void>();

  init(host: HTMLElement, slotFor: (id: string) => HTMLElement, stageEl: HTMLElement): void {
    this.#fake.init(host, slotFor, stageEl);
  }
  apply(...args: Parameters<EngineAdapter["apply"]>): void {
    this.#fake.apply(...args);
  }
  onOp(cb: Parameters<EngineAdapter["onOp"]>[0]): () => void {
    return this.#fake.onOp(cb);
  }
  onNotice(cb: (key: string) => void): () => void {
    this.#noticeListeners.add(cb);
    return () => this.#noticeListeners.delete(cb);
  }
  /** Test helper: fires a notice exactly as a real engine's internal source would. */
  emitNotice(key: string): void {
    for (const cb of this.#noticeListeners) cb(key);
  }
  /** Test helper: direct observation of subscriber count, so teardown can be
   * asserted without relying on a DOM side-effect (a post-unmount `$state`
   * write does not re-trigger an already-unmounted template's effect in
   * Svelte 5, regardless of whether the underlying listener was removed —
   * so DOM text is not a valid proxy for "was `unsubNotice?.()` called"). */
  get noticeListenerCount(): number {
    return this.#noticeListeners.size;
  }
  focus(id: string): void {
    this.#fake.focus(id);
  }
  /** Deliberately does NOT clear `#noticeListeners` — only the unsubscribe
   * function returned by `onNotice` may remove a listener, so a listener
   * left behind here is observable proof `unsubNotice?.()` did not run. */
  destroy(): void {
    this.#fake.destroy();
  }
}

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

  // Pop-out leg: dock⇄float⇄...⇄pop-out⇄pop-in never re-mounts. The
  // FakeEngine degrades pop-out to a floating window, so the slot is re-parented
  // (adopted), never recreated.
  engine.emitOp({ op: "popOut", id: "chat:panel" });
  await Promise.resolve();
  expect(mounts).toBe(1);

  engine.emitOp({ op: "popIn", id: "chat:panel" });
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

  // Compact switcher: the gmOnly panel never reaches `compact.order` either.
  // (The dock-chip strip is now rendered solely by the statusbar's
  // `panel-dock` Surface; its gmOnly filtering is enforced upstream by
  // `regsForRole`, covered in "regsForRole: a gmOnly registration is invisible to a non-GM role".)
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

test("removed-while-docked: disposing a docked contribution prunes it reactively without crashing the host, survivor stays adopted", async () => {
  const registry = new ContributionRegistry();
  const disposeA = registry.contribute({
    id: "a:panel",
    contract: PANEL_CONTRACT,
    component: CountingPanel,
    props: { onMountFn: () => {} },
    panel: { icon: "a", labelKey: "chat.tab", defaultPlacement: { kind: "docked", zone: "right" } },
  });
  registry.contribute({
    id: "b:panel",
    contract: PANEL_CONTRACT,
    component: CountingPanel,
    props: { onMountFn: () => {} },
    panel: { icon: "b", labelKey: "chat.tab", defaultPlacement: { kind: "docked", zone: "bottom" } },
  });
  const engine = new FakeEngine();
  const context = setAppContextForTest({ contributions: registry, role: "gm" });
  render(PanelHost, { props: { engine }, context });
  await Promise.resolve();

  expect(() => disposeA()).not.toThrow();
  await Promise.resolve();

  // Removed id's zone/group is gone entirely from the FakeEngine's tree.
  expect(engine.groupEl("right", 0)).toBeNull();

  // Host is alive: the survivor is still mounted and adopted into its group.
  const survivorSlot = screen
    .getAllByTestId("counting-panel")
    .map((el) => el.closest('[data-panel="b:panel"]'))
    .find((el) => el !== null);
  const survivorGroup = engine.groupEl("bottom", 0);
  expect(survivorSlot).toBeTruthy();
  expect(survivorGroup).toBeTruthy();
  expect(survivorSlot!.parentElement!.isSameNode(survivorGroup)).toBe(true);
});

test("crash-reload: the sole remount path is the {#key} bump — reset() is not also invoked, so exactly one new mount recovers the panel", async () => {
  let mounts = 0;
  const registry = new ContributionRegistry();
  registry.contribute({
    id: "crash:panel",
    contract: PANEL_CONTRACT,
    component: CrashOnceCountingPanel,
    props: {
      onMountFn: () => {
        mounts++;
      },
    },
    panel: { icon: "c", labelKey: "chat.tab", defaultPlacement: { kind: "docked", zone: "right" } },
  });
  const context = setAppContextForTest({ contributions: registry, role: "gm" });
  render(PanelHost, { context });

  expect(mounts).toBe(1);

  await fireEvent.click(screen.getByTestId("boom-btn"));
  expect(screen.getByTestId("crashed-crash:panel")).toBeTruthy();

  await fireEvent.click(screen.getByTestId("reload-crash:panel"));

  expect(screen.queryByTestId("crashed-crash:panel")).toBeNull();
  expect(screen.getByTestId("boom-btn")).toBeTruthy();
  expect(mounts).toBe(2);
});

test("compact staging: only the active view is adopted; flipping to expanded releases a launcher-only panel back to staging", async () => {
  const registry = new ContributionRegistry();
  registry.contribute({
    id: "chat:panel",
    contract: PANEL_CONTRACT,
    component: CountingPanel,
    props: { onMountFn: () => {} },
    panel: { icon: "c", labelKey: "chat.tab", defaultPlacement: { kind: "docked", zone: "right" } },
  });
  registry.contribute({
    id: "launcher:panel",
    contract: PANEL_CONTRACT,
    component: CountingPanel,
    props: { onMountFn: () => {} },
    panel: { icon: "l", labelKey: "chat.tab" },
  });
  const context = setAppContextForTest({ contributions: registry, role: "gm" });
  const { container } = render(PanelHost, { context });

  mql.fire(false); // compact
  await Promise.resolve();

  await fireEvent.click(screen.getByTestId("compact-switch-launcher:panel"));
  await Promise.resolve();

  const launcherSlot = container.querySelector('[data-panel="launcher:panel"]')!;
  const stagingEl = container.querySelector(".staging")!;
  expect(launcherSlot.parentElement!.isSameNode(stagingEl)).toBe(false);

  mql.fire(true); // expanded
  await Promise.resolve();

  // Never placed in `expanded` (launcher-only), so the engine's reconcile has
  // no reclaim path of its own for it — CompactSwitcher must release it.
  expect(launcherSlot.parentElement!.isSameNode(stagingEl)).toBe(true);
});

test("live region: minimizing a panel announces panels.moved with its label and 'Minimize'", async () => {
  const registry = new ContributionRegistry();
  registry.contribute({
    id: "chat:panel",
    contract: PANEL_CONTRACT,
    component: CountingPanel,
    props: { onMountFn: () => {} },
    panel: { icon: "c", labelKey: "chat.tab", defaultPlacement: { kind: "docked", zone: "right" } },
  });
  const engine = new FakeEngine();
  // The default test `t` (`(k) => k`, no interpolation) can't prove the
  // announcement text is actually assembled from the panel's label + the
  // op's destination — use the real catalog-backed `i18n.t` instead.
  const context = setAppContextForTest({ contributions: registry, role: "gm", t: (k, p) => i18n.t(k, p) });
  const { container } = render(PanelHost, { props: { engine }, context });
  await Promise.resolve();

  const liveRegion = container.querySelector('[role="status"]')!;
  expect(liveRegion).toBeTruthy();
  expect(liveRegion.textContent).toBe("");

  engine.emitOp({ op: "minimize", id: "chat:panel" });
  await Promise.resolve();

  expect(liveRegion.textContent).toBe("Chat moved to Minimize");
});

test("live region: a pure focus/resize op (activeTab) does not announce", async () => {
  const registry = new ContributionRegistry();
  registry.contribute({
    id: "chat:panel",
    contract: PANEL_CONTRACT,
    component: CountingPanel,
    props: { onMountFn: () => {} },
    panel: { icon: "c", labelKey: "chat.tab", defaultPlacement: { kind: "docked", zone: "right" } },
  });
  registry.contribute({
    id: "notes:panel",
    contract: PANEL_CONTRACT,
    component: CountingPanel,
    props: { onMountFn: () => {} },
    panel: { icon: "n", labelKey: "chat.tab", defaultPlacement: { kind: "docked", zone: "right" } },
  });
  const engine = new FakeEngine();
  const context = setAppContextForTest({ contributions: registry, role: "gm" });
  const { container } = render(PanelHost, { props: { engine }, context });
  await Promise.resolve();

  const liveRegion = container.querySelector('[role="status"]')!;
  engine.emitOp({ op: "activeTab", zone: "right", group: 0, id: "chat:panel" });
  await Promise.resolve();

  expect(liveRegion.textContent).toBe("");
});

test("live region: opening a panel that starts minimized announces its docked destination", async () => {
  const registry = new ContributionRegistry();
  registry.contribute({
    id: "chat:panel",
    contract: PANEL_CONTRACT,
    component: CountingPanel,
    props: { onMountFn: () => {} },
    panel: { icon: "c", labelKey: "chat.tab", defaultPlacement: { kind: "docked", zone: "right" } },
  });

  // Starts minimized, not launcher-closed — proves the minimized/closed/
  // popped-out fallthrough branch of `applyOp`'s `"open"` case (detach then
  // `placeByPlacement`) fires from EITHER prior state, not just "closed".
  let saved = defaultLayout([{ id: "chat:panel", placement: { kind: "docked", zone: "right" } }]);
  saved = applyOp(saved, { op: "minimize", id: "chat:panel" });

  const engine = new FakeEngine();
  const context = setAppContextForTest({
    contributions: registry,
    role: "gm",
    t: (k, p) => i18n.t(k, p),
    uiState: { getPanelLayout: () => saved, setPanelLayout: () => {}, getChatRead: () => null, setChatRead: () => {} },
  });
  const { container } = render(PanelHost, { props: { engine }, context });
  await Promise.resolve();

  const liveRegion = container.querySelector('[role="status"]')!;
  expect(liveRegion.textContent).toBe("");

  engine.emitOp({ op: "open", id: "chat:panel", placement: { kind: "docked", zone: "right" } });
  await Promise.resolve();

  expect(liveRegion.textContent).toBe("Chat moved to Dock right");
});

test("live region: opening a panel that starts closed (never placed) announces its docked destination", async () => {
  const registry = new ContributionRegistry();
  registry.contribute({
    id: "chat:panel",
    contract: PANEL_CONTRACT,
    component: CountingPanel,
    props: { onMountFn: () => {} },
    panel: { icon: "c", labelKey: "chat.tab", defaultPlacement: { kind: "docked", zone: "right" } },
  });

  // No placement passed to `defaultLayout` — launcher-only/closed, absent
  // from every location `locate` checks.
  const saved = defaultLayout([{ id: "chat:panel" }]);

  const engine = new FakeEngine();
  const context = setAppContextForTest({
    contributions: registry,
    role: "gm",
    t: (k, p) => i18n.t(k, p),
    uiState: { getPanelLayout: () => saved, setPanelLayout: () => {}, getChatRead: () => null, setChatRead: () => {} },
  });
  const { container } = render(PanelHost, { props: { engine }, context });
  await Promise.resolve();

  const liveRegion = container.querySelector('[role="status"]')!;

  engine.emitOp({ op: "open", id: "chat:panel", placement: { kind: "docked", zone: "right" } });
  await Promise.resolve();

  expect(liveRegion.textContent).toBe("Chat moved to Dock right");
});

test("live region: opening a panel that starts popped-out announces its docked destination", async () => {
  const registry = new ContributionRegistry();
  registry.contribute({
    id: "chat:panel",
    contract: PANEL_CONTRACT,
    component: CountingPanel,
    props: { onMountFn: () => {} },
    panel: { icon: "c", labelKey: "chat.tab", defaultPlacement: { kind: "docked", zone: "right" } },
  });

  const saved = defaultLayout([{ id: "chat:panel", placement: { kind: "docked", zone: "right" } }]);

  const engine = new FakeEngine();
  const context = setAppContextForTest({
    contributions: registry,
    role: "gm",
    t: (k, p) => i18n.t(k, p),
    uiState: { getPanelLayout: () => saved, setPanelLayout: () => {}, getChatRead: () => null, setChatRead: () => {} },
  });
  const { container } = render(PanelHost, { props: { engine }, context });
  await Promise.resolve();

  const liveRegion = container.querySelector('[role="status"]')!;

  // Pop out LIVE (via a gesture, post-mount) rather than seeding a
  // persisted `poppedOut` id — `PanelsController`'s constructor-time
  // `#rehydratePoppedOut` would otherwise convert a persisted popped-out id
  // straight to floating before this test ever gets to open it, hiding the
  // "popped-out" prior state this test needs to exercise. Popped-out is the
  // third fallthrough member of `applyOp`'s "open" case, alongside minimized
  // and closed, and requires the same narration: a panel reopened from its
  // own popped-out window is a real placement change, same as reopening a
  // minimized or closed one — a guard keyed on only two of the three would
  // silently miss it.
  engine.emitOp({ op: "popOut", id: "chat:panel" });
  await Promise.resolve();
  // The "popOut" op itself narrates too (an existing, unchanged case in
  // `describeOp`'s switch) — confirms the panel is actually popped-out
  // before the "open" assertion below, rather than assuming it silently.
  expect(liveRegion.textContent).toBe("Chat moved to Pop out");

  engine.emitOp({ op: "open", id: "chat:panel", placement: { kind: "docked", zone: "right" } });
  await Promise.resolve();

  expect(liveRegion.textContent).toBe("Chat moved to Dock right");
});

test("live region: opening an already-docked panel that becomes the active tab does not announce (focus bump)", async () => {
  const registry = new ContributionRegistry();
  registry.contribute({
    id: "chat:panel",
    contract: PANEL_CONTRACT,
    component: CountingPanel,
    props: { onMountFn: () => {} },
    panel: { icon: "c", labelKey: "chat.tab", defaultPlacement: { kind: "docked", zone: "right" } },
  });
  registry.contribute({
    id: "notes:panel",
    contract: PANEL_CONTRACT,
    component: CountingPanel,
    props: { onMountFn: () => {} },
    panel: { icon: "n", labelKey: "chat.tab", defaultPlacement: { kind: "docked", zone: "right" } },
  });

  // Both docked into the SAME zone-0 group; docking "notes:panel" second
  // makes it the active tab, leaving "chat:panel" present-but-inactive.
  let saved = defaultLayout([{ id: "chat:panel", placement: { kind: "docked", zone: "right" } }]);
  saved = applyOp(saved, { op: "dock", id: "notes:panel", zone: "right", group: 0 });
  expect(saved.expanded.zones.right.groups[0]).toEqual({ tabs: ["chat:panel", "notes:panel"], active: "notes:panel", size: 1 });

  const engine = new FakeEngine();
  const context = setAppContextForTest({
    contributions: registry,
    role: "gm",
    t: (k, p) => i18n.t(k, p),
    uiState: { getPanelLayout: () => saved, setPanelLayout: () => {}, getChatRead: () => null, setChatRead: () => {} },
  });
  const { container } = render(PanelHost, { props: { engine }, context });
  await Promise.resolve();

  const liveRegion = container.querySelector('[role="status"]')!;

  engine.emitOp({ op: "open", id: "chat:panel", placement: { kind: "docked", zone: "right" } });
  await Promise.resolve();

  expect(liveRegion.textContent).toBe("");
});

test("live region: opening an already-floating panel that gets bumped in z-order does not announce (focus bump)", async () => {
  const registry = new ContributionRegistry();
  registry.contribute({
    id: "chat:panel",
    contract: PANEL_CONTRACT,
    component: CountingPanel,
    props: { onMountFn: () => {} },
    panel: { icon: "c", labelKey: "chat.tab", defaultPlacement: { kind: "floating" } },
  });
  registry.contribute({
    id: "notes:panel",
    contract: PANEL_CONTRACT,
    component: CountingPanel,
    props: { onMountFn: () => {} },
    panel: { icon: "n", labelKey: "chat.tab", defaultPlacement: { kind: "floating" } },
  });

  // Both floating; registration order cascades "chat:panel" to z=0 and
  // "notes:panel" to z=1 — "chat:panel" starts NOT topmost.
  const saved = defaultLayout([
    { id: "chat:panel", placement: { kind: "floating" } },
    { id: "notes:panel", placement: { kind: "floating" } },
  ]);
  expect(saved.expanded.floating.find((f) => f.id === "chat:panel")!.z).toBeLessThan(
    saved.expanded.floating.find((f) => f.id === "notes:panel")!.z,
  );

  const engine = new FakeEngine();
  const context = setAppContextForTest({
    contributions: registry,
    role: "gm",
    t: (k, p) => i18n.t(k, p),
    uiState: { getPanelLayout: () => saved, setPanelLayout: () => {}, getChatRead: () => null, setChatRead: () => {} },
  });
  const { container } = render(PanelHost, { props: { engine }, context });
  await Promise.resolve();

  const liveRegion = container.querySelector('[role="status"]')!;

  engine.emitOp({ op: "open", id: "chat:panel", placement: { kind: "floating" } });
  await Promise.resolve();

  expect(liveRegion.textContent).toBe("");
});

test("PanelsController.dispatch: a true no-op open (already active docked / already topmost floating) never calls onOp", () => {
  const registry = new ContributionRegistry();
  registry.contribute({
    id: "chat:panel",
    contract: PANEL_CONTRACT,
    component: CountingPanel,
    props: { onMountFn: () => {} },
    panel: { icon: "c", labelKey: "chat.tab", defaultPlacement: { kind: "docked", zone: "right" } },
  });

  const saved = defaultLayout([{ id: "chat:panel", placement: { kind: "docked", zone: "right" } }]);
  const ops: unknown[] = [];
  const ctrl = new PanelsController({
    contributions: registry,
    role: "gm",
    getPanelLayout: () => saved,
    setPanelLayout: () => {},
    bridge: { bind: () => {} },
    logger: silentLogger,
    onOp: (op) => ops.push(op),
  });

  // Already the sole tab in its group and already active — `applyOp`'s
  // "open" case's `group.active === o.id` guard returns the SAME reference,
  // so `dispatch` never calls `onOp` at all (the SAME-REFERENCE NO-OP
  // CONTRACT `dispatch` documents).
  ctrl.dispatch({ op: "open", id: "chat:panel", placement: { kind: "docked", zone: "right" } });
  expect(ops).toEqual([]);

  // Already floating and already topmost — `applyOp`'s "open" case's
  // `current.z === maxZ` guard returns the SAME reference for the same
  // reason. The preceding "float" dispatch is a real change (docked ->
  // floating) and DOES call `onOp`; only the subsequent "open" on an
  // already-topmost floating panel must not.
  ctrl.dispatch({ op: "float", id: "chat:panel", rect: { x: 0, y: 0, w: 200, h: 200 } });
  expect(ops).toHaveLength(1);
  ctrl.dispatch({ op: "open", id: "chat:panel", placement: { kind: "docked", zone: "right" } });
  expect(ops).toHaveLength(1);
});

test("FakeEngine.init adopts the stageEl into a center-well container", async () => {
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
  const { container } = render(PanelHost, { props: { engine }, context });
  await Promise.resolve();

  const stageEl = container.querySelector(".stage")!;
  const centerEl = engine.centerEl();
  expect(centerEl).toBeTruthy();
  expect(stageEl.parentElement!.isSameNode(centerEl)).toBe(true);
});

test("live region: an engine notice announces the resolved i18n text", async () => {
  const registry = new ContributionRegistry();
  registry.contribute({
    id: "chat:panel",
    contract: PANEL_CONTRACT,
    component: CountingPanel,
    props: { onMountFn: () => {} },
    panel: { icon: "c", labelKey: "chat.tab", defaultPlacement: { kind: "docked", zone: "right" } },
  });
  const engine = new NoticeEngine();
  // Real catalog-backed `i18n.t`, mirroring the op-driven live-region test
  // above — proves the text is actually the RESOLVED i18n string, not the raw key.
  const context = setAppContextForTest({ contributions: registry, role: "gm", t: (k, p) => i18n.t(k, p) });
  const { container } = render(PanelHost, { props: { engine }, context });
  await Promise.resolve();

  const liveRegion = container.querySelector('[role="status"]')!;
  expect(liveRegion.textContent).toBe("");

  engine.emitNotice("panels.popoutRestoredFloating");
  await Promise.resolve();

  expect(liveRegion.textContent).toBe(i18n.t("panels.popoutRestoredFloating"));
});

test("a reload-restored popout's notice reaches the live region", async () => {
  const registry = new ContributionRegistry();
  registry.contribute({
    id: "chat:panel",
    contract: PANEL_CONTRACT,
    component: CountingPanel,
    props: { onMountFn: () => {} },
    panel: { icon: "c", labelKey: "chat.tab", defaultPlacement: { kind: "docked", zone: "right" } },
  });

  // A persisted layout with "chat:panel" popped-out — `PanelsController`
  // rehydrates it to floating + queues (but does not yet FIRE)
  // `panels.popoutRestoredFloating` at construction.
  let saved = defaultLayout([{ id: "chat:panel", placement: { kind: "docked", zone: "right" } }]);
  saved = applyOp(saved, { op: "dock", id: "chat:panel", zone: "right", group: "new" });
  saved = applyOp(saved, { op: "popOut", id: "chat:panel" });

  const engine = new FakeEngine();
  const context = setAppContextForTest({
    contributions: registry,
    role: "gm",
    t: (k, p) => i18n.t(k, p),
    uiState: { getPanelLayout: () => saved, setPanelLayout: () => {}, getChatRead: () => null, setChatRead: () => {} },
  });
  const { container } = render(PanelHost, { props: { engine }, context });
  await Promise.resolve();

  // Proves the notice is actually OBSERVABLE by the live region a screen
  // reader watches, not merely that some callback fired.
  // "rehydratePoppedOut: a persisted popped-out id comes back as
  // floating + a notice" proves the narrower claim (construction
  // alone must not call `onNotice`); the test below proves `PanelHost` is
  // what performs the flush that makes this text appear.
  const liveRegion = container.querySelector('[role="status"]')!;
  expect(liveRegion.textContent).toBe(i18n.t("panels.popoutRestoredFloating"));
});

test("PanelHost's post-mount effect — not PanelsController's constructor — is what flushes the reload notice", async () => {
  const registry = new ContributionRegistry();
  registry.contribute({
    id: "chat:panel",
    contract: PANEL_CONTRACT,
    component: CountingPanel,
    props: { onMountFn: () => {} },
    panel: { icon: "c", labelKey: "chat.tab", defaultPlacement: { kind: "docked", zone: "right" } },
  });

  let saved = defaultLayout([{ id: "chat:panel", placement: { kind: "docked", zone: "right" } }]);
  saved = applyOp(saved, { op: "dock", id: "chat:panel", zone: "right", group: "new" });
  saved = applyOp(saved, { op: "popOut", id: "chat:panel" });

  const notices: string[] = [];
  // Built OUTSIDE `PanelHost`, mirroring its own construction args exactly
  // (mounted via the `controller` test-injection prop below) — isolates
  // "did `onNotice` fire" from Svelte's internal template-effect scheduling,
  // which the DOM-text assertion above can't distinguish timing-wise (jsdom
  // + testing-library flush effects synchronously within `render()`, so a
  // pre/post-render `textContent` snapshot alone cannot prove WHEN the
  // notice fired relative to mount, only that it eventually did).
  const ctrl = new PanelsController({
    contributions: registry,
    role: "gm",
    getPanelLayout: () => saved,
    setPanelLayout: () => {},
    bridge: { bind: () => {} },
    logger: silentLogger,
    onNotice: (key) => notices.push(key),
  });
  // Construction alone must not have announced anything yet.
  expect(notices).toEqual([]);

  const engine = new FakeEngine();
  const context = setAppContextForTest({ contributions: registry, role: "gm", t: (k, p) => i18n.t(k, p) });
  render(PanelHost, { props: { engine, controller: ctrl }, context });

  // Mounting `PanelHost` with this already-constructed controller is what
  // triggers the flush — proving the post-mount `$effect`, not the
  // constructor, is the actual firing point.
  expect(notices).toEqual(["panels.popoutRestoredFloating"]);
});

test("live region: unmounting unsubscribes the engine's onNotice listener (unsubNotice teardown)", async () => {
  const registry = new ContributionRegistry();
  registry.contribute({
    id: "chat:panel",
    contract: PANEL_CONTRACT,
    component: CountingPanel,
    props: { onMountFn: () => {} },
    panel: { icon: "c", labelKey: "chat.tab", defaultPlacement: { kind: "docked", zone: "right" } },
  });
  const engine = new NoticeEngine();
  const context = setAppContextForTest({ contributions: registry, role: "gm", t: (k, p) => i18n.t(k, p) });
  const { unmount } = render(PanelHost, { props: { engine }, context });
  await Promise.resolve();

  // Mounting subscribes exactly one listener to the engine's notice source.
  expect(engine.noticeListenerCount).toBe(1);

  unmount();

  // Direct proof `unsubNotice?.()` ran: the engine's subscriber set is empty
  // post-unmount. (A DOM-text assertion here would be vacuous — Svelte 5
  // disposes a template's effect on unmount, so a post-unmount `$state`
  // write never re-renders the detached node whether or not the listener
  // was actually removed; see the getter's doc comment above.)
  expect(engine.noticeListenerCount).toBe(0);
});

test("PanelHost throws a clear error if ctx.panels doesn't look like a PanelsBridgeLike", () => {
  const registry = new ContributionRegistry();
  const context = setAppContextForTest({
    contributions: registry,
    role: "gm",
    panels: { notBind: true } as unknown as AppContext["panels"],
  });
  expect(() => render(PanelHost, { context })).toThrow(/PanelsBridgeLike/);
});

test("PanelHost mounts normally when ctx.panels has a bind method", () => {
  const registry = new ContributionRegistry();
  const context = setAppContextForTest({
    contributions: registry,
    role: "gm",
    panels: { bind: vi.fn() } as unknown as AppContext["panels"],
  });
  expect(() => render(PanelHost, { context })).not.toThrow();
});

/** Builds a two-panel layout with both panels docked into the same "right"
 * zone group-0, "notes:panel" active — mirrors the existing
 * "live region: opening an already-docked panel that becomes the active tab"
 * fixture. Reopening the present-but-inactive "chat:panel" is a REAL
 * placement change (`applyOp`'s "open" case flips `group.active`), the exact
 * "off-screen/behind another window" shape `EngineAdapter.focus` wiring
 * exists to raise. */
function inactiveTabLayout(): { registry: ContributionRegistry; saved: ReturnType<typeof defaultLayout> } {
  const registry = new ContributionRegistry();
  registry.contribute({
    id: "chat:panel",
    contract: PANEL_CONTRACT,
    component: CountingPanel,
    props: { onMountFn: () => {} },
    panel: { icon: "c", labelKey: "chat.tab", defaultPlacement: { kind: "docked", zone: "right" } },
  });
  registry.contribute({
    id: "notes:panel",
    contract: PANEL_CONTRACT,
    component: CountingPanel,
    props: { onMountFn: () => {} },
    panel: { icon: "n", labelKey: "chat.tab", defaultPlacement: { kind: "docked", zone: "right" } },
  });
  let saved = defaultLayout([{ id: "chat:panel", placement: { kind: "docked", zone: "right" } }]);
  saved = applyOp(saved, { op: "dock", id: "notes:panel", zone: "right", group: 0 });
  return { registry, saved };
}

test("focus wiring (FakeEngine): reopening an already-docked, present-but-inactive panel via ctx.panels.focus calls the engine's focus(id)", async () => {
  const { registry, saved } = inactiveTabLayout();
  const engine = new FakeEngine();
  const focusSpy = vi.spyOn(engine, "focus");
  const bridge = new PanelsBridge(silentLogger);
  const context = setAppContextForTest({
    contributions: registry,
    role: "gm",
    panels: bridge,
    uiState: { getPanelLayout: () => saved, setPanelLayout: () => {}, getChatRead: () => null, setChatRead: () => {} },
  });
  render(PanelHost, { props: { engine }, context });
  await Promise.resolve();

  expect(focusSpy).not.toHaveBeenCalled();

  // The reachable chain the TODO names: SheetsController.openDocument ->
  // PanelsBridge.focus -> PanelsController.focus -> this.open(id) -> dispatch
  // -> PanelHost's onOp -> eng.focus(id).
  bridge.focus("chat:panel");
  await Promise.resolve();

  expect(focusSpy).toHaveBeenCalledWith("chat:panel");
});

test("focus wiring (DockviewEngine): reopening an already-docked, present-but-inactive panel via ctx.panels.focus calls the engine's focus(id)", async () => {
  const { registry, saved } = inactiveTabLayout();
  const engine = new DockviewEngine(silentLogger);
  const focusSpy = vi.spyOn(engine, "focus");
  const bridge = new PanelsBridge(silentLogger);
  const context = setAppContextForTest({
    contributions: registry,
    role: "gm",
    panels: bridge,
    uiState: { getPanelLayout: () => saved, setPanelLayout: () => {}, getChatRead: () => null, setChatRead: () => {} },
  });
  render(PanelHost, { props: { engine }, context });
  await Promise.resolve();

  expect(focusSpy).not.toHaveBeenCalled();

  bridge.focus("chat:panel");
  await Promise.resolve();

  expect(focusSpy).toHaveBeenCalledWith("chat:panel");
});

test("FakeEngine.focus no-ops on STAGE_ID, mirroring DockviewEngine.focus's own guard", () => {
  const engine = new FakeEngine();
  const host = document.createElement("div");
  const stageEl = document.createElement("div");
  engine.init(host, () => document.createElement("div"), stageEl);

  expect(() => engine.focus(STAGE_ID)).not.toThrow();
  expect(engine.focused).toBeNull();

  engine.focus("chat:panel");
  expect(engine.focused).toBe("chat:panel");
});
