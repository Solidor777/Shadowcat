import { describe, it, expect, vi } from "vitest";
import { render, screen, fireEvent } from "@testing-library/svelte";
import { setAppContextForTest } from "@shadowcat/ui-kit/test";
import {
  DocumentStore,
  buildActorDoc,
  buildTokenDoc,
  buildTokenFromActor,
  type BarAttribute,
  type TokenEngine,
  type WireDocument,
} from "@shadowcat/core";
import TokenBarsControl from "./TokenBarsControl.svelte";

const hp: BarAttribute = { label: "HP", current: "hp.value", max: 10, color: "#cc2222", mode: "always" };
const actorEngine = {
  displayName: "G", visual: { kind: "image" as const, asset: "a1" }, size: { w: 1, h: 1 },
  shape: "square" as const, faction: null as string | null, conditions: [], prototype: false,
  vision: null, light: null, movement: [],
  aura: null, sound: null, vfx: null, bars: [hp],
};
const NO_OVERRIDES = { name: null, visual: null, size: null, shape: null, vision: null, light: null, movement: null, aura: null, sound: null, vfx: null, disposition: null };

function storeWith(...docs: WireDocument[]): DocumentStore {
  const s = new DocumentStore();
  s.applyCommand({ seq: 1, world_id: "w1", author: "a", ts: 0, ops: docs.map((doc) => ({ op: "create" as const, doc })) });
  return s;
}

function linkedSetup(overrides?: TokenEngine["overrides"]) {
  const actor = buildActorDoc("w1", "G", actorEngine, "act1");
  const token = buildTokenFromActor("w1", "scene1", actor, "link", { x: 0, y: 0 }, { w: 100, h: 100 }, "tok1");
  if (overrides !== undefined) (token.engine as TokenEngine).overrides = overrides;
  return storeWith(actor, token);
}

describe("TokenBarsControl", () => {
  it("renders nothing for a raw token, an instanced token, or when canEdit refuses", () => {
    const raw = buildTokenDoc("w1", "scene1", {
      x: 0, y: 0, w: 100, h: 100, rotation: 0, visual: null, actor_id: null, overrides: null, face: null, elevation: null, lock_rotation: false, identity_hidden: false,
    }, "tok-raw");
    render(TokenBarsControl, { context: setAppContextForTest({ documents: storeWith(raw), canEdit: () => true }), props: { tokenId: "tok-raw" } });
    expect(screen.queryByTestId("token-bars-source")).toBeNull();

    const instanced = buildTokenFromActor("w1", "scene1", buildActorDoc("w1", "G", actorEngine, "act1"), "instance", { x: 0, y: 0 }, { w: 100, h: 100 }, "tok-i");
    render(TokenBarsControl, { context: setAppContextForTest({ documents: storeWith(instanced), canEdit: () => true }), props: { tokenId: "tok-i" } });
    expect(screen.queryByTestId("token-bars-source")).toBeNull();

    render(TokenBarsControl, { context: setAppContextForTest({ documents: linkedSetup(), canEdit: () => false }), props: { tokenId: "tok1" } });
    expect(screen.queryByTestId("token-bars-source")).toBeNull();
  });

  it("inherits by default and shows no bar list", () => {
    render(TokenBarsControl, { context: setAppContextForTest({ documents: linkedSetup(), canEdit: () => true }), props: { tokenId: "tok1" } });
    expect((screen.getByTestId("token-bars-source") as HTMLSelectElement).value).toBe("inherit");
    expect((screen.getByTestId("token-nameplate-mode") as HTMLSelectElement).value).toBe("inherit");
    expect(screen.queryByTestId("bars-editor")).toBeNull();
  });

  it("choosing a nameplate mode writes the whole overrides object with the raw stored old", async () => {
    const dispatchIntent = vi.fn();
    render(TokenBarsControl, { context: setAppContextForTest({ documents: linkedSetup(), dispatchIntent, canEdit: () => true }), props: { tokenId: "tok1" } });
    await fireEvent.change(screen.getByTestId("token-nameplate-mode"), { target: { value: "always" } });
    expect(dispatchIntent).toHaveBeenCalledWith([
      { op: "update", doc_id: "tok1", changes: [{ path: "/engine/overrides", old: null, new: { ...NO_OVERRIDES, nameplate_mode: "always" } }] },
    ]);
  });

  it("switching to custom seeds the token's list from the actor's", async () => {
    const dispatchIntent = vi.fn();
    render(TokenBarsControl, { context: setAppContextForTest({ documents: linkedSetup(), dispatchIntent, canEdit: () => true }), props: { tokenId: "tok1" } });
    await fireEvent.change(screen.getByTestId("token-bars-source"), { target: { value: "custom" } });
    expect(dispatchIntent).toHaveBeenCalledWith([
      { op: "update", doc_id: "tok1", changes: [{ path: "/engine/overrides", old: null, new: { ...NO_OVERRIDES, bars: [hp] } }] },
    ]);
  });

  it("a stored custom list shows the editor; editing it replaces the list wholesale; inherit drops the key", async () => {
    const dispatchIntent = vi.fn();
    const stored = { ...NO_OVERRIDES, movement: ["flying"], bars: [{ ...hp, label: "Own" }] };
    render(TokenBarsControl, { context: setAppContextForTest({ documents: linkedSetup(stored), dispatchIntent, canEdit: () => true }), props: { tokenId: "tok1" } });
    expect((screen.getByTestId("token-bars-source") as HTMLSelectElement).value).toBe("custom");
    await fireEvent.change(screen.getByTestId("bar-color"), { target: { value: "#00ff00" } });
    expect(dispatchIntent).toHaveBeenLastCalledWith([
      { op: "update", doc_id: "tok1", changes: [{ path: "/engine/overrides", old: stored, new: { ...stored, bars: [{ ...hp, label: "Own", color: "#00ff00" }] } }] },
    ]);
    await fireEvent.change(screen.getByTestId("token-bars-source"), { target: { value: "inherit" } });
    const { bars: _dropped, ...withoutBars } = stored;
    expect(dispatchIntent).toHaveBeenLastCalledWith([
      { op: "update", doc_id: "tok1", changes: [{ path: "/engine/overrides", old: stored, new: withoutBars }] },
    ]);
  });
});
