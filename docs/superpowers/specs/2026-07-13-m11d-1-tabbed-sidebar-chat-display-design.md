# M11d-1 · Tabbed Sidebar + Chat Display Modules — Checkpoint Design

> Parent: `2026-07-03-m11-chat-system-design.md` §5 (default display modules). Chat core
> (M11c-1/2/3) is complete and merged: server-authoritative ingest, restricted audiences,
> sanitizer/commands/edit/delete. This checkpoint builds the **client display layer** plus the
> **tabbed sidebar** it mounts into, and three small server enablers. Roll embeds land in
> M11d-2; link previews in M11d-3.

## 0. Decomposition context (locked)

M11d is decomposed into three checkpoints, each its own spec→plan→execute cycle:

- **M11d-1 (this):** tabbed sidebar + chat panel host + composer + message card + server enablers.
- **M11d-2:** dice→chat wire integration (ts-rs dice bindings, wire-boundary validation TODOs,
  `/roll` execution at ingest, roll embeds on the card, entropy-seeded RNG at the transport).
- **M11d-3:** SSRF-guarded link-preview fetcher (the former M11c-4, folded into the M11d cycle) +
  preview-card rendering.

User decisions this session: 3-checkpoint slicing; the tabbed sidebar folds INTO M11d-1 (not a
separate M11d-0); Foundry-VTT-style vertical tab rail researched as the reference pattern.

## 1. Tabbed sidebar

### 1.1 Reference research (Foundry V14, decided on our merits)

Foundry V14's `Sidebar` (read from the real source, `client/applications/sidebar/sidebar.mjs`)
is a host with a static tab registry — per-tab `icon` + `tooltip` + optional `gmOnly` — rendered
as a **vertical icon rail on the sidebar's outer edge** plus a content host; every tab's app
stays alive while an `active-<tab>` class switches visibility; chat is the default tab; the
sidebar collapses to rail-only and clicking a tab re-expands it.

We adopt: vertical icon rail, all-tabs-stay-mounted visibility switching, per-tab gmOnly
filtering, chat as default, collapse-to-rail. We do NOT adopt: Font Awesome (no new dep — text
glyphs), right-click popouts (deferred), per-tab sub-applications (our contributions already are
independent components).

**Merit justification** (not "because Foundry"): all-mounted + CSS visibility preserves panel
state (form drafts, chat scroll) and keeps the GM seed `$effect`s of factions/conditions/
game-settings running regardless of which tab is active — with mount-on-activate, a GM who never
opens those tabs would never seed their registries. The M7-deferred "tabbed sidebar when there
are multiple sidebar panels" debt (5 panels already stack unscrolled today) is exactly due.

### 1.2 Contribution tab metadata (core)

`Contribution` (`src/client/core/src/contributions.ts`) gains one optional field — plain data,
framework-neutral:

```ts
tab?: {
  /** Rail glyph (emoji/unicode text). */
  icon: string;
  /** i18n key resolved by the host at render (locale-reactive). */
  labelKey: string;
  /** Hidden from non-GM users. */
  gmOnly?: boolean;
}
```

No breaking change: contributions without `tab` render with fallback metadata (icon = first
character of `id`, label = raw `id`). The surface contract id and `multi` cardinality are
unchanged — this checkpoint changes how core-ui *hosts* `shadowcat.surface:sidebar`, not the
contract model.

### 1.3 The sidebar host is itself a module: `@shadowcat/module-sidebar`

**UI-is-modules applies to the sidebar's own presentation** (user directive this session): the
tab host is a replaceable module, symmetric with `module-topbar`/`module-statusbar` — swapping
the sidebar's presentation (tabs → accordion → plain stack) replaces one small module, never
core-ui or any panel.

- `core-ui` declares a new **singleton** surface `shadowcat.surface:sidebar-host` and its
  `Layout.svelte` sidebar region renders `<Surface contract="shadowcat.surface:sidebar-host">`
  (it stops hosting `shadowcat.surface:sidebar` directly).
- New package `@shadowcat/module-sidebar` (requires `sidebar-host`, **provides** the existing
  multi contract `shadowcat.surface:sidebar` — ownership of the panel contract moves to it):
  contributes the tabbed host component, owns activeTab state + persistence wiring (§1.4) and
  the collapse toggle.
- The rendering primitive `TabbedSurface.svelte` lives in **ui-kit** as a generic reusable
  sibling of `Surface.svelte` (kit = shared primitives; module = the replaceable decision to
  use it for the sidebar):

- Props: `contract: string`, `activeId?: string | null`, `onTabChange?: (id: string) => void`.
- Reads contributions via the same `createSubscriber` bridge as `Surface.svelte`; filters out
  `tab.gmOnly` entries when `ctx.role !== "gm"`; sorts by `order`.
- Renders a **vertical icon rail** (each button: glyph, `title`/`aria-label` = `t(labelKey)`,
  `aria-pressed`, min 44×44px touch target) + a content area where **every** contribution is
  mounted once and inactive ones are hidden via CSS (`display: none` wrapper — state preserved).
- Active resolution: `activeId` prop if it matches a visible contribution, else the first
  visible contribution. Clicking a tab calls `onTabChange` and, when collapsed, re-expands.
- **Collapse:** a caret toggle at the rail's top collapses the content area to rail-only width.
  Collapse state is component-local (NOT persisted) in v1.
### 1.4 activeTab persistence

- `module-sidebar`'s host component renders `<TabbedSurface
  contract="shadowcat.surface:sidebar" activeId={ctx.uiState.getActiveTab()}
  onTabChange={ctx.uiState.setActiveTab} />`.
- New narrow AppContext seam: `uiState: { getActiveTab(): string | null; setActiveTab(id:
  string): void }`. The shell (`Table.svelte`/`WorldSession`) wires it to `sessionState`, which
  persists to the **already-reserved** `UiState.worlds[worldId].activeTab` field (M7 built the
  field and the debounced PUT path; this checkpoint finally consumes it). Session restore returns
  you to your last active tab per world.
- Sidebar CSS: the region stops being a plain `overflow: auto` stack; the tab host owns internal
  scrolling per panel. Phone media query keeps the sidebar stacked below the stage; the rail
  stays vertical (44px targets) — a dedicated mobile pass is out of scope, targets stay legal.

### 1.5 Migrating the five existing panels

Each existing sidebar contribution adds `tab` metadata (one-line change per module) and keeps
its component unchanged:

| Module | order | icon | labelKey | gmOnly |
|---|---|---|---|---|
| chat (new) | 0 | 💬 | `chat.tab` | — |
| assets | 1 | 🖼️ | `assets.tab` | — |
| actors | 2 | 👥 | `actors.tab` | — |
| factions | 3 | 🚩 | `factions.tab` | — |
| conditions | 4 | ✨ | `conditions.tab` | — |
| game-settings | 5 | ⚙️ | `gameSettings.tab` | ✓ |

Chat is `order: 0` ⇒ the default active tab for a fresh world (Foundry parity, and chat is the
one panel every role uses). `game-settings` gains `gmOnly` (its panel already self-gates
internally; the tab filter removes the dead tab for players — defense in depth retained).

## 2. Chat display modules

Three independently replaceable packages per parent §5 (M8.5c per-element precedent). A system
builder swaps the composer, the card, or both without touching the host.

### 2.1 `@shadowcat/module-chat` — panel host

- Contributes the sidebar tab (`ChatPanel`); **declares** two new surfaces:
  `shadowcat.surface:chat.composer` (singleton) and `shadowcat.surface:chat.message` (singleton —
  a *renderer contract*). For both, the host reads the contributed component from the registry
  directly (the `Surface.svelte` subscribe/snapshot idiom) instead of mounting `<Surface>`,
  because `Contribution.props` are static at contribute-time and the host must pass **reactive
  instance props** — the current post target to the composer, and per-message props to the card
  (instantiated once per rendered message). Replaceability is unchanged: swapping the
  contribution swaps the component.
- **Channel model** (parent §2: module-seeded config docs, implicit "All"): a world-scoped
  singleton `channel-registry` config document (id→`{name}` map — the factions/conditions
  registry shape; adds/renames/removes are single-key field Updates). The host seeds it (GM,
  idempotent, reactive-seed pattern per `FactionsPanel` /
  [[contribution-seed-reactive-before-resync]]) with one default: `general` → `{name:
  "General"}`. A GM-only inline editor (add/rename/remove) lives at the bottom of the channel
  selector, matching the factions editor idiom.
- **Channel selector** (horizontal strip above the list): `[All] [General] [...registry] [GM]`.
  - **All** = every readable message regardless of channel (the implicit fallback view; also
    covers messages whose channel no longer exists in the registry).
  - A named channel filters `system.channel === id`.
  - **GM** is a *pseudo-channel* mapped to `Audience::GmOnly` (the server has no reserved
    channel concept — the skill's "posting to a 'GM' channel is what sets `audience: GmOnly`").
    Its view filters `system.audience.type === "gm_only"`; every role sees the tab (any member
    may post reports to the GM; the server already restricts *readers*).
- **Message list:** `ctx.documents.query("message")` (reactive via the standard
  `createSubscriber` bridge), filtered by the active view, sorted by `created_at` then `id`
  (server-set envelope timestamps; edits touch only `updated_at`, so ordering is stable). Render
  cap: the most recent 200 per view (older messages exist via resync but are not rendered;
  virtualization is a logged deferral). Whispers render inline in whatever channel they were
  sent to, badge-styled (only recipients ever receive them — server-filtered).
- **Scroll behavior:** stick-to-bottom while the user is at the bottom; when scrolled up, new
  arrivals do NOT yank the view — a "new messages ↓" jump pill appears.
- The composer mounts pinned below the list via the `chat.composer` surface; the host passes the
  computed **post target** (`channel`, `audience`) as contribution props: on *All* it posts to
  `general` (placeholder shows "#General"), on a named channel to that channel, on *GM* to
  channel `general` + `audience: GmOnly`.

### 2.2 `@shadowcat/module-chat-composer`

- Single-line-growing textarea; Enter sends, Shift+Enter inserts a newline.
- Client-side validation mirrors the server's cheap rejects (the frames carry no `intent_id`, so
  server rejections are silent — the composer must pre-validate): non-empty after trim, ≤4096
  chars, with a live counter near the limit. Flood-limit rejections remain silent v1 (logged
  deferral: a reason channel needs protocol work).
- Sends raw content via `ctx.chat.send({ channel, content, audience })` — commands (`/me`,
  `/roll`, `/w @user`) stay in the content string; the **server** parses them (M11c-3). No
  client command parsing.
- `actor_owner`: v1 sends none (`null`). Speaking-as-actor UX is deferred to M11d-2 alongside
  roll attribution (logged) — the wire field, storage, and card rendering all already support it.

### 2.3 `@shadowcat/module-chat-card`

Renders one message document (props: `message: WireDocument`, `showChannel: boolean`).

- **Fail-closed parse:** `parseMessageSystem(doc)` (new, core) Zod-validates the opaque body; a
  malformed body renders nothing (dev-logged) — never a partial best-effort render.
- **Header:** author username (`ctx.members`), actor name *when present and viewer-visible* via
  the existing fail-closed `resolveTokenActor`/`actorDisplayName` chokepoint (OwnerOrGm privacy
  and dangling-ref fallback already handled there); timestamp (HH:MM, full date on hover);
  channel chip when `showChannel` (All view); `(edited)` marker when `edited_at` is set;
  audience badge — whisper: "→ @name, @name" (uuids resolved via `ctx.members`); GM-only: a "GM"
  badge.
- **Body:** renders `content: Segment[]` — `Text` as a text node (newlines → line breaks; NEVER
  innerHTML), `Html` via `{@html}` (**safe by construction ONLY because `chat::sanitize`
  produced it** — the invariant is documented at the render site; no other Html source may ever
  flow here). `kind: emote` renders italic with the author name prefixed run-in (parent §5's
  emote treatment); `kind: roll` renders the verbatim body in a monospace "pending roll" shell
  (M11d-2 replaces this with real roll embeds); `kind: system` reserved-styled (no producer
  exists yet).
- **Deleted:** `deleted_at` set ⇒ a muted tombstone row ("message deleted"); no content, no
  actions except nothing (content and source are cleared server-side).
- **Edit/delete affordances:** shown for own messages (`user_owner === ctx.selfId`) and for GM
  on any message (moderation authority is audience-independent — M11c-3). Edit opens an inline
  textarea prefilled from the new `source` field (§3.1) and submits via `ctx.chat.edit(id,
  content)`; delete confirms then `ctx.chat.delete(id)`.
- **Deferred to sheet infra (M12), logged:** actor-name→sheet navigation and internal doc-link
  buttons (parent §5 bullets) — no sheet system exists to open yet; names render as emphasized
  text v1.

### 2.4 Core client plumbing

- **`src/client/core/src/chat-docs.ts`** (new; `scene-docs.ts` idiom): client-declared Zod
  mirrors of the serde-only body types — `MessageSystemSchema` (`channel`, `user_owner`,
  `actor_owner?`, `kind`, `audience`, `content: Segment[]`, `source?`, `edited_at?`,
  `deleted_at?`), `SegmentSchema` (tagged `text`/`html`), `MessageKindSchema`,
  `AudienceSchema`; `parseMessageSystem(doc): MessageSystem | null` (fail-closed);
  `ChannelRegistrySystem` + `buildChannelRegistryDoc` (permissions mirroring the
  faction-registry builder). Per the chat skill: these mirrors are manually kept in sync with
  `chat/mod.rs` — a Rust body-shape change must update this file (drift note in both files).
- **`WsClient`**: `sendChatMessage({channel, content, actorOwner, audience})`,
  `editChatMessage(messageId, content)`, `deleteChatMessage(messageId)` — thin fire-and-forget
  senders over the existing generated `ClientMsg` variants (already in `SendMessageSchema`'s
  family; no correlation ids exist on these frames by design).
- **`AppContext.chat`**: grouped seam `{ send, edit, delete }` (the `sceneInteraction` grouping
  precedent), wired through `WorldSession` → `Table.svelte`. Plus the `uiState` seam (§1.4).
- **i18n:** new keys in `ui-kit/src/locales/en.ts` (tab labels, chat panel strings, composer
  placeholder/counter, card markers, editor strings).

## 3. Server enablers (three contained changes, each with tests)

### 3.1 `MessageSystem.source`

`source: Option<String>` (`#[serde(default, skip_serializing_if = "Option::is_none")]`) — the
raw message body (post-command-strip, pre-sanitize) stored at ingest and replaced on edit.
**Cleared (set to `None`) by the delete tombstone alongside `content`** — leaving it would leak
deleted content through the envelope. Purpose: edit prefill (sanitized `Segment::Html` cannot be
reversed to author input; without this, editing is broken on any markdown/html-enabled world).
It is data, never rendered as markup (the card puts it in a textarea value only). Readership
equals the message's readership (same doc body — no new redaction surface).

### 3.2 Emoji shortcodes in the sanitize pipeline

`:shortcode:` → unicode replacement runs server-side at the TOP of `sanitize()` (and thus
identically on edit re-runs), before any markdown/html processing, always-on (no policy toggle —
typing sugar, not enrichment; output is plain unicode text with no security surface). A curated
`&'static` map (~100 common: `:smile:`, `:heart:`, `:d20:`-class table glyphs, …), longest-match
on the `:name:` token grammar `[a-z0-9_+-]+`. Server-side so the *stored* content is final and
consistent for every viewer and every policy. v1 limitation (documented): replacement is
pre-parse, so shortcodes inside markdown code spans are also replaced — refinement logged, not
blocking.

### 3.3 Members endpoint widened to members

`GET /api/worlds/:id/members` gate changes `require_gm` → member-of-world (non-members keep the
existing 404-to-non-members behavior). Rationale: the card must resolve `user_owner` and whisper
recipient uuids to usernames for **every** viewer; a table's member roster (username + role) is
inherently visible in shared play — the GM-only gate predates any player-facing need, and a
parallel names-only endpoint would be a near-duplicate. Client: `WorldSession` fetches members
on *every* Welcome (not just GM); the `AppContext.members` doc comment updates accordingly.

## 4. Testing strategy

- **Server (cargo):** `source` lifecycle — set at ingest (post-command-strip: `/me x` stores
  `x`), replaced on edit, **cleared on delete**; shortcode replacement unit tests (match,
  longest-match, non-match passthrough, inside-markdown interaction pinned) + pipeline
  integration (policy off ⇒ `Text` segment already-replaced); members-gate tests (member 200,
  non-member 404, GM 200 unchanged). Existing chat suites stay green (the `source` addition is
  `#[serde(default)]`-compatible with stored c-1..c-3 messages).
- **Client (vitest + typecheck):** TabbedSurface (renders rail from contributions, gmOnly
  filtering by role, fallback metadata, active switching calls back, all-mounted/hidden
  semantics, collapse); activeTab persistence wiring (sessionState round-trip);
  chat-docs parse (valid bodies, each `Audience`/`kind`, fail-closed on malformed); host
  (channel filtering incl. All + GM pseudo-channel + unknown-channel messages, ordering, render
  cap, seed idempotence); composer (validation, Enter/Shift+Enter, target props honored);
  card (each kind, Text vs Html rendering — the `{@html}` boundary pinned, deleted tombstone,
  edited marker, whisper/GM badges with name resolution, own/GM affordance visibility,
  fail-closed body); module manifests (module-sidebar provides `sidebar` / requires
  `sidebar-host`; chat surfaces declared; tab metadata present on all six panels).
- **Cross-cutting (parent §8's client half):** card component tests with server-shaped redacted
  fixtures — a whisper never reaches a non-recipient's store (server-filtered; fixture asserts
  absence), a hidden actor name renders the fallback, both `Actor` and `TokenInstance` owner
  refs plus dangling-ref fail-closed.

## 5. Explicitly out of scope (logged where noted)

- Roll execution/embeds (M11d-2); link previews (M11d-3).
- List virtualization beyond the 200-render cap (TODO.md).
- Unread badges / notification pips on the chat tab; sounds (TODO.md).
- Tab popout windows (Foundry parity item; TODO.md if ever wanted).
- Actor-name→sheet + internal doc-link navigation (needs M12 sheet infra; TODO.md).
- Speaking-as-actor composer UX (`actor_owner` picker) — M11d-2 with roll attribution.
- `SendMessage`-family failure surfacing (needs correlation ids; existing logged deferral).
- Collapse-state persistence; mobile-dedicated sidebar pass.

## 6. Codebase-skill gate

`shadowcat-codebase-chat` (client model mirror, new chat modules, `source`/shortcode pipeline
changes) and `shadowcat-codebase-client-shell` (module-sidebar + `sidebar-host` contract,
TabbedSurface, tab metadata, `uiState`/`chat` AppContext seams) both update before merge,
reviewed per the standing gate.
