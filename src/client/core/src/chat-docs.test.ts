import { describe, expect, test } from "vitest";
import { parseMessageSystem, buildChannelRegistryDoc, isKnownSegment, MESSAGE_DOC_TYPE } from "./chat-docs";
import type { WireDocument } from "./wire";

function msgDoc(system: unknown, docType = MESSAGE_DOC_TYPE): WireDocument {
  return {
    id: "m1", scope: { kind: "world", world_id: "w1" }, doc_type: docType,
    schema_version: 1, source: null, owner: "u1",
    permissions: { default: "observer", users: {} } as WireDocument["permissions"],
    embedded: {}, parent_id: null, system, created_at: 1, updated_at: 1,
  };
}
const base = {
  channel: "general", user_owner: "u1", kind: "normal",
  audience: { kind: "public" }, content: [{ kind: "text", text: "hi" }],
};

describe("parseMessageSystem", () => {
  test("parses a plain public text message", () => {
    const sys = parseMessageSystem(msgDoc(base));
    expect(sys).not.toBeNull();
    expect(sys!.channel).toBe("general");
    expect(sys!.content).toEqual([{ kind: "text", text: "hi" }]);
  });
  test("parses whisper audience, html segment, markers, source", () => {
    const sys = parseMessageSystem(msgDoc({
      ...base, audience: { kind: "whisper", recipients: ["u2"] }, kind: "emote",
      content: [{ kind: "html", sanitized_html: "<em>waves</em>" }],
      source: "/me waves", edited_at: 5, deleted_at: null,
    }));
    expect(sys!.audience).toEqual({ kind: "whisper", recipients: ["u2"] });
    expect(sys!.kind).toBe("emote");
    expect(sys!.source).toBe("/me waves");
    expect(sys!.edited_at).toBe(5);
  });
  test("unknown segment kinds survive parse and are filtered by isKnownSegment", () => {
    const sys = parseMessageSystem(msgDoc({ ...base, content: [{ kind: "text", text: "a" }, { kind: "preview_card", url: "x" }] }));
    expect(sys).not.toBeNull();
    expect(sys!.content).toHaveLength(2);
    expect(sys!.content.filter(isKnownSegment)).toEqual([{ kind: "text", text: "a" }]);
  });
  test("fail-closed: a malformed KNOWN-kind segment fails the whole message parse", () => {
    // The unknown-kind fallback must not rescue a text/html segment with a
    // missing or wrong-typed payload — isKnownSegment would misclassify it.
    expect(parseMessageSystem(msgDoc({ ...base, content: [{ kind: "text" }] }))).toBeNull();
    expect(parseMessageSystem(msgDoc({ ...base, content: [{ kind: "text", text: 42 }] }))).toBeNull();
    expect(parseMessageSystem(msgDoc({ ...base, content: [{ kind: "html", sanitized_html: 123 }] }))).toBeNull();
  });
  test("fail-closed: wrong doc_type, malformed body, missing fields → null", () => {
    expect(parseMessageSystem(msgDoc(base, "actor"))).toBeNull();
    expect(parseMessageSystem(msgDoc("nonsense"))).toBeNull();
    expect(parseMessageSystem(msgDoc({ channel: "g" }))).toBeNull();
    expect(parseMessageSystem(msgDoc({ ...base, content: "not-an-array" }))).toBeNull();
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
    symbol_counts: {},
    ...overrides,
  };
}

describe("roll segments (M11d-2)", () => {
  test("parses a roll_embed segment", () => {
    const sys = parseMessageSystem(msgDoc({
      ...base, kind: "roll",
      content: [{ kind: "roll_embed", formula: "2d6+1", outcome: rollOutcome() }],
    }));
    expect(sys).not.toBeNull();
    expect(sys!.content).toEqual([{ kind: "roll_embed", formula: "2d6+1", outcome: rollOutcome() }]);
  });
  test("tolerates extra server-only fields on a DieRecord (passthrough)", () => {
    const sys = parseMessageSystem(msgDoc({
      ...base, kind: "roll",
      content: [{
        kind: "roll_embed", formula: "1d20",
        outcome: rollOutcome({
          records: [dieRecord({ id: 0, rerolled_from: null, ordered: true })],
        }),
      }],
    }));
    expect(sys).not.toBeNull();
  });
  test("parses a roll_button segment with and without label", () => {
    const withLabel = parseMessageSystem(msgDoc({
      ...base, content: [{ kind: "roll_button", formula: "1d20", label: "Attack" }],
    }));
    expect(withLabel!.content).toEqual([{ kind: "roll_button", formula: "1d20", label: "Attack" }]);
    const withoutLabel = parseMessageSystem(msgDoc({
      ...base, content: [{ kind: "roll_button", formula: "1d20", label: null }],
    }));
    expect(withoutLabel!.content).toEqual([{ kind: "roll_button", formula: "1d20", label: null }]);
  });
  test("fail-closed: roll_embed missing outcome fails the whole message parse", () => {
    expect(parseMessageSystem(msgDoc({
      ...base, content: [{ kind: "roll_embed", formula: "1d20" }],
    }))).toBeNull();
  });
  test("fail-closed: roll_embed with outcome.total wrong type fails the whole message parse", () => {
    expect(parseMessageSystem(msgDoc({
      ...base, content: [{ kind: "roll_embed", formula: "1d20", outcome: rollOutcome({ total: "four" }) }],
    }))).toBeNull();
  });
  test("fail-closed: roll_embed with outcome.records not an array fails the whole message parse", () => {
    expect(parseMessageSystem(msgDoc({
      ...base, content: [{ kind: "roll_embed", formula: "1d20", outcome: rollOutcome({ records: "nope" }) }],
    }))).toBeNull();
  });
  test("unknown segment kinds are still opaque alongside known roll kinds", () => {
    const sys = parseMessageSystem(msgDoc({
      ...base,
      content: [
        { kind: "roll_button", formula: "1d20", label: null },
        { kind: "preview_card", url: "https://example.com" },
      ],
    }));
    expect(sys).not.toBeNull();
    expect(sys!.content).toHaveLength(2);
    expect(sys!.content.filter(isKnownSegment)).toEqual([{ kind: "roll_button", formula: "1d20", label: null }]);
  });
});

test("buildChannelRegistryDoc builds a world-scoped parentless singleton map doc", () => {
  const d = buildChannelRegistryDoc("w1", { general: { name: "General" } });
  expect(d.doc_type).toBe("channel-registry");
  expect(d.parent_id).toBeNull();
  expect(d.scope).toEqual({ kind: "world", world_id: "w1" });
  expect((d.system as { channels: Record<string, { name: string }> }).channels.general.name).toBe("General");
});
