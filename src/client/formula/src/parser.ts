import { tokenize, type Tok } from "./lexer";
import { type FormulaError, MAX_AST_NODES, MAX_PARSE_DEPTH } from "./types";

/** Names accepted where a word is immediately followed by `(` — anything else
 * there is a parse error (spec: the library reserves no OTHER identifiers). */
const FN_NAMES = new Set(["min", "max", "floor", "ceil", "round"]);
type FnName = "min" | "max" | "floor" | "ceil" | "round";

export type Expr =
  | { kind: "num"; value: number }
  | { kind: "ref"; path: string[] } // opaque dotted path — the library assigns no meaning
  | { kind: "neg"; operand: Expr }
  | { kind: "bin"; op: "+" | "-" | "*" | "/" | "%"; left: Expr; right: Expr }
  | { kind: "call"; fn: FnName; args: Expr[] };

/** Recursive-descent parser over `tokenize`'s output.
 * Grammar: additive := multiplicative (('+'|'-') multiplicative)* ;
 *          multiplicative := unary (('*'|'/'|'%') unary)* ;
 *          unary := '-' unary | primary ;
 *          primary := num | '(' additive ')' | word ('(' args ')')? ('.' word)* .
 * A word immediately followed by '(' must be a known function name; otherwise
 * the word begins a dotted ref path (a dotted segment is never a call). */
class Parser {
  private pos = 0;
  private nodeCount = 0;

  /**
   * Wraps a token stream for recursive-descent parsing, starting at index 0.
   * @param toks The full token stream from `tokenize` — consumed by index, never mutated.
   * @param srcLen Original source length, used only to name a position past the last token
   * (an "expected X" error at end-of-input reports this rather than an undefined position).
   * @example
   * ```
   * // not part of the public `@shadowcat/formula` surface — Parser is not exported;
   * // reachable only through `parseFormula`.
   * new Parser(tokens, src.length);
   * ```
   */
  constructor(
    private readonly toks: Tok[],
    private readonly srcLen: number,
  ) {}

  /**
   * Looks at the next unconsumed token without advancing `pos`.
   * @returns The next token, or `undefined` at end of input.
   * @example
   * ```
   * // not part of the public `@shadowcat/formula` surface — Parser is not exported.
   * this.peek(); // { kind: "op", value: "+", pos: 3 } | undefined
   * ```
   */
  private peek(): Tok | undefined {
    return this.toks[this.pos];
  }

  /**
   * True when the next unconsumed token is the operator `value`. Never advances `pos`.
   * @param value An operator spelling, e.g. `"+"` or `"("`.
   * @returns `true` when `peek()` is an `op` token equal to `value`.
   * @example
   * ```
   * // not part of the public `@shadowcat/formula` surface — Parser is not exported.
   * this.atOp("("); // true only if the next token is a literal '(' op token
   * ```
   */
  private atOp(value: string): boolean {
    const t = this.peek();
    return t !== undefined && t.kind === "op" && t.value === value;
  }

  /**
   * Wraps a freshly-built `Expr` node, charging it against `MAX_AST_NODES`.
   * Every constructed node (regardless of kind) passes through here exactly
   * once, so `nodeCount` is a true total-node count, not per-kind.
   * @param e The node being constructed.
   * @returns `e` unchanged, or a `"cap"` error once the 256-node budget is exceeded.
   * @example
   * ```
   * // not part of the public `@shadowcat/formula` surface — Parser is not exported.
   * this.node({ kind: "num", value: 1 });
   * ```
   */
  private node(e: Expr): Expr | FormulaError {
    this.nodeCount++;
    if (this.nodeCount > MAX_AST_NODES) {
      return { error: "cap", detail: "formula exceeds 256 AST nodes" };
    }
    return e;
  }

  /**
   * Parses the whole token stream as one expression, then requires end of input —
   * a formula with unconsumed trailing tokens (e.g. `"1 2"`) is a parse error here,
   * not silently truncated to its first valid prefix.
   * @returns The parsed `Expr`, or a `FormulaError` from the grammar or a trailing-input check.
   * @example
   * ```
   * // not part of the public `@shadowcat/formula` surface — Parser is not exported;
   * // reachable only through `parseFormula`.
   * new Parser(tokens, src.length).parseTop();
   * ```
   */
  parseTop(): Expr | FormulaError {
    const e = this.additive(0);
    if (isErr(e)) return e;
    if (this.pos < this.toks.length) {
      const t = this.toks[this.pos];
      return { error: "parse", detail: `unexpected trailing input at position ${t.pos}` };
    }
    return e;
  }

  // INVARIANT: `depth` counts true structural-nesting boundaries only —
  // paren-open, call-argument, and unary-minus descents — so 32 documented
  // levels of nesting are genuinely available for each construct uniformly.
  // The flat additive/multiplicative/unary/primary production chain passes
  // `depth` through UNCHANGED; it never itself recurses unboundedly, so no
  // check is needed at those non-structural hops.
  /**
   * Rejects a structural-nesting descent past `MAX_PARSE_DEPTH`. Called only
   * at the three structural boundaries (paren-open, call-argument,
   * unary-minus) — see the INVARIANT comment above.
   * @param depth The new depth after this descent (already incremented by the caller).
   * @returns `undefined` when within budget, else a `"cap"` error.
   * @example
   * ```
   * // not part of the public `@shadowcat/formula` surface — Parser is not exported.
   * this.checkDepth(33); // { error: "cap", detail: "..." }
   * ```
   */
  private checkDepth(depth: number): FormulaError | undefined {
    if (depth > MAX_PARSE_DEPTH) {
      return { error: "cap", detail: "formula exceeds max nesting depth of 32" };
    }
    return undefined;
  }

  /**
   * `additive := multiplicative (('+'|'-') multiplicative)*` — left-associative,
   * lowest precedence in the grammar.
   * @param depth Current structural-nesting depth, passed through unchanged
   * (this production is not itself a structural boundary).
   * @returns The parsed left-associative `+`/`-` chain, or a `FormulaError`.
   * @example
   * ```
   * // not part of the public `@shadowcat/formula` surface — Parser is not exported;
   * // reachable only through `parseTop`/`parseFormula`.
   * this.additive(0); // parses "1 + 2 - 3" as ((1 + 2) - 3)
   * ```
   */
  private additive(depth: number): Expr | FormulaError {
    let left = this.multiplicative(depth);
    if (isErr(left)) return left;
    for (;;) {
      if (this.atOp("+") || this.atOp("-")) {
        const op = (this.peek() as { kind: "op"; value: "+" | "-" }).value;
        this.pos++;
        const right = this.multiplicative(depth);
        if (isErr(right)) return right;
        const e = this.node({ kind: "bin", op, left, right });
        if (isErr(e)) return e;
        left = e;
        continue;
      }
      break;
    }
    return left;
  }

  /**
   * `multiplicative := unary (('*'|'/'|'%') unary)*` — left-associative,
   * binds tighter than `+`/`-`.
   * @param depth Current structural-nesting depth, passed through unchanged
   * (this production is not itself a structural boundary).
   * @returns The parsed left-associative `*`/`/`/`%` chain, or a `FormulaError`.
   * @example
   * ```
   * // not part of the public `@shadowcat/formula` surface — Parser is not exported;
   * // reachable only through `additive`.
   * this.multiplicative(0); // parses "2 * 3 / 4" as ((2 * 3) / 4)
   * ```
   */
  private multiplicative(depth: number): Expr | FormulaError {
    let left = this.unary(depth);
    if (isErr(left)) return left;
    for (;;) {
      if (this.atOp("*") || this.atOp("/") || this.atOp("%")) {
        const op = (this.peek() as { kind: "op"; value: "*" | "/" | "%" }).value;
        this.pos++;
        const right = this.unary(depth);
        if (isErr(right)) return right;
        const e = this.node({ kind: "bin", op, left, right });
        if (isErr(e)) return e;
        left = e;
        continue;
      }
      break;
    }
    return left;
  }

  /**
   * `unary := '-' unary | primary` — right-recursive, so `--1` parses as
   * double negation (`neg(neg(1))`), not a single decrement-like token.
   * Highest-precedence construct in the grammar besides parens/calls.
   * @param depth Current structural-nesting depth; a leading `-` is a
   * structural boundary and increments it by 1 for the recursive call.
   * @returns A `"neg"` node wrapping the recursive `unary` result, or
   * whatever `primary` produces, or a `"cap"` error past `MAX_PARSE_DEPTH`.
   * @example
   * ```
   * // not part of the public `@shadowcat/formula` surface — Parser is not exported;
   * // reachable only through `multiplicative`.
   * this.unary(0); // parses "-5" as { kind: "neg", operand: { kind: "num", value: 5 } }
   * ```
   */
  private unary(depth: number): Expr | FormulaError {
    if (this.atOp("-")) {
      // Structural boundary: unary-minus descent.
      const newDepth = depth + 1;
      const capErr = this.checkDepth(newDepth);
      if (capErr) return capErr;
      this.pos++;
      const operand = this.unary(newDepth);
      if (isErr(operand)) return operand;
      return this.node({ kind: "neg", operand });
    }
    return this.primary(depth);
  }

  /**
   * `primary := num | '(' additive ')' | word ('(' args ')')? ('.' word)*` —
   * the leaf level: number literals, parenthesized subexpressions, function
   * calls, and dotted reference paths. An empty formula (no tokens at all)
   * reaches this as `t === undefined` and reports `"unexpected end of
   * formula"` rather than an empty/zero AST. A leading `.` (e.g. `.5` for a
   * decimal) is NOT a numeric literal here — the lexer tokenizes `.` as a
   * bare operator, which this function's `word`/`num`/`(` branches all
   * reject, falling through to "unexpected token" (write `0.5` instead).
   * @param depth Current structural-nesting depth; `(` and a call's argument
   * list are structural boundaries and increment it by 1 for their recursive calls.
   * @returns The parsed leaf `Expr`, or a `FormulaError` (empty input,
   * unknown function, mismatched parens/args, or an unrecognized token).
   * @example
   * ```
   * // not part of the public `@shadowcat/formula` surface — Parser is not exported;
   * // reachable only through `unary`.
   * this.primary(0); // parses "hp.max" as { kind: "ref", path: ["hp", "max"] }
   * ```
   */
  private primary(depth: number): Expr | FormulaError {
    const t = this.peek();
    if (t === undefined) {
      return { error: "parse", detail: "unexpected end of formula" };
    }

    if (t.kind === "num") {
      this.pos++;
      return this.node({ kind: "num", value: t.value });
    }

    if (t.kind === "op" && t.value === "(") {
      this.pos++;
      // Structural boundary: paren-open descent.
      const newDepth = depth + 1;
      const capErr = this.checkDepth(newDepth);
      if (capErr) return capErr;
      const inner = this.additive(newDepth);
      if (isErr(inner)) return inner;
      if (!this.atOp(")")) {
        const at = this.peek();
        return {
          error: "parse",
          detail: `expected ')' at position ${at !== undefined ? at.pos : this.srcLen}`,
        };
      }
      this.pos++;
      return inner;
    }

    if (t.kind === "word") {
      this.pos++;
      if (this.atOp("(")) {
        if (!FN_NAMES.has(t.value)) {
          return { error: "parse", detail: `unknown function '${t.value}' at position ${t.pos}` };
        }
        const fn = t.value as FnName;
        this.pos++;
        // Structural boundary: call-argument descent.
        const newDepth = depth + 1;
        const capErr = this.checkDepth(newDepth);
        if (capErr) return capErr;
        const args: Expr[] = [];
        if (!this.atOp(")")) {
          for (;;) {
            const arg = this.additive(newDepth);
            if (isErr(arg)) return arg;
            args.push(arg);
            if (this.atOp(",")) {
              this.pos++;
              continue;
            }
            break;
          }
        }
        if (!this.atOp(")")) {
          const at = this.peek();
          return {
            error: "parse",
            detail: `expected ')' at position ${at !== undefined ? at.pos : this.srcLen}`,
          };
        }
        this.pos++;
        const arityErr = checkArity(fn, args.length, t.pos);
        if (arityErr) return arityErr;
        return this.node({ kind: "call", fn, args });
      }

      const path = [t.value];
      while (this.atOp(".")) {
        this.pos++;
        const seg = this.peek();
        if (seg === undefined || seg.kind !== "word") {
          return {
            error: "parse",
            detail: `expected identifier after '.' at position ${seg !== undefined ? seg.pos : this.srcLen}`,
          };
        }
        this.pos++;
        path.push(seg.value);
      }
      // One ref = one node regardless of segment count; the dotted-path
      // length is bounded by MAX_FORMULA_LENGTH (lexer), not MAX_AST_NODES.
      return this.node({ kind: "ref", path });
    }

    return { error: "parse", detail: `unexpected token at position ${t.pos}` };
  }
}

/**
 * Validates a call's argument count at parse time: `min`/`max` require at
 * least 1 argument (unbounded above); `floor`/`ceil`/`round` require
 * exactly 1. Checked once, after both parens are matched — a wrong count
 * is reported at the FUNCTION's position (`pos`), not the offending argument's.
 * @param fn The called function name.
 * @param argc Number of arguments actually parsed.
 * @param pos Source position of the function name token, used in the error detail.
 * @returns `undefined` when `argc` satisfies `fn`'s arity, else a `"parse"` error.
 * @example
 * ```
 * // not part of the public `@shadowcat/formula` surface — Parser is not exported.
 * checkArity("floor", 2, 0); // { error: "parse", detail: "'floor' requires exactly 1 argument at position 0" }
 * ```
 */
function checkArity(fn: FnName, argc: number, pos: number): FormulaError | undefined {
  if (fn === "min" || fn === "max") {
    if (argc < 1) {
      return { error: "parse", detail: `'${fn}' requires at least 1 argument at position ${pos}` };
    }
    return undefined;
  }
  // floor/ceil/round take exactly 1 arg
  if (argc !== 1) {
    return { error: "parse", detail: `'${fn}' requires exactly 1 argument at position ${pos}` };
  }
  return undefined;
}

/**
 * Structural check for a `FormulaError` shape — used throughout `Parser` to
 * short-circuit on a sub-result without importing `internal.ts`'s stricter
 * `isWellFormedError` (this module only ever inspects its OWN freshly-built
 * values, never an untrusted consumer callback's return value).
 * @param v A parser sub-result, either a successful `T` or a `FormulaError`.
 * @returns `true` when `v` is object-shaped with an `"error"` key.
 * @example
 * ```
 * // not part of the public `@shadowcat/formula` surface — this helper is not exported.
 * isErr({ error: "parse", detail: "x" }); // true
 * ```
 */
function isErr<T>(v: T | FormulaError): v is FormulaError {
  return typeof v === "object" && v !== null && "error" in v;
}

/** Parses formula source text to an `Expr` AST. Never throws — every failure
 * (lex, parse, or cap) returns a `FormulaError` value instead.
 * @param src Formula source text, e.g. `"1 + hp.max"`.
 * @returns The parsed `Expr` on success, or the first `FormulaError`
 * encountered (from tokenizing, grammar violations, or a DoS cap).
 * @example
 * ```ts
 * import { parseFormula } from "@shadowcat/formula";
 *
 * parseFormula("floor(hp.max / 2) + str");
 * ```
 */
export function parseFormula(src: string): Expr | FormulaError {
  const toks = tokenize(src);
  if (isErr(toks)) return toks;
  return new Parser(toks, src.length).parseTop();
}
