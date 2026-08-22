# In-Body Doc-Link Segment & Speak-As-Token-Instance — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development
> (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use
> checkbox (`- [ ]`) syntax for tracking.

**Goal:** Implement both small features from `docs/TODO.md` bucket-C sub-projects 6 and 7 — a
free-form `Segment::DocLink` chat body link (Part A) and lifting `ActorOwnerRef::TokenInstance`'s
fail-closed ingest rejection into a real ownership-checked "speak as this token" feature (Part B).

**Spec:** `docs/superpowers/specs/2026-08-21-doclink-and-speak-as-token-design.md` — read in full
before implementing any task. Every design fork it resolves (DocLinkTarget mirroring the client's
`SheetRef` shape; `[[doc:...]]`/`[[token:...]]` reusing `scan_body`'s span mechanism; no
server-side existence/authz check at ingest for `DocLink`, fail-closed client-side rendering
reusing the actor-name-link precedent verbatim; `effective_owner` reuse for `TokenInstance`
ownership; scene-tools-surfaced UX, not the existing actor picker) is FINAL and not open for
re-litigation in this plan.

**Standing campaign directive (binding on every subagent dispatched against this plan — copy
verbatim into every dispatch):** "Invoke the shadowcat core skill immediately. You goal is to
close all existing bugs and to-dos within Shadowcat. The iron rule is no deferrals, of existing
work, or new work as it comes up - we fix this now unless I give my EXPRESS authorization. The
only exception is if a bug or to-do has a genuine blocker that is already logged in a milestone in
PLAN.md that has not been started yet. Another iron clad is rule is that when faced with a design
fork, determine the best long term shape in keeping with our plans and goals, and implement
accordingly. You only need to ask me if the question "what is the best long term shape in keeping
with our plans and goals?" is not able to answer the question. Churn is not a concern. This
paragraph must be copied verbatim to any agents dispatched in this campaign."

## Resolved spec gaps (binding on implementers — do not re-litigate)

1. **`Segment::DocLink.label` transport.** The spec's A.2 types `label: String` as a REQUIRED
   field ("the picker's resolved document/token name... rendering never re-resolves a live name
   lookup for `label`"), but A.3's illustrative span examples (`[[doc:<uuid>]]`,
   `[[token:<uuid>]]`) omit any label syntax, and A.5's composer-insertion prose likewise omits it.
   Since `label` is required and A.4 explicitly forbids any server-side resolution work at ingest
   (no doc lookup to derive a display name), the span MUST carry the label itself. Resolved: the
   on-wire grammar is `[[doc:<uuid>[/<embedded_path>]|<label>]]` / `[[token:<uuid>|<label>]]` —
   the exact `|<label>` suffix `scan_body` already uses for `[[roll:formula|label]]` buttons. A
   missing or empty label is a hard parse error (`RollError::MalformedDocLink`), not a silently
   accepted `None`, because the field is non-optional.
2. **"Right-click a placed token" (spec B.3).** No context-menu primitive exists anywhere in this
   codebase (`.claude`/graphify confirm zero "ContextMenu" component). Building one from scratch
   would be a scope expansion the spec does not request and the "small feature" framing does not
   support. The functional requirement — select a placed token, set a pending
   `TokenInstance{token_id}` speak-as target for the composer's next send — is implemented via the
   codebase's own existing precedent for "an action driven by the current token selection"
   (`FaceSwapPalette`/`ConditionsPanel`'s "token-selection-driven" pattern): a button in
   `ToolRail.svelte`, shown when exactly one token is selected via `ctx.tokenSelection`, gated
   client-side the same way the existing "Speak as" actor picker is gated
   (`ctx.role === "gm"` or `ownerFloorApplies`). This satisfies the resolved design fork
   ("scene-tools-surfaced UX, not the existing actor picker") without inventing new UI
   infrastructure.
3. **"For the next message sent" (spec B.3) is one-shot, not sticky.** Unlike the composer's
   sticky actor `<select>` (persists across sends until changed), the pending token selection is
   consumed and cleared on the very next send attempt — mirroring the spec's literal wording. If a
   sent attempt fails validation (e.g. empty body), the pending token is NOT consumed (the guard
   that blocks an empty send runs before the consume call).
4. **Part B.4's "first real integration test."** No chat end-to-end (client+server) test harness
   exists in this codebase today (confirmed: no `e2e` file references chat). Building one is out of
   this plan's scope. This is satisfied instead by: (a) Task 6's server-side ingest tests, which
   for the first time make `handle_send_message` actually PRODUCE a stored `TokenInstance`
   `actor_owner` (previously always rejected), and (b) `MessageCard.test.ts`'s pre-existing
   `token_instance` render-branch unit tests (`resolves an actor_owner{kind:'token_instance'}...`,
   `a dangling token_instance reference fails closed...`), which now exercise a code path real
   traffic can reach. No new test infrastructure is added.

## Global Constraints

- **No lint suppressions of any kind.** `#[allow(dead_code)]`, `#[allow(unused*)]`,
  `#[allow(clippy::*)]`, `#[expect(...)]`, `eslint-disable` of `no-unused-vars`, `@ts-ignore` /
  `@ts-nocheck` are ALL forbidden. Fix the code, make it live, `#[cfg(test)]`-scope test-only
  items, or delete them.
- **RULE 15:** cite symbols (type/function/method names) in code comments, never file names or
  line numbers.
- **RULE 16 (no ephemeral referents in CODE comments):** no milestone ids, no dated doc pointers,
  no history/process narration in `.rs`/`.ts`/`.svelte` source comments. The design spec and this
  plan are `docs/superpowers/` artifacts and are exempt from this rule themselves, but nothing from
  either may be copy-pasted into a CODE comment as a citation.
- **Every new/changed Rust item needs a doc comment.** `src/server/src/chat/mod.rs` and
  `src/server/src/chat/rolls.rs` both carry `#![deny(missing_docs)]` +
  `#![deny(clippy::missing_docs_in_private_items)]` — a new undocumented item, field, or variant
  fails the build.
- **`Segment`/`DocLinkTarget`/`RollError`/`BodyChunk` derive `Eq`** — every new field/variant must
  keep every variant `Eq`-comparable (no floats, no non-`Eq` types).
- **Never fork a decision across two paths.** The `TokenInstance` ownership check reuses
  `Repository::effective_owner_of` (the existing repo-level chokepoint wrapping
  `permission::effective_owner`) rather than re-deriving ownership inline. The client renders
  `Segment::DocLink` the same fail-closed, `ctx.documents`-presence-gated way the existing
  `actorOpenRef` derivation in `MessageCard.svelte` already does — do not invent a second gating
  mechanism.
- **No PII/secrets in test fixtures.** All example UUIDs, names, and text used below and in any
  test authored from this plan are synthetic.
- **Per-task CI gate battery**, all must exit 0 before a task is considered done:
  - Server tasks (from `src/server/`): `cargo fmt --all -- --check`; `cargo clippy --all-targets --
    -D warnings`; `cargo test --all`.
  - Client tasks (from the repo root): `pnpm -r typecheck`; `pnpm -r test`; `pnpm lint`.
  - A task touching both sides runs both batteries.
- **`i18n`:** every new UI string goes through `t()`, with its key added to
  `src/client/ui-kit/src/locales/en.ts` (the only locale file in this repo).
- **Reviewed Skill-Update Gate:** this work touches `src/server/src/chat/{mod.rs,rolls.rs,
  sanitize.rs}`, `src/client/core/src/chat-docs.ts`, `src/modules/chat-card/src/MessageCard.svelte`,
  `src/modules/chat-composer/src/Composer.svelte` (all `shadowcat-codebase-chat` territory);
  `src/client/ui-kit/src/{appContext.ts,speakAsToken.svelte.ts}`,
  `src/client/shell/src/lib/Table.svelte` (`shadowcat-codebase-client-shell` territory); and
  `src/modules/scene-tools/src/ToolRail.svelte` (`shadowcat-codebase-scene-rendering` territory,
  per that skill's own description covering `src/modules/scene-tools`). The final task updates all
  three skills, dispatches `shadowcat-spec-reviewer` on the skill diffs, and bumps
  `.claude/.claude-plugin/plugin.json`'s `version` from `1.0.59` to `1.0.60`.
- Never delete files with `rm`/`Remove-Item`; use `trash`.

## Task Decomposition Rationale

Eleven tasks. Part A (DocLink, Tasks 1–5) and Part B (speak-as-token, Tasks 6–9) are independent
features that happen to share `chat::mod`/`chat::rolls` and a few client files; they are ordered
sequentially (Part A first) rather than interleaved so each part lands as a reviewable, working
unit before the next begins. Task 10 is a lightweight combined-review pass confirming the two
parts still compose correctly in the shared files. Task 11 is the mandatory doc-sync/skill-update
closeout.

1. **Server DocLink types + pure `scan_body` parsing** — the new wire shape and its parser,
   independently unit-testable with zero ingest wiring.
2. **Server DocLink ingest wiring** — plugs the parser into `handle_send_message`, fixes the one
   exhaustive-match compile site the new `Segment` variant breaks, adds end-to-end ingest tests.
3. **Client DocLink Zod mirror** — `chat-docs.ts`'s `DocLinkTarget`/`ChatSegment` extension,
   independently testable against hand-built JSON bodies.
4. **Client DocLink rendering** — `MessageCard.svelte`'s fail-closed, presence-gated render arm.
5. **Client DocLink authoring UX** — the composer's `@doc` search-and-insert trigger.
6. **Server speak-as-token ownership check** — the `TokenInstance` ingest arm, reusing
   `effective_owner_of`; retargets the one existing "always rejected" test and adds the five
   ownership-matrix cases the spec calls for.
7. **`SpeakAsToken` AppContext seam** — the new client-side pending-selection primitive, mirroring
   `SceneSelection`'s exact shape, wired through every AppContext construction site.
8. **Scene-tools "speak as this token" affordance** — the `ToolRail.svelte` button.
9. **Composer speak-as-token consumption + indicator** — the composer reads/consumes the pending
   selection on send and shows "speaking as: `<name>`".
10. **Cross-part compile/behavior check** — both parts' edits to the same shared files
    (`chat::mod`, `chat::rolls`) are re-verified together; no new code, gate re-run only.
11. **Documentation + skill-update closeout** — `docs/TODO.md` sync, the three skill updates +
    `shadowcat-spec-reviewer` dispatch, plugin version bump, full-repo gate re-run.

---

## Task 1: Server — `DocLinkTarget`, `Segment::DocLink`, and `scan_body`'s `doc:`/`token:` parsing

**Files:**
- Modify: `src/server/src/chat/mod.rs` (add `DocLinkTarget`, add `Segment::DocLink`, remove the
  stale "Reserved for a future `DocLink` segment variant" comment)
- Modify: `src/server/src/chat/rolls.rs` (import `DocLinkTarget`; extend `BodyChunk`; add
  `RollError::MalformedDocLink` + `Display` arm; add `parse_doc_link`; wire it into `scan_body`;
  update doc comments; add unit tests)

**Interfaces:**
- Produces: `chat::DocLinkTarget` (`pub enum`, `Debug + Clone + PartialEq + Eq + Serialize +
  Deserialize`, `#[serde(tag = "kind", rename_all = "snake_case")]`) with variants `Doc { doc_id:
  Uuid, embedded_path: Option<String> }` and `Token { token_id: Uuid }`.
- Produces: `chat::Segment::DocLink { target: DocLinkTarget, label: String }`.
- Produces: `chat::rolls::RollError::MalformedDocLink` (new variant).
- Produces: `chat::rolls::BodyChunk::DocLink { target: DocLinkTarget, label: &'a str }` (new
  variant, `pub(crate)`).
- Produces (private): `chat::rolls::parse_doc_link(content: &str) -> Result<Option<BodyChunk<'_>>,
  ()>`.
- Consumed by: Task 2 (`handle_send_message`'s ingest match, `sanitize.rs`'s test-only exhaustive
  match), Task 3 (client Zod mirror of `Segment`/`DocLinkTarget`), Task 4 (`MessageCard.svelte`
  render).

### Steps

- [ ] **Step 1: Add `DocLinkTarget` and `Segment::DocLink` to `src/server/src/chat/mod.rs`.**

  Find:
  ```rust
      LinkPreview {
          /// The previewed URL as posted.
          url: String,
          /// Server-extracted title.
          title: String,
          /// Server-extracted description (may be empty).
          description: String,
      },
      // Reserved for a future `DocLink` segment variant.
  }
  ```

  Replace with:
  ```rust
      LinkPreview {
          /// The previewed URL as posted.
          url: String,
          /// Server-extracted title.
          title: String,
          /// Server-extracted description (may be empty).
          description: String,
      },
      /// A free-form, author-inserted link to a document or placed token, captured with its
      /// display label at authoring time (`label` is never re-resolved at render — only the
      /// fail-closed existence/visibility gate below re-checks `target`). Distinct from the
      /// actor-name header link, which is driven by `actor_owner` attribution, not body content.
      /// Produced by `chat::rolls::scan_body`'s `doc:`/`token:` prefix branch — reuses the SAME
      /// balanced `[[...]]` span mechanism as `RollEmbed`/`RollButton`, not a new one. No
      /// existence/visibility check runs against `target` at ingest: the CLIENT fails closed at
      /// render by checking `ctx.documents` presence for the target id (already redacted
      /// per-recipient by the normal document pipeline), the exact precedent the actor-name
      /// header link established.
      DocLink {
          /// What the link points at.
          target: DocLinkTarget,
          /// Display text captured at authoring time (the composer's `|<label>` span suffix).
          /// Rendering never re-resolves a live name lookup for this field.
          label: String,
      },
  }

  /// What a `Segment::DocLink` points at — mirrors the client's `SheetRef` shape (the
  /// established "one anonymous cross-file-shared shape gets one name" precedent), given a
  /// server-side equivalent since `SheetRef` itself is client-only TS. Carried inside
  /// `Segment::DocLink`; parsed in full by `chat::rolls::scan_body`'s `doc:`/`token:` prefix
  /// branch — `handle_send_message`'s ingest arm does no further parsing.
  #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
  #[serde(tag = "kind", rename_all = "snake_case")]
  pub enum DocLinkTarget {
      /// A top-level document, optionally one level into an embedded child.
      Doc {
          /// The top-level document's id.
          doc_id: Uuid,
          /// A `/embedded/<collection>/<index>` pointer, one level deep, or `None` for the
          /// top-level document itself. Opaque to the server — never validated against the
          /// referenced document's actual `embedded` shape at ingest (the client's own
          /// `resolveDocRef` fails closed on a malformed/dangling pointer at open time).
          embedded_path: Option<String>,
      },
      /// A placed token, resolved client-side via its linked/embedded actor — the same
      /// resolution `ctx.openDocument` already performs for a `{tokenId}` `SheetRef`.
      Token {
          /// The placed token's document id.
          token_id: Uuid,
      },
  }
  ```

- [ ] **Step 2: Import `DocLinkTarget` into `src/server/src/chat/rolls.rs`.**

  Find:
  ```rust
  use uuid::Uuid;

  use crate::dice::notation::{self, ParseContext, ParseError};
  ```

  Replace with:
  ```rust
  use uuid::Uuid;

  use super::DocLinkTarget;
  use crate::dice::notation::{self, ParseContext, ParseError};
  ```

- [ ] **Step 3: Extend `BodyChunk` and its doc comment.**

  Find:
  ```rust
  /// One scanned chunk of a message body: literal text between spans, an
  /// inline roll to execute, or a button to validate-and-store.
  #[derive(Debug, Clone, PartialEq, Eq)]
  pub(crate) enum BodyChunk<'a> {
      /// Literal body text between spans.
      Text(&'a str),
      /// An `[[formula]]` inline roll to execute.
      Inline(&'a str),
      /// A `[[roll:...]]` button to validate and store unexecuted.
      Button {
          /// The formula inside the span.
          formula: &'a str,
          /// Optional label after the `|` separator.
          label: Option<&'a str>,
      },
  }
  ```

  Replace with:
  ```rust
  /// One scanned chunk of a message body: literal text between spans, an
  /// inline roll to execute, a button to validate-and-store, or a doc/token
  /// link to store directly.
  #[derive(Debug, Clone, PartialEq, Eq)]
  pub(crate) enum BodyChunk<'a> {
      /// Literal body text between spans.
      Text(&'a str),
      /// An `[[formula]]` inline roll to execute.
      Inline(&'a str),
      /// A `[[roll:...]]` button to validate and store unexecuted.
      Button {
          /// The formula inside the span.
          formula: &'a str,
          /// Optional label after the `|` separator.
          label: Option<&'a str>,
      },
      /// A `[[doc:<uuid>[/<embedded_path>]|<label>]]` or `[[token:<uuid>|<label>]]` span: a
      /// free-form author-inserted link, captured with its target and display label fully
      /// parsed — `chat::mod`'s ingest arm does no further parsing (see `Segment::DocLink`'s
      /// own doc comment).
      DocLink {
          /// What the link points at.
          target: DocLinkTarget,
          /// Display text captured at authoring time (the composer's `|<label>` suffix); never
          /// empty (an empty/absent label is a `RollError::MalformedDocLink`).
          label: &'a str,
      },
  }
  ```

- [ ] **Step 4: Update `MAX_INLINE_ROLLS`'s doc comment.**

  Find:
  ```rust
  /// Cap on non-text chunks (`Inline`/`Button`) `scan_body` may extract from one
  /// message body.
  pub(crate) const MAX_INLINE_ROLLS: usize = 8;
  ```

  Replace with:
  ```rust
  /// Cap on non-text chunks (`Inline`/`Button`/`DocLink`) `scan_body` may extract from one
  /// message body.
  pub(crate) const MAX_INLINE_ROLLS: usize = 8;
  ```

- [ ] **Step 5: Rewrite `scan_body`'s doc comment and add the `doc:`/`token:` dispatch branch +
  `parse_doc_link` helper.**

  Find:
  ```rust
  /// Balanced span scanner. A span opens at `[[` and closes at the first `]]`
  /// reached while a per-span nesting `depth` is 0: inside the span, a single
  /// `[` increments `depth` and a single `]` decrements it (a lone `]` at
  /// `depth == 0` that is NOT immediately followed by a second `]` is left as
  /// literal content — `depth` never goes negative), so a notation label's own
  /// brackets (`[[4d6[atk]]]` -> formula `4d6[atk]`) survive intact. A `roll:`
  /// prefix on the span's content produces a `Button`; the content is then
  /// split on the first `|` into `formula`/an optional trimmed `label` (empty
  /// after trim => `None`). Every other span is an `Inline`. Errors: a span
  /// opened but never closed by a balanced `]]` (`RollError::Unterminated`);
  /// more than `MAX_INLINE_ROLLS` non-text chunks (`RollError::TooManyInline`).
  pub(crate) fn scan_body(body: &str) -> Result<Vec<BodyChunk<'_>>, RollError> {
  ```

  Replace with:
  ```rust
  /// Balanced span scanner. A span opens at `[[` and closes at the first `]]`
  /// reached while a per-span nesting `depth` is 0: inside the span, a single
  /// `[` increments `depth` and a single `]` decrements it (a lone `]` at
  /// `depth == 0` that is NOT immediately followed by a second `]` is left as
  /// literal content — `depth` never goes negative), so a notation label's own
  /// brackets (`[[4d6[atk]]]` -> formula `4d6[atk]`) survive intact. A `doc:`/
  /// `token:` prefix on the span's content produces a `DocLink`: grammar
  /// `doc:<uuid>[/<embedded_path>]|<label>` or `token:<uuid>|<label>`, fully
  /// parsed here (`chat::mod`'s ingest arm does no further parsing). A `roll:`
  /// prefix on the span's content produces a `Button`; the content is then
  /// split on the first `|` into `formula`/an optional trimmed `label` (empty
  /// after trim => `None`). Every other span is an `Inline`. Errors: a span
  /// opened but never closed by a balanced `]]` (`RollError::Unterminated`);
  /// more than `MAX_INLINE_ROLLS` non-text chunks (`RollError::TooManyInline`);
  /// a `doc:`/`token:`-prefixed span with an unparseable id or a missing/empty
  /// `|<label>` suffix (`RollError::MalformedDocLink`).
  pub(crate) fn scan_body(body: &str) -> Result<Vec<BodyChunk<'_>>, RollError> {
  ```

  Find:
  ```rust
          let content = &body[content_start..content_end];
          non_text += 1;
          if non_text > MAX_INLINE_ROLLS {
              return Err(RollError::TooManyInline(non_text));
          }
          if let Some(rest) = content.strip_prefix("roll:") {
              let (formula, label) = match rest.split_once('|') {
                  Some((f, l)) => {
                      let l = l.trim();
                      (f, if l.is_empty() { None } else { Some(l) })
                  }
                  None => (rest, None),
              };
              chunks.push(BodyChunk::Button { formula, label });
          } else {
              chunks.push(BodyChunk::Inline(content));
          }

          pos = content_end + 2; // past the terminating "]]"
          text_start = pos;
      }

      Ok(chunks)
  }
  ```

  Replace with:
  ```rust
          let content = &body[content_start..content_end];
          non_text += 1;
          if non_text > MAX_INLINE_ROLLS {
              return Err(RollError::TooManyInline(non_text));
          }
          match parse_doc_link(content) {
              Ok(Some(chunk)) => chunks.push(chunk),
              Err(()) => return Err(RollError::MalformedDocLink),
              Ok(None) => {
                  if let Some(rest) = content.strip_prefix("roll:") {
                      let (formula, label) = match rest.split_once('|') {
                          Some((f, l)) => {
                              let l = l.trim();
                              (f, if l.is_empty() { None } else { Some(l) })
                          }
                          None => (rest, None),
                      };
                      chunks.push(BodyChunk::Button { formula, label });
                  } else {
                      chunks.push(BodyChunk::Inline(content));
                  }
              }
          }

          pos = content_end + 2; // past the terminating "]]"
          text_start = pos;
      }

      Ok(chunks)
  }

  /// Parses `content` as a `doc:`/`token:`-prefixed span. `Ok(None)` when `content` carries
  /// neither prefix (the caller falls through to `roll:`/`Inline` handling); `Err(())` when the
  /// prefix is recognized but the id/label grammar is malformed (the caller returns
  /// `RollError::MalformedDocLink`); `Ok(Some(chunk))` on success. Grammar:
  /// `doc:<uuid>[/<embedded_path>]|<label>` or `token:<uuid>|<label>` — the id/path is
  /// everything before the FIRST `|`, split from an optional `/<embedded_path>` at the first `/`
  /// after the `doc:`/`token:` prefix; the label is everything after that `|`, trimmed, and must
  /// be non-empty (`Segment::DocLink.label` is a required field, unlike `Button`'s optional
  /// label).
  fn parse_doc_link(content: &str) -> Result<Option<BodyChunk<'_>>, ()> {
      if let Some(rest) = content.strip_prefix("doc:") {
          let (id_and_path, label) = rest.split_once('|').ok_or(())?;
          let label = label.trim();
          if label.is_empty() {
              return Err(());
          }
          let (id_part, embedded_path) = match id_and_path.split_once('/') {
              Some((id, p)) => (id, Some(format!("/{p}"))),
              None => (id_and_path, None),
          };
          let doc_id = Uuid::parse_str(id_part).map_err(|_| ())?;
          return Ok(Some(BodyChunk::DocLink {
              target: DocLinkTarget::Doc {
                  doc_id,
                  embedded_path,
              },
              label,
          }));
      }
      if let Some(rest) = content.strip_prefix("token:") {
          let (id_part, label) = rest.split_once('|').ok_or(())?;
          let label = label.trim();
          if label.is_empty() {
              return Err(());
          }
          let token_id = Uuid::parse_str(id_part).map_err(|_| ())?;
          return Ok(Some(BodyChunk::DocLink {
              target: DocLinkTarget::Token { token_id },
              label,
          }));
      }
      Ok(None)
  }
  ```

- [ ] **Step 6: Add `RollError::MalformedDocLink` and its `Display` arm.**

  Find:
  ```rust
      /// Two ladder rungs share one `margin_offset` -- `classify`'s
      /// max_by_key/min_by_key tie is caller-order-dependent, so which rung wins
      /// would be nondeterministic. Refused at construction so every downstream
      /// ladder is unambiguous (`dice::eval::classify`'s doc comment documents the tie).
      DuplicateTierOffset(i32),
  }
  ```

  Replace with:
  ```rust
      /// Two ladder rungs share one `margin_offset` -- `classify`'s
      /// max_by_key/min_by_key tie is caller-order-dependent, so which rung wins
      /// would be nondeterministic. Refused at construction so every downstream
      /// ladder is unambiguous (`dice::eval::classify`'s doc comment documents the tie).
      DuplicateTierOffset(i32),
      /// A `[[doc:...]]`/`[[token:...]]` span recognized by its prefix but malformed: an
      /// unparseable id, or a missing/empty `|<label>` suffix.
      MalformedDocLink,
  }
  ```

  Find:
  ```rust
              RollError::DuplicateTierOffset(o) => {
                  write!(f, "duplicate tier margin offset {o}")
              }
          }
      }
  }
  ```

  Replace with:
  ```rust
              RollError::DuplicateTierOffset(o) => {
                  write!(f, "duplicate tier margin offset {o}")
              }
              RollError::MalformedDocLink => {
                  write!(f, "that document/token link is malformed")
              }
          }
      }
  }
  ```

- [ ] **Step 7: Add unit tests.** In `src/server/src/chat/rolls.rs`'s `#[cfg(test)] mod tests`,
  after `scan_unterminated_span_errors`, add:
  ```rust
      #[test]
      fn scan_doc_link() {
          let chunks =
              scan_body("[[doc:00000000-0000-0000-0000-000000000001|My Document]]").unwrap();
          assert_eq!(
              chunks,
              vec![BodyChunk::DocLink {
                  target: DocLinkTarget::Doc {
                      doc_id: Uuid::from_u128(1),
                      embedded_path: None,
                  },
                  label: "My Document",
              }]
          );
      }

      #[test]
      fn scan_doc_link_with_embedded_path() {
          let chunks = scan_body(
              "[[doc:00000000-0000-0000-0000-000000000001/embedded/actor/0|My Item]]",
          )
          .unwrap();
          assert_eq!(
              chunks,
              vec![BodyChunk::DocLink {
                  target: DocLinkTarget::Doc {
                      doc_id: Uuid::from_u128(1),
                      embedded_path: Some("/embedded/actor/0".into()),
                  },
                  label: "My Item",
              }]
          );
      }

      #[test]
      fn scan_token_link() {
          let chunks =
              scan_body("[[token:00000000-0000-0000-0000-000000000002|Goblin]]").unwrap();
          assert_eq!(
              chunks,
              vec![BodyChunk::DocLink {
                  target: DocLinkTarget::Token {
                      token_id: Uuid::from_u128(2),
                  },
                  label: "Goblin",
              }]
          );
      }

      #[test]
      fn scan_doc_link_with_surrounding_text() {
          let chunks =
              scan_body("see [[doc:00000000-0000-0000-0000-000000000001|Doc]] please").unwrap();
          assert_eq!(
              chunks,
              vec![
                  BodyChunk::Text("see "),
                  BodyChunk::DocLink {
                      target: DocLinkTarget::Doc {
                          doc_id: Uuid::from_u128(1),
                          embedded_path: None,
                      },
                      label: "Doc",
                  },
                  BodyChunk::Text(" please"),
              ]
          );
      }

      #[test]
      fn scan_doc_link_missing_label_is_malformed() {
          assert_eq!(
              scan_body("[[doc:00000000-0000-0000-0000-000000000001]]"),
              Err(RollError::MalformedDocLink)
          );
      }

      #[test]
      fn scan_doc_link_empty_label_is_malformed() {
          assert_eq!(
              scan_body("[[doc:00000000-0000-0000-0000-000000000001|   ]]"),
              Err(RollError::MalformedDocLink)
          );
      }

      #[test]
      fn scan_doc_link_bad_uuid_is_malformed() {
          assert_eq!(
              scan_body("[[doc:not-a-uuid|Label]]"),
              Err(RollError::MalformedDocLink)
          );
      }

      #[test]
      fn scan_token_link_missing_label_is_malformed() {
          assert_eq!(
              scan_body("[[token:00000000-0000-0000-0000-000000000002]]"),
              Err(RollError::MalformedDocLink)
          );
      }

      #[test]
      fn scan_doc_link_counts_toward_max_inline_rolls() {
          let body = "[[doc:00000000-0000-0000-0000-000000000001|D]] ".repeat(MAX_INLINE_ROLLS);
          assert!(scan_body(&body).is_ok());
          let over =
              "[[doc:00000000-0000-0000-0000-000000000001|D]] ".repeat(MAX_INLINE_ROLLS + 1);
          assert!(matches!(scan_body(&over), Err(RollError::TooManyInline(_))));
      }
  ```

  Then extend the existing exhaustive-display test. Find:
  ```rust
      #[test]
      fn roll_error_display_has_no_debug_artifacts() {
          let variants = vec![
              RollError::Parse(ParseError::Empty),
              RollError::TooManyDice(200),
              RollError::TooManyRecords(2000),
              RollError::ExpertiseTooLarge(200),
              RollError::SidesTooLarge(20_000),
              RollError::TooManyInline(9),
              RollError::Unterminated,
              RollError::DuplicateTierOffset(5),
          ];
  ```

  Replace with:
  ```rust
      #[test]
      fn roll_error_display_has_no_debug_artifacts() {
          let variants = vec![
              RollError::Parse(ParseError::Empty),
              RollError::TooManyDice(200),
              RollError::TooManyRecords(2000),
              RollError::ExpertiseTooLarge(200),
              RollError::SidesTooLarge(20_000),
              RollError::TooManyInline(9),
              RollError::Unterminated,
              RollError::DuplicateTierOffset(5),
              RollError::MalformedDocLink,
          ];
  ```

- [ ] **Step 8: Run the gate.** From `src/server/`: `cargo fmt --all -- --check`; `cargo clippy
  --all-targets -- -D warnings`; `cargo test --all -- chat::rolls`. Fix forward on any failure
  before proceeding (a compile error in `chat::mod.rs`'s `Segment` enum from an unrelated
  exhaustive match — e.g. `sanitize.rs`'s test helper — is EXPECTED at this point and is fixed in
  Task 2; do not touch `sanitize.rs` in this task).

---

## Task 2: Server — wire `DocLink` into `handle_send_message`'s ingest, fix the exhaustive-match compile site, add ingest tests

**Files:**
- Modify: `src/server/src/chat/mod.rs` (`handle_send_message`'s per-chunk ingest match)
- Modify: `src/server/src/chat/sanitize.rs` (test-only exhaustive match over `Segment`)
- Test: `src/server/src/chat/mod.rs`'s `#[cfg(test)] mod tests`

**Interfaces:**
- Consumes: Task 1's `Segment::DocLink`, `chat::rolls::BodyChunk::DocLink`.
- Produces: no new public symbols — wiring only.

### Steps

- [ ] **Step 1: Wire the ingest arm.** In `src/server/src/chat/mod.rs`, inside
  `handle_send_message`'s per-chunk `match chunk { ... }` block, find:
  ```rust
                      rolls::BodyChunk::Button { formula, label } => {
                          if dice_ctx.is_none() {
                              dice_ctx = Some(resolve_dice_context(repo, room.world_id).await);
                          }
                          // Stored/validated formula is trimmed — the `roll:`/`|`
                          // split leaves incidental whitespace (e.g.
                          // "[[roll: 1d20|Attack]]") that must not survive into
                          // the button's stored formula or the click-to-send text.
                          let formula = formula.trim();
                          match rolls::validate_formula(formula, dice_ctx.unwrap()) {
                              Ok(()) => segments.push(Segment::RollButton {
                                  formula: formula.to_string(),
                                  label: label.map(|s| s.to_string()),
                              }),
                              Err(e) => {
                                  roll_err = Some(e);
                                  break;
                              }
                          }
                      }
                  }
              }
  ```

  Replace with:
  ```rust
                      rolls::BodyChunk::Button { formula, label } => {
                          if dice_ctx.is_none() {
                              dice_ctx = Some(resolve_dice_context(repo, room.world_id).await);
                          }
                          // Stored/validated formula is trimmed — the `roll:`/`|`
                          // split leaves incidental whitespace (e.g.
                          // "[[roll: 1d20|Attack]]") that must not survive into
                          // the button's stored formula or the click-to-send text.
                          let formula = formula.trim();
                          match rolls::validate_formula(formula, dice_ctx.unwrap()) {
                              Ok(()) => segments.push(Segment::RollButton {
                                  formula: formula.to_string(),
                                  label: label.map(|s| s.to_string()),
                              }),
                              Err(e) => {
                                  roll_err = Some(e);
                                  break;
                              }
                          }
                      }
                      rolls::BodyChunk::DocLink { target, label } => {
                          segments.push(Segment::DocLink {
                              target,
                              label: label.to_string(),
                          });
                      }
                  }
              }
  ```

- [ ] **Step 2: Fix `sanitize.rs`'s test-only exhaustive match over `Segment`.** Find:
  ```rust
      fn render(segs: &[Segment]) -> String {
          segs.iter()
              .map(|s| match s {
                  Segment::Text { text } => text.clone(),
                  Segment::Html { sanitized_html } => sanitized_html.clone(),
                  // `sanitize()` (the function under test) never produces a
                  // roll or link-preview segment -- those are `chat::rolls`'s
                  // and `chat::link_preview::enrich`'s own producers.
                  Segment::RollEmbed { .. }
                  | Segment::RollButton { .. }
                  | Segment::LinkPreview { .. } => {
                      unreachable!("sanitize() never produces roll or preview segments")
                  }
              })
              .collect()
      }
  ```

  Replace with:
  ```rust
      fn render(segs: &[Segment]) -> String {
          segs.iter()
              .map(|s| match s {
                  Segment::Text { text } => text.clone(),
                  Segment::Html { sanitized_html } => sanitized_html.clone(),
                  // `sanitize()` (the function under test) never produces a roll,
                  // link-preview, or doc-link segment -- those are `chat::rolls`'s
                  // and `chat::link_preview::enrich`'s own producers.
                  Segment::RollEmbed { .. }
                  | Segment::RollButton { .. }
                  | Segment::LinkPreview { .. }
                  | Segment::DocLink { .. } => {
                      unreachable!("sanitize() never produces roll, preview, or doc-link segments")
                  }
              })
              .collect()
      }
  ```

- [ ] **Step 3: Add ingest tests to `src/server/src/chat/mod.rs`'s test module.** These reuse the
  existing `seed_actor_doc`-style harness (`SqliteRepository::connect("sqlite::memory:")`,
  `RoomRegistry`, `PermissionContext`, `handle_send_message`). Add near the roll-stage tests:
  ```rust
      #[tokio::test]
      async fn send_message_stores_a_doc_link_segment() {
          use crate::auth::role::ServerRole;
          use crate::data::document::WorldRole;
          use crate::data::sqlite::SqliteRepository;
          use crate::ws::room::RoomRegistry;

          let repo = SqliteRepository::connect("sqlite::memory:").await.unwrap();
          let gm = repo
              .create_user("gm", None, ServerRole::User, 0)
              .await
              .unwrap();
          let player = repo
              .create_user("pl", None, ServerRole::User, 0)
              .await
              .unwrap();
          let w = repo.create_world_owned("W", gm, 0).await.unwrap();
          repo.add_member(w.id, player, WorldRole::Player)
              .await
              .unwrap();

          let ctx = PermissionContext {
              user_id: player,
              world_role: WorldRole::Player,
          };
          let reg = RoomRegistry::new();
          let room = reg.get_or_create(&repo, w.id).await.unwrap().unwrap();
          let rate = PingRateLimiter::new();
          let target_id = Uuid::new_v4();

          let cmd = handle_send_message(
              MessageRequestCtx {
                  room: &room,
                  repo: &repo,
                  ctx: &ctx,
                  rate: &rate,
                  preview: LinkPreviewDeps {
                      client: &super::link_preview::build_client_allow_loopback(),
                      cache: &LinkPreviewCache::new(),
                      rate: &PreviewRateLimiter::new(),
                  },
                  now: 0,
                  budget_per_min: 30,
              },
              "all".into(),
              format!("see [[doc:{target_id}|My Doc]] please"),
              None,
              Audience::Public,
          )
          .await
          .unwrap();
          let doc = match &cmd.ops[0] {
              Operation::Create { doc } => doc,
              other => panic!("expected Create, got {other:?}"),
          };
          let sys: MessageEngine = serde_json::from_value(doc.engine.clone().unwrap()).unwrap();
          assert_eq!(
              sys.content,
              vec![
                  Segment::Text {
                      text: "see ".into()
                  },
                  Segment::DocLink {
                      target: DocLinkTarget::Doc {
                          doc_id: target_id,
                          embedded_path: None,
                      },
                      label: "My Doc".into(),
                  },
                  Segment::Text {
                      text: " please".into()
                  },
              ]
          );
      }

      #[tokio::test]
      async fn send_message_stores_a_token_link_segment() {
          use crate::auth::role::ServerRole;
          use crate::data::document::WorldRole;
          use crate::data::sqlite::SqliteRepository;
          use crate::ws::room::RoomRegistry;

          let repo = SqliteRepository::connect("sqlite::memory:").await.unwrap();
          let gm = repo
              .create_user("gm", None, ServerRole::User, 0)
              .await
              .unwrap();
          let w = repo.create_world_owned("W", gm, 0).await.unwrap();
          let ctx = PermissionContext {
              user_id: gm,
              world_role: WorldRole::Gm,
          };
          let reg = RoomRegistry::new();
          let room = reg.get_or_create(&repo, w.id).await.unwrap().unwrap();
          let rate = PingRateLimiter::new();
          let token_id = Uuid::new_v4();

          let cmd = handle_send_message(
              MessageRequestCtx {
                  room: &room,
                  repo: &repo,
                  ctx: &ctx,
                  rate: &rate,
                  preview: LinkPreviewDeps {
                      client: &super::link_preview::build_client_allow_loopback(),
                      cache: &LinkPreviewCache::new(),
                      rate: &PreviewRateLimiter::new(),
                  },
                  now: 0,
                  budget_per_min: 30,
              },
              "all".into(),
              format!("[[token:{token_id}|Goblin]]"),
              None,
              Audience::Public,
          )
          .await
          .unwrap();
          let doc = match &cmd.ops[0] {
              Operation::Create { doc } => doc,
              other => panic!("expected Create, got {other:?}"),
          };
          let sys: MessageEngine = serde_json::from_value(doc.engine.clone().unwrap()).unwrap();
          assert_eq!(
              sys.content,
              vec![Segment::DocLink {
                  target: DocLinkTarget::Token { token_id },
                  label: "Goblin".into(),
              }]
          );
      }

      #[tokio::test]
      async fn send_message_with_a_dangling_doc_link_target_still_stores_it_unvalidated() {
          use crate::auth::role::ServerRole;
          use crate::data::document::WorldRole;
          use crate::data::sqlite::SqliteRepository;
          use crate::ws::room::RoomRegistry;

          // A.4: no server-side existence check runs against `DocLink`'s target at ingest — a
          // reference to a document that does not exist (or the sender cannot see) is stored
          // verbatim; only the client's render-time `ctx.documents` presence check gates it.
          let repo = SqliteRepository::connect("sqlite::memory:").await.unwrap();
          let gm = repo
              .create_user("gm", None, ServerRole::User, 0)
              .await
              .unwrap();
          let w = repo.create_world_owned("W", gm, 0).await.unwrap();
          let ctx = PermissionContext {
              user_id: gm,
              world_role: WorldRole::Gm,
          };
          let reg = RoomRegistry::new();
          let room = reg.get_or_create(&repo, w.id).await.unwrap().unwrap();
          let rate = PingRateLimiter::new();
          let nonexistent = Uuid::new_v4();

          let cmd = handle_send_message(
              MessageRequestCtx {
                  room: &room,
                  repo: &repo,
                  ctx: &ctx,
                  rate: &rate,
                  preview: LinkPreviewDeps {
                      client: &super::link_preview::build_client_allow_loopback(),
                      cache: &LinkPreviewCache::new(),
                      rate: &PreviewRateLimiter::new(),
                  },
                  now: 0,
                  budget_per_min: 30,
              },
              "all".into(),
              format!("[[doc:{nonexistent}|Ghost Doc]]"),
              None,
              Audience::Public,
          )
          .await
          .unwrap();
          let doc = match &cmd.ops[0] {
              Operation::Create { doc } => doc,
              other => panic!("expected Create, got {other:?}"),
          };
          let sys: MessageEngine = serde_json::from_value(doc.engine.clone().unwrap()).unwrap();
          assert_eq!(
              sys.content,
              vec![Segment::DocLink {
                  target: DocLinkTarget::Doc {
                      doc_id: nonexistent,
                      embedded_path: None,
                  },
                  label: "Ghost Doc".into(),
              }]
          );
      }

      #[tokio::test]
      async fn send_message_rejects_a_malformed_doc_link_and_authors_no_message() {
          use crate::auth::role::ServerRole;
          use crate::data::document::WorldRole;
          use crate::data::sqlite::SqliteRepository;
          use crate::ws::room::RoomRegistry;

          let repo = SqliteRepository::connect("sqlite::memory:").await.unwrap();
          let gm = repo
              .create_user("gm", None, ServerRole::User, 0)
              .await
              .unwrap();
          let w = repo.create_world_owned("W", gm, 0).await.unwrap();
          let ctx = PermissionContext {
              user_id: gm,
              world_role: WorldRole::Gm,
          };
          let reg = RoomRegistry::new();
          let room = reg.get_or_create(&repo, w.id).await.unwrap().unwrap();
          let rate = PingRateLimiter::new();
          let seq_before = repo.events_since(w.id, 0).await.unwrap().len();

          let cmd = handle_send_message(
              MessageRequestCtx {
                  room: &room,
                  repo: &repo,
                  ctx: &ctx,
                  rate: &rate,
                  preview: LinkPreviewDeps {
                      client: &super::link_preview::build_client_allow_loopback(),
                      cache: &LinkPreviewCache::new(),
                      rate: &PreviewRateLimiter::new(),
                  },
                  now: 0,
                  budget_per_min: 30,
              },
              "all".into(),
              "[[doc:not-a-uuid]]".into(),
              None,
              Audience::Public,
          )
          .await
          .unwrap();
          // A malformed doc-link, like any other roll-stage failure, authors ONE whispered
          // System notice instead of the intended message — never both, never neither.
          let doc = match &cmd.ops[0] {
              Operation::Create { doc } => doc,
              other => panic!("expected Create, got {other:?}"),
          };
          assert_eq!(doc.doc_type, MESSAGE_DOC_TYPE);
          let sys: MessageEngine = serde_json::from_value(doc.engine.clone().unwrap()).unwrap();
          assert_eq!(sys.kind, MessageKind::System);
          assert_eq!(
              repo.events_since(w.id, 0).await.unwrap().len(),
              seq_before + 1,
              "exactly one event (the System notice) authored, not the intended message"
          );
      }
  ```

- [ ] **Step 4: Run the gate.** From `src/server/`: `cargo fmt --all -- --check`; `cargo clippy
  --all-targets -- -D warnings`; `cargo test --all`.

---

## Task 3: Client — `chat-docs.ts`'s `DocLinkTarget`/`ChatSegment` mirror

**Files:**
- Modify: `src/client/core/src/chat-docs.ts`
- Modify: `src/client/core/src/index.ts` (export the new type/schema)
- Test: `src/client/core/src/chat-docs.test.ts`

**Interfaces:**
- Produces: `DocLinkTarget` (TS type, discriminated union `{kind:"doc", doc_id, embedded_path?} |
  {kind:"token", token_id}`), `DocLinkTargetSchema` (Zod).
- Produces: `ChatSegment`'s new `{kind:"doc_link", target: DocLinkTarget, label: string}` member;
  `isKnownSegment` recognizes `"doc_link"`.
- Consumed by: Task 4 (`MessageCard.svelte`), Task 5 (`Composer.svelte`, for the span the picker
  builds — no import needed there since the composer builds raw text, but the picker's search
  results are typed via `WireSearchHit`/`WireDocument`, already exported).

### Steps

- [ ] **Step 1: Add `DocLinkTarget` + its schema, and extend `ChatSegment`.** In
  `src/client/core/src/chat-docs.ts`, find:
  ```ts
  /** One piece of a message's sanitized content model — one of the five known segment
   * kinds. Mirrors `chat::Segment`. `html.sanitized_html` is innerHTML-safe ONLY because
  ```

  Replace with:
  ```ts
  /** What a `doc_link` segment points at. Mirrors `chat::DocLinkTarget`, itself modeled on the
   * client's own `SheetRef` shape. */
  export type DocLinkTarget =
    | {
        /** A top-level document. */
        kind: "doc";
        /** The top-level document's id. */
        doc_id: string;
        /** A `/embedded/<collection>/<index>` pointer, one level deep, or absent for the
         * top-level document itself. */
        embedded_path?: string | null;
      }
    | {
        /** A placed token, resolved via its linked/embedded actor. */
        kind: "token";
        /** The placed token's document id. */
        token_id: string;
      };

  // Unannotated impl const — see `dieRecordSchemaImpl`'s note above.
  export const docLinkTargetSchemaImpl = z.discriminatedUnion("kind", [
    z.object({ kind: z.literal("doc"), doc_id: z.string(), embedded_path: z.string().nullish() }),
    z.object({ kind: z.literal("token"), token_id: z.string() }),
  ]);
  /** Validator for a `DocLinkTarget`. */
  export const DocLinkTargetSchema: z.ZodType<DocLinkTarget> = docLinkTargetSchemaImpl;

  /** One piece of a message's sanitized content model — one of the six known segment
   * kinds. Mirrors `chat::Segment`. `html.sanitized_html` is innerHTML-safe ONLY because
  ```

  Find:
  ```ts
    | {
        /** A server-fetched, SSRF-guarded preview of a link in the message. */
        kind: "link_preview";
        /** The previewed URL as posted. */
        url: string;
        /** Server-extracted title. */
        title: string;
        /** Server-extracted description (may be empty). */
        description: string;
      };

  // Unannotated impl const — see `dieRecordSchemaImpl`'s note above.
  // The union-narrowing case this pattern guards against is exactly what a segment-kind arm
  // removal or a `kind: z.literal(...)` narrowing inside one would be.
  export const chatSegmentSchemaImpl = z.discriminatedUnion("kind", [
    z.object({ kind: z.literal("text"), text: z.string() }),
    z.object({ kind: z.literal("html"), sanitized_html: z.string() }),
    z.object({ kind: z.literal("roll_embed"), formula: z.string(), outcome: RollOutcomeSchema }),
    z.object({ kind: z.literal("roll_button"), formula: z.string(), label: z.string().nullish() }),
    z.object({ kind: z.literal("link_preview"), url: z.string(), title: z.string(), description: z.string() }),
  ]);
  ```

  Replace with:
  ```ts
    | {
        /** A server-fetched, SSRF-guarded preview of a link in the message. */
        kind: "link_preview";
        /** The previewed URL as posted. */
        url: string;
        /** Server-extracted title. */
        title: string;
        /** Server-extracted description (may be empty). */
        description: string;
      }
    | {
        /** A free-form, author-inserted link to a document or placed token. `label` is the
         * display text captured at authoring time — never re-resolved at render, only the
         * fail-closed presence gate against `ctx.documents` re-checks `target`. */
        kind: "doc_link";
        /** What the link points at. */
        target: DocLinkTarget;
        /** Display text captured at authoring time. */
        label: string;
      };

  // Unannotated impl const — see `dieRecordSchemaImpl`'s note above.
  // The union-narrowing case this pattern guards against is exactly what a segment-kind arm
  // removal or a `kind: z.literal(...)` narrowing inside one would be.
  export const chatSegmentSchemaImpl = z.discriminatedUnion("kind", [
    z.object({ kind: z.literal("text"), text: z.string() }),
    z.object({ kind: z.literal("html"), sanitized_html: z.string() }),
    z.object({ kind: z.literal("roll_embed"), formula: z.string(), outcome: RollOutcomeSchema }),
    z.object({ kind: z.literal("roll_button"), formula: z.string(), label: z.string().nullish() }),
    z.object({ kind: z.literal("link_preview"), url: z.string(), title: z.string(), description: z.string() }),
    z.object({ kind: z.literal("doc_link"), target: DocLinkTargetSchema, label: z.string() }),
  ]);
  ```

- [ ] **Step 2: Extend `isKnownSegment` and `UnknownSegmentSchema`'s refine + doc comment.** Find:
  ```ts
  /** Forward-compat: a segment kind this client doesn't know (e.g. a future server's
   * DocLink) parses as opaque and renders as nothing — the message still shows.
   * INVARIANT: refuses every KNOWN kind — without this, a malformed
   * text/html/roll_embed/roll_button/link_preview segment (missing/wrong-typed
   * payload) would be rescued by this fallback and then misclassified as
   * trustworthy by isKnownSegment, breaking fail-closed. */
  const UnknownSegmentSchema = z
    .object({ kind: z.string() })
    .passthrough()
    .refine(
      (s) =>
        s.kind !== "text" &&
        s.kind !== "html" &&
        s.kind !== "roll_embed" &&
        s.kind !== "roll_button" &&
        s.kind !== "link_preview",
    );
  ```

  Replace with:
  ```ts
  /** Forward-compat: a segment kind this client doesn't know (e.g. a future server-added kind)
   * parses as opaque and renders as nothing — the message still shows.
   * INVARIANT: refuses every KNOWN kind — without this, a malformed
   * text/html/roll_embed/roll_button/link_preview/doc_link segment (missing/wrong-typed
   * payload) would be rescued by this fallback and then misclassified as
   * trustworthy by isKnownSegment, breaking fail-closed. */
  const UnknownSegmentSchema = z
    .object({ kind: z.string() })
    .passthrough()
    .refine(
      (s) =>
        s.kind !== "text" &&
        s.kind !== "html" &&
        s.kind !== "roll_embed" &&
        s.kind !== "roll_button" &&
        s.kind !== "link_preview" &&
        s.kind !== "doc_link",
    );
  ```

  Find:
  ```ts
  export function isKnownSegment(s: ChatSegment | UnknownSegment): s is ChatSegment {
    return (
      s.kind === "text" ||
      s.kind === "html" ||
      s.kind === "roll_embed" ||
      s.kind === "roll_button" ||
      s.kind === "link_preview"
    );
  }
  ```

  Replace with:
  ```ts
  export function isKnownSegment(s: ChatSegment | UnknownSegment): s is ChatSegment {
    return (
      s.kind === "text" ||
      s.kind === "html" ||
      s.kind === "roll_embed" ||
      s.kind === "roll_button" ||
      s.kind === "link_preview" ||
      s.kind === "doc_link"
    );
  }
  ```

- [ ] **Step 3: Export from `src/client/core/src/index.ts`.** Find:
  ```ts
  export { MESSAGE_DOC_TYPE, CHANNEL_REGISTRY_DOC_TYPE, DICE_SETTINGS_DOC_TYPE, CHAT_SETTINGS_DOC_TYPE, MAX_MESSAGE_CHARS, MessageKindSchema, DieRecordSchema, RollOutcomeSchema, ChatSegmentSchema, ChatMessageEngineSchema, parseMessageEngine, isKnownSegment, buildChannelRegistryDoc, buildDiceSettingsDoc, buildChatSettingsDoc } from "./chat-docs";
  export type { MessageKind, DieRecord, RollOutcome, ChatSegment, UnknownSegment, ChatMessageEngine, ChannelRegistryEngine, ChatSettingsEngine, DiceSettingsEngine, ConstTerm } from "./chat-docs";
  ```

  Replace with:
  ```ts
  export { MESSAGE_DOC_TYPE, CHANNEL_REGISTRY_DOC_TYPE, DICE_SETTINGS_DOC_TYPE, CHAT_SETTINGS_DOC_TYPE, MAX_MESSAGE_CHARS, MessageKindSchema, DieRecordSchema, RollOutcomeSchema, DocLinkTargetSchema, ChatSegmentSchema, ChatMessageEngineSchema, parseMessageEngine, isKnownSegment, buildChannelRegistryDoc, buildDiceSettingsDoc, buildChatSettingsDoc } from "./chat-docs";
  export type { MessageKind, DieRecord, RollOutcome, DocLinkTarget, ChatSegment, UnknownSegment, ChatMessageEngine, ChannelRegistryEngine, ChatSettingsEngine, DiceSettingsEngine, ConstTerm } from "./chat-docs";
  ```

- [ ] **Step 4: Add tests to `src/client/core/src/chat-docs.test.ts`.** After the `link_preview
  segments` `describe` block, add:
  ```ts
  describe("doc_link segments", () => {
    test("parses a doc_link segment pointing at a top-level document", () => {
      const eng = parseMessageEngine(msgDoc({
        ...base,
        content: [{ kind: "doc_link", target: { kind: "doc", doc_id: "d1" }, label: "My Doc" }],
      }));
      expect(eng).not.toBeNull();
      expect(eng!.content).toEqual([
        { kind: "doc_link", target: { kind: "doc", doc_id: "d1" }, label: "My Doc" },
      ]);
    });
    test("parses a doc_link segment with an embedded_path", () => {
      const eng = parseMessageEngine(msgDoc({
        ...base,
        content: [{ kind: "doc_link", target: { kind: "doc", doc_id: "d1", embedded_path: "/embedded/actor/0" }, label: "Item" }],
      }));
      expect(eng).not.toBeNull();
      expect(eng!.content).toEqual([
        { kind: "doc_link", target: { kind: "doc", doc_id: "d1", embedded_path: "/embedded/actor/0" }, label: "Item" },
      ]);
    });
    test("parses a doc_link segment pointing at a token", () => {
      const eng = parseMessageEngine(msgDoc({
        ...base,
        content: [{ kind: "doc_link", target: { kind: "token", token_id: "t1" }, label: "Goblin" }],
      }));
      expect(eng).not.toBeNull();
      expect(eng!.content).toEqual([
        { kind: "doc_link", target: { kind: "token", token_id: "t1" }, label: "Goblin" },
      ]);
    });
    test("fail-closed: doc_link missing label fails the whole message parse", () => {
      expect(parseMessageEngine(msgDoc({
        ...base,
        content: [{ kind: "doc_link", target: { kind: "doc", doc_id: "d1" } }],
      }))).toBeNull();
    });
    test("fail-closed: doc_link with an unrecognized target kind fails the whole message parse", () => {
      expect(parseMessageEngine(msgDoc({
        ...base,
        content: [{ kind: "doc_link", target: { kind: "scene", scene_id: "s1" }, label: "x" }],
      }))).toBeNull();
    });
    test("unknown segment kinds are still opaque alongside known doc_link kinds", () => {
      const eng = parseMessageEngine(msgDoc({
        ...base,
        content: [
          { kind: "doc_link", target: { kind: "doc", doc_id: "d1" }, label: "My Doc" },
          { kind: "preview_card", url: "https://example.com/b" },
        ],
      }));
      expect(eng).not.toBeNull();
      expect(eng!.content).toHaveLength(2);
      expect(eng!.content.filter(isKnownSegment)).toEqual([
        { kind: "doc_link", target: { kind: "doc", doc_id: "d1" }, label: "My Doc" },
      ]);
    });
  });
  ```

  Also add `DocLinkTargetSchema` and `type DocLinkTarget` to this file's existing import block
  from `"./chat-docs"` if any test needs to reference the schema directly (the tests above use
  `parseMessageEngine`/`isKnownSegment` only, already imported — no import changes needed unless a
  chosen assertion references `DocLinkTargetSchema` directly).

- [ ] **Step 5: Run the gate.** From the repo root: `pnpm --filter @shadowcat/core typecheck`;
  `pnpm --filter @shadowcat/core test`.

---

## Task 4: Client — `MessageCard.svelte` renders `doc_link` segments

**Files:**
- Modify: `src/modules/chat-card/src/MessageCard.svelte`
- Test: `src/modules/chat-card/src/MessageCard.test.ts`

**Interfaces:**
- Consumes: Task 3's `DocLinkTarget`, `ChatSegment`'s `doc_link` member.
- Produces: no new exported symbols — a new render arm + one private helper
  (`docLinkOpenRef`).

### Steps

- [ ] **Step 1: Import `DocLinkTarget`.** Find:
  ```ts
  import {
    parseMessageEngine,
    isKnownSegment,
    resolveTokenActor,
    actorDisplayName,
    type ChatSegment,
    type UnknownSegment,
    type WireActorOwnerRef,
    type WireDocument,
  } from "@shadowcat/core";
  ```

  Replace with:
  ```ts
  import {
    parseMessageEngine,
    isKnownSegment,
    resolveTokenActor,
    actorDisplayName,
    type ChatSegment,
    type UnknownSegment,
    type WireActorOwnerRef,
    type WireDocument,
    type DocLinkTarget,
    type SheetRef,
  } from "@shadowcat/core";
  ```

- [ ] **Step 2: Add `docLinkOpenRef`, mirroring `actorOpenRef`'s presence-gate pattern verbatim.**
  Find:
  ```ts
    /** Host caption for a `link_preview` card. Never throws on a malformed `url` — a preview is
  ```

  Replace with:
  ```ts
    /** Resolves a `doc_link` segment's target to an openable `SheetRef` when the referenced
     * document/token is present in the per-recipient OPTIMISTIC store — the identical
     * presence-gate `actorOpenRef` (above) already applies to actor attribution, reused here
     * verbatim rather than reimplemented: presence implies READ (server-side redaction withholds
     * an unauthorized doc from `base` entirely), and absence renders inert plain text.
     * @param target The `doc_link` segment's target.
     * @returns The resolved `SheetRef`, or `null` if the target isn't present in the store.
     * @example
     * ```
     * // internal; call sites use the derived per-segment resolution in the template
     * declare const target: DocLinkTarget;
     * docLinkOpenRef(target);
     * ```
     */
    function docLinkOpenRef(target: DocLinkTarget): SheetRef | null {
      if (target.kind === "doc") {
        return ctx.documents.get(target.doc_id)
          ? { docId: target.doc_id, embeddedPath: target.embedded_path ?? undefined }
          : null;
      }
      return ctx.documents.get(target.token_id) ? { tokenId: target.token_id } : null;
    }

    /** Host caption for a `link_preview` card. Never throws on a malformed `url` — a preview is
  ```

- [ ] **Step 3: Add the render arm.** Find:
  ```svelte
                {:else if s.kind === "link_preview"}
                  <!-- Server-fetched preview (SSRF-guarded). The client NEVER fetches
                  `s.url` or any remote resource — only stored title/description/url strings are
                  rendered, all as escaped text. No <img>: an <img src> would make the viewer's
                  browser fetch a remote resource, leaking their IP to a URL an attacker chose. -->
                  <a
                    class="link-preview"
                    href={safeHref(s.url)}
                    target="_blank"
                    rel="noopener noreferrer nofollow"
                  >
                    <span class="link-preview-title">{s.title}</span>
                    <span class="link-preview-description">{s.description}</span>
                    <span class="link-preview-host">{hostOf(s.url)}</span>
                  </a>
                {/if}
  ```

  Replace with:
  ```svelte
                {:else if s.kind === "link_preview"}
                  <!-- Server-fetched preview (SSRF-guarded). The client NEVER fetches
                  `s.url` or any remote resource — only stored title/description/url strings are
                  rendered, all as escaped text. No <img>: an <img src> would make the viewer's
                  browser fetch a remote resource, leaking their IP to a URL an attacker chose. -->
                  <a
                    class="link-preview"
                    href={safeHref(s.url)}
                    target="_blank"
                    rel="noopener noreferrer nofollow"
                  >
                    <span class="link-preview-title">{s.title}</span>
                    <span class="link-preview-description">{s.description}</span>
                    <span class="link-preview-host">{hostOf(s.url)}</span>
                  </a>
                {:else if s.kind === "doc_link"}
                  {@const ref = docLinkOpenRef(s.target)}
                  {#if ref}
                    <button type="button" class="doc-link" onclick={() => ctx.openDocument(ref)}>{s.label}</button>
                  {:else}
                    <span class="seg-text">{s.label}</span>
                  {/if}
                {/if}
  ```

- [ ] **Step 4: Add CSS.** Find:
  ```scss
    .link-preview-host {
      font-size: 0.85em;
      opacity: 0.6;
    }
  </style>
  ```

  Replace with:
  ```scss
    .link-preview-host {
      font-size: 0.85em;
      opacity: 0.6;
    }
    .doc-link {
      background: none;
      border: none;
      padding: 0;
      color: var(--accent);
      cursor: pointer;
      font: inherit;
      text-decoration: underline;
    }
    .doc-link:focus-visible {
      outline: 2px solid var(--accent);
      outline-offset: 1px;
    }
  </style>
  ```

- [ ] **Step 5: Add tests to `src/modules/chat-card/src/MessageCard.test.ts`.** Following the
  existing `describe`/`it` conventions in this file (see the `actor_owner` tests around line 797
  for the exact `storeWith`/`setAppContextForTest` pattern), add a new `describe` block:
  ```ts
  describe("doc_link segments", () => {
    it("renders a resolvable doc_link as a clickable button using the stored label", () => {
      const target = msgDoc("doc1", { channel: "g" });
      const doc = msgDoc("m1", baseSystem({
        content: [{ kind: "doc_link", target: { kind: "doc", doc_id: "doc1" }, label: "My Doc" }],
      }));
      const opened: unknown[] = [];
      render(MessageCard, {
        props: { message: doc, showChannel: false },
        context: setAppContextForTest({ documents: storeWith(target, doc), openDocument: (r) => opened.push(r) }),
      });
      fireEvent.click(screen.getByText("My Doc"));
      expect(opened).toEqual([{ docId: "doc1", embeddedPath: undefined }]);
    });

    it("renders a resolvable doc_link with an embedded_path, passed through to openDocument", () => {
      const target = msgDoc("doc1", { channel: "g" });
      const doc = msgDoc("m1", baseSystem({
        content: [{ kind: "doc_link", target: { kind: "doc", doc_id: "doc1", embedded_path: "/embedded/actor/0" }, label: "My Item" }],
      }));
      const opened: unknown[] = [];
      render(MessageCard, {
        props: { message: doc, showChannel: false },
        context: setAppContextForTest({ documents: storeWith(target, doc), openDocument: (r) => opened.push(r) }),
      });
      fireEvent.click(screen.getByText("My Item"));
      expect(opened).toEqual([{ docId: "doc1", embeddedPath: "/embedded/actor/0" }]);
    });

    it("renders a resolvable token doc_link as a clickable button", () => {
      const token = msgDoc("tok1", { channel: "g" });
      const doc = msgDoc("m1", baseSystem({
        content: [{ kind: "doc_link", target: { kind: "token", token_id: "tok1" }, label: "Goblin" }],
      }));
      const opened: unknown[] = [];
      render(MessageCard, {
        props: { message: doc, showChannel: false },
        context: setAppContextForTest({ documents: storeWith(token, doc), openDocument: (r) => opened.push(r) }),
      });
      fireEvent.click(screen.getByText("Goblin"));
      expect(opened).toEqual([{ tokenId: "tok1" }]);
    });

    it("a dangling doc_link target fails closed to inert text, no throw", () => {
      const doc = msgDoc("m1", baseSystem({
        content: [{ kind: "doc_link", target: { kind: "doc", doc_id: "gone" }, label: "Ghost Doc" }],
      }));
      render(MessageCard, {
        props: { message: doc, showChannel: false },
        context: setAppContextForTest({ documents: storeWith(doc) }),
      });
      expect(screen.getByText("Ghost Doc").tagName).toBe("SPAN");
    });
  });
  ```

- [ ] **Step 6: Run the gate.** From the repo root: `pnpm --filter @shadowcat/module-chat-card
  typecheck`; `pnpm --filter @shadowcat/module-chat-card test`.

---

## Task 5: Client — composer `@doc` authoring trigger

**Files:**
- Modify: `src/modules/chat-composer/src/Composer.svelte`
- Modify: `src/client/ui-kit/src/locales/en.ts`
- Test: `src/modules/chat-composer/src/Composer.test.ts`

**Interfaces:**
- Consumes: `AppContext.searchDocuments` (existing seam), `WireSearchHit`, `SubscriptionHandle`,
  `resolveTokenActor`, `actorDisplayName` (all already exported from `@shadowcat/core`).
- Produces: no new exported symbols — composer-local state + two private functions
  (`docLinkLabel`, `insertDocLink`).

### Steps

- [ ] **Step 1: Extend imports.** Find:
  ```ts
  import { actorDisplayName, MAX_MESSAGE_CHARS, type WireActorOwnerRef, type WireAudience, type WireDocument } from "@shadowcat/core";
  ```

  Replace with:
  ```ts
  import { actorDisplayName, resolveTokenActor, MAX_MESSAGE_CHARS, type WireActorOwnerRef, type WireAudience, type WireDocument, type WireSearchHit, type SubscriptionHandle } from "@shadowcat/core";
  ```

- [ ] **Step 2: Add doc-picker state, search effect, and helpers.** Find:
  ```ts
    let value = $state("");
    let textarea = $state<HTMLTextAreaElement | undefined>(undefined);
  ```

  Replace with:
  ```ts
    let value = $state("");
    let textarea = $state<HTMLTextAreaElement | undefined>(undefined);

    // `@doc` trigger: a searchable document/token picker parallel in UI weight to the "Speak
    // as" picker above, wired to the existing `searchDocuments` AppContext seam — no new
    // search/lookup code. Live-search subscription mirrors ActorsPanel's own pattern
    // (torn down/recreated on every query change, guarded against a stale callback firing
    // after a newer query's subscription is already active).
    let docPickerOpen = $state(false);
    let docQuery = $state("");
    let docHits = $state<WireSearchHit[]>([]);
    $effect(() => {
      if (!docPickerOpen) { docHits = []; return; }
      const q = docQuery.trim();
      if (!q) { docHits = []; return; }
      let handle: SubscriptionHandle | null = null;
      let cancelled = false;
      void ctx
        .searchDocuments(q, { limit: 20 }, (hits: WireSearchHit[]) => {
          if (cancelled) return;
          docHits = hits;
        })
        .then((h) => { if (cancelled) h.unsubscribe(); else handle = h; })
        .catch(() => { /* no transport: leave last hits, re-subscribe on next keystroke */ });
      return () => { cancelled = true; handle?.unsubscribe(); };
    });

    /** Display label for a search hit: a token resolves through its linked/embedded actor (the
     * same `resolveTokenActor`/`actorDisplayName` read-through every other actor/token consumer
     * uses), any other document falls back to its envelope `name`.
     * @param doc The candidate document/token.
     * @returns The label to both display in the picker and capture into the inserted span.
     * @example
     * ```
     * // internal; used by the picker's result list and insertDocLink
     * declare const doc: WireDocument;
     * docLinkLabel(doc);
     * ```
     */
    function docLinkLabel(doc: WireDocument): string {
      if (doc.doc_type === "token") {
        const eff = resolveTokenActor(doc, ctx.documents);
        return eff ? actorDisplayName(eff) : (doc.name ?? doc.id.slice(0, 8));
      }
      return doc.name ?? `${doc.doc_type} ${doc.id.slice(0, 8)}`;
    }

    /** Inserts a `[[doc:<id>|<label>]]`/`[[token:<id>|<label>]]` span at the textarea's cursor
     * position and closes the picker. Strips `|`/`]]` from the label before building the span —
     * a document/token's authored name is free text that could otherwise corrupt the
     * `scan_body` grammar (split the label early on an embedded `|`, or terminate the span early
     * on an embedded `]]`).
     * @param doc The picked document/token.
     * @example
     * ```
     * // internal; wired to each result row's click handler
     * declare const doc: WireDocument;
     * insertDocLink(doc);
     * ```
     */
    function insertDocLink(doc: WireDocument): void {
      const label = docLinkLabel(doc).replaceAll("|", "").replaceAll("]]", "");
      const span = doc.doc_type === "token" ? `[[token:${doc.id}|${label}]]` : `[[doc:${doc.id}|${label}]]`;
      const el = textarea;
      const start = el?.selectionStart ?? value.length;
      const end = el?.selectionEnd ?? value.length;
      value = value.slice(0, start) + span + value.slice(end);
      docPickerOpen = false;
      docQuery = "";
      docHits = [];
      const caret = start + span.length;
      queueMicrotask(() => {
        autoGrow();
        el?.focus();
        el?.setSelectionRange(caret, caret);
      });
    }
  ```

- [ ] **Step 3: Add the trigger button + popover to the template.** Find:
  ```svelte
    <button type="button" onclick={send} disabled={!canSend}>{t("chat.composer.send")}</button>
  </div>
  {#if errorMsg}
  ```

  Replace with:
  ```svelte
    <button type="button" data-testid="doc-link-trigger" title={t("chat.composer.insertDocLink")} onclick={() => (docPickerOpen = !docPickerOpen)}>@doc</button>
    <button type="button" onclick={send} disabled={!canSend}>{t("chat.composer.send")}</button>
  </div>
  {#if docPickerOpen}
    <div class="doc-picker">
      <label class="visually-hidden" for="chat-composer-doc-search">{t("chat.composer.insertDocLink")}</label>
      <input id="chat-composer-doc-search" type="text" placeholder={t("chat.composer.docSearchPlaceholder")} bind:value={docQuery} />
      <ul class="doc-picker-results">
        {#each docHits as hit (hit.document.id)}
          <li>
            <button type="button" onclick={() => insertDocLink(hit.document)}>{docLinkLabel(hit.document)}</button>
          </li>
        {/each}
      </ul>
    </div>
  {/if}
  {#if errorMsg}
  ```

- [ ] **Step 4: Add CSS.** Find:
  ```scss
    .visually-hidden {
      position: absolute;
      width: 1px;
      height: 1px;
      padding: 0;
      margin: -1px;
      overflow: hidden;
      clip: rect(0, 0, 0, 0);
      white-space: nowrap;
      border: 0;
    }
  </style>
  ```

  Replace with:
  ```scss
    .visually-hidden {
      position: absolute;
      width: 1px;
      height: 1px;
      padding: 0;
      margin: -1px;
      overflow: hidden;
      clip: rect(0, 0, 0, 0);
      white-space: nowrap;
      border: 0;
    }
    .doc-picker {
      display: flex;
      flex-direction: column;
      gap: var(--space-1);
      margin-top: var(--space-1);
      padding: var(--space-1);
      border: 1px solid var(--border);
      border-radius: var(--radius-1);
    }
    .doc-picker input {
      min-height: 44px;
      padding: var(--space-1);
    }
    .doc-picker-results {
      list-style: none;
      margin: 0;
      padding: 0;
      max-height: 12em;
      overflow-y: auto;
    }
    .doc-picker-results button {
      width: 100%;
      min-height: 44px;
      text-align: left;
      padding: var(--space-1);
    }
  </style>
  ```

- [ ] **Step 5: Add i18n keys.** In `src/client/ui-kit/src/locales/en.ts`, find:
  ```ts
    "chat.composer.sendFailed": "Message could not be sent.",
  ```

  Replace with:
  ```ts
    "chat.composer.sendFailed": "Message could not be sent.",
    "chat.composer.insertDocLink": "Insert a document link",
    "chat.composer.docSearchPlaceholder": "Search documents…",
  ```

- [ ] **Step 6: Add tests to `src/modules/chat-composer/src/Composer.test.ts`.** Extend
  `renderComposer` to accept a `searchDocuments` override, then add a new `describe` block. Find:
  ```ts
  function renderComposer(
    opts: {
      audience?: WireAudience;
      send?: ReturnType<typeof vi.fn<(o: unknown) => Promise<void>>>;
      documents?: DocumentStore;
      role?: WorldRole;
      selfId?: string;
    } = {},
  ) {
    const send = opts.send ?? vi.fn<(o: unknown) => Promise<void>>(async () => {});
    const context = setAppContextForTest({
      chat: { send, edit: vi.fn(async () => {}), delete: vi.fn(async () => {}) },
      documents: opts.documents ?? new DocumentStore(),
      role: opts.role ?? "player",
      selfId: opts.selfId ?? "u-self",
    });
    render(Composer, { props: { channel: "general", audience: opts.audience ?? publicAudience, placeholderName: "Alice" }, context });
    return { send };
  }
  ```

  Replace with:
  ```ts
  function renderComposer(
    opts: {
      audience?: WireAudience;
      send?: ReturnType<typeof vi.fn<(o: unknown) => Promise<void>>>;
      documents?: DocumentStore;
      role?: WorldRole;
      selfId?: string;
      searchDocuments?: ReturnType<typeof vi.fn>;
    } = {},
  ) {
    const send = opts.send ?? vi.fn<(o: unknown) => Promise<void>>(async () => {});
    const context = setAppContextForTest({
      chat: { send, edit: vi.fn(async () => {}), delete: vi.fn(async () => {}) },
      documents: opts.documents ?? new DocumentStore(),
      role: opts.role ?? "player",
      selfId: opts.selfId ?? "u-self",
      searchDocuments: opts.searchDocuments,
    });
    render(Composer, { props: { channel: "general", audience: opts.audience ?? publicAudience, placeholderName: "Alice" }, context });
    return { send };
  }
  ```

  Then append:
  ```ts
  describe("Composer — @doc link insertion", () => {
    it("opens the doc picker, searches, and inserts a [[doc:id|label]] span at the cursor", async () => {
      const hitDoc = { ...buildActorDoc("w1", "My Doc", { displayName: "My Doc", visual: { kind: "image", asset: "a1" }, size: { w: 1, h: 1 }, shape: "square", faction: null, conditions: [], prototype: false, vision: null }, "doc1") };
      const searchDocuments = vi.fn((_q: string, _opts: unknown, onUpdate: (hits: unknown[]) => void) => {
        onUpdate([{ document: hitDoc, score: 1, snippet: "" }]);
        return Promise.resolve({ unsubscribe: () => {} });
      });
      renderComposer({ searchDocuments });
      await fireEvent.click(screen.getByTestId("doc-link-trigger"));
      const search = screen.getByPlaceholderText("Search documents…");
      await fireEvent.input(search, { target: { value: "My" } });
      await fireEvent.click(await screen.findByText("My Doc"));
      const textarea = screen.getByRole("textbox") as HTMLTextAreaElement;
      expect(textarea.value).toBe("[[doc:doc1|My Doc]]");
      expect(screen.queryByPlaceholderText("Search documents…")).toBeNull();
    });

    it("inserts a [[token:id|label]] span for a token-doc_type search hit", async () => {
      const tokenDoc: WireDocument = { ...buildActorDoc("w1", "unused", { displayName: "x", visual: { kind: "image", asset: "a1" }, size: { w: 1, h: 1 }, shape: "square", faction: null, conditions: [], prototype: false, vision: null }, "tok1"), doc_type: "token", name: "Goblin" };
      const searchDocuments = vi.fn((_q: string, _opts: unknown, onUpdate: (hits: unknown[]) => void) => {
        onUpdate([{ document: tokenDoc, score: 1, snippet: "" }]);
        return Promise.resolve({ unsubscribe: () => {} });
      });
      renderComposer({ searchDocuments });
      await fireEvent.click(screen.getByTestId("doc-link-trigger"));
      await fireEvent.input(screen.getByPlaceholderText("Search documents…"), { target: { value: "Gob" } });
      await fireEvent.click(await screen.findByText("Goblin"));
      const textarea = screen.getByRole("textbox") as HTMLTextAreaElement;
      expect(textarea.value).toBe("[[token:tok1|Goblin]]");
    });
  });
  ```

  Add `import type { WireDocument } from "@shadowcat/core";` to this file's existing import list
  if not already present (it is not — confirm and add it alongside the existing `WireAudience`/
  `WireCommand` type imports).

- [ ] **Step 7: Run the gate.** From the repo root: `pnpm --filter @shadowcat/module-chat-composer
  typecheck`; `pnpm --filter @shadowcat/module-chat-composer test`.

---

## Task 6: Server — `TokenInstance` ownership-check ingest arm

**Files:**
- Modify: `src/server/src/chat/mod.rs`

**Interfaces:**
- Consumes: `Repository::effective_owner_of` (existing trait method, `SqliteRepository`-backed,
  documented as the repo-level chokepoint for "who owns this doc, joining a linked actor with one
  pool read").
- Produces: no new public symbols — the `ActorOwnerRef::TokenInstance` ingest arm now performs a
  real ownership check instead of an unconditional rejection.

### Steps

- [ ] **Step 1: Replace the stub `TokenInstance` arm.** Find:
  ```rust
              // No first-party producer resolves a TokenInstance ref into a
              // display identity at send time.
              // TODO: implement speak-as-token attribution.
              // Until then, reject fail-closed rather than store an
              // unvalidated ref that a future consumer might trust.
              ActorOwnerRef::TokenInstance { .. } => {
                  return Err(SendMessageError::ActorNotSpeakable);
              }
  ```

  Replace with:
  ```rust
              ActorOwnerRef::TokenInstance { token_id } => {
                  let token_doc = repo
                      .get_document(*token_id)
                      .await
                      .map_err(SendMessageError::Data)?;
                  let is_gm = ctx.world_role == WorldRole::Gm;
                  let allowed = match &token_doc {
                      // Same world-pinning + GM-bypass shape as the `Actor` arm above.
                      // Ownership itself resolves through `effective_owner_of` — the
                      // repo-level chokepoint wrapping `permission::effective_owner` (a
                      // token's own `owner` override wins, else it inherits its linked
                      // actor's owner) — never reimplemented here.
                      Some(d)
                          if d.doc_type == crate::data::permission::TOKEN_DOC_TYPE
                              && crate::data::document::world_of(d) == Some(room.world_id) =>
                      {
                          is_gm
                              || repo
                                  .effective_owner_of(d)
                                  .await
                                  .map_err(SendMessageError::Data)?
                                  == Some(ctx.user_id)
                      }
                      _ => false,
                  };
                  if !allowed {
                      return Err(SendMessageError::ActorNotSpeakable);
                  }
              }
  ```

- [ ] **Step 2: Update `SendMessageError::ActorNotSpeakable`'s doc comment.** Find:
  ```rust
      /// The requested `actor_owner` cannot be attributed by this sender: the
      /// referenced actor doc does not exist, is not an `actor` doc_type, or
      /// is owned by someone else and the sender is not a GM; or the ref is a
      /// `TokenInstance` (rejected fail-closed pending speak-as-token).
      ActorNotSpeakable,
  ```

  Replace with:
  ```rust
      /// The requested `actor_owner` cannot be attributed by this sender: for an `Actor` ref,
      /// the referenced doc does not exist, is not an `actor` doc_type, is outside the sending
      /// room's world, or is owned by someone else and the sender is not a GM; for a
      /// `TokenInstance` ref, the referenced doc does not exist, is not a `token` doc_type, is
      /// outside the sending room's world, or its effective owner (own override, else linked
      /// actor's owner) is someone else and the sender is not a GM.
      ActorNotSpeakable,
  ```

- [ ] **Step 3: Retarget the now-partially-stale existing test.** Find:
  ```rust
      #[tokio::test]
      async fn send_message_rejects_token_instance_attribution() {
  ```

  Replace with:
  ```rust
      #[tokio::test]
      async fn send_message_rejects_attributing_a_nonexistent_token() {
  ```

  Find (within that same test, the trailing assertion):
  ```rust
          assert!(
              matches!(err, Err(SendMessageError::ActorNotSpeakable)),
              "token-instance attribution has no first-party producer yet — rejected fail-closed"
          );
      }
  ```

  Replace with:
  ```rust
          assert!(
              matches!(err, Err(SendMessageError::ActorNotSpeakable)),
              "a token_id with no matching stored document is rejected fail-closed"
          );
      }
  ```

- [ ] **Step 4: Add the `seed_token_doc` test helper and the five ownership-matrix tests.** Add
  near `seed_actor_doc`:
  ```rust
      /// A minimal token doc for the speak-as-token ingest gate tests, seeded directly via
      /// `apply_command` (bypasses permission checks — these tests exercise
      /// `handle_send_message`'s attribution gate, not the token-create authorization path).
      /// `engine` must be a well-formed `TokenEngine` body, same rationale as `seed_actor_doc`.
      fn seed_token_doc(id: Uuid, world: Uuid, owner: Option<Uuid>, actor_id: Option<Uuid>) -> Document {
          Document {
              id,
              scope: Scope::World { world_id: world },
              doc_type: "token".into(),
              schema_version: 1,
              name: None,
              source: None,
              base: None,
              owner,
              permissions: crate::data::document::PermissionSet::default(),
              embedded: Default::default(),
              parent_id: None,
              engine: Some(serde_json::json!({
                  "x": 0.0, "y": 0.0, "w": 1.0, "h": 1.0, "rotation": 0.0,
                  "actor_id": actor_id,
              })),
              system: serde_json::json!({}),
              created_at: 0,
              updated_at: 0,
          }
      }

      #[tokio::test]
      async fn send_message_allows_token_owner_via_its_own_override_to_speak_as_it() {
          use crate::auth::role::ServerRole;
          use crate::data::command::UnsequencedCommand;
          use crate::data::document::WorldRole;
          use crate::data::sqlite::SqliteRepository;
          use crate::ws::room::RoomRegistry;

          let repo = SqliteRepository::connect("sqlite::memory:").await.unwrap();
          let gm = repo.create_user("gm", None, ServerRole::User, 0).await.unwrap();
          let player = repo.create_user("pl", None, ServerRole::User, 0).await.unwrap();
          let w = repo.create_world_owned("W", gm, 0).await.unwrap();
          repo.add_member(w.id, player, WorldRole::Player).await.unwrap();
          let token_id = Uuid::new_v4();
          repo.apply_command(UnsequencedCommand {
              world_id: w.id,
              author: player,
              ts: 0,
              ops: vec![Operation::Create { doc: seed_token_doc(token_id, w.id, Some(player), None) }],
          })
          .await
          .unwrap();

          let ctx = PermissionContext { user_id: player, world_role: WorldRole::Player };
          let reg = RoomRegistry::new();
          let room = reg.get_or_create(&repo, w.id).await.unwrap().unwrap();
          let rate = PingRateLimiter::new();

          let cmd = handle_send_message(
              MessageRequestCtx {
                  room: &room,
                  repo: &repo,
                  ctx: &ctx,
                  rate: &rate,
                  preview: LinkPreviewDeps {
                      client: &super::link_preview::build_client_allow_loopback(),
                      cache: &LinkPreviewCache::new(),
                      rate: &PreviewRateLimiter::new(),
                  },
                  now: 100,
                  budget_per_min: 30,
              },
              "all".into(),
              "grr".into(),
              Some(ActorOwnerRef::TokenInstance { token_id }),
              Audience::Public,
          )
          .await
          .unwrap();
          let doc = match &cmd.ops[0] {
              Operation::Create { doc } => doc,
              other => panic!("expected Create, got {other:?}"),
          };
          let sys: MessageEngine = serde_json::from_value(doc.engine.clone().unwrap()).unwrap();
          assert_eq!(sys.actor_owner, Some(ActorOwnerRef::TokenInstance { token_id }));
      }

      #[tokio::test]
      async fn send_message_allows_the_linked_actors_owner_to_speak_as_its_token() {
          use crate::auth::role::ServerRole;
          use crate::data::command::UnsequencedCommand;
          use crate::data::document::WorldRole;
          use crate::data::sqlite::SqliteRepository;
          use crate::ws::room::RoomRegistry;

          let repo = SqliteRepository::connect("sqlite::memory:").await.unwrap();
          let gm = repo.create_user("gm", None, ServerRole::User, 0).await.unwrap();
          let player = repo.create_user("pl", None, ServerRole::User, 0).await.unwrap();
          let w = repo.create_world_owned("W", gm, 0).await.unwrap();
          repo.add_member(w.id, player, WorldRole::Player).await.unwrap();
          let actor_id = Uuid::new_v4();
          let token_id = Uuid::new_v4();
          repo.apply_command(UnsequencedCommand {
              world_id: w.id,
              author: player,
              ts: 0,
              ops: vec![
                  Operation::Create { doc: seed_actor_doc(actor_id, w.id, Some(player)) },
                  Operation::Create { doc: seed_token_doc(token_id, w.id, None, Some(actor_id)) },
              ],
          })
          .await
          .unwrap();

          let ctx = PermissionContext { user_id: player, world_role: WorldRole::Player };
          let reg = RoomRegistry::new();
          let room = reg.get_or_create(&repo, w.id).await.unwrap().unwrap();
          let rate = PingRateLimiter::new();

          let cmd = handle_send_message(
              MessageRequestCtx {
                  room: &room,
                  repo: &repo,
                  ctx: &ctx,
                  rate: &rate,
                  preview: LinkPreviewDeps {
                      client: &super::link_preview::build_client_allow_loopback(),
                      cache: &LinkPreviewCache::new(),
                      rate: &PreviewRateLimiter::new(),
                  },
                  now: 100,
                  budget_per_min: 30,
              },
              "all".into(),
              "grr".into(),
              Some(ActorOwnerRef::TokenInstance { token_id }),
              Audience::Public,
          )
          .await
          .unwrap();
          let doc = match &cmd.ops[0] {
              Operation::Create { doc } => doc,
              other => panic!("expected Create, got {other:?}"),
          };
          let sys: MessageEngine = serde_json::from_value(doc.engine.clone().unwrap()).unwrap();
          assert_eq!(sys.actor_owner, Some(ActorOwnerRef::TokenInstance { token_id }));
      }

      #[tokio::test]
      async fn send_message_rejects_a_non_owner_non_gm_speaking_as_a_token() {
          use crate::auth::role::ServerRole;
          use crate::data::command::UnsequencedCommand;
          use crate::data::document::WorldRole;
          use crate::data::sqlite::SqliteRepository;
          use crate::ws::room::RoomRegistry;

          let repo = SqliteRepository::connect("sqlite::memory:").await.unwrap();
          let gm = repo.create_user("gm", None, ServerRole::User, 0).await.unwrap();
          let player = repo.create_user("pl", None, ServerRole::User, 0).await.unwrap();
          let other = repo.create_user("ot", None, ServerRole::User, 0).await.unwrap();
          let w = repo.create_world_owned("W", gm, 0).await.unwrap();
          repo.add_member(w.id, player, WorldRole::Player).await.unwrap();
          repo.add_member(w.id, other, WorldRole::Player).await.unwrap();
          let token_id = Uuid::new_v4();
          repo.apply_command(UnsequencedCommand {
              world_id: w.id,
              author: other,
              ts: 0,
              ops: vec![Operation::Create { doc: seed_token_doc(token_id, w.id, Some(other), None) }],
          })
          .await
          .unwrap();

          let ctx = PermissionContext { user_id: player, world_role: WorldRole::Player };
          let reg = RoomRegistry::new();
          let room = reg.get_or_create(&repo, w.id).await.unwrap().unwrap();
          let rate = PingRateLimiter::new();

          let err = handle_send_message(
              MessageRequestCtx {
                  room: &room,
                  repo: &repo,
                  ctx: &ctx,
                  rate: &rate,
                  preview: LinkPreviewDeps {
                      client: &super::link_preview::build_client_allow_loopback(),
                      cache: &LinkPreviewCache::new(),
                      rate: &PreviewRateLimiter::new(),
                  },
                  now: 100,
                  budget_per_min: 30,
              },
              "all".into(),
              "grr".into(),
              Some(ActorOwnerRef::TokenInstance { token_id }),
              Audience::Public,
          )
          .await;
          assert!(matches!(err, Err(SendMessageError::ActorNotSpeakable)));
      }

      #[tokio::test]
      async fn send_message_rejects_a_token_from_another_world_even_for_its_owner() {
          use crate::auth::role::ServerRole;
          use crate::data::command::UnsequencedCommand;
          use crate::data::document::WorldRole;
          use crate::data::sqlite::SqliteRepository;
          use crate::ws::room::RoomRegistry;

          let repo = SqliteRepository::connect("sqlite::memory:").await.unwrap();
          let gm = repo.create_user("gm", None, ServerRole::User, 0).await.unwrap();
          let player = repo.create_user("pl", None, ServerRole::User, 0).await.unwrap();
          let world_a = repo.create_world_owned("A", gm, 0).await.unwrap();
          let world_b = repo.create_world_owned("B", gm, 0).await.unwrap();
          repo.add_member(world_a.id, player, WorldRole::Player).await.unwrap();
          repo.add_member(world_b.id, player, WorldRole::Player).await.unwrap();

          // The token lives in world B and IS owned by `player` — ownership alone must not be
          // enough to speak as it from world A's room.
          let token_id = Uuid::new_v4();
          repo.apply_command(UnsequencedCommand {
              world_id: world_b.id,
              author: player,
              ts: 0,
              ops: vec![Operation::Create { doc: seed_token_doc(token_id, world_b.id, Some(player), None) }],
          })
          .await
          .unwrap();

          let ctx = PermissionContext { user_id: player, world_role: WorldRole::Player };
          let reg = RoomRegistry::new();
          let room_a = reg.get_or_create(&repo, world_a.id).await.unwrap().unwrap();
          let rate = PingRateLimiter::new();

          let err = handle_send_message(
              MessageRequestCtx {
                  room: &room_a,
                  repo: &repo,
                  ctx: &ctx,
                  rate: &rate,
                  preview: LinkPreviewDeps {
                      client: &super::link_preview::build_client_allow_loopback(),
                      cache: &LinkPreviewCache::new(),
                      rate: &PreviewRateLimiter::new(),
                  },
                  now: 0,
                  budget_per_min: 30,
              },
              "all".into(),
              "hi".into(),
              Some(ActorOwnerRef::TokenInstance { token_id }),
              Audience::Public,
          )
          .await
          .unwrap_err();
          assert!(matches!(err, SendMessageError::ActorNotSpeakable));
      }

      #[tokio::test]
      async fn send_message_allows_gm_to_speak_as_any_token_regardless_of_owner() {
          use crate::auth::role::ServerRole;
          use crate::data::command::UnsequencedCommand;
          use crate::data::document::WorldRole;
          use crate::data::sqlite::SqliteRepository;
          use crate::ws::room::RoomRegistry;

          let repo = SqliteRepository::connect("sqlite::memory:").await.unwrap();
          let gm = repo.create_user("gm", None, ServerRole::User, 0).await.unwrap();
          let player = repo.create_user("pl", None, ServerRole::User, 0).await.unwrap();
          let w = repo.create_world_owned("W", gm, 0).await.unwrap();
          repo.add_member(w.id, player, WorldRole::Player).await.unwrap();
          let token_id = Uuid::new_v4();
          repo.apply_command(UnsequencedCommand {
              world_id: w.id,
              author: player,
              ts: 0,
              ops: vec![Operation::Create { doc: seed_token_doc(token_id, w.id, Some(player), None) }],
          })
          .await
          .unwrap();

          let ctx = PermissionContext { user_id: gm, world_role: WorldRole::Gm };
          let reg = RoomRegistry::new();
          let room = reg.get_or_create(&repo, w.id).await.unwrap().unwrap();
          let rate = PingRateLimiter::new();

          let cmd = handle_send_message(
              MessageRequestCtx {
                  room: &room,
                  repo: &repo,
                  ctx: &ctx,
                  rate: &rate,
                  preview: LinkPreviewDeps {
                      client: &super::link_preview::build_client_allow_loopback(),
                      cache: &LinkPreviewCache::new(),
                      rate: &PreviewRateLimiter::new(),
                  },
                  now: 100,
                  budget_per_min: 30,
              },
              "all".into(),
              "grr".into(),
              Some(ActorOwnerRef::TokenInstance { token_id }),
              Audience::Public,
          )
          .await
          .unwrap();
          let doc = match &cmd.ops[0] {
              Operation::Create { doc } => doc,
              other => panic!("expected Create, got {other:?}"),
          };
          let sys: MessageEngine = serde_json::from_value(doc.engine.clone().unwrap()).unwrap();
          assert_eq!(sys.actor_owner, Some(ActorOwnerRef::TokenInstance { token_id }));
      }
  ```

- [ ] **Step 5: Run the gate.** From `src/server/`: `cargo fmt --all -- --check`; `cargo clippy
  --all-targets -- -D warnings`; `cargo test --all`.

---

## Task 7: Client — `SpeakAsToken` AppContext seam

**Files:**
- Create: `src/client/ui-kit/src/speakAsToken.svelte.ts`
- Create: `src/client/ui-kit/src/speakAsToken.test.ts`
- Modify: `src/client/ui-kit/src/index.ts`
- Modify: `src/client/ui-kit/src/appContext.ts`
- Modify: `src/client/ui-kit/src/__fixtures__/appContextTest.ts`
- Modify: `src/client/ui-kit/src/__fixtures__/SurfaceHarness.svelte`
- Modify: `src/client/shell/src/lib/Table.svelte`

**Interfaces:**
- Produces: `SpeakAsToken` class (`tokenId: string | null` getter, `select(id: string | null):
  void`, `consume(): string | null`).
- Produces: `AppContext.speakAsToken: SpeakAsToken` (new required field).
- Consumed by: Task 8 (`ToolRail.svelte` writes via `.select`), Task 9 (`Composer.svelte` reads
  `.tokenId` and calls `.consume()`).

### Steps

- [ ] **Step 1: Create `src/client/ui-kit/src/speakAsToken.svelte.ts`.**
  ```ts
  /**
   * The token instance a scene-tools affordance has picked to speak as for the composer's NEXT
   * message send — a one-shot pending selection, distinct from the composer's own sticky actor
   * `<select>`. A stable instance held by the shell and shared via AppContext: `ToolRail` sets it
   * (the "speak as this token" button), the composer consumes it on send.
   *
   * Sibling of `SceneSelection`: same stable-instance/mutate-in-place shape (`$state` +
   * `select`), and likewise does not prune when the referenced token is later deleted/deselected
   * — the composer resolves against the current document store and handles a miss itself.
   * Diverges in offering `consume()`, since the pending value here is read-once by design (the
   * spec's "for the next message sent"), not a persistent selection like `SceneSelection`'s.
   */
  export class SpeakAsToken {
    /** Backing store for {@link SpeakAsToken.tokenId}. */
    #tokenId = $state<string | null>(null);

    /** The pending speak-as token id, or `null` when nothing is pending.
     * @returns The pending token id, or `null`. */
    get tokenId(): string | null {
      return this.#tokenId;
    }

    /** Set (or clear, with `null`) the pending speak-as token.
     * @param id - The token id to target, or `null` to clear.
     * @example speakAsToken.select("tok-1");
     */
    select(id: string | null): void {
      this.#tokenId = id;
    }

    /** Reads the pending token id and clears it in the same step — the composer's one-shot
     * consume-on-send contract ("for the next message sent"): a rejected/aborted send does not
     * restore the consumed value, mirroring how the composer's draft text is also cleared
     * optimistically before a send's outcome is known.
     * @returns The pending token id that was set, or `null` if nothing was pending.
     * @example speakAsToken.consume(); // returns "tok-1" and clears back to null
     */
    consume(): string | null {
      const id = this.#tokenId;
      this.#tokenId = null;
      return id;
    }
  }
  ```

- [ ] **Step 2: Create `src/client/ui-kit/src/speakAsToken.test.ts`.**
  ```ts
  import { describe, it, expect } from "vitest";
  import { SpeakAsToken } from "./speakAsToken.svelte";

  describe("SpeakAsToken", () => {
    it("holds and clears the pending token id via select", () => {
      const s = new SpeakAsToken();
      expect(s.tokenId).toBeNull();
      s.select("tok-1");
      expect(s.tokenId).toBe("tok-1");
      s.select(null);
      expect(s.tokenId).toBeNull();
    });

    it("consume reads and clears in one step", () => {
      const s = new SpeakAsToken();
      s.select("tok-1");
      expect(s.consume()).toBe("tok-1");
      expect(s.tokenId).toBeNull();
    });

    it("consume returns null when nothing is pending", () => {
      const s = new SpeakAsToken();
      expect(s.consume()).toBeNull();
    });
  });
  ```

- [ ] **Step 3: Export from `src/client/ui-kit/src/index.ts`.** Find:
  ```ts
  export { SceneSelection } from "./sceneSelection.svelte";
  ```

  Replace with:
  ```ts
  export { SceneSelection } from "./sceneSelection.svelte";
  export { SpeakAsToken } from "./speakAsToken.svelte";
  ```

- [ ] **Step 4: Add to `AppContext` in `src/client/ui-kit/src/appContext.ts`.** Find:
  ```ts
    sceneSelection: SceneSelection;
  ```

  Replace with:
  ```ts
    sceneSelection: SceneSelection;
    /** The pending "speak as this token" selection for the composer's next send — see
     * `SpeakAsToken`'s class doc for the one-shot consume contract. */
    speakAsToken: SpeakAsToken;
  ```

  `SpeakAsToken` is a `ui-kit`-local class (not exported from `@shadowcat/core`), so it is
  imported the same way `SceneSelection` already is — a local relative import. Find:
  ```ts
  import type { SceneSelection } from "./sceneSelection.svelte";
  ```

  Replace with:
  ```ts
  import type { SceneSelection } from "./sceneSelection.svelte";
  import type { SpeakAsToken } from "./speakAsToken.svelte";
  ```

- [ ] **Step 5: Wire into `src/client/ui-kit/src/__fixtures__/appContextTest.ts`.** Find:
  ```ts
  import { SceneSelection } from "../sceneSelection.svelte";
  ```

  Replace with:
  ```ts
  import { SceneSelection } from "../sceneSelection.svelte";
  import { SpeakAsToken } from "../speakAsToken.svelte";
  ```

  Find:
  ```ts
      sceneSelection: over.sceneSelection ?? new SceneSelection(),
  ```

  Replace with:
  ```ts
      sceneSelection: over.sceneSelection ?? new SceneSelection(),
      speakAsToken: over.speakAsToken ?? new SpeakAsToken(),
  ```

- [ ] **Step 6: Wire into `src/client/ui-kit/src/__fixtures__/SurfaceHarness.svelte`.** Find:
  ```ts
    import { SceneSelection } from "../sceneSelection.svelte";
  ```

  Replace with:
  ```ts
    import { SceneSelection } from "../sceneSelection.svelte";
    import { SpeakAsToken } from "../speakAsToken.svelte";
  ```

  Find (inside the single `setAppContext({...})` call, the `sceneSelection: new SceneSelection(),`
  segment):
  ```ts
  sceneSelection: new SceneSelection(),
  ```

  Replace with:
  ```ts
  sceneSelection: new SceneSelection(), speakAsToken: new SpeakAsToken(),
  ```

- [ ] **Step 7: Wire into `src/client/shell/src/lib/Table.svelte`.** Find:
  ```ts
    import { setAppContext, Surface, PanelsBridge, SheetsController, SceneSelection, TemplatesController, TemplateModalHost, NotificationHost, notifications } from "@shadowcat/ui-kit";
  ```

  Replace with:
  ```ts
    import { setAppContext, Surface, PanelsBridge, SheetsController, SceneSelection, SpeakAsToken, TemplatesController, TemplateModalHost, NotificationHost, notifications } from "@shadowcat/ui-kit";
  ```

  Find:
  ```ts
    const sceneSelection = new SceneSelection();
  ```

  Replace with:
  ```ts
    const sceneSelection = new SceneSelection();

    // Speak-as-token pending selection: the scene-tools affordance sets it, the composer
    // consumes it on send. Stable per Table, like `sceneSelection`.
    const speakAsToken = new SpeakAsToken();
  ```

  Find:
  ```ts
      sceneSelection,
  ```

  Replace with:
  ```ts
      sceneSelection,
      speakAsToken,
  ```

- [ ] **Step 8: Run the gate.** From the repo root: `pnpm --filter @shadowcat/ui-kit typecheck`;
  `pnpm --filter @shadowcat/ui-kit test`; `pnpm --filter @shadowcat/shell typecheck`; `pnpm -r
  typecheck` (catches every consumer of `AppContext` across every module package now that
  `speakAsToken` is a required field).

---

## Task 8: Client — scene-tools "speak as this token" affordance

**Files:**
- Modify: `src/modules/scene-tools/src/ToolRail.svelte`
- Modify: `src/client/ui-kit/src/locales/en.ts`
- Test: `src/modules/scene-tools/src/ToolRail.test.ts`

**Interfaces:**
- Consumes: Task 7's `AppContext.speakAsToken`; `ownerFloorApplies` (existing, exported from
  `@shadowcat/core`).
- Produces: no new exported symbols.

### Steps

- [ ] **Step 1: Extend imports.** Find:
  ```ts
    import { resolveSceneSettings, type WireDocument } from "@shadowcat/core";
  ```

  Replace with:
  ```ts
    import { resolveSceneSettings, ownerFloorApplies, type WireDocument } from "@shadowcat/core";
  ```

- [ ] **Step 2: Add the selected-token derivation and click handler.** Find:
  ```ts
    const drawModes: DrawMode[] = ["freehand", "rect", "ellipse", "line"];
  ```

  Replace with:
  ```ts
    // Speak-as-token affordance: shown whenever exactly one token is selected and the current
    // user may plausibly speak as it (GM, or the effective owner) — advisory only, mirroring the
    // "Speak as" composer picker's own client-side offer/server-reauthorizes split. Reuses the
    // subscriber bridge already established above for `activeScene`/`snapToGrid`.
    const selectedSpeakToken = $derived.by((): WireDocument | null => {
      subscribe();
      const ids = ctx.tokenSelection.ids;
      if (ids.size !== 1) return null;
      const [id] = ids;
      return ctx.documents.get(id) ?? null;
    });
    const canSpeakAsSelected = $derived.by((): boolean => {
      const tok = selectedSpeakToken;
      if (!tok) return false;
      return ctx.role === "gm" || ownerFloorApplies(tok, ctx.selfId, ctx.documents);
    });

    /** Sets the pending speak-as-token selection from the currently selected token (a no-op if
     * none is selected, guarded by `canSpeakAsSelected` at the call site).
     * @example
     * ```
     * // internal; wired to the "speak as this token" button
     * speakAsSelectedToken();
     * ```
     */
    function speakAsSelectedToken(): void {
      const tok = selectedSpeakToken;
      if (tok) ctx.speakAsToken.select(tok.id);
    }

    const drawModes: DrawMode[] = ["freehand", "rect", "ellipse", "line"];
  ```

- [ ] **Step 3: Render the button.** Find:
  ```svelte
    <!-- Snap is a scene-document write (`/engine/snapToGrid`), i.e. authoring: GM-only. -->
    {#if isGm && activeScene}
      <button
        type="button"
        class="tool"
        aria-pressed={snapToGrid}
        data-testid="snap-toggle"
        title={t("tools.snap")}
        onclick={toggleSnap}
      >
        {t("tools.snap")}
      </button>
    {/if}
  ```

  Replace with:
  ```svelte
    <!-- Snap is a scene-document write (`/engine/snapToGrid`), i.e. authoring: GM-only. -->
    {#if isGm && activeScene}
      <button
        type="button"
        class="tool"
        aria-pressed={snapToGrid}
        data-testid="snap-toggle"
        title={t("tools.snap")}
        onclick={toggleSnap}
      >
        {t("tools.snap")}
      </button>
    {/if}

    {#if canSpeakAsSelected}
      <button
        type="button"
        class="tool"
        data-testid="speak-as-token"
        title={t("tools.speakAsToken")}
        onclick={speakAsSelectedToken}
      >
        {t("tools.speakAsToken")}
      </button>
    {/if}
  ```

- [ ] **Step 4: Add the i18n key.** In `src/client/ui-kit/src/locales/en.ts`, find:
  ```ts
    "tools.snap": "Snap to grid",
  ```

  Replace with:
  ```ts
    "tools.snap": "Snap to grid",
    "tools.speakAsToken": "Speak as this token",
  ```

- [ ] **Step 5: Add tests to `src/modules/scene-tools/src/ToolRail.test.ts`.**
  ```ts
  test("a player who owns the single selected token sees and can use the speak-as-token button", async () => {
    const { scene } = captureScene();
    const documents = new DocumentStore();
    const sceneDoc = buildSceneDoc("w1", "S1");
    documents.applyCommand({ seq: 1, world_id: "w1", author: "gm", ts: 0, ops: [{ op: "create" as const, doc: sceneDoc }] });
    const token = { ...buildTokenDoc("w1", sceneDoc.id, { x: 0, y: 0, w: 1, h: 1, rotation: 0 }, "tok1"), owner: "u-self" };
    documents.applyCommand({ seq: 2, world_id: "w1", author: "u-self", ts: 0, ops: [{ op: "create" as const, doc: token }] });
    const tokenSelection = new TokenSelection();
    tokenSelection.set(["tok1"]);
    const speakAsToken = new SpeakAsToken();
    render(ToolRail, { context: setAppContextForTest({ role: "player", selfId: "u-self", scene, documents, tokenSelection, speakAsToken }) });
    const button = screen.getByTestId("speak-as-token");
    await fireEvent.click(button);
    expect(speakAsToken.tokenId).toBe("tok1");
  });

  test("a non-owner player does not see the speak-as-token button", () => {
    const { scene } = captureScene();
    const documents = new DocumentStore();
    const sceneDoc = buildSceneDoc("w1", "S1");
    documents.applyCommand({ seq: 1, world_id: "w1", author: "gm", ts: 0, ops: [{ op: "create" as const, doc: sceneDoc }] });
    const token = { ...buildTokenDoc("w1", sceneDoc.id, { x: 0, y: 0, w: 1, h: 1, rotation: 0 }, "tok1"), owner: "someone-else" };
    documents.applyCommand({ seq: 2, world_id: "w1", author: "gm", ts: 0, ops: [{ op: "create" as const, doc: token }] });
    const tokenSelection = new TokenSelection();
    tokenSelection.set(["tok1"]);
    render(ToolRail, { context: setAppContextForTest({ role: "player", selfId: "u-self", scene, documents, tokenSelection }) });
    expect(screen.queryByTestId("speak-as-token")).toBeNull();
  });

  test("no button renders when zero or more than one token is selected", () => {
    const { scene } = captureScene();
    render(ToolRail, { context: setAppContextForTest({ role: "gm", scene }) });
    expect(screen.queryByTestId("speak-as-token")).toBeNull();
  });
  ```

  Add `import { SpeakAsToken } from "@shadowcat/ui-kit";` to this file's existing import list (it
  already imports `TokenSelection` from the same package on a neighboring line).

- [ ] **Step 6: Run the gate.** From the repo root: `pnpm --filter @shadowcat/module-scene-tools
  typecheck`; `pnpm --filter @shadowcat/module-scene-tools test`.

---

## Task 9: Client — composer consumes the pending speak-as-token selection

**Files:**
- Modify: `src/modules/chat-composer/src/Composer.svelte`
- Modify: `src/client/ui-kit/src/locales/en.ts`
- Test: `src/modules/chat-composer/src/Composer.test.ts`

**Interfaces:**
- Consumes: Task 7's `AppContext.speakAsToken`.
- Produces: no new exported symbols.

### Steps

- [ ] **Step 1: Add the reactive indicator name derivation.** Find:
  ```ts
    const placeholder = $derived(audience.kind === "gm_only" ? t("chat.composer.placeholderGm") : t("chat.composer.placeholder", { name: placeholderName }));
  ```

  Replace with:
  ```ts
    const placeholder = $derived(audience.kind === "gm_only" ? t("chat.composer.placeholderGm") : t("chat.composer.placeholder", { name: placeholderName }));

    // Speak-as-token indicator: mirrors how the sticky actor `<select>` above surfaces its own
    // selection, but reads the one-shot `ctx.speakAsToken` pending value instead.
    const speakAsTokenName = $derived.by((): string | null => {
      subscribe();
      const id = ctx.speakAsToken.tokenId;
      if (!id) return null;
      const tok = ctx.documents.get(id);
      if (!tok) return null;
      const eff = resolveTokenActor(tok, ctx.documents);
      return eff ? actorDisplayName(eff) : (tok.name ?? id.slice(0, 8));
    });
  ```

- [ ] **Step 2: Give the pending token precedence over the actor `<select>` in `send`, consuming
  it.** Find:
  ```ts
      const actorOwner: WireActorOwnerRef | undefined = selectedActorId ? { kind: "actor", actor_id: selectedActorId } : undefined;
  ```

  Replace with:
  ```ts
      const pendingToken = ctx.speakAsToken.consume();
      const actorOwner: WireActorOwnerRef | undefined = pendingToken
        ? { kind: "token_instance", token_id: pendingToken }
        : selectedActorId
          ? { kind: "actor", actor_id: selectedActorId }
          : undefined;
  ```

- [ ] **Step 3: Render the indicator.** Find:
  ```svelte
  <div class="composer">
  ```

  Replace with:
  ```svelte
  {#if speakAsTokenName}
    <div class="speaking-as">{t("chat.composer.speakingAsToken", { name: speakAsTokenName })}</div>
  {/if}
  <div class="composer">
  ```

- [ ] **Step 4: Add CSS.** Find:
  ```scss
    .send-error {
      margin-top: var(--space-1);
      font-size: 0.85em;
      color: var(--danger);
    }
  ```

  Replace with:
  ```scss
    .send-error {
      margin-top: var(--space-1);
      font-size: 0.85em;
      color: var(--danger);
    }
    .speaking-as {
      margin-bottom: var(--space-1);
      font-size: 0.85em;
      font-style: italic;
      opacity: 0.85;
    }
  ```

- [ ] **Step 5: Add the i18n key.** In `src/client/ui-kit/src/locales/en.ts`, find:
  ```ts
    "chat.composer.docSearchPlaceholder": "Search documents…",
  ```

  Replace with:
  ```ts
    "chat.composer.docSearchPlaceholder": "Search documents…",
    "chat.composer.speakingAsToken": "Speaking as: {name}",
  ```

- [ ] **Step 6: Extend `renderComposer` with a `speakAsToken` override, then add tests.** First,
  in `src/modules/chat-composer/src/Composer.test.ts`, find the `renderComposer` helper as it
  stands after Task 5's Step 6 edit:
  ```ts
  function renderComposer(
    opts: {
      audience?: WireAudience;
      send?: ReturnType<typeof vi.fn<(o: unknown) => Promise<void>>>;
      documents?: DocumentStore;
      role?: WorldRole;
      selfId?: string;
      searchDocuments?: ReturnType<typeof vi.fn>;
    } = {},
  ) {
    const send = opts.send ?? vi.fn<(o: unknown) => Promise<void>>(async () => {});
    const context = setAppContextForTest({
      chat: { send, edit: vi.fn(async () => {}), delete: vi.fn(async () => {}) },
      documents: opts.documents ?? new DocumentStore(),
      role: opts.role ?? "player",
      selfId: opts.selfId ?? "u-self",
      searchDocuments: opts.searchDocuments,
    });
    render(Composer, { props: { channel: "general", audience: opts.audience ?? publicAudience, placeholderName: "Alice" }, context });
    return { send };
  }
  ```

  Replace with:
  ```ts
  function renderComposer(
    opts: {
      audience?: WireAudience;
      send?: ReturnType<typeof vi.fn<(o: unknown) => Promise<void>>>;
      documents?: DocumentStore;
      role?: WorldRole;
      selfId?: string;
      searchDocuments?: ReturnType<typeof vi.fn>;
      speakAsToken?: SpeakAsToken;
    } = {},
  ) {
    const send = opts.send ?? vi.fn<(o: unknown) => Promise<void>>(async () => {});
    const context = setAppContextForTest({
      chat: { send, edit: vi.fn(async () => {}), delete: vi.fn(async () => {}) },
      documents: opts.documents ?? new DocumentStore(),
      role: opts.role ?? "player",
      selfId: opts.selfId ?? "u-self",
      searchDocuments: opts.searchDocuments,
      speakAsToken: opts.speakAsToken,
    });
    render(Composer, { props: { channel: "general", audience: opts.audience ?? publicAudience, placeholderName: "Alice" }, context });
    return { send };
  }
  ```

  Then add `import { SpeakAsToken } from "@shadowcat/ui-kit";` to this file's import list, and
  append the tests:
  ```ts
  describe("Composer — speak-as-token", () => {
    it("sends a token_instance actor_owner and consumes the pending selection", async () => {
      const speakAsToken = new SpeakAsToken();
      speakAsToken.select("tok1");
      const token = { ...buildActorDoc("w1", "unused", { displayName: "x", visual: { kind: "image", asset: "a1" }, size: { w: 1, h: 1 }, shape: "square", faction: null, conditions: [], prototype: false, vision: null }, "tok1"), doc_type: "token", name: "Goblin" };
      const documents = new DocumentStore();
      documents.applyCommand({ seq: 1, world_id: "w1", author: "gm", ts: 0, ops: [{ op: "create" as const, doc: token }] });
      const { send } = renderComposer({ documents, speakAsToken });
      expect(screen.getByText("Speaking as: Goblin")).toBeTruthy();
      const textarea = screen.getByRole("textbox") as HTMLTextAreaElement;
      await fireEvent.input(textarea, { target: { value: "hello" } });
      await fireEvent.keyDown(textarea, { key: "Enter" });
      expect(send).toHaveBeenCalledWith({ channel: "general", content: "hello", audience: publicAudience, actorOwner: { kind: "token_instance", token_id: "tok1" } });
      expect(speakAsToken.tokenId).toBeNull();
    });

    it("shows no indicator when nothing is pending", () => {
      renderComposer();
      expect(screen.queryByText(/Speaking as:/)).toBeNull();
    });
  });
  ```

- [ ] **Step 7: Run the gate.** From the repo root: `pnpm --filter @shadowcat/module-chat-composer
  typecheck`; `pnpm --filter @shadowcat/module-chat-composer test`.

---

## Task 10: Cross-part compile/behavior check

No new code. Both Part A (Tasks 1–5) and Part B (Tasks 6–9) touched `src/server/src/chat/mod.rs`'s
`handle_send_message` and its test module; this task re-verifies the combined file compiles and
every test in it passes together (a task-by-task gate re-run does not, by itself, prove the LAST
task's edits didn't silently shadow an earlier task's — e.g. a copy-paste of the `TokenInstance`
arm accidentally landing inside the `DocLink` match arm's block).

- [ ] **Step 1:** From `src/server/`: `cargo fmt --all -- --check`; `cargo clippy --all-targets --
  -D warnings`; `cargo test --all -- chat::`. Confirm every test added in Tasks 1, 2, and 6 is
  present in the test output (grep the run's test-name list for `scan_doc_link`,
  `send_message_stores_a_doc_link_segment`, `send_message_allows_token_owner_via_its_own_override`)
  — a name silently absent from the list (not merely passing) means a task's edit was lost by a
  later task's `Edit` diff, not that the feature is untested.
- [ ] **Step 2:** From the repo root: `pnpm -r typecheck`; `pnpm -r test`; `pnpm lint`. Confirm
  every test file touched by Tasks 3–5 and 7–9 is present in the run's file list.
- [ ] **Step 3:** If either step fails, fix forward (this is a compile/regression fix, not new
  scope) before proceeding to Task 11.

---

## Task 11: Documentation + skill-update closeout

**Files:**
- Modify: `docs/TODO.md` (remove the two closed sub-projects from bucket C)
- Modify: `.claude/skills/shadowcat-codebase-chat/SKILL.md`
- Modify: `.claude/skills/shadowcat-codebase-client-shell/SKILL.md`
- Modify: `.claude/skills/shadowcat-codebase-scene-rendering/SKILL.md`
- Modify: `.claude/.claude-plugin/plugin.json`

### Steps

- [ ] **Step 1: Read `docs/TODO.md`'s bucket-C entries for these two sub-projects verbatim**, and
  remove them (the design is now implemented, not merely scheduled) — do not leave a partial or
  reworded trace.

- [ ] **Step 2: Update `shadowcat-codebase-chat`'s SKILL.md.** At minimum:
  - The `Dice wire — chat::rolls + the ingest roll stage` section's "Attribution authz
    (world-pinned)" bullet currently states `TokenInstance` refs are "REJECTED until speak-as-token
    ships (same error, nothing persisted)" — replace with a description of the real check: a
    `TokenInstance` ref is validated the same world-pinned way as `Actor`, except ownership
    resolves through `Repository::effective_owner_of` (own `owner` override, else the linked
    actor's owner) rather than a stored `owner` field read directly.
  - Add a `Segment::DocLink`/`DocLinkTarget` bullet to the `chat::mod` key-files description
    (mirrors the existing `Segment` bullet's level of detail): the `doc:`/`token:` `scan_body`
    prefix, the required `|<label>` grammar, and the "no server-side existence/authz check —
    client fails closed at render" invariant.
  - Add the composer's `@doc` trigger and `SpeakAsToken`/scene-tools speak-as-token affordance to
    the `Client display layer` section's `module-chat-composer` bullet.
  - Update the `Segment` "Design note" sentence stating segments never get typed variants beyond
    `Text`/`Html` if it now reads as contradicted by `DocLink`'s addition (it should not — `DocLink`
    is a structured reference, not inline formatting/markup, so the existing design note about
    `Html` staying the single rich-content variant remains true; add one clarifying clause if
    needed rather than removing the note).

- [ ] **Step 3: Update `shadowcat-codebase-client-shell`'s SKILL.md.** Add `speakAsToken` to the
  `AppContext` bullet list already naming `chat`/`uiState.panelLayout`/`sceneSelection` (mirror
  that bullet's existing citation style), describing the one-shot consume-on-send contract in one
  sentence.

- [ ] **Step 4: Update `shadowcat-codebase-scene-rendering`'s SKILL.md.** Add one sentence to the
  `scene-tools` coverage area (wherever `ToolRail`'s existing tool/control list is described)
  noting the new speak-as-token button and its `ownerFloorApplies`-gated visibility.

- [ ] **Step 5: Dispatch `shadowcat-spec-reviewer`** on the three skill diffs from Steps 2–4,
  confirming each accurately captures the change with no omission, drift, or broken pointer. Fix
  any finding before proceeding.

- [ ] **Step 6: Bump `.claude/.claude-plugin/plugin.json`'s `version`** from `1.0.59` to `1.0.60`.

- [ ] **Step 7: Full-repo gate re-run.** From `src/server/`: `cargo fmt --all -- --check`; `cargo
  clippy --all-targets -- -D warnings`; `cargo test --all`. From the repo root: `pnpm -r
  typecheck`; `pnpm -r test`; `pnpm lint`. All must exit 0.
