# M11c-2 Restricted-Audience Messaging (Whisper + GM Channel) — Design

**Status:** Approved design (brainstorm complete), pre-plan.
**Parent spec:** `2026-07-08-m11c-chat-core-design.md` §3 "M11c-2 · Whisper recipient allowlist" —
this document supersedes that section's implementation sketch with a codebase-grounded design and
widens its scope to also cover a GM-only channel (see §0).
**Depends on:** M11c-1 (message model + server-authoritative ingest — DONE, branch
`m11c-1-message-model`).

## 0. Scope change from the parent spec

The parent spec's M11c-2 sketch proposed a bespoke `recipients` read directly inside
`resolve_access`/`can_see`. Building c-1 first revealed an existing, already-tested whole-document
fail-closed suppression mechanism (`PermissionSet.default: DocRole::None` + per-user grants,
proven by the M10g secret-region precedent — a `default: DocRole::None` document's `Create` op is
dropped *entirely* for a non-permitted recipient, across every one of `filter_command`'s branches,
`search`'s per-hit filter, and `query_documents`/`get_document`'s HTTP reads). This checkpoint's
design reuses that mechanism instead of inventing a parallel one.

While brainstorming, a second requirement surfaced: **a GM-only channel that any regular world
member can post into, but only GM(s) can read.** This is the mirror image of a whisper (a fixed
restricted audience rather than sender-picked recipients) and shares the same underlying
permission primitive, so it is folded into this checkpoint rather than deferred. The checkpoint is
retitled **"Restricted-audience messaging"** to cover both.

Two explicit product decisions, made during brainstorming, drive the design:
1. **A whisper hides from the GM by default.** The GM sees a whisper only if their own user id is
   among its `recipients` — symmetric with how any other non-participant is treated. (The parent
   spec's "(per world policy, default-on) the GM" language is superseded by this.)
2. **The GM-only channel is dynamically resolved**, not a frozen snapshot: whoever holds
   `WorldRole::Gm` at read time sees it, including a co-GM promoted after messages were sent, and
   excluding one demoted since.

## 1. The core primitive: `PermissionSet.gm_role: Option<DocRole>`

Today, `resolve_access` gives every `WorldRole::Gm` user unconditional `all: true` access to every
document, before any document-level permission is even consulted:

```rust
pub fn resolve_access(user: Uuid, world_role: WorldRole, doc: &Document) -> Access {
    if world_role == WorldRole::Gm {
        return Access { caps: BTreeSet::new(), all: true, see_gm_only: true, is_owner: true };
    }
    // ... per-document DocRole floor resolution for everyone else ...
}
```

This is correct and load-bearing for every existing document type (actors, scenes, secret
regions — the GM must always see a secret region even though it's `default: DocRole::None`). A
whisper or GM-only message needs the *opposite*: GM access must be gated by the same per-document
role floor everyone else uses.

**New field:** `PermissionSet.gm_role: Option<DocRole>` (`#[serde(default)]`, ts-rs exported since
`PermissionSet` already is). `None` is the default for every document — including every document
that predates this change (deserializes via `serde(default)`) — and preserves today's behavior
exactly. Only `build_message_doc`'s whisper/GM-only branches ever set it to `Some(_)`.

**`resolve_access` becomes:**

```rust
pub fn resolve_access(user: Uuid, world_role: WorldRole, doc: &Document) -> Access {
    if world_role == WorldRole::Gm {
        match doc.permissions.gm_role {
            None => return Access { caps: BTreeSet::new(), all: true, see_gm_only: true, is_owner: true },
            Some(fallback_role) => {
                // Fall through to the same floor-resolution the non-GM branch uses below,
                // seeded with `fallback_role` instead of `doc.permissions.default` when this
                // GM isn't individually listed in `permissions.users`. Extract the shared
                // floor-computation (role_floor + capabilities.by_role/by_user) into a small
                // helper so this branch and the existing non-GM branch don't duplicate it.
            }
        }
    }
    // ... existing non-GM floor resolution, now also the fall-through target above ...
}
```

`see_gm_only` stays `true` for a GM even in the `Some(_)` branch (they remain the GM for
property-tier — `GmOnly`/`OwnerOrGm` — visibility purposes; messages don't use property overrides,
so this is inert in practice but keeps the field's meaning consistent). `is_owner` is computed the
same way as for any user (`doc.owner == Some(user)`).

Because `resolve_access`/`resolve_access_world` is the **single chokepoint** every egress path
already calls — `filter_command`'s `Create`/`Update`/`Delete` branches (broadcast), `search`'s
per-hit `access.has(cap::READ)` filter (search, independent of the FTS column split), and
`query_documents`/`get_document` (HTTP resync/load) — this one field change is automatically
honored on every path. **No other file changes.** This preserves the exact "one chokepoint"
property `shadowcat-codebase-documents-permissions` already documents for the `OwnerOrGm` tier.

## 2. Message audience model

New wire enum, ts-rs exported alongside `ActorOwnerRef` (same `#[serde(tag = "kind", rename_all =
"snake_case")]` pattern), added to the `SendMessage` frame and stored in `MessageSystem`:

```rust
#[derive(Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/")]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Audience {
    Public,
    Whisper { recipients: Vec<Uuid> },
    GmOnly,
}
```

`SendMessage` gains `audience: Audience`. `MessageSystem` gains `audience: Audience` (stored
verbatim in the opaque body, same as `kind`/`actor_owner` today).

`build_message_doc` maps `audience` to the document's `PermissionSet`:

| Audience | `default` | `gm_role` | `users` |
|---|---|---|---|
| `Public` | `Observer` | `None` | `{owner: Owner}` — **c-1's existing shape, unchanged** |
| `Whisper{recipients}` | `None` | `Some(DocRole::None)` | `{owner: Owner, ...recipients: Observer}` |
| `GmOnly` | `None` | `Some(DocRole::Observer)` | `{owner: Owner}` |

Reading the table against §1: a `Whisper` GM only gets `Observer` if their own uuid is in
`recipients` (present in `users`, which the floor-resolution fallback checks *before* falling back
to `gm_role`). A `GmOnly` message needs no `users` entry for any GM at all — `gm_role =
Some(Observer)` grants it to **any** `WorldRole::Gm` user, resolved fresh every time
`resolve_access` runs (every broadcast recipient loop iteration, every search, every page load) —
this is what makes GM-channel visibility dynamic rather than a frozen roster. The `channel` string
field is untouched and still never server-validated (c-1's existing boundary) — M11d's GM-channel
UI module is what chooses to set `audience: GmOnly` when posting to its "GM" channel; the server
has no concept of a reserved channel name. Posting rights are unchanged from c-1's baseline (any
world member may `SendMessage`) — `audience` only restricts *readers*, never senders.

## 3. Validation & edge cases

- **Recipients must resolve to real world members.** `handle_send_message` validates every
  `Whisper` recipient uuid against current world membership before constructing the document; any
  unknown/foreign uuid rejects the *entire* send (fail-closed — this is the parent spec's mandated
  "fail-closed on a malformed `recipients`" test).
- **`Whisper { recipients: [] }` is accepted**, not rejected — a private note-to-self (the owner
  still reads their own message via `users[owner] = Owner`). This is a valid if unusual input; no
  special-case rejection (YAGNI).
- **A whisper naming the GM** works exactly like naming any other recipient: their uuid lands in
  `users` as `Observer`, so they see it despite `gm_role = Some(DocRole::None)`.
- **A `recipients` list that includes the sender's own uuid must not downgrade them.** `users` is
  built by inserting `owner: Owner` *after* (or by filtering `user` out of) the recipient inserts,
  so a redundant self-recipient can never overwrite the owner's `Owner` role with `Observer` via
  map-insertion order.
- No new size/count cap on `recipients` — the existing generic 256 KB `validate_system_size` cap
  already bounds it; a world's membership size is the practical ceiling.

## 4. Client-core wire mirror

`src/client/core/src/wire.ts` gains:
- `AudienceSchema` — a `z.discriminatedUnion("kind", ...)` mirroring the three `Audience`
  variants, added to the `SendMessage`-frame Zod schema alongside the existing `ActorOwnerRef`
  mirror.
- `PermissionSetSchema` gains the optional `gm_role` field (`DocRoleSchema.optional()` or
  equivalent), since `PermissionSet` is a generic, already-mirrored envelope type used by every
  document type, not just messages.

No other client-core changes — same "headless, wire-mirror only" boundary c-1 established.

## 5. Testing strategy

Per-recipient proof on **every** egress path (broadcast create/update/delete, search, resync/load)
that:
- A whisper's non-recipient receives nothing (existence, id, content — nothing).
- A whisper that does not name the GM is invisible to the GM.
- A whisper that does name the GM is visible to them.
- A `GmOnly` message is visible to a GM and invisible to a regular (non-owner) member.
- A user promoted to GM **after** a `GmOnly` message was sent immediately sees it on their next
  read (proves dynamic resolution, not a frozen snapshot at send time); a user demoted from GM
  immediately loses access to prior `GmOnly` messages.
- A malformed/foreign `recipients` uuid rejects the whole `SendMessage` (nothing is persisted).
- `Public` messages are unaffected by any of the above (regression coverage for c-1's existing
  behavior, since `resolve_access`'s GM branch is being modified).
- `Audience` ts-rs↔Zod parity (drift guard).

**Mandatory buddy-check: two blind security reviewers.** This is a real change to the one
permission chokepoint every subsystem depends on — per the parent spec's risk rating (HIGH,
architecture-consent) and this project's reviewed-change discipline for security-sensitive
permission work.

## 6. Codebase-skill gate

Updates `shadowcat-codebase-chat` (the `Audience` model, the `gm_role`-driven audience table above)
and `shadowcat-codebase-documents-permissions` (the new `gm_role` field and its effect on
`resolve_access`'s GM branch) under the reviewed skill-update gate — both skills describe
subsystems this checkpoint changes.
