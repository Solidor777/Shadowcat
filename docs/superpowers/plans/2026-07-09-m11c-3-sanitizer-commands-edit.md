# M11c-3 Sanitizer + Commands + Edit/Delete Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Turn Shadowcat chat messages from verbatim plain text into sanitized, command-aware,
editable/deletable content — without ever letting a client author, forge, edit, or delete a message
outside the server-authoritative path.

**Architecture:** The server enriches the c-1 `SendMessage` producer with an `ammonia` +
`pulldown-cmark` sanitizer (gated by a fail-closed `chat-settings` policy Document) and a
server-side command parser (`/me`→Emote, `/roll`→Roll kind; `/w`→whisper). Editing and deleting are
new dedicated server-authoritative frames (`EditMessage`/`DeleteMessage`) that re-run the same
pipeline and publish an authoritative revision; c-1's blanket rejection of client `Update`s to a
message doc stays intact, re-opened only for the server revision via an internal `WriteOrigin`
marker no wire frame can set.

**Tech Stack:** Rust (server), `ammonia` (HTML sanitizer, strict allowlist), `pulldown-cmark`
(Markdown), `sqlx`/SQLite, ts-rs (wire-type export), `serde`.

**Design doc:** `docs/superpowers/specs/2026-07-09-m11c-3-sanitizer-commands-edit-design.md`
(read it before starting — every decision D1–D7 and the §6 authz seam are load-bearing).

## Global Constraints

- **New deps pinned at plan time:** `ammonia` and `pulldown-cmark`, minimal features. After adding
  them, run `cargo bloat --release --crates` (or check `target/release` binary size delta) and
  confirm the increase stays well under the **60 MiB CI binary budget**.
- **Cross-platform:** server builds/tests on macOS, Linux, Windows. No path/shell assumptions in new
  code. Pure-Rust deps only (both `ammonia` and `pulldown-cmark` are pure Rust).
- **`Segment`, `MessageKind`, `MessageSystem`, `ChatContentPolicy` are `serde`-only — NEVER
  ts-rs-exported.** The client mirrors them in Zod at M11d. Only wire frames get `#[derive(TS)]`.
- **New wire frames (`EditMessage`, `DeleteMessage`) ARE ts-rs-exported** and need Zod-mirror parity
  (mirror added in M11d — this checkpoint only exports the Rust side and keeps the existing ts-rs
  export test green).
- **Fail-closed always:** an absent/malformed `chat-settings` doc ⇒ all enrichment OFF ⇒ plain text.
  Unknown/over-cap whisper recipient ⇒ reject the whole request, persist nothing.
- **CSS is always stripped** regardless of any toggle (no `style` attr, no `<style>`/`<link>`).
- **The authz seam (Tasks 5, 7, 8, 9) is ONE coupled surface** — create-exemption / ingress-guard /
  Update-rejection / `WriteOrigin` edit-marker. It gets a mandatory buddy-check + two blind security
  reviews (see "Buddy-check directives"). Do not weaken any one part without re-verifying the others.
- **`kind` is server-authoritative:** no client input path may yield `MessageKind::System`.

## Model/Effort directives

Per the tier-switch checkpoint, the human chose **mainline plan authoring in the originating session**
(Opus 4.8, effort high) rather than dispatching `sdd-plan-writer-*`. Implementation proceeds via
**superpowers:subagent-driven-development** with the project's named `shadowcat-*` agents
(`shadowcat-coder` for implementation, `shadowcat-spec-reviewer` + `shadowcat-code-reviewer` as the
two-reviewer pair), per the project CLAUDE.md multi-agent pipeline directive. `shadowcat-coder` runs
`effort: medium`; both reviewers run `effort: high`; escalate to the `-opus` twins on BLOCKED or
shallow findings.

## Buddy-check directives

This plan carries high-risk signals: an **XSS sanitizer** (Task 3/4) and an **authorization seam
that re-opens a deliberately-closed write path** (Tasks 5, 7, 8, 9). Per the design §6 and the project
convention, fold a **mandatory buddy-check offer** into the final review:

- **Buddy-check target 1 (security core):** `chat/sanitize.rs` — two blind reviewers confirm the XSS
  corpus is neutralized, CSS is always stripped, and every toggle-off actually removes the
  corresponding capability.
- **Buddy-check target 2 (authz seam):** the `WriteOrigin` marker + `apply_intent` exemption +
  edit/delete handlers, reviewed **as a unit** — confirm no client transport can set the marker, no
  client `Update`/`Create`/`Delete` to a message doc survives, and edit cannot re-target
  (`/w`-in-edit) or forge `kind`/`user_owner`/`channel`/`audience`.

Record the buddy-check outcome in the plan-completion notes before merge.

---

## File Structure

| File | Responsibility |
|---|---|
| `src/server/Cargo.toml` | add `ammonia`, `pulldown-cmark` (pinned) |
| `src/server/src/chat/mod.rs` | `Segment::Html`; `MessageSystem.edited_at/deleted_at`; `kind` param on `build_message_doc`; rewired `handle_send_message`; new `handle_edit_message`/`handle_delete_message`; `SendMessageError` variants |
| `src/server/src/chat/settings.rs` | **new** — `ChatContentPolicy`, `CHAT_SETTINGS_DOC_TYPE`, fail-closed `resolve_content_policy` |
| `src/server/src/chat/sanitize.rs` | **new** — `sanitize(raw, policy) -> Vec<Segment>` (ammonia + pulldown-cmark) |
| `src/server/src/chat/commands.rs` | **new** — pure `parse_command(raw) -> ParsedCommand` |
| `src/server/src/data/command.rs` | **new** `WriteOrigin` enum |
| `src/server/src/data/repository.rs` | `apply_intent` gains `origin`; new `member_id_by_username` |
| `src/server/src/data/sqlite.rs` | `apply_intent` honors `origin` (Update-rejection exemption); `member_id_by_username` impl; update internal callers |
| `src/server/src/ws/room.rs` | `Room::publish` gains `origin`, forwards to `apply_intent`; update callers |
| `src/server/src/ws/protocol.rs` | `ClientMsg::EditMessage` / `DeleteMessage` (ts-rs) |
| `src/server/src/ws/conn.rs` | dispatch arms for the two new frames |
| `src/server/tests/chat_content.rs` | **new** — integration tests for enrich/command/edit/delete end-to-end |

Task order is linear (each depends on the prior): **1 → 2 → 3 → 4 → 5 → 6 → 7 → 8 → 9 → 10**.

---

## Task 1: Content model — `Segment::Html`, message edit/delete markers, `kind` threading

**Files:**
- Modify: `src/server/src/chat/mod.rs` (`Segment`, `MessageSystem`, `build_message_doc`, its one call site in `handle_send_message`)
- Test: `src/server/src/chat/mod.rs` (`#[cfg(test)] mod tests`)

**Interfaces:**
- Produces: `Segment::Html { sanitized_html: String }`; `MessageSystem.edited_at: Option<i64>`,
  `MessageSystem.deleted_at: Option<i64>`; `build_message_doc(world_id, user, channel, actor_owner,
  audience, kind, content, now)` (new `kind: MessageKind` param inserted before `content`).

- [ ] **Step 1: Write failing tests** in `chat/mod.rs` tests module:

```rust
#[test]
fn html_segment_tagged_roundtrip() {
    let s = Segment::Html { sanitized_html: "<em>hi</em>".into() };
    let j = serde_json::to_value(&s).unwrap();
    assert_eq!(j["kind"], "html");
    assert_eq!(j["sanitized_html"], "<em>hi</em>");
    assert_eq!(s, serde_json::from_value(j).unwrap());
}

#[test]
fn message_system_omits_absent_edit_delete_markers() {
    let sys = MessageSystem {
        channel: "all".into(),
        user_owner: Uuid::from_u128(1),
        actor_owner: None,
        kind: MessageKind::Normal,
        audience: Audience::Public,
        content: plain_text_content("hi"),
        edited_at: None,
        deleted_at: None,
    };
    let j = serde_json::to_value(&sys).unwrap();
    assert!(j.get("edited_at").is_none(), "None edited_at must not serialize");
    assert!(j.get("deleted_at").is_none(), "None deleted_at must not serialize");
    // Round-trips (a stored c-1 message with no markers deserializes unchanged).
    assert_eq!(sys, serde_json::from_value(j).unwrap());
}

#[test]
fn build_message_doc_threads_kind() {
    let doc = build_message_doc(
        Uuid::from_u128(10), Uuid::from_u128(20), "all".into(), None,
        Audience::Public, MessageKind::Emote, plain_text_content("waves"), 1,
    );
    let sys: MessageSystem = serde_json::from_value(doc.system).unwrap();
    assert_eq!(sys.kind, MessageKind::Emote);
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p shadowcat-server --lib chat::tests`
Expected: FAIL — `Segment::Html` unknown variant, `MessageSystem` missing fields, `build_message_doc` arity mismatch.

- [ ] **Step 3: Implement**

In `chat/mod.rs`, extend `Segment`:

```rust
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Segment {
    /// Literal text. Rendered as a DOM text node by the client (never innerHTML).
    Text { text: String },
    /// A run of ammonia-sanitized HTML (safe by construction; the client renders
    /// it via innerHTML). Produced only by `sanitize` (chat/sanitize.rs).
    Html { sanitized_html: String },
    // Reserved, produced later: RollEmbed (M11d), PreviewCard (c-4), DocLink (M11d).
}
```

Add the two markers to `MessageSystem` (after `content`):

```rust
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub edited_at: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deleted_at: Option<i64>,
```

Add a `kind: MessageKind` parameter to `build_message_doc` (before `content`) and use it instead of
the hardcoded `MessageKind::Normal`. In the `MessageSystem { .. }` literal set `kind` and
`edited_at: None, deleted_at: None`. Update the sole call site in `handle_send_message` to pass
`MessageKind::Normal` for now (Task 6 replaces this with the parsed kind).

- [ ] **Step 4: Run to verify pass**

Run: `cargo test -p shadowcat-server --lib chat::tests`
Expected: PASS (existing c-1 tests still green — they construct `MessageSystem` literals, so update
those literals to include `edited_at: None, deleted_at: None` if the compiler flags them).

- [ ] **Step 5: Commit**

```bash
git add src/server/src/chat/mod.rs
git commit -m "feat(chat/m11c-3): Segment::Html + message edit/delete markers + kind threading"
```

---

## Task 2: `chat-settings` policy Document + fail-closed resolver

**Files:**
- Create: `src/server/src/chat/settings.rs`
- Modify: `src/server/src/chat/mod.rs` (`mod settings; pub use settings::*;`)
- Test: `src/server/src/chat/settings.rs` (`#[cfg(test)] mod tests`)

**Interfaces:**
- Produces: `pub const CHAT_SETTINGS_DOC_TYPE: &str = "chat-settings";`
  `pub struct ChatContentPolicy { markdown, html, images, hyperlinks, emails: bool }` (`Default` =
  all false); `pub async fn resolve_content_policy(repo: &dyn Repository, world_id: Uuid) ->
  ChatContentPolicy`.
- Consumes: `Repository::query_documents(world, doc_type)` (existing).

- [ ] **Step 1: Write failing tests** in `settings.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::sqlite::SqliteRepo;         // adjust to the crate's test repo ctor
    use crate::data::document::{Document, Scope, PermissionSet, DocRole};
    use uuid::Uuid;
    use std::collections::BTreeMap;

    #[test]
    fn default_policy_is_all_off() {
        let p = ChatContentPolicy::default();
        assert!(!p.markdown && !p.html && !p.images && !p.hyperlinks && !p.emails);
    }

    #[tokio::test]
    async fn absent_settings_doc_resolves_to_default() {
        let (repo, world) = crate::chat::settings::tests::world().await;
        assert_eq!(resolve_content_policy(&repo, world).await, ChatContentPolicy::default());
    }

    #[tokio::test]
    async fn malformed_settings_body_resolves_to_default() {
        let (repo, world) = crate::chat::settings::tests::world().await;
        // A chat-settings doc whose /system is not a ChatContentPolicy shape.
        let mut doc = settings_doc(world, serde_json::json!({ "garbage": 1 }));
        insert_raw(&repo, &mut doc).await;
        assert_eq!(resolve_content_policy(&repo, world).await, ChatContentPolicy::default());
    }

    #[tokio::test]
    async fn present_policy_is_read() {
        let (repo, world) = crate::chat::settings::tests::world().await;
        let doc = settings_doc(world, serde_json::json!({
            "markdown": true, "html": false, "images": true, "hyperlinks": true, "emails": false
        }));
        insert_raw(&repo, &doc).await;
        let p = resolve_content_policy(&repo, world).await;
        assert!(p.markdown && p.images && p.hyperlinks && !p.html && !p.emails);
    }
}
```

> **Test-helper note:** reuse the existing chat test scaffolding (see `chat_audience.rs` / the c-2
> integration tests for how a world + a raw document are created — `create_user`, `add_member`,
> and a direct doc insert). `settings_doc(world, system)` builds a `Document` with
> `doc_type: CHAT_SETTINGS_DOC_TYPE` and the given `system`. `insert_raw` persists it via whatever
> low-level insert the existing chat tests use. Do not invent a new persistence path.

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p shadowcat-server --lib chat::settings`
Expected: FAIL — `chat::settings` module does not exist.

- [ ] **Step 3: Implement** `settings.rs`:

```rust
use crate::data::repository::Repository;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Per-world GM chat content policy, stored as the `system` body of the single
/// `chat-settings` config Document. Every field defaults false: absent or
/// malformed settings ⇒ plain text (fail-closed). The toggles only ever WIDEN
/// enrichment from that safe baseline, so a missing/corrupt doc degrades safe.
pub const CHAT_SETTINGS_DOC_TYPE: &str = "chat-settings";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ChatContentPolicy {
    pub markdown: bool,
    pub html: bool,
    pub images: bool,
    pub hyperlinks: bool,
    pub emails: bool,
}

/// Read the world's chat content policy, fail-closed. Absent doc, empty result,
/// or a `system` body that does not deserialize into `ChatContentPolicy` all
/// yield `ChatContentPolicy::default()` (all off).
pub async fn resolve_content_policy(repo: &dyn Repository, world_id: Uuid) -> ChatContentPolicy {
    let docs = match repo.query_documents(world_id, CHAT_SETTINGS_DOC_TYPE).await {
        Ok(d) => d,
        Err(_) => return ChatContentPolicy::default(),
    };
    let Some(doc) = docs.into_iter().next() else {
        return ChatContentPolicy::default();
    };
    serde_json::from_value(doc.system).unwrap_or_default()
}
```

Note `#[serde(default)]` on the struct so a partial `system` (e.g. only `markdown` set) fills the
rest with `false` rather than failing — a partial policy still degrades safe.

Wire the module in `chat/mod.rs`: `mod settings; pub use settings::{ChatContentPolicy,
CHAT_SETTINGS_DOC_TYPE, resolve_content_policy};`

- [ ] **Step 4: Run to verify pass**

Run: `cargo test -p shadowcat-server --lib chat::settings`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/server/src/chat/settings.rs src/server/src/chat/mod.rs
git commit -m "feat(chat/m11c-3): chat-settings policy doc + fail-closed resolver"
```

---

## Task 3: Sanitizer core — deps, ammonia allowlist, Markdown, CSS-always-stripped, XSS corpus

**Files:**
- Modify: `src/server/Cargo.toml`
- Create: `src/server/src/chat/sanitize.rs`
- Modify: `src/server/src/chat/mod.rs` (`mod sanitize; pub use sanitize::sanitize;`)
- Test: `src/server/src/chat/sanitize.rs`

**Interfaces:**
- Produces: `pub fn sanitize(raw: &str, policy: &ChatContentPolicy) -> Vec<Segment>`.
- Consumes: `ChatContentPolicy` (Task 2), `Segment` (Task 1).

- [ ] **Step 1: Add deps.** In `src/server/Cargo.toml` `[dependencies]`, add (pin exact versions at
  implementation time — resolve the latest compatible with the workspace, then pin):

```toml
ammonia = "4"          # pin to the exact resolved version, e.g. "4.1.2"
pulldown-cmark = { version = "0.13", default-features = false, features = ["html"] }
```

Run `cargo build -p shadowcat-server` then `cargo bloat --release --crates | head` (or compare
`target/release` binary size before/after) and record the delta stays well under 60 MiB.

- [ ] **Step 2: Write failing tests** in `sanitize.rs`. Policy helpers: `off()` =
  `ChatContentPolicy::default()`; `md()` = `{ markdown: true, ..default() }`; `html_on()` =
  `{ html: true, ..default() }`.

```rust
#[test]
fn all_off_is_plain_text() {
    assert_eq!(
        sanitize("**bold** <b>x</b>", &off()),
        vec![Segment::Text { text: "**bold** <b>x</b>".into() }],
    );
}

#[test]
fn markdown_renders_to_sanitized_html_run() {
    let segs = sanitize("**bold**", &md());
    match segs.as_slice() {
        [Segment::Html { sanitized_html }] => {
            assert!(sanitized_html.contains("<strong>bold</strong>"), "got {sanitized_html}");
        }
        other => panic!("expected one Html run, got {other:?}"),
    }
}

#[test]
fn script_tag_is_neutralized() {
    for policy in [md(), html_on()] {
        let out = render(&sanitize("<script>alert(1)</script>hi", &policy));
        assert!(!out.contains("<script"), "script survived under {policy:?}: {out}");
        assert!(!out.to_lowercase().contains("alert(1)") || !out.contains("<script"));
    }
}

#[test]
fn event_handler_and_js_url_stripped() {
    let out = render(&sanitize(r#"<a href="javascript:alert(1)" onclick="evil()">x</a>"#, &html_on()));
    assert!(!out.contains("javascript:"), "js url survived: {out}");
    assert!(!out.contains("onclick"), "event handler survived: {out}");
}

#[test]
fn css_is_always_stripped_even_when_html_on() {
    let out = render(&sanitize(r#"<b style="expression(x)">x</b><style>*{}</style>"#, &html_on()));
    assert!(!out.contains("style"), "style survived: {out}");
    assert!(!out.contains("expression"), "css survived: {out}");
}

#[test]
fn raw_html_in_markdown_escaped_when_html_off() {
    // markdown ON, html OFF: the author's raw <b> must NOT become a live tag.
    let out = render(&sanitize("hi <b>x</b>", &md()));
    assert!(!out.contains("<b>"), "raw html leaked through markdown-only: {out}");
}
```

Add a tiny `render` test helper that concatenates a `&[Segment]` to a string (Text verbatim, Html
`sanitized_html`) so assertions read uniformly.

> **XSS corpus:** in addition to the above, add a `xss_corpus_neutralized` test iterating a slice of
> known vectors — `<img src=x onerror=alert(1)>`, `<svg/onload=alert(1)>`, `<iframe src=...>`,
> `<a href="data:text/html,...">`, `<body onload=...>`, `<math><mtext></mtext></math>` — asserting
> that under BOTH `md()` and `html_on()` the rendered output contains no `on`-handler substring,
> no `<script`, no `<iframe`, no `javascript:`, and no `data:text/html`.

- [ ] **Step 3: Run to verify failure**

Run: `cargo test -p shadowcat-server --lib chat::sanitize`
Expected: FAIL — `sanitize` not defined.

- [ ] **Step 4: Implement** `sanitize.rs`:

```rust
use crate::chat::{ChatContentPolicy, Segment};
use pulldown_cmark::{html, Event, Options, Parser};

/// Enrich raw user input into a sanitized `Segment` list under `policy`.
/// INVARIANT: the ONLY producer of `Segment::Html`. ammonia is the single
/// security boundary, crossed exactly once here. All-off ⇒ one `Text` segment
/// (fail-closed baseline, identical to c-1).
pub fn sanitize(raw: &str, policy: &ChatContentPolicy) -> Vec<Segment> {
    if !policy.markdown && !policy.html {
        return vec![Segment::Text { text: raw.to_string() }];
    }
    // Produce an HTML string, then hand the WHOLE thing to ammonia once.
    let html_input = if policy.markdown {
        let mut opts = Options::empty();
        opts.insert(Options::ENABLE_STRIKETHROUGH);
        opts.insert(Options::ENABLE_TABLES);
        let parser = Parser::new_ext(raw, opts).filter(|ev| {
            // When raw HTML is not allowed, drop cmark's raw-HTML events so the
            // author's embedded tags never reach ammonia as markup.
            policy.html || !matches!(ev, Event::Html(_) | Event::InlineHtml(_))
        });
        let mut s = String::new();
        html::push_html(&mut s, parser);
        s
    } else {
        // html-only: feed the raw input straight to ammonia.
        raw.to_string()
    };
    let cleaned = ammonia_for(policy).clean(&html_input).to_string();
    vec![Segment::Html { sanitized_html: cleaned }]
}

/// Build the ammonia sanitizer for `policy`. ammonia's DEFAULT already strips
/// `<script>`/`<style>`, the `style` attribute, and non-allowlisted URL schemes;
/// this only NARROWS it further per toggle (never widens beyond http/https/mailto).
fn ammonia_for(policy: &ChatContentPolicy) -> ammonia::Builder<'static> {
    use std::collections::HashSet;
    let mut b = ammonia::Builder::default();
    // CSS is never permitted (belt-and-suspenders over ammonia's default).
    b.rm_tags(std::iter::once("style"));
    b.rm_tag_attributes("*", std::iter::once("style"));
    if !policy.images { b.rm_tags(std::iter::once("img")); }
    if !policy.hyperlinks { b.rm_tags(std::iter::once("a")); }
    let mut schemes: HashSet<&str> = HashSet::new();
    schemes.insert("http");
    schemes.insert("https");
    if policy.emails { schemes.insert("mailto"); }
    b.url_schemes(schemes);
    b
}
```

> **Version-API note (not a placeholder):** the exact `ammonia::Builder` method names
> (`rm_tags`, `rm_tag_attributes`, `url_schemes`, `attribute_filter`) are confirmed against the
> pinned ammonia version's docs at implementation time; adjust call spelling to the pinned API if it
> differs, keeping the behavior (CSS stripped, per-toggle tag removal, scheme allowlist) identical.

Wire in `chat/mod.rs`: `mod sanitize; pub use sanitize::sanitize;`

- [ ] **Step 5: Run to verify pass**

Run: `cargo test -p shadowcat-server --lib chat::sanitize`
Expected: PASS (all corpus vectors neutralized under both policies).

- [ ] **Step 6: Commit**

```bash
git add src/server/Cargo.toml src/server/src/chat/sanitize.rs src/server/src/chat/mod.rs Cargo.lock
git commit -m "feat(chat/m11c-3): ammonia+pulldown-cmark sanitizer core (XSS corpus, CSS stripped)"
```

---

## Task 4: Sanitizer policy granularity — images / hyperlinks / emails + image content-type

**Files:**
- Modify: `src/server/src/chat/sanitize.rs`
- Test: `src/server/src/chat/sanitize.rs`

**Interfaces:** unchanged signature; extends `ammonia_for` behavior.

- [ ] **Step 1: Write failing tests**:

```rust
#[test]
fn images_off_strips_img() {
    let out = render(&sanitize("![a](https://x.example/a.png)", &md()));  // images off in md()
    assert!(!out.contains("<img"), "img survived with images off: {out}");
}

#[test]
fn images_on_allows_https_png_only() {
    let p = ChatContentPolicy { markdown: true, images: true, ..Default::default() };
    let ok = render(&sanitize("![a](https://x.example/a.png)", &p));
    assert!(ok.contains("<img"), "png image dropped: {ok}");
    // Disallowed extension is rejected (img dropped or src stripped).
    let bad = render(&sanitize("![a](https://x.example/a.exe)", &p));
    assert!(!bad.contains("x.example/a.exe"), "non-image src survived: {bad}");
}

#[test]
fn hyperlinks_off_unwraps_anchor_to_text() {
    let out = render(&sanitize("[label](https://x.example)", &md())); // hyperlinks off in md()
    assert!(!out.contains("<a "), "anchor survived with hyperlinks off: {out}");
    assert!(out.contains("label"), "anchor text lost: {out}");
}

#[test]
fn emails_toggle_gates_mailto() {
    let off = ChatContentPolicy { html: true, hyperlinks: true, ..Default::default() };
    assert!(!render(&sanitize(r#"<a href="mailto:a@b.example">m</a>"#, &off)).contains("mailto:"));
    let on = ChatContentPolicy { html: true, hyperlinks: true, emails: true, ..Default::default() };
    assert!(render(&sanitize(r#"<a href="mailto:a@b.example">m</a>"#, &on)).contains("mailto:"));
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p shadowcat-server --lib chat::sanitize`
Expected: FAIL — image content-type not yet enforced; mailto/anchor gating may partially fail.

- [ ] **Step 3: Implement** — add an image-src content-type/extension filter to `ammonia_for`:

```rust
    // Image src must be an allowlisted raster extension (png/jpg/jpeg/webp/gif).
    // Runs only when images are permitted; an unrecognized src drops the attribute
    // (ammonia then drops the src-less <img>).
    if policy.images {
        b.attribute_filter(|element, attribute, value| {
            if element == "img" && attribute == "src" {
                let lower = value.to_ascii_lowercase();
                let ok = ["http://", "https://"].iter().any(|s| lower.starts_with(s))
                    && [".png", ".jpg", ".jpeg", ".webp", ".gif"]
                        .iter()
                        .any(|ext| lower.split(['?', '#']).next().unwrap_or("").ends_with(ext));
                return if ok { Some(value.into()) } else { None };
            }
            Some(value.into())
        });
    }
```

Confirm `rm_tags(["a"])` when `!hyperlinks` unwraps to text (ammonia's `rm_tags` keeps element
content), and that `mailto` is in `url_schemes` only when `emails`.

- [ ] **Step 4: Run to verify pass**

Run: `cargo test -p shadowcat-server --lib chat::sanitize`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/server/src/chat/sanitize.rs
git commit -m "feat(chat/m11c-3): per-toggle image/link/email gating + image content-type filter"
```

---

## Task 5: `WriteOrigin` marker + `apply_intent` message-Update exemption

**Files:**
- Modify: `src/server/src/data/command.rs` (new `WriteOrigin`)
- Modify: `src/server/src/data/repository.rs` (`apply_intent` signature), `src/server/src/data/sqlite.rs` (impl + all internal callers)
- Modify: `src/server/src/ws/room.rs` (`Room::publish` signature + forward + callers)
- Test: `src/server/src/data/sqlite.rs`

**Interfaces:**
- Produces: `pub enum WriteOrigin { Client, ServerMessageRevision }` (`Copy, Clone, PartialEq, Eq,
  Debug`); `apply_intent(ctx, world_id, ops, ts, origin)`; `Room::publish(repo, ctx, ops, ts,
  origin)`.

- [ ] **Step 1: Write a failing test** in `sqlite.rs` tests (reuse the existing `apply_intent`
  message-Update test scaffolding — see the c-1 test that proves a client Update to a message is
  rejected):

```rust
#[tokio::test]
async fn message_update_rejected_for_client_allowed_for_server_revision() {
    let (repo, world, owner_ctx, msg_id) = seed_owned_message().await; // helper: a stored message owned by a Player
    let change = FieldChange {
        path: "/system/content".into(),
        old: serde_json::json!([{ "kind": "text", "text": "hi" }]),
        new: serde_json::json!([{ "kind": "text", "text": "edited" }]),
    };
    let ops = || vec![Operation::Update { doc_id: msg_id, changes: vec![change.clone()] }];

    // Client origin: still blanket-rejected (c-1 invariant intact).
    let client = repo.apply_intent(&owner_ctx, world, ops(), 2, WriteOrigin::Client).await;
    assert!(matches!(client, Err(DataError::Forbidden)), "client update must be forbidden");

    // Server revision origin: permitted (owner holds WRITE_FIELDS via DocRole::Owner).
    let server = repo
        .apply_intent(&owner_ctx, world, ops(), 3, WriteOrigin::ServerMessageRevision)
        .await;
    assert!(server.is_ok(), "server revision update must be allowed: {server:?}");
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p shadowcat-server --lib sqlite::`
Expected: FAIL — `WriteOrigin` undefined; `apply_intent` arity mismatch.

- [ ] **Step 3: Implement**

In `data/command.rs`:

```rust
/// Who originated a write reaching `apply_intent`. A stored `message` doc's
/// `Update` is blanket-rejected for `Client` (c-1 invariant); `ServerMessageRevision`
/// — set ONLY by the server edit/delete handlers, never derivable from any wire
/// frame — re-opens that path for the sanitized authoritative revision.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WriteOrigin {
    Client,
    ServerMessageRevision,
}
```

In `repository.rs`, add `origin: crate::data::command::WriteOrigin` as the final `apply_intent`
parameter (update the doc-comment to explain the message-Update exemption).

In `sqlite.rs` `apply_intent`, thread `origin` through and change the message-Update rejection
(currently `if cur.doc_type == crate::chat::MESSAGE_DOC_TYPE { return Err(DataError::Forbidden); }`):

```rust
                    if cur.doc_type == crate::chat::MESSAGE_DOC_TYPE
                        && origin != crate::data::command::WriteOrigin::ServerMessageRevision
                    {
                        // Client Updates to a message stay blanket-rejected (c-1).
                        // The server edit/delete path passes ServerMessageRevision to
                        // re-open this ONLY for its sanitized authoritative revision;
                        // the owner/GM check + re-sanitize happen in the handler, and
                        // the ordinary WRITE_FIELDS/OCC checks below still apply here.
                        return Err(DataError::Forbidden);
                    }
```

In `ws/room.rs`, add `origin: WriteOrigin` to `Room::publish` and forward it:
`repo.apply_intent(ctx, self.world_id, ops, ts, origin).await`.

Update ALL callers. Find them:

```bash
grep -rn "\.publish(" src/server/src | grep -v "fn publish"
grep -rn "\.apply_intent(" src/server/src
```

Every existing caller passes `WriteOrigin::Client` (the generic intent path in `conn.rs`,
`handle_send_message`'s create, HTTP `write_ops`, tests). Only the Task 7/8 edit/delete handlers
will pass `ServerMessageRevision`.

- [ ] **Step 4: Run to verify pass**

Run: `cargo test -p shadowcat-server`
Expected: PASS (new test green; all prior tests green with `WriteOrigin::Client` threaded).

- [ ] **Step 5: Commit**

```bash
git add src/server/src/data/command.rs src/server/src/data/repository.rs src/server/src/data/sqlite.rs src/server/src/ws/room.rs src/server/src/ws/conn.rs src/server/src/http/routes.rs
git commit -m "feat(chat/m11c-3): WriteOrigin marker gates the message-Update exemption"
```

---

## Task 6: Command parser + wire commands/audience/sanitizer into `handle_send_message`

**Files:**
- Create: `src/server/src/chat/commands.rs`
- Modify: `src/server/src/chat/mod.rs` (`mod commands;`, rewire `handle_send_message`, extend `SendMessageError`)
- Modify: `src/server/src/data/repository.rs` + `src/server/src/data/sqlite.rs` (`member_id_by_username`)
- Test: `src/server/src/chat/commands.rs` (parser units); `src/server/tests/chat_content.rs` (integration)

**Interfaces:**
- Produces: `pub fn parse_command(raw: &str) -> ParsedCommand` where `pub struct ParsedCommand {
  pub kind: MessageKind, pub whisper_to: Option<Vec<String>>, pub body: String }`;
  `Repository::member_id_by_username(world, username) -> Result<Option<Uuid>, DataError>`.
- Consumes: `sanitize` (T3/4), `resolve_content_policy` (T2), `build_message_doc` w/ `kind` (T1),
  c-2 whisper validation (existing).

- [ ] **Step 1: Write failing parser unit tests** in `commands.rs`:

```rust
#[test]
fn no_command_is_normal_passthrough() {
    let p = parse_command("hello world");
    assert_eq!(p.kind, MessageKind::Normal);
    assert!(p.whisper_to.is_none());
    assert_eq!(p.body, "hello world");
}

#[test]
fn me_variants_set_emote_and_strip_token() {
    for cmd in ["/me waves", "/em waves", "/emote waves"] {
        let p = parse_command(cmd);
        assert_eq!(p.kind, MessageKind::Emote, "{cmd}");
        assert_eq!(p.body, "waves");
    }
}

#[test]
fn roll_sets_roll_kind_and_keeps_expression_verbatim() {
    let p = parse_command("/roll 2d6+3");
    assert_eq!(p.kind, MessageKind::Roll);
    assert_eq!(p.body, "2d6+3");            // stored unparsed/unexecuted (M11d runs it)
    let short = parse_command("/1d20");
    assert_eq!(short.kind, MessageKind::Roll);
    assert_eq!(short.body, "1d20");
}

#[test]
fn whisper_captures_usernames_and_strips_them() {
    let p = parse_command("/w @alice @bob hey there");
    assert_eq!(p.kind, MessageKind::Normal);
    assert_eq!(p.whisper_to, Some(vec!["alice".into(), "bob".into()]));
    assert_eq!(p.body, "hey there");
}

#[test]
fn no_command_yields_system_never() {
    // Exhaustive: no input can produce System via the parser.
    for s in ["/system hi", "system", "/sys", "/me x", "/roll 1d4", "/w @a hi", "plain"] {
        assert_ne!(parse_command(s).kind, MessageKind::System, "{s}");
    }
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p shadowcat-server --lib chat::commands`
Expected: FAIL — `parse_command` undefined.

- [ ] **Step 3: Implement** `commands.rs`:

```rust
use crate::chat::MessageKind;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedCommand {
    pub kind: MessageKind,
    /// Raw `@usernames` from a `/w` command (unresolved). The async caller
    /// resolves these to UUIDs and builds `Audience::Whisper`.
    pub whisper_to: Option<Vec<String>>,
    pub body: String,
}

/// Parse a leading chat command. Pure (no repo/async). `kind` is
/// server-authoritative and can NEVER be `System`. Only a leading token counts;
/// a command mid-message is literal text.
pub fn parse_command(raw: &str) -> ParsedCommand {
    let trimmed = raw.trim_start();
    // Emote.
    for tok in ["/me ", "/em ", "/emote "] {
        if let Some(rest) = trimmed.strip_prefix(tok) {
            return ParsedCommand { kind: MessageKind::Emote, whisper_to: None, body: rest.trim().to_string() };
        }
    }
    // Roll: explicit /roll|/r, or /NdM shorthand.
    for tok in ["/roll ", "/r "] {
        if let Some(rest) = trimmed.strip_prefix(tok) {
            return ParsedCommand { kind: MessageKind::Roll, whisper_to: None, body: rest.trim().to_string() };
        }
    }
    if let Some(expr) = trimmed.strip_prefix('/') {
        if is_dice_shorthand(expr) {
            return ParsedCommand { kind: MessageKind::Roll, whisper_to: None, body: expr.to_string() };
        }
    }
    // Whisper: /w @a @b message
    if let Some(rest) = trimmed.strip_prefix("/w ") {
        let mut names = Vec::new();
        let mut body_start = rest;
        for word in rest.split_whitespace() {
            if let Some(name) = word.strip_prefix('@') {
                names.push(name.to_string());
                // advance body_start past this token
                if let Some(idx) = body_start.find(word) {
                    body_start = &body_start[idx + word.len()..];
                }
            } else {
                break;
            }
        }
        return ParsedCommand {
            kind: MessageKind::Normal,
            whisper_to: if names.is_empty() { None } else { Some(names) },
            body: body_start.trim().to_string(),
        };
    }
    ParsedCommand { kind: MessageKind::Normal, whisper_to: None, body: raw.to_string() }
}

/// `NdM` (optionally with a trailing `+K`/`-K`) — the dice shorthand after `/`.
fn is_dice_shorthand(s: &str) -> bool {
    let core = s.split_whitespace().next().unwrap_or("");
    let mut parts = core.splitn(2, 'd');
    let (Some(n), Some(rest)) = (parts.next(), parts.next()) else { return false };
    !n.is_empty() && n.chars().all(|c| c.is_ascii_digit())
        && rest.chars().next().is_some_and(|c| c.is_ascii_digit())
}
```

> Keep the `/w` token-advance simple and correct; if the inline `find` bookkeeping feels fragile,
> reimplement `body` extraction by counting the leading `@word` tokens and re-joining the remainder
> from `split_whitespace` — the test `whisper_captures_usernames_and_strips_them` is the contract.

Add `Repository::member_id_by_username`:

```rust
// repository.rs (trait)
/// The UUID of a member of `world` whose username matches exactly, or `None`.
/// Used to resolve a `/w @name` whisper target server-side.
async fn member_id_by_username(&self, world: Uuid, username: &str)
    -> Result<Option<Uuid>, DataError>;
```

```rust
// sqlite.rs (impl)
async fn member_id_by_username(&self, world: Uuid, username: &str)
    -> Result<Option<Uuid>, DataError> {
    let row = sqlx::query(
        "SELECT m.user_id FROM world_members m JOIN users u ON u.id = m.user_id \
         WHERE m.world_id = ? AND u.username = ?",
    )
    .bind(world.to_string())
    .bind(username)
    .fetch_optional(&self.pool)
    .await?;
    row.map(|r| Uuid::parse_str(r.get::<String, _>("user_id").as_str())
        .map_err(|e| DataError::OpFailed(e.to_string())))
        .transpose()
}
```

Extend `SendMessageError` with `NotFound`, `Forbidden`, `AudienceLocked` (the latter two used by
Tasks 7/8; add them now to keep the enum stable):

```rust
    /// The target message does not exist (edit/delete).
    NotFound,
    /// The requester is neither the message owner nor a GM (edit/delete).
    Forbidden,
    /// An edit attempted to change audience (a `/w` inside an edit). Frozen.
    AudienceLocked,
```

Rewire `handle_send_message` (replace the `plain_text_content` line and audience handling):

```rust
    // Parse leading command (server-authoritative kind; /w whisper targets).
    let parsed = crate::chat::parse_command(&content);
    // Effective audience: an explicit /w wins; otherwise the c-2 frame field.
    let audience = if let Some(names) = parsed.whisper_to {
        let mut recipients = Vec::with_capacity(names.len());
        for name in &names {
            match repo.member_id_by_username(room.world_id, name).await
                .map_err(SendMessageError::Data)? {
                Some(uid) => recipients.push(uid),
                None => return Err(SendMessageError::UnknownRecipient),
            }
        }
        Audience::Whisper { recipients }
    } else {
        audience
    };
    // Re-validate audience (whisper cap + membership) — covers BOTH front-doors.
    if let Audience::Whisper { recipients } = &audience {
        if recipients.len() > MAX_WHISPER_RECIPIENTS {
            return Err(SendMessageError::TooLong);
        }
        for &r in recipients {
            if repo.member_role(room.world_id, r).await.map_err(SendMessageError::Data)?.is_none() {
                return Err(SendMessageError::UnknownRecipient);
            }
        }
    }
    let policy = crate::chat::resolve_content_policy(repo, room.world_id).await;
    let content_segments = crate::chat::sanitize(&parsed.body, &policy);
    let doc = build_message_doc(
        room.world_id, ctx.user_id, channel, actor_owner,
        audience, parsed.kind, content_segments, now,
    );
    room.publish(repo, ctx, vec![Operation::Create { doc }], now, WriteOrigin::Client)
        .await
        .map_err(SendMessageError::Data)
```

Remove the now-duplicated pre-existing whisper-recipient validation block if it becomes redundant
(the block above validates the effective audience once). Keep the empty/size/rate checks on the raw
`content` at the top unchanged.

- [ ] **Step 4: Write integration tests** in `src/server/tests/chat_content.rs` (new; model it on
  `chat_audience.rs`): seed a world with GM + Players `alice`/`bob`, a `chat-settings` doc with
  `markdown: true`, then drive `handle_send_message` and assert the stored doc:

```rust
#[tokio::test]
async fn me_command_produces_emote() {
    let f = fixture().await;
    let cmd = handle_send_message(&f.room, &f.repo, &f.alice, &f.rate, "all".into(),
        "/me waves".into(), None, Audience::Public, 1, 60).await.unwrap();
    let sys = f.stored_message_system(&cmd).await;
    assert_eq!(sys.kind, MessageKind::Emote);
    assert_eq!(sys.content, vec![Segment::Text { text: "waves".into() }]); // md() default off unless enabled
}

#[tokio::test]
async fn whisper_command_targets_named_user() {
    let f = fixture().await;
    let cmd = handle_send_message(&f.room, &f.repo, &f.alice, &f.rate, "all".into(),
        "/w @bob secret".into(), None, Audience::Public, 1, 60).await.unwrap();
    let doc = f.stored_message_doc(&cmd).await;
    assert_eq!(doc.permissions.default, DocRole::None);      // whisper hides from world
    assert!(doc.permissions.users.contains_key(&f.bob_id));  // bob included
}

#[tokio::test]
async fn unknown_whisper_target_rejects_whole_send() {
    let f = fixture().await;
    let r = handle_send_message(&f.room, &f.repo, &f.alice, &f.rate, "all".into(),
        "/w @nobody hi".into(), None, Audience::Public, 1, 60).await;
    assert!(matches!(r, Err(SendMessageError::UnknownRecipient)));
}

#[tokio::test]
async fn markdown_enriched_when_policy_on() {
    let f = fixture_with_policy(ChatContentPolicy { markdown: true, ..Default::default() }).await;
    let cmd = handle_send_message(&f.room, &f.repo, &f.alice, &f.rate, "all".into(),
        "**bold**".into(), None, Audience::Public, 1, 60).await.unwrap();
    let sys = f.stored_message_system(&cmd).await;
    assert!(matches!(sys.content.as_slice(), [Segment::Html { .. }]));
}
```

- [ ] **Step 5: Run to verify pass**

Run: `cargo test -p shadowcat-server --lib chat::commands && cargo test -p shadowcat-server --test chat_content`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add src/server/src/chat/ src/server/src/data/repository.rs src/server/src/data/sqlite.rs src/server/tests/chat_content.rs
git commit -m "feat(chat/m11c-3): command parser + username resolution + wired send pipeline"
```

---

## Task 7: `EditMessage` frame + `handle_edit_message` + dispatch

**Files:**
- Modify: `src/server/src/ws/protocol.rs` (`ClientMsg::EditMessage`)
- Modify: `src/server/src/chat/mod.rs` (`handle_edit_message`)
- Modify: `src/server/src/ws/conn.rs` (dispatch arm)
- Test: `src/server/tests/chat_content.rs`

**Interfaces:**
- Produces: `ClientMsg::EditMessage { message_id: Uuid, content: String }`;
  `pub async fn handle_edit_message(room, repo, ctx, rate, message_id, content, now,
  budget_per_min) -> Result<Command, SendMessageError>`.

- [ ] **Step 1: Write failing integration tests** in `chat_content.rs`:

```rust
#[tokio::test]
async fn owner_can_edit_and_content_resanitizes() {
    let f = fixture_with_policy(ChatContentPolicy { markdown: true, ..Default::default() }).await;
    let sent = handle_send_message(&f.room, &f.repo, &f.alice, &f.rate, "all".into(),
        "first".into(), None, Audience::Public, 1, 60).await.unwrap();
    let id = f.message_id(&sent).await;
    let edited = handle_edit_message(&f.room, &f.repo, &f.alice, &f.rate, id, "**second**".into(), 2, 60).await.unwrap();
    let sys = f.stored_message_system(&edited).await;
    assert!(matches!(sys.content.as_slice(), [Segment::Html { .. }]));
    assert_eq!(sys.edited_at, Some(2));
}

#[tokio::test]
async fn non_owner_non_gm_cannot_edit() {
    let f = fixture().await;
    let sent = handle_send_message(&f.room, &f.repo, &f.alice, &f.rate, "all".into(),
        "hi".into(), None, Audience::Public, 1, 60).await.unwrap();
    let id = f.message_id(&sent).await;
    let r = handle_edit_message(&f.room, &f.repo, &f.bob, &f.rate, id, "hax".into(), 2, 60).await;
    assert!(matches!(r, Err(SendMessageError::Forbidden)));
}

#[tokio::test]
async fn gm_can_edit_players_message() {
    let f = fixture().await;
    let sent = handle_send_message(&f.room, &f.repo, &f.alice, &f.rate, "all".into(),
        "hi".into(), None, Audience::Public, 1, 60).await.unwrap();
    let id = f.message_id(&sent).await;
    assert!(handle_edit_message(&f.room, &f.repo, &f.gm, &f.rate, id, "moderated".into(), 2, 60).await.is_ok());
}

#[tokio::test]
async fn edit_cannot_retarget_audience() {
    let f = fixture().await;
    let sent = handle_send_message(&f.room, &f.repo, &f.alice, &f.rate, "all".into(),
        "hi".into(), None, Audience::Public, 1, 60).await.unwrap();
    let id = f.message_id(&sent).await;
    let r = handle_edit_message(&f.room, &f.repo, &f.alice, &f.rate, id, "/w @bob sneaky".into(), 2, 60).await;
    assert!(matches!(r, Err(SendMessageError::AudienceLocked)));
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p shadowcat-server --test chat_content`
Expected: FAIL — `handle_edit_message` undefined; `EditMessage` variant missing.

- [ ] **Step 3: Implement**

In `protocol.rs`, add to `ClientMsg` (ts-rs exported like `SendMessage`):

```rust
    /// Edit an existing message the requester owns (or any, if GM). The server
    /// re-runs the sanitize+command pipeline; audience/channel are frozen.
    EditMessage { message_id: Uuid, content: String },
```

In `chat/mod.rs`:

```rust
#[allow(clippy::too_many_arguments)]
pub async fn handle_edit_message(
    room: &Room,
    repo: &dyn Repository,
    ctx: &PermissionContext,
    rate: &PingRateLimiter,
    message_id: Uuid,
    content: String,
    now: i64,
    budget_per_min: usize,
) -> Result<Command, SendMessageError> {
    if content.trim().is_empty() { return Err(SendMessageError::Empty); }
    if content.chars().count() > MAX_MESSAGE_CHARS { return Err(SendMessageError::TooLong); }
    if !rate.check(ctx.user_id, now, budget_per_min) { return Err(SendMessageError::RateLimited); }

    let cur = repo.get_document(message_id).await.map_err(SendMessageError::Data)?
        .ok_or(SendMessageError::NotFound)?;
    if cur.doc_type != MESSAGE_DOC_TYPE { return Err(SendMessageError::NotFound); }
    // Authorize: message owner OR a GM.
    let is_gm = ctx.world_role == crate::data::document::WorldRole::Gm;
    if cur.owner != Some(ctx.user_id) && !is_gm { return Err(SendMessageError::Forbidden); }

    let parsed = crate::chat::parse_command(&content);
    // Audience is frozen on edit — a /w in an edit is rejected.
    if parsed.whisper_to.is_some() { return Err(SendMessageError::AudienceLocked); }

    let policy = crate::chat::resolve_content_policy(repo, room.world_id).await;
    let segments = crate::chat::sanitize(&parsed.body, &policy);

    // Build the revised system: new content + kind, edited_at=now; preserve
    // channel/user_owner/actor_owner/audience/deleted_at from the stored doc.
    let mut sys: MessageSystem = serde_json::from_value(cur.system.clone())
        .map_err(|e| SendMessageError::Data(DataError::OpFailed(e.to_string())))?;
    sys.content = segments;
    sys.kind = parsed.kind;
    sys.edited_at = Some(now);
    let new_system = serde_json::to_value(&sys)
        .map_err(|e| SendMessageError::Data(DataError::OpFailed(e.to_string())))?;

    let op = Operation::Update {
        doc_id: message_id,
        changes: vec![FieldChange { path: "/system".into(), old: cur.system, new: new_system }],
    };
    room.publish(repo, ctx, vec![op], now, WriteOrigin::ServerMessageRevision)
        .await
        .map_err(SendMessageError::Data)
}
```

Import `FieldChange` and `DataError` as needed. In `conn.rs`, add the dispatch arm (mirroring
`SendMessage`):

```rust
                        Ok(ClientMsg::EditMessage { message_id, content }) => {
                            if let Err(e) = crate::chat::handle_edit_message(
                                &room, repo.as_ref(), &ctx, &message_rate,
                                message_id, content, now_millis(), MESSAGE_RATE_PER_MIN,
                            ).await {
                                tracing::debug!(world = %world_id, user = %user_id, ?e, "edit rejected");
                            }
                        }
```

- [ ] **Step 4: Run to verify pass**

Run: `cargo test -p shadowcat-server --test chat_content && cargo test -p shadowcat-server`
Expected: PASS (ts-rs export test still green — new frame exports fine).

- [ ] **Step 5: Commit**

```bash
git add src/server/src/ws/protocol.rs src/server/src/chat/mod.rs src/server/src/ws/conn.rs src/server/tests/chat_content.rs
git commit -m "feat(chat/m11c-3): EditMessage frame + validated sanitizing edit (audience frozen)"
```

---

## Task 8: `DeleteMessage` frame + `handle_delete_message` (soft tombstone) + dispatch

**Files:**
- Modify: `src/server/src/ws/protocol.rs` (`ClientMsg::DeleteMessage`)
- Modify: `src/server/src/chat/mod.rs` (`handle_delete_message`)
- Modify: `src/server/src/ws/conn.rs` (dispatch arm)
- Test: `src/server/tests/chat_content.rs`

**Interfaces:**
- Produces: `ClientMsg::DeleteMessage { message_id: Uuid }`; `pub async fn handle_delete_message(room,
  repo, ctx, message_id, now) -> Result<Command, SendMessageError>`.

- [ ] **Step 1: Write failing tests**:

```rust
#[tokio::test]
async fn owner_soft_delete_clears_content_and_keeps_doc() {
    let f = fixture().await;
    let sent = handle_send_message(&f.room, &f.repo, &f.alice, &f.rate, "all".into(),
        "secret".into(), None, Audience::Public, 1, 60).await.unwrap();
    let id = f.message_id(&sent).await;
    handle_delete_message(&f.room, &f.repo, &f.alice, id, 2).await.unwrap();
    let doc = f.repo.get_document(id).await.unwrap().expect("doc still present (tombstone)");
    let sys: MessageSystem = serde_json::from_value(doc.system).unwrap();
    assert!(sys.content.is_empty(), "content cleared");
    assert_eq!(sys.deleted_at, Some(2));
}

#[tokio::test]
async fn non_owner_non_gm_cannot_delete() {
    let f = fixture().await;
    let sent = handle_send_message(&f.room, &f.repo, &f.alice, &f.rate, "all".into(),
        "hi".into(), None, Audience::Public, 1, 60).await.unwrap();
    let id = f.message_id(&sent).await;
    assert!(matches!(
        handle_delete_message(&f.room, &f.repo, &f.bob, id, 2).await,
        Err(SendMessageError::Forbidden)
    ));
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p shadowcat-server --test chat_content`
Expected: FAIL — `handle_delete_message` / `DeleteMessage` missing.

- [ ] **Step 3: Implement**

`protocol.rs`:

```rust
    /// Soft-delete a message the requester owns (or any, if GM): the doc stays
    /// in the sequenced log as a tombstone (content cleared, deleted_at set).
    DeleteMessage { message_id: Uuid },
```

`chat/mod.rs`:

```rust
pub async fn handle_delete_message(
    room: &Room,
    repo: &dyn Repository,
    ctx: &PermissionContext,
    message_id: Uuid,
    now: i64,
) -> Result<Command, SendMessageError> {
    let cur = repo.get_document(message_id).await.map_err(SendMessageError::Data)?
        .ok_or(SendMessageError::NotFound)?;
    if cur.doc_type != MESSAGE_DOC_TYPE { return Err(SendMessageError::NotFound); }
    let is_gm = ctx.world_role == crate::data::document::WorldRole::Gm;
    if cur.owner != Some(ctx.user_id) && !is_gm { return Err(SendMessageError::Forbidden); }

    let mut sys: MessageSystem = serde_json::from_value(cur.system.clone())
        .map_err(|e| SendMessageError::Data(DataError::OpFailed(e.to_string())))?;
    sys.content = Vec::new();
    sys.deleted_at = Some(now);
    let new_system = serde_json::to_value(&sys)
        .map_err(|e| SendMessageError::Data(DataError::OpFailed(e.to_string())))?;
    let op = Operation::Update {
        doc_id: message_id,
        changes: vec![FieldChange { path: "/system".into(), old: cur.system, new: new_system }],
    };
    room.publish(repo, ctx, vec![op], now, WriteOrigin::ServerMessageRevision)
        .await
        .map_err(SendMessageError::Data)
}
```

`conn.rs`:

```rust
                        Ok(ClientMsg::DeleteMessage { message_id }) => {
                            if let Err(e) = crate::chat::handle_delete_message(
                                &room, repo.as_ref(), &ctx, message_id, now_millis(),
                            ).await {
                                tracing::debug!(world = %world_id, user = %user_id, ?e, "delete rejected");
                            }
                        }
```

- [ ] **Step 4: Run to verify pass**

Run: `cargo test -p shadowcat-server`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/server/src/ws/protocol.rs src/server/src/chat/mod.rs src/server/src/ws/conn.rs src/server/tests/chat_content.rs
git commit -m "feat(chat/m11c-3): DeleteMessage frame + soft-tombstone delete"
```

---

## Task 9: Authz-seam integration proof (client cannot author/edit/delete/forge)

**Files:**
- Test: `src/server/tests/chat_content.rs` (or extend `chat_audience.rs`)

This task adds NO production code — it proves the §6 coupled seam end-to-end and is the anchor for
the mandatory buddy-check.

- [ ] **Step 1: Write the seam tests**:

```rust
#[tokio::test]
async fn client_intent_update_to_message_still_forbidden() {
    let f = fixture().await;
    let sent = handle_send_message(&f.room, &f.repo, &f.alice, &f.rate, "all".into(),
        "hi".into(), None, Audience::Public, 1, 60).await.unwrap();
    let id = f.message_id(&sent).await;
    // A raw client Update (WriteOrigin::Client) forging kind=System must be rejected.
    let op = Operation::Update {
        doc_id: id,
        changes: vec![FieldChange {
            path: "/system/kind".into(),
            old: serde_json::json!("normal"),
            new: serde_json::json!("system"),
        }],
    };
    let r = f.repo.apply_intent(&f.alice, f.world, vec![op], 2, WriteOrigin::Client).await;
    assert!(matches!(r, Err(DataError::Forbidden)));
}

#[tokio::test]
async fn client_intent_create_and_delete_message_still_forbidden_at_ingress() {
    // ops_target_message rejects a raw client Create/Delete of a message doc.
    let doc = crate::chat::build_message_doc(/* ... a forged message doc ... */);
    assert!(crate::chat::ops_target_message(&[Operation::Create { doc: doc.clone() }]));
    assert!(crate::chat::ops_target_message(&[Operation::Delete { doc }]));
}
```

- [ ] **Step 2: Run**

Run: `cargo test -p shadowcat-server --test chat_content`
Expected: PASS — proves the seam holds from the client side.

- [ ] **Step 3: Commit**

```bash
git add src/server/tests/chat_content.rs
git commit -m "test(chat/m11c-3): prove client cannot author/edit/delete/forge a message"
```

---

## Task 10: Docs + reviewed skill-update gate

**Files:**
- Modify: `docs/PLAN.md` (M11c-3 → DONE), `src/server/src/chat/...` doc-comments as needed
- Modify: `.claude/skills/shadowcat-codebase-chat/SKILL.md`

- [ ] **Step 1:** Update `shadowcat-codebase-chat` to capture the new seams: the `EditMessage`/
  `DeleteMessage` frames, `handle_edit_message`/`handle_delete_message`, the `WriteOrigin` marker and
  the `apply_intent` Update-exemption, the `chat-settings` fail-closed policy doc + `sanitize`
  producer, and the `parse_command` server-authoritative kind + `/w` second front-door. Update the
  "Hard invariants" (the coupled seam is now four-part: create-exemption / ingress-guard /
  Update-rejection / edit-marker) and "Gotchas" (Segment now has `Html`; edit re-runs the pipeline
  and freezes audience; delete is a soft tombstone).

- [ ] **Step 2:** Update `docs/PLAN.md` M11c-3 entry to DONE with a one-paragraph summary and the
  plan/spec links.

- [ ] **Step 3: Reviewed skill-update gate.** Dispatch `shadowcat-spec-reviewer` on the
  `shadowcat-codebase-chat` diff to confirm it accurately captures the implemented changes (no
  omission/drift/broken pointer). Fix any gaps.

- [ ] **Step 4: Commit**

```bash
git add docs/PLAN.md .claude/skills/shadowcat-codebase-chat/SKILL.md src/server/src/chat/
git commit -m "docs(chat/m11c-3): update chat skill + PLAN for sanitizer/commands/edit/delete"
```

---

## Final verification (before merge)

- [ ] `cargo test -p shadowcat-server` — all green.
- [ ] `cargo clippy -p shadowcat-server --all-targets -- -D warnings` — clean.
- [ ] `cargo fmt --check`.
- [ ] `cargo build -p shadowcat-server --release` succeeds; binary-size delta from the two new deps
  recorded and well under 60 MiB.
- [ ] ts-rs export test regenerates `EditMessage`/`DeleteMessage` bindings without touching
  `Segment`/`MessageSystem`/`ChatContentPolicy` (those stay Rust-only).
- [ ] **Mandatory buddy-check** run on the two targets in "Buddy-check directives"; outcome recorded.
- [ ] Two blind security reviews of the sanitizer + authz seam; findings resolved.

## Self-review (plan vs. spec)

- **§1 sanitizer** → Tasks 3–4 (deps, ammonia, markdown, CSS-strip, per-toggle, image content-type).
- **§2 content model** → Task 1 (`Segment::Html`; typed Link/Image correctly deferred per the design refinement).
- **§3 chat-settings fail-closed** → Task 2.
- **§4 command parser (kind + `/w` second front-door, System unforgeable)** → Tasks 6 (parser + wiring).
- **§5 edit (audience frozen) + delete (soft tombstone)** → Tasks 7, 8.
- **§6 coupled authz seam (WriteOrigin marker)** → Task 5 (mechanism) + Task 9 (proof) + Task 10 (skill).
- **§8 testing** → per-task tests + Task 9 seam proof + final verification.
- **§9 out-of-scope (dice exec, previews, client render, channel seeding)** → not implemented; noted in tasks as deferred.
