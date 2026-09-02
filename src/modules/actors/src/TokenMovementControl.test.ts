import { describe, it, expect, vi } from "vitest";
import { render, screen, fireEvent } from "@testing-library/svelte";
import { setAppContextForTest } from "@shadowcat/ui-kit/test";
import {
  DocumentStore,
  buildActorDoc,
  buildFactionRegistryDoc,
  buildTokenDoc,
  buildTokenFromActor,
  type TokenEngine,
  type WireDocument,
  type WireOperation,
} from "@shadowcat/core";
import TokenMovementControl from "./TokenMovementControl.svelte";

const actorEngine = {
  displayName: "G", visual: { kind: "image" as const, asset: "a1" }, size: { w: 1, h: 1 },
  shape: "square" as const, faction: null as string | null, conditions: [], prototype: false,
  vision: null, light: null, movement: ["flying"],
};

function storeWith(...docs: WireDocument[]): DocumentStore {
  const s = new DocumentStore();
  s.applyCommand({ seq: 1, world_id: "w1", author: "a", ts: 0, ops: docs.map((doc) => ({ op: "create" as const, doc })) });
  return s;
}

/** A linked token ("link" mode) against the flying actor. */
function linkedSetup(overrides?: TokenEngine["overrides"]) {
  const actor = buildActorDoc("w1", "G", actorEngine, "act1");
  const token = buildTokenFromActor("w1", "scene1", actor, "link", { x: 0, y: 0 }, { w: 100, h: 100 }, "tok1");
  if (overrides !== undefined) (token.engine as TokenEngine).overrides = overrides;
  return storeWith(actor, token);
}

describe("TokenMovementControl", () => {
  it("renders nothing for a raw token, an instanced token, or when canEdit refuses", () => {
    const raw = buildTokenDoc("w1", "scene1", {
      x: 0, y: 0, w: 100, h: 100, rotation: 0, visual: null, actor_id: null, overrides: null, face: null, elevation: null,
    }, "tok-raw");
    render(TokenMovementControl, {
      context: setAppContextForTest({ documents: storeWith(raw), dispatchIntent: vi.fn(), canEdit: () => true }),
      props: { tokenId: "tok-raw" },
    });
    expect(screen.queryByTestId("token-movement-mode")).toBeNull();

    const instanced = buildTokenFromActor("w1", "scene1", buildActorDoc("w1", "G", actorEngine, "act1"), "instance", { x: 0, y: 0 }, { w: 100, h: 100 }, "tok-i");
    render(TokenMovementControl, {
      context: setAppContextForTest({ documents: storeWith(instanced), dispatchIntent: vi.fn(), canEdit: () => true }),
      props: { tokenId: "tok-i" },
    });
    expect(screen.queryByTestId("token-movement-mode")).toBeNull();

    render(TokenMovementControl, {
      context: setAppContextForTest({ documents: linkedSetup(), dispatchIntent: vi.fn(), canEdit: () => false }),
      props: { tokenId: "tok1" },
    });
    expect(screen.queryByTestId("token-movement-mode")).toBeNull();
  });

  it("shows inherit with the resolved set read-only when no override is stored; custom seeds from it", async () => {
    const dispatchIntent = vi.fn();
    render(TokenMovementControl, {
      context: setAppContextForTest({ documents: linkedSetup(), dispatchIntent, canEdit: () => true }),
      props: { tokenId: "tok1" },
    });
    const select = screen.getByTestId("token-movement-mode") as HTMLSelectElement;
    expect(select.value).toBe("inherit");
    // The inherited resolved set is displayed (read-only) in inherit mode.
    expect(screen.getByTestId("movement-inherited").textContent).toContain("flying");

    await fireEvent.change(select, { target: { value: "custom" } });
    expect(dispatchIntent).toHaveBeenCalledWith([
      {
        op: "update",
        doc_id: "tok1",
        changes: [
          {
            path: "/engine/overrides",
            old: null, // the RAW stored overrides object (absent on a fresh linked token)
            new: { name: null, visual: null, size: null, shape: null, vision: null, light: null, movement: ["flying"] },
          },
        ],
      },
    ]);
  });

  it("the inherited set unions the linked faction's tags (resolveTokenActor's precedence)", () => {
    const actor = buildActorDoc("w1", "G", { ...actorEngine, faction: "f1" }, "act1");
    const registry = buildFactionRegistryDoc("w1", { f1: { name: "F", color: "#fff", stance: "neutral", movement: ["incorporeal"] } }, "reg1");
    const token = buildTokenFromActor("w1", "scene1", actor, "link", { x: 0, y: 0 }, { w: 100, h: 100 }, "tok1");
    render(TokenMovementControl, {
      context: setAppContextForTest({ documents: storeWith(registry, actor, token), dispatchIntent: vi.fn(), canEdit: () => true }),
      props: { tokenId: "tok1" },
    });
    const shown = screen.getByTestId("movement-inherited").textContent;
    expect(shown).toContain("flying");
    expect(shown).toContain("incorporeal");
  });

  it("a stored empty override list reads as custom (wholesale 'no movement tags', not inherit)", () => {
    const documents = linkedSetup({ name: null, visual: null, size: null, shape: null, vision: null, light: null, movement: [] });
    render(TokenMovementControl, {
      context: setAppContextForTest({ documents, dispatchIntent: vi.fn(), canEdit: () => true }),
      props: { tokenId: "tok1" },
    });
    expect((screen.getByTestId("token-movement-mode") as HTMLSelectElement).value).toBe("custom");
  });

  it("the tag editor commits whole-payload override writes with the raw stored old", async () => {
    const dispatchIntent = vi.fn();
    const prior = ["flying"];
    const documents = linkedSetup({ name: null, visual: null, size: null, shape: null, vision: null, light: null, movement: prior });
    render(TokenMovementControl, {
      context: setAppContextForTest({ documents, dispatchIntent, canEdit: () => true }),
      props: { tokenId: "tok1" },
    });
    expect((screen.getByTestId("token-movement-mode") as HTMLSelectElement).value).toBe("custom");
    await fireEvent.click(screen.getByTestId("movement-toggle-incorporeal"));
    expect(dispatchIntent).toHaveBeenCalledWith([
      {
        op: "update",
        doc_id: "tok1",
        changes: [
          {
            path: "/engine/overrides",
            old: { name: null, visual: null, size: null, shape: null, vision: null, light: null, movement: prior },
            new: { name: null, visual: null, size: null, shape: null, vision: null, light: null, movement: ["flying", "incorporeal"] },
          },
        ],
      },
    ]);
  });

  it("switching back to inherit clears the override (movement: null)", async () => {
    const dispatchIntent = vi.fn();
    const documents = linkedSetup({ name: null, visual: null, size: null, shape: null, vision: null, light: null, movement: ["flying"] });
    render(TokenMovementControl, {
      context: setAppContextForTest({ documents, dispatchIntent, canEdit: () => true }),
      props: { tokenId: "tok1" },
    });
    await fireEvent.change(screen.getByTestId("token-movement-mode"), { target: { value: "inherit" } });
    const call = dispatchIntent.mock.calls.at(-1)![0] as WireOperation[];
    if (call[0].op !== "update") throw new Error("expected update");
    expect(call[0].changes[0].new).toMatchObject({ movement: null });
  });
});
