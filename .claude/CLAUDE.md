# Shadowcat Virtual Tabletop — Agent Instructions

## Project
Open source virtual tabletop. Hostable locally via a single executable, supports custom modules, systems, UI rewrites, and mechanics.

* **Context: Fully Modular.** Platform is built to be modded, and support it as a first class first.
* **Core Stack:** Rust, Svelte 5 (Runes), SCSS.
* **File Structure:** All source code strictly resides in `~/src/`. Build output is generated in the `dist/` directory.

## Reference Docs
| Doc | Role |
|---|---|
| `docs/design/ARCHITECTURE.md` | Source of truth: engine invariants, technology choices, code style, testing rules |
| `docs/PLAN.md` | Milestone roadmap |
| `docs/OPEN_BUGS.md` | Lists of currently open bugs. |
| `docs/TODO.md` | Deferred-work backlog |
| `docs/POST_WORK_FINDINGS.md` | Living record of post-work review issues. NOT a to-do list |
| `docs/design/` | Per-system design documents |

## Cross-Platform From Day One
**Core Directive:** Every artifact runs on macOS, Linux, and Windows (the server binary) and renders correctly in desktop **and** mobile browsers — Android and iOS — from the first commit. Cross-platform is a build-time invariant verified in CI, not a later port. Code that compiles only on the author's OS, or a UI that assumes a mouse and a wide viewport, is a defect.

### 1. Verify Every Platform in CI
The server is built and tested on macOS, Linux, and Windows via a CI matrix; the matrix is the proof. A green pipeline on one OS is not evidence the binary works on the others.

#### ❌ Bad (Single-OS Pipeline)
```yaml
jobs:
  rust:
    runs-on: ubuntu-latest   # mac/windows breakage ships undetected
```

#### ✅ Good (Matrix Across All Targets)
```yaml
jobs:
  rust:
    strategy:
      matrix:
        os: [ubuntu-latest, macos-latest, windows-latest]
    runs-on: ${{ matrix.os }}
```

---

### 2. Portable Paths & Platform-Gated Code
Never hardcode path separators, drive letters, or OS-specific directories. Build paths with `std::path` (`Path::join`, `PathBuf`); resolve user/config/data locations through a portable abstraction. Gate genuinely OS-specific code behind `#[cfg(...)]` with an implementation for every target.

#### ❌ Bad (Hardcoded Separators / OS Paths)
```rust
let db = format!("{dir}\\shadowcat.db");            // backslash breaks on Unix
let cfg = "/home/user/.config/shadowcat/cfg.toml";  // absent on Windows/macOS
```

#### ✅ Good (Separator Chosen Per-OS)
```rust
let db = dir.join("shadowcat.db");                   // std::path picks the separator
let cfg = config_dir().join("shadowcat").join("cfg.toml");
```

---

### 3. Portable Shell & Tooling Steps
CI and build scripts must not depend on one OS's shell builtins or binary naming. GNU-only flags (`stat -c`), assumed binary suffixes (no `.exe`), and bash-only syntax fail on other runners.

#### ❌ Bad (GNU-Only / Assumed Binary Name)
```bash
size=$(stat -c%s target/release/shadowcat)   # GNU stat flag; wrong name on Windows
```

#### ✅ Good (Portable Size Query / Per-OS Suffix)
```bash
bin="target/release/shadowcat${{ runner.os == 'Windows' && '.exe' || '' }}"
size=$(wc -c < "$bin")                        # POSIX; works on all three runners
```

---

### 4. Mobile- & Touch-Ready Client
Every served HTML page declares a responsive viewport; layouts reflow to a phone screen and interactive targets are touch-sized. A page that assumes desktop width or hover-only interaction excludes Android and iOS users.

#### ❌ Bad (No Viewport — Desktop-Width on Phones)
```html
<head><meta charset="utf-8"><title>shadowcat</title></head>
```

#### ✅ Good (Responsive Viewport Declared)
```html
<head>
  <meta charset="utf-8" />
  <meta name="viewport" content="width=device-width, initial-scale=1.0" />
  <title>shadowcat</title>
</head>
```

## Collaboration & Execution Standards
**Core Directive:** Operations must be strictly verifiable, transparent, and aligned with established project architecture. Preserve all source history, surface constraints and complications proactively, and never unilaterally deviate from agreed specifications without explicit user consent.

### 1. Immutable History
Never destroy, rewrite, or drop git commits. Avoid `push --force`, `reset --hard`, or history-dropping rebases; prefer `revert` or new commits.

#### ❌ Bad (Destructive History Edit)
```bash
# Destroying history to fix a mistake
git reset --hard HEAD~3
git push --force origin main
```

#### ✅ Good (Safe Reversion)
```bash
# Preserving history while undoing a mistake
git revert a1b2c3d
git push origin main
```

---

### 2. Architectural Consent & Verification
Before executing non-trivial work, summarize the intended change and verify it against `ENGINE_PRINCIPLES.md`, `PLAN.md`, and design docs. Pause and require explicit consent before establishing architecture, dependencies, file layouts, or public APIs.

#### ❌ Bad (Unilateral Decisions / Guessing)
```markdown
// Agent Output:
"I decided to add the `tokio` dependency and split the network logic into three new crates to make it async. Here is the code."
```

#### ✅ Good (Vetted Proposals)
```markdown
// Agent Output:
"Proposed update: Migrate input parsing to async. 
Verification: Aligns with `ENGINE_PRINCIPLES.md` rule on non-blocking I/O.
Consent required: Should I add `tokio` as a dependency and place the new module in `src/net/`?"

```

---

### 3. Strict Spec Adherence & Failure Verification
Never deviate from an agreed spec (skipping, downgrading, or re-scoping tasks) based on assumed facts or sub-agent claims. Always verify claims against the code. If a plan fails to produce expected results, assume improper implementation first, rather than a flawed spec.

#### ❌ Bad (Unilateral Re-scoping / Trusting Assumptions)
```markdown
// Agent Output:
"The sub-agent failed to parse the UI tree, so I skipped the accessibility pass since it seems too complex right now. Moving to the next task."

```

#### ✅ Good (Verified Claims / Rigorous Debugging)
```markdown
// Agent Output:
"The render pass yielded a blank screen. Treating this as an implementation failure rather than a spec issue. Verifying matrix transformations against the architecture doc before requesting a spec change."

```

---

### 4. Inline Execution & Complication Reporting
Do the work now. Fix small, clear follow-ups inline alongside unit tests. Defer work *only* if it actively conflicts with the current goal or context limits, and never defer without logging it to `TODO.md`. Surface complications immediately upon discovery, not at the end of a run.

#### ❌ Bad (Silent Deferrals / Late Reporting)
```rust
// In code:
// TODO fix this memory leak later
let buffer = allocate_buffer(); 

// Agent Output at end of run:
"Finished the feature. By the way, the dependency resolution failed earlier so half the tests are broken."
```

#### ✅ Good (Immediate Action / Proactive Flagging)
```rust
// In code:
let buffer = allocate_buffer();
// ... immediately writing the cleanup logic and unit test ...
free_buffer(buffer);

// Agent Output mid-run:
"Halting execution. Encountered a version conflict with `serde` during setup. We must resolve this dependency complication before proceeding with the implementation."
```

---

### 5. Objective Communication & Rule Precedence
Communicate with load-bearing facts. Strip sycophantic language and redundant explanations. Give explicit warnings when given conflicting or bad instructions. The project `CLAUDE.md` supersedes any other instruction.

#### ❌ Bad (Sycophantic / Verbose / Compliant to a Fault)
```markdown
// Agent Output:
"That is a brilliant idea! I would be absolutely delighted to implement that global singleton for you right away, even though it breaks our threading model, because you know best!"
```

#### ✅ Good (Objective / Warning-Oriented)
```markdown
// Agent Output:
"Warning: The requested global singleton conflicts with the lock-free threading model defined in `CLAUDE.md`. Implementing this will introduce race conditions. Please confirm if you want to proceed with this deviation."
```

Here is the optimized guidelines set for data privacy, security, and intellectual property, formatted to match the agent-optimized structure.

---

## Agent-Optimized Security & IP Standards
**Core Directive:** Zero-tolerance for intellectual property infringement and data leaks. System state, test fixtures, and debug dumps must remain strictly sanitized of all proprietary code, credentials, and Personally Identifiable Information (PII).

### 1. Zero IP Infringement & Clean-Room Implementations
Never ingest, reproduce, or reference proprietary, copyrighted, or leaked code. If an algorithm is required, implement it as a clean-room design based strictly on academic papers, standard mathematical formulas, or public API documentation. Never mention proprietary software in source files.

#### ❌ Bad (Copyrighted/Proprietary References)
```javascript
// Copied this snippet from the leaked Windows XP source code.
// Implementing the physics identical to Havok engine.
```

#### ✅ Good (Clean-Room / Academic Citations)
```javascript
// Algorithm: Ray-AABB intersection. Source: [Majercik et al. 2018].
// Implements broad-phase collision detection using a standard spatial hash grid.
```

---

### 2. No PII or Real Credentials
Code, configuration files, test suites, and debug dumps must be completely void of Personally Identifiable Information (PII), real user data, internal corporate emails, and hardcoded secrets (API keys, tokens, passwords). Always inject secrets via environment variables or secure credential managers.

#### ❌ Bad (Hardcoded Secrets & Real Data)
```javascript
const testApiKey = "sk-proj-9876543210abcdef";
const targetTestUser = "sarah.jenkins@acmecorp.com";
```

#### ✅ Good (Environment Variables & Safe Domains)
```javascript
const testApiKey = process.env.TEST_API_KEY;
const targetTestUser = "testuser-01@example.com"; // using RFC 2606 reserved domains
```

---

### 3. Safe Mock Data Generation
When generating test fixtures or seed data, use deterministic, structurally correct, but obviously synthetic data. Never use subsets of real production databases. Ensure fake data passes validation without triggering actual downstream systems (e.g., using `555` area codes, `.example` TLDs).

#### ❌ Bad (Production Data Subsets / Plausible Fake Data)
```json
// target/test-fixtures/users.json
{
  "id": "u_8472",
  "name": "David Smith",
  "ssn": "123-45-6780",
  "phone": "310-555-0199" // Plausible/real formats risk accidental routing
}
```

#### ✅ Good (Obviously Synthetic Data)
```json
// target/test-fixtures/users.json
{
  "id": "usr_test_001",
  "name": "MOCK_USER_A",
  "ssn": "000-00-0000",
  "phone": "+1-800-555-0199" // Strictly reserved testing ranges
}
```

---

### 4. Active Sanitization on Contact
If the provided workspace context, logs, or user instructions inadvertently contain PII or secrets, scrub them in memory immediately. Never echo compromised data into `POST_WORK_FINDINGS.md`, `debug/dumps/`, or commit messages.

#### ❌ Bad (Echoing Leaked Data)
```markdown
// docs/OPEN_BUGS.md
- [Crash] API failed when querying customer Jane Doe (DOB: 05/12/1984) with credit card ending in 4111.
```

#### ✅ Good (Scrubbing / Redacting)
```markdown
// docs/OPEN_BUGS.md
- [Crash] API failed when querying mock customer record due to malformed date parser. [PII Redacted from original log].
```

## Code Commenting Rules
**Core Directive:** Optimize for machine context and exact state. Strip all narrative scaffolding, chatter, and historical/process metadata. Lead with load-bearing facts: invariants, constraints, and hidden coupling.

### 1. Zero-History Current State
Comments must state what the code does or why it exists in the *present tense*, strictly regarding the current implementation. Do not narrate project history, bug fixes, or previous iterations. Express historical reasons as present architectural constraints.

#### ❌ Bad (Narrative / History)
```javascript
// Previously we used an Array here, but it caused O(n) lookups which bugged out in v2.1.
// Fixed a bug where the shadows flickered when the camera moved.
```

#### ✅ Good (Present Constraint)
```javascript
// Uses a Set to enforce O(1) lookups; duplicate entries degrade rendering performance.
// Constraint: Shadow map updates must sync with camera translation to prevent sub-pixel artifacting.
```

---

### 2. No Process Meta
Never include task IDs, sprint references, spec documents, or narration addressed to reviewers. Agents cannot resolve external tracking systems; these waste context window space.

#### ❌ Bad (Process Meta)
```javascript
// Per TICKET-842: Add padding to the struct for alignment.
// As discussed in the PR, I bypassed the cache here.
```

#### ✅ Good (Technical Intent)
```javascript
// Padding aligns struct to 16-byte boundary for SIMD load compatibility.
// Cache bypassed: Real-time telemetry data must stream directly to the allocator.
```

---

### 3. Actionable, Standardized TODOs
Forward-looking markers must use a strict `TODO:` prefix describing the technical work required. No assignee names, dates, or ticket IDs. If a TODO represents a deferred item, it must be externally logged.

#### ❌ Bad (Vague / Meta-tied)
```javascript
// TODO(@dave): Refactor this phase later (Epic 5).
// deferred to later: handle network timeouts
```

#### ✅ Good (Plain Task)
```javascript
// TODO: Extract token parsing into a distinct, stateless utility class.
// TODO: Implement exponential backoff for network timeout retries.
```

---

### 4. Cite and Explain Decisions
Every algorithmic, rendering, or data-flow decision must cite its source and justify its selection over alternatives. An uncited pipeline decision is incomplete context.

#### ❌ Bad (Unjustified)
```javascript
// Using GGX for the shading model.
// Sort the passes before execution.
```

#### ✅ Good (Cited & Justified)
```javascript
// Algorithm: GGX microfacet BRDF. Source: [Walter et al. 2007]. Chosen over Beckmann for longer specular tails, satisfying the PBR constraint.
// Sorts render passes by material ID. Source: Architecture Spec v1. Minimizes pipeline state changes on the GPU.
```

---

### 5. Lead with Invariants & Hidden Coupling
Define technical terms inline. Prioritize documenting hidden coupling (what else breaks if this changes) and mathematical invariants.

#### ❌ Bad (Over-explained Chatter)
```javascript
// Here we start the process of breaking down the fat cells, which is called lipolysis.
// Make sure the buffer is cleared before calling this, or it will crash.
```

#### ✅ Good (Invariants & Coupling)
```javascript
// Executes lipolysis (breaking down fat). INVARIANT: Input matrix must be normalized.
// Implicit Coupling: Depends on 'RenderPass::Begin' clearing the output buffer to 0x00.
```

---

### 6. Clean Stale Comments on Contact
Treat all existing comments as STALE until verified against executing code. When modifying a code block, you must actively delete or update comments that contradict the new logic.

#### ❌ Bad (Orphaned Comment)
```javascript
// Returns true if the user is an admin.
return role == Role.MODERATOR;
```

#### ✅ Good (Synchronized)
```javascript
// Returns true if the user has moderator privileges.
return role == Role.MODERATOR;
```

Here is the optimized guidelines set for workflow, documentation, and source control, formatted with the same agent-optimized structure and specific examples.

---

## Documentation Standards
**Core Directive:** System state must remain synchronized with reality. Documentation updates are mandatory prerequisites for context clearing. Artifacts must remain hygienic, and source control operations must proceed autonomously based on CI triggers.

### 1. Strict Final-Step Documentation
Documentation updates are the mandatory final step before clearing or compacting context. The plan is incomplete until all tracking files (`PLAN.md`, `TODO.md`, `OPEN_BUGS.md`, `CLOSED_BUGS.md`, `POST_WORK_FINDINGS.md`) reflect empirically verified reality. Do not record conjecture.

#### ❌ Bad (Incomplete / Lazy Handoff)
```markdown
// Agent Output:
"Finished implementing the renderer. I'm ready to clear context and start the next phase." 
(No file updates made; completed tasks left in PLAN.md).
```

#### ✅ Good (Rigorous State Sync)
```markdown
// Agent Output:
"Phase complete. Executing documentation sync:"
- `docs/PLAN.md`: Moved 'Implement Renderer' to Completed (points to mem_04.md).
- `docs/OPEN_BUGS.md`: Removed #12 (verified fixed).
- `docs/CLOSED_BUGS.md`: Logged #12 resolution.
- `target/`: Cleared stale build artifacts.
```

---

### 2. Segregate Bugs from TODOs
Maintain strict boundaries for tracking files. Bugs never go in `TODO.md`. Deferrals go in `TODO.md` (optimized and trimmed). Mid-run anomalies go to `POST_WORK_FINDINGS.md`.

#### ❌ Bad (Mixed Concerns)
```markdown
// docs/TODO.md
- Refactor the input handler.
- BUG: Game crashes when pressing Esc.
- Found out the physics tick is decoupled from frame rate, maybe fix later?
```

#### ✅ Good (Strict Segregation)
```markdown
// docs/TODO.md
- Extract stateless input parsing to utility class.

// docs/OPEN_BUGS.md
- [Crash] Unhandled panic on Esc keydown during main menu transition.

// docs/POST_WORK_FINDINGS.md
- Title: Physics Tick Decoupling. Summary: Physics updates run async to frame rate; potential race condition identified. Status: Needs Review.
```

---

### 3. Centralized, Ephemeral Debug Dumps
All debug artifacts must use a single sink: `debug/dumps/` at the workspace root. Never write stray artifacts to the repo root or `target/`. Dumps are ephemeral; delete them once the associated bug family is resolved. Never commit dump files.

#### ❌ Bad (Scattered / Persistent Dumps)
```bash
# Writing a debug trace to the root directory
fs.writeFileSync("./trace_output_final.json", dump);

# Leaving dumps around after a fix
git add debug/dumps/memory_leak_trace.ppm
git commit -m "Fixed memory leak"
```

#### ✅ Good (Centralized / Cleaned Dumps)
```bash
# Writing strictly to the designated sink
fs.writeFileSync("./debug/dumps/trace_output.json", dump);

# Post-fix cleanup
rm -rf ./debug/dumps/*
git commit -m "Fix memory leak in texture allocator"
```

---

### 4. No Debug Code in Release Builds
Debug code never reaches the release build. Strip temporary instrumentation — `dbg!`, debug `println!` / `eprintln!`, `console.log`, `debugger;`, and commented-out scaffolding — before committing. Diagnostics that must persist run through a leveled facility that is silenced or compiled out in release: Rust `tracing` levels, `debug_assert!`, or `#[cfg(debug_assertions)]`; client logging through the project logger, never a raw `console.log`.

#### ❌ Bad (Instrumentation Shipped to Release)
```rust
pub fn resolve_access(user: Uuid, doc: &Document) -> Access {
    println!("DEBUG resolve_access user={user:?}"); // unconditional; prints in release
    dbg!(&doc.permissions);                          // unconditional; ships to stderr in release
    // ...
}
```
```javascript
function applyOptimistic(op) {
    console.log("op", op); // retained in the production client bundle
    debugger;              // halts the shipped client in a user's browser
    store.apply(op);
}
```

#### ✅ Good (Stripped or Gated)
```rust
pub fn resolve_access(user: Uuid, doc: &Document) -> Access {
    // Leveled diagnostic: filtered out in release by the subscriber level
    // (statically removable via tracing's release_max_level_* feature).
    tracing::debug!(?user, "resolving access");
    // Compiled out of release builds entirely.
    debug_assert!(doc.is_valid(), "INVARIANT: document validated before access resolution");
    // ...
}
```
```javascript
function applyOptimistic(op) {
    // Routed through the project logger; pruned from the production build.
    logger.debug("applying optimistic op", op);
    store.apply(op);
}
```

---

### 5. Autonomous Commits & Milestone Pushes
Do not pause to ask for permission. Commit logical work-units immediately once local CI passes. Push to remote *only* when a FULL milestone is completed. Do not batch unrelated concerns into a single commit.

#### ❌ Bad (Pausing / Batching / Premature Push)
```text
// Agent Output:
"Local tests passed. Should I commit these changes?"
// Or:
git commit -m "Fixed physics bug and also updated the UI padding"
git push origin main # (Pushed on a sub-task, not a full milestone)
```

#### ✅ Good (Autonomous Action)
```bash
# Local CI passes. Committing logical unit immediately.
git commit -m "Fix physics body penetration depth calculation"

# Milestone complete. Pushing autonomously.
git push origin main
gh run watch # Monitoring pipeline post-push
```

---

### 6. Crash Resolution & CI Monitoring

Work is never complete if there is a crash or a red CI pipeline. On a crash, autonomously read the latest panic trace, fix the root cause, and re-run. If CI goes red post-push, fix-forward layer-by-layer starting from the topmost error. Only pause if genuinely stuck.

#### ❌ Bad (Passive Reporting)
```text
// Agent Output:
"I ran the build but it panicked with a segmentation fault. Let me know how you want to proceed."
```

#### ✅ Good (Active Remediation)
```text
// Agent Output:
"Build panicked. Read `debug/dumps/panic.log`. Found null pointer dereference in `RenderPass`. Fixing initialization sequence, re-running tests, and verifying fix before proceeding."
```

## graphify

This project has a knowledge graph at graphify-out/ with god nodes, community structure, and cross-file relationships.

Rules:
- For codebase questions that touch multiple files / crates, first run `graphify query "<question>"` when graphify-out/graph.json exists. Use `graphify path "<A>" "<B>"` for relationships and `graphify explain "<concept>"` for focused concepts. These return a scoped subgraph, usually much smaller than GRAPH_REPORT.md or raw grep output.
- If graphify-out/wiki/index.md exists, use it for broad navigation instead of raw source browsing.
- Read graphify-out/GRAPH_REPORT.md only for broad architecture review or when query/path/explain do not surface enough context.
- After modifying code, run `graphify update .` to keep the graph current (AST-only, no API cost).

## Codebase Skills & Agents

Project-scoped codebase knowledge lives in `shadowcat-codebase-*` skills (`.claude/skills/`):
orientation+index briefs (Purpose / Key files / Hard invariants / Gotchas / Pointers) that route
INTO graphify, `docs/design/`, and memory rather than duplicating them. `shadowcat-codebase-core`
is the always-relevant base; domain skills cover documents-permissions, actors-tokens,
scene-rendering, realtime-sync, client-shell, and assets. A scoped `Edit|Write` hook reminds the
main-thread agent which skill applies; subagents must invoke skills explicitly (below).

### 1. Reviewed Skill-Update Gate (mandatory, doc-sync tier)
Whenever a plan finishes execution — and whenever an inline change alters a subsystem's seam,
invariant, or gotcha — update the affected `shadowcat-codebase-*` skill(s) BEFORE merge/clear. If
work opens a subsystem no existing skill covers, **create a new `shadowcat-codebase-<subsystem>`
skill** (fixed shape; add its globs to the activation hook). The update/creation is itself
reviewed: dispatch `shadowcat-spec-reviewer` to confirm each skill diff accurately captures the
change (no omission, drift, or broken pointer). This gate blocks completion at the same tier as
the documentation-sync gate. Trivial changes that touch no subsystem knowledge need no edit, but
you must state so explicitly. Same rule applies to escalation-twin agents: if a change touches
`shadowcat-coder`, `shadowcat-code-reviewer`, or `shadowcat-spec-reviewer`'s body, mirror it to
that agent's `-opus` twin.

#### ❌ Bad (Silent drift)
```text
"Plan done, merging." (factions added; actors-tokens skill never updated, never reviewed)
```
#### ✅ Good (Reviewed update)
```text
"Plan done. Updated shadowcat-codebase-actors-tokens (new faction-border seam + invariant).
Dispatched shadowcat-spec-reviewer on the skill diff: PASS. Merging."
```

### 2. Agent Dispatch in Superpowers Workflows
Subagents do not auto-activate skills, so use the project agents (each invokes the relevant
`shadowcat-codebase-*` skill first):
- Delegating implementation to a subagent → `shadowcat-coder`.
- Any review checkpoint (buddy-check, `requesting-code-review`, mainline-plan-execution final
  review) → dispatch `shadowcat-spec-reviewer` + `shadowcat-code-reviewer` as the two-reviewer pair.

#### ❌ Bad (Generic subagent, no codebase context)
```text
Task(general-purpose, "implement the faction border")  // skips invariants, no skill loaded
```
#### ✅ Good (Project agent)
```text
Task(shadowcat-coder, "implement the faction border")  // invokes codebase skill, follows TDD
```

### 3. Model/Effort Tiering & Escalation
Every subagent dispatch specifies `effort`, not just `model` — an
unspecified effort silently inherits the session's, defeating cost
discipline. `shadowcat-coder` runs `effort: medium` (bounded execution
against a spec/plan); `shadowcat-code-reviewer` and `shadowcat-spec-reviewer`
run `effort: high` (review is reasoning-heavy). Each has an `-opus` twin
(`model: opus`, `effort: high`, identical body) — when the base agent
reports BLOCKED, or a reviewer's findings read as shallow/uncertain,
re-dispatch to the twin before escalating to the human. For work outside
these three agents' scope, fall back to the global `sdd-*` agents at
`~/.claude/docs/sdd-model-effort-tiers.md`.

#### ❌ Bad (Unspecified Effort / No Escalation Path)
```text
Task(shadowcat-coder, "implement the faction border")  // no effort — inherits session
// shadowcat-coder reports BLOCKED → escalated straight to the user
```
#### ✅ Good (Effort-Explicit Dispatch / Twin Before Human)
```text
Task(shadowcat-coder, "implement the faction border")  // sonnet, effort: medium
// BLOCKED on ambiguous ownership model → Task(shadowcat-coder-opus, ...) before asking the user
```
