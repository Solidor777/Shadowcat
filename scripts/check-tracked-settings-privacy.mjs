// Fails when a tracked agent-configuration file carries a user path, an address, or a credential.
//
// .claude/settings.json is tracked so the bypass guard survives a fresh clone. Tracking files that
// are otherwise machine-local puts them one careless edit away from committing personal data, and
// "remember not to" is the discipline this whole mechanism exists to replace.
//
// The scanned corpus is every tracked file under .claude/, enumerated from git at run time. A
// hardcoded filename list cannot see a file added later, and would report success over a corpus
// that no longer matches the repository.
//
// Machine-specific settings belong in .claude/settings.local.json, which stays untracked.

import { readFileSync } from "node:fs";
import process from "node:process";
import { isDirectEntry } from "./lib/is-main.mjs";
import { runGit } from "./lib/run-git.mjs";

// Each entry states why the file cannot be scanned as-is. An exclusion is a hole in the check,
// not a convenience: adding one requires the owner's explicit sign-off.
export const EXCLUDED_FILES = new Set([]);

const RULES = [
  { kind: "user path", re: /(?:[A-Za-z]:\\{1,2}Users\\{1,2}|\/home\/|\/Users\/)[A-Za-z0-9._-]+/ },
  { kind: "address", re: /[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.[A-Za-z]{2,}/ },
  {
    kind: "credential",
    re: /["']?(?:api[_-]?key|apikey|token|secret|password|passwd)["']?\s*[:=]\s*["'][^"']{8,}["']/i,
  },
];

/** Every tracked file under .claude/, minus the documented exclusions. */
export function trackedAgentFiles() {
  const r = runGit(["ls-files", "--", ".claude"], "list tracked agent files");
  if (!r.ok) throw new Error(r.message);
  return r.stdout
    .split("\n")
    .map((l) => l.trim())
    .filter(Boolean)
    .filter((f) => !EXCLUDED_FILES.has(f));
}

/**
 * Every privacy violation in `text`, with the line it sits on.
 * `markdown: true` skips fenced blocks, which carry deliberate counter-examples.
 */
export function scanPrivacy(text, { markdown = false } = {}) {
  const out = [];
  const lines = text.split("\n").map((l) => l.replace(/\r$/, ""));
  let fenced = false;
  for (let i = 0; i < lines.length; i++) {
    if (markdown && /^\s*(?:```|~~~)/.test(lines[i])) {
      fenced = !fenced;
      continue;
    }
    if (fenced) continue;
    for (const rule of RULES) {
      const m = lines[i].match(rule.re);
      if (m) out.push({ line: i + 1, kind: rule.kind, excerpt: m[0] });
    }
  }
  return out;
}

if (isDirectEntry(import.meta.url)) {
  const files = trackedAgentFiles();
  let bad = 0;
  for (const path of files) {
    const text = readFileSync(path, "utf8");
    for (const f of scanPrivacy(text, { markdown: path.endsWith(".md") })) {
      // The excerpt is deliberately not printed: echoing the value copies it into CI logs.
      console.error(`${path}:${f.line}: ${f.kind} in a tracked file`);
      bad++;
    }
  }
  if (bad > 0) {
    console.error(
      `\nlint:settings-privacy: ${bad} violation(s). Move machine-specific settings to .claude/settings.local.json.`,
    );
    process.exit(1);
  }
  console.log(
    `lint:settings-privacy: ${files.length} file(s) scanned, ${EXCLUDED_FILES.size} excluded, 0 error(s)`,
  );
}
