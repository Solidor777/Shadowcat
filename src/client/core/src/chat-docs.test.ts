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
    const sys = parseMessageSystem(msgDoc({ ...base, content: [{ kind: "text", text: "a" }, { kind: "roll_embed", roll: {} }] }));
    expect(sys).not.toBeNull();
    expect(sys!.content).toHaveLength(2);
    expect(sys!.content.filter(isKnownSegment)).toEqual([{ kind: "text", text: "a" }]);
  });
  test("fail-closed: wrong doc_type, malformed body, missing fields → null", () => {
    expect(parseMessageSystem(msgDoc(base, "actor"))).toBeNull();
    expect(parseMessageSystem(msgDoc("nonsense"))).toBeNull();
    expect(parseMessageSystem(msgDoc({ channel: "g" }))).toBeNull();
    expect(parseMessageSystem(msgDoc({ ...base, content: "not-an-array" }))).toBeNull();
  });
});

test("buildChannelRegistryDoc builds a world-scoped parentless singleton map doc", () => {
  const d = buildChannelRegistryDoc("w1", { general: { name: "General" } });
  expect(d.doc_type).toBe("channel-registry");
  expect(d.parent_id).toBeNull();
  expect(d.scope).toEqual({ kind: "world", world_id: "w1" });
  expect((d.system as { channels: Record<string, { name: string }> }).channels.general.name).toBe("General");
});
