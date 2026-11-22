import { describe, it, expect, vi } from "vitest";
import { render, screen, fireEvent } from "@testing-library/svelte";
import { setAppContextForTest } from "@shadowcat/ui-kit/test";
import { DocumentStore, buildTokenDoc, type TokenEngine, type WireDocument } from "@shadowcat/core";
import TokenIdentityHiddenControl from "./TokenIdentityHiddenControl.svelte";

function tokenWith(over: Partial<TokenEngine> = {}): DocumentStore {
  const doc: WireDocument = buildTokenDoc("w1", "scene1", {
    x: 0, y: 0, w: 100, h: 100, rotation: 0, visual: null, actor_id: null, overrides: null, face: null, elevation: null, lock_rotation: false, identity_hidden: false, ...over,
  }, "tok1");
  const s = new DocumentStore();
  s.applyCommand({ seq: 1, world_id: "w1", author: "a", ts: 0, ops: [{ op: "create", doc }] });
  return s;
}

describe("TokenIdentityHiddenControl", () => {
  it("renders nothing for a non-GM, even one who can edit the token, and for no selection", () => {
    render(TokenIdentityHiddenControl, {
      context: setAppContextForTest({ documents: tokenWith(), dispatchIntent: vi.fn(), role: "player", canEdit: () => true }),
      props: { tokenId: "tok1" },
    });
    expect(screen.queryByTestId("token-identity-hidden")).toBeNull();
    render(TokenIdentityHiddenControl, {
      context: setAppContextForTest({ documents: tokenWith(), dispatchIntent: vi.fn(), role: "gm" }),
      props: { tokenId: null },
    });
    expect(screen.queryByTestId("token-identity-hidden")).toBeNull();
  });

  it("a GM toggling it writes /engine/identity_hidden with the stored value as old", async () => {
    const dispatchIntent = vi.fn();
    render(TokenIdentityHiddenControl, {
      context: setAppContextForTest({ documents: tokenWith(), dispatchIntent, role: "gm" }),
      props: { tokenId: "tok1" },
    });
    const box = screen.getByTestId("token-identity-hidden") as HTMLInputElement;
    expect(box.checked).toBe(false);
    await fireEvent.click(box);
    expect(dispatchIntent).toHaveBeenCalledWith([
      { op: "update", doc_id: "tok1", changes: [{ path: "/engine/identity_hidden", old: false, new: true }] },
    ]);
  });

  it("a stored true reads as checked and unchecking writes false", async () => {
    const dispatchIntent = vi.fn();
    render(TokenIdentityHiddenControl, {
      context: setAppContextForTest({ documents: tokenWith({ identity_hidden: true }), dispatchIntent, role: "gm" }),
      props: { tokenId: "tok1" },
    });
    const box = screen.getByTestId("token-identity-hidden") as HTMLInputElement;
    expect(box.checked).toBe(true);
    await fireEvent.click(box);
    expect(dispatchIntent).toHaveBeenCalledWith([
      { op: "update", doc_id: "tok1", changes: [{ path: "/engine/identity_hidden", old: true, new: false }] },
    ]);
  });
});
