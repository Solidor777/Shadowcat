import { describe, it, expect } from "vitest";
import { snapshotBase, stampInstance, type StampOpts, findInstances, syncState } from "./templates";
import type { WireDocument } from "./wire";
import { normalizeBase, type MergeBase } from "./merge";

function doc(over: Partial<WireDocument> & { id: string }): WireDocument {
  return {
    id: over.id,
    scope: over.scope ?? { kind: "world", world_id: "w1" },
    doc_type: over.doc_type ?? "actor",
    schema_version: 1,
    name: over.name ?? null,
    source: over.source ?? null,
    owner: over.owner ?? null,
    permissions: over.permissions ?? { default: "none", users: {}, property_overrides: {}, capabilities: { by_role: {}, by_user: {} }, gm_role: null },
    embedded: over.embedded ?? {},
    parent_id: over.parent_id ?? null,
    engine: over.engine,
    system: over.system ?? {},
    created_at: 0,
    updated_at: 0,
  };
}

const opts: StampOpts = { worldId: "w1", ownerId: "u-self", parentId: "scene-1" };

describe("snapshotBase", () => {
  it("captures bands + embedded children keyed by their source.id", () => {
    const child = doc({ id: "ic", source: { id: "tc", pack: null, version: 2 }, name: "Kid", system: { hp: 3 } });
    const d = doc({ id: "C", name: "Inst", engine: { hp: 9 }, system: { a: 1 }, embedded: { items: [child] } });
    const snap = snapshotBase(d);
    expect(snap).toEqual<MergeBase>({
      name: "Inst",
      engine: { hp: 9 },
      system: { a: 1 },
      embedded: { items: [{ sourceId: "tc", name: "Kid", engine: null, system: { hp: 3 }, embedded: {}, propertyOverrides: {} }] },
      property_overrides: {},
    });
  });

  it("records the mergeable-band policy at every depth, never a /base policy", () => {
    const child = doc({
      id: "ic", source: { id: "tc", pack: null, version: 1 },
      permissions: { default: "none", users: {}, property_overrides: { "/engine/hp": "owner_or_gm" }, capabilities: { by_role: {}, by_user: {} }, gm_role: null },
    });
    const d = doc({
      id: "C", embedded: { items: [child] },
      permissions: {
        default: "none", users: {}, capabilities: { by_role: {}, by_user: {} }, gm_role: null,
        property_overrides: { "/system/secret": "gm_only", "/name": "owner_or_gm", "/base/system/x": "gm_only" },
      },
    });
    const snap = snapshotBase(d);
    expect(snap.property_overrides).toEqual({ "/system/secret": "gm_only", "/name": "owner_or_gm" });
    expect(snap.embedded.items[0].propertyOverrides).toEqual({ "/engine/hp": "owner_or_gm" });
  });

  it("deep-clones so the snapshot does not alias the document", () => {
    const d = doc({ id: "C", system: { nested: { x: 1 } } });
    const snap = snapshotBase(d);
    (d.system as { nested: { x: number } }).nested.x = 99;
    expect((snap.system as { nested: { x: number } }).nested.x).toBe(1);
  });
});

describe("stampInstance", () => {
  it("creates a new doc: fresh id, initiator owner/parent, source pointing at the template", () => {
    const tmpl = doc({ id: "T", name: "Preset", owner: "gm", system: { hp: 10 } });
    const inst = stampInstance(tmpl, opts);
    expect(inst.id).not.toBe("T");
    expect(inst.owner).toBe("u-self");
    expect(inst.parent_id).toBe("scene-1");
    expect(inst.source).toEqual({ id: "T", pack: null, version: 1 });
    expect(inst.system).toEqual({ hp: 10 });
  });

  it("deep-clone independence: nested bands are not aliased (recursively)", () => {
    const tmplChild = doc({ id: "tc", system: { deep: { v: 1 } } });
    const tmpl = doc({ id: "T", system: { s: { v: 1 } }, embedded: { items: [tmplChild] } });
    const inst = stampInstance(tmpl, opts);
    expect(inst.system).not.toBe(tmpl.system);
    expect(inst.embedded.items[0].system).not.toBe(tmplChild.system);
    (tmpl.system as { s: { v: number } }).s.v = 42;
    (tmplChild.system as { deep: { v: number } }).deep.v = 42;
    expect((inst.system as { s: { v: number } }).s.v).toBe(1);
    expect((inst.embedded.items[0].system as { deep: { v: number } }).deep.v).toBe(1);
  });

  it("recursively assigns embedded children fresh ids + source = template child id", () => {
    const tmplChild = doc({ id: "tc", name: "Item" });
    const tmpl = doc({ id: "T", embedded: { items: [tmplChild] } });
    const inst = stampInstance(tmpl, opts);
    const sc = inst.embedded.items[0];
    expect(sc.id).not.toBe("tc");
    expect(sc.source).toEqual({ id: "tc", pack: null, version: 1 });
  });

  it("sets base to a snapshot keyed by the new children's source.id (correlation)", () => {
    const tmpl = doc({ id: "T", name: "P", system: { hp: 1 }, embedded: { items: [doc({ id: "tc", system: { k: 1 } })] } });
    const inst = stampInstance(tmpl, opts);
    const base = inst.base as MergeBase;
    expect(base.name).toBe("P");
    expect(base.system).toEqual({ hp: 1 });
    expect(base.embedded.items[0].sourceId).toBe("tc"); // == the stamped child's source.id
    expect(base.embedded.items[0].system).toEqual({ k: 1 });
  });

  it("copies the compendium pack into source when the template is compendium-scoped", () => {
    const tmpl = doc({ id: "T", scope: { kind: "compendium", pack: "example-system" } });
    const inst = stampInstance(tmpl, opts);
    expect(inst.source).toEqual({ id: "T", pack: "example-system", version: 1 });
  });

  it("does not inherit the template's own provenance pack when the template itself is world-scoped", () => {
    // The template is itself an instance of a compendium item, so its own `source.pack` is set —
    // but that provenance belongs to the TEMPLATE, not to this stamp.
    const tmpl = doc({ id: "T", scope: { kind: "world", world_id: "w1" }, source: { id: "orig", pack: "example-system", version: 3 } });
    const inst = stampInstance(tmpl, opts);
    expect(inst.source).toEqual({ id: "T", pack: null, version: 3 });
  });

  it("gives the stamped instance fresh created_at/updated_at, not the template's", () => {
    const tmpl = doc({ id: "T", name: "Old" });
    (tmpl as { created_at: number }).created_at = 12345;
    (tmpl as { updated_at: number }).updated_at = 12345;
    const inst = stampInstance(tmpl, opts);
    expect(inst.created_at).not.toBe(tmpl.created_at);
    expect(inst.updated_at).not.toBe(tmpl.updated_at);
    expect(inst.created_at).toBeGreaterThan(tmpl.created_at);
    expect(inst.updated_at).toBeGreaterThan(tmpl.updated_at);
  });

  it("clones opts.permissions rather than aliasing the caller's object", () => {
    const tmpl = doc({ id: "T" });
    const perms: WireDocument["permissions"] = {
      default: "none",
      users: { u1: "owner" },
      property_overrides: {},
      capabilities: { by_role: {}, by_user: {} },
      gm_role: null,
    };
    const inst = stampInstance(tmpl, { ...opts, permissions: perms });
    perms.users.u1 = "observer";
    expect(inst.permissions.users.u1).toBe("owner");
  });

  it("deep-clone independence holds 2 levels deep (grandchild)", () => {
    const grandchild = doc({ id: "gc", system: { deep: { v: 1 } } });
    const child = doc({ id: "tc", system: { v: 1 }, embedded: { items: [grandchild] } });
    const tmpl = doc({ id: "T", embedded: { items: [child] } });
    const inst = stampInstance(tmpl, opts);
    const instChild = inst.embedded.items[0];
    const instGrandchild = instChild.embedded.items[0];
    (grandchild.system as { deep: { v: number } }).deep.v = 99;
    expect((instGrandchild.system as { deep: { v: number } }).deep.v).toBe(1);
  });
});

describe("findInstances", () => {
  it("returns only docs whose source.id is the template id", () => {
    const a = doc({ id: "a", source: { id: "T", pack: null, version: 1 } });
    const b = doc({ id: "b", source: { id: "OTHER", pack: null, version: 1 } });
    const c = doc({ id: "c" });
    expect(findInstances("T", [a, b, c]).map((d) => d.id)).toEqual(["a"]);
  });
});

describe("syncState", () => {
  it("none when the doc has no source, or the template is not in store", () => {
    expect(syncState(doc({ id: "C" }), undefined)).toBe("none");
    const child = doc({ id: "C", source: { id: "T", pack: null, version: 1 } });
    expect(syncState(child, undefined)).toBe("none");
  });

  it("up_to_date when base equals the template's current snapshot", () => {
    const tmpl = doc({ id: "T", name: "T", system: { hp: 1 } });
    const child = doc({ id: "C", source: { id: "T", pack: null, version: 1 } });
    child.base = { name: "T", engine: null, system: { hp: 1 }, embedded: {}, property_overrides: {} };
    expect(syncState(child, tmpl)).toBe("up_to_date");
  });

  // The server's egress of a stored base to each seat, pinned as explicit fixtures: the stored
  // snapshot is the FULL template (`{ hp, gm_secret, owner_note }`, the template's policy recorded
  // verbatim); a player who owns the instance but not the template receives it minus the recorded
  // `gm_only` and `owner_or_gm` paths, and receives the template minus the same paths under its own
  // policy; a GM receives both whole. The policy maps themselves are never redacted.
  const fullTemplate = () =>
    doc({
      id: "T", name: "T", system: { hp: 11, gm_secret: "S2", owner_note: "N2" },
      permissions: {
        default: "observer", users: {}, capabilities: { by_role: {}, by_user: {} }, gm_role: null,
        property_overrides: { "/system/gm_secret": "gm_only", "/system/owner_note": "owner_or_gm" },
      },
    });
  const playerTemplateView = () => {
    const t = fullTemplate();
    t.system = { hp: 11 };
    return t;
  };
  const playerBaseView = (): MergeBase => ({
    name: "T",
    engine: null,
    system: { hp: 11 },
    embedded: {},
    property_overrides: { "/system/gm_secret": "gm_only", "/system/owner_note": "owner_or_gm" },
  });
  const gmBaseView = (): MergeBase => ({
    name: "T",
    engine: null,
    system: { hp: 11, gm_secret: "S2", owner_note: "N2" },
    embedded: {},
    property_overrides: { "/system/gm_secret": "gm_only", "/system/owner_note": "owner_or_gm" },
  });

  it("parity: a player's redacted base against their redacted template reads up_to_date", () => {
    const child = doc({ id: "C", source: { id: "T", pack: null, version: 1 } });
    child.base = playerBaseView();
    expect(syncState(child, playerTemplateView())).toBe("up_to_date");
  });

  it("parity: a GM's full base against the full template reads up_to_date", () => {
    const child = doc({ id: "C", source: { id: "T", pack: null, version: 1 } });
    child.base = gmBaseView();
    expect(syncState(child, fullTemplate())).toBe("up_to_date");
  });

  it("parity: a genuine template edit still flips both seats", () => {
    const edited = fullTemplate();
    edited.system = { hp: 12, gm_secret: "S2", owner_note: "N2" };
    const gmChild = doc({ id: "C", source: { id: "T", pack: null, version: 1 } });
    gmChild.base = gmBaseView();
    expect(syncState(gmChild, edited)).toBe("template_changed");
    const playerEdited = playerTemplateView();
    playerEdited.system = { hp: 12 };
    const playerChild = doc({ id: "C2", source: { id: "T", pack: null, version: 1 } });
    playerChild.base = playerBaseView();
    expect(syncState(playerChild, playerEdited)).toBe("template_changed");
  });

  it("parity: a stripped snapshot key and a nulled template band read as one value", () => {
    // `/name` hidden: egress NULLS the template's band but REMOVES the key from the snapshot.
    const tmpl = doc({ id: "T", name: null, system: { hp: 1 } });
    tmpl.permissions.property_overrides = { "/name": "gm_only" };
    const child = doc({ id: "C", source: { id: "T", pack: null, version: 1 } });
    child.base = { engine: null, system: { hp: 1 }, embedded: {}, property_overrides: { "/name": "gm_only" } };
    expect(syncState(child, tmpl)).toBe("up_to_date");
  });

  it("a template policy change reads template_changed on every seat; the merge that propagates it clears it", () => {
    // The stored policy is the template's policy at last sync, verbatim, so hiding one more path
    // on the template is a template change for the GM and the player alike — until a merge writes
    // the snapshot with the new policy recorded.
    const hidden = fullTemplate();
    hidden.permissions.property_overrides["/system/hp"] = "owner_or_gm";
    const gmChild = doc({ id: "C", source: { id: "T", pack: null, version: 1 } });
    gmChild.base = gmBaseView();
    expect(syncState(gmChild, hidden)).toBe("template_changed");
    const playerHidden = playerTemplateView();
    playerHidden.permissions.property_overrides["/system/hp"] = "owner_or_gm";
    const playerChild = doc({ id: "C2", source: { id: "T", pack: null, version: 1 } });
    playerChild.base = playerBaseView();
    expect(syncState(playerChild, playerHidden)).toBe("template_changed");

    const refreshed = gmBaseView();
    refreshed.property_overrides["/system/hp"] = "owner_or_gm";
    gmChild.base = refreshed;
    expect(syncState(gmChild, hidden)).toBe("up_to_date");
    const playerRefreshed = playerBaseView();
    playerRefreshed.property_overrides["/system/hp"] = "owner_or_gm";
    playerChild.base = playerRefreshed;
    expect(syncState(playerChild, playerHidden)).toBe("up_to_date");
  });

  it("a malformed stored base never reads up_to_date — it falls back exactly as an absent base does", () => {
    // `child.base` is present but missing the STRUCTURAL `property_overrides` key (never subject
    // to redaction, so its absence is not a legitimate wire shape), and its `name`/`system` happen
    // to already match the template's current snapshot. A coalescing reader would default the
    // missing key and read this as caught up; the server itself never accepts such a value as a
    // legitimate snapshot (`check_base_node_shape`), so the client must not either — it falls back
    // to the child's own current bands, the same treatment `syncState` already gives a genuinely
    // absent `base`.
    const tmpl = doc({ id: "T", name: "T", system: { hp: 5 } });
    const child = doc({ id: "C", source: { id: "T", pack: null, version: 1 }, name: "T", system: { hp: 999 } });
    child.base = { name: "T", engine: null, system: { hp: 5 }, embedded: {} };
    expect(syncState(child, tmpl)).toBe("template_changed");
  });

  it("template_changed when the template diverged from base (ignoring placement)", () => {
    const tmpl = doc({ id: "T", doc_type: "token", name: "T", engine: { x: 5, hp: 9 }, system: {} });
    const child = doc({ id: "C", doc_type: "token", source: { id: "T", pack: null, version: 1 } });
    // base engine hp:1; template hp:9 → changed. But an x-only move must NOT count.
    child.base = { name: "T", engine: { x: 0, hp: 1 }, system: {}, embedded: {}, property_overrides: {} };
    expect(syncState(child, tmpl)).toBe("template_changed");
    child.base = { name: "T", engine: { x: 0, hp: 9 }, system: {}, embedded: {}, property_overrides: {} };
    expect(syncState(child, tmpl)).toBe("up_to_date"); // only x differs → excluded
  });

  it("parity: an embedded record's recorded policy reads up_to_date on every seat", () => {
    // A record-level override (`/engine/hp` hidden from non-GMs) is recorded on the EMBEDDED
    // CHILD's own snapshot position (`EmbeddedBaseChild.propertyOverrides`), not the root.
    // `structuralDiff` treats the whole `/embedded/<coll>` array as one opaque leaf, so the base
    // and the template's current snapshot must agree on the record's content AND its recorded
    // policy for the array to structurally match — pinning the ordinary happy path once the
    // (unreachable) per-record policy exclusion is gone.
    const itemChild = doc({
      id: "tc", source: { id: "tc", pack: null, version: 1 }, engine: { hp: 5 },
      permissions: {
        default: "observer", users: {}, capabilities: { by_role: {}, by_user: {} }, gm_role: null,
        property_overrides: { "/engine/hp": "gm_only" },
      },
    });
    const fullTmpl = doc({ id: "T", name: "T", system: {}, embedded: { items: [itemChild] } });
    const gmChild = doc({ id: "C", source: { id: "T", pack: null, version: 1 } });
    gmChild.base = snapshotBase(fullTmpl);
    expect(syncState(gmChild, fullTmpl)).toBe("up_to_date");

    // The player's view of both the template and the stored base has the record's hidden leaf
    // stripped (the record's own tier is unaffected — it is recorded verbatim, not re-expressed).
    const playerTmpl = doc({
      id: "T", name: "T", system: {},
      embedded: { items: [{ ...itemChild, engine: null }] },
    });
    const playerBase = snapshotBase(playerTmpl);
    playerBase.embedded.items[0].propertyOverrides = { "/engine/hp": "gm_only" };
    const playerChild = doc({ id: "C2", source: { id: "T", pack: null, version: 1 } });
    playerChild.base = playerBase;
    expect(syncState(playerChild, playerTmpl)).toBe("up_to_date");
  });
});

describe("normalizeBase", () => {
  it("parses a snapshot carrying every required key, recursively", () => {
    expect(
      normalizeBase({
        name: null,
        engine: null,
        system: { hp: 1 },
        property_overrides: {},
        embedded: { items: [{ sourceId: "t", name: null, engine: null, system: null, embedded: {}, propertyOverrides: {} }] },
      }),
    ).toEqual<MergeBase>({
      name: null,
      engine: null,
      system: { hp: 1 },
      embedded: { items: [{ sourceId: "t", name: null, engine: null, system: null, embedded: {}, propertyOverrides: {} }] },
      property_overrides: {},
    });
  });

  it("reads an absent content band (name/engine/system) as null — a redacted band is legitimately missing", () => {
    // The server's egress REMOVES a hidden `/base/<band>` key wholesale (`redaction_target`
    // classifies it `Within`), so a recipient's redacted copy misses any of these three keys with
    // nothing wrong; reading the gap as `null` is what matches the live template's own hidden band
    // (nulled in place there, per `redaction_target::Band`).
    expect(normalizeBase({ system: { hp: 1 }, embedded: {}, property_overrides: {} })).toEqual<MergeBase>({
      name: null,
      engine: null,
      system: { hp: 1 },
      embedded: {},
      property_overrides: {},
    });
  });

  it("rejects a root missing a structural key (never subject to redaction)", () => {
    // `embedded` and `property_overrides` are never named by a recorded policy entry
    // (`writes_a_content_band` admits only `/name`/`/engine…`/`/system…`), so their absence is
    // not a legitimate redacted shape — it can only be corrupted/foreign data.
    expect(normalizeBase({ name: "T", engine: null, system: null, property_overrides: {} })).toBeNull();
    expect(normalizeBase({ name: "T", engine: null, system: null, embedded: {} })).toBeNull();
  });

  it("rejects an embedded record missing a structural key", () => {
    expect(normalizeBase({ embedded: { items: [{ sourceId: "t", embedded: {} }] }, property_overrides: {} })).toBeNull();
    expect(normalizeBase({ embedded: { items: [{ embedded: {}, propertyOverrides: {} }] }, property_overrides: {} })).toBeNull();
  });

  it("rejects a non-object", () => {
    expect(normalizeBase(undefined)).toBeNull();
    expect(normalizeBase(42)).toBeNull();
  });
});
