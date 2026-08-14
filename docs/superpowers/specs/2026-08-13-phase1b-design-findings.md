# Phase 1b — reviewed design findings, and the decisions taken

Two blind reviewers examined the first design proposal before it became a spec. Both returned
**needs rework**. This file is the input to the re-brainstorm; the superseded proposal beside it is
kept only so the corrections have something to point at.

## Decisions taken by the owner

1. **Do both**: bound the resync request's starting sequence NOW, and build the snapshot as the
   point-in-time seam the deferred audit-replay milestone needs.
2. **Re-brainstorm the design** from these findings rather than patching the proposal.
3. Snapshot rides on the operation (not a sibling map, not a log column) — those rejections were
   confirmed sound by both reviewers.
4. The world-grants input is recorded as a design note, not a bug — after the author's own
   overstatement of it was corrected.
5. Audit-grade point-in-time replay is scheduled as its own milestone.

## What both reviewers found independently — treat as established

### The input enumeration was incomplete

Both derived the set from code before reading the proposal's table, and both found the same two
gaps. The proposal claimed four inputs; there are six.

- **The target's EXISTENCE at redaction time is a live input.** `let Some(cur) … else { continue }`
  is the only thing dropping an Update whose document has since been deleted. The proposal's claim
  that the pre-image load "loses its reason to exist" would silently start delivering those ops.
- **`collect_hidden` addresses embedded children by ARRAY INDEX.** Hidden pointers are built as
  `{prefix}/embedded/{key}/{idx}` from the CURRENT arrays. Snapshotting override values without the
  positional layout means an inserted or removed sibling shifts every later index, so the hidden set
  names the wrong children — the hidden field leaks or an innocent sibling is nulled. **That is the
  original defect reproduced one level deeper, arriving after an anti-drift test declares the class
  closed.**
- `doc_type` has TWO uses, not one: it selects the world grant set AND gates the token owner-floor
  in `effective_role`. A design treating it as a grant-map key gets role resolution wrong for
  tokens, the most common document in the system.

### `Create` and `Delete` are NOT already point-in-time correct

Both verified this against source. Both arms call `effective_owner_via` against the room's LIVE
actor table and project LIVE world grants. They carry a point-in-time document but resolve access
against current state — exactly the condition the proposal's own argument says requires a snapshot.
**The "fix surface is one match arm" scoping claim is false**, and it would have defined the
anti-drift test's coverage.

### The proposed type shape cannot work

The event log and the wire share one `Serialize` impl. A field on `Operation::Update` is either
serialized — persisted AND transmitted to every recipient — or skipped, and therefore never
persisted. There is no single-type configuration satisfying both.

Worse: `Operation` is ALSO the ingress type (`ClientMsg::Intent`), so a snapshot field would be
**client-forgeable** — a forged redaction context written once into the log would govern every
future replay of that op, un-retractably. Both write loops happen to rebuild the op today, so a
forgery would be discarded, but that is incidental structure, not enforcement.

What a forgotten strip would leak is not generic: the world grants carry the unprojected `by_user`
map — every user's id and grants — which is exactly what `project_grants_for` exists to withhold.
And the client could not detect it: the Zod object is non-strict, so a leaked field parses clean and
vanishes silently.

**Remedy both proposed: split the types.** A snapshot-free type for ingress and egress, and a
snapshot-bearing server-internal type for the authoritative loops and the log. The strip becomes a
compile-time impossibility rather than a discipline.

### THE CRITICAL ONE — a pure snapshot breaks the tightening direction

Both found this independently and neither was prompted toward it.

Today, redaction reading CURRENT state is what makes retroactive hiding work: a GM adds a `gm_only`
override and every historical change to that pointer is dropped on replay. The retraction branch
keyed on `touches_permissions` exists precisely to make hiding stick client-side.

Under "redaction reads ONLY what the op carries", that property is destroyed. A GM revokes access —
narrows an override, reassigns a token, revokes a grant — and the player resyncs from sequence zero
and reconstructs every value the GM has since hidden, writing them over a correctly-redacted store.

**The proposal closed "loosening exposes the past" and opened "tightening no longer hides it."** The
reachability argument is identical in both directions; the proposal analysed only the direction that
helped.

**Remedy both proposed, and it is a better invariant than the original:**

> An Update's redaction is the CONJUNCTION of commit-time and current visibility. Commit-time state
> comes only from what the op carries; current state may only SUBTRACT visibility, never add it.

That is more checkable, not less — monotonicity is a testable property.

### A per-op snapshot leaks within a single command

One reviewer, verified: `load_update_docs` runs AFTER the whole transaction, so today every op is
redacted against the command's FINAL post-image. A per-op snapshot captures the post-image of THAT
op. So `[Update {secret := X}, Update {add gm_only override}]` broadcasts X live, because op one's
snapshot has no override yet. The same failure occurs within one op if the changes are ordered
value-then-override.

**The snapshot must be captured from the whole command's post-image, after the mutation loop
completes** — not from the intermediate value inside it. "At commit time current state IS commit
state" is false for any multi-op command.

### Defect 2 is only half closed

The redaction half closes: the reused id no longer resolves to the wrong permission set. But the op
is still addressed by `doc_id`, and the client applies it to whatever that id names in its store —
the new document. Removing the existence gate makes this MORE reachable, not less. One reviewer
proposes a commit-time identity witness so a mismatch drops the op.

### Growth is worse than priced

The payload is one permission subtree PER EMBEDDED DESCENDANT plus two membership-scaled `by_user`
maps, on every Update — including one per token move, the highest-frequency op. An actor with a
large embedded inventory carries its whole inventory's permission tree on a single stat write.

A derivable prune exists: `redact_change` only matches overrides equal to, ancestor of, or
descendant of a change path — so the snapshot can be reduced to what this op's paths reach, EXCEPT
when the retraction branch fires and needs the whole tree. Both cases decidable from the op alone.

### The anti-drift test's shape

Both rejected the proposal's instinct as one level too abstract. A type that cannot be built from
live state is not achievable — the constructor runs at commit time, where live IS commit.

What works is **signature deprivation**: remove the live-state parameters from the function
entirely, so reintroducing a lookup requires adding a parameter and threading it from every call
site — a loud diff, not a one-line addition. This is only possible once all three arms are
snapshotted, which is another reason the scope claim mattered.

Pair it with a **behavioural mutation test**: mutate each live input independently — the target's
overrides, its default, the linked actor's owner, an embedded child's index, the world grants — and
assert byte-identical output each time. That catches a read through a helper nobody thought to grep
for. The embedded-index case is the one a grep-shaped test could never find.

### The alternative that was never priced

Both flagged that the proposal names the unbounded, client-supplied `from_seq` twice as what makes
the defects reachable, then never considers bounding it. Any world member can request the entire
world history at any time, unvalidated — an unbounded read amplification independent of these
defects, and a growing liability the snapshot would make worse.

Bounding it collapses the exposure window from forever to ring depth, at zero per-op cost, with no
client-forgeable field, and it PRESERVES retroactive hiding, which the snapshot design loses.

**Owner's decision: do both.** The bound is fail-closed and immediately valuable; the snapshot
builds the seam the audit milestone needs.

## Smaller findings worth carrying

- The wire drift guard for the operation type compares discriminant TAGS only, so a new field on the
  Rust side fails no test. Worth strengthening regardless of this work.
- `apply_command` never loads world capability defaults at all, and neither loop resolves the
  effective owner from the POST-image — an Update may itself write the owner or the actor link.
- The command's inverse operation gains an undefined snapshot semantic; decide it rather than
  leaving a security field unspecified on a public method.
- The recipient's role is resolved once at socket open and never refreshed, so a mid-connection
  demotion is not honoured on replay. Pre-existing, separate, worth naming.
- Log back-compat must be stated: a missing snapshot must DROP the op, never fall back to a live
  lookup. A silent fallback leaves the defect intact for every pre-fix event behind a green test.

## Where the re-brainstorm starts

The corrected problem is not "snapshot the redaction inputs". It is:

> Redaction must be the conjunction of two views — what was permitted at commit, and what is
> permitted now — where the commit-time view is carried by the op and the current view can only
> withhold. Establish that across all three operation arms, capture it per command rather than per
> op, keep it off the wire by construction, and bound how far back a client may ask to replay.
