# Docs Sweep 4 — HTTP + Auth Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: on a Fable-class model use
> `mainline-plan-execution`; otherwise superpowers:subagent-driven-development.
> Steps use checkbox (`- [ ]`) syntax.

**Goal:** Document `src/server/src/http/` + `src/server/src/auth/` — measured
backlog 109 (routes.rs 54, http/mod.rs 13, error.rs 11, session.rs 11,
throttle.rs 5, invite.rs 4, auth/mod.rs 4, module_routes.rs 3, role.rs 2,
assets.rs 2; embed.rs/password.rs/setup.rs already clean) — then flip both
trees to deny.

**Architecture:** Same calibrated pattern (prior sweep plans' Global
Constraints verbatim). Branch `docs-sweep4-http-auth`. Ship with the LOCAL
matrix. Reviews under the no-shell protocol (pre-generated diff + relayed
evidence; reviewers must not run `cargo test`).

**Truthfulness hot spots:** route docs must state the real authz gate per
route (`require_gm`/`AdminUser`/`AuthUser`/`permission_context`) and cite it;
existence-hiding claims (404-uniform) verified per route; throttle docs cite
the exact budget constants/config keys; session docs must match
`load_or_create_key`'s DB-persistence truth (the Sweep-1 Critical — do not
regress it); invite docs match `consume_invite`'s single guarded UPDATE gate.

## Model/Effort directives

Mainline (Fable 5, effort high) per standing directive. No-shell final review
pair; fixes pre-merge.

## Buddy-check directives

No high-risk signals (docs + lint attrs only). Standard final review only.

---

### Task 1: http/routes.rs (54)

- [ ] Enumerate live (expect 54); document every item — request/response
  structs' fields (wire-visible via serde), route handlers' one-line purpose +
  authz gate citation + failure semantics. Doctests per policy (handlers are
  axum-bound → ` ```text `; pure helpers runnable). Gates; commit.

### Task 2: http/{mod,error,throttle,assets,module_routes}.rs (34)

- [ ] Enumerate live (expect 13+11+5+2+3); document (AppState fields, error
  mapping table AppError→status incl. existence-hiding, throttle budgets with
  config-key cross-refs, upload caps, module-serving guard). Doctests per
  policy. Gates; commit.

### Task 3: auth/{mod,session,invite,role}.rs (21)

- [ ] Enumerate live (expect 4+11+4+2); document (session key persistence
  EXACTLY per `load_or_create_key`; sweep window; invite mint/redeem split;
  ServerRole orthogonality). Doctests per policy (role/pure helpers runnable).
  Gates; commit.

### Task 4: Deny flip + verify + sync + ship

- [ ] Inner deny pair in ALL http/ files (assets, embed, error, mod,
  module_routes, routes, throttle) and ALL auth/ files (invite, mod, password,
  role, session, setup) — clean files get the attr too (same end-state as
  data/ and ws/). Mutation proof on routes.rs + one auth file; restore via
  python. Full local matrix. Docs-sync: PLAN.md; realtime-sync skill Gotcha
  extension (http/auth ratchet); server-ops skill note if its files are
  referenced. No-shell review pair with pre-generated diff + relayed evidence;
  fix findings; merge `--ff-only`; push; delete branch; memory update.

---

## Deferred (logged, not dropped)

- Sweep 5+: scene/ (~200), chat/+dice/ (~180 remaining server), then client
  packages, then modules. Then buddy-check convergence → final ratchet →
  skills reference pass.
