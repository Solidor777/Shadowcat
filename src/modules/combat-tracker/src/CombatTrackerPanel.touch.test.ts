import { describe, it, expect, afterEach, vi } from "vitest";
import { cleanup } from "@testing-library/svelte";
import combatTrackerPanelSource from "./CombatTrackerPanel.svelte?raw";
import combatHeaderSource from "./CombatHeader.svelte?raw";
import combatantRowSource from "./CombatantRow.svelte?raw";

/** Minimal fake `MediaQueryList` (mirrors `PanelHost.test.ts`'s own) so `sizeClass()`'s
 * `(min-width: 48rem)` query is deterministic under jsdom, which has no real `matchMedia`. Must
 * be stubbed before any static import of `@shadowcat/ui-kit` (including transitively, via the
 * component under test), since `sizeClass()`'s module reads `matchMedia` once at module load. */
class FakeMediaQueryList {
  matches: boolean;
  constructor(matches: boolean) {
    this.matches = matches;
  }
  addEventListener(): void {}
  removeEventListener(): void {}
}
const mql = new FakeMediaQueryList(false); // start compact (query does not match)
vi.stubGlobal("matchMedia", () => mql);

const { render } = await import("@testing-library/svelte");
const { setAppContextForTest } = await import("@shadowcat/ui-kit/test");
const { DocumentStore, buildCombatDoc, buildCombatantDoc, newCombatEngine } = await import("@shadowcat/core");
const { default: CombatTrackerPanel } = await import("./CombatTrackerPanel.svelte");
const { TurnBadge } = await import("./turnBadge");
const { fakeCombatApi } = await import("./__fixtures__/fakeCombatApi");
import type { CombatantEngine } from "@shadowcat/core";

afterEach(() => cleanup());

describe("CombatTrackerPanel compact sizing", () => {
  it("applies the compact class to the panel, header, and rows under a narrow viewport", () => {
    const store = new DocumentStore();
    const combatEngine = newCombatEngine("scene-1");
    combatEngine.active = true;
    const combat = buildCombatDoc("w1", combatEngine, "c1");
    const combatantEngine: CombatantEngine = { kind: { type: "actor", token_id: null, actor_id: null }, initiative: null, tiebreak: 0, resources: {} };
    (combat.engine as { order: string[] }).order = ["a"];
    const combatant = buildCombatantDoc("w1", "c1", combatantEngine, { id: "a", name: "a" });
    store.applyCommand({ seq: 1, world_id: "w1", author: "x", ts: 0, ops: [{ op: "create", doc: combat }, { op: "create", doc: combatant }] });
    const combatApi = fakeCombatApi(store, { role: "gm" });
    const { container } = render(CombatTrackerPanel, {
      props: { badge: new TurnBadge() },
      context: setAppContextForTest({ store, documents: store, combat: combatApi, viewedSceneId: "scene-1", role: "gm" }),
    });
    expect(container.querySelector("section.combat-tracker.compact")).toBeTruthy();
    expect(container.querySelector("header.compact")).toBeTruthy();
    expect(container.querySelector(".row.compact")).toBeTruthy();
  });

  it("every button/input carries the 44px coarse-target rule in the compact branch of each component's own styles", () => {
    for (const source of [combatTrackerPanelSource, combatHeaderSource, combatantRowSource]) {
      const ruleMatch = source.match(/&\.compact\s*\{([^{}]*(?:\{[^{}]*\}[^{}]*)*)\}/);
      expect(ruleMatch, source.slice(0, 40)).toBeTruthy();
      expect(ruleMatch?.[1]).toMatch(/min-height:\s*44px/);
    }
  });
});
