// Fails when a code comment, or a codebase-skill brief, references an ephemeral,
// process-assigned thing.
//
// A code comment is durable commentary about the code and only the code. Milestone ids,
// repo-document pointers, dated spec files, sweep names and process markers all name something
// whose identity a process assigns; when it is renumbered, closed or superseded the comment points
// at nothing, and — unlike a stale claim about code — no reader and no tool can detect it. That
// undetectability is the whole defect, and this script is the missing detector.
//
// A codebase-skill brief is prose, not code, but the same defect applies to a narrower set of its
// markers (milestone ids, task ids, sweep/round markers, dates, dated plan filenames) — a skill may
// still cite a durable document by path + section anchor, since that citation cannot go stale the
// same way a symbol-free pointer does.
//
// Hard gate: every occurrence fails, legacy included. A grandfathered site is indistinguishable
// from a new one to every future reader, so exempting a backlog preserves exactly the defect being
// removed.
//
// This checker is itself in scope (see ROOTS) and names no document, rule number or workspace, for
// the reason it enforces: a checker that cites the numbered rule it implements breaks silently
// when that rule is renumbered, while still reporting success. The property is the durable name.

import {
  readFileSync,
  readdirSync,
  statSync,
  writeFileSync,
  mkdirSync,
} from "node:fs";
import { join } from "node:path";
import { createHash } from "node:crypto";
import { isDirectEntry } from "./lib/is-main.mjs";

const SKIP_DIRS = new Set([
  "node_modules",
  "dist",
  "target",
  ".git",
  "dist-docs",
]);
// Every extension whose files carry comments a human reads. A stylesheet is code that decides what
// ships exactly as a module is, and its `//` and `/* */` comments go stale the same undetectable
// way — an extension list narrower than the rule's scope reports zero over the part it cannot see,
// which is indistinguishable from a part that was checked.
const EXTS = [".ts", ".rs", ".svelte", ".mjs", ".js", ".scss"];

// The checkers are in scope with the code they check. A gate that excludes its own directory
// guarantees exactly one blind spot, and it is the blind spot where a violation does the most
// damage: an enforcement script that cites an ephemeral document teaches every reader that the
// citation is acceptable, and no run will ever say otherwise.
//
// `examples` is a published workspace whose packages ship as the authoring reference, so a comment
// there is read by more people than most of `src`. Every other repo-wide gate already covers it.
const ROOTS = ["src", "scripts", "examples"];

// The codebase-skill briefs are prose about the code, not code. Their exemption from this
// scanner's rule is narrow, not total: a skill may still cite a durable document by its path plus
// a section anchor, but not a milestone id, a task id, a sweep marker, or a date. A separate root
// (not folded into ROOTS) because these files carry a different extension and a different notion
// of "comment" — the whole line is prose, there is no surrounding code to split it from.
export const MD_ROOTS = [".claude/skills"];
export const MD_EXTS = [".md"];

// Repo-root config files are code too — an eslint config decides what ships. They are collected by
// walking the root non-recursively rather than by listing them, so a new config is in scope the
// day it is added instead of the day someone remembers to enumerate it.
const rootFiles = () =>
  readdirSync(".").filter(
    (n) => EXTS.some((e) => n.endsWith(e)) && statSync(n).isFile(),
  );

/** Repo-relative path with forward slashes, so a scope reads the same on every platform. */
const norm = (p) => p.split("\\").join("/");

// Prefix matching is path-boundary-aware: a raw `startsWith` makes "src/modules/chat" also claim
// "src/modules/chat-card", silently pulling a sibling directory into a scope that never named it.
// The over-match is invisible — the count is simply larger, and larger reads as more thorough.
const under = (p, prefix) => p === norm(prefix) || p.startsWith(norm(prefix) + "/");

/** True when `p` sits inside one of `scopes` (or `scopes` is empty, meaning "everything"). */
export const inScope = (scopes, p) =>
  scopes.length === 0 || scopes.some((s) => under(p, s));

// Single source of the two corpora this gate governs. Every consumer — the main scan, `--cover`,
// and the `--residue` coverage control — calls this one function rather than re-deriving its own
// path list, so a future change to what the gate scans cannot silently leave the control scanning
// a narrower set: there is no second derivation left to drift.
export function collectFiles() {
  const codeFiles = [
    ...ROOTS.flatMap((d) => sources(d, EXTS)),
    ...rootFiles(),
  ].map(norm);
  const mdFiles = MD_ROOTS.flatMap((d) => sources(d, MD_EXTS)).map(norm);
  return { codeFiles, mdFiles };
}

/**
 * The gate's own scope-filtered file set, plus a lookup for which of those files are skill prose
 * rather than code. `--residue` reads this exact return value — see `residueReport` — so its
 * corpus cannot diverge from the gate's without changing this one function.
 */
export function gateFileSet(scopes = []) {
  const { codeFiles, mdFiles } = collectFiles();
  const isMdFile = new Set(mdFiles);
  const scanned = [...codeFiles, ...mdFiles].filter((p) => inScope(scopes, p));
  return { scanned, isMdFile };
}

// Patterns below are documented by describing the shape they match wherever describing is as clear
// as showing. Where a specimen genuinely carries more than a description — a phrase whose exact
// wording is the thing being matched — the line carries this marker and is skipped.
//
// The exemption is narrow by construction and its active count prints with every result. An
// exemption nobody counts is a backdoor, and a silent one is indistinguishable from a rule that
// does not apply; this one is neither, and it is the only exemption the scanner has.
const EXAMPLE_EXEMPT = /\bEXAMPLE:/;

// The comment/code split, and its controls, live in one module so this gate and the
// suppression gate cannot drift apart about what counts as a comment.
import { splitLine } from "./lib/comment-span.mjs";

const BANNED = [
  // A capital M, digits, an optional letter and an optional dashed number are one id shape: the
  // unsuffixed form carries no less process identity than the suffixed one, so a pattern that
  // required the suffix would read the short form as clean. The spelled-out `Task N` form is the
  // same id shape written in full rather than abbreviated behind the capital letter, so it is the
  // same category, not a second one: both writers point outside the code identically, so a pattern
  // carrying only the abbreviated form reads the spelled-out one as clean.
  { name: "milestone/task id", re: /\bM\d+[a-z]?(?:-\d+)?\b|\bTask\s+\d+[a-z]?(?:-\d+)?\b/ },
  // A capital D, I, T or W followed by digits: phase checkpoints, numbered invariants, tasks and
  // workstreams. All are ids a process assigns, resolvable only by a reader holding that artifact.
  // The T form collides with a generic type parameter, but only where a comment names one WITHOUT
  // backticks — and a comment naming a type is required to cite it as a symbol regardless, so the
  // collision resolves the same way every other value/document collision here does.
  { name: "phase / workstream / invariant id", re: /\b[DITW]\d+\b/ },
  // The same local-marker shape as the entry above under a second letter set: a capital A, C, F,
  // H, R or V and a single digit, optionally hyphen-separated and optionally carrying a dashed
  // sub-number. It is how a review finding, a numbered fix pass, a version label, or any other
  // locally numbered item gets written as an initial plus its number, and it resolves only for a
  // reader holding the artifact that assigned it.
  //
  // Restricted to exactly ONE digit and to that letter set, because the governed corpora carry
  // legitimate two-digit tokens sharing a first letter (a Fortran source-extension name), and
  // letters outside this set that a vendored skill uses for its own step headings. `V` is included
  // on a measured zero collisions across both corpora; a versioned SYMBOL is unaffected, since the
  // token boundary excludes a letter run before the V (`PanelLayoutV1`, `Vec2`). It governs BOTH
  // corpora: an initial-plus-number points outside the code from a comment exactly as it does from
  // skill prose, so the two file classes share the entry by reference rather than each carrying
  // its own copy.
  { name: "local letter+digit marker", re: /\b[ACFHRV]-?\d(?:-\d+)?\b/ },
  {
    name: "repo document pointer",
    re: /docs\/[\w./-]+\.md|\b(?:TODO|OPEN_BUGS|CLOSED_BUGS|POST_WORK_FINDINGS|ARCHITECTURE|PLAN)\.md|ARCHITECTURE\s*[§#]|\binvariant\s*#?\s*\d+/i,
  },
  { name: "dated plan/spec file", re: /\b20\d\d-\d\d-\d\d[\w-]*\.md/ },
  // A `Constraint N` / `Global Constraint N` reference is the same bare-numbered-registry shape as
  // the `invariant N` form above: it names an item outside the line with no document to resolve it
  // against, and the number is assigned by a process that renumbers freely. The numbered section of
  // a plan document is exactly the repo-document pointer this list already bans, written without
  // the filename that would at least say which document went stale.
  {
    name: "numbered constraint",
    re: /\b(?:Global\s+)?Constraint\s*#?\s*\d+\b/i,
  },
  // A date stamps a comment with when someone wrote it, which is not behaviour. A match requires a
  // parenthesised or "as of" form, because a bare ISO date also appears inside illustrative
  // program data (a backup path, a sample record) where it names a value rather than a writing.
  {
    name: "date stamp",
    re: /\(\s*20\d\d-\d\d-\d\d\s*\)|\bas of \d|\bas of (?:Jan|Feb|Mar|Apr|May|Jun|Jul|Aug|Sep|Oct|Nov|Dec)/i,
  },
  // Prose describing a superseded state of the code. Only high-precision forms match. Phrases like
  // "no longer" overwhelmingly describe runtime data rather than the code's past, so flagging them
  // would train writers to dodge the wording instead of dropping the narration. Removing history
  // narration is therefore a review obligation this pattern only partly covers, never a claim that
  // a clean run means none remains.
  //
  // EXAMPLE: `pre-fix`/`post-fix`/`pre-refactor`/`post-refactor` are the same shape as
  // EXAMPLE: "before/after the fix" written as a compound instead of a phrase. The compound names
  // the same superseded state the phrase does, so a pattern carrying only the phrase reads the
  // compound as clean.
  //
  // The `used to` form is bound to an explicit subject pronoun (`it`/`this`/`that`/`which`/`they`)
  // rather than matched bare: unqualified "used to <verb>" is also the ordinary present-tense
  // passive-purpose construction ("the id used to recognize our own echoes"), which this corpus
  // carries dozens of and which is not narration at all. Requiring a subject pronoun keeps the
  // EXAMPLE: match on the past-habitual reading ("the check that used to live here") without the
  // recall needed to remember every purpose-clause phrasing that would otherwise collide.
  {
    name: "history narration",
    re: /\bpreviously\b|\bformerly\b|\bhistorically\b|\b(?:before|after) the (?:fix|refactor|change|rewrite)\b|\bpre-(?:fix|refactor)\b|\bpost-(?:fix|refactor)\b|\b(?:it|this|that|which|they)\s+used to\b/i,
  },
  // EXAMPLE: An unnamed reference to "the spec" is the same defect as a named one and strictly
  // worse to resolve: the reader cannot even tell which document went stale. Matches a spec
  // DOCUMENT, not the bare word — that word is also a common parameter name and an end-to-end
  // test-file suffix, and neither points outside the code.
  {
    name: "unnamed spec reference",
    // The trailing-colon form excludes `::`, which is a Rust path segment (a `dice::spec::DieKind`
    // in a doctest names a module in this crate) rather than a document reference.
    re: /\bspec\s*§|\b(?:the|this|design|parent|wire|per)\s+spec\b|\bspec'?d\b|\bspec\s*:(?!:)/i,
  },
  // EXAMPLE: A section pointer that names no document — a bare `§7`, or a "design doc §3" whose
  // EXAMPLE: document is only ever "the design doc" — cannot be resolved from the code at all, so
  // it is strictly worse than a stale named citation: nothing tells a reader which document to
  // go and fail to find. A pointer prefixed by a named public standard resolves for as long as
  // EXAMPLE: that standard exists (`RFC 4291 §2.5.5.2`), so the ban is on the unnamed form, not
  // on the section symbol. The named-document prefix is the property, never a list of the
  // EXAMPLE: specific spellings already written: an ISO or W3C citation is admissible on the same
  // ground and needs no separate ruling.
  {
    name: "unnamed section pointer",
    re: /(?<!\b(?:RFC|ISO|IEC|IEEE|ANSI|W3C|WHATWG|Unicode)[\s-]?\d{1,5}[,;:]?\s{0,3})§\s*\d|\bSection\s+\d+(?:\.\d+)*\b/,
  },
  // EXAMPLE: "the brief" names the same scaffolding as "task brief"/"dispatch brief" above, but the
  // EXAMPLE: bare noun collides with the brief-rules checker's own subject matter, which describes
  // EXAMPLE: what a dispatch brief IS throughout its prose. The pointer CONSTRUCTION — a
  // EXAMPLE: possessive ("the brief's X") or an imperative deferring to it ("the brief requires" /
  // EXAMPLE: "says" / "specifies" / "states") — is what a genuine reference to one specific,
  // now-gone document looks like; a generic description of the category uses neither shape.
  {
    name: "unnamed brief pointer",
    re: /\bthe brief'?s\b|\bthe brief\s+(?:requires|says|specifies|states)\b/i,
  },
  {
    name: "sweep / round / review marker",
    // EXAMPLE: A joined plural writer ("sweeps 2a+2b") names the same process-assigned marker as
    // EXAMPLE: a lone singular one, and a hyphenated writer ("Sweep-1 lesson") names the same
    // EXAMPLE: marker as the spaced form. Requiring only the singular, spaced form would read a
    // genuine hit as clean — a ruled-in category missed, not a category widened.
    //
    // EXAMPLE: A numbered SEVERITY word ("Critical 2") names a review finding, and a numbered
    // EXAMPLE: remedy ("FIX 2", "Fix 3") names a numbered fix pass — the same process-assigned
    // marker the sibling forms in this entry already name, written with the vocabulary a reviewer
    // actually uses. The NUMBER is what makes the phrase point outside the code, so the bare
    // severity word — a roll's `tier_label`, a doc comment calling a defect critical — is
    // deliberately not matched.
    re: /\b[Ss]weeps?[ -]\d+|\bfix[- ]round|\bbuddy-check|\bwhole-branch[- ]review|\bfinding \d+|\b(?:critical|important|major|minor|blocker|fix|issue|bug)\s*#?\s*\d+\b/i,
  },
  // EXAMPLE: A dispatch brief, task or plan is scaffolding that stops existing once the work
  // lands, so a comment deferring to one leaves the reader an instruction they cannot retrieve.
  //
  // EXAMPLE: the plan form stays broad even though a `MergePlan`-typed value gets described the
  // EXAMPLE: same way in prose. Both senses take the possessive, so no wording test separates
  // them — but a comment naming a VALUE must cite it as a symbol in backticks anyway, which
  // resolves the collision at its source. Narrowing to spare the value sense costs real
  // EXAMPLE: detections: the deferring forms are open-ended ("named by the plan", "see the
  // plan's decision"), so any list of them enumerates the phrasings already written, never the
  // ones still to come.
  {
    name: "process marker",
    re: /POST_WORK:|\b(?:task|dispatch) brief\b|\bthe plan\b/i,
  },
];

// The subset of BANNED the owner's ruling actually named for skills: milestone ids, task ids,
// dated plan filenames, sweep/round/review markers, local letter+digit markers, history narration,
// unnamed spec references and
// numbered constraints. A skill's "Pointers" section may cite a durable design document by path plus a
// section anchor, but a dated spec or plan file is a superseded-by-construction record rather
// than a durable one, so citing one by its dated filename is exactly the "dated plan/spec file"
// shape and stays banned. Reusing the CODE entries by reference (rather than re-deriving the
// regexes) keeps a milestone/task/sweep/narration/spec pattern change from silently diverging
// between the two file classes.
const skillBannedByName = (name) => BANNED.find((b) => b.name === name);
const SKILL_BANNED = [
  skillBannedByName("milestone/task id"),
  skillBannedByName("phase / workstream / invariant id"),
  skillBannedByName("dated plan/spec file"),
  // A skill's dates are almost always narrative ("user directive 2026-08-05"), not the
  // parenthesised or "as of" shape the code pattern requires, so a bare ISO date is its own entry
  // rather than a reuse — narrowing the code pattern to match would weaken it for code files, which
  // the gate must not do.
  {
    name: "date stamp",
    re: /\b20\d\d-\d\d-\d\d\b/,
  },
  skillBannedByName("sweep / round / review marker"),
  skillBannedByName("history narration"),
  skillBannedByName("unnamed spec reference"),
  // EXAMPLE: The "ephemeral doc pointer" category, named apart from the
  // EXAMPLE: shared "repo document pointer" CODE entry because the split is different for
  // EXAMPLE: skills: the code entry also bans a durable architecture reference and a bare
  // EXAMPLE: numbered invariant, for which code has no durable-citation carve-out at all, while
  // EXAMPLE: a skill may cite one of those by its full design-doc path. Matching on the FILENAME
  // EXAMPLE: rather than a generic doc-path prefix is what keeps the split correct without any
  // EXAMPLE: extra carve-out logic: none of these five churn trackers live under the durable
  // EXAMPLE: design-doc directory, and none of the durable design docs are named after one of
  // EXAMPLE: them.
  {
    name: "ephemeral doc pointer",
    re: /\b(?:TODO|PLAN|OPEN_BUGS|CLOSED_BUGS|POST_WORK_FINDINGS)\.md\b/,
  },
  skillBannedByName("local letter+digit marker"),
  skillBannedByName("numbered constraint"),
];

// The file class → ban list mapping, held as one value so both scanners select through the same
// symbol rather than each repeating the conditional; two derivations of the same decision are how
// they come to disagree about a file class.
const BAN_LISTS = { code: BANNED, md: SKILL_BANNED };

/**
 * The ban list governing one file class: the skill ruleset for `.md` prose, the code ruleset
 * otherwise.
 *
 * `lists` is a seam, not a knob — production has exactly one mapping and passes nothing. It exists
 * because no real specimen can pin `scanCandidates`'s half of this selection: every code-list entry
 * a candidate pattern can reach is also reachable from the skill list, and the entries the two
 * lists do not share match no candidate shape at all, so a fixture's classification is identical
 * under either list. Two fabricated lists that deliberately differ are the only way to observe
 * which one a code file is checked against.
 */
function bannedFor(isMd, lists = BAN_LISTS) {
  return isMd ? lists.md : lists.code;
}

// EXAMPLE: A durable design-doc citation can carry digits that are part of its FILENAME, not a
// EXAMPLE: process marker (a milestone-numbered data-model doc under the durable design-doc
// EXAMPLE: directory) — those digits must not feed the milestone/task-id pattern (or any other
// EXAMPLE: SKILL_BANNED pattern) below. The citation is stripped from the subject before
// EXAMPLE: matching only; the reported text still comes from the untouched source line, so a
// EXAMPLE: separate violation riding on the same line (a bare milestone id beside the citation)
// EXAMPLE: still surfaces.
const DESIGN_DOC_CITATION = /docs\/design\/[\w.-]+\.md/g;

// Coverage control: a pattern vocabulary enumerated from remembered examples can always miss a
// shape nobody happened to remember, and reasoning about the pattern list in isolation cannot
// surface that gap — only reading the governed corpus can. This section makes that reading
// repeatable: deliberately BROAD matchers run over the governed skill corpus, and every match is
// required to resolve one of two ways — caught by an existing BANNED/SKILL_BANNED pattern already
// (a real hit, not a coverage gap), or named on an ACKNOWLEDGED list below with a reason. Anything
// left over is RESIDUE: a shape nobody has looked at, and the point is that it fails loudly
// instead of passing silently.
//
// This is a review aid, not a third ban list — it never fails a file itself. `main`'s `--residue`
// mode fails only when RESIDUE is non-empty, never on an acknowledged match.
//
// Two independent candidate SHAPES run side by side because they miss different things: an
// EXAMPLE: identifier-shaped candidate (a letter run plus digits) cannot see a marker with no
// EXAMPLE: digit at all — `pre-fix` has none — which is the gap the second class below
// covers. Neither shape subsumes the other, so both must run for the control to
// cover what BANNED's own entries already ban by shape (an id, and separately a narration phrase).
//
// The identifier shape itself splits into two sub-forms that behave differently per corpus. A
// EXAMPLE: real local design marker is LABEL-shaped and unspaced (`S1`, `D9`, `E8`, `M13-0`). Over
// `src`/`scripts` the spaced sub-form (a capitalized word plus a bare number, e.g. a fixture or
// scene counter) is almost entirely an ordinary-English "Word Number" test-scenario counter rather
// than a marker, while the skill corpus's spaced form carries genuine hits (BANNED's own
// spelled-out "Task N" entry names one spaced shape that IS a real
// marker). The unspaced label form stays a candidate in both corpora; the spaced "Word Number"
// form is scoped to the skill corpus only.
const CANDIDATE_TOKEN_LABEL = /\b[A-Z][A-Za-z]{0,20}\d+[a-z]?(?:-\d+)?\b/g;
const CANDIDATE_TOKEN_WORD = /\b[A-Z][A-Za-z]{0,20}\s\d+[a-z]?(?:-\d+)?\b/g;

// The narration-shaped class: comment text carrying temporal/comparative language about the code
// itself. Scoped narrower than "every word that can describe a past state" on purpose: the common
// English words `new`/`still`/`now`/`was`/`were`/`since`/`until`/`after`/`old` match the
// overwhelming majority of raw candidates over the governed corpus, essentially all of
// them present-tense or runtime-data prose ("the caller now owns the buffer", "if the previous
// sample was inside the mask") with no shape-level way to tell a genuine narration instance from
// the noise. Acknowledging that volume, or worse acknowledging it by a reason that amounts to "the
// word is usually fine," would be exactly the false-negative-to-false-positive swap this class
// exists to avoid: the acknowledged list is not a place to launder low-signal noise, and a list
// that large is not reviewable by anyone.
//
// The pre-/post- compound shape is meaningful in BOTH corpora and runs unconditionally.
const PRE_POST_NARRATION_TOKEN = /\bpre-[a-z]+\b|\bpost-[a-z]+\b/gi;

// The six single words below are meaningful ONLY over the skill corpus, not code: over
// `src`/`scripts`, `moved`/`replaced`/`renamed`/`deprecated` are
// almost exclusively runtime-data or wire-protocol vocabulary — an `AssetChanged` op literally
// named `"replaced"`, a doc comment on "the token that moved", a wire-drift test naming "a renamed
// … enum variant" — none of it narration of the CODE's own past, all of it the ordinary present-
// tense description this class is designed not to flag. The skill corpus is pure prose about a
// subsystem's current shape, where the same six words are mostly genuine narration
// (`legacy` is the sole exception — see ACKNOWLEDGED_NARRATION); code comments are not,
// so this half of the class is scoped to `isMd` rather than widened to swallow that noise.
const WORD_NARRATION_TOKEN =
  /\boriginally\b|\blegacy\b|\bdeprecated\b|\brenamed\b|\bmoved\b|\breplaced\b/gi;

// Named, counted, one reason each — an unnamed or uncounted acknowledgement is a backdoor by the
// same reasoning as the EXAMPLE exemption. Every entry names a token the corpus really carries: a
// product/standard name, a versioned code symbol, or a vendored tool skill's own internal
// structure, none of which is a Shadowcat-process-assigned id.
const ACKNOWLEDGED = [
  {
    name: "product, protocol or algorithm name carrying a version-like number",
    re: /\bNeo4j\b|\bIPv4\b|\bIpv4\b|\bIPv6\b|\bNAT64\b|\bFTS5\b|\bI18n\b|\bPowerShell\s?\d+(?:\.\d+)?\b|\bUUIDv5\b|\bRFC\s?\d+\b|\bArgon2\b|\bSplitMix32\b|\bSplitMix64\b|\bSvelte\s?\d+\b|\bHTTP\s?\d+\b|\bPolyanya\s?\d+\b|\bWin32\b|\bJSON1\b|\bGIF89a\b|\bBM25\b|\bPF1e\b|\bTS2322\b/,
  },
  {
    name: "a versioned product/spec/language-edition name spaced in prose (a real recognisable vocabulary, not an id)",
    re: /\bNode\s?22\b|\bD\s?5e\b|\bCSS\s?3\b/,
  },
  {
    name: "a SQL micro-query/clause literal used as a lightweight existence or connectivity probe, not prose",
    re: /\bSELECT\s?1\b|\bLIMIT\s?1\b/,
  },
  {
    name: "a code symbol cited as a value, not a process id",
    re: /\bPanelLayoutV1\b|\bVec2\b/,
  },
  {
    name: "a Fortran source-extension entry inside a vendored skill's own file-extension list",
    re: /\bF(?:90|95|03|08)\b/,
  },
  {
    name: "a CLI placeholder argument name inside a vendored skill's example command line",
    re: /\bNODE[12]\b/,
  },
  {
    name: "a POSIX character-class fragment inside a vendored skill's shell excerpt",
    re: /\bZ0-9\b/,
  },
  {
    name: "a plain quantity in prose, not an id",
    re: /\bMaximum\s?\d+\b|\bLast\s?\d+\b/,
  },
  {
    name: "a vendored tool skill's own procedural step heading (self-contained table of contents)",
    re: /\b[Ss]teps?[ -]?\d+[a-z]?(?:-\d+)?\b|\bB[0-3]\b/,
  },
  {
    name: "the two-phase validate-then-commit structural label naming apply_intent's Phase 1 validate / Phase 2 insert split",
    re: /\bPhase[ -]?\d+\b/,
  },
  {
    name: "a numbered-rule citation into the durable truthfulness-rules design doc",
    re: /\bRULE\s?\d+\b/,
  },
];

// Deliberately narrow: no entry here matches a bare, single English verb by itself. `re` is
// tested against a short WINDOW starting at the match (the token plus a few trailing
// characters), not the bare token, so an entry can additionally require the specific word
// immediately after the match.
const ACKNOWLEDGED_NARRATION = [
  {
    name: "a pre-/post- compound naming a technical stage, artifact or concept (e.g. pre-pass, post-commit, pre-image), distinct in shape from the compounds BANNED already covers",
    re: /^(?:pre|post)-[a-z]+/i,
  },
  {
    name: "'legacy' naming a still-supported compatibility path or a third-party product's own version",
    re: /^legacy\b/i,
  },
  {
    name: "the passive 'replaced BY' construction stating a substitution/derivation rule",
    re: /^replaced\s+by\b/i,
  },
];

/**
 * Runs the broad candidate matchers over one file's text and classifies every match: already a
 * real BANNED/SKILL_BANNED hit (not a coverage gap — it will already fail the main scan),
 * acknowledged as a named legitimate token, or RESIDUE — an unrecognised shape that must be
 * looked at. Pure function of its argument, mirroring `scanContent`, so a test exercises it on
 * fabricated text without touching the filesystem.
 *
 * `contextChars` sets how many characters past the match an ACKNOWLEDGED entry's `re` can see:
 * 0 for the identifier class (its entries match the bare token), a few for the narration class
 * (an entry there can require a specific following word, e.g. "replaced BY").
 */
/**
 * Extracts the comment/prose text one line contributes, mirroring exactly what `scanContent`
 * checks against BANNED/SKILL_BANNED. Shared so the coverage control (`scanCandidates`) can never
 * see a different subject than the gate does — a defect class this gate has hit three times
 * already (a vocabulary gap, a scope gap, a corpus-filter gap), each time because a second
 * derivation of "what counts" existed somewhere and drifted from the first.
 */
function lineSubject(line, isMd, lexState) {
  if (isMd) {
    return { subject: line.trim().replace(DESIGN_DOC_CITATION, ""), state: lexState };
  }
  const split = splitLine(line, lexState);
  const literals = split.code.match(STRING_LITERAL) ?? [];
  const explanatory = EXPLANATORY_STRING.test(split.code);
  const subject = [
    split.comment,
    ...literals.filter((l) => explanatory || PROSE_LITERAL.test(l.slice(1, -1))),
  ]
    .join(" ")
    .trim();
  return { subject, state: split.state };
}

/**
 * Runs the broad candidate matchers over one file's text and classifies every match: already a
 * real BANNED/SKILL_BANNED hit (not a coverage gap — it will already fail the main scan),
 * acknowledged as a named legitimate token, or RESIDUE — an unrecognised shape that must be
 * looked at. Pure function of its arguments, mirroring `scanContent`, so a test exercises it on
 * fabricated text without touching the filesystem.
 *
 * `isMd` selects the same comment/prose subject extraction `scanContent` uses (`lineSubject`) and,
 * through `bannedFor`, the matching BANNED/SKILL_BANNED list to shadow against, so a code-file
 * candidate already caught by a CODE pattern is not double-reported here, and a skill-file
 * candidate is checked against the skill ruleset rather than the code one. `banLists` overrides
 * that mapping — see `bannedFor` for why the seam exists and why production never passes it.
 *
 * A class's own `contextChars` sets how many characters past the match an ACKNOWLEDGED entry's
 * `re` can see: 0 for the identifier class (its entries match the bare token), a few for the
 * narration class (an entry there can require a specific following word, e.g. "replaced BY").
 */
export function scanCandidates(content, { isMd, banLists } = { isMd: true }) {
  const banned = bannedFor(isMd, banLists);
  const lines = content.split("\n");
  const acknowledged = [];
  const residue = [];
  let exempted = 0;
  let lexState = { inBlock: false, inHtml: false };
  lines.forEach((line, i) => {
    if (EXAMPLE_EXEMPT.test(line)) {
      exempted += 1;
      return;
    }
    const { subject, state } = lineSubject(line, isMd, lexState);
    lexState = state;
    if (subject === "") return;
    const classes = [
      { re: CANDIDATE_TOKEN_LABEL, acks: ACKNOWLEDGED, contextChars: 0 },
      // Skill-only: see CANDIDATE_TOKEN_WORD's own comment for why code is excluded.
      ...(isMd
        ? [{ re: CANDIDATE_TOKEN_WORD, acks: ACKNOWLEDGED, contextChars: 0 }]
        : []),
      { re: PRE_POST_NARRATION_TOKEN, acks: ACKNOWLEDGED_NARRATION, contextChars: 8 },
      // Skill-only: see WORD_NARRATION_TOKEN's own comment for why code is excluded.
      ...(isMd
        ? [{ re: WORD_NARRATION_TOKEN, acks: ACKNOWLEDGED_NARRATION, contextChars: 8 }]
        : []),
    ];
    for (const { re, acks, contextChars } of classes) {
      for (const m of subject.matchAll(re)) {
        const token = m[0];
        if (banned.some((b) => b.re.test(token))) continue;
        const context = subject.slice(m.index, m.index + token.length + contextChars);
        const ack = acks.find((a) => a.re.test(context));
        if (ack) {
          acknowledged.push({ line: i + 1, token, reason: ack.name });
          continue;
        }
        residue.push({ line: i + 1, token, text: line.trim() });
      }
    }
  });
  return { acknowledged, residue, exempted };
}

// The rule extends to code-facing string literals (assert! messages, test names): a developer
// reads an assertion message at failure time exactly as they read a comment, and it goes stale
// the same undetectable way. Ruled in scope by the user.
const STRING_LITERAL = /"(?:[^"\\]|\\.)*"|'(?:[^'\\]|\\.)*'|`(?:[^`\\]|\\.)*`/g;

// The in-scope/out-of-scope line is drawn by the STRING'S OWN SHAPE, not by the syntax around it.
// A literal containing whitespace is prose some human reads; a whitespace-free literal is a token
// the program acts on (a fixture's world name, a document key, an id), where an id-shaped
// collision is not a reference to anything.
//
// Shape, not context, because context is a whitelist and prose escapes through whatever the list
// EXAMPLE: `veto("no top dock zone exists (spec D4)")` reaches a developer at runtime exactly as an
// assertion message does, but sits in no assert/test call. Whitelisting contexts means enumerating
// every way a human-readable string can surface, and being silently wrong each time one is missed.
const PROSE_LITERAL = /\s/;

// Explanatory contexts stay in scope on top of the shape test, since a test name or assertion
// EXAMPLE: message can legitimately be a single token — `it("M13-0")` is prose minus the spaces.
// `.expect(` is Rust's method form and takes a human-readable message. A BARE `expect(` is the
// JavaScript assertion form and takes the SUBJECT under test, so treating its literals as prose
// pulls program data into scope. Only the dotted form qualifies; a genuine message passed to the
// JavaScript form is prose and is already caught by the shape test.
const EXPLANATORY_STRING =
  /\bassert(?:_eq|_ne)?!|\bpanic!|\.expect\(|^\s*(?:async\s+)?(?:test|it|describe)\s*(?:\.\w+)?\s*\(/;

/** Recursively collects paths matching `exts` under `dir`; called once per entry in a roots list. */
export function sources(dir, exts) {
  const out = [];
  for (const name of readdirSync(dir)) {
    if (SKIP_DIRS.has(name)) continue;
    const p = join(dir, name);
    if (statSync(p).isDirectory()) out.push(...sources(p, exts));
    else if (exts.some((e) => name.endsWith(e))) out.push(p);
  }
  return out;
}

/**
 * Scans one file's already-read text for banned references, returning its hits (line + kind +
 * the trimmed source line) and how many lines its EXAMPLE-exempt marker covered.
 *
 * `isMd` selects the subject-extraction mode: a `.md` skill has no code/comment boundary — the
 * whole line is prose — so it is checked directly against SKILL_BANNED, while a code file keeps
 * the comment/string-literal split against the full BANNED list. Pure function of its argument, so
 * a test exercises it on fabricated text without touching the filesystem.
 */
export function scanContent(content, { isMd }) {
  const banned = bannedFor(isMd);
  const lines = content.split("\n");
  const hits = [];
  let exempted = 0;
  let lexState = { inBlock: false, inHtml: false };
  lines.forEach((line, i) => {
    if (EXAMPLE_EXEMPT.test(line)) {
      exempted += 1;
      return;
    }
    // Comment text is checked whole; the code span is checked only inside its string literals, so
    // identifiers and paths that are part of the program are never flagged. Of those literals,
    // prose ones always count and token-shaped ones count only in an explanatory context.
    const { subject, state } = lineSubject(line, isMd, lexState);
    lexState = state;
    if (subject === "") return;
    const hit = banned.find((b) => b.re.test(subject));
    if (hit) hits.push({ line: i + 1, kind: hit.name, text: line.trim() });
  });
  return { hits, exempted };
}

/**
 * The `--residue` coverage control's full computation, over the SAME file set `gateFileSet`
 * returns for `scopes` — not a second derivation of it. Returns the acknowledged tally (grouped
 * by reason) and the residue list, plus how many files were actually read, so a caller (or a
 * test) can compare that count against the gate's own `scanned.length` for the identical scopes
 * and prove the two can never silently diverge.
 */
export function residueReport(scopes = []) {
  const { scanned, isMdFile } = gateFileSet(scopes);
  let ackTotal = 0;
  const ackByReason = new Map();
  const residue = [];
  for (const path of scanned) {
    const content = readFileSync(path, "utf8");
    const result = scanCandidates(content, { isMd: isMdFile.has(path) });
    ackTotal += result.acknowledged.length;
    for (const a of result.acknowledged)
      ackByReason.set(a.reason, (ackByReason.get(a.reason) ?? 0) + 1);
    for (const r of result.residue) residue.push({ path, ...r });
  }
  return { ackTotal, ackByReason, residue, filesScanned: scanned.length };
}

// Query interface. It exists so no caller has to re-derive a subset by grepping this script's
// own output: an ad-hoc pattern is a fresh, unvalidated instrument every time it is written, it
// cannot be compared against a number a DIFFERENT ad-hoc pattern produced earlier, and one keyed
// on `path:line` silently goes stale the moment an edit shifts a line. Ask this script instead.
//   --scope <prefix>   restrict to paths under a prefix (repeatable); forward slashes always
//   --by-area          per-directory table instead of the per-site list
//   --json             machine-readable {total, byKind, byArea, hits}
//   --cover <prefix>   verify prefixes partition the tree disjointly and exhaustively (repeatable)
//   --residue          coverage control: report the skill corpus's unrecognised candidate tokens
function main() {
  const argv = process.argv.slice(2);
  const scopes = argv
    .flatMap((a, i) => (a === "--scope" ? [argv[i + 1]] : []))
    .filter(Boolean);
  const wantArea = argv.includes("--by-area");
  const wantJson = argv.includes("--json");
  const covers = argv
    .flatMap((a, i) => (a === "--cover" ? [argv[i + 1]] : []))
    .filter(Boolean);

  // --residue: the coverage control. Reads `gateFileSet(scopes)` — the exact array the main scan
  // below also reads — so it cannot silently narrow to one corpus. Exits 1 only on non-empty
  // RESIDUE — an acknowledged match is not a failure, it is the mechanism working.
  if (argv.includes("--residue")) {
    const { scanned: residueFiles } = gateFileSet(scopes);
    if (scopes.length > 0 && residueFiles.length === 0) {
      console.error(`--scope matched 0 file(s): ${scopes.join(", ")}`);
      console.error(
        "Nothing was examined, so this is not a clean result. Check the prefix.",
      );
      process.exit(2);
    }
    const { ackTotal, ackByReason, residue, filesScanned } = residueReport(scopes);
    console.log(`${filesScanned} file(s) scanned (code + skill corpora).`);
    console.log(`${ackTotal} acknowledged candidate(s):`);
    for (const [reason, n] of [...ackByReason].sort((a, b) => b[1] - a[1]))
      console.log(`  ${String(n).padStart(4)}  ${reason}`);
    console.log("");
    if (residue.length === 0) {
      console.log("0 unrecognised candidate(s). Coverage control: clean.");
      process.exit(0);
    }
    console.error(`${residue.length} unrecognised candidate(s):`);
    for (const r of residue)
      console.error(`  ${r.path}:${r.line}  ${JSON.stringify(r.token)}  ${r.text}`);
    console.error(
      "\nEach must become a BANNED/SKILL_BANNED pattern (genuine miss) or a named, reasoned " +
        "ACKNOWLEDGED entry (legitimate token) — never silently ignored.",
    );
    process.exit(1);
  }

  const { scanned, isMdFile } = gateFileSet(scopes);
  const hits = [];
  let exempted = 0;
  for (const path of scanned) {
    const content = readFileSync(path, "utf8");
    const result = scanContent(content, { isMd: isMdFile.has(path) });
    exempted += result.exempted;
    for (const h of result.hits) hits.push({ path, ...h });
  }

  // A scope that matches no files and a scope that is genuinely clean both produce zero hits, and
  // telling them apart by eye is impossible — a mistyped prefix reads as success. Refuse to report
  // the ambiguous zero: a scope matching no files is an error, never a pass.
  if (scopes.length > 0 && scanned.length === 0) {
    console.error(`--scope matched 0 files: ${scopes.join(", ")}`);
    console.error(
      "Nothing was examined, so this is not a clean result. Check the prefix.",
    );
    process.exit(2);
  }

  // A bare count carries no record of the instrument that produced it, so a widened pattern and a
  // regressed codebase are the same number going up, and a broken scanner and a clean scope are the
  // same zero. Every count this script prints is therefore stamped with a fingerprint of the rules
  // that produced it, and a run whose fingerprint differs from the previous run for the same scope
  // says so instead of inviting a comparison that is not valid.
  //
  // Hashed function sources are line-ending-normalized. `Function.prototype.toString` returns the
  // literal source text, so the same function hashes differently depending on how the file reached
  // the disk — git stores LF and checks out CRLF on Windows. Without normalizing, a CI run and a
  // local run report different instruments for identical rules, and every comparison between them is
  // refused for a reason that has nothing to do with the rules.
  const stableSource = (fn) => fn.toString().split("\r\n").join("\n");

  const INSTRUMENT = createHash("sha256")
    .update(
      JSON.stringify([
        BANNED.map((b) => [b.name, b.re.source, b.re.flags]),
        SKILL_BANNED.map((b) => [b.name, b.re.source, b.re.flags]),
        [
          stableSource(splitLine),
          STRING_LITERAL.source,
          EXPLANATORY_STRING.source,
          PROSE_LITERAL.source,
          EXAMPLE_EXEMPT.source,
          DESIGN_DOC_CITATION.source,
        ],
        ROOTS,
        EXTS,
        MD_ROOTS,
        MD_EXTS,
        [...SKIP_DIRS].sort(),
        // The scope-matching rule is part of the ruler: changing what a prefix claims changes every
        // scoped count without touching a pattern. Hashing the function's source keeps the
        // fingerprint honest without anyone remembering to bump a version.
        stableSource(under),
      ]),
    )
    .digest("hex")
    .slice(0, 8);

  // A run memo, so the next run can distinguish a changed ruler from changed code. It lives in the
  // conventional JS tool cache rather than any workspace a process creates: a durable script that
  // hardcodes a scaffolding directory stops working when that scaffolding is renamed or cleaned, and
  // the failure is a silently skipped comparison, not an error.
  const STATE_DIR = "node_modules/.cache/comment-refs";
  const STATE_PATH = `${STATE_DIR}/instrument.json`;
  const scopeKey =
    scopes.length > 0 ? scopes.map(norm).sort().join(",") : "<repo>";

  /** Prior run for this scope, or null. Absent/corrupt state is simply no prior run. */
  function priorRun() {
    try {
      return JSON.parse(readFileSync(STATE_PATH, "utf8"))[scopeKey] ?? null;
    } catch {
      return null;
    }
  }

  /** Records this run so the NEXT one can tell an instrument change from a code change. */
  function recordRun(total) {
    try {
      let all = {};
      try {
        all = JSON.parse(readFileSync(STATE_PATH, "utf8"));
      } catch {
        /* first run, or unreadable state — start fresh */
      }
      all[scopeKey] = {
        instrument: INSTRUMENT,
        total,
        filesScanned: scanned.length,
      };
      mkdirSync(STATE_DIR, { recursive: true });
      writeFileSync(STATE_PATH, JSON.stringify(all, null, 2));
    } catch {
      /* state is an aid, never a gate: a read-only checkout must still be able to run this */
    }
  }

  /** The line that makes a count self-describing. Print it beside every total. */
  function provenance(total) {
    const prior = priorRun();
    const ex = exempted > 0 ? `; ${exempted} line(s) EXAMPLE-exempt` : "";
    const head = `instrument ${INSTRUMENT}; ${scanned.length} file(s) scanned${ex}`;
    if (!prior) return `${head}; no prior run recorded for this scope`;
    if (prior.instrument !== INSTRUMENT) {
      return (
        `${head}\nINSTRUMENT CHANGED since the last run for this scope ` +
        `(${prior.instrument} -> ${INSTRUMENT}). The previous total of ${prior.total} was measured ` +
        `by different rules and is NOT comparable to ${total}. Any movement here is the ruler, ` +
        `not necessarily the code.`
      );
    }
    const delta = total - prior.total;
    const dir =
      delta === 0 ? "unchanged" : delta > 0 ? `+${delta}` : `${delta}`;
    return `${head}; same instrument as last run: ${prior.total} -> ${total} (${dir})`;
  }

  // --cover: do these prefixes partition the tree, disjointly and exhaustively?
  //
  // Whenever a surface is divided for parallel work, the division is the least-reviewed artifact
  // involved: written once, read by each worker only for its own slice, never checked whole. Its
  // failure modes are asymmetric. An overlap duplicates effort loudly enough to notice; a file no
  // slice claims is simply never examined, while every worker still reports a clean scope and the
  // totals still look plausible. Unmeasured coverage is indistinguishable from complete coverage.
  //
  // This asks only about durable things — path prefixes and the files actually on disk. It has no
  // notion of a task, an owner, a campaign or a plan: those expire, and durable tooling that models
  // them rots in step with them while continuing to report success. The prefix-to-worker mapping
  // belongs in the ephemeral brief that assigns it; the question of whether a set of prefixes covers
  // the tree is a property of the tree alone, and is answered here.
  //
  // Fails on: a prefix matching no file (a typo is indistinguishable from a legitimately empty
  // slice), a file claimed by two prefixes, and any scanned file no prefix claims.
  if (covers.length > 0) {
    if (scopes.length > 0) {
      console.error(
        "--cover measures the whole tree; do not combine it with --scope.",
      );
      process.exit(2);
    }
    const owner = new Map();
    const problems = [];

    for (const prefix of covers) {
      const matched = scanned.filter((p) => under(p, prefix));
      if (matched.length === 0) {
        problems.push(
          `EMPTY    "${prefix}" matches no file. A typo reads as an empty slice.`,
        );
      }
      for (const p of matched) {
        if (owner.has(p))
          problems.push(
            `OVERLAP  ${p} claimed by "${owner.get(p)}" and "${prefix}".`,
          );
        else owner.set(p, prefix);
      }
    }

    const unclaimed = scanned.filter((p) => !owner.has(p));
    if (unclaimed.length > 0) {
      const n = hits.filter((h) => !owner.has(h.path)).length;
      problems.push(
        `GAP      ${unclaimed.length} file(s) under no prefix, holding ${n} site(s):`,
      );
      for (const p of unclaimed.slice(0, 20)) {
        const c = hits.filter((h) => h.path === p).length;
        problems.push(`           ${p}${c ? `  (${c} site(s))` : ""}`);
      }
      if (unclaimed.length > 20)
        problems.push(`           … ${unclaimed.length - 20} more`);
    }

    console.log(provenance(hits.length));
    console.log("");
    for (const prefix of covers) {
      const n = hits.filter((h) => owner.get(h.path) === prefix).length;
      const f = scanned.filter((p) => owner.get(p) === prefix).length;
      console.log(
        `  ${String(n).padStart(4)} site(s)  ${String(f).padStart(4)} file(s)  ${prefix}`,
      );
    }
    console.log("");
    if (problems.length === 0) {
      console.log(
        `OK: ${covers.length} prefix(es) cover all ${scanned.length} file(s) exactly once.`,
      );
      console.log(`Sites accounted for: ${hits.length}.`);
      process.exit(0);
    }
    for (const p of problems) console.error(p);
    console.error(
      `\n${problems.length} structural problem(s). This split is not safe to dispatch.`,
    );
    process.exit(2);
  }

  /** Groups by the first three path segments (`src/server/src`, `src/modules/panels`). */
  const tally = (keyOf) => {
    const m = new Map();
    for (const h of hits) m.set(keyOf(h), (m.get(keyOf(h)) ?? 0) + 1);
    return [...m].sort((a, b) => b[1] - a[1]);
  };
  const byKind = tally((h) => h.kind);
  const byArea = tally((h) => h.path.split("/").slice(0, 3).join("/"));
  const files = new Set(hits.map((h) => h.path)).size;

  if (wantJson) {
    const prior = priorRun();
    console.log(
      JSON.stringify(
        {
          total: hits.length,
          instrument: INSTRUMENT,
          comparableToPrior: prior ? prior.instrument === INSTRUMENT : null,
          prior,
          filesScanned: scanned.length,
          filesWithHits: files,
          byKind,
          byArea,
          hits,
        },
        null,
        2,
      ),
    );
    recordRun(hits.length);
    process.exit(hits.length > 0 ? 1 : 0);
  }

  if (wantArea) {
    const scopeNote = scopes.length > 0 ? ` under ${scopes.join(", ")}` : "";
    console.log(`${hits.length} site(s) in ${files} file(s)${scopeNote}`);
    console.log(provenance(hits.length));
    for (const [area, n] of byArea)
      console.log(`${String(n).padStart(5)}  ${area}`);
    recordRun(hits.length);
    process.exit(hits.length > 0 ? 1 : 0);
  }

  if (hits.length > 0) {
    console.error(`${hits.length} ephemeral reference(s).`);
    console.error(
      "A code comment or a skill must be durable commentary and cite nothing ephemeral.",
    );
    console.error(provenance(hits.length));
    console.error(
      "Fix by stating the CONSTRAINT inline and dropping the POINTER.\n",
    );
    for (const [kind, n] of byKind)
      console.error(`  ${String(n).padStart(4)}  ${kind}`);
    console.error("");
    for (const [area, n] of byArea)
      console.error(`  ${String(n).padStart(4)}  ${area}`);
    console.error("");
    for (const h of hits)
      console.error(`  ${h.path}:${h.line}  [${h.kind}]  ${h.text}`);
    recordRun(hits.length);
    process.exit(1);
  }

  console.log(`No ephemeral references.`);
  console.log(provenance(0));
  recordRun(0);
}

// The scan-and-report pipeline runs only under `isDirectEntry`, never on import — a
// test imports `scanContent` to exercise the ruleset against fabricated text, and running the whole
// repo scan (with its own `process.exit`) as a side effect of that import would make the function
// untestable in isolation.
if (isDirectEntry(import.meta.url)) main();
