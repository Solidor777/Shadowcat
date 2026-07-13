# M11d-1 · Tabbed Sidebar + Chat Display Modules — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development
> (chosen for this run — Sonnet `shadowcat-coder` implementers, this session as dispatcher) or
> superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`)
> syntax for tracking.

**Goal:** The client display layer for chat (panel host + composer + message card as three
replaceable modules) mounted in a new Foundry-style tabbed sidebar (itself a replaceable
module), plus three contained server enablers (`MessageSystem.source`, emoji shortcodes,
member-visible member list).

**Architecture:** Spec `docs/superpowers/specs/2026-07-13-m11d-1-tabbed-sidebar-chat-display-design.md`
(committed). UI-is-modules everywhere: core-ui hosts a new singleton `sidebar-host` surface;
`@shadowcat/module-sidebar` provides the tab host; every panel is a tab contribution; chat's
composer and card are contributions into chat-host-declared surfaces. Messages are ordinary
documents already delivered/redacted by the existing store — the client only renders.

**Tech Stack:** Svelte 5 (runes), TS, Zod, SCSS tokens, Vitest + @testing-library/svelte, Rust
(axum/serde) for the server enablers.

## Model/Effort directives

Plan written MAINLINE by the design session (Fable 5 / effort high) — the user was asleep at
the tier-switch checkpoint; precedent (M11b/M11c: user declined `sdd-plan-writer-*` dispatch)
plus full in-context design knowledge favored mainline. Execution: user directive this session —
`shadowcat-coder` (Sonnet, `effort: medium`) per implementation task, dispatched as UNNAMED
one-off Agent calls ([[buddy-check-named-teammate-unreliable]]); reviewers
`shadowcat-spec-reviewer` + `shadowcat-code-reviewer` at `effort: high`; `-opus` twins on
BLOCKED/shallow. Trivial mechanical diffs may be done by the dispatcher mainline.

## Buddy-check directives

- **Server-enablers unit (Tasks 2+3+4):** after all three land, ONE buddy-check over the
  combined diff (two blind reviewers + debate; PHASE = code). Rationale: Task 2 has a
  content-leak edge (deleted `source` must clear), Task 4 is an authz widening; reviewed "as a
  unit" per [[m11c3-buddy-check-seam-scoping]].
- **Task 11 (message card):** standard two-reviewer gate, but the review brief MUST direct both
  reviewers at the `{@html}` boundary: prove no path renders `Segment::Text` (or any
  non-`chat::sanitize`-produced string) through `{@html}`, and that a malformed body renders
  nothing rather than partially.
- Everything else: standard per-task two-reviewer gate.

## Global Constraints

- Workspace gates (all must pass before any merge): `pnpm -r test`, `pnpm -r exec tsc --noEmit`
  (or per-package `typecheck` scripts), `pnpm lint`, and from `src/server/`: `cargo test`,
  `cargo clippy --all-targets -- -D warnings` (pre-existing known failure:
  `scene/move_exec.rs` `region_doc` `too_many_arguments` — tracked in TODO.md, not yours),
  `cargo fmt --check`. Client `pnpm build` BEFORE any cargo build (rust-embed).
- `@shadowcat/core` stays Svelte-free (framework-neutral TS only).
- Modules communicate ONLY through seams (contracts, ContributionRegistry, Surface, AppContext)
  — never import each other or the shell.
- Comments: present-tense, invariants/coupling first, no history/process meta (CLAUDE.md rules).
- Message bodies: `Segment::Html` is innerHTML-safe ONLY because `chat::sanitize` produced it —
  every render site states this invariant.
- New UI: SCSS theme tokens (`var(--surface-*)`, `var(--text-*)`, `var(--border)`), touch
  targets ≥ 44px, no hardcoded colors.
- Plan-vs-code drift (M11a lesson): implementers READ THE REAL FILES first; where this plan's
  snippet disagrees with the tree, the tree wins and the deviation is reported.

## File Structure (created/modified)

```
src/client/core/src/contributions.ts        [M] Contribution.tab metadata
src/client/core/src/chat-docs.ts            [C] Zod mirrors + channel registry + builders
src/client/core/src/scene-docs.ts           [M] export envelope()
src/client/core/src/ws-client.ts            [M] sendChatMessage/editChatMessage/deleteChatMessage
src/client/core/src/index.ts                [M] exports
src/client/ui-kit/src/TabbedSurface.svelte  [C] generic tabbed surface host
src/client/ui-kit/src/appContext.ts         [M] chat + uiState seams
src/client/ui-kit/src/index.ts              [M] exports
src/client/ui-kit/src/locales/en.ts         [M] new keys (per-task)
src/client/shell/src/lib/sessionState.svelte.ts [M] activeTab get/set
src/client/shell/src/lib/Table.svelte       [M] wire chat + uiState
src/client/shell/src/lib/worldSession.svelte.ts [M] members for all roles; chat delegates
src/client/shell/src/App.svelte             [M] register 4 new modules
src/modules/core-ui/src/index.ts            [M] sidebar → sidebar-host provide
src/modules/core-ui/src/Layout.svelte       [M] region hosts sidebar-host
src/modules/sidebar/**                      [C] @shadowcat/module-sidebar
src/modules/chat/**                         [C] @shadowcat/module-chat (host)
src/modules/chat-composer/**                [C] @shadowcat/module-chat-composer
src/modules/chat-card/**                    [C] @shadowcat/module-chat-card
src/modules/{assets,actors,factions,conditions,game-settings}/src/index.ts [M] tab metadata
src/server/src/chat/mod.rs                  [M] MessageSystem.source lifecycle
src/server/src/chat/shortcodes.rs           [C] :name: → unicode
src/server/src/chat/sanitize.rs             [M] shortcode pre-pass
src/server/src/http/routes.rs               [M] list_members member-gated
```

New module packages copy the factions module's `package.json` / `svelte.config.js` /
`tsconfig.json` / `vitest.config.ts` / `vitest.setup.ts` verbatim (name changed). pnpm picks
them up via the existing `src/modules/*` workspace glob — run `pnpm install` after creating a
package so the workspace links it.

---

### Task 1: Core — `Contribution.tab` metadata + `chat-docs.ts` mirrors and builders

**Files:**
- Modify: `src/client/core/src/contributions.ts` (add `tab` field)
- Modify: `src/client/core/src/scene-docs.ts` (export `envelope`)
- Create: `src/client/core/src/chat-docs.ts`
- Create: `src/client/core/src/chat-docs.test.ts`
- Modify: `src/client/core/src/index.ts` (exports)

**Interfaces (produced — later tasks rely on these exact names):**
```ts
// contributions.ts
export interface ContributionTab { icon: string; labelKey: string; gmOnly?: boolean }
export interface Contribution { /* existing */; tab?: ContributionTab }
// chat-docs.ts
export type MessageKind = "normal" | "emote" | "roll" | "system";
export type ChatSegment = { kind: "text"; text: string } | { kind: "html"; sanitized_html: string };
export type UnknownSegment = { kind: string };
export interface ChatMessageSystem {
  channel: string; user_owner: string; actor_owner?: WireActorOwnerRef | null;
  kind: MessageKind; audience: WireAudience; content: (ChatSegment | UnknownSegment)[];
  source?: string | null; edited_at?: number | null; deleted_at?: number | null;
}
export function parseMessageSystem(doc: WireDocument): ChatMessageSystem | null;
export function isKnownSegment(s: ChatSegment | UnknownSegment): s is ChatSegment;
export interface ChatChannel { name: string }
export interface ChannelRegistrySystem { channels: Record<string, ChatChannel> }
export function buildChannelRegistryDoc(worldId: string, channels: Record<string, ChatChannel>, id?: string): WireDocument;
export const MESSAGE_DOC_TYPE = "message";
export const CHANNEL_REGISTRY_DOC_TYPE = "channel-registry";
```

- [ ] **Step 1: failing tests** — `chat-docs.test.ts`:

```ts
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
```

- [ ] **Step 2:** `pnpm --filter @shadowcat/core test` → FAIL (module missing).
- [ ] **Step 3: implement.** `contributions.ts` — add above `Contribution`:

```ts
/** Optional tab metadata a tabbed host (e.g. the sidebar) renders for a contribution.
 * Plain data — framework-neutral. `labelKey` is an i18n key the HOST resolves at
 * render (locale-reactive); `gmOnly` tabs are hidden from non-GM users by the host. */
export interface ContributionTab {
  icon: string;
  labelKey: string;
  gmOnly?: boolean;
}
```
and inside `Contribution`: `tab?: ContributionTab;` (after `props`).

`scene-docs.ts`: change `function envelope(` to `export function envelope(` and extend its doc
comment: `/** Package-internal document envelope builder (shared by scene-docs and chat-docs). */`

`chat-docs.ts` (complete file):

```ts
// Client mirror of the chat message content model (src/server/src/chat/mod.rs).
// The body types are serde-only on the server (NO ts-rs) — this file is the
// manually-kept-in-sync Zod mirror; a Rust body-shape change MUST update it.
// Fail-closed: a body that does not parse renders as nothing, never partially.
import { z } from "zod";
import { ActorOwnerRefSchema, AudienceSchema, type WireDocument } from "./wire";
import { envelope } from "./scene-docs";

export const MESSAGE_DOC_TYPE = "message";
export const CHANNEL_REGISTRY_DOC_TYPE = "channel-registry";
/** Server-enforced content cap (chat/mod.rs MAX_MESSAGE_CHARS) — composer pre-validates. */
export const MAX_MESSAGE_CHARS = 4096;

export const MessageKindSchema = z.enum(["normal", "emote", "roll", "system"]);
export type MessageKind = z.infer<typeof MessageKindSchema>;

/** Known segment kinds. `html.sanitized_html` is innerHTML-safe ONLY because the
 * server's chat::sanitize (ammonia) produced it — no client code may construct one. */
export const ChatSegmentSchema = z.discriminatedUnion("kind", [
  z.object({ kind: z.literal("text"), text: z.string() }),
  z.object({ kind: z.literal("html"), sanitized_html: z.string() }),
]);
export type ChatSegment = z.infer<typeof ChatSegmentSchema>;
/** Forward-compat: a segment kind this client doesn't know (e.g. a newer server's
 * roll_embed) parses as opaque and renders as nothing — the message still shows. */
const UnknownSegmentSchema = z.object({ kind: z.string() }).passthrough();
export type UnknownSegment = z.infer<typeof UnknownSegmentSchema>;
const SegmentListSchema = z.array(z.union([ChatSegmentSchema, UnknownSegmentSchema]));

export function isKnownSegment(s: ChatSegment | UnknownSegment): s is ChatSegment {
  return s.kind === "text" || s.kind === "html";
}

export const ChatMessageSystemSchema = z.object({
  channel: z.string(),
  user_owner: z.string(),
  actor_owner: ActorOwnerRefSchema.nullish(),
  kind: MessageKindSchema,
  audience: AudienceSchema.default({ kind: "public" }),
  content: SegmentListSchema,
  source: z.string().nullish(),
  edited_at: z.number().nullish(),
  deleted_at: z.number().nullish(),
});
export type ChatMessageSystem = z.infer<typeof ChatMessageSystemSchema>;

/** Fail-closed body parse: null unless `doc` is a message with a valid body. */
export function parseMessageSystem(doc: WireDocument): ChatMessageSystem | null {
  if (doc.doc_type !== MESSAGE_DOC_TYPE) return null;
  const r = ChatMessageSystemSchema.safeParse(doc.system);
  return r.success ? r.data : null;
}

/** A chat channel's display config. Channels are a purely client-side label
 * taxonomy — the server never validates `channel` (chat skill: audience, not
 * channel, is the only server-enforced visibility). */
export interface ChatChannel {
  name: string;
}
/** Singleton config doc (doc_type "channel-registry"): id→channel MAP, so
 * add/rename/remove are single-key field Updates (set_pointer cannot grow arrays). */
export interface ChannelRegistrySystem {
  channels: Record<string, ChatChannel>;
}
export function buildChannelRegistryDoc(
  worldId: string,
  channels: Record<string, ChatChannel>,
  id?: string,
): WireDocument {
  return envelope(worldId, CHANNEL_REGISTRY_DOC_TYPE, null, { channels } satisfies ChannelRegistrySystem, id);
}
```

`index.ts` — add:
```ts
export { MESSAGE_DOC_TYPE, CHANNEL_REGISTRY_DOC_TYPE, MAX_MESSAGE_CHARS, MessageKindSchema, ChatSegmentSchema, ChatMessageSystemSchema, parseMessageSystem, isKnownSegment, buildChannelRegistryDoc } from "./chat-docs";
export type { MessageKind, ChatSegment, UnknownSegment, ChatMessageSystem, ChatChannel, ChannelRegistrySystem } from "./chat-docs";
export type { ContributionTab } from "./contributions";
```

- [ ] **Step 4:** `pnpm --filter @shadowcat/core test` → PASS; `pnpm --filter @shadowcat/core typecheck` → clean.
- [ ] **Step 5:** Commit: `feat(chat/m11d-1): contribution tab metadata + client chat body mirror`

---

### Task 2: Server — `MessageSystem.source` lifecycle

**Files:**
- Modify: `src/server/src/chat/mod.rs`

**Interfaces:**
- Produces: `MessageSystem.source: Option<String>`; `build_message_doc(..., source: Option<String>, ...)` (new param before `now`).
- Rule (pin in code comment): `source` = the author's raw input for edit-prefill, with a parsed
  `/w`-prefix STRIPPED (an unmodified resubmit of a whisper prefill must not trip the edit
  path's `AudienceLocked` rejection): `if parsed.whisper_to.is_some() { parsed.body } else { content }`.
  Replaced on edit under the same rule (edit rejects `/w`, so there it is always the full
  content). CLEARED (`None`) by the delete tombstone alongside `content` — leaving it leaks
  deleted content through the envelope.

- [ ] **Step 1: failing tests** in `chat/mod.rs` `#[cfg(test)]` (follow the file's existing
  test-helper idiom — read it first; helpers like `build_message_doc` callers at ~579-722 need
  the new arg):

```rust
#[test]
fn source_stores_raw_input_for_plain_and_command_messages() {
    // plain: source == the full content
    // /me waves: source == "/me waves" (command prefix KEPT — prefill re-parses to the same kind)
    // /w @alice hi (via handle_send_message path): source == "hi" (the /w prefix STRIPPED)
}
#[test]
fn edit_replaces_source_and_delete_clears_it() {
    // after handle_edit_message: sys.source == Some(new content)
    // after handle_delete_message: sys.source == None AND sys.content is empty
}
#[test]
fn stored_pre_source_message_still_deserializes() {
    // serde_json round-trip of a MessageSystem JSON WITHOUT the `source` key parses (None)
}
```
(Write these as real tests against the file's existing async-test scaffolding — the sketch
above names the behaviors; the assertions must be executable code.)

- [ ] **Step 2:** `cargo test -p shadowcat chat::` → FAIL.
- [ ] **Step 3: implement.**
  - `MessageSystem`: add after `content`:
    ```rust
    /// The author's raw input (post-`/w`-strip), kept for client edit-prefill —
    /// sanitized `Segment::Html` cannot be reversed to author input. Data only,
    /// never rendered as markup. MUST be cleared by the delete tombstone with
    /// `content` (a retained source would leak deleted content).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    ```
  - `build_message_doc(...)`: add `source: Option<String>` parameter (before `now`); set
    `source` in the `MessageSystem` literal. Update every existing call site (tests included).
  - `handle_send_message`: at the `build_message_doc` call, pass
    `Some(if had_whisper { parsed.body.clone() } else { content.clone() })` — introduce
    `let had_whisper = parsed.whisper_to.is_some();` BEFORE `parsed.whisper_to` is consumed at
    line ~316.
  - `handle_edit_message`: alongside `sys.kind = parsed.kind;` add `sys.source = Some(content.clone());`
    (edit rejects `/w`, so the full content is always correct here).
  - `handle_delete_message`: alongside `sys.content = Vec::new();` add `sys.source = None;`.
- [ ] **Step 4:** `cargo test` (whole crate) → PASS; `cargo fmt`; `cargo clippy --all-targets -- -D warnings` (modulo the pre-existing known failure).
- [ ] **Step 5:** Commit: `feat(chat/m11d-1): store raw message source for edit prefill; cleared on delete`

---

### Task 3: Server — emoji shortcodes in the sanitize pipeline

**Files:**
- Create: `src/server/src/chat/shortcodes.rs`
- Modify: `src/server/src/chat/sanitize.rs` (pre-pass), `src/server/src/chat/mod.rs` (`mod shortcodes;`)

**Interfaces:**
- Produces: `pub(crate) fn replace_shortcodes(raw: &str) -> Cow<'_, str>` — replaces every
  `:name:` (name = `[a-z0-9_+-]+`) that matches the curated map; unknown names pass through
  verbatim; returns `Cow::Borrowed` when nothing matched.
- `sanitize()` gains as its FIRST line: `let raw: &str = &replace_shortcodes(raw);` — runs for
  BOTH the plain-text early-return and the enriched path, so stored content is final and
  policy-independent. Always-on (typing sugar, not enrichment — no policy toggle).

- [ ] **Step 1: failing tests** (in `shortcodes.rs` + one pipeline test in `sanitize.rs`):

```rust
#[test]
fn replaces_known_shortcodes_and_keeps_unknown() {
    assert_eq!(replace_shortcodes("hi :smile:!"), "hi 😄!");
    assert_eq!(replace_shortcodes(":+1: and :unknown_thing: and :d20:"), "👍 and :unknown_thing: and 🎲");
    assert_eq!(replace_shortcodes("no codes"), "no codes"); // borrowed passthrough
    assert_eq!(replace_shortcodes("a:b: :c"), "a:b: :c"); // malformed → untouched
    assert_eq!(replace_shortcodes("::smile::"), ":😄:"); // inner match only
}
// sanitize.rs:
#[test]
fn sanitize_replaces_shortcodes_in_plain_text_mode() {
    let policy = ChatContentPolicy::default(); // everything off
    let out = sanitize("gg :heart:", &policy);
    assert_eq!(out, vec![Segment::Text { text: "gg ❤️".into() }]);
}
```

- [ ] **Step 2:** `cargo test -p shadowcat chat::` → FAIL.
- [ ] **Step 3: implement** `shortcodes.rs` — a `&'static [(&str, &str)]` sorted table +
  binary search (no new deps, no lazy-static), single left-to-right scanner:

```rust
//! `:shortcode:` → unicode emoji, applied to raw chat input BEFORE any
//! markdown/html processing (sanitize.rs pre-pass) so stored content is final
//! and identical under every content policy. Always-on typing sugar — output is
//! plain unicode text with no security surface. v1 limitation (documented in the
//! design spec): replacement is pre-parse, so a shortcode inside a markdown code
//! span is also replaced.
use std::borrow::Cow;

/// Sorted by name (binary-searched). Curated common set; extend freely.
const TABLE: &[(&str, &str)] = &[
    ("+1", "👍"), ("-1", "👎"), ("100", "💯"), ("angry", "😠"),
    ("cat", "🐱"), ("check", "✅"), ("clap", "👏"), ("cool", "😎"),
    ("cry", "😢"), ("crossed_swords", "⚔️"), ("crown", "👑"), ("d20", "🎲"),
    ("dagger", "🗡️"), ("dog", "🐶"), ("dragon", "🐉"), ("eyes", "👀"),
    ("fire", "🔥"), ("ghost", "👻"), ("grin", "😁"), ("heart", "❤️"),
    ("hourglass", "⏳"), ("joy", "😂"), ("key", "🗝️"), ("laughing", "😆"),
    ("lightning", "⚡"), ("mage", "🧙"), ("map", "🗺️"), ("moneybag", "💰"),
    ("moon", "🌙"), ("muscle", "💪"), ("neutral_face", "😐"), ("party", "🎉"),
    ("pray", "🙏"), ("rage", "😡"), ("rofl", "🤣"), ("sad", "😞"),
    ("scream", "😱"), ("shield", "🛡️"), ("skull", "💀"), ("sleep", "😴"),
    ("smile", "😄"), ("smirk", "😏"), ("sparkles", "✨"), ("star", "⭐"),
    ("sun", "☀️"), ("sweat", "😅"), ("sword", "🗡️"), ("tada", "🎉"),
    ("thinking", "🤔"), ("thumbsdown", "👎"), ("thumbsup", "👍"), ("wave", "👋"),
    ("wink", "😉"), ("wizard", "🧙"), ("x", "❌"), ("zzz", "💤"),
];

fn lookup(name: &str) -> Option<&'static str> {
    TABLE.binary_search_by_key(&name, |(n, _)| n).ok().map(|i| TABLE[i].1)
}

fn is_name_char(c: char) -> bool {
    c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '_' | '+' | '-')
}

/// Replace every `:name:` whose name is in the table; everything else passes
/// through verbatim. Borrowed (zero-alloc) when nothing matches.
pub(crate) fn replace_shortcodes(raw: &str) -> Cow<'_, str> {
    let bytes = raw.as_bytes();
    let mut out: Option<String> = None;
    let mut i = 0;
    let mut last_emit = 0;
    while i < bytes.len() {
        if bytes[i] == b':' {
            // Find a closing ':' with a valid, non-empty name between.
            let start = i + 1;
            let mut j = start;
            while j < bytes.len() && is_name_char(bytes[j] as char) {
                j += 1;
            }
            if j > start && j < bytes.len() && bytes[j] == b':' {
                if let Some(emoji) = lookup(&raw[start..j]) {
                    let out = out.get_or_insert_with(|| String::with_capacity(raw.len()));
                    out.push_str(&raw[last_emit..i]);
                    out.push_str(emoji);
                    i = j + 1;
                    last_emit = i;
                    continue;
                }
            }
        }
        i += 1;
    }
    match out {
        None => Cow::Borrowed(raw),
        Some(mut s) => {
            s.push_str(&raw[last_emit..]);
            Cow::Owned(s)
        }
    }
}
```
  `sanitize.rs`: first line of `sanitize()` body:
  ```rust
  let replaced = super::shortcodes::replace_shortcodes(raw);
  let raw: &str = &replaced;
  ```
  `mod.rs`: `mod shortcodes;` near the other module decls.
- [ ] **Step 4:** `cargo test` → PASS; fmt/clippy clean.
- [ ] **Step 5:** Commit: `feat(chat/m11d-1): server-side emoji shortcode replacement in the sanitize pipeline`

---

### Task 4: Server + client — member-visible member list

**Files:**
- Modify: `src/server/src/http/routes.rs` (`list_members`, ~line 344)
- Modify: `src/client/shell/src/lib/worldSession.svelte.ts` (~line 315 fetch site)
- Modify: `src/client/ui-kit/src/appContext.ts` (members doc comment)

- [ ] **Step 1: failing server test** (follow routes' existing test idiom — real world + real
  users via `create_user`, [[server-test-doc-helper-and-owner-fk]]):

```rust
#[tokio::test]
async fn members_list_visible_to_every_member_but_not_outsiders() {
    // player GET /api/worlds/{w}/members → 200, contains the GM and themselves
    // spectator → 200
    // authenticated non-member → 403 (world-scoped route keeps Forbidden; only
    //   by-id doc routes remap to 404 — see by_id_not_found's comment)
}
```
- [ ] **Step 2:** run → FAIL (player currently 403).
- [ ] **Step 3: implement.** In `list_members`, replace `require_gm(&state, &user, world).await?;` with:
```rust
// Any world member may list the roster: the chat card resolves user ids to
// usernames for every viewer (author names, whisper recipient labels), and a
// table's roster is inherently visible in shared play. permission_context
// rejects non-members (Forbidden) — server admins resolve to GM.
state.repo.permission_context(world, user.id, user.role).await?;
```
  Client `worldSession.svelte.ts`: remove the `if (w.user_role === "gm")` guard around the
  members fetch (keep the try/catch + in-place mutation), and update the comment: members now
  resolve chat author/recipient names for every role; see-as labels remain the GM use.
  `appContext.ts`: members doc comment → `/** userId → username for the world's members (all roles; used for chat name resolution + GM see-as labels). */`
- [ ] **Step 4:** `cargo test` → PASS; `pnpm --filter @shadowcat/shell test` + typecheck → PASS.
- [ ] **Step 5:** Commit: `feat(chat/m11d-1): member-visible world roster for chat name resolution`

> **After Tasks 2–4: run the pre-authorized buddy-check on the combined server-enabler diff
> (see Buddy-check directives), fix findings, re-verify before proceeding.**

---

### Task 5: ui-kit — `TabbedSurface.svelte`

**Files:**
- Create: `src/client/ui-kit/src/TabbedSurface.svelte`
- Create: `src/client/ui-kit/src/TabbedSurface.test.ts`
- Modify: `src/client/ui-kit/src/index.ts` (`export { default as TabbedSurface } from "./TabbedSurface.svelte";`)
- Modify: `src/client/ui-kit/src/locales/en.ts` (add `"sidebar.collapse": "Collapse sidebar"`, `"sidebar.expand": "Expand sidebar"`)

**Interfaces:**
- Consumes: `Contribution.tab` (Task 1), `getAppContext()` (`contributions`, `role`, `t`).
- Produces: `<TabbedSurface contract activeId onTabChange>` — props
  `{ contract: string; activeId?: string | null; onTabChange?: (id: string) => void }`.

- [ ] **Step 1: failing tests** (@testing-library/svelte, follow an existing ui-kit component
  test for the AppContext fixture idiom — ui-kit has a `/test` fixture subpath from M8.5a; read
  it first):

```ts
// Renders one rail button per contribution (icon + aria-label from t(labelKey));
// gmOnly tab hidden for role "player", shown for "gm";
// fallback metadata (icon = first char of id, label = id) when tab is absent;
// ALL tab panels are mounted; only the active one is visible (hidden attr);
// clicking a tab calls onTabChange with the id and switches visibility;
// activeId prop selects the tab; an activeId not in the visible set falls back to the first;
// collapse toggle hides the content area and clicking a tab while collapsed re-expands.
```
Write each as a real `test(...)` with two stub Svelte components contributed into a fresh
`ContributionRegistry` (one with `tab: { icon: "💬", labelKey: "chat.tab" }`, one with
`tab: { icon: "⚙️", labelKey: "gameSettings.tab", gmOnly: true }`, one with no `tab`).

- [ ] **Step 2:** `pnpm --filter @shadowcat/ui-kit test` → FAIL.
- [ ] **Step 3: implement:**

```svelte
<script lang="ts">
  import type { Component } from "svelte";
  import { createSubscriber } from "svelte/reactivity";
  import { getAppContext } from "./appContext";

  let {
    contract,
    activeId = null,
    onTabChange,
  }: {
    contract: string;
    activeId?: string | null;
    onTabChange?: (id: string) => void;
  } = $props();

  const ctx = getAppContext();
  const t = ctx.t;

  const subscribe = createSubscriber((update) => ctx.contributions.subscribe(update));
  // gmOnly tabs are host-filtered for non-GM (panels may additionally self-gate).
  const items = $derived.by(() => {
    subscribe();
    return ctx.contributions
      .contributionsFor(contract)
      .filter((c) => !(c.tab?.gmOnly && ctx.role !== "gm"));
  });
  // activeId wins when it names a visible tab; otherwise first visible.
  const active = $derived(items.find((c) => c.id === activeId)?.id ?? items[0]?.id ?? null);

  let collapsed = $state(false);

  function pick(id: string): void {
    if (collapsed) collapsed = false;
    onTabChange?.(id);
  }
  function label(c: (typeof items)[number]): string {
    return c.tab ? t(c.tab.labelKey) : c.id;
  }
</script>

<div class="tabbed" class:collapsed>
  <nav class="rail" aria-orientation="vertical" role="tablist">
    <button
      type="button"
      class="rail-btn toggle"
      aria-label={collapsed ? t("sidebar.expand") : t("sidebar.collapse")}
      title={collapsed ? t("sidebar.expand") : t("sidebar.collapse")}
      onclick={() => (collapsed = !collapsed)}
    >{collapsed ? "◂" : "▸"}</button>
    {#each items as c (c.id)}
      <button
        type="button"
        class="rail-btn"
        role="tab"
        aria-selected={c.id === active}
        aria-label={label(c)}
        title={label(c)}
        data-testid="tab-{c.id}"
        onclick={() => pick(c.id)}
      >{c.tab?.icon ?? c.id.slice(0, 1)}</button>
    {/each}
  </nav>
  {#if !collapsed}
    <div class="content">
      <!-- Every panel stays mounted (state/scroll preserved; GM seed $effects run
           regardless of the active tab); the inactive ones are display:none. -->
      {#each items as c (c.id)}
        {@const Comp = c.component as Component<Record<string, unknown>>}
        <div class="panel" role="tabpanel" hidden={c.id !== active} data-testid="panel-{c.id}">
          <Comp {...(c.props ?? {})} />
        </div>
      {/each}
    </div>
  {/if}
</div>

<style lang="scss">
  .tabbed {
    display: flex;
    flex-direction: row-reverse; /* rail on the outer (right) edge */
    height: 100%;
    min-height: 0;
  }
  .rail {
    display: flex;
    flex-direction: column;
    gap: 0.25rem;
    padding: 0.25rem;
    border-left: 1px solid var(--border);
    background: var(--surface-overlay);
  }
  .rail-btn {
    /* Touch target floor (mobile invariant). */
    min-width: 44px;
    min-height: 44px;
    border: none;
    border-radius: 0.375rem;
    background: transparent;
    color: var(--text-primary);
    font-size: 1.25rem;
    cursor: pointer;
    &:hover { background: var(--surface-raised); }
    &[aria-selected="true"] { background: var(--surface-base); }
  }
  .content {
    flex: 1;
    min-width: 0;
    min-height: 0;
    display: flex;
    flex-direction: column;
  }
  .panel {
    flex: 1;
    min-height: 0;
    overflow: auto;
    &[hidden] { display: none; }
  }
</style>
```

- [ ] **Step 4:** ui-kit test + typecheck → PASS.
- [ ] **Step 5:** Commit: `feat(ui/m11d-1): generic TabbedSurface host (vertical rail, gmOnly filter, collapse)`

---

### Task 6: `@shadowcat/module-sidebar` + core-ui handoff + activeTab persistence

**Files:**
- Create: `src/modules/sidebar/{package.json,svelte.config.js,tsconfig.json,vitest.config.ts,vitest.setup.ts}` (copy from `src/modules/factions/`, name `@shadowcat/module-sidebar`)
- Create: `src/modules/sidebar/src/index.ts`, `src/modules/sidebar/src/SidebarHost.svelte`, `src/modules/sidebar/src/index.test.ts`
- Modify: `src/modules/core-ui/src/index.ts` (provides: `sidebar` → `sidebar-host`), `src/modules/core-ui/src/Layout.svelte` (region hosts `sidebar-host`; drop the region's own `overflow: auto`), `src/modules/core-ui/src/coreUi.test.ts`
- Modify: `src/client/ui-kit/src/appContext.ts` (add `uiState` seam), `src/client/shell/src/lib/sessionState.svelte.ts` (activeTab get/set), `src/client/shell/src/lib/Table.svelte` (wire), `src/client/shell/src/App.svelte` (register module)

**Interfaces:**
- Produces (AppContext): `uiState: { getActiveTab(): string | null; setActiveTab(id: string): void }`.
- Produces (sessionState): `export function getActiveTab(world: string): string | null` /
  `export function setActiveTab(world: string, id: string): void` (mutates
  `state.worlds[world] ??= {}`, sets `.activeTab`, `schedulePersist()`).
- Manifest: id `sidebar`, requires `["shadowcat.surface:sidebar-host"]`, provides
  `[{ contract: "shadowcat.surface:sidebar", cardinality: "multi" }]`; contributes
  `{ id: "sidebar:host", contract: "shadowcat.surface:sidebar-host", component: SidebarHost }`.
- core-ui provides list: REPLACE the `sidebar` entry with
  `{ contract: "shadowcat.surface:sidebar-host", cardinality: "singleton" }` (ownership of the
  multi `sidebar` contract moves to module-sidebar).

`SidebarHost.svelte`:
```svelte
<script lang="ts">
  import { getAppContext, TabbedSurface } from "@shadowcat/ui-kit";
  const ctx = getAppContext();
  // Restore once at mount; persistence is write-through on change.
  let active = $state<string | null>(ctx.uiState.getActiveTab());
  function change(id: string): void {
    active = id;
    ctx.uiState.setActiveTab(id);
  }
</script>

<TabbedSurface contract="shadowcat.surface:sidebar" activeId={active} onTabChange={change} />
```

`Table.svelte` — add to the `setAppContext({...})` literal:
```ts
uiState: {
  getActiveTab: () => getActiveTab(session.world!),
  setActiveTab: (id) => setActiveTab(session.world!, id),
},
```
with `import { getActiveTab, setActiveTab } from "./sessionState.svelte";`

`App.svelte`: `import { sidebar } from "@shadowcat/module-sidebar";` + add `sidebar` to the
modules array (before `coreUi` is fine — topology is order-independent).

- [ ] **Step 1: failing tests.** module-sidebar `index.test.ts` (manifest shape: provides
  `sidebar` multi, requires `sidebar-host`, contributes SidebarHost); core-ui test updated:
  provides contains `sidebar-host` and NOT `sidebar`; shell `sessionState.test.ts` gains
  activeTab get/set round-trip (+ absent world → null) following its existing test idiom.
- [ ] **Step 2:** run those package tests → FAIL.
- [ ] **Step 3:** implement all files above; `pnpm install` to link the new package.
- [ ] **Step 4:** `pnpm -r test` + `pnpm -r typecheck` → PASS (this catches every panel module's
  topology expectations; their `requires: sidebar` now resolves to module-sidebar's provide).
- [ ] **Step 5:** Commit: `feat(ui/m11d-1): sidebar host as a module — tabbed rail + per-world activeTab persistence`

---

### Task 7: Tab metadata on the five existing panels

**Files:**
- Modify: `src/modules/assets/src/index.ts`, `src/modules/actors/src/index.ts`,
  `src/modules/factions/src/index.ts`, `src/modules/conditions/src/index.ts`,
  `src/modules/game-settings/src/index.ts` — each `contribute({...})` gains a `tab`.
- Modify: `src/client/ui-kit/src/locales/en.ts` — keys below.
- Modify: each module's manifest test that asserts contribution shape (read them; extend, don't weaken).

Per the spec's table (exact values):
```ts
// assets:   order 1, tab: { icon: "🖼️", labelKey: "assets.tab" }
// actors:   order 2, tab: { icon: "👥", labelKey: "actors.tab" }
// factions: order 3, tab: { icon: "🚩", labelKey: "factions.tab" }
// conditions: order 4, tab: { icon: "✨", labelKey: "conditions.tab" }
// game-settings: order 5, tab: { icon: "⚙️", labelKey: "gameSettings.tab", gmOnly: true }
```
Locale keys: `"assets.tab": "Assets"`, `"actors.tab": "Actors"`, `"factions.tab": "Factions"`,
`"conditions.tab": "Conditions"`, `"gameSettings.tab": "Game settings"`.

- [ ] Steps: extend each module's index test to assert its `tab` metadata → FAIL → implement →
  `pnpm -r test` + typecheck PASS → Commit:
  `feat(ui/m11d-1): tab metadata for the five sidebar panels (game-settings gmOnly)`

---

### Task 8: Chat transport — WsClient methods + `AppContext.chat`

**Files:**
- Modify: `src/client/core/src/ws-client.ts` (three methods)
- Modify: `src/client/core/src/ws-client.test.ts` (or the file's existing test home — read it first)
- Modify: `src/client/ui-kit/src/appContext.ts` (`chat` seam + `ChatApi` type)
- Modify: `src/client/shell/src/lib/worldSession.svelte.ts` (delegates), `src/client/shell/src/lib/Table.svelte` (wire)

**Interfaces:**
```ts
// ws-client.ts — fire-and-forget senders (these frames carry no correlation id
// by design; server rejections are logged server-side only — the composer
// pre-validates the cheap rejects client-side).
sendChatMessage(opts: { channel: string; content: string; actorOwner?: WireActorOwnerRef | null; audience?: WireAudience }): void {
  this.send({ type: "send_message", channel: opts.channel, content: opts.content,
    actor_owner: opts.actorOwner ?? null, audience: opts.audience ?? { kind: "public" } });
}
editChatMessage(messageId: string, content: string): void {
  this.send({ type: "edit_message", message_id: messageId, content });
}
deleteChatMessage(messageId: string): void {
  this.send({ type: "delete_message", message_id: messageId });
}
// appContext.ts
export interface ChatApi {
  send(opts: { channel: string; content: string; actorOwner?: WireActorOwnerRef | null; audience?: WireAudience }): void;
  edit(messageId: string, content: string): void;
  delete(messageId: string): void;
}
// AppContext gains: chat: ChatApi;
// worldSession delegates (null-safe like sendPing): sendChatMessage/editChatMessage/deleteChatMessage
// Table wires: chat: { send: (o) => session.sendChatMessage(o), edit: (id, c) => session.editChatMessage(id, c), delete: (id) => session.deleteChatMessage(id) },
```

- [ ] **Steps:** failing ws-client test (fake-transport idiom — assert the exact JSON frames for
  all three methods, incl. the `audience` default) → FAIL → implement → core+ui-kit+shell tests
  & typecheck PASS → Commit: `feat(chat/m11d-1): client chat transport + AppContext.chat seam`

---

### Task 9: `@shadowcat/module-chat` — panel host

**Files:**
- Create: `src/modules/chat/{package.json,svelte.config.js,tsconfig.json,vitest.config.ts,vitest.setup.ts}` (factions copies, name `@shadowcat/module-chat`)
- Create: `src/modules/chat/src/index.ts`, `src/modules/chat/src/ChatPanel.svelte`,
  `src/modules/chat/src/channels.ts`, `src/modules/chat/src/{index,ChatPanel,channels}.test.ts`
- Modify: `src/client/ui-kit/src/locales/en.ts` (keys), `src/client/shell/src/App.svelte` (register)

**Interfaces:**
- Manifest: id `chat`, requires `["shadowcat.surface:sidebar"]`, provides
  `[{ contract: "shadowcat.surface:chat.composer", cardinality: "singleton" }, { contract: "shadowcat.surface:chat.message", cardinality: "singleton" }]`;
  contributes `{ id: "chat:sidebar", contract: "shadowcat.surface:sidebar", order: 0, component: ChatPanel, tab: { icon: "💬", labelKey: "chat.tab" } }`.
- `channels.ts` (pure, tested): view model + filters —
```ts
import type { ChatMessageSystem, WireDocument } from "@shadowcat/core";
export type ChatView = { kind: "all" } | { kind: "channel"; id: string } | { kind: "gm" };
/** Post target for a view: All → the default channel; GM → gm_only audience. */
export function postTarget(view: ChatView): { channel: string; audience: import("@shadowcat/core").WireAudience } {
  if (view.kind === "channel") return { channel: view.id, audience: { kind: "public" } };
  if (view.kind === "gm") return { channel: "general", audience: { kind: "gm_only" } };
  return { channel: "general", audience: { kind: "public" } };
}
export function inView(view: ChatView, sys: ChatMessageSystem): boolean {
  if (view.kind === "all") return true;
  if (view.kind === "gm") return sys.audience.kind === "gm_only";
  return sys.channel === view.id;
}
/** Sort by envelope created_at then id (server-set; stable under edits). */
export function byCreation(a: WireDocument, b: WireDocument): number {
  return a.created_at - b.created_at || (a.id < b.id ? -1 : a.id > b.id ? 1 : 0);
}
export const RENDER_CAP = 200;
```
- `ChatPanel.svelte` responsibilities (complete component; key structure):
  - Reactive `query("message")` via the standard `createSubscriber` bridge; parse each with
    `parseMessageSystem` (null → skipped); filter `inView`; sort `byCreation`; slice the LAST
    `RENDER_CAP`.
  - Channel strip: All + registry channels (`query("channel-registry")[0]`, `ChannelRegistrySystem`)
    + GM; active view is local `$state<ChatView>({ kind: "all" })`.
  - GM registry seed (copy the FactionsPanel idiom EXACTLY — reactive `subscribe()` inside the
    `$effect`, `seeded` latch, [[contribution-seed-reactive-before-resync]]):
    `buildChannelRegistryDoc(ctx.world, { general: { name: "General" } })`.
  - GM channel editor behind a "⚙" toggle in the strip: add (random uuid key, name input),
    rename, remove — single-key `dispatchIntent` updates on `/system/channels/<id>` (FactionsPanel
    idiom).
  - Card + composer instantiation: read the singleton contributions directly (the
    `Surface.svelte` subscribe/snapshot idiom — NOT `<Surface>`, which cannot pass reactive
    instance props):
    ```ts
    const cardComp = $derived.by(() => { subscribe(); return ctx.contributions.contributionsFor("shadowcat.surface:chat.message")[0]?.component as Component<{ message: WireDocument; showChannel: boolean }> | undefined; });
    const composerComp = $derived.by(() => { subscribe(); return ctx.contributions.contributionsFor("shadowcat.surface:chat.composer")[0]?.component as Component<{ channel: string; audience: WireAudience; placeholderName: string }> | undefined; });
    ```
    Render `{#each visibleDocs as m (m.id)}<Card message={m} showChannel={view.kind === "all"} />{/each}`
    and `<Composer {...postTarget(view)} placeholderName={...} />` pinned at the bottom.
  - Scroll: container `bind:this`; track `atBottom` on scroll (`scrollTop + clientHeight >=
    scrollHeight - 4`); `$effect` on message count → if `atBottom` scroll to bottom, else show
    the "new messages ↓" pill (click = scroll to bottom + clear).
- Locale keys: `"chat.tab": "Chat"`, `"chat.all": "All"`, `"chat.gmChannel": "GM"`,
  `"chat.channels.edit": "Edit channels"`, `"chat.channels.add": "Add channel"`,
  `"chat.channels.name": "Channel name"`, `"chat.channels.remove": "Remove"`,
  `"chat.newMessages": "New messages ↓"`.

- [ ] **Step 1: failing tests** — `channels.test.ts` (postTarget/inView/byCreation exhaustive:
  all three views × public/whisper/gm_only, channel match/mismatch, sort ties); `index.test.ts`
  (manifest: provides both chat surfaces, requires sidebar, tab metadata `order: 0` +
  `icon: "💬"`); `ChatPanel.test.ts` (renders messages from a seeded store fixture through a
  stub card contribution; GM sees the ⚙ editor, player does not; seed creates the registry doc
  once for GM).
- [ ] **Step 2:** FAIL → **Step 3:** implement → **Step 4:** `pnpm -r test` + typecheck PASS.
- [ ] **Step 5:** Commit: `feat(chat/m11d-1): chat panel host module (channels, list, seed, surfaces)`

---

### Task 10: `@shadowcat/module-chat-composer`

**Files:**
- Create: `src/modules/chat-composer/**` (factions copies, name `@shadowcat/module-chat-composer`)
- Create: `src/modules/chat-composer/src/{index.ts,Composer.svelte,index.test.ts,Composer.test.ts}`
- Modify: locales (`"chat.composer.placeholder": "Message #{name}"`,
  `"chat.composer.placeholderGm": "Message the GM"`, `"chat.composer.send": "Send"`,
  `"chat.composer.count": "{used} / {max}"`), `App.svelte` (register)

**Interfaces:**
- Manifest: id `chat-composer`, requires `["shadowcat.surface:chat.composer"]`, provides `[]`;
  contributes `{ id: "chat-composer:main", contract: "shadowcat.surface:chat.composer", component: Composer }`.
- Props (from the host): `{ channel: string; audience: WireAudience; placeholderName: string }`.
- Behavior: textarea; Enter sends / Shift+Enter newline; trims; blocks empty and
  `> MAX_MESSAGE_CHARS` (core constant); shows the counter when `used > MAX_MESSAGE_CHARS - 200`;
  sends via `ctx.chat.send({ channel, content, audience })`; clears on send; `/`-commands ride
  the content verbatim (server parses — NO client parsing); `actor_owner` not sent (v1).

- [ ] **Steps:** failing Composer tests (Enter sends trimmed content with the given
  channel/audience and clears; Shift+Enter doesn't send; empty/whitespace blocked; over-limit
  blocked + counter visible; GM audience prop passes through verbatim) → FAIL → implement →
  PASS → Commit: `feat(chat/m11d-1): default composer module`

---

### Task 11: `@shadowcat/module-chat-card`

**Files:**
- Create: `src/modules/chat-card/**` (factions copies, name `@shadowcat/module-chat-card`)
- Create: `src/modules/chat-card/src/{index.ts,MessageCard.svelte,index.test.ts,MessageCard.test.ts}`
- Modify: locales (`"chat.edited": "(edited)"`, `"chat.deleted": "Message deleted"`,
  `"chat.whisperTo": "to {names}"`, `"chat.gmBadge": "GM"`, `"chat.edit": "Edit"`,
  `"chat.delete": "Delete"`, `"chat.save": "Save"`, `"chat.cancel": "Cancel"`,
  `"chat.deleteConfirm": "Delete this message?"`, `"chat.rollPending": "🎲 {formula}"`),
  `App.svelte` (register)

**Interfaces:**
- Manifest: id `chat-card`, requires `["shadowcat.surface:chat.message"]`, provides `[]`;
  contributes `{ id: "chat-card:main", contract: "shadowcat.surface:chat.message", component: MessageCard }`.
- Props: `{ message: WireDocument; showChannel: boolean }`.

**Component contract (implement exactly):**
- `const sys = $derived(parseMessageSystem(message))` — `null` ⇒ render NOTHING (fail-closed).
- Header: author = `ctx.members.get(sys.user_owner) ?? sys.user_owner.slice(0, 8)`; actor name
  (when `sys.actor_owner` present) via `resolveTokenActor`/`actorDisplayName` from
  `@shadowcat/core` (fail-closed fallback + OwnerOrGm redaction already inside — render as
  emphasized text, NO sheet link v1); time `new Date(message.created_at)` as HH:MM with the full
  locale string in `title`; channel chip when `showChannel`; `(edited)` when `sys.edited_at`;
  whisper badge `to {names}` (recipients → members map, fallback short id); GM badge when
  `sys.audience.kind === "gm_only"`.
- Body:
  - `sys.deleted_at` ⇒ muted tombstone `chat.deleted`, nothing else.
  - `kind === "emote"` ⇒ italic run-in: author name + segments inline.
  - `kind === "roll"` ⇒ `chat.rollPending` with the concatenated text of the segments
    (monospace shell; M11d-2 replaces with real embeds).
  - Segments: `{#each sys.content.filter(isKnownSegment) as s}` — `text` ⇒
    `<span class="seg-text">{s.text}</span>` (CSS `white-space: pre-wrap`; NEVER `{@html}`);
    `html` ⇒ `<span class="seg-html">{@html s.sanitized_html}</span>` with this comment above
    it: `<!-- INVARIANT: sanitized_html is ammonia-cleaned by the server's chat::sanitize —
    the ONLY string this app may ever pass to {@html}. -->`. Unknown kinds are filtered out.
  - `.seg-html :global(img) { max-width: 100%; display: block; }` (spec §5: images sized to
    the card on their own line).
- Actions (hover/focus-visible row): shown when `sys.user_owner === ctx.selfId || ctx.role ===
  "gm"`, and not deleted. Edit ⇒ inline textarea prefilled `sys.source ?? textOf(sys.content)`
  (where `textOf` concatenates `text` segments only), Save ⇒ `ctx.chat.edit(message.id, draft)`
  + exit edit mode (the broadcast updates the doc), Cancel reverts; Delete ⇒
  `window.confirm(t("chat.deleteConfirm"))` then `ctx.chat.delete(message.id)`.

- [ ] **Step 1: failing tests** — each rendering behavior above as its own test with fixture
  docs (public text, html segment via `{@html}` asserted by innerHTML, emote, roll, whisper
  badge + name resolution, gm badge, edited marker, deleted tombstone suppresses body+actions,
  unknown segment filtered, malformed body renders nothing, actions visible for owner and GM
  but not for another player, edit prefill prefers `source`, delete confirm wiring). Include
  the redaction fixtures (parent §8's client half): a hidden-actor-name doc renders the
  fallback name (build fixtures through `resolveTokenActor`'s real inputs — read
  `src/client/core/src/actor.ts` first); dangling `TokenInstance` fails closed.
- [ ] **Step 2:** FAIL → **Step 3:** implement → **Step 4:** `pnpm -r test` + typecheck PASS.
- [ ] **Step 5:** Commit: `feat(chat/m11d-1): default message card module (fail-closed render, edit/delete)`

> **Task 11 review brief MUST include the `{@html}` boundary directive (see Buddy-check
> directives).**

---

### Task 12: Whole-feature integration pass

**Files:**
- Modify: anything the pass surfaces; expected: none beyond small fixes.

- [ ] **Step 1:** `pnpm install && pnpm -r test && pnpm -r typecheck && pnpm lint` → all green.
- [ ] **Step 2:** `pnpm build` then from `src/server/`: `cargo test && cargo fmt --check &&
  cargo clippy --all-targets -- -D warnings` (modulo the tracked pre-existing lint) → green.
- [ ] **Step 3:** Manual smoke via the dev flow (see the project `run`/verify skill): boot the
  binary, two browsers (GM + player): GM sees 6 tabs / player 5 (no ⚙ game-settings); chat
  default; send public message both ways; `/me`; `/w @<player>` visible only to recipient; GM
  pseudo-channel (player posts, GM reads, other player blind); edit own message on an
  html-enabled world (prefill = source); delete → tombstone both sides; `:smile:` renders 😄;
  channel add + post to it; activeTab survives reload; collapse/expand.
- [ ] **Step 4:** Commit any fixes: `fix(chat/m11d-1): integration findings`

---

### Task 13: Docs + reviewed skill-update gate + final review

- [ ] `docs/PLAN.md`: M11d-1 completion entry (follow the M11c entry style; factual, verified).
- [ ] `docs/TODO.md`: log the spec §5 deferrals (virtualization beyond the 200 cap; unread
  badges/notifications; tab popouts; actor-name→sheet + internal doc links pending M12 sheets;
  speaking-as-actor composer; shortcodes-inside-code-spans refinement; collapse persistence).
- [ ] Update `shadowcat-codebase-chat` (source field + shortcode pre-pass + client mirror
  `chat-docs.ts` + the three modules + members widening) and `shadowcat-codebase-client-shell`
  (sidebar-host contract + module-sidebar + TabbedSurface + tab metadata + `chat`/`uiState`
  AppContext seams). Dispatch `shadowcat-spec-reviewer` (effort high) on the skill diffs —
  the reviewed skill-update gate.
- [ ] Whole-branch final review: `shadowcat-spec-reviewer` + `shadowcat-code-reviewer` (opus
  twins if findings shallow) over the full branch diff; fix + re-verify findings.
- [ ] Merge `--no-ff` to local main once green. **NO push.**

## Self-review (done at write time)

- Spec coverage: §1.2→T1, §1.3→T5+T6, §1.4→T6, §1.5→T7, §2.1→T9, §2.2→T10, §2.3→T11, §2.4→T1+T8,
  §3.1→T2, §3.2→T3, §3.3→T4, §4→per-task tests + T12, §6→T13. No gaps.
- No placeholders: every step names real files/symbols with code; T2's test sketch is explicitly
  marked "write as real tests against the file's existing scaffolding" with behaviors pinned.
- Type consistency: `ContributionTab`/`tab`, `ChatApi`, `uiState`, `postTarget/inView/byCreation`,
  `parseMessageSystem/isKnownSegment`, `MAX_MESSAGE_CHARS` used consistently across tasks.
