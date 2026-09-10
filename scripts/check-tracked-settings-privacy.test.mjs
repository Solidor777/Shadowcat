import { test, expect } from "vitest";
import {
  scanPrivacy,
  trackedAgentFiles,
  EXCLUDED_FILES,
} from "./check-tracked-settings-privacy.mjs";

test("an absolute Windows user path is a violation", () => {
  const f = scanPrivacy('{"a":"C:\\\\Users\\\\someone\\\\notes.txt"}');
  expect(f.map((x) => x.kind)).toEqual(["user path"]);
});

test("absolute unix home paths are violations", () => {
  expect(scanPrivacy('{"a":"/home/someone/x"}')[0].kind).toBe("user path");
  expect(scanPrivacy('{"a":"/Users/someone/x"}')[0].kind).toBe("user path");
});

test("an address is a violation", () => {
  expect(scanPrivacy('{"a":"someone@example.com"}')[0].kind).toBe("address");
});

test("a credential-shaped value is a violation, quoted key or bare", () => {
  expect(scanPrivacy('{"api_key":"abcdefgh12345678"}')[0].kind).toBe("credential");
  expect(scanPrivacy('{"token": "abcdefgh12345678"}')[0].kind).toBe("credential");
  expect(scanPrivacy('token = "abcdefgh12345678"')[0].kind).toBe("credential");
});

test("the portable forms the tracked files actually use are clean", () => {
  expect(scanPrivacy('{"command":"[ -f \\"$CLAUDE_PROJECT_DIR/graphify-out/graph.json\\" ]"}')).toEqual([]);
  expect(scanPrivacy('{"deny":["Bash(rm *)","PowerShell(Remove-Item *)"]}')).toEqual([]);
  expect(scanPrivacy('path = "src/server/src/data/sqlite.rs"')).toEqual([]);
  expect(scanPrivacy("shadowcat-codebase@skills-dir")).toEqual([]);
});

test("the line number of each finding is reported", () => {
  expect(scanPrivacy('{\n"a":1,\n"b":"/home/someone/x"\n}')[0].line).toBe(3);
});

test("markdown fenced blocks are counter-example territory and are not scanned", () => {
  const md = 'Never do this:\n\n```javascript\nconst k = "sk-proj-9876543210abcdef";\n```\n';
  expect(scanPrivacy(md, { markdown: true })).toEqual([]);
});

test("markdown prose outside a fence is still scanned", () => {
  const md = "Contact someone@example.com about it.\n\n```\nfree text\n```\n";
  expect(scanPrivacy(md, { markdown: true }).map((f) => f.kind)).toEqual(["address"]);
});

test("the corpus is enumerated from git, not hardcoded", () => {
  const files = trackedAgentFiles();
  // Every tracked file under .claude/ is either scanned or excluded by name; none is invisible.
  expect(files).toContain(".claude/settings.json");
  expect(files).toContain(".claude/hooks/guard-git.mjs");
  expect(files.length).toBeGreaterThan(1);
  for (const f of files) expect(EXCLUDED_FILES.has(f)).toBe(false);
});
