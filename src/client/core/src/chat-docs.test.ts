import { describe, expect, expectTypeOf, test } from "vitest";
import { z } from "zod";
import {
  parseMessageEngine,
  buildChannelRegistryDoc,
  buildDiceSettingsDoc,
  buildChatSettingsDoc,
  isKnownSegment,
  MESSAGE_DOC_TYPE,
  dieRecordSchemaImpl,
  constTermSchemaImpl,
  chatSegmentSchemaImpl,
  rollOutcomeSchemaImpl,
  chatMessageEngineSchemaImpl,
  baseRollDice,
  numericBounds,
  type DieRecord,
  type ConstTerm,
  type ChatSegment,
  type RollOutcome,
  type ChatMessageEngine,
} from "./chat-docs";
import type { WireDocument } from "./wire";

// Non-vacuous schema/type guard: asserting against the unannotated `xImpl` const, not
// `z.infer<typeof XSchema>`, is what makes a dropped union arm or a narrowed field fail this
// assertion. `RollOutcomeSchema` and `ChatMessageEngineSchema` are covered by their own describe
// block below, since both widen their INPUT type via `.default(...)` and so need the matching
// 3-arg `z.ZodType` form.
describe("chat-docs drift guard — non-vacuous schema/type assertions", () => {
  test("DieRecord", () => {
    expectTypeOf<z.infer<typeof dieRecordSchemaImpl>>().toEqualTypeOf<DieRecord>();
  });
  test("ConstTerm", () => {
    expectTypeOf<z.infer<typeof constTermSchemaImpl>>().toEqualTypeOf<ConstTerm>();
  });
  test("ChatSegment", () => {
    expectTypeOf<z.infer<typeof chatSegmentSchemaImpl>>().toEqualTypeOf<ChatSegment>();
  });
  test("RollOutcome", () => {
    expectTypeOf<z.infer<typeof rollOutcomeSchemaImpl>>().toEqualTypeOf<RollOutcome>();
  });
  test("ChatMessageEngine", () => {
    expectTypeOf<z.infer<typeof chatMessageEngineSchemaImpl>>().toEqualTypeOf<ChatMessageEngine>();
  });
});

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
  test("parses roll_id, GM-visible spec/raw, and recalc_history when present", () => {
    const eng = parseMessageEngine(msgDoc({
      ...base, kind: "roll",
      content: [{
        kind: "roll_embed", formula: "1d6", outcome: rollOutcome(),
        roll_id: "11111111-1111-1111-1111-111111111111",
        raw: {
          dice: [{ id: 0, kind: { Numeric: { min: 1, max: 6 } }, natural: 4 }],
          group_spans: [[0, 1]],
        },
        recalc_history: [{
          previous_outcome: rollOutcome({ total: 2 }),
          recalculated_by: "u-gm",
          recalculated_at: 100,
        }],
      }],
    }));
    expect(eng).not.toBeNull();
    const seg = eng!.content[0] as Extract<NonNullable<typeof eng>["content"][number], { kind: "roll_embed" }>;
    expect(seg.roll_id).toBe("11111111-1111-1111-1111-111111111111");
    expect(seg.raw?.dice[0].natural).toBe(4);
    expect(seg.recalc_history?.[0].recalculated_by).toBe("u-gm");
  });
  test("roll_embed without roll_id/spec/raw/recalc_history still parses (legacy/non-GM shape)", () => {
    const eng = parseMessageEngine(msgDoc({
      ...base, kind: "roll",
      content: [{ kind: "roll_embed", formula: "1d6", outcome: rollOutcome() }],
    }));
    expect(eng).not.toBeNull();
    expect(eng!.content).toEqual([{ kind: "roll_embed", formula: "1d6", outcome: rollOutcome() }]);
  });
});

describe("baseRollDice / numericBounds", () => {
  test("baseRollDice slices raw.dice by each group_spans range, in order", () => {
    const raw = {
      dice: [
        { id: 0, kind: { Numeric: { min: 1, max: 6 } }, natural: 3 },
        { id: 1, kind: { Numeric: { min: 1, max: 6 } }, natural: 5 },
        { id: 2, kind: { Numeric: { min: 1, max: 6 } }, natural: 6 }, // an explosion child past both spans
      ],
      group_spans: [[0, 2]] as [number, number][],
    };
    expect(baseRollDice(raw).map((d) => d.id)).toEqual([0, 1]);
  });
  test("numericBounds returns the range for a Numeric die, null for Faces", () => {
    expect(numericBounds({ Numeric: { min: 1, max: 20 } })).toEqual({ min: 1, max: 20 });
    expect(numericBounds({ Faces: { faces: [{ value: 1, symbols: [] }] } })).toBeNull();
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
  test("parses a link_preview segment with image_asset_id", () => {
    const eng = parseMessageEngine(msgDoc({
      ...base,
      content: [
        {
          kind: "link_preview",
          url: "https://example.com/a",
          title: "Example",
          description: "A page.",
          image_asset_id: "00000000-0000-0000-0000-000000000001",
        },
      ],
    }));
    expect(eng).not.toBeNull();
    expect(eng!.content).toEqual([
      {
        kind: "link_preview",
        url: "https://example.com/a",
        title: "Example",
        description: "A page.",
        image_asset_id: "00000000-0000-0000-0000-000000000001",
      },
    ]);
  });
  test("parses a link_preview segment without image_asset_id (absent field)", () => {
    const eng = parseMessageEngine(msgDoc({
      ...base,
      content: [{ kind: "link_preview", url: "https://example.com/a", title: "Example", description: "A page." }],
    }));
    expect(eng).not.toBeNull();
    expect(eng!.content).toEqual([
      { kind: "link_preview", url: "https://example.com/a", title: "Example", description: "A page." },
    ]);
  });
});

describe("oembed segments", () => {
  test("parses an oembed segment", () => {
    const eng = parseMessageEngine(msgDoc({
      ...base,
      content: [
        {
          kind: "oembed",
          url: "https://www.youtube.com/watch?v=abc",
          provider_name: "YouTube",
          title: "A Video",
          author_name: "Someone",
          thumbnail_asset_id: "00000000-0000-0000-0000-000000000002",
        },
      ],
    }));
    expect(eng).not.toBeNull();
    expect(eng!.content).toEqual([
      {
        kind: "oembed",
        url: "https://www.youtube.com/watch?v=abc",
        provider_name: "YouTube",
        title: "A Video",
        author_name: "Someone",
        thumbnail_asset_id: "00000000-0000-0000-0000-000000000002",
      },
    ]);
  });
  test("fail-closed: oembed missing provider_name fails the whole message parse", () => {
    expect(parseMessageEngine(msgDoc({
      ...base,
      content: [{ kind: "oembed", url: "https://www.youtube.com/watch?v=abc" }],
    }))).toBeNull();
  });
  test("a stray html key alongside legitimate oembed fields is stripped from the parsed value", () => {
    const eng = parseMessageEngine(msgDoc({
      ...base,
      content: [
        {
          kind: "oembed",
          url: "https://www.youtube.com/watch?v=abc",
          provider_name: "YouTube",
          html: "<script>alert(1)</script>",
        },
      ],
    }));
    expect(eng).not.toBeNull();
    const seg = eng!.content[0] as Record<string, unknown>;
    expect(seg.kind).toBe("oembed");
    expect(seg).not.toHaveProperty("html");
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
  const d = buildDiceSettingsDoc("w1", { mode: "success_count", direction: "low_wins", channel_overrides: {} });
  expect(d.doc_type).toBe("dice-settings");
  expect(d.parent_id).toBeNull();
  expect(d.scope).toEqual({ kind: "world", world_id: "w1" });
  expect(d.engine).toEqual({ mode: "success_count", direction: "low_wins", channel_overrides: {} });
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
