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
  existsSync,
} from "node:fs";
import { join } from "node:path";
import { createHash } from "node:crypto";
import { isDirectEntry } from "./lib/is-main.mjs";
// Scope vocabulary and the specimen marker live one level down, in the module both documentation
// gates import from. Importing the sibling gate instead formed a cycle: nothing evaluated an
// imported binding at module scope, so it worked by accident rather than by construction.
import {
  EXAMPLE_EXEMPT,
  GENERATED_ROOT,
  defaultSkillsRoot,
  listSkillDirs,
  MD_EXTS,
  norm,
  SKIP_DIRS,
  sources,
  under,
} from "./lib/gate-corpus.mjs";
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

// Repo-root config files are code too — an eslint config decides what ships. They are collected by
// walking the root non-recursively rather than by listing them, so a new config is in scope the
// day it is added instead of the day someone remembers to enumerate it.
const rootFiles = () =>
  readdirSync(".").filter(
    (n) => EXTS.some((e) => n.endsWith(e)) && statSync(n).isFile(),
  );

/** True when `p` sits inside one of `scopes` (or `scopes` is empty, meaning "everything"). */
export const inScope = (scopes, p) =>
  scopes.length === 0 || scopes.some((s) => under(p, s));

// Single source of the two corpora this gate governs. Every consumer — the main scan, `--cover`,
// and the `--residue` coverage control — calls this one function rather than re-deriving its own
// path list, so a future change to what the gate scans cannot silently leave the control scanning
// a narrower set: there is no second derivation left to drift.
//
// The skill corpus is a standalone plugin checkout since the shadowcat-codebase migration, not
// part of this repo — CI never has it, so an absent `skillsRoot` degrades to an empty skill corpus
// (mdFiles/untrackedSkillFiles both []) rather than failing the whole gate: the CODE corpus is
// this repo's own and must always be checked, independent of whether a skills checkout exists on
// this machine.
//
// Present, the skill corpus is scoped to the TRACKED skill directories, read from the same
// `listSkillDirs` the skill-symbol-citation gate reads (now scoped to the skills checkout's own
// git repo). An untracked skill directory is vendored third-party prose: this repo neither wrote
// it nor may edit it, so holding it to a rule about how THIS repo writes prose leaves only two
// outcomes, editing a vendored file or carving out an exemption, and both are wrong. Tracked-ness
// is the durable property that says whose prose it is — never a name pattern, which would have to
// be updated for every vendored tool and reads as clean when it is not. Sharing the derivation is
// what keeps the two skill gates from disagreeing about the size of the corpus; the excluded count
// prints on every run, because an uncounted exclusion is a backdoor.
export function collectFiles(skillsRoot = defaultSkillsRoot()) {
  const codeFiles = [
    ...ROOTS.flatMap((d) => sources(d, EXTS)),
    ...rootFiles(),
  ]
    .map(norm)
    .filter((p) => !under(p, GENERATED_ROOT));
  const generatedFiles = sources(GENERATED_ROOT, EXTS).map(norm);
  if (!existsSync(skillsRoot)) return { codeFiles, mdFiles: [], generatedFiles, untrackedSkillFiles: [] };
  const dirs = listSkillDirs(skillsRoot);
  if (dirs === null)
    throw new Error(
      "git could not list the skill corpus: this gate scopes skill prose by tracked-ness and " +
        `cannot report a trustworthy count without it (looked in ${skillsRoot}).`,
    );
  const mdFiles = [...dirs.tracked]
    .flatMap((name) => sources(join(skillsRoot, name), MD_EXTS))
    .map(norm);
  const untrackedSkillFiles = dirs.untracked
    .flatMap((name) => sources(join(skillsRoot, name), MD_EXTS))
    .map(norm);
  return { codeFiles, mdFiles, generatedFiles, untrackedSkillFiles };
}

/**
 * The gate's own scope-filtered file set, plus a lookup for which of those files are skill prose
 * rather than code. `--residue` reads this exact return value — see `residueReport` — so its
 * corpus cannot diverge from the gate's without changing this one function.
 */
export function gateFileSet(scopes = []) {
  const { codeFiles, mdFiles, generatedFiles, untrackedSkillFiles } = collectFiles();
  const isMdFile = new Set(mdFiles);
  const scanned = [...codeFiles, ...mdFiles].filter((p) => inScope(scopes, p));
  const generatedExcluded = generatedFiles.filter((p) => inScope(scopes, p)).length;
  const untrackedSkillExcluded = untrackedSkillFiles.filter((p) => inScope(scopes, p)).length;
  return { scanned, isMdFile, generatedExcluded, untrackedSkillExcluded };
}

// Patterns below are documented by describing the shape they match wherever describing is as clear
// as showing. Where a specimen genuinely carries more than a description — a phrase whose exact
// wording is the thing being matched — the line carries `EXAMPLE_EXEMPT`'s marker and is skipped.
// That marker is shared with the skill-symbol-citation gate and defined beside the rest of the
// corpus vocabulary; its active count prints with every result here, because an exemption nobody
// counts is a backdoor. It is the only exemption this scanner has.

// The comment/code split, and its controls, live in one module so this gate and the
// suppression gate cannot drift apart about what counts as a comment.
import { splitLine } from "./lib/comment-span.mjs";

export const BANNED = [
  // A capital M, digits, an optional letter and an optional dashed number are one id shape: the
  // unsuffixed form carries no less process identity than the suffixed one, so a pattern that
  // required the suffix would read the short form as clean. The spelled-out `Task N` form is the
  // same id shape written in full rather than abbreviated behind the capital letter, so it is the
  // same category, not a second one: both writers point outside the code identically, so a pattern
  // carrying only the abbreviated form reads the spelled-out one as clean.
  //
  // `Task[\s]+` spells the word separator as a one-member class so `separatorFlexible` reaches the
  // hyphen and underscore writers too — see `SEPARATOR_CLASS`. The sub-number separator in
  // `(?:-\d+)?` is deliberately NOT respelled: the group is optional and its head already matches,
  // so widening it can only extend the reported span, never reach a subject the entry was missing.
  { name: "milestone/task id", re: /\bM\d+[a-z]?(?:-\d+)?\b|\bTask[\s]+\d+[a-z]?(?:-\d+)?\b/ },
  // A capital D, I, T or W followed by digits: phase checkpoints, numbered invariants, tasks and
  // workstreams. All are ids a process assigns, resolvable only by a reader holding that artifact.
  // The T form collides with a generic type parameter, but only where a comment names one WITHOUT
  // backticks — and a comment naming a type is required to cite it as a symbol regardless, so the
  // collision resolves the same way every other value/document collision here does.
  //
  // The separated writers are reached by ALTERNATIVES rather than by respelling a separator: this
  // entry's own spelling has no separator for `separatorFlexible` to widen. Written as a class
  // (`[DITW][-_]?`) they WOULD be widened, and that widening admits the SPACE writer, where a
  // single capital followed by a space and a quantity is ordinary English — the same collision
  // that keeps `local letter+digit marker`'s hyphen at one spelling. Each alternative's separator
  // sits between a group boundary and an escape, which the neighbour test leaves alone, so the two
  // spellings written here stay the two spellings matched. Measured over both corpora at 0 live
  // lines each: they cost nothing and close a spelling a future author would otherwise walk
  // through.
  {
    name: "phase / workstream / invariant id",
    re: /\b[DITW]\d+\b|\b[DITW]-\d+\b|\b[DITW]_\d+\b/,
  },
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
  //
  // Its separators are the one place in this list where a word separator stays at its single
  // spelling deliberately, and the bound is measured rather than assumed. Respelling the optional
  // hyphen as a one-member class would admit the SPACE writer, and an initial followed by a space
  // and a digit is ordinary English — an indefinite article in front of a quantity, as in a
  // one-hex token or a three-face die — which fires 13 times across the governed corpora.
  //
  // Widening a separator is safe in the direction that ADDS a hyphen or underscore between two
  // words, because nobody writes English that way by accident. It is not safe in the direction that
  // adds a SPACE between a letter and a digit, because that fuses two ordinary tokens into a
  // marker. The optional sub-number group is unwidenable for the reason its counterpart in the
  // milestone entry is: the group is optional and its head already matches.
  { name: "local letter+digit marker", re: /\b[ACFHRV]-?\d(?:-\d+)?\b/ },
  {
    name: "repo document pointer",
    re: /docs\/[\w./-]+\.md|\b(?:TODO|OPEN_BUGS|CLOSED_BUGS|POST_WORK_FINDINGS|ARCHITECTURE|PLAN)\.md/i,
  },
  // The same five churn trackers named WITHOUT the extension. The extension is a spelling, not the
  // referent, so requiring it lets the identical pointer through by dropping four characters.
  //
  // A POINTER CONSTRUCTION is required rather than a bare occurrence, and that is the whole
  // precision of the entry: a preposition or verb of reference in front of the name is what turns
  // it into a deferral to a document. The bare marker form is PERMITTED — a lone `TODO` is a code
  // marker the rule keeps deliberately — and these words also occur as ordinary prose, so keying on
  // the name alone would flag both. Preventive: zero live sites, and the entry must not change any
  // tracked file's verdict.
  {
    name: "extensionless tracker pointer",
    re: /\b(?:see|in|per|from|under|logged[\s]+in|recorded[\s]+in|tracked[\s]+in|filed[\s]+in|listed[\s]+in|cites?|names?|references?)[\s]+(?:the[\s]+)?(?:TODO|OPEN_BUGS|CLOSED_BUGS|POST_WORK_FINDINGS|PLAN)\b(?!\.md)/,
  },
  // A comment naming a codebase skill BY NAME points at a knowledge artifact outside the code whose
  // identity a process assigns: skills are created, renamed, split and retired, and when one goes
  // the comment points at nothing with nothing in the code to say so — the same undetectability
  // every other entry here exists to remove.
  //
  // CODE ONLY, deliberately absent from the skill list: a skill naming a sibling skill is the
  // documented, intended structure of that knowledge layer, and the core skill's own subsystem list
  // is written that way. The entry is scoped to the corpus where the reference is a defect, not
  // widened to the corpus where it is the design.
  {
    name: "codebase skill pointer",
    re: /\bshadowcat-codebase-[a-z][a-z-]*\b/,
  },
  // The PATHLESS forms of the same reference, held apart from the entry above because the two
  // halves are governed differently outside code. A skill may cite a durable design document by
  // path plus anchor, and the entry above is what that carve-out has to make room for; these forms
  // name the same document with the path omitted, so nothing tells a reader which file to open and
  // the carve-out has nothing to key on. Splitting is what lets the skill list reuse THIS half by
  // reference under a guard, instead of substituting a narrower entry and dropping the check.
  //
  // The separator between the invariant keyword and its number is a word separator spelled as a
  // one-member class, so the hyphenated and underscored writers reach the entry alongside the
  // spaced one. The `\s*` before `[§#]` is optional space in front of a punctuation mark rather
  // than a word joiner, so it keeps its single spelling.
  {
    name: "pathless durable document reference",
    re: /ARCHITECTURE\s*[§#]|\binvariant[\s]*#?[\s]*\d+/i,
  },
  // A date's hyphens are its FORMAT, not a word separator, so they keep their single spelling in
  // this entry and in `date stamp` below: widening them would read `2026 08 17.md` as a date.
  { name: "dated plan/spec file", re: /\b20\d\d-\d\d-\d\d[\w-]*\.md/ },
  // A `Constraint N` / `Global Constraint N` reference is the same bare-numbered-registry shape as
  // the `invariant N` form above: it names an item outside the line with no document to resolve it
  // against, and the number is assigned by a process that renumbers freely. The numbered section of
  // a plan document is exactly the repo-document pointer this list already bans, written without
  // the filename that would at least say which document went stale.
  {
    name: "numbered constraint",
    // Both word separators are spelled as one-member classes, so the hyphenated and underscored
    // writers of the qualifier and of the number reach the entry alongside their spaced ones.
    re: /\b(?:Global[\s]+)?Constraint[\s]*#?[\s]*\d+\b/i,
  },
  // A date stamps a comment with when someone wrote it, which is not behaviour. A match requires a
  // parenthesised or "as of" form, because a bare ISO date also appears inside illustrative
  // program data (a backup path, a sample record) where it names a value rather than a writing.
  {
    name: "date stamp",
    // The separator before the date is spelled as a one-member class so the whole phrase widens
    // together. Only the separator INSIDE the qualifying phrase passed the neighbour test, so a
    // fully hyphenated writer of the phrase went unmatched while a half-hyphenated one matched —
    // one marker reachable under two of its three writers, split on nothing a reader could see.
    re: /\(\s*20\d\d-\d\d-\d\d\s*\)|\bas of[ ]\d|\bas of[ ](?:Jan|Feb|Mar|Apr|May|Jun|Jul|Aug|Sep|Oct|Nov|Dec)/i,
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
    // Every word separator in the phrase and compound forms is spelled as a one-member class. Both
    // sit next to a GROUP boundary, which the bare-separator widening cannot read, so without the
    // respelling the phrase form matched only its spaced writer and the compound form only its
    // hyphenated one. That left the hyphenated and underscored writers of the phrase reachable by
    // any author who happens to hyphenate, and this entry's own prose names that phrase as the
    // canonical banned form — the widest gap in the list, because it is the most-cited entry.
    re: /\bpreviously\b|\bformerly\b|\bhistorically\b|\b(?:before|after)[ ]the[ ](?:fix|refactor|change|rewrite)\b|\bpre[-](?:fix|refactor)\b|\bpost[-](?:fix|refactor)\b|\b(?:it|this|that|which|they)\s+used to\b/i,
  },
  // History narration by ALLUSION: the sibling gap the entry above cannot see. The entry above
  // anchors on a fixed word marking the narration itself; an allusive reference to an incident
  // carries no such word — the incident is named as though the reader already knows it. The
  // tractable property is a CONSTRUCTION rather than a wordlist of incidents: a DETERMINER, an
  // OBSERVATION/REPORTING PARTICIPLE, and an INCIDENT NOUN, in that order. The determiner and
  // participle are closed grammatical classes and are enumerated directly; the incident itself is
  // what stays open-ended, and this construction lets the pattern anchor around it without ever
  // naming it.
  //
  // EXAMPLE: "the reported panic" is the construction's canonical shape: determiner, reporting
  // EXAMPLE: participle, incident noun, with no lexical marker on the incident itself.
  //
  // The collision this is designed against: the same nouns name a CODE CONSTRUCT rather than an
  // event (a panic path, a crash handler, a failure mode, an error type) — visible in the code,
  // not a reference to something that happened. The reporting participle is what separates the
  // two: a construct name is a bare determiner plus noun, never a reporting participle between
  // them. A negative lookahead additionally refuses a construct-noun suffix immediately after the
  // incident noun, since the participle alone does not resolve a compound like "the observed
  // failure mode" — that phrase describes the mode's observability, not a reported incident.
  //
  // RESIDUAL, by design: this pattern requires the participle to sit directly between the
  // determiner and the noun. A reordered allusion ("the panic that was reported"), an allusion
  // carrying no participle at all, or an incident noun outside this enumerated set is invisible
  // to it — allusion is semantic and this pattern is lexical, so a clean run over this
  // construction is NOT evidence that no allusive reference remains. Catching those is a review
  // obligation, on the same footing as the lowercase hyphenated local-marker class the core skill
  // documents as permanently ungated by design.
  //
  // The suffix guard is also a closed word list (`mode`, `path`, `handler`, `type`, `kind`,
  // `variant`, `case`), so it runs the opposite risk: a construct compound using a suffix outside
  // that list (a reported error STATE, a known crash recovery MECHANISM) still reads as a false
  // positive. A false positive is visible on the next run and a false negative is not, so the
  // pressure this produces always points toward narrowing the pattern — the correct response to a
  // genuine miss here is to extend the suffix list, never to drop the participle requirement or
  // narrow the noun sets that make the construction detectable at all.
  {
    name: "history narration by allusion",
    re: /\b(?:the|that|this|these|those)\s+(?:reported|logged|observed|documented|noted|described|discovered|identified|known|flagged|surfaced|raised|witnessed|filed)\s+(?:panic|crash|bug|failure|incident|regression|outage|defect|deadlock|leak|corruption|exploit|vulnerability)s?\b(?!\s+(?:path|handler|mode|type|kind|variant|case))/i,
  },
  // EXAMPLE: An unnamed reference to "the spec" is the same defect as a named one and strictly
  // worse to resolve: the reader cannot even tell which document went stale. Matches a spec
  // DOCUMENT, not the bare word — that word is also a common parameter name and an end-to-end
  // test-file suffix, and neither points outside the code.
  {
    name: "unnamed spec reference",
    // The trailing-colon form excludes `::`, which is a Rust path segment (a `dice::spec::DieKind`
    // in a doctest names a module in this crate) rather than a document reference.
    //
    // The whitespace before `spec` is the one word separator here that must NOT be respelled as a
    // one-member class. Hyphenated, the words form an adjectival compound with a different
    // referent: `per-spec` qualifies a `RollSpec`-scoped field, not a document, and the widened
    // form fires on it. The space is load-bearing evidence that "spec" is being used as a noun.
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
    // The separator after the section keyword is spelled as a one-member class, so its hyphenated
    // and underscored writers reach the entry. Nothing inside the negative lookbehind is widened at
    // any depth — the mechanism refuses it, because widening an exclusion makes the pattern match
    // strictly less.
    re: /(?<!\b(?:RFC|ISO|IEC|IEEE|ANSI|W3C|WHATWG|Unicode)[\s-]?\d{1,5}[,;:]?\s{0,3})§\s*\d|\bSection[\s]+\d+(?:\.\d+)*\b/,
  },
  // EXAMPLE: "the brief"/"this brief" names the same scaffolding as "task brief"/"dispatch brief"
  // EXAMPLE: above, but the bare noun collides with the brief-rules checker's own subject matter,
  // EXAMPLE: which describes what a dispatch brief IS throughout its prose ("an implementer obeys
  // EXAMPLE: the brief, not the guidance" — "brief" as the deferring verb's OBJECT, describing the
  // EXAMPLE: category rather than pointing at one specific, now-gone document). The pointer
  // EXAMPLE: CONSTRUCTION — a possessive ("the brief's X" / "this brief's X") or the determiner-
  // EXAMPLE: plus-brief phrase acting as a deferring verb's SUBJECT ("the brief requires" / "says" /
  // EXAMPLE: "specifies" / "states" / "calls for") — is what a genuine reference looks like; the
  // EXAMPLE: collision above puts "brief" on the other side of the verb and stays unmatched.
  {
    name: "unnamed brief pointer",
    // The separator between the determiner group and "brief", and the one before the deferring
    // verb, both sit next to a group boundary the bare-separator widening cannot read, so both are
    // spelled as one-member classes.
    re: /\b(?:the|this)[\s]+brief'?s\b|\b(?:the|this)[\s]+brief[\s]+(?:requires|says|specifies|states|calls for)\b/i,
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
    //
    // The separator before each number is spelled as a one-member class. Both sit next to an
    // ESCAPE, which the bare-separator widening cannot read, so the hyphenated and underscored
    // writers of a numbered finding and of a numbered severity went unmatched entirely.
    re: /\b[Ss]weeps?[ -]\d+|\bfix[- ]round|\bbuddy-check|\bwhole-branch[- ]review|\bfinding[ ]\d+|\b(?:critical|important|major|minor|blocker|fix|issue|bug)[\s]*#?[\s]*\d+\b/i,
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
    // The separator before the noun sits next to a GROUP boundary, which the bare-separator
    // widening cannot read, so it is spelled as a one-member class. Without it the entry reached
    // only the spaced writer of each qualifier, while the hyphenated and underscored writers of
    // the same two-word marker passed clean.
    re: /POST_WORK:|\b(?:task|dispatch)[ ]brief\b|\bthe plan\b/i,
  },
];

// EXAMPLE: A durable design-doc citation can carry digits that are part of its FILENAME, not a
// EXAMPLE: process marker (a milestone-numbered data-model doc under the durable design-doc
// EXAMPLE: directory) — those digits must not feed the milestone/task-id pattern (or any other
// EXAMPLE: SKILL_BANNED pattern) below. The citation is stripped from the subject before
// EXAMPLE: matching only; the reported text still comes from the untouched source line, so a
// EXAMPLE: separate violation riding on the same line (a bare milestone id beside the citation)
// EXAMPLE: still surfaces.
const DESIGN_DOC_CITATION = /docs\/design\/[\w.-]+\.md/g;

/**
 * Whether one RAW source line carries a full durable design-doc path — the citation form a skill
 * is permitted to use, and the only evidence that a section anchor beside it resolves to something.
 *
 * Read against the RAW line, never the subject: the Markdown branch of `lineSubject` replaces every
 * design-doc citation with the empty string before matching, which deletes exactly the evidence
 * this predicate needs. That strip stays — it exists because a durable filename can carry digits
 * that would otherwise feed the milestone-id pattern — so the two read different texts on purpose.
 *
 * The unit is the LINE, not the token. Several permitted citations name the path once and carry a
 * second anchor later on the same line, joined by a `+`; a predicate keyed to immediate adjacency
 * would admit the first anchor and flag the second. The unit is not the GROUP either, even though
 * the group is what every ban pattern matches against: one permitted citation anywhere in a
 * paragraph would then exempt every bare anchor sharing that paragraph, and a hole that opens when
 * unrelated prose happens to sit nearby is the invisible failure direction this gate refuses. The
 * cost is that a citation wrapped across two lines must carry its path on the line the anchor sits
 * on, which is a visible failure an author fixes by reflowing.
 *
 * Derived from `DESIGN_DOC_CITATION` rather than restating its source: a second spelling of "this
 * names a durable design doc" is free to disagree with the first about what a durable path is.
 * The clone drops the global flag, whose `lastIndex` state would make `test` answer differently on
 * successive calls with the same argument.
 *
 * @param {string} rawLine - one untouched source line.
 * @returns {boolean} whether the line names a durable design document by its full path.
 */
const namesDurableDesignDoc = (rawLine) =>
  new RegExp(DESIGN_DOC_CITATION.source).test(rawLine);

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
export const SKILL_BANNED = [
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
  // Same defect, same corpus reasoning: a skill's prose points at an incident by allusion exactly
  // as a code comment does, and the construction is the same one — see the CODE entry's own
  // comment for the property and its residual coverage bound.
  skillBannedByName("history narration by allusion"),
  skillBannedByName("unnamed spec reference"),
  // EXAMPLE: The "ephemeral doc pointer" category, named apart from the
  // EXAMPLE: shared "repo document pointer" CODE entry because the split is different for
  // EXAMPLE: skills: code may cite no repo document at all, while a skill may cite a durable
  // EXAMPLE: design document by its full path. Matching on the FILENAME rather than a generic
  // EXAMPLE: doc-path prefix is what keeps the split correct without any extra carve-out logic:
  // EXAMPLE: none of these five churn trackers live under the durable design-doc directory, and
  // EXAMPLE: none of the durable design docs are named after one of them.
  {
    name: "ephemeral doc pointer",
    re: /\b(?:TODO|PLAN|OPEN_BUGS|CLOSED_BUGS|POST_WORK_FINDINGS)\.md\b/,
  },
  skillBannedByName("local letter+digit marker"),
  skillBannedByName("numbered constraint"),
  // The two PATHLESS reference forms, reused from the code list by reference and QUALIFIED by the
  // carve-out rather than dropped for it. A skill may name a durable design document, so a bare
  // architecture reference, a bare numbered invariant and a bare section anchor are permitted
  // exactly when the line also carries the path that says WHICH document — and banned otherwise,
  // where they are strictly worse than a stale named citation: nothing tells the reader which file
  // to go and fail to find.
  //
  // `skipLine` is a predicate on the ban RECORD rather than a pre-filter over the list, so the one
  // decision "this line names a durable doc" is stated once and read wherever the record is applied.
  // A pre-filter would have to re-derive it at every site that selects a ban list, and two
  // derivations of one decision are how the two come to disagree.
  { ...skillBannedByName("pathless durable document reference"), skipLine: namesDurableDesignDoc },
  { ...skillBannedByName("unnamed section pointer"), skipLine: namesDurableDesignDoc },
];

// The file class → ban list mapping, held as one value so both scanners select through the same
// symbol rather than each repeating the conditional; two derivations of the same decision are how
// they come to disagree about a file class.
const BAN_LISTS = { code: BANNED, md: SKILL_BANNED };

/**
 * Refuses a ban pattern carrying the global flag, at CONSTRUCTION rather than at report time.
 *
 * A global pattern carries mutable `lastIndex` state, so the same pattern applied to successive
 * subjects starts wherever the previous subject left off and skips real hits; `separatorFlexible`
 * preserves flags, so a derived form inherits the defect. Failing here names the offending entry,
 * while failing at report time looks exactly like a clean corpus.
 *
 * @param {Record<string, {name: string, re: RegExp}[]>} lists - ban lists by file class.
 * @throws {Error} naming the first entry that carries the flag.
 */
export function assertNoGlobalPatterns(lists) {
  for (const [label, list] of Object.entries(lists))
    for (const entry of list)
      if (entry.re.global)
        throw new Error(
          `${label} ban entry "${entry.name}" carries the global flag. ` +
            "Ban patterns are applied to many subjects and must hold no match state.",
        );
}

assertNoGlobalPatterns(BAN_LISTS);

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
// product/standard name, a versioned code symbol, or a structural label, none of which is a
// Shadowcat-process-assigned id. "Really carries" is ENFORCED rather than asserted: every entry is
// hit-counted on each full-corpus run and one that reaches nothing fails the gate, so an entry
// cannot outlive the sites that justified it. The corpus is the tracked skill directories plus the
// code roots, so a token living only in vendored third-party prose justifies no entry here.
const ACKNOWLEDGED = [
  {
    name: "product, protocol or algorithm name carrying a version-like number",
    re: /\bNeo4j\b|\bIPv4\b|\bIpv4\b|\bIPv6\b|\bNAT64\b|\bFTS5\b|\bI18n\b|\bPowerShell\s?\d+(?:\.\d+)?\b|\bUUIDv5\b|\bRFC\s?\d+\b|\bArgon2\b|\bSplitMix32\b|\bSplitMix64\b|\bSvelte\s?\d+\b|\bHTTP\s?\d+\b|\bPolyanya\s?\d+\b|\bWin32\b|\bJSON1\b|\bGIF89a\b|\bBM25\b|\bPF1e\b|\bTS2322\b/,
  },
  {
    name: "a code symbol cited as a value, not a process id",
    re: /\bPanelLayoutV1\b|\bVec2\b/,
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
  // The general ACKNOWLEDGED list already names this shape once for a different checker class
  // ("a code symbol cited as a value, not a process id" — PanelLayoutV1/Vec2): a symbol quoted in
  // backticks is program vocabulary, not prose about the code's own history. This class needs its
  // own entry rather than reuse because the two lists key on different windows — ACKNOWLEDGED tests
  // the bare token (contextChars: 0), while this class must see past the token to the backtick that
  // follows it (contextChars: 8).
  //
  // CASE-SENSITIVE and requiring the backtick to sit IMMEDIATELY after the matched word, deliberately
  // narrower than "any backtick-adjacent spelling": this codebase's enum variants are single
  // PascalCase words (`Replaced`, `Deleted`, `Renamed`), so a capital letter followed by lowercase
  // letters and a backtick is the real shape a wire-protocol/code citation takes. Lowercase prose
  // narration ("this endpoint was replaced`" for some unrelated reason) must NOT be exempted just
  // because a backtick happens to follow it — the capitalization requirement is what keeps the
  // exemption to the code-symbol form and nothing wider. A lowercase word cited in backticks as a
  // literal string value (`` `replaced` ``) is likewise NOT this shape: it carries no PascalCase
  // signal distinguishing it from quoted prose, so it stays residue rather than being acknowledged.
  {
    name: "an enum-variant name (PascalCase) cited inline in backticks as a wire-protocol/code value, not narration of the code's own past",
    re: /^[A-Z][a-z]+`/,
  },
];

/**
 * The acknowledgement entries no candidate in the corpus reached, by name.
 *
 * An entry that absorbs nothing is a standing invitation to absorb a future defect: it stays on
 * the list, and the day a real candidate happens to spell it, the coverage control reports that
 * candidate as a known-legitimate token instead of surfacing it for review. Nothing in any output
 * moves. This is the same rule `check-skill-symbol-refs.mjs` applies to its own acknowledgement
 * lists, stated once per gate because the two lists are separate data.
 *
 * The measurement is only valid over the WHOLE corpus. On a subset a zero says the scope did not
 * reach the token, not that the entry is dead, so the caller must not run this against a scoped
 * tally.
 *
 * @param {Map<string, number>} ackByReason - hits per entry name, from `scanCandidates`.
 * @returns {string[]} every entry name with no hit, in list order.
 */
export function unusedAcknowledgements(ackByReason) {
  return [...ACKNOWLEDGED, ...ACKNOWLEDGED_NARRATION]
    .map((a) => a.name)
    .filter((name) => !ackByReason.has(name));
}

/**
 * Extracts the comment/prose text one line contributes, mirroring exactly what `scanContent`
 * checks against BANNED/SKILL_BANNED. Shared so the coverage control (`scanCandidates`) can never
 * see a different subject than the gate does: a second derivation of "what counts" drifts from the
 * first, and the drift is invisible because both still report a number.
 *
 * `kind` says what the line contributed, which is what decides whether the line may be JOINED to
 * its neighbours: `"comment"` for a line that is nothing but commentary, `"literal"` for a code
 * line whose only prose is a string the program carries, `"trailing"` for a code line carrying
 * commentary beside code, `"prose"` for a Markdown line.
 *
 * `"trailing"` is separated from `"comment"` for the same reason `"literal"` is. A trailing
 * comment is written against the statement on its own line, not as a continuation of the block
 * above it, so joining it to that block splices a doc comment's last word onto a trailing
 * comment's first and manufactures a phrase nobody wrote - the false-positive direction the group
 * boundary exists to prevent.
 */
function lineSubject(line, isMd, lexState) {
  if (isMd) {
    return {
      subject: line.trim().replace(DESIGN_DOC_CITATION, ""),
      state: lexState,
      kind: "prose",
    };
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
  const standalone = split.comment.trim() === "" ? "literal" : "trailing";
  return {
    subject,
    state: split.state,
    kind: split.code.trim() === "" ? "comment" : standalone,
  };
}

// The separator joining two WORDS of a marker phrase is a spelling, never part of the marker's
// identity: a hyphen, an underscore and a space all name the same process artifact, so a pattern
// carrying one spelling reads the other two as clean.
//
// The widening is derived from each pattern's own source rather than written into the patterns one
// spelling at a time. An enumerated list can only ever carry the spellings someone remembered, so
// an entry written with one separator leaves the other two unreachable for that entry alone.
// Deriving covers every entry at once, including entries not yet written.
//
// It rewrites the PATTERN and never the subject. Rewriting the subject looks equivalent and is not:
// `_` is a word character while `-` and a space are not, so any subject-side substitution retokenizes
// the text and exposes token boundaries the patterns rely on — a constant like `SCHEMA_FORMAT_V1`
// splits into three tokens and its version suffix reads as a local marker, and a quantity like
// "A 1-hex token" fuses into one and reads the same way. Both were measured, in bulk.
//
// Two spellings of a separator are widened, and they differ in where the disambiguation comes from:
//   - a BARE separator character, only between two LITERAL ALPHABETIC characters. The pattern
//     source offers nothing else to tell a word separator from regex punctuation, so the
//     neighbours are the whole evidence.
//   - a CHARACTER CLASS whose every member is a separator (`[ -]`), with no neighbour requirement:
//     such a class cannot be anything but a separator, so it needs no disambiguating context. A
//     class carrying any non-separator member is emitted untouched, because a `-` inside `[a-z]`
//     is a RANGE and widening it corrupts the pattern outright.
// RESIDUAL, and why that coverage claim is bounded rather than universal: the neighbour test
// requires a LITERAL ALPHABETIC character on both sides, so a bare separator keeps its single
// spelling whenever either neighbour is anything else — a group boundary (`(?:task|dispatch) brief`),
// an escape (`finding \d+`), a class, a quantifier or a punctuation mark. Naming only the escape
// case would state a bound narrower than the code's and read as better coverage than exists; the
// group boundary is the commoner of the two, because an alternation of writers is how a marker with
// several spellings gets written in the first place.
//
// The fix at a SITE is to respell that separator as a ONE-MEMBER CHARACTER CLASS — `[ ]`, `[-]`,
// `[\s]` — which routes it through the class path above with no new mechanism and no new residual.
// The source still declares exactly one spelling; the class is what marks it as a word separator
// rather than regex punctuation, supplying the evidence the neighbours could not. Loosening the
// neighbour test instead is the wrong repair: it is correct for every entry whose separator is not
// a word separator at all, and relaxing it is how a matcher acquires silent over-widening.
//
// Widening is not safe in every direction, so a separator is respelled only where its ADDED
// writers are not ordinary prose. Adding a hyphen or an underscore between two words is safe —
// nobody writes English that way by accident. Adding a SPACE where the pattern had a hyphen is not:
// see `local letter+digit marker`, where it fuses an article and a quantity into a marker.
//
// Nothing inside a NEGATIVE lookaround is widened, at any nesting depth. Widening there widens an
// EXCLUSION, so the pattern matches strictly less and the gate reports strictly cleaner — the
// silent direction, and the inverse of what every other widening here does.
const SEPARATOR_CLASS = "[-_\\s]";
const SEPARATOR_CHARS = new Set(["-", "_", " "]);
// A negative lookaround opener, matched against the pattern source from its `(`. A named group and
// a POSITIVE lookaround are ordinary: widening inside them adds matches, which is this gate's
// deliberate failure direction.
const NEGATIVE_LOOKAROUND = /^\(\?(?:!|<!)/;

// The single-character escapes that denote whitespace, so a class member written as one is
// recognised as the separator it is. Derived alongside `SEPARATOR_CHARS` rather than restating
// which characters separate words: `SEPARATOR_CLASS` is what a widened class becomes, and a member
// it already matches must not read as non-separator on the way in.
const WHITESPACE_ESCAPES = new Set(["s", "t", "n", "r", "f", "v"]);

/**
 * Whether a character-class body denotes separators and nothing else, so the whole class can be
 * replaced by `SEPARATOR_CLASS`.
 *
 * A member counts when it is a separator character written as a LITERAL or as a SINGLE-CHARACTER
 * ESCAPE — `[ -]`, `[\s]`, `[\-_]`, `[\t]` all qualify, and so does a one-member literal
 * space class (`[ ]`), which is one of the sanctioned respellings a site uses to mark a separator
 * as a word separator rather than regex punctuation. A separator spelled as a NUMERIC escape
 * (`[\x20]`, `[\u0020]`) is rejected: that spelling never appears in a hand-written word
 * separator, and rejecting leaves the class unwidened, which is the visible failure direction.
 *
 * A `-` counts as a member only at the body's first or last position; anywhere else it is a RANGE
 * operator, and a range is rejected outright rather than read through — `[ -_]` spans every
 * character from space to underscore. An ESCAPED `-` is never a range operator, so it carries no
 * position requirement.
 *
 * @param {string} body - the class source between `[` and `]`.
 * @returns {boolean} whether every member of the class is a separator.
 */
export function separatorOnlyClass(body) {
  if (body === "" || body.startsWith("^")) return false;
  for (let i = 0; i < body.length; i += 1) {
    const c = body[i];
    if (c === "\\") {
      const escaped = body[i + 1];
      if (escaped === undefined) return false;
      if (!WHITESPACE_ESCAPES.has(escaped) && !SEPARATOR_CHARS.has(escaped)) return false;
      i += 1;
      continue;
    }
    if (c === "-") {
      if (i !== 0 && i !== body.length - 1) return false;
      continue;
    }
    if (!SEPARATOR_CHARS.has(c)) return false;
  }
  return true;
}

/**
 * The separator-flexible form of one ban pattern, or null when it carries no widenable separator.
 * @param {RegExp} re - one ban pattern.
 * @returns {RegExp|null} the widened pattern, or null when the source is unchanged.
 */
export function separatorFlexible(re) {
  const src = re.source;
  const isAlpha = (c) => c !== undefined && /[A-Za-z]/.test(c);
  let out = "";
  // One entry per open group, each carrying whether it sits inside a negative lookaround; a nested
  // group inherits its parent's answer, so the top of the stack is the whole test.
  const groups = [];
  const excluded = () => groups.length > 0 && groups[groups.length - 1];
  // The last character emitted as a plain literal; reset by anything that is not one, so an escape
  // sequence's trailing letter (`\s`, `\w`) is never read as the word a separator follows.
  let prevLiteral = "";
  for (let i = 0; i < src.length; i += 1) {
    const c = src[i];
    if (c === "\\") {
      out += src.slice(i, i + 2);
      i += 1;
      prevLiteral = "";
      continue;
    }
    if (c === "(") {
      groups.push(excluded() || NEGATIVE_LOOKAROUND.test(src.slice(i)));
      out += c;
      prevLiteral = "";
      continue;
    }
    if (c === ")") {
      groups.pop();
      out += c;
      prevLiteral = "";
      continue;
    }
    if (c === "[") {
      let end = i + 1;
      while (end < src.length && src[end] !== "]") end += src[end] === "\\" ? 2 : 1;
      const body = src.slice(i + 1, end);
      out += !excluded() && separatorOnlyClass(body) ? SEPARATOR_CLASS : src.slice(i, end + 1);
      i = end;
      prevLiteral = "";
      continue;
    }
    if (!excluded() && SEPARATOR_CHARS.has(c) && isAlpha(prevLiteral) && isAlpha(src[i + 1])) {
      out += SEPARATOR_CLASS;
      prevLiteral = "";
      continue;
    }
    out += c;
    prevLiteral = c;
  }
  return out === src ? null : new RegExp(out, re.flags);
}

// Derived once per pattern object: the ban lists share entries by reference, so a per-call
// derivation would rebuild the same regex for every line of every file.
const flexibleCache = new Map();
const flexibleFor = (re) => {
  if (!flexibleCache.has(re)) flexibleCache.set(re, separatorFlexible(re));
  return flexibleCache.get(re);
};

// A global clone of a ban pattern, for iterating every match in one subject. Cloned rather than
// mutated, and cached per source pattern: a ban entry is shared by reference between the two ban
// lists and applied to every subject in the corpus, so flipping its own flag would leave match
// state on a value the whole scan reads.
const globalCache = new Map();
const globalFor = (re) => {
  if (!globalCache.has(re)) globalCache.set(re, new RegExp(re.source, `${re.flags}g`));
  return globalCache.get(re);
};

/**
 * EVERY place `re` matches `subject`, under its own spelling and under its separator-flexible
 * form, ordered by offset and carrying WHAT each match consumed.
 *
 * Every match is returned, not just the first, because a subject carrying two instances of the
 * same pattern would otherwise be fixable only one gate run at a time: the run reporting the
 * second instance looks like a regression introduced by the fix for the first. The two spellings
 * are merged and de-duplicated by offset rather than short-circuiting on the direct form, since a
 * subject can carry one writer of a marker at one offset and another writer at the next.
 *
 * The matched text travels with each offset because a subject is a group and a phrase may wrap:
 * the line an offset lands on is the line the phrase STARTS on, whose text does not contain the
 * phrase. A finding naming a line that does not show the violation is materially harder to act on
 * than one carrying the matched words.
 *
 * @param {RegExp} re - one ban pattern.
 * @param {string} subject - a grouped comment block or Markdown paragraph.
 * @returns {{index: number, text: string}[]} every offset into `subject` and the text it matched.
 */
export function bannedMatchesIn(re, subject) {
  const byIndex = new Map();
  const flexible = flexibleFor(re);
  for (const spelling of flexible === null ? [re] : [re, flexible])
    for (const m of subject.matchAll(globalFor(spelling))) {
      // A wider spelling of the same marker at the same offset is one violation, not two; the
      // longer text is the more informative report of it.
      const prior = byIndex.get(m.index);
      if (prior === undefined || m[0].length > prior.length) byIndex.set(m.index, m[0]);
    }
  return [...byIndex]
    .sort((a, b) => a[0] - b[0])
    .map(([index, text]) => ({ index, text }));
}

// A comment block, or a Markdown paragraph, is the unit a sentence is written in; a LINE is only
// where the text happened to wrap. A line-scoped subject therefore reads every multi-word ban
// pattern as clean the moment ordinary prose wrapping puts a line break at one of its spaces, and
// the two halves are individually innocent — nothing in the output says a phrase was split.
//
// The GROUP BOUNDARY is what makes joining safe, and it is the whole design. A group ends at a
// blank line, at any line contributing no prose, and — in code — at any line that is not commentary,
// so a doc comment is never joined to the declaration beneath it. Joining across that boundary
// manufactures phrases nobody wrote out of a comment's last words and a signature's first, turning
// a false negative into a false positive.
/**
 * Groups a file's lines into the units a sentence is written in, joining each group's per-line
 * subjects with the single space a wrap replaced.
 *
 * Each group carries `lineAt`, mapping any offset in the joined text back to the source line that
 * contributed it, so a hit inside a wrapped phrase still names the line the phrase starts on.
 *
 * @param {string} content - one file's raw contents.
 * @param {boolean} isMd - true for skill prose, false for source code.
 * @returns {{groups: {text: string, lineAt: (index: number) => {line: number, source: string}}[],
 *   exempted: number}} the joined groups, and how many lines an EXAMPLE marker covered.
 */
export function subjectGroups(content, isMd) {
  const groups = [];
  let parts = [];
  let exempted = 0;
  const flush = () => {
    if (parts.length === 0) return;
    let text = "";
    const starts = [];
    for (const part of parts) {
      if (text !== "") text += " ";
      starts.push({ start: text.length, line: part.line, source: part.source });
      text += part.subject;
    }
    const lineAt = (index) => {
      let found = starts[0];
      for (const s of starts) if (s.start <= index) found = s;
      return found;
    };
    groups.push({ text, lineAt });
    parts = [];
  };
  let lexState = { inBlock: false, inHtml: false };
  content.split("\n").forEach((line, i) => {
    if (EXAMPLE_EXEMPT.test(line)) {
      exempted += 1;
      flush();
      return;
    }
    const { subject, state, kind } = lineSubject(line, isMd, lexState);
    lexState = state;
    if (subject === "") {
      flush();
      return;
    }
    const part = { line: i + 1, subject, source: line.trim() };
    // A code line's prose - a string LITERAL the program carries, or a comment trailing the
    // statement - is written against that one line. It neither extends a comment block nor is
    // extended by one, so it stands as its own group.
    if (kind === "literal" || kind === "trailing") {
      flush();
      parts.push(part);
      flush();
      return;
    }
    parts.push(part);
  });
  flush();
  return { groups, exempted };
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
  const acknowledged = [];
  const residue = [];
  const { groups, exempted } = subjectGroups(content, isMd);
  for (const group of groups) {
    const subject = group.text;
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
        if (banned.some((b) => bannedMatchesIn(b.re, token).length > 0)) continue;
        const context = subject.slice(m.index, m.index + token.length + contextChars);
        const ack = acks.find((a) => a.re.test(context));
        const at = group.lineAt(m.index);
        if (ack) {
          acknowledged.push({ line: at.line, token, reason: ack.name });
          continue;
        }
        residue.push({ line: at.line, token, text: at.source });
      }
    }
  }
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

/**
 * Scans one file's already-read text for banned references, returning its hits (line + kind + the
 * matched text + the trimmed source line) and how many lines its EXAMPLE-exempt marker covered.
 *
 * `isMd` selects the subject-extraction mode: a `.md` skill has no code/comment boundary — the
 * whole line is prose — so it is checked directly against SKILL_BANNED, while a code file keeps
 * the comment/string-literal split against the full BANNED list. Pure function of its argument, so
 * a test exercises it on fabricated text without touching the filesystem.
 */
export function scanContent(content, { isMd }) {
  const banned = bannedFor(isMd);
  const hits = [];
  // Comment text is checked whole; the code span is checked only inside its string literals, so
  // identifiers and paths that are part of the program are never flagged. Of those literals, prose
  // ones always count and token-shaped ones count only in an explanatory context. The subject is a
  // GROUP — see `subjectGroups` — so a phrase split by a line wrap is one subject, not two.
  const { groups, exempted } = subjectGroups(content, isMd);
  for (const group of groups) {
    // EVERY violation a group carries is reported: every pattern that matches it, and every place
    // each pattern matches. Reporting one leaves a block carrying two violations fixable only one
    // gate run at a time, and the run that reports the second looks like a regression introduced by
    // the fix for the first — which holds whether the two are different patterns or two instances
    // of the same one.
    for (const b of banned)
      for (const m of bannedMatchesIn(b.re, group.text)) {
        const at = group.lineAt(m.index);
        // A guarded entry is qualified by the line the match sits on, not disabled: see
        // `namesDurableDesignDoc` for why the raw line is the unit and the subject is not.
        if (b.skipLine?.(at.source)) continue;
        hits.push({ line: at.line, kind: b.name, text: at.source, match: m.text });
      }
  }
  hits.sort((a, b) => a.line - b.line);
  return { hits, exempted };
}

/**
 * The `--residue` coverage control's full computation, over the SAME file set `gateFileSet`
 * returns for `scopes` — not a second derivation of it. Returns the acknowledged tally (grouped
 * by reason) and the residue list, plus how many files were actually read, so a caller (or a
 * test) can compare that count against the gate's own `scanned.length` for the identical scopes
 * and prove the two can never silently diverge.
 */
export function residueReport(scopes = [], { skillsRoot = defaultSkillsRoot() } = {}) {
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
  // Scoped runs get an empty list rather than a wrong one: see `unusedAcknowledgements` for why a
  // zero on a subset cannot falsify an entry. The same reasoning applies when the skill checkout
  // that supplies every markdown file in the corpus is simply absent (CI, or a machine that never
  // cloned the plugin repo): mdFiles is then always empty regardless of which entries are truly
  // reached, so a zero there says the checkout is missing, not that an entry's sites are gone. An
  // environment WITH the checkout still holds every entry — skill or code — accountable in full.
  const hasSkillsCorpus = existsSync(skillsRoot);
  const unused =
    scopes.length > 0 || !hasSkillsCorpus ? [] : unusedAcknowledgements(ackByReason);
  return { ackTotal, ackByReason, unused, residue, filesScanned: scanned.length };
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

// Every function whose SOURCE decides what gets counted. The membership rule is the whole design:
// a function belongs here when changing it alone moves a total without touching any `re`. Every
// one does — where a line's prose ends, where a group boundary falls, how many hits one group yields, how
// many spellings a pattern reaches, and which files a scope claims.
//
// Held as function VALUES, not names, so each name can be read off the function itself. A
// hand-written label drifts silently when the function it labels is renamed; a name read from the
// value cannot.
const INSTRUMENT_FUNCTIONS = [
  splitLine,
  lineSubject,
  subjectGroups,
  scanContent,
  scanCandidates,
  bannedMatchesIn,
  separatorFlexible,
  separatorOnlyClass,
  under,
];

// One record per hashed component: the group it is reported under, its name, and how it enters the
// hash. `instrumentComponents` reads the NAMES off this list and `instrumentFingerprint` reads the
// hashed values off the SAME entries, so the reported component list and the hash input cannot name
// different sets. Enumerating them twice is what let a name be dropped from the hash while both the
// report and its test stayed green — the fingerprint then holds still exactly when the rule it
// governs changes, which is the failure the fingerprint exists to make impossible.
const banList = (name, list) => ({
  group: "patterns",
  name,
  hashed: () => list.map((b) => [b.name, b.re.source, b.re.flags]),
});
const patternSource = (name, re) => ({ group: "patterns", name, hashed: () => re.source });
// A value enters the hash through a thunk because each carries its own normalization: a Set has no
// JSON order of its own, so it is sorted on the way in or the hash moves without the rules moving.
const valueOf = (name, hashed) => ({ group: "values", name, hashed });

const INSTRUMENT_COMPONENTS = [
  ...INSTRUMENT_FUNCTIONS.map((fn) => ({
    group: "functions",
    name: fn.name,
    hashed: () => stableSource(fn),
  })),
  banList("BANNED", BANNED),
  banList("SKILL_BANNED", SKILL_BANNED),
  patternSource("STRING_LITERAL", STRING_LITERAL),
  patternSource("EXPLANATORY_STRING", EXPLANATORY_STRING),
  patternSource("PROSE_LITERAL", PROSE_LITERAL),
  patternSource("EXAMPLE_EXEMPT", EXAMPLE_EXEMPT),
  patternSource("DESIGN_DOC_CITATION", DESIGN_DOC_CITATION),
  // The coverage control's own matchers, hashed for the same reason the ban lists are: they decide
  // a printed count (the reached-entry tally) and a gate outcome (a zero-hit entry), so a change to
  // them makes two runs incomparable.
  patternSource("CANDIDATE_TOKEN_LABEL", CANDIDATE_TOKEN_LABEL),
  patternSource("CANDIDATE_TOKEN_WORD", CANDIDATE_TOKEN_WORD),
  patternSource("PRE_POST_NARRATION_TOKEN", PRE_POST_NARRATION_TOKEN),
  patternSource("WORD_NARRATION_TOKEN", WORD_NARRATION_TOKEN),
  // The separator value sets sit beside the functions that read them: `separatorOnlyClass`'s source
  // is unchanged by adding a character to either set, while every count that turns on which
  // spellings a pattern reaches moves.
  valueOf("SEPARATOR_CLASS", () => SEPARATOR_CLASS),
  valueOf("SEPARATOR_CHARS", () => [...SEPARATOR_CHARS].sort()),
  valueOf("WHITESPACE_ESCAPES", () => [...WHITESPACE_ESCAPES].sort()),
  valueOf("ROOTS", () => ROOTS),
  valueOf("EXTS", () => EXTS),
  valueOf("MD_EXTS", () => MD_EXTS),
  valueOf("SKIP_DIRS", () => [...SKIP_DIRS].sort()),
  valueOf("GENERATED_ROOT", () => GENERATED_ROOT),
  valueOf("ACKNOWLEDGED", () => ACKNOWLEDGED.map((a) => [a.name, a.re.source, a.re.flags])),
  valueOf("ACKNOWLEDGED_NARRATION", () =>
    ACKNOWLEDGED_NARRATION.map((a) => [a.name, a.re.source, a.re.flags])),
];

/**
 * The named parts of the ruler, exactly as `instrumentFingerprint` hashes them.
 *
 * Exported so a test can pin WHICH parts are hashed rather than only that the hash is stable. The
 * omission this guards against is invisible from the fingerprint itself: a component left out
 * produces a perfectly stable hash that fails to change when the rule it governs does, and the
 * banner then offers a comparison between two runs measured by different rulers.
 *
 * @returns {{functions: string[], patterns: string[], values: string[]}} each hashed component's
 *   name, grouped by what kind of thing it is.
 */
export function instrumentComponents() {
  const named = (group) =>
    INSTRUMENT_COMPONENTS.filter((c) => c.group === group).map((c) => c.name);
  return { functions: named("functions"), patterns: named("patterns"), values: named("values") };
}

/**
 * The exact array `instrumentFingerprint` hashes, each entry paired with the component name it
 * came from.
 *
 * Exported so a test can assert the hash input and the reported component list enumerate the same
 * names in the same order. That identity holds by construction today; the test is what fails if a
 * second hand-written enumeration is reintroduced into either one.
 *
 * @returns {Array<[string, unknown]>} one `[name, hashed value]` pair per component.
 */
export function instrumentHashInput() {
  return INSTRUMENT_COMPONENTS.map((c) => [c.name, c.hashed()]);
}

/**
 * The short hash stamped beside every count this script prints.
 *
 * @returns {string} an 8-character fingerprint of the ban lists, the subject rules and the scope
 *   rules currently in force.
 */
export function instrumentFingerprint() {
  return createHash("sha256")
    .update(JSON.stringify(instrumentHashInput()))
    .digest("hex")
    .slice(0, 8);
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
    const { ackTotal, ackByReason, unused, residue, filesScanned } = residueReport(scopes);
    console.log(`${filesScanned} file(s) scanned (code + skill corpora).`);
    console.log(`${ackTotal} acknowledged candidate(s):`);
    for (const [reason, n] of [...ackByReason].sort((a, b) => b[1] - a[1]))
      console.log(`  ${String(n).padStart(4)}  ${reason}`);
    console.log("");
    // BOTH failure classes are collected. Reporting only the residue would let one unrecognised
    // token hide every dead entry behind it, so the two could be fixed only one run apart.
    if (unused.length > 0) {
      console.error(`${unused.length} acknowledgement entry(ies) matched nothing:`);
      for (const name of unused) console.error(`  ${name}`);
      console.error(
        "\nDelete each one. An entry the corpus never reaches cannot be justified by the corpus, " +
          "and it silently absorbs the first future candidate that happens to spell it.\n",
      );
    }
    if (residue.length === 0 && unused.length === 0) {
      console.log("0 unrecognised candidate(s). Coverage control: clean.");
      process.exit(0);
    }
    if (residue.length > 0) {
      console.error(`${residue.length} unrecognised candidate(s):`);
      for (const r of residue)
        console.error(`  ${r.path}:${r.line}  ${JSON.stringify(r.token)}  ${r.text}`);
      console.error(
        "\nEach must become a BANNED/SKILL_BANNED pattern (genuine miss) or a named, reasoned " +
          "ACKNOWLEDGED entry (legitimate token) — never silently ignored.",
      );
    }
    process.exit(1);
  }

  const { scanned, isMdFile, generatedExcluded, untrackedSkillExcluded } = gateFileSet(scopes);
  const hits = [];
  let exempted = 0;
  // The coverage control's acknowledgement entries are hit-counted on the CI-wired run, not only
  // in `--residue`, because that is the run whose result anyone acts on. An entry that reaches
  // nothing is an exemption nobody counts, which is a backdoor by the same reasoning the EXAMPLE
  // marker's printed count answers. Counted off the content already in hand rather than by a
  // second pass over the tree.
  const ackByReason = new Map();
  for (const path of scanned) {
    const content = readFileSync(path, "utf8");
    const isMd = isMdFile.has(path);
    const result = scanContent(content, { isMd });
    exempted += result.exempted;
    for (const h of result.hits) hits.push({ path, ...h });
    if (scopes.length === 0)
      for (const a of scanCandidates(content, { isMd }).acknowledged)
        ackByReason.set(a.reason, (ackByReason.get(a.reason) ?? 0) + 1);
  }
  // Empty on a scoped run, where a zero hit says the scope missed the token rather than that the
  // entry is dead — see `unusedAcknowledgements`. Empty too when the skill checkout supplying
  // every markdown file in the corpus is simply absent (CI, or a machine that never cloned the
  // plugin repo): mdFiles is then always empty regardless of which entries a WITH-checkout run
  // would have reached, so a zero there says the checkout is missing, not that an entry's sites
  // are gone — see `residueReport`'s identical reasoning for its own `unused` field.
  const hasSkillsCorpus = existsSync(defaultSkillsRoot());
  const deadAcknowledgements =
    scopes.length > 0 || !hasSkillsCorpus ? [] : unusedAcknowledgements(ackByReason);

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

  const INSTRUMENT = instrumentFingerprint();

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

  /**
   * Names every acknowledgement entry the corpus never reached, and says whether there were any.
   * Returns rather than exits, so a run carrying both this and real hits reports both — one
   * failure class hiding another behind it is what makes them fixable only one run at a time.
   */
  function reportDeadAcknowledgements() {
    if (deadAcknowledgements.length === 0) return false;
    console.error(
      `\n${deadAcknowledgements.length} acknowledgement entry(ies) matched nothing in the corpus:`,
    );
    for (const name of deadAcknowledgements) console.error(`  ${name}`);
    console.error(
      "\nDelete each one. An entry the corpus never reaches cannot be justified by the corpus, " +
        "and it silently absorbs the first future candidate that happens to spell it.",
    );
    return true;
  }

  /** The line that makes a count self-describing. Print it beside every total. */
  function provenance(total) {
    const prior = priorRun();
    const ex = exempted > 0 ? `; ${exempted} line(s) EXAMPLE-exempt` : "";
    const gen =
      generatedExcluded > 0
        ? `; ${generatedExcluded} generated file(s) excluded (${GENERATED_ROOT})`
        : "";
    const vendored =
      untrackedSkillExcluded > 0
        ? `; ${untrackedSkillExcluded} untracked (vendored) skill file(s) excluded`
        : "";
    // The acknowledgement list's live size prints on every full-corpus run, for the same reason
    // the EXAMPLE count does: an exemption whose size nobody sees is indistinguishable from a rule
    // that does not apply. A scoped run measures a subset and prints nothing rather than a figure
    // that would read as the whole list's.
    const acks =
      scopes.length > 0 ? "" : `; ${ackByReason.size} acknowledgement entry(ies) reached`;
    const head = `instrument ${INSTRUMENT}; ${scanned.length} file(s) scanned${ex}${gen}${vendored}${acks}`;
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
          deadAcknowledgements,
          hits,
        },
        null,
        2,
      ),
    );
    recordRun(hits.length);
    process.exit(hits.length > 0 || deadAcknowledgements.length > 0 ? 1 : 0);
  }

  if (wantArea) {
    const scopeNote = scopes.length > 0 ? ` under ${scopes.join(", ")}` : "";
    console.log(`${hits.length} site(s) in ${files} file(s)${scopeNote}`);
    console.log(provenance(hits.length));
    for (const [area, n] of byArea)
      console.log(`${String(n).padStart(5)}  ${area}`);
    const dead = reportDeadAcknowledgements();
    recordRun(hits.length);
    process.exit(hits.length > 0 || dead ? 1 : 0);
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
    // The matched text prints beside the source line because a subject is a GROUP: a wrapped
    // phrase is attributed to the line it starts on, and that line does not contain the phrase.
    for (const h of hits)
      console.error(`  ${h.path}:${h.line}  [${h.kind}]  matched "${h.match}" in: ${h.text}`);
    reportDeadAcknowledgements();
    recordRun(hits.length);
    process.exit(1);
  }

  if (reportDeadAcknowledgements()) {
    console.error(provenance(0));
    recordRun(0);
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
