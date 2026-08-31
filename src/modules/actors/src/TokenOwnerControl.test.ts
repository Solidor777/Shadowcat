import { describe, it, expect, vi } from "vitest";
import { render, screen, fireEvent } from "@testing-library/svelte";
import { setAppContextForTest } from "@shadowcat/ui-kit/test";
import { DocumentStore, buildActorDoc, buildTokenFromActor, type WireDocument, type WireOperation } from "@shadowcat/core";
import TokenOwnerControl from "./TokenOwnerControl.svelte";

const cmd = (ops: WireOperation[]) => ({ seq: 1, world_id: "w1", author: "a", ts: 0, ops });
function storeWith(...docs: WireDocument[]): DocumentStore {
  const s = new DocumentStore();
  s.applyCommand(cmd(docs.map((doc) => ({ op: "create" as const, doc }))));
  return s;
}

// RFC 2606 / obviously-synthetic ids: no real user data.
const PLAYER_A = "usr_test_a";
const PLAYER_B = "usr_test_b";
// The default test `t` drops interpolation params, which would make an assertion on
// the resolved-owner hint vacuous (it would match the <option> labels instead). This
// `t` renders the param, so the hint assertions below actually read the RESOLVED owner.
const t = (k: string, p?: Record<string, unknown>) => (p ? `${k}:${Object.values(p).join(",")}` : k);
const members = new Map([
  [PLAYER_A, "MOCK_PLAYER_A"],
  [PLAYER_B, "MOCK_PLAYER_B"],
]);

function actorOwnedBy(owner: string | null): WireDocument {
  const a = buildActorDoc(
    "w1",
    "Goblin",
    { displayName: "Goblin", visual: { kind: "image", asset: "n1" }, size: { w: 1, h: 1 }, shape: "square", faction: null, conditions: [], prototype: false, vision: null, light: null },
    "act1",
  );
  a.owner = owner;
  return a;
}

function linkedToken(actor: WireDocument, override: string | null): WireDocument {
  const t = buildTokenFromActor("w1", "scene1", actor, "link", { x: 0, y: 0 }, { w: 100, h: 100 }, "tok1");
  t.owner = override;
  return t;
}

describe("TokenOwnerControl", () => {
  it("renders nothing without a selected token", () => {
    render(TokenOwnerControl, {
      context: setAppContextForTest({ role: "gm", world: "w1", members, t, documents: storeWith(actorOwnedBy(PLAYER_A)), dispatchIntent: vi.fn() }),
      props: { tokenId: null },
    });
    expect(screen.queryByLabelText("actors.tokenOwner")).toBeNull();
  });

  it("is GM-only: a player never sees the override control", () => {
    const actor = actorOwnedBy(PLAYER_A);
    render(TokenOwnerControl, {
      context: setAppContextForTest({ role: "player", world: "w1", members, t, documents: storeWith(actor, linkedToken(actor, null)), dispatchIntent: vi.fn() }),
      props: { tokenId: "tok1" },
    });
    expect(screen.queryByLabelText("actors.tokenOwner")).toBeNull();
  });

  it("shows the INHERITED owner while the override stays unset", () => {
    const actor = actorOwnedBy(PLAYER_A);
    render(TokenOwnerControl, {
      context: setAppContextForTest({ role: "gm", world: "w1", members, t, documents: storeWith(actor, linkedToken(actor, null)), dispatchIntent: vi.fn() }),
      props: { tokenId: "tok1" },
    });
    // The select itself sits on "inherit" — the token carries no override...
    expect((screen.getByLabelText("actors.tokenOwner") as HTMLSelectElement).value).toBe("");
    // ...while the resolved label names the actor's owner, not "nobody".
    expect(screen.getByText("actors.tokenOwnerEffective:MOCK_PLAYER_A")).toBeTruthy();
  });

  it("shows the OVERRIDE holder, not the actor's owner, when one is set", () => {
    const actor = actorOwnedBy(PLAYER_A);
    render(TokenOwnerControl, {
      context: setAppContextForTest({ role: "gm", world: "w1", members, t, documents: storeWith(actor, linkedToken(actor, PLAYER_B)), dispatchIntent: vi.fn() }),
      props: { tokenId: "tok1" },
    });
    expect((screen.getByLabelText("actors.tokenOwner") as HTMLSelectElement).value).toBe(PLAYER_B);
    expect(screen.getByText("actors.tokenOwnerEffective:MOCK_PLAYER_B")).toBeTruthy();
    expect(screen.queryByText("actors.tokenOwnerEffective:MOCK_PLAYER_A")).toBeNull();
  });

  it("dispatches an /owner override with the RAW stored value as the OCC pre-image", async () => {
    const actor = actorOwnedBy(PLAYER_A);
    const dispatchIntent = vi.fn();
    render(TokenOwnerControl, {
      // Token has NO override: `old` must be null, NOT the inherited PLAYER_A —
      // a resolved `old` would fail the server's field-level OCC check.
      context: setAppContextForTest({ role: "gm", world: "w1", members, t, documents: storeWith(actor, linkedToken(actor, null)), dispatchIntent }),
      props: { tokenId: "tok1" },
    });
    await fireEvent.change(screen.getByLabelText("actors.tokenOwner"), { target: { value: PLAYER_B } });
    expect(dispatchIntent).toHaveBeenCalledWith([
      { op: "update", doc_id: "tok1", changes: [{ path: "/owner", old: null, new: PLAYER_B }] },
    ]);
  });

  it("clearing the override writes null (back to inheritance), never a sentinel", async () => {
    const actor = actorOwnedBy(PLAYER_A);
    const dispatchIntent = vi.fn();
    render(TokenOwnerControl, {
      context: setAppContextForTest({ role: "gm", world: "w1", members, t, documents: storeWith(actor, linkedToken(actor, PLAYER_B)), dispatchIntent }),
      props: { tokenId: "tok1" },
    });
    await fireEvent.change(screen.getByLabelText("actors.tokenOwner"), { target: { value: "" } });
    expect(dispatchIntent).toHaveBeenCalledWith([
      { op: "update", doc_id: "tok1", changes: [{ path: "/owner", old: PLAYER_B, new: null }] },
    ]);
  });
});
