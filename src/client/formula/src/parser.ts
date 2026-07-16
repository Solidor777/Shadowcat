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

  constructor(
    private readonly toks: Tok[],
    private readonly srcLen: number,
  ) {}

  private peek(): Tok | undefined {
    return this.toks[this.pos];
  }

  private atOp(value: string): boolean {
    const t = this.peek();
    return t !== undefined && t.kind === "op" && t.value === value;
  }

  private node(e: Expr): Expr | FormulaError {
    this.nodeCount++;
    if (this.nodeCount > MAX_AST_NODES) {
      return { error: "cap", detail: "formula exceeds 256 AST nodes" };
    }
    return e;
  }

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
  private checkDepth(depth: number): FormulaError | undefined {
    if (depth > MAX_PARSE_DEPTH) {
      return { error: "cap", detail: "formula exceeds max nesting depth of 32" };
    }
    return undefined;
  }

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

function isErr<T>(v: T | FormulaError): v is FormulaError {
  return typeof v === "object" && v !== null && "error" in v;
}

/** Parses formula source text to an `Expr` AST. Never throws — every failure
 * (lex, parse, or cap) returns a `FormulaError` value instead. */
export function parseFormula(src: string): Expr | FormulaError {
  const toks = tokenize(src);
  if (isErr(toks)) return toks;
  return new Parser(toks, src.length).parseTop();
}
