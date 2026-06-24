#!/usr/bin/env bash
# Self-test for the codebase-skill reminder hook. Exits non-zero on any failure.
set -u
H="python3 .claude/hooks/codebase-skill-reminder.py"
SID="testsession-$$"
mk() { printf '{"session_id":"%s","tool_name":"Edit","tool_input":{"file_path":"%s"}}' "$SID" "$1"; }

# 1) First edit under data/ => emits documents-permissions reminder
out1=$(mk "src/server/src/data/permission.rs" | $H)
echo "$out1" | grep -q "shadowcat-codebase-documents-permissions" || { echo "FAIL: no reminder on first data edit"; exit 1; }

# 2) Second edit same subsystem same session => silent (deduped)
out2=$(mk "src/server/src/data/document.rs" | $H)
[ -z "$out2" ] || { echo "FAIL: not deduped on second data edit: $out2"; exit 1; }

# 3) Unmapped path => silent
out3=$(mk "README.md" | $H)
[ -z "$out3" ] || { echo "FAIL: emitted for unmapped path"; exit 1; }

# 4) Fresh session under data/ => emits again (dedup is per session, not global)
out4=$(printf '{"session_id":"%sb","tool_name":"Edit","tool_input":{"file_path":"src/server/src/data/permission.rs"}}' "$SID" | $H)
echo "$out4" | grep -q "shadowcat-codebase-documents-permissions" || { echo "FAIL: dedup leaked across sessions"; exit 1; }

# 5) Malformed input => silent, exit 0 (fail open)
out5=$(echo 'not json' | $H) ; rc=$?
{ [ -z "$out5" ] && [ $rc -eq 0 ]; } || { echo "FAIL: not fail-open on garbage"; exit 1; }

# 6) A representative path per subsystem maps to the right skill (fresh session each)
check() { # <session-suffix> <path> <expected-skill>
  o=$(printf '{"session_id":"%s%s","tool_name":"Write","tool_input":{"file_path":"%s"}}' "$SID" "$1" "$2" | $H)
  echo "$o" | grep -q "$3" || { echo "FAIL: $2 did not map to $3 (got: $o)"; exit 1; }
}
check m1 "src/modules/assets/src/Assets.svelte"          "shadowcat-codebase-assets"
check m2 "src/modules/actors/src/ActorsPanel.svelte"     "shadowcat-codebase-actors-tokens"
check m3 "src/server/src/scene/vision.rs"                 "shadowcat-codebase-scene-rendering"
check m4 "src/server/src/ws/room.rs"                      "shadowcat-codebase-realtime-sync"
check m5 "src/modules/topbar/src/index.ts"               "shadowcat-codebase-client-shell"
# Routing-order disambiguation: asset files under shared dirs must beat the broader globs.
check m6 "src/server/src/data/asset.rs"                   "shadowcat-codebase-assets"
check m7 "src/server/src/http/assets.rs"                  "shadowcat-codebase-assets"
check m8 "src/server/src/data/permission.rs"             "shadowcat-codebase-documents-permissions"

echo "ALL HOOK TESTS PASS"
