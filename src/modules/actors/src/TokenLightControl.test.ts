import { describe, it, expect, vi } from "vitest";
import { render, screen, fireEvent } from "@testing-library/svelte";
import { setAppContextForTest } from "@shadowcat/ui-kit/test";
import {
  DocumentStore,
  buildActorDoc,
  buildTokenDoc,
  buildTokenFromActor,
  DEFAULT_LIGHT_EMISSION,
  type LightEmission,
  type TokenEngine,
  type WireDocument,
  type WireOperation,
} from "@shadowcat/core";
import TokenLightControl from "./TokenLightControl.svelte";

const torch: LightEmission = { color: "#ffcc66", intensity: 1, brightRadius: 2, dimRadius: 4, falloff: null, enabled: true };

const actorEngine = {
  displayName: "G", visual: { kind: "image" as const, asset: "a1" }, size: { w: 1, h: 1 },
  shape: "square" as const, faction: null, conditions: [], prototype: false, vision: null, light: torch,
};

function storeWith(...docs: WireDocument[]): DocumentStore {
  const s = new DocumentStore();
  s.applyCommand({ seq: 1, world_id: "w1", author: "a", ts: 0, ops: docs.map((doc) => ({ op: "create" as const, doc })) });
  return s;
}

/** A linked token ("link" mode) against the torch-carrying actor. */
function linkedSetup(overrides?: TokenEngine["overrides"]) {
  const actor = buildActorDoc("w1", "G", actorEngine, "act1");
  const token = buildTokenFromActor("w1", "scene1", actor, "link", { x: 0, y: 0 }, { w: 100, h: 100 }, "tok1");
  if (overrides !== undefined) (token.engine as TokenEngine).overrides = overrides;
  return storeWith(actor, token);
}

describe("TokenLightControl", () => {
  it("renders nothing for a raw token, an instanced token, or a non-GM", () => {
    // A genuinely raw token (no actor link, no embedded copy).
    const raw = buildTokenDoc("w1", "scene1", {
      x: 0, y: 0, w: 100, h: 100, rotation: 0, visual: null, actor_id: null, overrides: null, face: null, elevation: null,
    }, "tok-raw");
    render(TokenLightControl, {
      context: setAppContextForTest({ documents: storeWith(raw), dispatchIntent: vi.fn(), role: "gm" }),
      props: { tokenId: "tok-raw" },
    });
    expect(screen.queryByTestId("token-light-mode")).toBeNull();

    // Instanced token: overrides do not apply → no control even with a carried light inside.
    const instanced = buildTokenFromActor("w1", "scene1", buildActorDoc("w1", "G", actorEngine, "act1"), "instance", { x: 0, y: 0 }, { w: 100, h: 100 }, "tok-i");
    render(TokenLightControl, {
      context: setAppContextForTest({ documents: storeWith(instanced), dispatchIntent: vi.fn(), role: "gm" }),
      props: { tokenId: "tok-i" },
    });
    expect(screen.queryByTestId("token-light-mode")).toBeNull();

    // A non-GM gets no editor affordance (the server's carried-light gate is GM-only).
    render(TokenLightControl, {
      context: setAppContextForTest({ documents: linkedSetup(), dispatchIntent: vi.fn(), role: "player" }),
      props: { tokenId: "tok1" },
    });
    expect(screen.queryByTestId("token-light-mode")).toBeNull();
  });

  it("shows inherit when no override is stored; suppress writes the effective emission disabled", async () => {
    const dispatchIntent = vi.fn();
    render(TokenLightControl, {
      context: setAppContextForTest({ documents: linkedSetup(), dispatchIntent, canEdit: () => true }),
      props: { tokenId: "tok1" },
    });
    const select = screen.getByTestId("token-light-mode") as HTMLSelectElement;
    expect(select.value).toBe("inherit");
    await fireEvent.change(select, { target: { value: "suppress" } });
    expect(dispatchIntent).toHaveBeenCalledWith([
      {
        op: "update",
        doc_id: "tok1",
        changes: [
          {
            path: "/engine/overrides",
            old: null, // the RAW stored overrides object (absent on a fresh linked token)
            new: { name: null, visual: null, size: null, shape: null, vision: null, light: { ...torch, enabled: false } },
          },
        ],
      },
    ]);
  });

  it("custom mode opens the field editor, which commits whole-payload override writes with the raw stored old", async () => {
    const dispatchIntent = vi.fn();
    const prior: LightEmission = { ...DEFAULT_LIGHT_EMISSION, enabled: true };
    const documents = linkedSetup({ name: null, visual: null, size: null, shape: null, vision: null, light: prior });
    render(TokenLightControl, {
      context: setAppContextForTest({ documents, dispatchIntent, canEdit: () => true }),
      props: { tokenId: "tok1" },
    });
    expect((screen.getByTestId("token-light-mode") as HTMLSelectElement).value).toBe("custom");
    await fireEvent.change(screen.getByTestId("emission-dim"), { target: { value: "9" } });
    expect(dispatchIntent).toHaveBeenCalledWith([
      {
        op: "update",
        doc_id: "tok1",
        changes: [
          {
            path: "/engine/overrides",
            old: { name: null, visual: null, size: null, shape: null, vision: null, light: prior },
            new: { name: null, visual: null, size: null, shape: null, vision: null, light: { ...prior, dimRadius: 9 } },
          },
        ],
      },
    ]);
  });

  it("switching back to inherit clears the override (light: null)", async () => {
    const dispatchIntent = vi.fn();
    const suppressed: LightEmission = { ...torch, enabled: false };
    const documents = linkedSetup({ name: null, visual: null, size: null, shape: null, vision: null, light: suppressed });
    render(TokenLightControl, {
      context: setAppContextForTest({ documents, dispatchIntent, canEdit: () => true }),
      props: { tokenId: "tok1" },
    });
    expect((screen.getByTestId("token-light-mode") as HTMLSelectElement).value).toBe("suppress");
    await fireEvent.change(screen.getByTestId("token-light-mode"), { target: { value: "inherit" } });
    const call = dispatchIntent.mock.calls.at(-1)![0] as WireOperation[];
    if (call[0].op !== "update") throw new Error("expected update");
    expect(call[0].changes[0].new).toMatchObject({ light: null });
  });
});
