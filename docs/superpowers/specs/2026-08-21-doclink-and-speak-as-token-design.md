# In-Body Doc-Link Segment & Speak-As-Token-Instance — Design

**Status:** approved (self-directed design under the standing debt-burndown campaign authority).

**Spec for:** `docs/TODO.md` bucket-C sub-projects 6 and 7 — grouped in one spec because both are
small, both touch `chat::mod`'s message-ingest pipeline, and both lift a "reserved but not yet
built" half-feature the codebase already left a clear seam for.

---

## Part A — `Segment::DocLink`

### A.1 Scope

A free-form, author-inserted link from chat body text to any document (or a placed token,
resolved through its actor) the author has visibility into — distinct from the existing
header-level actor-name → sheet link, which is driven by `actor_owner` attribution, not body
content.

### A.2 Data model

```rust
/// What a `Segment::DocLink` points at — mirrors the client's `SheetRef` shape (the
/// already-established "one anonymous cross-file-shared shape gets one name" precedent), given a
/// server-side equivalent since `SheetRef` itself is client-only TS.
pub enum DocLinkTarget {
    /// A top-level document, optionally a one-level embedded child.
    Doc { doc_id: Uuid, embedded_path: Option<String> },
    /// A placed token, resolved client-side via its linked/embedded actor — same resolution
    /// `ctx.openDocument` already performs for a `{tokenId}` `SheetRef`.
    Token { token_id: Uuid },
}

pub enum Segment {
    // ...existing variants unchanged...
    DocLink { target: DocLinkTarget, label: String },
}
```

`label` is the display text captured at authoring time (the picker's resolved document/token name)
— rendering never re-resolves a live name lookup for `label` itself, only for the fail-closed
existence/visibility gate below.

### A.3 Server producer — reuses `scan_body`'s span mechanism, not a new mechanism

`scan_body` already recognizes `[[formula]]` (inline roll) and `[[roll:formula|label]]` (button) by
prefix-sniffing a balanced `[[...]]` span's content. This adds two more prefixes to the same scan:
`[[doc:<uuid>]]`, `[[doc:<uuid>/<embedded_path>]]`, and `[[token:<uuid>]]` — a new `BodyChunk::
DocLink(DocLinkTarget)` variant alongside `Inline`/`Button`/`Text`, produced by the same balanced-
bracket scanner with one more prefix branch. No dice-notation ambiguity: `doc:`/`token:` are not
valid dice-formula prefixes, so the existing roll-formula branch never mis-claims these spans.
`chat::mod`'s ingest match over `BodyChunk` gains a `DocLink` arm that constructs
`Segment::DocLink` directly (no parse/roll/authz work needed at ingest — see A.4, the target
document is never validated to exist or be visible at write time, exactly like the actor-name
header link's fail-closed client-side gate).

### A.4 Visibility — resolved design fork: no server-side existence/authz check at ingest,
fail-closed at render, reusing the actor-name-link precedent exactly

The existing actor-name header link already establishes the pattern this reuses verbatim: the
server never validates that a link's target is visible to (or even exists for) the sender at
ingest time; the CLIENT gates rendering per-recipient by checking `ctx.documents.get(target_id)`
presence (already redacted per-recipient by the normal document pipeline) and rendering inert
plain text on a miss. `Segment::DocLink` needs zero new server-side authz code — visibility is
already fully handled by the fact that a recipient's `ctx.documents` only ever contains documents
their own `PermissionSet` admits. This is a direct "never fork a decision across two paths" win:
building a second, bespoke existence/visibility check at ingest would be a second implementation of
exactly what per-recipient document redaction already guarantees.

### A.5 Client authoring UX

A new `@doc`-style composer trigger (parallel to the existing "Speak as" picker's UI weight, not
its code) opens a searchable document picker via the existing `searchDocuments` AppContext seam,
and inserts the corresponding `[[doc:<id>]]`/`[[token:<id>]]` span into the composer's raw text at
the cursor — the server-side scan in A.3 is what turns it into a real segment on send. No new
search/lookup code: this is a new trigger wired to search machinery that already exists.

---

## Part B — Speak-as-token-instance

### B.1 Scope

Lift `ActorOwnerRef::TokenInstance`'s current fail-closed ingest rejection
(`SendMessageError::ActorNotSpeakable`) by giving it the same real ownership check the accepted
`ActorOwnerRef::Actor` arm already has, plus a composer/scene-tools UX to select it.

### B.2 Ownership check — reuses `effective_owner`, not a new resolution rule

The `Actor` arm's existing check (doc exists, `doc_type == "actor"`, same-world, `owner == sender
|| sender is GM`) becomes, for `TokenInstance{token_id}`:

1. Load the token document; must exist, `doc_type == "token"`, same world as the sending room.
2. Load its linked actor (if any) the same way token/actor resolution already works elsewhere.
3. Call `effective_owner(token_doc, linked_actor_doc)` — the SAME function `permission.rs` already
   uses as the single source of truth for "who owns this placed token" (a token's own `owner`
   override wins, else it inherits its linked actor's owner). This is a direct reuse, not a
   reimplementation, per the codebase's own "never fork a decision across two paths" invariant —
   this exact function already backs write-permission floor resolution for tokens.
4. `effective_owner(..) == Some(sender) || sender is GM` — same authority shape as the `Actor` arm.

On success, the message stores `ActorOwnerRef::TokenInstance{token_id}` (already accepted by
storage — only the ingest match arm currently rejects it) and the client's ALREADY-BUILT
`resolveActorOwnerName`'s `TokenInstance` render branch (confirmed dead code today, since no
message could ever carry this ref) starts rendering for real, with no client rendering change
needed.

### B.3 Composer UX — surfaced from the token, not the existing actor picker

A token instance is a per-scene placement, not a member of the world-wide actor list the existing
"Speak as" picker enumerates — so this is a NEW affordance (scene-tools: right-click a placed token
→ "Speak as this token"), not an addition to the existing picker's option list. Selecting it sets
the composer's pending `actor_owner` to `TokenInstance{token_id}` for the next message sent, mirrored
by a small "speaking as: <token name>" indicator in the composer, matching how the existing actor
picker's selection is surfaced today.

### B.4 Testing

- Ingest tests: token owner (via its own override) can speak as it; token inheriting its linked
  actor's owner — that actor's owner can speak as the token; a non-owner, non-GM sender is
  rejected; cross-world token id is rejected (mirrors the `Actor` arm's same-world check);
  GM can always speak as any token regardless of owner.
- `resolveActorOwnerName`'s existing `TokenInstance` branch gets its first real integration test
  now that ingest can produce the ref it renders.

---

## Non-goals (both parts)

- No change to `Segment`'s other variants or to the existing actor-name header link.
- No retroactive `DocLink` authoring for already-sent messages (edit-then-insert-a-link is just an
  ordinary edit through the existing pipeline, not new work).
- No change to token/actor ownership resolution itself (`effective_owner` is reused, not modified).
