import { describe, it, expect, afterEach, vi } from "vitest";
import { render, cleanup } from "@testing-library/svelte";
import { setAppContextForTest } from "./__fixtures__/appContextTest";
import {
  DocumentStore,
  type WireDocument,
  type WireOperation,
  type RollOutcome,
  type TableDrawSegment,
} from "@shadowcat/core";
import SegmentList from "./SegmentList.svelte";

afterEach(() => cleanup());

const cmd = (ops: WireOperation[]) => ({ seq: 1, world_id: "w1", author: "a", ts: 0, ops });
function storeWith(...docs: WireDocument[]): DocumentStore {
  const s = new DocumentStore();
  s.applyCommand(cmd(docs.map((doc) => ({ op: "create" as const, doc }))));
  return s;
}

/** A localized `t` that interpolates params, needed for `chat.oembedOpenOn`. */
function fakeT(key: string, params?: Record<string, string | number>): string {
  const templates: Record<string, string> = {
    "chat.oembedOpenOn": "Open on {provider}",
  };
  let s = templates[key] ?? key;
  if (params) for (const [k, v] of Object.entries(params)) s = s.replaceAll(`{${k}}`, String(v));
  return s;
}

describe("SegmentList — the {@html} boundary", () => {
  it("renders a text segment as a literal DOM text node, never executed as HTML", () => {
    const { container } = render(SegmentList, {
      props: { segments: [{ kind: "text", text: "<b>x</b>" }], channel: "general" },
      context: setAppContextForTest({}),
    });
    expect(container.querySelector(".seg-text b")).toBeNull();
    expect(container.querySelector(".seg-text")?.textContent).toBe("<b>x</b>");
  });

  it("renders an html segment's sanitized_html as real innerHTML markup", () => {
    const { container } = render(SegmentList, {
      props: { segments: [{ kind: "html", sanitized_html: "<strong>bold</strong>" }], channel: "general" },
      context: setAppContextForTest({}),
    });
    const strong = container.querySelector(".seg-html strong");
    expect(strong).not.toBeNull();
    expect(strong?.textContent).toBe("bold");
  });

  it("filters out an unknown segment kind without crashing, rendering only known segments", () => {
    const { container } = render(SegmentList, {
      props: {
        segments: [{ kind: "text", text: "a" }, { kind: "preview_card", url: "x" }],
        channel: "general",
      },
      context: setAppContextForTest({}),
    });
    expect(container.querySelectorAll(".seg-text, .seg-html").length).toBe(1);
    expect(container.textContent).toContain("a");
  });
});

describe("SegmentList — link preview", () => {
  it("renders title/description/host as text, with no img", () => {
    const { container } = render(SegmentList, {
      props: {
        segments: [{ kind: "link_preview", url: "https://example.com/article", title: "An Article", description: "A short summary." }],
        channel: "general",
      },
      context: setAppContextForTest({}),
    });
    expect(container.querySelector(".link-preview-title")?.textContent).toBe("An Article");
    expect(container.querySelector(".link-preview-description")?.textContent).toBe("A short summary.");
    expect(container.querySelector(".link-preview-host")?.textContent).toBe("example.com");
    expect(container.querySelector("img")).toBeNull();
  });

  it("the anchor has the exact href, rel, and target", () => {
    const { container } = render(SegmentList, {
      props: {
        segments: [{ kind: "link_preview", url: "https://example.com/x", title: "T", description: "D" }],
        channel: "general",
      },
      context: setAppContextForTest({}),
    });
    const a = container.querySelector("a.link-preview");
    expect(a?.getAttribute("href")).toBe("https://example.com/x");
    expect(a?.getAttribute("rel")).toBe("noopener noreferrer nofollow");
    expect(a?.getAttribute("target")).toBe("_blank");
  });

  it("a non-http(s) scheme url renders no clickable href (defensive scheme guard)", () => {
    const { container } = render(SegmentList, {
      props: {
        segments: [{ kind: "link_preview", url: "javascript:alert(1)", title: "T", description: "D" }],
        channel: "general",
      },
      context: setAppContextForTest({}),
    });
    expect(container.querySelector("a.link-preview")?.hasAttribute("href")).toBe(false);
    expect(container.querySelector(".link-preview-title")?.textContent).toBe("T");
  });

  it("a malformed url falls back to the raw string as the host caption without throwing", () => {
    const { container } = render(SegmentList, {
      props: {
        segments: [{ kind: "link_preview", url: "not a url", title: "T", description: "D" }],
        channel: "general",
      },
      context: setAppContextForTest({}),
    });
    expect(container.querySelector(".link-preview-host")?.textContent).toBe("not a url");
    // An unparseable URL yields no clickable href (safeHref returns undefined) —
    // the card still renders, just non-clickable.
    expect(container.querySelector("a.link-preview")?.hasAttribute("href")).toBe(false);
  });

  it("renders an <img> whose src starts with /api/assets/ when image_asset_id is present", () => {
    const { container } = render(SegmentList, {
      props: {
        segments: [{
          kind: "link_preview",
          url: "https://example.com/article",
          title: "An Article",
          description: "A short summary.",
          image_asset_id: "00000000-0000-0000-0000-000000000001",
        }],
        channel: "general",
      },
      context: setAppContextForTest({}),
    });
    const img = container.querySelector("img.link-preview-thumb");
    expect(img).not.toBeNull();
    expect(img?.getAttribute("src")).toMatch(/^\/api\/assets\//);
  });
});

describe("SegmentList — oembed", () => {
  it("renders the provider name, title, and the open-on link text", () => {
    const { container } = render(SegmentList, {
      props: {
        segments: [{
          kind: "oembed",
          url: "https://www.youtube.com/watch?v=abc",
          provider_name: "YouTube",
          title: "A Video",
          author_name: "Someone",
        }],
        channel: "general",
      },
      context: setAppContextForTest({ t: fakeT }),
    });
    expect(container.querySelector(".oembed-provider")?.textContent).toBe("YouTube");
    expect(container.querySelector(".oembed-title")?.textContent).toBe("A Video");
    expect(container.querySelector(".oembed-author")?.textContent).toBe("Someone");
    expect(container.querySelector(".oembed-open")?.textContent).toBe("Open on YouTube");
  });

  it("renders an <img> whose src starts with /api/assets/ when thumbnail_asset_id is present", () => {
    const { container } = render(SegmentList, {
      props: {
        segments: [{
          kind: "oembed",
          url: "https://www.youtube.com/watch?v=abc",
          provider_name: "YouTube",
          thumbnail_asset_id: "00000000-0000-0000-0000-000000000002",
        }],
        channel: "general",
      },
      context: setAppContextForTest({}),
    });
    const img = container.querySelector("img.oembed-thumb");
    expect(img).not.toBeNull();
    expect(img?.getAttribute("src")).toMatch(/^\/api\/assets\//);
  });
});

describe("SegmentList — doc_link", () => {
  it("renders a clickable button when the target is present in the store", () => {
    const target: WireDocument = {
      id: "d1",
      scope: { kind: "world", world_id: "w1" },
      doc_type: "actor",
      schema_version: 1,
      name: "Goblin",
      source: null,
      owner: "u1",
      permissions: { default: "observer", users: {}, property_overrides: {}, capabilities: { by_role: {}, by_user: {} }, gm_role: null },
      embedded: {},
      parent_id: null,
      engine: {},
      system: {},
      created_at: 0,
      updated_at: 0,
    };
    const { container } = render(SegmentList, {
      props: {
        segments: [{ kind: "doc_link", target: { kind: "doc", doc_id: "d1" }, label: "My Doc" }],
        channel: "general",
      },
      context: setAppContextForTest({ documents: storeWith(target) }),
    });
    const btn = container.querySelector("button.doc-link");
    expect(btn).not.toBeNull();
    expect(btn?.textContent).toBe("My Doc");
  });

  it("renders inert plain text when the target is absent from the store", () => {
    const { container } = render(SegmentList, {
      props: {
        segments: [{ kind: "doc_link", target: { kind: "doc", doc_id: "dangling" }, label: "Gone" }],
        channel: "general",
      },
      context: setAppContextForTest({}),
    });
    expect(container.querySelector("button.doc-link")).toBeNull();
    expect(container.querySelector(".seg-text")?.textContent).toBe("Gone");
  });
});

describe("SegmentList — roll_button", () => {
  it("renders a button with the label, falling back to the formula", () => {
    const { container } = render(SegmentList, {
      props: {
        segments: [{ kind: "roll_button", formula: "1d20", label: "Attack" }],
        channel: "general",
      },
      context: setAppContextForTest({}),
    });
    expect(container.querySelector("button.roll-btn")?.textContent?.trim()).toBe("Attack");
  });

  it("falls back to the formula when no label is present", () => {
    const { container } = render(SegmentList, {
      props: {
        segments: [{ kind: "roll_button", formula: "1d20", label: null }],
        channel: "general",
      },
      context: setAppContextForTest({}),
    });
    expect(container.querySelector("button.roll-btn")?.textContent?.trim()).toBe("1d20");
  });
});

describe("SegmentList — image", () => {
  it("renders an <img> with a preview-variant src and the escaped alt text", () => {
    const { container } = render(SegmentList, {
      props: {
        segments: [{ kind: "image", asset_id: "00000000-0000-0000-0000-000000000003", alt: "<b>a map</b>" }],
        channel: "general",
      },
      context: setAppContextForTest({}),
    });
    const img = container.querySelector("img.image-segment");
    expect(img).not.toBeNull();
    expect(img?.getAttribute("src")).toMatch(/^\/api\/assets\/.*variant=preview/);
    // Alt is set via attribute binding, never {@html} — a markup-looking string stays literal.
    expect(img?.getAttribute("alt")).toBe("<b>a map</b>");
    expect(img?.querySelector("b")).toBeNull();
  });

  it("the enclosing anchor links to the canonical (non-variant) asset URL", () => {
    const { container } = render(SegmentList, {
      props: {
        segments: [{ kind: "image", asset_id: "00000000-0000-0000-0000-000000000004", alt: "" }],
        channel: "general",
      },
      context: setAppContextForTest({}),
    });
    const a = container.querySelector("a.image-segment-link");
    expect(a?.getAttribute("href")).toMatch(/^\/api\/assets\//);
    expect(a?.getAttribute("href")).not.toMatch(/variant=/);
    expect(a?.getAttribute("target")).toBe("_blank");
    expect(a?.getAttribute("rel")).toBe("noopener noreferrer");
  });
});

function rollOutcome(): RollOutcome {
  return {
    total: 4, records: [],
    successes: null, pass: null, margin: null, tier_label: null, tier_value: null,
    crit_successes: 0, crit_fails: 0, positive_counter: 0, negative_counter: 0,
    symbol_counts: {}, labeled_consts: [],
  };
}

function tableDraw(overrides: Partial<TableDrawSegment> = {}): TableDrawSegment {
  return {
    kind: "table_draw",
    table_id: "t1",
    table_name: "Loot",
    roll_id: "r1",
    formula: "1d6",
    outcome: rollOutcome(),
    row: { index: 0, label: "a sword", content: [{ kind: "text", text: "shiny" }], nested: [] },
    ...overrides,
  };
}

const tableDoc: WireDocument = {
  id: "t1",
  scope: { kind: "world", world_id: "w1" },
  doc_type: "table",
  schema_version: 1,
  name: "Loot",
  source: null,
  owner: null,
  permissions: { default: "observer", users: {}, property_overrides: {}, capabilities: { by_role: {}, by_user: {} }, gm_role: null },
  embedded: {},
  parent_id: null,
  engine: { draw: { kind: "weighted" }, rows: [], description: "" },
  system: {},
  created_at: 0,
  updated_at: 0,
};

describe("SegmentList — table_draw", () => {
  it("renders a nested draw indented under its parent", () => {
    const nested = tableDraw({
      table_id: "t2",
      roll_id: "r2",
      row: { index: 0, label: "a gem", content: [], nested: [] },
    });
    const outer = tableDraw({
      row: { index: 0, label: "spawns", content: [], nested: [nested] },
    });
    const { container } = render(SegmentList, {
      props: { segments: [outer], channel: "general" },
      context: setAppContextForTest({}),
    });
    expect(container.querySelector(".table-draw-nested .table-draw")).not.toBeNull();
    expect(container.textContent).toContain("a gem");
    // The nested block carries an accessible label (chat.table.nested) --
    // a screen reader gets nothing from the CSS indent alone. This fixture's
    // `t` is an identity echo (see appContextTest.ts), so the resolved
    // value is the raw key, not the catalog's rendered string.
    expect(container.querySelector(".table-draw-nested")?.getAttribute("aria-label")).toBe(
      "chat.table.nested",
    );
  });

  it("renders row.content through this same component", () => {
    const { container } = render(SegmentList, {
      props: { segments: [tableDraw()], channel: "general" },
      context: setAppContextForTest({}),
    });
    expect(container.querySelector(".seg-text")?.textContent).toBe("shiny");
  });

  it("shows a no-matching-row message when row is absent", () => {
    const { container } = render(SegmentList, {
      props: { segments: [tableDraw({ row: null })], channel: "general" },
      context: setAppContextForTest({}),
    });
    expect(container.querySelector(".table-draw-no-row")).not.toBeNull();
  });

  it("shows a Draw button on a doc_link only when the target resolves to a table in the store", () => {
    const { container } = render(SegmentList, {
      props: {
        segments: [{ kind: "doc_link", target: { kind: "doc", doc_id: "t1" }, label: "Loot Table" }],
        channel: "general",
      },
      context: setAppContextForTest({ documents: storeWith(tableDoc) }),
    });
    expect(container.querySelector('[data-testid="table-draw-button"]')).not.toBeNull();
  });

  it("shows no Draw button on a doc_link when the target is not a table", () => {
    const actorDoc: WireDocument = { ...tableDoc, id: "a1", doc_type: "actor", engine: {} };
    const { container } = render(SegmentList, {
      props: {
        segments: [{ kind: "doc_link", target: { kind: "doc", doc_id: "a1" }, label: "Goblin" }],
        channel: "general",
      },
      context: setAppContextForTest({ documents: storeWith(actorDoc) }),
    });
    expect(container.querySelector('[data-testid="table-draw-button"]')).toBeNull();
  });

  it("clicking the Draw button calls ctx.chat.drawTable with the message's channel", async () => {
    const drawTable = vi.fn().mockResolvedValue(undefined);
    const { container } = render(SegmentList, {
      props: {
        segments: [{ kind: "doc_link", target: { kind: "doc", doc_id: "t1" }, label: "Loot Table" }],
        channel: "ic",
      },
      context: setAppContextForTest({
        documents: storeWith(tableDoc),
        chat: { send: () => Promise.resolve(), edit: () => Promise.resolve(), delete: () => Promise.resolve(), recalc: () => Promise.resolve(), drawTable },
      }),
    });
    const btn = container.querySelector('[data-testid="table-draw-button"]') as HTMLButtonElement;
    btn.click();
    await Promise.resolve();
    expect(drawTable).toHaveBeenCalledWith({ tableId: "t1", channel: "ic" });
  });
});
