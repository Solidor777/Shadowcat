import { describe, it, expect, vi } from "vitest";
import { render, screen, fireEvent } from "@testing-library/svelte";
import { setAppContextForTest } from "@shadowcat/ui-kit/test";
import {
  DocumentStore,
  buildActorDoc,
  buildTokenDoc,
  buildTokenFromActor,
  type TokenEngine,
  type VisionAssignment,
  type WireDocument,
  type WireOperation,
} from "@shadowcat/core";
import TokenVisionControl from "./TokenVisionControl.svelte";

const actorVision: VisionAssignment[] = [{ mode: "darkvision", range: 12 }];

const actorEngine = {
  displayName: "G", visual: { kind: "image" as const, asset: "a1" }, size: { w: 1, h: 1 },
  shape: "square" as const, faction: null, conditions: [], prototype: false, vision: actorVision, light: null,
};

function storeWith(...docs: WireDocument[]): DocumentStore {
  const s = new DocumentStore();
  s.applyCommand({ seq: 1, world_id: "w1", author: "a", ts: 0, ops: docs.map((doc) => ({ op: "create" as const, doc })) });
  return s;
}

/** A linked token ("link" mode) against the darkvision-carrying actor. */
function linkedSetup(overrides?: TokenEngine["overrides"]) {
  const actor = buildActorDoc("w1", "G", actorEngine, "act1");
  const token = buildTokenFromActor("w1", "scene1", actor, "link", { x: 0, y: 0 }, { w: 100, h: 100 }, "tok1");
  if (overrides !== undefined) (token.engine as TokenEngine).overrides = overrides;
  return storeWith(actor, token);
}

describe("TokenVisionControl", () => {
  it("renders nothing for a raw token, an instanced token, or when canEdit refuses", () => {
    const raw = buildTokenDoc("w1", "scene1", {
      x: 0, y: 0, w: 100, h: 100, rotation: 0, visual: null, actor_id: null, overrides: null, face: null, elevation: null,
    }, "tok-raw");
    render(TokenVisionControl, {
      context: setAppContextForTest({ documents: storeWith(raw), dispatchIntent: vi.fn(), canEdit: () => true }),
      props: { tokenId: "tok-raw" },
    });
    expect(screen.queryByTestId("token-vision-mode")).toBeNull();

    const instanced = buildTokenFromActor("w1", "scene1", buildActorDoc("w1", "G", actorEngine, "act1"), "instance", { x: 0, y: 0 }, { w: 100, h: 100 }, "tok-i");
    render(TokenVisionControl, {
      context: setAppContextForTest({ documents: storeWith(instanced), dispatchIntent: vi.fn(), canEdit: () => true }),
      props: { tokenId: "tok-i" },
    });
    expect(screen.queryByTestId("token-vision-mode")).toBeNull();

    render(TokenVisionControl, {
      context: setAppContextForTest({ documents: linkedSetup(), dispatchIntent: vi.fn(), canEdit: () => false }),
      props: { tokenId: "tok1" },
    });
    expect(screen.queryByTestId("token-vision-mode")).toBeNull();
  });

  it("shows inherit when no override is stored; custom seeds from the actor's effective list", async () => {
    const dispatchIntent = vi.fn();
    render(TokenVisionControl, {
      context: setAppContextForTest({ documents: linkedSetup(), dispatchIntent, canEdit: () => true }),
      props: { tokenId: "tok1" },
    });
    const select = screen.getByTestId("token-vision-mode") as HTMLSelectElement;
    expect(select.value).toBe("inherit");
    await fireEvent.change(select, { target: { value: "custom" } });
    expect(dispatchIntent).toHaveBeenCalledWith([
      {
        op: "update",
        doc_id: "tok1",
        changes: [
          {
            path: "/engine/overrides",
            old: null, // the RAW stored overrides object (absent on a fresh linked token)
            new: { name: null, visual: null, size: null, shape: null, vision: actorVision, light: null },
          },
        ],
      },
    ]);
  });

  it("a stored empty override list reads as custom (wholesale 'no senses', not inherit)", () => {
    const documents = linkedSetup({ name: null, visual: null, size: null, shape: null, vision: [], light: null });
    render(TokenVisionControl, {
      context: setAppContextForTest({ documents, dispatchIntent: vi.fn(), canEdit: () => true }),
      props: { tokenId: "tok1" },
    });
    expect((screen.getByTestId("token-vision-mode") as HTMLSelectElement).value).toBe("custom");
  });

  it("the list editor commits whole-payload override writes with the raw stored old", async () => {
    const dispatchIntent = vi.fn();
    const prior: VisionAssignment[] = [{ mode: "darkvision", range: 4 }];
    const documents = linkedSetup({ name: null, visual: null, size: null, shape: null, vision: prior, light: null });
    render(TokenVisionControl, {
      context: setAppContextForTest({ documents, dispatchIntent, canEdit: () => true }),
      props: { tokenId: "tok1" },
    });
    expect((screen.getByTestId("token-vision-mode") as HTMLSelectElement).value).toBe("custom");
    await fireEvent.change(screen.getByTestId("vision-range-0"), { target: { value: "9" } });
    expect(dispatchIntent).toHaveBeenCalledWith([
      {
        op: "update",
        doc_id: "tok1",
        changes: [
          {
            path: "/engine/overrides",
            old: { name: null, visual: null, size: null, shape: null, vision: prior, light: null },
            new: { name: null, visual: null, size: null, shape: null, vision: [{ mode: "darkvision", range: 9 }], light: null },
          },
        ],
      },
    ]);
  });

  it("switching back to inherit clears the override (vision: null)", async () => {
    const dispatchIntent = vi.fn();
    const documents = linkedSetup({ name: null, visual: null, size: null, shape: null, vision: actorVision, light: null });
    render(TokenVisionControl, {
      context: setAppContextForTest({ documents, dispatchIntent, canEdit: () => true }),
      props: { tokenId: "tok1" },
    });
    await fireEvent.change(screen.getByTestId("token-vision-mode"), { target: { value: "inherit" } });
    const call = dispatchIntent.mock.calls.at(-1)![0] as WireOperation[];
    if (call[0].op !== "update") throw new Error("expected update");
    expect(call[0].changes[0].new).toMatchObject({ vision: null });
  });
});
