# M19a · Chat Media — Implementation Plan

> **For agentic workers:** Execute task-by-task in order; each task's steps use checkbox
> (`- [ ]`) syntax. Written for a sonnet-class implementer with no conversation context —
> every path, symbol and test name below is exact; read the cited code before editing it.

**Goal:** Images reach chat only as `Segment::Image { asset_id, alt }` served by this server —
from an `[[asset:<uuid>|alt]]` span (composer picker) or from an external image URL the server
fetches through the SSRF-guarded client and asset-ifies post-publish. The sanitizer never emits
an `<img>` with a `src`. Edits keep structured references. The segment renderer moves to ui-kit
as the single `{@html}` sink. YouTube stays as shipped (evidence + docs only).

**Architecture:** `chat::body` (extracted chunk→segment composer shared by send/edit and, later,
notes/tables); `chat::sanitize` returns `Sanitized { segments, image_urls }`;
`PendingEnrichment::InlineImage` resolved by `post_publish::resolve_inline_image` through
`fetch_image_bytes` + `create_asset_from_bytes` (`Provenance::ChatImage`); ui-kit
`SegmentList.svelte` + `RollTooltip.svelte`; composer "Insert image" over `AppContext.pickAsset`.

**Tech stack:** Rust (server), ts-rs (bindings regenerated), Svelte 5 runes, Vitest, Playwright.

**Spec:** `docs/superpowers/specs/2026-09-02-m19-tables-notes-chat-media-design.md` — §0, §2.3,
§2.4, §3.3, §3.4, §4, §6, §7, §8, §11 (C3–C5, A1–A7). Read it first.

**Prerequisite:** M15b (`m15b-asset-browser`) merged to `main` — Task 9 calls
`AppContext.pickAsset(opts?: PickAssetOptions): Promise<string | null>` from
`src/client/ui-kit/src/appContext.ts`. If that member is absent when this plan starts, stop and
report before Task 1; do not stub it.

## Execution directives

**Every dispatched agent's first prompt MUST contain this paragraph verbatim:**

> The iron rule is no deferrals of existing work, or new work as it comes up - we fix this now
> unless I give my EXPRESS authorization. The only exception is if a bug or to-do has a genuine
> blocker that is already logged in a milestone in PLAN.md that has not been started yet. Another
> iron clad is rule is that when faced with a design fork, determine the best long term shape in
> keeping with our plans and goals, and implement accordingly. You only need to ask me if the
> question "what is the best long term shape in keeping with our plans and goals?" is not able to
> answer the question. Churn is not a concern. This paragraph must be copied verbatim to any
> agents dispatched in this campaign.

…plus the reporting rule: a subagent delivers its report as the Agent tool result or writes it to
a named file; the prompt states which. Opus is banned for every dispatch in this campaign.

## Global constraints

- No lint suppressions (`#[allow]`, `#[expect]`, `eslint-disable`, `@ts-ignore`); no file-size
  allowlist entries (split instead); Rust test bodies in sibling files (`pnpm lint:inline-tests`).
- Comments cite symbols, never files/lines; no milestone ids, spec pointers, dates, or history
  narration in code comments, `assert!` messages or test names (`pnpm lint:comments`).
- `pnpm build` before any cargo command (rust-embed validates `dist/`). Never two cargo commands
  concurrently in one worktree.
- Deletions via `trash`, never `rm`/`Remove-Item`/`git rm`; commits always `git add <paths>` +
  `git commit -- <paths>`; commit trailer per the campaign brief.
- Full gate list (run before claiming green): `pnpm build`, `pnpm -r typecheck`, `pnpm -r test`,
  `pnpm lint`, `pnpm lint:docs`, `pnpm lint:props`, `pnpm lint:comments`, `pnpm lint:allowances`,
  `pnpm lint:file-size`, `pnpm lint:inline-tests`, `pnpm docs:check-examples`,
  `pnpm run test:scripts`, `pnpm --filter "shadowcat-example-*" build`; from `src/server/`:
  `cargo fmt --all -- --check`, `cargo clippy --all-targets -- -D warnings`,
  `cargo clippy --all-targets -- -D missing-docs -D clippy::missing-docs-in-private-items`,
  `cargo test --all`, `git diff --exit-code src/types/generated`; e2e: `pnpm --filter
  @shadowcat/core test:e2e`, `pnpm --filter @shadowcat/shell e2e` (port 31999 must be free);
  skills: `node scripts/check-skill-symbol-refs-cli.mjs`, `node
  scripts/check-skill-api-refs-cli.mjs`.
- `src/server/src/chat/mod.rs` is 1,609 lines and `chat/tests.rs` is 3,770; a task that would
  push either past 5,000 splits it by subject first (`chat/tests/<subject>.rs` with shared
  fixtures in `chat/tests/mod.rs`).

---

### Task 1: `chat::body` — extract the chunk→segment composer (pure move)

**Files:**
- Create: `src/server/src/chat/body.rs`, `src/server/src/chat/body/tests.rs`
- Modify: `src/server/src/chat/mod.rs` (`mod body;`, `handle_send_message`'s Normal/Emote branch
  delegates), `src/server/src/chat/rolls.rs` (`scan_body_capped`)

**Interfaces:**
- `pub(crate) fn scan_body_capped(body: &str, max_spans: usize) -> Result<Vec<BodyChunk<'_>>,
  RollError>` — the existing loop with the cap as a parameter; `scan_body(body)` becomes
  `scan_body_capped(body, MAX_INLINE_ROLLS)`.
- `pub(crate) enum ScanMode { Execute, NoExecute }`.
- `pub(crate) struct ComposeDeps<'a> { repo: &'a dyn Repository, world_id: Uuid, channel: &'a str,
  actor_owner: Option<&'a ActorOwnerRef>, policy: &'a ChatContentPolicy }`.
- `pub(crate) async fn compose_message(body: &str, deps: ComposeDeps<'_>, mode: ScanMode) ->
  Result<Vec<Segment>, ComposeError>` — moves the per-chunk arms out of `handle_send_message`
  verbatim: `Text` → `sanitize`; `Inline` → (Execute) lazy `resolve_dice_context` + lazy
  `host::host_for_actor_owner` + `execute_roll` → `RollEmbed`, (NoExecute) `ComposeError::Inline`;
  `Button` → `validate_formula` → `RollButton`; `DocLink` → `DocLink`. The all-`Text` fast path
  (`sanitize(body)`) is preserved byte-identically.
- `pub(crate) enum ComposeError { Roll(RollError), Inline, Data(DataError) }` — `handle_send_message`
  maps `Roll` to the existing whispered `build_roll_error_notice` path and `Data` to
  `SendMessageError::Data`; `Inline` cannot occur in `Execute` mode (documented).

- [ ] **Step 1:** write `body/tests.rs` pinning current behaviour through the new function (text
  only ⇒ one segment identical to `sanitize`; inline roll ⇒ `RollEmbed`; button ⇒ `RollButton`;
  doc link ⇒ `DocLink`; `NoExecute` + inline ⇒ `ComposeError::Inline`). Use the existing chat test
  fixtures (`chat/tests.rs` builds worlds/actors; reuse via `pub(super)` helpers — move them to
  `chat/tests/mod.rs` if `tests.rs` must be split).
- [ ] **Step 2:** implement; `handle_send_message` calls `compose_message(.., ScanMode::Execute)`.
  Every existing `chat::tests` test stays green unmodified — that is the "pure move" proof.
- [ ] **Step 3:** `cargo test -p shadowcat chat` PASS; `cargo clippy` clean; `cargo fmt`.
- [ ] **Step 4:** `git commit -m "refactor(chat): extract the body composer from message ingest" -- src/server/src/chat/`

### Task 2: extract `validate_audience` and `validate_actor_owner` (pure moves)

**Files:**
- Modify: `src/server/src/chat/mod.rs`

**Interfaces:**
- `pub(crate) async fn validate_audience(repo: &dyn Repository, world_id: Uuid, sender: Uuid,
  audience: &Audience) -> Result<(), SendMessageError>` — the `MAX_WHISPER_RECIPIENTS` cap +
  per-recipient `member_role` loop exactly as `handle_send_message` performs them today.
- `pub(crate) async fn validate_actor_owner(repo: &dyn Repository, room: &Room, ctx:
  &PermissionContext, owner: &ActorOwnerRef) -> Result<(), SendMessageError>` — the world-pinned
  `Actor`/`TokenInstance` attribution gate (`world_of`, `effective_owner_of`) as it exists today.

- [ ] **Step 1:** move both bodies out; `handle_send_message` calls them at the same points.
  Existing tests (`unknown_recipient_*`, `actor_not_speakable_*`, the whisper-cap test) stay green
  unmodified.
- [ ] **Step 2:** `cargo test -p shadowcat chat` PASS; clippy; fmt.
- [ ] **Step 3:** `git commit -m "refactor(chat): name the audience and attribution gates" -- src/server/src/chat/mod.rs`

### Task 3: `Segment::Image` + `[[asset:…]]` spans at ingest

**Files:**
- Modify: `src/server/src/chat/mod.rs` (`Segment::Image { asset_id: Uuid, alt: String }`,
  `MAX_IMAGE_ALT_CHARS = 200`), `src/server/src/chat/rolls.rs` (`BodyChunk::Image { asset_id,
  alt: Option<&str> }`; `parse_doc_link` renamed `parse_ref_span` with an `asset:` arm: grammar
  `asset:<uuid>[|<alt>]`, alt trimmed, empty ⇒ `None`; `RollError::MalformedAssetSpan`,
  `RollError::UnknownAsset`, `RollError::ImagesDisabled` with player-presentable `Display` arms),
  `src/server/src/chat/body.rs` (`Image` arm: `policy.images()` false ⇒ `ImagesDisabled`;
  `repo.get_asset(asset_id)` absent or `world_id != deps.world_id` ⇒ `UnknownAsset`; else
  `Segment::Image { asset_id, alt: alt.unwrap_or("").chars().take(MAX_IMAGE_ALT_CHARS)… }` — refuse
  over-length rather than truncate: `RollError::AltTooLong`)
- Modify: `src/server/src/chat/rolls/tests.rs`, `src/server/src/chat/body/tests.rs`, the
  no-debug-artifacts `Display` test (iterates every `RollError` variant — add the new ones).

- [ ] **Step 1:** failing tests — `scan_body` yields `BodyChunk::Image` for `[[asset:<uuid>|map]]`
  and `[[asset:<uuid>]]`; malformed id ⇒ `MalformedAssetSpan`; ingest with an in-world asset
  (seed a row via the `data::sqlite` asset test helpers — `AssetMeta::unprocessed`) ⇒ message with
  `Image`; a foreign-world asset ⇒ whispered System notice with `UnknownAsset`'s text; `images`
  off ⇒ `ImagesDisabled` notice; exactly one message per attempt in every case.
- [ ] **Step 2:** implement. `cargo test -p shadowcat chat` PASS; clippy; fmt.
- [ ] **Step 3:** `git commit -m "feat(chat): asset image segments from [[asset:]] spans" -- src/server/src/chat/`

### Task 4: sanitizer — no hotlinks, `Sanitized { segments, image_urls }`

**Files:**
- Modify: `src/server/src/chat/sanitize.rs`, `src/server/src/chat/sanitize/tests.rs`, every
  `sanitize(` caller (`chat/mod.rs`, `chat/body.rs`, `chat/link_preview.rs` tests) to read
  `.segments`.

**Behavior (spec §3.4):**
- `pub struct Sanitized { pub segments: Vec<Segment>, pub image_urls: Vec<String> }`;
  `pub fn sanitize(raw, policy) -> Sanitized`.
- `images` off: `rm_tags("img")`; cmark `Event::Start(Tag::Image { .. })`/`End(TagEnd::Image)`
  are replaced by their alt text (a `Text` event of the image's inner text); `image_urls` empty.
- `images` on: markdown image events are consumed into `image_urls` (dest_url) and replaced by
  alt text; html-mode `<img src>` is collected by the `attribute_filter` (returns `None` for
  `src`); after `clean()`, `strip_img_tags(&cleaned)` removes every `<img …>` element from the
  normalized output. Document the safety argument on `strip_img_tags` (ammonia has escaped every
  literal `<`; removing an element from sanitized output cannot create markup).
- Delete the lexical extension allowlist and its comment; the module doc states the invariant:
  ammonia is crossed exactly once and an `<img>` with a `src` never survives `sanitize`.
- `image_urls` are deduped in first-seen order; no cap here (Task 6 caps at enrich).

- [ ] **Step 1:** failing tests — markdown `![a](https://h/x.png)` with images on ⇒ `image_urls ==
  ["https://h/x.png"]` and the html contains no `<img`; html-mode `<img src=… alt=…>` ⇒ same;
  images off ⇒ empty `image_urls`, alt text present, no `<img`; a protocol-relative or
  `javascript:` src never lands in `image_urls`; the existing `url_relative(Deny)` and CSS tests
  unchanged.
- [ ] **Step 2:** implement. `cargo test -p shadowcat chat` PASS; clippy; fmt.
- [ ] **Step 3:** `git commit -m "feat(chat): sanitizer collects image sources and never emits a hotlinked img" -- src/server/src/chat/`

### Task 5: `Provenance::ChatImage`

**Files:**
- Modify: `src/server/src/data/asset.rs` (`Provenance::ChatImage`), `src/server/src/data/asset/tags.rs`
  (`CHAT_IMAGE_TAG = "chat-image"`; `derive` + `provenance_of` arms), `src/server/src/data/asset/tags/tests.rs`

- [ ] **Step 1:** failing tests — `derive` with `ChatImage` yields the tag; `provenance_of`
  round-trips it; the exhaustive provenance-tag test (if present) covers three variants.
- [ ] **Step 2:** implement. `cargo test -p shadowcat asset` PASS.
- [ ] **Step 3:** `git commit -m "feat(assets): chat-image provenance" -- src/server/src/data/asset.rs src/server/src/data/asset/`

### Task 6: `InlineImage` post-publish enrichment

**Files:**
- Modify: `src/server/src/chat/link_preview.rs` (`MAX_INLINE_IMAGES = 4`,
  `MAX_INLINE_IMAGE_BYTES = 4 * 1024 * 1024`; `fetch_image_bytes` gains a `max_bytes: usize`
  parameter — existing callers pass `MAX_IMAGE_BYTES`; `enrich` takes `image_urls: &[String]`
  and, when `policy.images()`, queues `PendingEnrichment::InlineImage { image_url, alt }` for
  the first `MAX_INLINE_IMAGES` that pass `validate_url`, each behind `rate.check(user, now_ms,
  PREVIEW_FETCH_PER_MIN)`), `src/server/src/chat/post_publish.rs`
  (`PendingEnrichment::InlineImage`, `ResolvedEnrichment::NewImageSegment(Segment)`,
  `resolve_inline_image` mirroring `resolve_preview_image`: `with_preview_url_lock` → persisted
  `link_preview_cache` `image_asset_id` hit ⇒ reuse → miss ⇒ `fetch_image_bytes(client, url,
  deadline, MAX_INLINE_IMAGE_BYTES)` → `create_asset_from_bytes` with `Provenance::ChatImage`,
  `created_by: None`, `original_name` = the URL's last path segment or `"image"` →
  `set_link_preview_cache_image`; publish appends `Segment::Image { asset_id, alt }`),
  `src/server/src/chat/mod.rs` (`handle_send_message`/`handle_edit_message` pass
  `sanitized.image_urls` — collect them across every `Text` chunk of the composed body: extend
  `compose_message` to return `(Vec<Segment>, Vec<String>)`), `src/server/src/ws/conn.rs` (no
  change expected — `pending` already spawns `run_pending_enrichments`).
- Tests: `src/server/src/chat/link_preview_ingest_tests.rs`, `src/server/src/chat/post_publish/tests.rs`.

**Alt text:** the markdown alt / html `alt` attribute travels with the URL: `Sanitized.image_urls`
becomes `Vec<ImageSource { url: String, alt: String }>` (adjust Task 4's type before this task
lands — one commit may amend the other; keep the invariant tests).

- [ ] **Step 1:** failing tests — `enrich` queues `InlineImage` only when `images` is on, caps at
  `MAX_INLINE_IMAGES`, never for a `validate_url`-rejected URL; `resolve_inline_image` with
  `build_client_allow_loopback` against a local `image/png` responder creates an asset with the
  `chat-image` derived tag and republishes an `Image` segment appended at the end; a
  `link_preview_cache` hit performs no fetch (assert via the responder's hit counter); a
  blocked-IP URL never connects; a body over `MAX_INLINE_IMAGE_BYTES` is refused; a tombstoned
  message is a no-op (the existing OCC re-read).
- [ ] **Step 2:** implement. `cargo test -p shadowcat chat` PASS; clippy; fmt.
- [ ] **Step 3:** `git commit -m "feat(chat): external images are fetched server-side and asset-ified" -- src/server/src/chat/`

### Task 7: edits keep structured references

**Files:**
- Modify: `src/server/src/chat/mod.rs` (`handle_edit_message` composes the new content through
  `compose_message(.., ScanMode::NoExecute)`; `ComposeError::Inline` ⇒
  `SendMessageError::RollImmutable`; the stored-content `RollEmbed`/`RollButton` check and the
  whisper literal-body rule are unchanged; `Image`/`DocLink`/`Button` chunks are honored),
  `src/server/src/chat/tests.rs` (or its split).

- [ ] **Step 1:** failing tests — an edit whose new content carries `[[doc:<id>|x]]` stores a
  `DocLink`; `[[asset:<id>|x]]` stores an `Image` (in-world check applies); `[[1d6]]` ⇒
  `RollImmutable`; `[[roll:1d6|x]]` in an edit of a message that carries NO roll segment stores a
  `RollButton` (validate-only, never executes), while an edit of a message that ALREADY carries a
  `RollEmbed`/`RollButton` is still refused by the unchanged stored-content check — pin both; a
  whisper edit with a span still scans.
- [ ] **Step 2:** implement; update the doc comments on `handle_edit_message` that say edits never
  call `scan_body`. `cargo test -p shadowcat chat` PASS.
- [ ] **Step 3:** `git commit -m "feat(chat): edits keep doc links, images and buttons; inline rolls stay immutable" -- src/server/src/chat/`

### Task 8: client core + ui-kit `SegmentList`

**Files:**
- Modify: `src/client/core/src/chat-docs.ts` (`image` arm: `{ kind: "image", asset_id: string,
  alt: string }`; `isKnownSegment` + `UnknownSegmentSchema` refuse `"image"`), `chat-docs.test.ts`.
- Create: `src/client/ui-kit/src/SegmentList.svelte`, `src/client/ui-kit/src/SegmentList.test.ts`,
  move `src/modules/chat-card/src/RollTooltip.svelte` + `RollTooltip.test.ts` to
  `src/client/ui-kit/src/` (via `git mv`; adjust imports), export both from
  `src/client/ui-kit/src/index.ts`.
- Modify: `src/modules/chat-card/src/MessageCard.svelte` (segment loop replaced by
  `<SegmentList segments={…} messageId={message.id} channel={sys.channel} />`; the block-roll
  form, recalc menu, header/actions stay), `MessageCard.test.ts` (tests that exercised segment
  rendering move to `SegmentList.test.ts`; card tests assert delegation).
- Modify: `docs/site/modules/chat-card.md` (the `{@html}` sink now lives in ui-kit's
  `SegmentList`; segment kinds list gains `image`).

**Behavior:** `SegmentList` renders every kind `MessageCard` renders today plus `image`
(`<a href={ctx.assets.url(id)} target="_blank" rel="noopener noreferrer"><img
src={ctx.assets.url(id, "preview")} alt={alt} loading="lazy" /></a>`, `max-width: 100%`, a 44px
minimum touch target); it obtains `ctx` through the ui-kit context getter `SheetHost` uses; it
is the ONLY `{@html}` in the client (`rg "\{@html" src/` returns exactly one production hit).

- [ ] **Step 1:** failing tests — `chat-docs` parses/refuses `image`; `SegmentList` renders an
  `image` segment with a `/api/assets/<id>?variant=preview` src and escaped alt; every moved
  renderer test passes in its new home; `MessageCard` delegates.
- [ ] **Step 2:** implement. `pnpm -r test`, `pnpm -r typecheck`, `pnpm lint` PASS; `rg "\{@html"
  src/ --glob '!**/*.test.*'` → one hit, in `SegmentList.svelte`.
- [ ] **Step 3:** `git commit -m "feat(ui-kit): SegmentList becomes the single html sink; image segments" -- src/client/ src/modules/chat-card/ docs/site/modules/chat-card.md`

### Task 9: composer "Insert image" + settings copy

**Files:**
- Modify: `src/modules/chat-composer/src/Composer.svelte` (button `data-testid="image-insert"`
  beside `doc-link-trigger`; `const id = await ctx.pickAsset({ kind: "image" })` — read
  `PickAssetOptions` in `src/client/ui-kit/src/assetPickController.svelte.ts` for the exact
  filter field names; `null` ⇒ no-op; inserts `[[asset:${id}|${label}]]` where `label` is the
  asset's `name` from `ctx.assets`' listing if available else the id's first 8 chars, stripped of
  `[`/`]`/`|` like `insertDocLink`; hidden when the world's `chat-settings` `images` is false —
  read the singleton via `ctx.documents.query(CHAT_SETTINGS_DOC_TYPE)` with the
  `createSubscriber` bridge the composer already uses for its other reactive reads),
  `Composer.test.ts`; i18n key `chat.composer.insertImage` in the ui-kit locale table.
- Modify: `src/modules/game-settings/src/GameSettingsPanel.svelte` (images toggle copy per spec
  §4), `src/modules/game-settings/src/chat-settings.test.ts`; `docs/site/modules/chat-composer.md`,
  `docs/site/modules/game-settings.md`.

- [ ] **Step 1:** failing tests — button hidden when images off; click resolves the mocked
  `pickAsset` and inserts the span at the cursor; cancel inserts nothing.
- [ ] **Step 2:** implement. `pnpm -r test`, `pnpm -r typecheck`, `pnpm lint` PASS.
- [ ] **Step 3:** `git commit -m "feat(chat-composer): insert image via the asset picker" -- src/modules/chat-composer/ src/modules/game-settings/ src/client/ui-kit/ docs/site/modules/`

### Task 10: e2e

**Files:**
- Create: `src/client/core/src/e2e/chat-image.e2e.test.ts` (spec §8: upload via
  `fetch(`${baseUrl}/api/worlds/${world}/assets`)` multipart with the GM cookie — `uploadAsset`
  assumes a browser origin, so post the `FormData` directly; send `[[asset:<id>|map]]` with
  `WsClient.sendChatMessage`; the player's `onCommand` sees `image` with that `asset_id`; a random
  uuid ⇒ the send promise rejects with `UnknownAsset`'s text; enable images first by writing the
  `chat-settings` singleton via an `Intent` `Update` on `/engine/images` — read
  `recalc-roll.e2e.test.ts` and `capabilities.e2e.test.ts` for the connect/intent idioms).
- Create: `src/client/shell/e2e/chat-media.spec.ts` (spec §8; follow `assets.spec.ts` for the
  upload flow and `fixtures.ts` for login; enable images through the Game Settings panel's chat
  section first).

- [ ] **Step 1:** write both; `pnpm --filter @shadowcat/core test:e2e` and `pnpm --filter
  @shadowcat/shell e2e` PASS (run alone; 31999 free).
- [ ] **Step 2:** `git commit -m "test(e2e): chat images over the wire and through the composer" -- src/client/core/src/e2e/ src/client/shell/e2e/`

### Task 11: docs, skills, gates, review

- [ ] `docs/site/protocol.md`: `send_message` row mentions `[[asset:<uuid>|alt]]`; a paragraph
  under the dice-reference note: images are asset-served, external image URLs are fetched
  server-side, YouTube/Vimeo links become oEmbed cards (thumbnail + link, no embed HTML).
- [ ] `docs/design/ARCHITECTURE.md` §6 "Stable asset identity" bullet: chat-image provenance.
- [ ] Skills in `~/.claude/skills/shadowcat-codebase/skills/`: `shadowcat-codebase-chat`
  (`Segment::Image`, `chat::body`, `Sanitized`, `InlineImage`, edit scan, the `{@html}` sink's
  new home, the deleted extension allowlist), `shadowcat-codebase-assets`
  (`Provenance::ChatImage`), `shadowcat-codebase-client-shell` (`SegmentList`). Only edit and
  `git add`/commit the files this task owns (other sessions hold uncommitted edits there). Run
  `node scripts/check-skill-symbol-refs-cli.mjs` + `node scripts/check-skill-api-refs-cli.mjs` +
  `pnpm run test:scripts`; dispatch `shadowcat-codebase:shadowcat-spec-reviewer` on the skill
  diff; commit + push in the plugin repo.
- [ ] `docs/HISTORY.md`: M19a delivery entry; `docs/PLAN.md`: mark M19a done.
- [ ] Full gate run (Global constraints); regenerate + commit `src/types/generated` if any ts-rs
  type changed (none expected in M19a — `Segment` is serde-only).
- [ ] Final review: dispatch `shadowcat-codebase:shadowcat-spec-reviewer` +
  `shadowcat-codebase:shadowcat-code-reviewer` on `git diff origin/main...HEAD` (pre-generate the
  diff to a file; reviewers have no Bash). Address findings. Report to the dispatcher; the
  dispatcher merges.
