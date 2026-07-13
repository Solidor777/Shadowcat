import { describe, it, expect, vi, afterEach } from "vitest";
import { render, screen, fireEvent } from "@testing-library/svelte";
import { setAppContextForTest } from "@shadowcat/ui-kit/test";
import {
  DocumentStore,
  buildActorDoc,
  buildTokenFromActor,
  type ActorSystem,
  type WireDocument,
  type WireOperation,
} from "@shadowcat/core";
import MessageCard from "./MessageCard.svelte";

const cmd = (ops: WireOperation[]) => ({ seq: 1, world_id: "w1", author: "a", ts: 0, ops });
function storeWith(...docs: WireDocument[]): DocumentStore {
  const s = new DocumentStore();
  s.applyCommand(cmd(docs.map((doc) => ({ op: "create" as const, doc }))));
  return s;
}

function msgDoc(id: string, system: Record<string, unknown>): WireDocument {
  return {
    id,
    scope: { kind: "world", world_id: "w1" },
    doc_type: "message",
    schema_version: 1,
    source: null,
    owner: "u1",
    permissions: { default: "observer", users: {}, property_overrides: {}, capabilities: { by_role: {}, by_user: {} }, gm_role: null },
    embedded: {},
    parent_id: null,
    system,
    created_at: Date.UTC(2026, 0, 1, 14, 30),
    updated_at: Date.UTC(2026, 0, 1, 14, 30),
  };
}

function baseSystem(over: Record<string, unknown> = {}): Record<string, unknown> {
  return {
    channel: "general",
    user_owner: "u1",
    kind: "normal",
    audience: { kind: "public" },
    content: [{ kind: "text", text: "hello" }],
    ...over,
  };
}

/** A minimal ActorSystem builder — every field ActorSystem needs, overridable per test. */
function actorSystem(over: Partial<ActorSystem> = {}): ActorSystem {
  return {
    name: "Real Name",
    displayName: "The Mysterious Figure",
    visual: { kind: "image", asset: "a1" },
    size: { w: 1, h: 1 },
    shape: "square",
    faction: null,
    conditions: [],
    prototype: false,
    ...over,
  };
}

/** A localized `t` that actually interpolates params, needed to assert on chip text
 * (chat.rollPending / chat.whisperTo / chat.roll.*) whose whole value is parameterized. */
function fakeT(key: string, params?: Record<string, string | number>): string {
  const templates: Record<string, string> = {
    "chat.rollPending": "🎲 {formula}",
    "chat.whisperTo": "to {names}",
    "chat.systemBadge": "System",
    "chat.roll.successes": "{n} successes",
    "chat.roll.pass": "Success",
    "chat.roll.fail": "Failure",
  };
  let s = templates[key] ?? key;
  if (params) for (const [k, v] of Object.entries(params)) s = s.replaceAll(`{${k}}`, String(v));
  return s;
}

/** Mirrors dice::outcome::DieRecord's defaults, overridable per test. */
function dieRecord(over: Record<string, unknown> = {}): Record<string, unknown> {
  return {
    value: 4, natural: 4, kept: true, exploded: false,
    crit_success: false, crit_fail: false, expertise: 0, group_index: 0,
    label: null, symbols: [],
    ...over,
  };
}

/** Mirrors dice::outcome::RollOutcome's defaults, overridable per test. */
function rollOutcome(over: Record<string, unknown> = {}): Record<string, unknown> {
  return {
    total: 4, records: [dieRecord()],
    successes: null, pass: null, margin: null, tier_label: null, tier_value: null,
    crit_successes: 0, crit_fails: 0, positive_counter: 0, negative_counter: 0,
    symbol_counts: {},
    ...over,
  };
}

describe("MessageCard — fail-closed body parse", () => {
  it("renders nothing for a malformed body (missing required field)", () => {
    const doc = msgDoc("m1", { channel: "general", kind: "normal" }); // missing user_owner/audience/content
    const { container } = render(MessageCard, { props: { message: doc, showChannel: false }, context: setAppContextForTest({ documents: storeWith(doc) }) });
    expect(container.querySelector("article")).toBeNull();
  });

  it("renders nothing for a non-message doc_type", () => {
    const doc = { ...msgDoc("m1", baseSystem()), doc_type: "actor" };
    const { container } = render(MessageCard, { props: { message: doc, showChannel: false }, context: setAppContextForTest({ documents: storeWith(doc) }) });
    expect(container.querySelector("article")).toBeNull();
  });

  it("renders nothing for a malformed roll_embed segment (missing outcome)", () => {
    // chat-docs.ts's fail-closed pattern: a known-kind segment with a malformed payload
    // fails BOTH the strict schema AND the unknown-segment rescue (which refuses known
    // kinds), so the whole message parse fails, not just the one segment.
    const doc = msgDoc("m1", baseSystem({ kind: "roll", content: [{ kind: "roll_embed", formula: "1d6" }] }));
    const { container } = render(MessageCard, { props: { message: doc, showChannel: false }, context: setAppContextForTest({ documents: storeWith(doc) }) });
    expect(container.querySelector("article")).toBeNull();
  });

  it("renders nothing for a malformed roll_button segment (missing formula)", () => {
    const doc = msgDoc("m1", baseSystem({ content: [{ kind: "roll_button" }] }));
    const { container } = render(MessageCard, { props: { message: doc, showChannel: false }, context: setAppContextForTest({ documents: storeWith(doc) }) });
    expect(container.querySelector("article")).toBeNull();
  });
});

describe("MessageCard — the {@html} boundary", () => {
  it("renders a text segment as a literal DOM text node, never executed as HTML", () => {
    const doc = msgDoc("m1", baseSystem({ content: [{ kind: "text", text: "<b>x</b>" }] }));
    const { container } = render(MessageCard, { props: { message: doc, showChannel: false }, context: setAppContextForTest({ documents: storeWith(doc) }) });
    expect(container.querySelector(".seg-text b")).toBeNull();
    expect(container.querySelector(".seg-text")?.textContent).toBe("<b>x</b>");
  });

  it("renders an html segment's sanitized_html as real innerHTML markup", () => {
    const doc = msgDoc("m1", baseSystem({ content: [{ kind: "html", sanitized_html: "<strong>bold</strong>" }] }));
    const { container } = render(MessageCard, { props: { message: doc, showChannel: false }, context: setAppContextForTest({ documents: storeWith(doc) }) });
    const strong = container.querySelector(".seg-html strong");
    expect(strong).not.toBeNull();
    expect(strong?.textContent).toBe("bold");
  });

  it("filters out an unknown segment kind without crashing, rendering only known segments", () => {
    // "preview_card" is a genuinely unknown kind (per chat-docs.ts's fail-closed pattern, a
    // malformed roll_embed/roll_button/text/html segment would fail the WHOLE message parse
    // instead of being rescued here).
    const doc = msgDoc("m1", baseSystem({ content: [{ kind: "text", text: "a" }, { kind: "preview_card", url: "x" }] }));
    const { container } = render(MessageCard, { props: { message: doc, showChannel: false }, context: setAppContextForTest({ documents: storeWith(doc) }) });
    expect(container.querySelectorAll(".seg-text, .seg-html").length).toBe(1);
    expect(container.textContent).toContain("a");
  });
});

describe("MessageCard — link preview (M11d-3)", () => {
  it("renders title/description/host as text, with no img", () => {
    const doc = msgDoc("m1", baseSystem({
      content: [
        { kind: "text", text: "check this out" },
        { kind: "link_preview", url: "https://example.com/article", title: "An Article", description: "A short summary." },
      ],
    }));
    const { container } = render(MessageCard, {
      props: { message: doc, showChannel: false },
      context: setAppContextForTest({ documents: storeWith(doc) }),
    });
    expect(container.querySelector(".link-preview-title")?.textContent).toBe("An Article");
    expect(container.querySelector(".link-preview-description")?.textContent).toBe("A short summary.");
    expect(container.querySelector(".link-preview-host")?.textContent).toBe("example.com");
    expect(container.querySelector("img")).toBeNull();
  });

  it("the anchor has the exact href, rel, and target", () => {
    const doc = msgDoc("m1", baseSystem({
      content: [{ kind: "link_preview", url: "https://example.com/x", title: "T", description: "D" }],
    }));
    const { container } = render(MessageCard, {
      props: { message: doc, showChannel: false },
      context: setAppContextForTest({ documents: storeWith(doc) }),
    });
    const a = container.querySelector("a.link-preview");
    expect(a?.getAttribute("href")).toBe("https://example.com/x");
    expect(a?.getAttribute("rel")).toBe("noopener noreferrer nofollow");
    expect(a?.getAttribute("target")).toBe("_blank");
  });

  it("a malformed url falls back to the raw string as the host caption without throwing", () => {
    const doc = msgDoc("m1", baseSystem({
      content: [{ kind: "link_preview", url: "not a url", title: "T", description: "D" }],
    }));
    const { container } = render(MessageCard, {
      props: { message: doc, showChannel: false },
      context: setAppContextForTest({ documents: storeWith(doc) }),
    });
    expect(container.querySelector("article")).not.toBeNull();
    expect(container.querySelector(".link-preview-host")?.textContent).toBe("not a url");
    // An unparseable URL yields no clickable href (safeHref returns undefined) —
    // the card still renders, just non-clickable.
    expect(container.querySelector("a.link-preview")?.hasAttribute("href")).toBe(false);
  });

  it("a non-http(s) scheme url renders no clickable href (defensive scheme guard)", () => {
    // Defense-in-depth: the server only ever stores http/https preview URLs, but the
    // card independently refuses to emit a live href for any other scheme, so a
    // javascript:/data: URL from any future bypass path can never become a live anchor.
    const doc = msgDoc("m1", baseSystem({
      content: [{ kind: "link_preview", url: "javascript:alert(1)", title: "T", description: "D" }],
    }));
    const { container } = render(MessageCard, {
      props: { message: doc, showChannel: false },
      context: setAppContextForTest({ documents: storeWith(doc) }),
    });
    expect(container.querySelector("article")).not.toBeNull();
    expect(container.querySelector("a.link-preview")?.hasAttribute("href")).toBe(false);
    expect(container.querySelector(".link-preview-title")?.textContent).toBe("T");
  });

  it("renders both a text segment and a link_preview segment in the same message", () => {
    const doc = msgDoc("m1", baseSystem({
      content: [
        { kind: "text", text: "look at this" },
        { kind: "link_preview", url: "https://example.com", title: "Example", description: "Desc" },
      ],
    }));
    const { container } = render(MessageCard, {
      props: { message: doc, showChannel: false },
      context: setAppContextForTest({ documents: storeWith(doc) }),
    });
    expect(container.querySelector(".seg-text")?.textContent).toBe("look at this");
    expect(container.querySelector(".link-preview-title")?.textContent).toBe("Example");
  });
});

describe("MessageCard — header", () => {
  it("shows the resolved author name from ctx.members, falling back to a short id", () => {
    const doc = msgDoc("m1", baseSystem({ user_owner: "u1" }));
    const { container } = render(MessageCard, {
      props: { message: doc, showChannel: false },
      context: setAppContextForTest({ documents: storeWith(doc), members: new Map([["u1", "Alice"]]) }),
    });
    expect(container.querySelector(".author")?.textContent).toBe("Alice");
  });

  it("falls back to a short id when the author is not in ctx.members", () => {
    const doc = msgDoc("m1", baseSystem({ user_owner: "unregistered-user-uuid" }));
    const { container } = render(MessageCard, { props: { message: doc, showChannel: false }, context: setAppContextForTest({ documents: storeWith(doc) }) });
    expect(container.querySelector(".author")?.textContent).toBe("unregist");
  });

  it("shows HH:MM with the full locale string as the title", () => {
    const doc = msgDoc("m1", baseSystem());
    const { container } = render(MessageCard, { props: { message: doc, showChannel: false }, context: setAppContextForTest({ documents: storeWith(doc) }) });
    const time = container.querySelector("time");
    const d = new Date(doc.created_at);
    const expectedHHMM = `${String(d.getHours()).padStart(2, "0")}:${String(d.getMinutes()).padStart(2, "0")}`;
    expect(time?.textContent).toBe(expectedHHMM);
    expect(time?.getAttribute("title")).toBe(d.toLocaleString());
  });

  it("shows a channel chip only when showChannel is true", () => {
    const doc = msgDoc("m1", baseSystem({ channel: "ooc" }));
    const shown = render(MessageCard, { props: { message: doc, showChannel: true }, context: setAppContextForTest({ documents: storeWith(doc) }) });
    expect(shown.container.querySelector(".chip.channel")?.textContent).toBe("ooc");
    shown.unmount();

    const hidden = render(MessageCard, { props: { message: doc, showChannel: false }, context: setAppContextForTest({ documents: storeWith(doc) }) });
    expect(hidden.container.querySelector(".chip.channel")).toBeNull();
  });

  it("shows an (edited) marker only when edited_at is set", () => {
    const doc = msgDoc("m1", baseSystem({ edited_at: 123 }));
    const { container } = render(MessageCard, { props: { message: doc, showChannel: false }, context: setAppContextForTest({ documents: storeWith(doc) }) });
    expect(container.querySelector(".chip.edited")).not.toBeNull();
  });

  it("shows a GM badge only when audience is gm_only", () => {
    const doc = msgDoc("m1", baseSystem({ audience: { kind: "gm_only" } }));
    const { container } = render(MessageCard, { props: { message: doc, showChannel: false }, context: setAppContextForTest({ documents: storeWith(doc) }) });
    expect(container.querySelector(".chip.gm")).not.toBeNull();
  });

  it("shows a whisper badge naming the resolved recipients", () => {
    const doc = msgDoc("m1", baseSystem({ audience: { kind: "whisper", recipients: ["u2", "u3"] } }));
    const { container } = render(MessageCard, {
      props: { message: doc, showChannel: false },
      context: setAppContextForTest({ documents: storeWith(doc), members: new Map([["u2", "Bob"]]), t: fakeT }),
    });
    expect(container.querySelector(".chip.whisper")?.textContent).toBe("to Bob, u3");
  });
});

describe("MessageCard — emote/roll rendering", () => {
  it("renders an emote as an italic run-in of author name + segments", () => {
    const doc = msgDoc("m1", baseSystem({ kind: "emote", content: [{ kind: "text", text: "waves" }] }));
    const { container } = render(MessageCard, {
      props: { message: doc, showChannel: false },
      context: setAppContextForTest({ documents: storeWith(doc), members: new Map([["u1", "Alice"]]) }),
    });
    const line = container.querySelector(".card.emote .body p");
    expect(line).not.toBeNull();
    expect(line?.textContent?.trim()).toBe("Alice waves");
  });

  it("renders a roll as a monospace pending shell with the concatenated formula text", () => {
    const doc = msgDoc("m1", baseSystem({ kind: "roll", content: [{ kind: "text", text: "1d20+5" }] }));
    const { container } = render(MessageCard, {
      props: { message: doc, showChannel: false },
      context: setAppContextForTest({ documents: storeWith(doc), t: fakeT }),
    });
    expect(container.querySelector(".roll-pending")?.textContent).toBe("🎲 1d20+5");
  });

  it("derives the roll formula from source (stripping the /roll prefix) when content is wrapped as a single html segment", () => {
    // On a markdown/html-enabled world, sanitize() wraps even a bare formula body into one
    // Segment::Html — textOf() alone would read empty here. `source` carries the FULL raw
    // input including the command token, which must be stripped before display.
    const doc = msgDoc(
      "m1",
      baseSystem({ kind: "roll", source: "/roll 1d20+5", content: [{ kind: "html", sanitized_html: "<p>1d20+5</p>" }] }),
    );
    const { container } = render(MessageCard, {
      props: { message: doc, showChannel: false },
      context: setAppContextForTest({ documents: storeWith(doc), t: fakeT }),
    });
    expect(container.querySelector(".roll-pending")?.textContent).toBe("🎲 1d20+5");
  });

  it("strips a /r prefix from source the same way, leaving a bare shorthand source untouched", () => {
    const doc = msgDoc("m1", baseSystem({ kind: "roll", source: "/r 2d6", content: [{ kind: "text", text: "2d6" }] }));
    const { container } = render(MessageCard, {
      props: { message: doc, showChannel: false },
      context: setAppContextForTest({ documents: storeWith(doc), t: fakeT }),
    });
    expect(container.querySelector(".roll-pending")?.textContent).toBe("🎲 2d6");
  });

  it("falls back to textOf(content) for the roll formula when source is absent", () => {
    const doc = msgDoc("m1", baseSystem({ kind: "roll", content: [{ kind: "text", text: "1d20" }] }));
    const { container } = render(MessageCard, {
      props: { message: doc, showChannel: false },
      context: setAppContextForTest({ documents: storeWith(doc), t: fakeT }),
    });
    expect(container.querySelector(".roll-pending")?.textContent).toBe("🎲 1d20");
  });
});

describe("MessageCard — roll block (kind=roll, content = single roll_embed)", () => {
  it("shows total prominently when successes is null", () => {
    const doc = msgDoc("m1", baseSystem({
      kind: "roll",
      content: [{ kind: "roll_embed", formula: "2d6+1", outcome: rollOutcome({ total: 9 }) }],
    }));
    const { container } = render(MessageCard, {
      props: { message: doc, showChannel: false },
      context: setAppContextForTest({ documents: storeWith(doc), t: fakeT }),
    });
    expect(container.querySelector(".roll-formula")?.textContent).toBe("2d6+1");
    expect(container.querySelector(".roll-total")?.textContent).toBe("9");
    expect(container.querySelector(".roll-successes")).toBeNull();
    expect(container.querySelector(".roll-pending")).toBeNull();
  });

  it("labels the formula line with the localized chat.roll.formula key", () => {
    const doc = msgDoc("m1", baseSystem({
      kind: "roll",
      content: [{ kind: "roll_embed", formula: "2d6+1", outcome: rollOutcome({ total: 9 }) }],
    }));
    const { container } = render(MessageCard, {
      props: { message: doc, showChannel: false },
      context: setAppContextForTest({ documents: storeWith(doc), t: fakeT }),
    });
    expect(container.querySelector(".roll-formula")?.getAttribute("aria-label")).toBe("chat.roll.formula");
  });

  it("falls back to the pending shell (not the block) when the raw content carries an extra unknown segment alongside the roll_embed", () => {
    // Server invariant: a roll message's content is exactly one RollEmbed. The guard must
    // check the RAW content length, not the known-segment-filtered length — filtering first
    // would silently drop the unknown segment and still render the block.
    const doc = msgDoc("m1", baseSystem({
      kind: "roll",
      source: "/roll 1d6",
      content: [
        { kind: "roll_embed", formula: "1d6", outcome: rollOutcome() },
        { kind: "preview_card", url: "x" },
      ],
    }));
    const { container } = render(MessageCard, {
      props: { message: doc, showChannel: false },
      context: setAppContextForTest({ documents: storeWith(doc), t: fakeT }),
    });
    expect(container.querySelector(".roll-block")).toBeNull();
    expect(container.querySelector(".roll-pending")).not.toBeNull();
  });

  it("shows successes + pass/fail over total when successes is present", () => {
    const doc = msgDoc("m1", baseSystem({
      kind: "roll",
      content: [{ kind: "roll_embed", formula: "3d10", outcome: rollOutcome({ total: 12, successes: 2, pass: true }) }],
    }));
    const { container } = render(MessageCard, {
      props: { message: doc, showChannel: false },
      context: setAppContextForTest({ documents: storeWith(doc), t: fakeT }),
    });
    expect(container.querySelector(".roll-successes")?.textContent).toBe("2 successes");
    expect(container.querySelector(".roll-pass")?.textContent?.trim()).toBe("Success");
    expect(container.querySelector(".roll-pass.pass")).not.toBeNull();
    expect(container.querySelector(".roll-total")).toBeNull();
  });

  it("shows Failure styling when pass is false", () => {
    const doc = msgDoc("m1", baseSystem({
      kind: "roll",
      content: [{ kind: "roll_embed", formula: "3d10", outcome: rollOutcome({ successes: 0, pass: false }) }],
    }));
    const { container } = render(MessageCard, {
      props: { message: doc, showChannel: false },
      context: setAppContextForTest({ documents: storeWith(doc), t: fakeT }),
    });
    expect(container.querySelector(".roll-pass")?.textContent?.trim()).toBe("Failure");
    expect(container.querySelector(".roll-pass.fail")).not.toBeNull();
  });

  it("shows tier_label instead of pass/fail when present", () => {
    const doc = msgDoc("m1", baseSystem({
      kind: "roll",
      content: [{ kind: "roll_embed", formula: "3d10", outcome: rollOutcome({ successes: 3, pass: true, tier_label: "Critical" }) }],
    }));
    const { container } = render(MessageCard, {
      props: { message: doc, showChannel: false },
      context: setAppContextForTest({ documents: storeWith(doc), t: fakeT }),
    });
    expect(container.querySelector(".roll-tier")?.textContent).toBe("Critical");
    expect(container.querySelector(".roll-pass")).toBeNull();
  });

  it("marks dropped, crit-success, and crit-fail dice with the right classes", () => {
    const doc = msgDoc("m1", baseSystem({
      kind: "roll",
      content: [{
        kind: "roll_embed", formula: "4d6dl1",
        outcome: rollOutcome({
          records: [
            dieRecord({ value: 1, kept: false }),
            dieRecord({ value: 6, crit_success: true }),
            dieRecord({ value: 1, crit_fail: true }),
          ],
        }),
      }],
    }));
    const { container } = render(MessageCard, {
      props: { message: doc, showChannel: false },
      context: setAppContextForTest({ documents: storeWith(doc), t: fakeT }),
    });
    const chips = container.querySelectorAll(".die-chip");
    expect(chips).toHaveLength(3);
    expect(chips[0].classList.contains("dropped")).toBe(true);
    expect(chips[1].classList.contains("crit-success")).toBe(true);
    expect(chips[2].classList.contains("crit-fail")).toBe(true);
  });

  it("renders a die's label chip and space-joined symbols when present", () => {
    const doc = msgDoc("m1", baseSystem({
      kind: "roll",
      content: [{
        kind: "roll_embed", formula: "1d20[atk]",
        outcome: rollOutcome({ records: [dieRecord({ label: "atk", symbols: ["*", "!"] })] }),
      }],
    }));
    const { container } = render(MessageCard, {
      props: { message: doc, showChannel: false },
      context: setAppContextForTest({ documents: storeWith(doc), t: fakeT }),
    });
    expect(container.querySelector(".die-label")?.textContent).toBe("atk");
    expect(container.querySelector(".die-symbols")?.textContent).toBe("* !");
  });

  it("renders positive/negative counter rows only when non-zero", () => {
    const withCounters = msgDoc("m1", baseSystem({
      kind: "roll",
      content: [{ kind: "roll_embed", formula: "5d6", outcome: rollOutcome({ positive_counter: 2, negative_counter: 1 }) }],
    }));
    const shown = render(MessageCard, {
      props: { message: withCounters, showChannel: false },
      context: setAppContextForTest({ documents: storeWith(withCounters), t: fakeT }),
    });
    expect(shown.container.querySelector(".roll-counters")).not.toBeNull();
    expect(shown.container.querySelector(".counter.positive")?.textContent).toBe("+2");
    expect(shown.container.querySelector(".counter.negative")?.textContent).toBe("-1");
    shown.unmount();

    const zero = msgDoc("m2", baseSystem({
      kind: "roll",
      content: [{ kind: "roll_embed", formula: "5d6", outcome: rollOutcome() }],
    }));
    const hidden = render(MessageCard, {
      props: { message: zero, showChannel: false },
      context: setAppContextForTest({ documents: storeWith(zero), t: fakeT }),
    });
    expect(hidden.container.querySelector(".roll-counters")).toBeNull();
  });

  it("renders symbol_counts rows only when non-empty", () => {
    const withSymbols = msgDoc("m1", baseSystem({
      kind: "roll",
      content: [{ kind: "roll_embed", formula: "5d6", outcome: rollOutcome({ symbol_counts: { success: 3, advantage: 1 } }) }],
    }));
    const shown = render(MessageCard, {
      props: { message: withSymbols, showChannel: false },
      context: setAppContextForTest({ documents: storeWith(withSymbols), t: fakeT }),
    });
    expect(shown.container.querySelector(".roll-symbol-counts")).not.toBeNull();
    expect(shown.container.querySelectorAll(".symbol-count")).toHaveLength(2);
    shown.unmount();

    const empty = msgDoc("m2", baseSystem({
      kind: "roll",
      content: [{ kind: "roll_embed", formula: "5d6", outcome: rollOutcome() }],
    }));
    const hidden = render(MessageCard, {
      props: { message: empty, showChannel: false },
      context: setAppContextForTest({ documents: storeWith(empty), t: fakeT }),
    });
    expect(hidden.container.querySelector(".roll-symbol-counts")).toBeNull();
  });

  it("a kind=roll message with more than one segment falls back to the pending shell, not the block", () => {
    const doc = msgDoc("m1", baseSystem({
      kind: "roll",
      source: "/roll 1d6",
      content: [
        { kind: "roll_embed", formula: "1d6", outcome: rollOutcome() },
        { kind: "text", text: "extra" },
      ],
    }));
    const { container } = render(MessageCard, {
      props: { message: doc, showChannel: false },
      context: setAppContextForTest({ documents: storeWith(doc), t: fakeT }),
    });
    expect(container.querySelector(".roll-block")).toBeNull();
    expect(container.querySelector(".roll-pending")).not.toBeNull();
  });
});

describe("MessageCard — inline roll chip (roll_embed inside Normal/Emote content)", () => {
  it("shows successes over total, with a title tooltip of formula + kept die values", () => {
    const doc = msgDoc("m1", baseSystem({
      kind: "normal",
      content: [
        { kind: "text", text: "I rolled: " },
        {
          kind: "roll_embed", formula: "2d6",
          outcome: rollOutcome({
            total: 7, successes: 2,
            records: [dieRecord({ value: 3, kept: true }), dieRecord({ value: 4, kept: false })],
          }),
        },
      ],
    }));
    const { container } = render(MessageCard, {
      props: { message: doc, showChannel: false },
      context: setAppContextForTest({ documents: storeWith(doc), t: fakeT }),
    });
    const chip = container.querySelector(".roll-chip");
    expect(chip?.textContent).toBe("2");
    expect(chip?.getAttribute("title")).toBe("2d6: 3");
    // Not rendered as a block — inline within the normal paragraph.
    expect(container.querySelector(".roll-block")).toBeNull();
  });

  it("shows total when successes is null", () => {
    const doc = msgDoc("m1", baseSystem({
      kind: "emote",
      content: [{ kind: "roll_embed", formula: "1d20", outcome: rollOutcome({ total: 15 }) }],
    }));
    const { container } = render(MessageCard, {
      props: { message: doc, showChannel: false },
      context: setAppContextForTest({ documents: storeWith(doc), t: fakeT }),
    });
    expect(container.querySelector(".roll-chip")?.textContent).toBe("15");
  });
});

describe("MessageCard — roll button", () => {
  it("sends the exact channel/content on click", async () => {
    const doc = msgDoc("m1", baseSystem({
      channel: "ooc",
      content: [{ kind: "roll_button", formula: "1d20+5", label: "Attack" }],
    }));
    const send = vi.fn();
    render(MessageCard, {
      props: { message: doc, showChannel: false },
      context: setAppContextForTest({ documents: storeWith(doc), t: fakeT, chat: { send, edit: () => {}, delete: () => {} } }),
    });
    await fireEvent.click(screen.getByText("Attack"));
    expect(send).toHaveBeenCalledWith({ channel: "ooc", content: "/roll 1d20+5" });
  });

  it("falls back to the formula as the label when label is absent", () => {
    const doc = msgDoc("m1", baseSystem({
      content: [{ kind: "roll_button", formula: "1d20+5" }],
    }));
    const { container } = render(MessageCard, {
      props: { message: doc, showChannel: false },
      context: setAppContextForTest({ documents: storeWith(doc), t: fakeT }),
    });
    const btn = container.querySelector(".roll-btn");
    expect(btn?.textContent?.trim()).toBe("1d20+5");
  });
});

describe("MessageCard — system notices", () => {
  it("renders a muted card with the System badge and the plain-text notice body", () => {
    const doc = msgDoc("m1", baseSystem({
      kind: "system",
      content: [{ kind: "text", text: "Roll rejected: too many dice" }],
    }));
    const { container } = render(MessageCard, {
      props: { message: doc, showChannel: false },
      context: setAppContextForTest({ documents: storeWith(doc), t: fakeT }),
    });
    expect(container.querySelector(".card.system")).not.toBeNull();
    expect(container.querySelector(".chip.system-badge")?.textContent).toBe("System");
    expect(container.querySelector(".seg-text")?.textContent).toBe("Roll rejected: too many dice");
  });
});

describe("MessageCard — whitespace preservation", () => {
  it("preserves a newline in a multi-line plain-text segment via .seg-text's pre-wrap", () => {
    const doc = msgDoc("m1", baseSystem({ content: [{ kind: "text", text: "line one\nline two" }] }));
    const { container } = render(MessageCard, { props: { message: doc, showChannel: false }, context: setAppContextForTest({ documents: storeWith(doc) }) });
    const seg = container.querySelector(".seg-text");
    expect(seg).not.toBeNull();
    expect(seg?.classList.contains("seg-text")).toBe(true);
    expect(seg?.textContent).toBe("line one\nline two");
  });
});

describe("MessageCard — deletion tombstone", () => {
  it("a deleted message shows only the tombstone, suppressing body and actions", () => {
    const doc = msgDoc("m1", baseSystem({ deleted_at: 999, content: [] }));
    const { container } = render(MessageCard, {
      props: { message: doc, showChannel: false },
      context: setAppContextForTest({ documents: storeWith(doc), selfId: "u1", role: "gm" }),
    });
    expect(container.querySelector(".tombstone")).not.toBeNull();
    expect(container.querySelector(".body")).toBeNull();
    expect(container.querySelector(".actions")).toBeNull();
  });
});

describe("MessageCard — moderation actions visibility", () => {
  it("shows actions for the message owner", () => {
    const doc = msgDoc("m1", baseSystem({ user_owner: "u1" }));
    const { container } = render(MessageCard, {
      props: { message: doc, showChannel: false },
      context: setAppContextForTest({ documents: storeWith(doc), selfId: "u1", role: "player" }),
    });
    expect(container.querySelector(".actions")).not.toBeNull();
  });

  it("shows actions for a GM who is not the owner", () => {
    const doc = msgDoc("m1", baseSystem({ user_owner: "u1" }));
    const { container } = render(MessageCard, {
      props: { message: doc, showChannel: false },
      context: setAppContextForTest({ documents: storeWith(doc), selfId: "u2", role: "gm" }),
    });
    expect(container.querySelector(".actions")).not.toBeNull();
  });

  it("hides actions for another player who is neither owner nor GM", () => {
    const doc = msgDoc("m1", baseSystem({ user_owner: "u1" }));
    const { container } = render(MessageCard, {
      props: { message: doc, showChannel: false },
      context: setAppContextForTest({ documents: storeWith(doc), selfId: "u2", role: "player" }),
    });
    expect(container.querySelector(".actions")).toBeNull();
  });
});

describe("MessageCard — edit", () => {
  it("prefills the edit textarea from source when present, preferring it over textOf(content)", async () => {
    const doc = msgDoc("m1", baseSystem({ user_owner: "u1", source: "raw *markdown* source", content: [{ kind: "html", sanitized_html: "<em>markdown</em>" }] }));
    render(MessageCard, {
      props: { message: doc, showChannel: false },
      context: setAppContextForTest({ documents: storeWith(doc), selfId: "u1", role: "player" }),
    });
    await fireEvent.click(screen.getByText("chat.edit"));
    expect((screen.getByLabelText("chat.edit") as HTMLTextAreaElement).value).toBe("raw *markdown* source");
  });

  it("falls back to the concatenated text segments when source is absent", async () => {
    const doc = msgDoc("m1", baseSystem({ user_owner: "u1", content: [{ kind: "text", text: "hello " }, { kind: "text", text: "world" }] }));
    render(MessageCard, {
      props: { message: doc, showChannel: false },
      context: setAppContextForTest({ documents: storeWith(doc), selfId: "u1", role: "player" }),
    });
    await fireEvent.click(screen.getByText("chat.edit"));
    expect((screen.getByLabelText("chat.edit") as HTMLTextAreaElement).value).toBe("hello world");
  });

  it("Save dispatches ctx.chat.edit(messageId, draft) and exits edit mode", async () => {
    const doc = msgDoc("m1", baseSystem({ user_owner: "u1", content: [{ kind: "text", text: "hi" }] }));
    const edit = vi.fn();
    render(MessageCard, {
      props: { message: doc, showChannel: false },
      context: setAppContextForTest({ documents: storeWith(doc), selfId: "u1", role: "player", chat: { send: () => {}, edit, delete: () => {} } }),
    });
    await fireEvent.click(screen.getByText("chat.edit"));
    const textarea = screen.getByLabelText("chat.edit") as HTMLTextAreaElement;
    await fireEvent.input(textarea, { target: { value: "edited text" } });
    await fireEvent.click(screen.getByText("chat.save"));
    expect(edit).toHaveBeenCalledWith("m1", "edited text");
    expect(screen.queryByLabelText("chat.edit")).toBeNull();
  });

  it("Cancel reverts without calling ctx.chat.edit", async () => {
    const doc = msgDoc("m1", baseSystem({ user_owner: "u1", content: [{ kind: "text", text: "hi" }] }));
    const edit = vi.fn();
    render(MessageCard, {
      props: { message: doc, showChannel: false },
      context: setAppContextForTest({ documents: storeWith(doc), selfId: "u1", role: "player", chat: { send: () => {}, edit, delete: () => {} } }),
    });
    await fireEvent.click(screen.getByText("chat.edit"));
    await fireEvent.click(screen.getByText("chat.cancel"));
    expect(edit).not.toHaveBeenCalled();
    expect(screen.queryByLabelText("chat.edit")).toBeNull();
  });
});

describe("MessageCard — delete", () => {
  afterEach(() => {
    vi.restoreAllMocks();
  });

  it("Delete calls ctx.chat.delete only after window.confirm returns true", async () => {
    const doc = msgDoc("m1", baseSystem({ user_owner: "u1" }));
    const del = vi.fn();
    vi.spyOn(window, "confirm").mockReturnValue(true);
    render(MessageCard, {
      props: { message: doc, showChannel: false },
      context: setAppContextForTest({ documents: storeWith(doc), selfId: "u1", role: "player", chat: { send: () => {}, edit: () => {}, delete: del } }),
    });
    await fireEvent.click(screen.getByText("chat.delete"));
    expect(window.confirm).toHaveBeenCalledWith("chat.deleteConfirm");
    expect(del).toHaveBeenCalledWith("m1");
  });

  it("Delete does not call ctx.chat.delete when window.confirm returns false", async () => {
    const doc = msgDoc("m1", baseSystem({ user_owner: "u1" }));
    const del = vi.fn();
    vi.spyOn(window, "confirm").mockReturnValue(false);
    render(MessageCard, {
      props: { message: doc, showChannel: false },
      context: setAppContextForTest({ documents: storeWith(doc), selfId: "u1", role: "player", chat: { send: () => {}, edit: () => {}, delete: del } }),
    });
    await fireEvent.click(screen.getByText("chat.delete"));
    expect(del).not.toHaveBeenCalled();
  });
});

describe("MessageCard — actor attribution + redaction fixtures (real resolveTokenActor inputs)", () => {
  it("resolves an actor_owner{kind:'actor'} to the actor's real name", () => {
    const actor = buildActorDoc("w1", actorSystem({ name: "Grog" }), "actor1");
    const doc = msgDoc("m1", baseSystem({ actor_owner: { kind: "actor", actor_id: "actor1" } }));
    const { container } = render(MessageCard, { props: { message: doc, showChannel: false }, context: setAppContextForTest({ documents: storeWith(actor, doc) }) });
    expect(container.querySelector(".actor-name")?.textContent).toBe("(Grog)");
  });

  it("resolves an actor_owner{kind:'token_instance'} to the embedded instance's real name", () => {
    const actor = buildActorDoc("w1", actorSystem({ name: "Grog" }), "actor1");
    const token = buildTokenFromActor("w1", "scene1", actor, "instance", { x: 0, y: 0 }, 100, "token1");
    const doc = msgDoc("m1", baseSystem({ actor_owner: { kind: "token_instance", token_id: "token1" } }));
    const { container } = render(MessageCard, { props: { message: doc, showChannel: false }, context: setAppContextForTest({ documents: storeWith(actor, token, doc) }) });
    expect(container.querySelector(".actor-name")?.textContent).toBe("(Grog)");
  });

  it("a hidden actor name (server-redacted: /system/name absent) renders the displayName fallback for a non-GM viewer", () => {
    // Simulates what a non-owner, non-GM player actually receives on the wire once the
    // server strips an OwnerOrGm-tiered /system/name — the key is ABSENT, not empty, per
    // the documents-permissions redaction invariant (stripped before transmission).
    const redactedSystem = actorSystem({ displayName: "The Mysterious Figure" });
    delete (redactedSystem as Partial<ActorSystem>).name;
    const actor = buildActorDoc("w1", redactedSystem, "actor1");
    const doc = msgDoc("m1", baseSystem({ actor_owner: { kind: "actor", actor_id: "actor1" } }));
    const { container } = render(MessageCard, { props: { message: doc, showChannel: false }, context: setAppContextForTest({ documents: storeWith(actor, doc), role: "player", selfId: "someone-else" }) });
    expect(container.querySelector(".actor-name")?.textContent).toBe("(The Mysterious Figure)");
  });

  it("a dangling token_instance reference fails closed (no actor name shown, no throw)", () => {
    const doc = msgDoc("m1", baseSystem({ actor_owner: { kind: "token_instance", token_id: "does-not-exist" } }));
    const { container } = render(MessageCard, { props: { message: doc, showChannel: false }, context: setAppContextForTest({ documents: storeWith(doc) }) });
    expect(container.querySelector(".actor-name")).toBeNull();
    expect(container.querySelector(".author")).not.toBeNull(); // rest of the header still renders
  });

  it("a dangling actor reference fails closed (no actor name shown, no throw)", () => {
    const doc = msgDoc("m1", baseSystem({ actor_owner: { kind: "actor", actor_id: "does-not-exist" } }));
    const { container } = render(MessageCard, { props: { message: doc, showChannel: false }, context: setAppContextForTest({ documents: storeWith(doc) }) });
    expect(container.querySelector(".actor-name")).toBeNull();
    expect(container.querySelector(".author")).not.toBeNull(); // rest of the header still renders
  });

  it("a message with no actor_owner shows no actor-name span", () => {
    const doc = msgDoc("m1", baseSystem());
    const { container } = render(MessageCard, { props: { message: doc, showChannel: false }, context: setAppContextForTest({ documents: storeWith(doc) }) });
    expect(container.querySelector(".actor-name")).toBeNull();
  });
});
