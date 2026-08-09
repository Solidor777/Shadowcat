# Nightfox Agent Parity Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Give Nightfox sessions the same codebase skills, agents, standards, and skill-routing hook as Shadowcat sessions, with the shared material stored exactly once.

**Architecture:** Shadowcat's repository doubles as a Claude Code plugin marketplace whose single plugin is sourced from the existing `.claude/` directory — no files move. Nightfox enables that plugin and additionally carries the material a plugin cannot supply: `CLAUDE.md` prose and permission rules. Kimi Code, which reads neither Claude plugin manifests nor Claude `agents/` directories, gets a skills shim plus per-repo `.kimi-code/agents/`.

**Tech Stack:** Claude Code plugin manifests (JSON), Claude Code settings/hooks (JSON), Python 3 (the routing hook), Bash (the hook's self-test), Kimi Code manifests (JSON/TOML).

## Global Constraints

- **Nothing leaves Shadowcat's git history.** All 26 files currently tracked under `.claude/` stay tracked at their current paths. Verified by `git ls-files .claude/ | wc -l` returning 26 at the end.
- **The plugin is enabled in Nightfox only.** Enabling it in Shadowcat too would double-register every skill and agent name.
- **No machine-local absolute path may enter a committed file in either public repo.** `C:/Dev/Shadowcat/...` may appear only in git-ignored files.
- **Agent bodies are sync-paired.** Every edit to a `shadowcat-*` agent body is mirrored verbatim to its `-opus` twin, per the `<!-- Sync-paired with ... -->` marker each file already carries. Frontmatter (`model`, `effort`) is NOT mirrored — it is what distinguishes the twins.
- **The routing hook is shared by both repos.** Any pattern added for Nightfox must not capture a Shadowcat path. The `nightfox` entry sits second in `SUBSYSTEMS` and first-match-wins, so an over-broad pattern silently hijacks another subsystem's routing.
- **`rm` is banned.** Use `trash` with relative paths. No `Remove-Item`/`ri`/`del`/`rd`/`rmdir`.
- **Commit messages** end with `Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>`. Never `push --force`, `reset --hard`, or history-dropping rebases.

## Model/Effort directives

The human directed: **mainline continuation with no model switch**, and **subagent dispatch is allowed**. So the controlling session stays on its current model/effort and does not switch; per-task work dispatches to the project agents at their own frontmatter tiers — `shadowcat-coder` (sonnet/medium) for implementation, `shadowcat-spec-reviewer` + `shadowcat-code-reviewer` (sonnet/high) as the two-reviewer pair at review checkpoints. Escalate to an `-opus` twin only on a BLOCKED report or a shallow-reading review; do not switch the mainline model.

## Buddy-check directives

No high-risk signals: this plan touches configuration and prose, ships no runtime code, and has no security or concurrency surface. The two-reviewer pair at the final review checkpoint is sufficient. Task 4 carries the plan's only empirical unknown (hook double-fire) and closes it with a direct observation step rather than a review.

## File Structure

**Shadowcat repo (`C:\Dev\Shadowcat`):**

| File | Responsibility |
|---|---|
| `.claude-plugin/marketplace.json` | NEW. Declares marketplace `shadowcat` and its one plugin, sourced from `./.claude`. |
| `.claude/hooks/codebase-skill-reminder.py` | MODIFY. Add standalone-Nightfox path patterns to the `nightfox` subsystem entry. |
| `.claude/hooks/test-codebase-skill-reminder.sh` | MODIFY. Add routing assertions for the new patterns. |
| `.claude/hooks/hooks.json` | NEW. Plugin-side registration of the routing hook via `${CLAUDE_PLUGIN_ROOT}`. |
| `.claude/agents/*.md` (6 files) | MODIFY. Correct the skill-fallback instruction, which assumes a project-relative skills path. |
| `.claude/CLAUDE.md` | MODIFY. Add sync markers to the five blocks Nightfox copies. |
| `.claude/kimi.plugin.json` | NEW. Kimi skills shim. Git-ignored. |
| `.kimi-code/agents/*.md` (6 files) | NEW. Kimi-format agents. Git-ignored. |
| `.gitignore` | MODIFY. Ignore the two Kimi artifacts above. |

**Nightfox repo (`C:\Dev\Nightfox`):**

| File | Responsibility |
|---|---|
| `.claude/CLAUDE.md` | MODIFY. Grow from the Project block alone to a full standards file. |
| `.claude/settings.json` | NEW. Permission rules (`trash` allowed, deletion commands denied). Committed. |
| `.claude/settings.local.json` | NEW. Plugin enablement. Git-ignored — it names a machine-local path. |
| `.kimi-code/agents/*.md` (6 files) | NEW. Kimi-format agents. Git-ignored. |
| `.gitignore` | MODIFY. Ignore `settings.local.json` and `.kimi-code/`. |

Task order is dependency-driven: Task 1 creates the plugin, Task 2 makes Nightfox load it (and must land before any manual verification of Tasks 3–4 in a Nightfox session).

---

### Task 1: Marketplace manifest

Makes Shadowcat's `.claude/` addressable as a plugin without moving anything.

**Files:**
- Create: `C:\Dev\Shadowcat\.claude-plugin\marketplace.json`

**Interfaces:**
- Consumes: nothing.
- Produces: marketplace name `shadowcat`, plugin name `shadowcat-codebase`. Task 2 references the pair as `shadowcat-codebase@shadowcat`.

- [ ] **Step 1: Create the manifest**

Write `C:\Dev\Shadowcat\.claude-plugin\marketplace.json`:

```json
{
  "name": "shadowcat",
  "owner": {
    "name": "emper"
  },
  "metadata": {
    "description": "Shadowcat codebase skills, agents, and the skill-routing hook, shared with downstream consumers of the engine.",
    "version": "1.0.0"
  },
  "plugins": [
    {
      "name": "shadowcat-codebase",
      "description": "Shadowcat subsystem orientation skills, the coder/reviewer agent set, and the codebase-skill routing hook.",
      "source": "./.claude"
    }
  ]
}
```

The relative `source` is the established convention — the official superpowers marketplace uses `"source": "./"`.

- [ ] **Step 2: Verify the JSON parses**

Run from `C:\Dev\Shadowcat`:

```bash
python3 -c "import json;d=json.load(open('.claude-plugin/marketplace.json'));print(d['name'],d['plugins'][0]['name'],d['plugins'][0]['source'])"
```

Expected: `shadowcat shadowcat-codebase ./.claude`

- [ ] **Step 3: Verify the source directory has the layout a plugin expects**

```bash
ls .claude/skills .claude/agents .claude/hooks >/dev/null && echo "LAYOUT OK"
```

Expected: `LAYOUT OK`

- [ ] **Step 4: Confirm nothing left the repo**

```bash
git ls-files .claude/ | wc -l
```

Expected: `26`

- [ ] **Step 5: Commit**

```bash
git add .claude-plugin/marketplace.json
git commit -m "feat(plugin): declare the shadowcat marketplace sourced from .claude

Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>"
```

---

### Task 2: Nightfox settings and gitignore

Makes Nightfox load the plugin and inherit the deletion-safety rules. Note this task edits the **Nightfox** repo (`C:\Dev\Nightfox`), which has its own git history — commit there, not in Shadowcat.

**Files:**
- Create: `C:\Dev\Nightfox\.claude\settings.json`
- Create: `C:\Dev\Nightfox\.claude\settings.local.json`
- Modify: `C:\Dev\Nightfox\.gitignore`

**Interfaces:**
- Consumes: `shadowcat-codebase@shadowcat` from Task 1.
- Produces: a Nightfox session in which the 15 `shadowcat-codebase-*` skills and 6 `shadowcat-*` agents resolve.

- [ ] **Step 1: Extend Nightfox's `.gitignore` first**

The file currently contains exactly two lines (`node_modules/`, `dist/`). Ordering matters: `settings.local.json` must be ignored **before** it is created, or it lands in `git status` as a tracked-candidate carrying a machine-local absolute path into a public repo.

Append to `C:\Dev\Nightfox\.gitignore`:

```gitignore
.claude/settings.local.json
.kimi-code/
```

- [ ] **Step 2: Verify the ignore rules bite**

Run from `C:\Dev\Nightfox`:

```bash
git check-ignore -v .claude/settings.local.json .kimi-code/agents/x.md
```

Expected: two lines, each naming `.gitignore` and the matching rule. A silent exit means the rule did not match — stop and fix before continuing.

- [ ] **Step 3: Create the committed permission rules**

Write `C:\Dev\Nightfox\.claude\settings.json`. This is copied deliberately: plugins cannot ship permission rules.

```json
{
  "permissions": {
    "allow": [
      "Bash(trash *)",
      "PowerShell(trash *)"
    ],
    "deny": [
      "Bash(rm *)",
      "Bash(sudo rm *)",
      "PowerShell(rm *)",
      "PowerShell(Remove-Item *)",
      "PowerShell(ri *)",
      "PowerShell(del *)",
      "PowerShell(erase *)",
      "PowerShell(rd *)",
      "PowerShell(rmdir *)"
    ]
  }
}
```

- [ ] **Step 4: Create the machine-local plugin enablement**

Write `C:\Dev\Nightfox\.claude\settings.local.json`:

```json
{
  "enabledPlugins": {
    "shadowcat-codebase@shadowcat": true
  }
}
```

- [ ] **Step 5: Verify both files parse and the local one stays untracked**

```bash
python3 -c "import json;json.load(open('.claude/settings.json'));json.load(open('.claude/settings.local.json'));print('JSON OK')"
git status --porcelain .claude/
```

Expected: `JSON OK`, and `git status` lists `.claude/settings.json` only — `settings.local.json` must NOT appear.

- [ ] **Step 6: Register the marketplace (human-run, once per machine)**

In a Claude Code session, run:

```
/plugin marketplace add C:\Dev\Shadowcat
```

Expected: the marketplace `shadowcat` is added and lists plugin `shadowcat-codebase`.

- [ ] **Step 7: Commit (in the Nightfox repo)**

```bash
git add .gitignore .claude/settings.json
git commit -m "feat(claude): enable the shadowcat-codebase plugin and deletion-safety rules

Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>"
```

---

### Task 3: Agent skill-fallback correction

Each agent tells its reader to fall back to a project-relative skills path if the Skill tool is unavailable. Reached through the plugin, that path does not exist, so the fallback silently points at nothing.

**Files:**
- Modify: `C:\Dev\Shadowcat\.claude\agents\shadowcat-coder.md` (fallback text at the end of the numbered hard-first-step list)
- Modify: `C:\Dev\Shadowcat\.claude\agents\shadowcat-coder-opus.md` (same text)
- Modify: `C:\Dev\Shadowcat\.claude\agents\shadowcat-code-reviewer.md` (fallback text in the HARD FIRST STEP paragraph)
- Modify: `C:\Dev\Shadowcat\.claude\agents\shadowcat-code-reviewer-opus.md` (same)
- Modify: `C:\Dev\Shadowcat\.claude\agents\shadowcat-spec-reviewer.md` (same)
- Modify: `C:\Dev\Shadowcat\.claude\agents\shadowcat-spec-reviewer-opus.md` (same)

**Interfaces:**
- Consumes: nothing.
- Produces: agent bodies valid in both a Shadowcat checkout and a plugin-sourced Nightfox session.

**Design note — why not just write the absolute plugin path.** The obvious fix is to add `C:/Dev/Shadowcat/.claude/skills/<name>/SKILL.md` as a second fallback. Rejected: these files are committed to a public repo, and that violates the global constraint on machine-local paths. All six agents already carry `Skill` in their `tools:` list, so the fallback is a genuine edge case; the correct behavior when it fires in a Nightfox session is to stop, not to guess a path.

- [ ] **Step 1: Replace the coder fallback (both twins)**

In `shadowcat-coder.md` and `shadowcat-coder-opus.md`, find:

```
   FALLBACK: if the Skill tool is unavailable to you, `Read` the file(s) directly at
   `.claude/skills/<name>/SKILL.md`. Never proceed without this context.
```

Replace with:

```
   FALLBACK: if the Skill tool is unavailable, `Read` `.claude/skills/<name>/SKILL.md` — this
   path exists only in a Shadowcat checkout. In a consumer repo the skills arrive through the
   `shadowcat-codebase` plugin and have no readable project path: report BLOCKED rather than
   guessing one. Never proceed without this context.
```

- [ ] **Step 2: Replace the code-reviewer fallback (both twins)**

In `shadowcat-code-reviewer.md` and `shadowcat-code-reviewer-opus.md`, find:

```
Skill tool (FALLBACK: `Read` `.claude/skills/<name>/SKILL.md`). Use their invariants/gotchas as
review criteria.
```

Replace with:

```
Skill tool (FALLBACK: `Read` `.claude/skills/<name>/SKILL.md` — Shadowcat checkouts only; in a
consumer repo the skills arrive through the `shadowcat-codebase` plugin and have no readable
project path, so report BLOCKED instead of guessing one). Use their invariants/gotchas as
review criteria.
```

- [ ] **Step 3: Replace the spec-reviewer fallback (both twins)**

In `shadowcat-spec-reviewer.md` and `shadowcat-spec-reviewer-opus.md`, find:

```
Skill tool (FALLBACK: `Read` `.claude/skills/<name>/SKILL.md`). Use them as the bar for
subsystem invariants.
```

Replace with:

```
Skill tool (FALLBACK: `Read` `.claude/skills/<name>/SKILL.md` — Shadowcat checkouts only; in a
consumer repo the skills arrive through the `shadowcat-codebase` plugin and have no readable
project path, so report BLOCKED instead of guessing one). Use them as the bar for
subsystem invariants.
```

- [ ] **Step 4: Verify the stale path is gone and the twins match**

```bash
cd /c/Dev/Shadowcat/.claude/agents
grep -c "consumer repo" *.md
diff <(tail -n +10 shadowcat-coder.md) <(tail -n +10 shadowcat-coder-opus.md) && echo "CODER TWINS MATCH"
diff <(tail -n +10 shadowcat-code-reviewer.md) <(tail -n +10 shadowcat-code-reviewer-opus.md) && echo "CODE-REVIEWER TWINS MATCH"
diff <(tail -n +10 shadowcat-spec-reviewer.md) <(tail -n +10 shadowcat-spec-reviewer-opus.md) && echo "SPEC-REVIEWER TWINS MATCH"
```

Expected: every file reports `1`, and all three MATCH lines print. If a diff prints, the bodies drifted — reconcile before committing.

**The offset is `+10`, not `+8`.** Frontmatter is 7 lines, but line 8 is blank and **line 9 is the `<!-- Sync-paired with ... -->` comment, which names the OTHER twin and therefore legitimately differs in every pair.** At `+8` the diff always reports that one line and can never print MATCH, no matter how correct the edit is. Do not "resolve" such a diff by making the two sync comments identical — each must keep pointing at its own counterpart; a matching pair of comments would point one file at itself.

Afterwards, restore the working directory before running any further git command — a `cd` here persists into later steps and silently re-roots relative paths like `.claude/`:

```bash
cd /c/Dev/Shadowcat
```

- [ ] **Step 5: Commit**

```bash
cd /c/Dev/Shadowcat
git add .claude/agents/
git commit -m "fix(agents): skill fallback path is Shadowcat-only; block rather than guess

Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>"
```

---

### Task 4: Routing hook — Nightfox patterns and plugin registration

The hook maps edited paths to skill names. Standalone Nightfox paths match nothing today, because the only Nightfox pattern (`src/modules/nightfox`) describes the nested checkout.

**Files:**
- Modify: `C:\Dev\Shadowcat\.claude\hooks\test-codebase-skill-reminder.sh` (add assertions)
- Modify: `C:\Dev\Shadowcat\.claude\hooks\codebase-skill-reminder.py` (the `nightfox` entry in `SUBSYSTEMS`)
- Create: `C:\Dev\Shadowcat\.claude\hooks\hooks.json`

**Interfaces:**
- Consumes: nothing.
- Produces: reminders naming `shadowcat-codebase-nightfox` for standalone Nightfox paths, in both repos.

- [ ] **Step 1: Write the failing assertions**

In `test-codebase-skill-reminder.sh`, immediately after the existing `check m8 ...` line and before the final `echo "ALL HOOK TESTS PASS"`, add:

```bash
# Standalone-Nightfox paths (the Nightfox repo opened on its own, not nested under
# src/modules/nightfox/) must route to the nightfox skill.
check n1 "src/roll.ts"                                    "shadowcat-codebase-nightfox"
check n2 "src/resolve.ts"                                 "shadowcat-codebase-nightfox"
check n3 "src/contributions.ts"                           "shadowcat-codebase-nightfox"
check n4 "src/nightfox-docs.ts"                           "shadowcat-codebase-nightfox"
check n5 "src/sheets/StatRow.svelte"                      "shadowcat-codebase-nightfox"
check n6 "src/sheets/sheet-model.ts"                      "shadowcat-codebase-nightfox"
```

- [ ] **Step 2: Run the suite and watch the new assertions fail**

Run from `C:\Dev\Shadowcat`:

```bash
bash .claude/hooks/test-codebase-skill-reminder.sh
```

Expected: `FAIL: src/roll.ts did not map to shadowcat-codebase-nightfox (got: )`, exit non-zero. The empty `got:` is the point — no pattern matched at all.

- [ ] **Step 3: Add the patterns**

In `codebase-skill-reminder.py`, replace the `nightfox` line in `SUBSYSTEMS`:

```python
    ("nightfox",             [r"src/client/formula/", r"src/modules/nightfox"]),
```

with:

```python
    # Standalone-Nightfox paths are repo-root-relative (`src/roll.ts`), so they are listed
    # alongside the nested form. Each is anchored to a filename that exists only in the Nightfox
    # repo — this entry is second in SUBSYSTEMS and first-match-wins, so a broader pattern here
    # would hijack routing for every subsystem below it.
    ("nightfox",             [r"src/client/formula/", r"src/modules/nightfox",
                              r"src/(roll|resolve|contributions|nightfox-docs)\.ts",
                              r"src/sheets/"]),
```

- [ ] **Step 4: Run the suite and watch everything pass**

```bash
bash .claude/hooks/test-codebase-skill-reminder.sh
```

Expected: `ALL HOOK TESTS PASS`, exit 0. The pre-existing assertions (`m1`–`m8` and tests 1–5) passing is the regression check that the new patterns captured no Shadowcat path.

- [ ] **Step 5: Prove the new patterns cannot match a real Shadowcat file**

The suite proves the eight sampled paths still route correctly. This step checks the whole tree:

```bash
cd /c/Dev/Shadowcat
git ls-files | grep -E "src/(roll|resolve|contributions|nightfox-docs)\.ts|src/sheets/" || echo "NO SHADOWCAT COLLISION"
```

Expected: `NO SHADOWCAT COLLISION`. Any listed file means the pattern is over-broad — narrow it and return to Step 4.

- [ ] **Step 6: Add the plugin-side hook registration**

Write `C:\Dev\Shadowcat\.claude\hooks\hooks.json`:

```json
{
  "hooks": {
    "PreToolUse": [
      {
        "matcher": "Edit|Write|MultiEdit",
        "hooks": [
          {
            "type": "command",
            "command": "python3 \"${CLAUDE_PLUGIN_ROOT}/hooks/codebase-skill-reminder.py\"",
            "shell": "bash",
            "async": false
          }
        ]
      }
    ]
  }
}
```

The `Edit|Write|MultiEdit` matcher is load-bearing and is copied verbatim from Shadowcat's `settings.json`: the script's own comment records that widening it would fire reminders on reads.

- [ ] **Step 7: Verify the JSON parses and the referenced script exists**

```bash
python3 -c "import json;d=json.load(open('.claude/hooks/hooks.json'));print(d['hooks']['PreToolUse'][0]['matcher'])"
ls .claude/hooks/codebase-skill-reminder.py
```

Expected: `Edit|Write|MultiEdit`, then the script path.

- [ ] **Step 8: Resolve the plan's one empirical unknown — does Shadowcat now double-fire?**

`hooks.json` sits inside a project `.claude/hooks/` directory. That directory is not itself a settings source, so it *should* be inert for Shadowcat and active only for Nightfox via the plugin. This is inferred, not observed.

In a **fresh** Shadowcat session (dedup is per-session, so a reused session hides the answer), edit any file under `src/server/src/data/` and count the reminder messages naming `shadowcat-codebase-documents-permissions`.

- Exactly one: inferred behavior confirmed. Proceed.
- Two: the plugin registration is also loading for Shadowcat. Apply the fallback — `trash .claude/hooks/hooks.json` (relative path; `trash-cli` silently no-ops on absolute Windows paths), then instead add the same `PreToolUse` block to `C:\Dev\Nightfox\.claude\settings.local.json` with the command `python3 "C:/Dev/Shadowcat/.claude/hooks/codebase-skill-reminder.py"`. The absolute path is permitted there because that file is git-ignored. Record which branch was taken in the commit message.

- [ ] **Step 9: Commit**

```bash
git add .claude/hooks/
git commit -m "feat(hooks): route standalone-Nightfox paths to the nightfox skill

Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>"
```

---

### Task 5: Nightfox CLAUDE.md and Shadowcat sync markers

The only deliberately duplicated prose in the design. Sync markers make the pairing visible at the point of edit.

**Files:**
- Modify: `C:\Dev\Nightfox\.claude\CLAUDE.md` (grow from the Project block alone)
- Modify: `C:\Dev\Shadowcat\.claude\CLAUDE.md` (add markers to the five paired blocks)

**Interfaces:**
- Consumes: nothing.
- Produces: nothing consumed by later tasks.

**What moves, exactly.** From Shadowcat's `.claude/CLAUDE.md`:

| Block | Disposition |
|---|---|
| `## Collaboration & Execution Standards` | Copy; swap Shadowcat examples for Nightfox ones |
| `## Lint Suppressions Require Explicit User Approval` | Copy; drop the Rust `#[allow]`/`#[expect]` examples (Nightfox is TypeScript-only), keep `eslint-disable` and `@ts-ignore`/`@ts-nocheck` |
| `## Agent-Optimized Security & IP Standards` | Copy verbatim |
| `## Code Commenting Rules` | Copy verbatim |
| `## Documentation Standards` | Copy; retarget the tracking-file list to Nightfox's own docs, and keep the cross-repo rule already in Nightfox's Project block (API friction is filed to Shadowcat's `docs/POST_WORK_FINDINGS.md`) |
| `## Cross-Platform From Day One` | **Rewrite, not copy** — see Step 3 |
| `## Reference Docs` table, `## graphify`, `## Codebase Skills & Agents` | **Omit** — all three point at Shadowcat-only artifacts |

- [ ] **Step 1: Add sync markers to Shadowcat's CLAUDE.md**

Directly above each of the five paired headings listed in the table above, insert exactly:

```markdown
<!-- Sync-paired with the Nightfox repo's .claude/CLAUDE.md — mirror any edit to this block there. -->
```

No absolute path: `.claude/CLAUDE.md` is tracked in the public repo, and final verification #2 rejects any new `C:\Dev` occurrence in a tracked file.

The reciprocal marker in Nightfox (Step 3) names Shadowcat the same way:

```markdown
<!-- Sync-paired with the Shadowcat repo's .claude/CLAUDE.md — mirror any edit to this block there. -->
```

- [ ] **Step 2: Verify five markers landed**

```bash
grep -c "Sync-paired with" /c/Dev/Shadowcat/.claude/CLAUDE.md
```

Expected: `5`.

- [ ] **Step 3: Write Nightfox's CLAUDE.md**

Keep the existing `# Nightfox — Agent Instructions` heading and `## Project` block exactly as they are. After them, add the reciprocal sync marker above each copied block, then the adapted cross-platform block:

```markdown
## Cross-Platform From Day One
**Core Directive:** Nightfox ships no binary and has no OS build matrix — it is a browser-loaded
ES module. Cross-platform therefore means every sheet and control renders correctly in desktop
**and** mobile browsers, Android and iOS, from the first commit. A UI that assumes a mouse and a
wide viewport is a defect.

### 1. Touch-Sized, Hover-Free Controls
Every interactive target is touch-sized, and no affordance is reachable by hover alone. Stat rows,
modifier editors, and roll buttons are the dense surfaces where this breaks first.

#### ❌ Bad (Hover-Only Affordance)
```svelte
<!-- The delete control exists only on hover: unreachable on touch. -->
<div class="modifier-row">
  <button class="delete" onclick={remove}>×</button>
</div>
<style>.delete { opacity: 0; } .modifier-row:hover .delete { opacity: 1; }</style>
```

#### ✅ Good (Always-Present, Touch-Sized)
```svelte
<div class="modifier-row">
  <button class="delete" onclick={remove} aria-label={t("modifier.remove")}>×</button>
</div>
<style>.delete { min-inline-size: 44px; min-block-size: 44px; }</style>
```

### 2. Reflowing Sheet Layouts
Sheets reflow to a phone screen. Stat tables are the usual offender: a fixed multi-column grid
forces horizontal scrolling of the whole page.

#### ❌ Bad (Fixed Column Count)
```scss
.stat-table { display: grid; grid-template-columns: repeat(4, 1fr); }
```

#### ✅ Good (Reflowing Track List)
```scss
.stat-table { display: grid; grid-template-columns: repeat(auto-fit, minmax(12rem, 1fr)); }
```
```

Then append the five copied blocks, in this order, each preceded by the reciprocal sync marker from Step 1. Source them by reading `C:\Dev\Shadowcat\.claude\CLAUDE.md` and copying each block from its `##` heading to the line before the next `##` heading:

1. **`## Collaboration & Execution Standards`** — copy all five numbered subsections. In subsection 2, replace the `ENGINE_PRINCIPLES.md`/`PLAN.md` verification targets with Nightfox's `README.md` and `CHANGELOG.md`, and replace the `tokio`/`src/net/` dependency example with a Nightfox-shaped one: adding a runtime dependency to a package whose engine imports are build-time externals. In subsection 4, replace the Rust buffer example with a TypeScript one.

2. **`## Lint Suppressions Require Explicit User Approval`** — copy the Core Directive, but **do not name a gate command**. Nightfox has no ESLint and no lint script; its `package.json` scripts are `dev`, `build`, `typecheck`, `test`, `test:e2e`, `test:e2e:roll-wire`. State plainly that the covered suppressions are caught by `pnpm typecheck` and by review, not by an automated allowances gate — inventing a gate name that resolves to nothing would be worse than admitting there is none. In section 1, keep `eslint-disable` of `no-unused-vars` and `@ts-ignore`/`@ts-nocheck`; delete the four Rust attribute forms. Delete section 2 entirely (`#[expect(...)]` is Rust-only). In section 3, replace both Rust code examples with TypeScript equivalents: a `@ts-ignore` hiding a real type mismatch versus the mismatch actually fixed.

3. **`## Agent-Optimized Security & IP Standards`** — copy verbatim, all four numbered subsections. The examples are already JavaScript/JSON.

4. **`## Code Commenting Rules`** — copy verbatim, all six numbered subsections. The examples are already JavaScript.

5. **`## Documentation Standards`** — copy all six numbered subsections. In subsection 1, replace the Shadowcat tracking-file list with Nightfox's actual docs (`README.md`, `CHANGELOG.md`); state that bug and deferral tracking for engine-API friction goes to Shadowcat's `docs/POST_WORK_FINDINGS.md`, which the Project block already establishes. In subsection 2, retarget the same way. In subsection 4, delete the Rust `tracing`/`debug_assert!` example and keep the JavaScript logger example. In subsection 6, replace "panic trace" with the browser-console and Vitest failure equivalents.

Do not copy `## Cross-Platform From Day One` from Shadowcat — Step 3's rewritten version above replaces it.

- [ ] **Step 4: Verify the omissions and the pairing**

```bash
cd /c/Dev/Nightfox
grep -c "Sync-paired with" .claude/CLAUDE.md
grep -E "^## (Reference Docs|graphify|Codebase Skills)" .claude/CLAUDE.md || echo "SHADOWCAT-ONLY BLOCKS CORRECTLY ABSENT"
grep -n "allow(dead_code)\|#\[expect\|cargo\|rustfmt" .claude/CLAUDE.md || echo "NO RUST LEFTOVERS"
```

Expected: `5`, then both confirmation lines. A Rust hit means the lint-suppressions block was copied without applying its disposition.

- [ ] **Step 5: Commit both repos separately**

```bash
cd /c/Dev/Shadowcat
git add .claude/CLAUDE.md
git commit -m "docs(claude): mark the five blocks paired with Nightfox

Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>"

cd /c/Dev/Nightfox
git add .claude/CLAUDE.md
git commit -m "docs(claude): adopt the shared engineering standards

Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>"
```

---

### Task 6: Kimi Code parity

Kimi reads neither Claude plugin manifests nor Claude `agents/` directories, so it needs a skills shim plus per-repo agent copies.

**Files:**
- Create: `C:\Dev\Shadowcat\.claude\kimi.plugin.json`
- Create: `C:\Dev\Shadowcat\.kimi-code\agents\` (6 files)
- Create: `C:\Dev\Nightfox\.kimi-code\agents\` (6 files)
- Modify: `C:\Dev\Shadowcat\.gitignore`

**Interfaces:**
- Consumes: the agent bodies as corrected in Task 3.
- Produces: nothing consumed by later tasks.

**Effort and model remapping.** Kimi's `k3` declares `support_efforts = ["low", "high", "max"]` — there is no `medium`, and `opus` is not a Kimi model:

| Claude agent | Claude frontmatter | Kimi frontmatter |
|---|---|---|
| `shadowcat-coder` | `model: sonnet`, `effort: medium` | `model: kimi-code/k3`, `effort: low` |
| `shadowcat-code-reviewer`, `shadowcat-spec-reviewer` | `model: sonnet`, `effort: high` | `model: kimi-code/k3`, `effort: high` |
| all three `-opus` twins | `model: opus`, `effort: high` | `model: kimi-code/k3`, `effort: max` |

The base coder maps *down* to `low` to preserve the tier gap that makes escalating to the twin meaningful; collapsing both onto `high` would erase the ladder.

- [ ] **Step 1: Ignore the Kimi artifacts in Shadowcat first**

Append to `C:\Dev\Shadowcat\.gitignore`:

```gitignore
.claude/kimi.plugin.json
.kimi-code/
```

Nightfox's equivalent rule was already added in Task 2, Step 1.

- [ ] **Step 2: Verify both repos ignore them**

```bash
cd /c/Dev/Shadowcat && git check-ignore -v .claude/kimi.plugin.json .kimi-code/agents/x.md
cd /c/Dev/Nightfox  && git check-ignore -v .kimi-code/agents/x.md
```

Expected: a rule line for each path. A silent exit means no match — fix before creating the files.

- [ ] **Step 3: Create the Kimi skills shim**

Write `C:\Dev\Shadowcat\.claude\kimi.plugin.json`, matching the shape used by the `personal-skills` shims:

```json
{
  "name": "shadowcat-codebase",
  "description": "Shadowcat subsystem orientation skills for the engine and its downstream game systems.",
  "skills": "./skills/"
}
```

- [ ] **Step 4: Port the six agents into each repo's `.kimi-code/agents/`**

For each of the six files in `C:\Dev\Shadowcat\.claude\agents\`, copy it to **both** `C:\Dev\Shadowcat\.kimi-code\agents\` and `C:\Dev\Nightfox\.kimi-code\agents\`, changing only the `model:` and `effort:` frontmatter lines per the remapping table above. Bodies are copied unchanged — they already carry the Task 3 correction.

```bash
mkdir -p /c/Dev/Shadowcat/.kimi-code/agents /c/Dev/Nightfox/.kimi-code/agents
for f in /c/Dev/Shadowcat/.claude/agents/*.md; do
  cp "$f" /c/Dev/Shadowcat/.kimi-code/agents/
  cp "$f" /c/Dev/Nightfox/.kimi-code/agents/
done
```

Then edit the frontmatter of all twelve copies. Every file gets `model: kimi-code/k3`. Effort: `low` for `shadowcat-coder.md`, `max` for the three `*-opus.md` files, `high` for the two base reviewers.

- [ ] **Step 5: Verify the remapping**

```bash
cd /c/Dev/Shadowcat/.kimi-code/agents
grep -H "^model:\|^effort:" *.md
grep -L "kimi-code/k3" *.md || echo "ALL ON K3"
grep -c "^effort: medium" *.md | grep -v ":0" || echo "NO MEDIUM EFFORT REMAINS"
```

Expected: every file shows `model: kimi-code/k3`; `ALL ON K3`; `NO MEDIUM EFFORT REMAINS`. Repeat the same three commands in `C:\Dev\Nightfox\.kimi-code\agents`.

- [ ] **Step 6: Install into Kimi (human-run, once per machine)**

In the Kimi TUI — CLI/prompt-mode install does not exist:

```
/plugins install C:/Dev/Shadowcat/.claude
/reload
```

The third-party trust prompt defaults to cancel; approve it. Expected afterwards: the skills list includes the `shadowcat-codebase-*` entries.

- [ ] **Step 7: Confirm nothing Kimi-related became tracked, then commit the ignore rule**

```bash
cd /c/Dev/Shadowcat
git status --porcelain | grep -i "kimi" || echo "NO KIMI ARTIFACTS TRACKED"
git add .gitignore
git commit -m "chore(git): ignore local Kimi Code artifacts

Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>"
```

Expected: `NO KIMI ARTIFACTS TRACKED` before the commit.

---

## Final verification

Run after all six tasks. These are the spec's acceptance criteria.

- [ ] **1. Nothing left Shadowcat's repo**

```bash
cd /c/Dev/Shadowcat && git ls-files .claude/ | wc -l
```
Expected: `26`.

- [ ] **2. No machine-local path introduced by this branch, either repo**

Scope the check to files this branch changed. A whole-tree grep is wrong: 10 tracked Shadowcat files already contain `C:\Dev` — including `.claude/skills/shadowcat-codebase-nightfox/SKILL.md`, `docs/PLAN.md`, six older plans, and this branch's own spec and plan documents, where naming the sibling repo is legitimate. A whole-tree check therefore fails on content this branch never touched.

```bash
cd /c/Dev/Shadowcat
git diff --name-only main...HEAD -- . ':!docs/superpowers/*' \
  | xargs -r git grep -n -F -e 'C:\Dev' -e 'C:/Dev' -- \
  || echo "SHADOWCAT CLEAN"

cd /c/Dev/Nightfox
git diff --name-only main...HEAD \
  | xargs -r git grep -n -F -e 'C:\Dev' -e 'C:/Dev' -- \
  || echo "NIGHTFOX CLEAN"
```

Expected: both CLEAN. `-F` is required — `\D` in a regex is not a literal backslash-D. The `:!docs/superpowers/*` exclusion covers this branch's spec and plan, which legitimately name both repo paths. Git-ignored files (`settings.local.json`, `.kimi-code/`, `kimi.plugin.json`) never appear in `git diff --name-only`, so the Task 4 fallback's absolute path is out of scope by construction.

- [ ] **3. Hook suite green**

```bash
cd /c/Dev/Shadowcat && bash .claude/hooks/test-codebase-skill-reminder.sh
```
Expected: `ALL HOOK TESTS PASS`.

- [ ] **4. Skills and agents resolve in a Nightfox session**

In a fresh Claude Code session opened on `C:\Dev\Nightfox`: invoke `shadowcat-codebase-core` and confirm it returns content; confirm `shadowcat-coder` appears in the agent list.

- [ ] **5. No double registration in a Shadowcat session**

In a fresh session on `C:\Dev\Shadowcat`: confirm each skill name and each agent name appears exactly once.

- [ ] **6. Reminder fires once per subsystem, both repos**

Covered for Shadowcat by Task 4 Step 8. For Nightfox: in a fresh session, edit `src/sheets/StatRow.svelte` and confirm exactly one reminder naming `shadowcat-codebase-nightfox`.

- [ ] **7. Reviewed skill-update gate**

Per the project `CLAUDE.md`, dispatch `shadowcat-spec-reviewer` + `shadowcat-code-reviewer` over the full branch diff. Note for the dispatcher: reviewers have no Bash by directive — pre-generate the diff to a file and pass its path, and relay the gate outputs from steps 1–3 above rather than asking them to run anything.

This work changes no subsystem seam, invariant, or gotcha, so **no `shadowcat-codebase-*` skill body needs updating**. The `shadowcat-codebase-core` skill does document the agent set and the `.claude/` layout; if its text describes the skills as project-local in a way the plugin now contradicts, that is the one skill edit this branch owes — check it explicitly and state the finding either way.
