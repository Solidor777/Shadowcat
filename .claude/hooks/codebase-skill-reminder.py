#!/usr/bin/env python3
"""PreToolUse(Edit|Write|MultiEdit): remind the agent which shadowcat-codebase-* skill
covers the file being edited. Deduped once per (session, subsystem). Fails open.

Ordering: most-specific subsystems first — `assets` precedes `documents-permissions`
(shared `src/server/src/data/`) and `realtime-sync` (shared `src/server/src/http/`) so
asset files route to `assets`, not the broader globs. First match wins."""
import sys, json, os, tempfile, re

# (subsystem-id, [path regexes]). Order = priority; first match wins.
SUBSYSTEMS = [
    ("dice",                 [r"src/server/src/dice/"]),
    ("nightfox",             [r"src/client/formula/", r"src/modules/nightfox"]),
    ("chat",                 [r"src/server/src/chat/", r"src/client/core/src/chat-docs\.ts", r"src/modules/chat/", r"src/modules/chat-composer/", r"src/modules/chat-card/"]),
    ("assets",               [r"src/modules/assets/", r"src/server/src/data/asset\.rs", r"src/server/src/http/assets\.rs"]),
    ("module-toolchain",     [r"src/server/src/modules\.rs", r"src/server/src/http/module_routes\.rs", r"src/client/core/src/(loader|module-rest)\.ts", r"src/modules/settings/src/ModuleManager"]),
    ("documents-permissions", [r"src/server/src/data/", r"src/client/core/src/wire\.ts"]),
    ("actors-tokens",        [r"src/modules/actors/", r"src/modules/factions/", r"src/client/core/src/actor\.ts"]),
    ("scene-rendering",      [r"src/server/src/scene/", r"src/modules/stage/", r"src/modules/scene-tools/", r"src/modules/scene-browser/", r"src/client/render/", r"src/client/core/src/scene-docs\.ts"]),
    ("realtime-sync",        [r"src/server/src/ws/", r"src/server/src/http/", r"src/server/src/auth/", r"src/client/core/src/(store|optimistic|ws-client)\.ts"]),
    ("panels",               [r"src/modules/panels/", r"src/client/ui-kit/src/panelsBridge"]),
    ("templates",            [r"src/client/core/src/(merge|templates)\.ts", r"src/client/ui-kit/src/(templatesController|TemplateControls|TemplateModalHost|MergeConflictModal|SheetHost)"]),
    ("sheets",               [r"src/client/core/src/sheets\.ts", r"src/client/ui-kit/src/(sheetsController|sheetEdit|SystemTreeEditor)", r"src/modules/sheet-(fallback|actor|item)/"]),
    ("client-shell",         [r"src/modules/entry/", r"src/modules/core-ui/", r"src/modules/topbar/", r"src/modules/statusbar/", r"src/modules/settings/", r"src/client/shell/", r"src/client/ui-kit/"]),
    ("server-ops",           [r"src/server/src/main\.rs", r"src/server/src/config\.rs", r"src/server/src/db\.rs", r"src/server/src/backup\.rs", r"src/server/tests/backup_cli\.rs"]),
    # Standalone-Nightfox paths are repo-root-relative (`src/roll.ts`), distinct from the nested
    # `nightfox` entry above (which matches Shadowcat-side paths only). Deliberately placed LAST,
    # not merged into the entry above: every real Shadowcat subsystem must win the routing tie
    # against these patterns, and first-match-wins means only last position guarantees that. This
    # costs nothing in the Nightfox repo itself, since no Shadowcat path exists there to lose the
    # tie to. Do not "tidy" this back into one `nightfox` entry — that reintroduces the hijack
    # risk this split exists to prevent. Both start- and end-anchored (`^...$` on the extension)
    # so a nested Shadowcat file (`.../src/contributions.ts`) or a `.tsx` sibling cannot match.
    ("nightfox",             [r"^src/(roll|resolve|contributions|nightfox-docs)\.ts$",
                              r"^src/sheets/"]),
]


def main():
    try:
        d = json.loads(sys.stdin.read())
    except Exception:
        return  # fail open on any parse error
    # SAFETY: routing keys purely on file_path; tool_name is intentionally not checked.
    # Read-exclusion is enforced by the settings.json matcher (Edit|Write|MultiEdit) — that
    # matcher scoping is load-bearing; widening it would fire reminders on reads.
    t = d.get("tool_input", d) or {}
    path = str(t.get("file_path") or "").replace(chr(92), "/").lower()
    session = str(d.get("session_id") or "nosession")
    if not path:
        return
    sub = None
    for name, pats in SUBSYSTEMS:
        if any(re.search(p.lower(), path) for p in pats):
            sub = name
            break
    if sub is None:
        return
    # Per-(session, subsystem) dedup via marker file; fire once per subsystem per session.
    try:
        mdir = os.path.join(tempfile.gettempdir(), "shadowcat-skill-markers")
        os.makedirs(mdir, exist_ok=True)
        marker = os.path.join(mdir, "%s-%s.seen" % (re.sub(r"[^A-Za-z0-9_.-]", "_", session), sub))
        if os.path.exists(marker):
            return
        with open(marker, "w") as f:
            f.write("1")
    except Exception:
        pass  # if dedup bookkeeping fails, still emit (a repeat beats silence)
    skill = "shadowcat-codebase-%s" % sub
    msg = ("You are editing the %s subsystem. Consider invoking the `%s` skill "
           "(plus `shadowcat-codebase-core`) for invariants and pointers before changing it."
           % (sub, skill))
    print(json.dumps({"hookSpecificOutput": {
        "hookEventName": "PreToolUse", "additionalContext": msg}}))


if __name__ == "__main__":
    main()
