import { describe, it, expect } from "vitest";
import { render, fireEvent } from "@testing-library/svelte";
import { ContributionRegistry, sheetContract, DocumentStore, envelope } from "@shadowcat/core";
import { setAppContextForTest } from "@shadowcat/ui-kit/test";
import ActorSheet from "./ActorSheet.svelte";
import { sheetActor } from "./index";

/** Builds an actor doc on the three-band shape: `name` (envelope) is pulled out of the
 * legacy flat fixture object, the rest lands in `engine` (the actor's engine-owned body). */
function storeWith(fields: Record<string, unknown>) {
  const { name, ...engine } = fields;
  const s = new DocumentStore();
  s.applyCommand({
    seq: 1, world_id: "w1", author: "u", ts: 0,
    ops: [{ op: "create", doc: envelope("w1", "actor", null, {}, "a1", engine, (name as string | null) ?? null) }],
  });
  return s;
}

/** Builds a TOKEN doc ("t1") with an embedded, instanced actor copy at `/embedded/actor/0`
 * (mirrors `resolveDocRef`'s instanced-token resolution: `docId` resolves to the token,
 * `systemPrefix` to `/embedded/actor/0/system`). */
function storeWithEmbeddedActor(fields: Record<string, unknown>) {
  const { name, ...engine } = fields;
  const s = new DocumentStore();
  const embeddedActor = envelope("w1", "actor", null, {}, "embedded-a0", engine, (name as string | null) ?? null);
  const token = envelope("w1", "token", "scene1", {}, "t1", { x: 0, y: 0, w: 100, h: 100, rotation: 0, visual: null, actor_id: null, overrides: null, face: null }, null);
  token.embedded = { actor: [embeddedActor] };
  s.applyCommand({ seq: 1, world_id: "w1", author: "u", ts: 0, ops: [{ op: "create", doc: token }] });
  return s;
}

describe("module-sheet-actor registration", () => {
  it("registers ActorSheet under shadowcat.sheet:actor at priority 0", () => {
    const contributions = new ContributionRegistry();
    // Mirrors the real ModuleContext.contributions wrapper (`ModuleRegistry.activate`): a
    // 1-arg contribute(c) closure that auto-injects the module id — the module entry never
    // self-declares it, matching every other first-party module.
    sheetActor.register({
      contributions: { contribute: (c: Parameters<typeof contributions.contribute>[0]) => contributions.contribute(c, { module: "sheet-actor" }) },
    } as never);
    const entry = contributions.entriesFor(sheetContract("actor"))[0];
    expect(entry?.contribution.sheet?.priority).toBe(0);
    expect(entry?.module).toBe("sheet-actor");
  });
});

describe("ActorSheet edits", () => {
  it("edits the name with the real pre-image", async () => {
    const calls: unknown[] = [];
    const documents = storeWith({ name: "Goblin", displayName: "Creature", faction: null, shape: "square", size: { w: 1, h: 1 }, conditions: [], prototype: false, visual: { kind: "image", asset: "x" } });
    const context = setAppContextForTest({ documents, dispatchIntent: (ops) => calls.push(ops), canEdit: () => true });
    const { getByLabelText } = render(ActorSheet, { props: { docId: "a1", systemPrefix: "/system", close: () => {} }, context });
    await fireEvent.change(getByLabelText("sheetActor.name"), { target: { value: "Orc" } });
    expect(calls).toEqual([[{ op: "update", doc_id: "a1", changes: [{ path: "/name", old: "Goblin", new: "Orc" }] }]]);
  });

  it("disables controls for a non-editor (canEdit false)", () => {
    const documents = storeWith({ name: "Goblin", displayName: "Creature", faction: null, shape: "square", size: { w: 1, h: 1 }, conditions: [], prototype: false, visual: { kind: "image", asset: "x" } });
    const context = setAppContextForTest({ documents, canEdit: () => false, role: "player" });
    const { getByLabelText } = render(ActorSheet, { props: { docId: "a1", systemPrefix: "/system", close: () => {} }, context });
    expect((getByLabelText("sheetActor.name") as HTMLInputElement).disabled).toBe(true);
  });

  it("editing sizeW dispatches the real current {w,h} pair as old, preserving h in new", async () => {
    const calls: unknown[] = [];
    const documents = storeWith({ name: "Goblin", displayName: "Creature", faction: null, shape: "square", size: { w: 2, h: 3 }, conditions: [], prototype: false, visual: { kind: "image", asset: "x" } });
    const context = setAppContextForTest({ documents, dispatchIntent: (ops) => calls.push(ops), canEdit: () => true });
    const { getByLabelText } = render(ActorSheet, { props: { docId: "a1", systemPrefix: "/system", close: () => {} }, context });
    await fireEvent.change(getByLabelText("sheetActor.sizeW"), { target: { value: "5" } });
    expect(calls).toEqual([[{ op: "update", doc_id: "a1", changes: [{ path: "/engine/size", old: { w: 2, h: 3 }, new: { w: 5, h: 3 } }] }]]);
  });

  // Pins: a second edit in the same rendered instance reads the FIRST edit's result as `old`,
  // not the doc snapshot frozen at first render — `ctx.documents.get()` must be re-derived on
  // each dispatch, or the second dispatch's `old` is still the pre-edit doc's field.
  it("a second edit in the same instance dispatches a fresh old reflecting the first edit", async () => {
    const calls: unknown[] = [];
    const documents = storeWith({ name: "Goblin", displayName: "Creature", faction: null, shape: "square", size: { w: 1, h: 1 }, conditions: [], prototype: false, visual: { kind: "image", asset: "x" } });
    const dispatchIntent = (ops: unknown) => {
      calls.push(ops);
      // Simulate the server confirming the intent: apply it back onto the store as an
      // authoritative command so the reactive subscribers (subscribe()-calling deriveds) re-run.
      const changes = (ops as { changes: { path: string; new: unknown }[] }[])[0].changes;
      documents.applyCommand({
        seq: documents.appliedSeq + 1, world_id: "w1", author: "u", ts: 0,
        ops: [{ op: "update", doc_id: "a1", changes }],
      });
    };
    const context = setAppContextForTest({ documents, dispatchIntent, canEdit: () => true });
    const { getByLabelText } = render(ActorSheet, { props: { docId: "a1", systemPrefix: "/system", close: () => {} }, context });

    await fireEvent.change(getByLabelText("sheetActor.name"), { target: { value: "Orc" } });
    await fireEvent.change(getByLabelText("sheetActor.name"), { target: { value: "Orc Warlord" } });

    expect(calls).toEqual([
      [{ op: "update", doc_id: "a1", changes: [{ path: "/name", old: "Goblin", new: "Orc" }] }],
      [{ op: "update", doc_id: "a1", changes: [{ path: "/name", old: "Orc", new: "Orc Warlord" }] }],
    ]);
  });

  // Combines the compound-field and repeat-edit scenarios. Pins: editing sizeH after sizeW
  // reads the FIRST edit's w back as old/new from a re-derived doc, not a frozen pre-render
  // snapshot that would revert w to its original value on the second dispatch.
  it("editing sizeH after sizeW preserves the first edit's w, not the frozen pre-render value", async () => {
    const calls: unknown[] = [];
    const documents = storeWith({ name: "Goblin", displayName: "Creature", faction: null, shape: "square", size: { w: 1, h: 1 }, conditions: [], prototype: false, visual: { kind: "image", asset: "x" } });
    const dispatchIntent = (ops: unknown) => {
      calls.push(ops);
      const changes = (ops as { changes: { path: string; new: unknown }[] }[])[0].changes;
      documents.applyCommand({
        seq: documents.appliedSeq + 1, world_id: "w1", author: "u", ts: 0,
        ops: [{ op: "update", doc_id: "a1", changes }],
      });
    };
    const context = setAppContextForTest({ documents, dispatchIntent, canEdit: () => true });
    const { getByLabelText } = render(ActorSheet, { props: { docId: "a1", systemPrefix: "/system", close: () => {} }, context });

    await fireEvent.change(getByLabelText("sheetActor.sizeW"), { target: { value: "5" } });
    await fireEvent.change(getByLabelText("sheetActor.sizeH"), { target: { value: "9" } });

    expect(calls).toEqual([
      [{ op: "update", doc_id: "a1", changes: [{ path: "/engine/size", old: { w: 1, h: 1 }, new: { w: 5, h: 1 } }] }],
      [{ op: "update", doc_id: "a1", changes: [{ path: "/engine/size", old: { w: 5, h: 1 }, new: { w: 5, h: 9 } }] }],
    ]);
  });

  // An instanced token's embedded actor copy: `docId` resolves to the TOKEN, `systemPrefix`
  // to `/embedded/actor/0/system`. Asserts `basePrefix`'s `/system`-suffix strip correctly
  // derives `/embedded/actor/0/name` as the sibling name path for this shape too.
  it("edits the name of an embedded (instanced-token) actor copy, not a top-level actor", async () => {
    const calls: unknown[] = [];
    const documents = storeWithEmbeddedActor({
      name: "Goblin", displayName: "Creature", faction: null, shape: "square",
      size: { w: 1, h: 1 }, conditions: [], prototype: false, visual: { kind: "image", asset: "x" },
    });
    const context = setAppContextForTest({ documents, dispatchIntent: (ops) => calls.push(ops), canEdit: () => true });
    const { getByLabelText } = render(ActorSheet, {
      props: { docId: "t1", systemPrefix: "/embedded/actor/0/system", close: () => {} },
      context,
    });
    expect((getByLabelText("sheetActor.name") as HTMLInputElement).value).toBe("Goblin");
    await fireEvent.change(getByLabelText("sheetActor.name"), { target: { value: "Orc" } });
    expect(calls).toEqual([
      [{ op: "update", doc_id: "t1", changes: [{ path: "/embedded/actor/0/name", old: "Goblin", new: "Orc" }] }],
    ]);
  });
});

describe("ActorSheet carried light", () => {
  it("toggling the carried-light checkbox writes /engine/light with the raw stored pre-image", async () => {
    const calls: unknown[] = [];
    const documents = storeWith({ name: "Goblin", displayName: "Creature", faction: null, shape: "square", size: { w: 1, h: 1 }, conditions: [], prototype: false, visual: { kind: "image", asset: "x" }, light: null });
    const context = setAppContextForTest({ documents, dispatchIntent: (ops) => calls.push(ops), canEdit: () => true });
    const { getByLabelText } = render(ActorSheet, { props: { docId: "a1", systemPrefix: "/system", close: () => {} }, context });
    await fireEvent.click(getByLabelText("actors.carriedLight"));
    expect(calls).toEqual([
      [
        {
          op: "update",
          doc_id: "a1",
          changes: [
            {
              path: "/engine/light",
              old: null,
              new: { color: "#ffd9a0", intensity: 1, brightRadius: 2, dimRadius: 6, falloff: null, enabled: true },
            },
          ],
        },
      ],
    ]);
  });

  it("the field editor edits an existing emission through setEngine (whole-payload write)", async () => {
    const calls: unknown[] = [];
    const torch = { color: "#ffcc66", intensity: 1, brightRadius: 2, dimRadius: 4, falloff: null, enabled: true };
    const documents = storeWith({ name: "Goblin", displayName: "Creature", faction: null, shape: "square", size: { w: 1, h: 1 }, conditions: [], prototype: false, visual: { kind: "image", asset: "x" }, light: torch });
    const context = setAppContextForTest({ documents, dispatchIntent: (ops) => calls.push(ops), canEdit: () => true });
    const { getByTestId } = render(ActorSheet, { props: { docId: "a1", systemPrefix: "/system", close: () => {} }, context });
    await fireEvent.change(getByTestId("emission-bright"), { target: { value: "3" } });
    expect(calls).toEqual([
      [{ op: "update", doc_id: "a1", changes: [{ path: "/engine/light", old: torch, new: { ...torch, brightRadius: 3 } }] }],
    ]);
  });
});

describe("ActorSheet vision assignments", () => {
  it("the list editor writes /engine/vision with the raw stored list as old", async () => {
    const calls: unknown[] = [];
    const stored = [{ mode: "darkvision", range: 12 }];
    const documents = storeWith({ name: "Goblin", displayName: "Creature", faction: null, shape: "square", size: { w: 1, h: 1 }, conditions: [], prototype: false, visual: { kind: "image", asset: "x" }, vision: stored });
    const context = setAppContextForTest({ documents, dispatchIntent: (ops) => calls.push(ops), canEdit: () => true });
    const { getByTestId } = render(ActorSheet, { props: { docId: "a1", systemPrefix: "/system", close: () => {} }, context });
    await fireEvent.change(getByTestId("vision-range-0"), { target: { value: "20" } });
    expect(calls).toEqual([
      [{ op: "update", doc_id: "a1", changes: [{ path: "/engine/vision", old: stored, new: [{ mode: "darkvision", range: 20 }] }] }],
    ]);
  });

  it("removing the only assignment normalizes the write to null", async () => {
    const calls: unknown[] = [];
    const stored = [{ mode: "tremorsense", range: null }];
    const documents = storeWith({ name: "Goblin", displayName: "Creature", faction: null, shape: "square", size: { w: 1, h: 1 }, conditions: [], prototype: false, visual: { kind: "image", asset: "x" }, vision: stored });
    const context = setAppContextForTest({ documents, dispatchIntent: (ops) => calls.push(ops), canEdit: () => true });
    const { getByTestId } = render(ActorSheet, { props: { docId: "a1", systemPrefix: "/system", close: () => {} }, context });
    await fireEvent.click(getByTestId("vision-remove-0"));
    expect(calls).toEqual([
      [{ op: "update", doc_id: "a1", changes: [{ path: "/engine/vision", old: stored, new: null }] }],
    ]);
  });
});

describe("ActorSheet movement tags", () => {
  const goblin = (movement?: string[]) => ({
    name: "Goblin", displayName: "Creature", faction: null, shape: "square",
    size: { w: 1, h: 1 }, conditions: [], prototype: false, visual: { kind: "image", asset: "x" },
    ...(movement !== undefined ? { movement } : {}),
  });

  it("a tag toggle writes /engine/movement with the raw stored list as old", async () => {
    const calls: unknown[] = [];
    const documents = storeWith(goblin(["flying"]));
    const context = setAppContextForTest({ documents, dispatchIntent: (ops) => calls.push(ops), canEdit: () => true });
    const { getByTestId } = render(ActorSheet, { props: { docId: "a1", systemPrefix: "/system", close: () => {} }, context });
    await fireEvent.click(getByTestId("movement-toggle-incorporeal"));
    expect(calls).toEqual([
      [{ op: "update", doc_id: "a1", changes: [{ path: "/engine/movement", old: ["flying"], new: ["flying", "incorporeal"] }] }],
    ]);
  });

  it("untoggling the last tag writes [] (a required non-null array — never null)", async () => {
    const calls: unknown[] = [];
    const documents = storeWith(goblin(["flying"]));
    const context = setAppContextForTest({ documents, dispatchIntent: (ops) => calls.push(ops), canEdit: () => true });
    const { getByTestId } = render(ActorSheet, { props: { docId: "a1", systemPrefix: "/system", close: () => {} }, context });
    await fireEvent.click(getByTestId("movement-toggle-flying"));
    expect(calls).toEqual([
      [{ op: "update", doc_id: "a1", changes: [{ path: "/engine/movement", old: ["flying"], new: [] }] }],
    ]);
  });

  it("a genuinely absent movement key reads as empty and writes with old: null", async () => {
    const calls: unknown[] = [];
    const documents = storeWith(goblin());
    const context = setAppContextForTest({ documents, dispatchIntent: (ops) => calls.push(ops), canEdit: () => true });
    const { getByTestId } = render(ActorSheet, { props: { docId: "a1", systemPrefix: "/system", close: () => {} }, context });
    expect(getByTestId("movement-toggle-flying").getAttribute("aria-pressed")).toBe("false");
    await fireEvent.click(getByTestId("movement-toggle-flying"));
    expect(calls).toEqual([
      [{ op: "update", doc_id: "a1", changes: [{ path: "/engine/movement", old: null, new: ["flying"] }] }],
    ]);
  });

  it("disables the tag controls for a non-editor (canEdit false)", () => {
    const documents = storeWith(goblin(["flying"]));
    const context = setAppContextForTest({ documents, canEdit: () => false, role: "player" });
    const { getByTestId } = render(ActorSheet, { props: { docId: "a1", systemPrefix: "/system", close: () => {} }, context });
    expect((getByTestId("movement-toggle-flying") as HTMLButtonElement).disabled).toBe(true);
  });
});
