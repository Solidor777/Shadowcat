# Docs Sweep 6a — Chat Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: on a Fable-class model use
> `mainline-plan-execution`; otherwise superpowers:subagent-driven-development.
> Steps use checkbox (`- [ ]`) syntax.

**Goal:** Document `src/server/src/chat/` — measured backlog 83
(link_preview.rs 36, mod.rs 23, rolls.rs 13, settings.rs 5, shortcodes.rs 2,
preview_cache.rs 2, commands.rs 2; sanitize.rs already clean) — then flip the
whole chat/ tree (all 8 files) to deny.

**Architecture:** Same calibrated pattern (prior sweep plans' Global
Constraints verbatim). Branch `docs-sweep6a-chat`. Ship with the LOCAL matrix.
Reviews under the no-shell protocol (pre-generated diff + relayed evidence;
reviewers must not run `cargo test`).

**Truthfulness hot spots:** SSRF-guard docs must state BOTH arms — the
DNS-resolution guard AND the literal-IP-host check (the literal-IP blind-spot
fix: reqwest/hyper skip DNS for literal IPs, so the URL check validates them
directly) — citing the enforcing functions, not a generic "SSRF-guarded"
label; sweeps also FIX stale pre-existing docs in scope (the Sweep-5 lesson),
so verify every existing chat doc touched-adjacent claims too;
actor-attribution docs cite the `world_of` pin in `handle_send_message`
(refusing cross-world actor references); dice→chat roll docs match the actual
`rolls.rs` recompute/verification path; preview-cache docs match its eviction/
keying code; message redaction claims match the per-recipient egress sites.

## Model/Effort directives

Mainline (Fable 5, effort high) per standing directive. No-shell final review
pair; fixes pre-merge.

## Buddy-check directives

No high-risk signals (docs + lint attrs only). Standard final review only.

---

### Task 1: chat/link_preview.rs (36)

- [ ] Enumerate live (expect 36); document every item — the fetch pipeline,
  BOTH SSRF-guard arms with enforcing-function citations, size/time budgets,
  HTML meta extraction. Doctests per policy (network-bound → ` ```text `;
  pure parsers/validators runnable where constructible). Gates; commit.

### Task 2: chat/{mod,rolls}.rs (36)

- [ ] Enumerate live (expect 23+13); document — message flow
  (`handle_send_message`'s gates incl. the `world_of` actor pin), channel
  routing, roll embedding/recompute. Doctests per policy. Gates; commit.

### Task 3: chat/{settings,shortcodes,preview_cache,commands}.rs (11)

- [ ] Enumerate live (expect 5+2+2+2); document — chat-settings singleton
  resolution, shortcode expansion, preview cache keying/eviction, slash-command
  parsing. Doctests per policy. Gates; commit.

### Task 4: Deny flip + verify + sync + ship

- [ ] Inner deny pair in ALL 8 chat/ files (commands, link_preview, mod,
  preview_cache, rolls, sanitize, settings, shortcodes — clean files get the
  attr too). Mutation proof on link_preview.rs + one other file; restore via
  python. Full local matrix. Docs-sync: PLAN.md; chat skill ratchet Gotcha.
  No-shell review pair with pre-generated diff + relayed evidence; fix
  findings; merge `--ff-only`; push; delete branch; memory update.

---

## Deferred (logged, not dropped)

- Sweep 6b: dice/ (172). Then client packages, then modules. Then buddy-check
  convergence → final ratchet → skills reference pass.
