import { describe, expect, test } from "vitest";
import { parseMessageEngine, buildChannelRegistryDoc, buildDiceSettingsDoc, buildChatSettingsDoc, isKnownSegment, MESSAGE_DOC_TYPE } from "./chat-docs";
import type { WireDocument } from "./wire";

function msgDoc(engine: unknown, docType = MESSAGE_DOC_TYPE): WireDocument {
  return {
    id: "m1", scope: { kind: "world", world_id: "w1" }, doc_type: docType,
    schema_version: 1, name: null, source: null, owner: "u1",
    permissions: { default: "observer", users: {} } as WireDocument["permissions"],
    embedded: {}, parent_id: null, engine, system: {}, created_at: 1, updated_at: 1,
  };
}
const base = {
  channel: "general", user_owner: "u1", kind: "normal",
  audience: { kind: "public" }, content: [{ kind: "text", text: "hi" }],
};

describe("parseMessageEngine", () => {
  test("parses a plain public text message", () => {
    const eng = parseMessageEngine(msgDoc(base));
    expect(eng).not.toBeNull();
    expect(eng!.channel).toBe("general");
    expect(eng!.content).toEqual([{ kind: "text", text: "hi" }]);
  });
  test("parses whisper audience, html segment, markers, source", () => {
    const eng = parseMessageEngine(msgDoc({
      ...base, audience: { kind: "whisper", recipients: ["u2"] }, kind: "emote",
      content: [{ kind: "html", sanitized_html: "<em>waves</em>" }],
      source: "/me waves", edited_at: 5, deleted_at: null,
    }));
    expect(eng!.audience).toEqual({ kind: "whisper", recipients: ["u2"] });
    expect(eng!.kind).toBe("emote");
    expect(eng!.source).toBe("/me waves");
    expect(eng!.edited_at).toBe(5);
  });
  test("unknown segment kinds survive parse and are filtered by isKnownSegment", () => {
    const eng = parseMessageEngine(msgDoc({ ...base, content: [{ kind: "text", text: "a" }, { kind: "preview_card", url: "x" }] }));
    expect(eng).not.toBeNull();
    expect(eng!.content).toHaveLength(2);
    expect(eng!.content.filter(isKnownSegment)).toEqual([{ kind: "text", text: "a" }]);
  });
  test("fail-closed: a malformed KNOWN-kind segment fails the whole message parse", () => {
    // The unknown-kind fallback must not rescue a text/html segment with a
    // missing or wrong-typed payload — isKnownSegment would misclassify it.
    expect(parseMessageEngine(msgDoc({ ...base, content: [{ kind: "text" }] }))).toBeNull();
    expect(parseMessageEngine(msgDoc({ ...base, content: [{ kind: "text", text: 42 }] }))).toBeNull();
    expect(parseMessageEngine(msgDoc({ ...base, content: [{ kind: "html", sanitized_html: 123 }] }))).toBeNull();
  });
  test("fail-closed: wrong doc_type, malformed body, missing fields → null", () => {
    expect(parseMessageEngine(msgDoc(base, "actor"))).toBeNull();
    expect(parseMessageEngine(msgDoc("nonsense"))).toBeNull();
    expect(parseMessageEngine(msgDoc({ channel: "g" }))).toBeNull();
    expect(parseMessageEngine(msgDoc({ ...base, content: "not-an-array" }))).toBeNull();
  });
});

function dieRecord(overrides: Partial<Record<string, unknown>> = {}) {
  return {
    value: 4, natural: 4, kept: true, exploded: false,
    crit_success: false, crit_fail: false, expertise: 0, group_index: 0,
    label: null, symbols: [],
    ...overrides,
  };
}
function rollOutcome(overrides: Partial<Record<string, unknown>> = {}) {
  return {
    total: 4, records: [dieRecord()],
    successes: null, pass: null, margin: null, tier_label: null, tier_value: null,
    crit_successes: 0, crit_fails: 0, positive_counter: 0, negative_counter: 0,
    symbol_counts: {}, labeled_consts: [],
    ...overrides,
  };
}

describe("roll segments", () => {
  test("parses a roll_embed segment", () => {
    const eng = parseMessageEngine(msgDoc({
      ...base, kind: "roll",
      content: [{ kind: "roll_embed", formula: "2d6+1", outcome: rollOutcome() }],
    }));
    expect(eng).not.toBeNull();
    expect(eng!.content).toEqual([{ kind: "roll_embed", formula: "2d6+1", outcome: rollOutcome() }]);
  });
  test("parses a labeled Const term in labeled_consts, defaulting to [] when absent", () => {
    const withConst = parseMessageEngine(msgDoc({
      ...base, kind: "roll",
      content: [{
        kind: "roll_embed", formula: "1d20 + 3[dex]",
        outcome: rollOutcome({ labeled_consts: [{ value: 3, label: "dex" }] }),
      }],
    }));
    expect(withConst!.content).toEqual([
      { kind: "roll_embed", formula: "1d20 + 3[dex]", outcome: rollOutcome({ labeled_consts: [{ value: 3, label: "dex" }] }) },
    ]);

    // Absent on a roll persisted before this field existed -> defaults to [].
    const legacy = rollOutcome();
    delete (legacy as Record<string, unknown>).labeled_consts;
    const withoutConst = parseMessageEngine(msgDoc({
      ...base, kind: "roll",
      content: [{ kind: "roll_embed", formula: "1d20", outcome: legacy }],
    }));
    expect(withoutConst).not.toBeNull();
    const seg = withoutConst!.content[0] as { outcome: { labeled_consts: unknown[] } };
    expect(seg.outcome.labeled_consts).toEqual([]);
  });

  test("tolerates extra server-only fields on a DieRecord (passthrough)", () => {
    const eng = parseMessageEngine(msgDoc({
      ...base, kind: "roll",
      content: [{
        kind: "roll_embed", formula: "1d20",
        outcome: rollOutcome({
          records: [dieRecord({ id: 0, rerolled_from: null, ordered: true })],
        }),
      }],
    }));
    expect(eng).not.toBeNull();
  });
  test("parses a roll_button segment with and without label", () => {
    const withLabel = parseMessageEngine(msgDoc({
      ...base, content: [{ kind: "roll_button", formula: "1d20", label: "Attack" }],
    }));
    expect(withLabel!.content).toEqual([{ kind: "roll_button", formula: "1d20", label: "Attack" }]);
    const withoutLabel = parseMessageEngine(msgDoc({
      ...base, content: [{ kind: "roll_button", formula: "1d20", label: null }],
    }));
    expect(withoutLabel!.content).toEqual([{ kind: "roll_button", formula: "1d20", label: null }]);
  });
  test("fail-closed: roll_embed missing outcome fails the whole message parse", () => {
    expect(parseMessageEngine(msgDoc({
      ...base, content: [{ kind: "roll_embed", formula: "1d20" }],
    }))).toBeNull();
  });
  test("fail-closed: roll_embed with outcome.total wrong type fails the whole message parse", () => {
    expect(parseMessageEngine(msgDoc({
      ...base, content: [{ kind: "roll_embed", formula: "1d20", outcome: rollOutcome({ total: "four" }) }],
    }))).toBeNull();
  });
  test("fail-closed: roll_embed with outcome.records not an array fails the whole message parse", () => {
    expect(parseMessageEngine(msgDoc({
      ...base, content: [{ kind: "roll_embed", formula: "1d20", outcome: rollOutcome({ records: "nope" }) }],
    }))).toBeNull();
  });
  test("unknown segment kinds are still opaque alongside known roll kinds", () => {
    const eng = parseMessageEngine(msgDoc({
      ...base,
      content: [
        { kind: "roll_button", formula: "1d20", label: null },
        { kind: "preview_card", url: "https://example.com" },
      ],
    }));
    expect(eng).not.toBeNull();
    expect(eng!.content).toHaveLength(2);
    expect(eng!.content.filter(isKnownSegment)).toEqual([{ kind: "roll_button", formula: "1d20", label: null }]);
  });
});

describe("link_preview segments", () => {
  test("parses a link_preview segment", () => {
    const eng = parseMessageEngine(msgDoc({
      ...base,
      content: [
        { kind: "text", text: "check this out" },
        { kind: "link_preview", url: "https://example.com/a", title: "Example", description: "A page." },
      ],
    }));
    expect(eng).not.toBeNull();
    expect(eng!.content).toEqual([
      { kind: "text", text: "check this out" },
      { kind: "link_preview", url: "https://example.com/a", title: "Example", description: "A page." },
    ]);
  });
  test("fail-closed: link_preview missing title fails the whole message parse", () => {
    expect(parseMessageEngine(msgDoc({
      ...base,
      content: [{ kind: "link_preview", url: "https://example.com/a", description: "A page." }],
    }))).toBeNull();
  });
  test("fail-closed: link_preview with url not a string fails the whole message parse", () => {
    expect(parseMessageEngine(msgDoc({
      ...base,
      content: [{ kind: "link_preview", url: 42, title: "Example", description: "A page." }],
    }))).toBeNull();
  });
  test("unknown segment kinds are still opaque alongside known link_preview kinds", () => {
    const eng = parseMessageEngine(msgDoc({
      ...base,
      content: [
        { kind: "link_preview", url: "https://example.com/a", title: "Example", description: "A page." },
        { kind: "preview_card", url: "https://example.com/b" },
      ],
    }));
    expect(eng).not.toBeNull();
    expect(eng!.content).toHaveLength(2);
    expect(eng!.content.filter(isKnownSegment)).toEqual([
      { kind: "link_preview", url: "https://example.com/a", title: "Example", description: "A page." },
    ]);
  });
});

test("buildChannelRegistryDoc builds a world-scoped parentless singleton map doc", () => {
  const d = buildChannelRegistryDoc("w1", { general: { name: "General" } });
  expect(d.doc_type).toBe("channel-registry");
  expect(d.parent_id).toBeNull();
  expect(d.scope).toEqual({ kind: "world", world_id: "w1" });
  expect((d.engine as { channels: Record<string, { name: string }> }).channels.general.name).toBe("General");
  expect(d.system).toEqual({});
});

test("buildDiceSettingsDoc builds a world-scoped parentless singleton doc", () => {
  const d = buildDiceSettingsDoc("w1", { mode: "success_count", direction: "low_wins" });
  expect(d.doc_type).toBe("dice-settings");
  expect(d.parent_id).toBeNull();
  expect(d.scope).toEqual({ kind: "world", world_id: "w1" });
  expect(d.engine).toEqual({ mode: "success_count", direction: "low_wins" });
  expect(d.system).toEqual({});
});

test("buildChatSettingsDoc builds a world-scoped parentless singleton doc", () => {
  const d = buildChatSettingsDoc("w1", {
    markdown: null, html: null, images: null, hyperlinks: true, emails: null, link_previews: false,
  });
  expect(d.doc_type).toBe("chat-settings");
  expect(d.parent_id).toBeNull();
  expect(d.scope).toEqual({ kind: "world", world_id: "w1" });
  expect(d.engine).toEqual({ markdown: null, html: null, images: null, hyperlinks: true, emails: null, link_previews: false });
  expect(d.system).toEqual({});
});
