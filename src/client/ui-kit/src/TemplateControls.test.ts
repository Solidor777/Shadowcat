import { describe, it, expect } from "vitest";
import { render, fireEvent, waitFor } from "@testing-library/svelte";
import TemplateControls from "./TemplateControls.svelte";
import { setAppContextForTest } from "./__fixtures__/appContextTest";
import { DocumentStore, type WireDocument, type WireOperation } from "@shadowcat/core";

function doc(over: Partial<WireDocument> & { id: string }): WireDocument {
  return {
    id: over.id, scope: { kind: "world", world_id: "w1" }, doc_type: "actor", schema_version: 1,
    name: over.name ?? null, source: over.source ?? null, owner: over.owner ?? null,
    permissions: { default: "owner", users: {}, property_overrides: {}, capabilities: { by_role: {}, by_user: {} }, gm_role: null },
    embedded: {}, parent_id: null, engine: over.engine, system: over.system ?? {}, created_at: 0, updated_at: 0,
  };
}

function storeWith(docs: WireDocument[]): DocumentStore {
  const s = new DocumentStore();
  s.applyCommand({ seq: 1, world_id: "w1", author: "a", ts: 0, ops: docs.map((d) => ({ op: "create", doc: d } as WireOperation)) });
  return s;
}

describe("TemplateControls", () => {
  it("renders nothing for a document with no source and no instances", () => {
    const store = storeWith([doc({ id: "C" })]);
    const context = setAppContextForTest({ store, documents: store });
    const { queryByRole } = render(TemplateControls, { props: { docId: "C" }, context });
    expect(queryByRole("button")).toBeNull();
  });

  it("shows the source badge + pull/revert for a stamped, pull-authorized doc", () => {
    const tmpl = doc({ id: "T", name: "Preset" });
    const child = doc({ id: "C", source: { id: "T", pack: null, version: 1 } });
    const store = storeWith([tmpl, child]);
    const pulled: string[] = [];
    const context = setAppContextForTest({
      store, documents: store,
      templates: {
        stampInstance: (s) => s, pull: (id) => pulled.push(id), push: () => {}, revert: () => {},
        findInstances: () => [], syncState: () => "template_changed", canPull: () => true, canPush: () => false,
      },
    });
    const { getByText } = render(TemplateControls, { props: { docId: "C" }, context });
    expect(getByText("templates.badge.source")).toBeTruthy();
    fireEvent.click(getByText("templates.action.pull"));
    expect(pulled).toEqual(["C"]);
  });

  it("shows push when the doc has instances and push is authorized", () => {
    const tmpl = doc({ id: "T" });
    const store = storeWith([tmpl, doc({ id: "A", source: { id: "T", pack: null, version: 1 } })]);
    const pushed: string[] = [];
    const context = setAppContextForTest({
      store, documents: store,
      templates: {
        stampInstance: (s) => s, pull: () => {}, push: (id) => pushed.push(id), revert: () => {},
        findInstances: () => [doc({ id: "A", source: { id: "T", pack: null, version: 1 } })],
        syncState: () => "none", canPull: () => false, canPush: () => true,
      },
    });
    const { getByText } = render(TemplateControls, { props: { docId: "T" }, context });
    fireEvent.click(getByText("templates.action.push"));
    expect(pushed).toEqual(["T"]);
  });

  // The badge/state must re-derive from a POST-MOUNT store mutation, not just at initial
  // render — a bare (unsubscribed) $derived reading ctx.documents freezes at first read.
  // syncState below is wired to read the LIVE store (not a static closure value), so this
  // only passes if TemplateControls actually calls subscribe() before every read that
  // (transitively) depends on the store.
  it("re-renders the badge after the store mutates post-mount", async () => {
    const tmpl = doc({ id: "T", name: "Preset", system: { changed: false } });
    const child = doc({ id: "C", source: { id: "T", pack: null, version: 1 }, system: { changed: false } });
    const store = storeWith([tmpl, child]);
    const context = setAppContextForTest({
      store, documents: store,
      templates: {
        stampInstance: (s) => s, pull: () => {}, push: () => {}, revert: () => {},
        findInstances: (id) => store.snapshot().filter((d) => d.source?.id === id),
        syncState: (id) => {
          const d = store.get(id);
          const tmpl = d?.source ? store.get(d.source.id) : undefined;
          return (tmpl?.system as { changed?: boolean } | undefined)?.changed ? "template_changed" : "up_to_date";
        },
        canPull: () => true,
        canPush: (id) => store.snapshot().some((d) => d.source?.id === id),
      },
    });

    const { getByText, queryByText } = render(TemplateControls, { props: { docId: "C" }, context });

    expect(getByText("templates.badge.upToDate")).toBeTruthy();

    // Mutate the template's `system.changed` post-mount -> syncState flips to template_changed.
    store.applyCommand({
      seq: 2, world_id: "w1", author: "a", ts: 0,
      ops: [{ op: "update", doc_id: "T", changes: [{ path: "/system/changed", old: false, new: true }] }],
    });

    await waitFor(() => expect(getByText("templates.badge.changed")).toBeTruthy());
    expect(queryByText("templates.badge.upToDate")).toBeNull();
  });
});
