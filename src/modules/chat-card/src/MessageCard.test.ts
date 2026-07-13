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
 * (chat.rollPending / chat.whisperTo) whose whole value is parameterized. */
function fakeT(key: string, params?: Record<string, string | number>): string {
  const templates: Record<string, string> = {
    "chat.rollPending": "🎲 {formula}",
    "chat.whisperTo": "to {names}",
  };
  let s = templates[key] ?? key;
  if (params) for (const [k, v] of Object.entries(params)) s = s.replaceAll(`{${k}}`, String(v));
  return s;
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
    const doc = msgDoc("m1", baseSystem({ content: [{ kind: "text", text: "a" }, { kind: "roll_embed", data: 1 }] }));
    const { container } = render(MessageCard, { props: { message: doc, showChannel: false }, context: setAppContextForTest({ documents: storeWith(doc) }) });
    expect(container.querySelectorAll(".seg-text, .seg-html").length).toBe(1);
    expect(container.textContent).toContain("a");
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
    const line = container.querySelector(".emote-line");
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

  it("a message with no actor_owner shows no actor-name span", () => {
    const doc = msgDoc("m1", baseSystem());
    const { container } = render(MessageCard, { props: { message: doc, showChannel: false }, context: setAppContextForTest({ documents: storeWith(doc) }) });
    expect(container.querySelector(".actor-name")).toBeNull();
  });
});
