// @vitest-environment node
// Plain document reads; no DOM.
import { test, expect } from "vitest";
import { DocumentStore, buildTokenDoc } from "@shadowcat/core";
import { facingWedgeVisibleTo } from "./facing-wedge";

const OWNER = "usr_test_owner";
const OTHER = "usr_test_other";

/** A store holding one token owned by `OWNER`, and that token. */
function fixture() {
  const token = buildTokenDoc("w1", "s1", {
    x: 0, y: 0, w: 10, h: 10, rotation: 0,
    visual: { kind: "image", asset: "a" }, actor_id: null, overrides: null, face: null, elevation: null, lock_rotation: false, identity_hidden: false,
  }, "t1");
  token.owner = OWNER;
  const store = new DocumentStore();
  store.applyCommand({ seq: 1, world_id: "w1", author: "a", ts: 0, ops: [{ op: "create", doc: token }] });
  return { token, store };
}

test("the GM sees every token's facing wedge", () => {
  const { token, store } = fixture();
  expect(facingWedgeVisibleTo("gm", OTHER, token, store)).toBe(true);
});

test("a player sees the wedge of their own token only", () => {
  const { token, store } = fixture();
  expect(facingWedgeVisibleTo("player", OWNER, token, store)).toBe(true);
  expect(facingWedgeVisibleTo("player", OTHER, token, store)).toBe(false);
});
