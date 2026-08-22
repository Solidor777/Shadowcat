#![deny(missing_docs)]
#![deny(clippy::missing_docs_in_private_items)]

use crate::dice::notation::lexer::{describe_token, lex, Token};
use crate::dice::notation::{ModeKind, ParseContext, ParseError};
use crate::dice::spec::{
    BinOp, Comparator, ConstTerm, DiceGroup, DieKind, Direction, ExplodeKind, Expr, FnName,
    GroupModifier, Mode, RollSpec, SuccessConfig, SuccessRule, Tier, TotalConfig,
};

/// Recursion depth (via `expr`/`term`/`factor`'s mutual calls, e.g. through
/// nested `(...)` groups or a `-` chain) is bounded ONLY by input length --
/// there is no explicit depth counter or recursion-depth cap in this parser.
/// A caller exposing `parse` to untrusted input relies entirely on its own
/// length cap to keep worst-case nesting (and therefore stack usage) bounded.
/// Chat's `chat::MAX_MESSAGE_CHARS = 4096` caps a single formula at
/// well under that, so worst-case nesting is roughly ~2k levels (each `(` or
/// unary `-` costs at least 2 input chars) -- a few thousand light recursive-
/// descent frames, safe on all three target OSes' default thread stacks.
/// Re-evaluate with an explicit depth counter if any future caller ever
/// exposes this parser to longer untrusted input than that.
struct P {
    /// The full token stream.
    toks: Vec<Token>,
    /// Cursor into `toks`.
    pos: usize,
    /// Success rule set by a `cs` (or `cf`-implied) modifier; forces
    /// SuccessCount mode once seen.
    success: Option<SuccessRule>,
    /// Mode-agnostic `t<N>` target: resolves to
    /// `TotalConfig::difficulty` in Total mode, or a direction-derived
    /// `SuccessRule` in SuccessCount mode.
    t_target: Option<i32>,
    /// Roll-level expertise budget from an `e<N>` token. Shared state,
    /// not per-`DiceGroup`; applied only when the resolved mode is SuccessCount.
    expertise: Option<u32>,
    /// Roll-level required-successes target from an `rs<N>` token — the number of net
    /// successes a `SuccessCount` roll needs to pass overall
    /// (`SuccessConfig.required_successes`, gating `evaluate_success`'s
    /// pass/margin/tier classification). Shared state, not per-`DiceGroup`; consumed
    /// only when the resolved mode is `SuccessCount`, mirroring `expertise`'s
    /// silent-drop under `Total` — `TotalConfig` has no equivalent field, `t<N>`
    /// already fills that role via `TotalConfig.difficulty`.
    required_successes: Option<i32>,
    /// Tier-ladder rungs accumulated from `tr<offset>[:<value>][<label>]` modifiers, in
    /// occurrence order. Threaded into whichever `TotalConfig.tiers`/`SuccessConfig.tiers` the
    /// resolved mode builds; empty means the default 2-rung pass/fail ladder
    /// (`eval::classify::classify`). Repeatable — unlike `success`/`t_target`/`expertise`, a
    /// second `tr` is not an error; `chat::rolls::validate_tiers` rejects a duplicate
    /// `margin_offset` at the wire boundary, not this parser.
    tiers: Vec<Tier>,
}

/// Recursive-descent parser: `expr := term (('+'|'-') term)*`;
/// `term := factor (('*'|'/') factor)*`; `factor := '(' expr ')' | '-' factor | fn_call | dice
/// | int`; a dice factor is `int 'd' int modifier*`; `fn_call` is `ident '(' expr (',' expr)*
/// ')'` (see `fn_call`). `ctx` supplies the ambient mode/
/// direction the notation string itself does not encode; an explicit `cs`/`cf`
/// forces `SuccessCount` regardless of `ctx.mode`.
pub fn parse(input: &str, ctx: ParseContext) -> Result<RollSpec, ParseError> {
    let toks = lex(input)?;
    if toks.is_empty() {
        return Err(ParseError::Empty);
    }
    let mut p = P {
        toks,
        pos: 0,
        success: None,
        t_target: None,
        expertise: None,
        required_successes: None,
        tiers: Vec::new(),
    };
    let expr = p.expr()?;
    if p.pos != p.toks.len() {
        return Err(ParseError::Trailing(p.toks[p.pos].to_string()));
    }

    // Explicit cs/cf forces SuccessCount; otherwise the ambient mode governs.
    let success_count = p.success.is_some() || ctx.mode == ModeKind::SuccessCount;
    let mode = if success_count {
        // Resolve the per-die success rule: cs/cf rule, else a t<N> target with a
        // direction-derived comparator. Both present => collision.
        let rule = match (p.success, p.t_target) {
            (Some(_), Some(_)) => return Err(ParseError::DuplicateSuccessRule),
            (Some(r), None) => r,
            (None, Some(t)) => SuccessRule::Numeric {
                comp: match ctx.direction {
                    Direction::HighWins => Comparator::Gte,
                    Direction::LowWins => Comparator::Lte,
                },
                target: t,
            },
            (None, None) => {
                return Err(ParseError::Unexpected(
                    "SuccessCount mode requires a per-die target (t<N> or cs)".into(),
                ))
            }
        };
        Mode::SuccessCount(SuccessConfig {
            success: rule,
            required_successes: p.required_successes,
            tiers: p.tiers,
            crit_success: None,
            crit_fail: None,
            expertise: p.expertise.unwrap_or(0),
        })
    } else {
        Mode::Total(TotalConfig {
            difficulty: p.t_target,
            tiers: p.tiers,
        })
    };
    Ok(RollSpec {
        expr,
        direction: ctx.direction,
        mode,
    })
}

impl P {
    /// The current token without consuming it.
    fn peek(&self) -> Option<&Token> {
        self.toks.get(self.pos)
    }

    /// Consume and return the current token.
    fn bump(&mut self) -> Option<Token> {
        let t = self.toks.get(self.pos).cloned();
        if t.is_some() {
            self.pos += 1;
        }
        t
    }

    /// Consumes an optional trailing `Token::Label`, shared by any atomic factor
    /// that can carry one (a `DiceGroup` after its modifiers, or a bare `Const`).
    fn take_label(&mut self) -> Option<String> {
        if let Some(Token::Label(_)) = self.peek() {
            match self.bump() {
                Some(Token::Label(l)) => Some(l),
                _ => unreachable!(),
            }
        } else {
            None
        }
    }

    /// Consume an `Int` or fail with a player-presentable message.
    fn expect_int(&mut self) -> Result<i32, ParseError> {
        match self.bump() {
            Some(Token::Int(n)) => Ok(n),
            other => Err(ParseError::Unexpected(format!(
                "expected a number, found {}",
                describe_token(other.as_ref())
            ))),
        }
    }

    /// `expr := term (('+' | '-') term)*` — left-associative.
    fn expr(&mut self) -> Result<Expr, ParseError> {
        let mut lhs = self.term()?;
        while let Some(op) = match self.peek() {
            Some(Token::Plus) => Some(BinOp::Add),
            Some(Token::Minus) => Some(BinOp::Sub),
            _ => None,
        } {
            self.bump();
            let rhs = self.term()?;
            lhs = Expr::Bin {
                op,
                lhs: Box::new(lhs),
                rhs: Box::new(rhs),
            };
        }
        Ok(lhs)
    }

    /// `term := factor (('*' | '/') factor)*` — binds tighter than +/-.
    fn term(&mut self) -> Result<Expr, ParseError> {
        let mut lhs = self.factor()?;
        while let Some(op) = match self.peek() {
            Some(Token::Star) => Some(BinOp::Mul),
            Some(Token::Slash) => Some(BinOp::Div),
            _ => None,
        } {
            self.bump();
            let rhs = self.factor()?;
            lhs = Expr::Bin {
                op,
                lhs: Box::new(lhs),
                rhs: Box::new(rhs),
            };
        }
        Ok(lhs)
    }

    /// `factor := '-' factor | '(' expr ')' | fn_call | dice | int` — the leaf level;
    /// dice factors continue into `modifiers`; `fn_call` is `ident '(' expr (',' expr)*
    /// ')'` (see `fn_call`).
    fn factor(&mut self) -> Result<Expr, ParseError> {
        match self.peek() {
            Some(Token::Minus) => {
                self.bump();
                Ok(Expr::Neg(Box::new(self.factor()?)))
            }
            Some(Token::LParen) => {
                self.bump();
                let e = self.expr()?;
                match self.bump() {
                    Some(Token::RParen) => Ok(e),
                    other => Err(ParseError::Unexpected(format!(
                        "expected ')', found {}",
                        describe_token(other.as_ref())
                    ))),
                }
            }
            Some(Token::Int(_)) => {
                let n = self.expect_int()?;
                if matches!(self.peek(), Some(Token::D)) {
                    self.bump();
                    let sides = self.expect_int()?;
                    // Reject a non-positive sides count at parse time.
                    // `DieKind::Numeric { min, max }` with `min > max` (or a
                    // non-positive span) is only `debug_assert!`-guarded deep in
                    // `rng::roll_uniform`, a no-op in release builds. Never construct
                    // that variant here for an invalid range.
                    if sides < 1 {
                        return Err(ParseError::InvalidDieSides(sides));
                    }
                    let modifiers = self.modifiers(sides)?;
                    let label = self.take_label();
                    Ok(Expr::Dice(DiceGroup {
                        label,
                        count: n as u32,
                        kind: DieKind::Numeric { min: 1, max: sides },
                        modifiers,
                    }))
                } else {
                    // Generalized (was dice-only): a bare constant can carry the
                    // same trailing `[label]` a `DiceGroup` can. Root-cause fix —
                    // the grammar's own intent is that labels decorate ANY atomic
                    // factor, not just dice groups (see `ConstTerm` doc comment).
                    let label = self.take_label();
                    Ok(Expr::Const(ConstTerm { value: n, label }))
                }
            }
            Some(Token::Ident(name)) => {
                let name = name.clone();
                if matches!(self.toks.get(self.pos + 1), Some(Token::LParen)) {
                    self.fn_call(name)
                } else {
                    Err(ParseError::Unexpected(format!(
                        "expected a number or dice expression, found {}",
                        describe_token(self.peek())
                    )))
                }
            }
            other => Err(ParseError::Unexpected(format!(
                "expected a number or dice expression, found {}",
                describe_token(other)
            ))),
        }
    }

    /// Parses `fn_call := ident '(' expr (',' expr)* ')'`. Called from `factor` after peeking
    /// (not yet consuming) the leading `Ident` and confirming it is immediately followed by `(`
    /// — the only place an `Ident` is recognized as a function name rather than a dice-group
    /// modifier keyword. `name` is the already-lowercased identifier text (the lexer lowercases
    /// every `Ident` it emits). Checks the parsed argument count against `FnName::arity` before
    /// returning, so an `Expr::Call` this parser produces always carries the exact argument
    /// count its `name` requires.
    fn fn_call(&mut self, name: String) -> Result<Expr, ParseError> {
        let fn_name = match name.as_str() {
            "floor" => FnName::Floor,
            "ceil" => FnName::Ceil,
            "round" => FnName::Round,
            "abs" => FnName::Abs,
            "min" => FnName::Min,
            "max" => FnName::Max,
            other => {
                return Err(ParseError::Unexpected(format!(
                    "unknown function '{other}'"
                )))
            }
        };
        self.bump(); // the Ident
        match self.bump() {
            Some(Token::LParen) => {}
            other => {
                return Err(ParseError::Unexpected(format!(
                    "expected '(', found {}",
                    describe_token(other.as_ref())
                )))
            }
        }
        let mut args = vec![self.expr()?];
        while matches!(self.peek(), Some(Token::Comma)) {
            self.bump();
            args.push(self.expr()?);
        }
        match self.bump() {
            Some(Token::RParen) => {}
            other => {
                return Err(ParseError::Unexpected(format!(
                    "expected ')', found {}",
                    describe_token(other.as_ref())
                )))
            }
        }
        let expected = fn_name.arity();
        if args.len() != expected {
            return Err(ParseError::Unexpected(format!(
                "function '{name}' expects {expected} argument(s), found {}",
                args.len()
            )));
        }
        Ok(Expr::Call {
            name: fn_name,
            args,
        })
    }

    /// Zero or more trailing group modifiers (`!`/`!!`/`!p`, keep/drop,
    /// reroll, comparator targets); `sides` feeds default explode targets.
    fn modifiers(&mut self, sides: i32) -> Result<Vec<GroupModifier>, ParseError> {
        let mut mods = Vec::new();
        loop {
            match self.peek() {
                Some(Token::Bang) => {
                    self.bump();
                    mods.push(self.explode(ExplodeKind::Standard, sides)?);
                }
                Some(Token::BangBang) => {
                    self.bump();
                    mods.push(self.explode(ExplodeKind::Compound, sides)?);
                }
                Some(Token::BangP) => {
                    self.bump();
                    mods.push(self.explode(ExplodeKind::Penetrate, sides)?);
                }
                Some(Token::Ident(id)) => {
                    let id = id.clone();
                    self.bump();
                    match id.as_str() {
                        "kh" => mods.push(GroupModifier::KeepHighest(self.expect_int()? as u32)),
                        "kl" => mods.push(GroupModifier::KeepLowest(self.expect_int()? as u32)),
                        "dh" => mods.push(GroupModifier::DropHighest(self.expect_int()? as u32)),
                        "dl" => mods.push(GroupModifier::DropLowest(self.expect_int()? as u32)),
                        "r" | "ro" => {
                            let (comp, target) = self.cmp_target_required()?;
                            mods.push(GroupModifier::Reroll {
                                comp,
                                target,
                                once: id == "ro",
                            });
                        }
                        "cs" => {
                            let (comp, target) = self.cmp_target_required()?;
                            if self.success.is_some() {
                                return Err(ParseError::DuplicateSuccessRule);
                            }
                            self.success = Some(SuccessRule::Numeric { comp, target });
                        }
                        "t" => {
                            let n = self.expect_int()?;
                            if self.t_target.is_some() {
                                return Err(ParseError::DuplicateSuccessRule);
                            }
                            self.t_target = Some(n);
                        }
                        "e" => {
                            let n = self.expect_int()?;
                            if self.expertise.is_some() {
                                return Err(ParseError::DuplicateExpertise);
                            }
                            self.expertise = Some(n as u32);
                        }
                        "rs" => {
                            let n = self.expect_int()?;
                            if self.required_successes.is_some() {
                                return Err(ParseError::DuplicateRequiredSuccesses);
                            }
                            self.required_successes = Some(n);
                        }
                        "cf" => {
                            // Failure-counting is parsed as success on the inverted
                            // comparator — the grammar has no separate fail-count
                            // representation.
                            let (comp, target) = self.cmp_target_required()?;
                            if self.success.is_some() {
                                return Err(ParseError::DuplicateSuccessRule);
                            }
                            self.success = Some(SuccessRule::Numeric {
                                comp: invert(comp),
                                target,
                            });
                        }
                        "tr" => {
                            let margin_offset = self.expect_int()?;
                            let tier_value = if matches!(self.peek(), Some(Token::Colon)) {
                                self.bump();
                                Some(self.expect_int()?)
                            } else {
                                None
                            };
                            let label = self.take_label();
                            self.tiers.push(Tier {
                                margin_offset,
                                label,
                                tier_value,
                            });
                        }
                        other => {
                            return Err(ParseError::Unexpected(format!(
                                "unknown dice modifier '{other}'"
                            )))
                        }
                    }
                }
                _ => break,
            }
        }
        Ok(mods)
    }

    /// Explode: optional `[cmp] int`; when omitted, default to `Gte` the die max.
    fn explode(&mut self, kind: ExplodeKind, sides: i32) -> Result<GroupModifier, ParseError> {
        let (comp, target) = match self.peek() {
            Some(Token::Cmp(c)) => {
                let c = *c;
                self.bump();
                (c, self.expect_int()?)
            }
            Some(Token::Int(_)) => (Comparator::Gte, self.expect_int()?),
            _ => (Comparator::Gte, sides),
        };
        Ok(GroupModifier::Explode { kind, comp, target })
    }

    /// Require `cmp int` or a bare `int` (defaults comparator to `Gte`).
    fn cmp_target_required(&mut self) -> Result<(Comparator, i32), ParseError> {
        match self.bump() {
            Some(Token::Cmp(c)) => Ok((c, self.expect_int()?)),
            Some(Token::Int(n)) => Ok((Comparator::Gte, n)),
            other => Err(ParseError::Unexpected(format!(
                "expected a comparator or number, found {}",
                describe_token(other.as_ref())
            ))),
        }
    }
}

/// Logical complement of a comparator (`>=` <-> `<`, `=` <-> `!=`).
fn invert(c: Comparator) -> Comparator {
    match c {
        Comparator::Gte => Comparator::Lt,
        Comparator::Gt => Comparator::Lte,
        Comparator::Lte => Comparator::Gt,
        Comparator::Lt => Comparator::Gte,
        Comparator::Eq => Comparator::Ne,
        Comparator::Ne => Comparator::Eq,
    }
}

#[cfg(test)]
mod tests {
    use super::parse;
    use crate::dice::notation::{ModeKind, ParseContext, ParseError};
    use crate::dice::spec::*;

    fn dice(count: u32, min: i32, max: i32, mods: Vec<GroupModifier>) -> Expr {
        Expr::Dice(DiceGroup {
            label: None,
            count,
            kind: DieKind::Numeric { min, max },
            modifiers: mods,
        })
    }

    #[test]
    fn parses_keep_highest_plus_const() {
        let spec = parse("4d6kh3+2", ParseContext::default()).unwrap();
        assert!(matches!(spec.mode, Mode::Total(_)));
        assert_eq!(
            spec.expr,
            Expr::Bin {
                op: BinOp::Add,
                lhs: Box::new(dice(4, 1, 6, vec![GroupModifier::KeepHighest(3)])),
                rhs: Box::new(Expr::Const(ConstTerm {
                    value: 2,
                    label: None,
                })),
            }
        );
    }

    #[test]
    fn parses_success_pool() {
        let spec = parse("5d10cs>=7", ParseContext::default()).unwrap();
        match &spec.mode {
            Mode::SuccessCount(cfg) => assert_eq!(
                cfg.success,
                SuccessRule::Numeric {
                    comp: Comparator::Gte,
                    target: 7
                }
            ),
            other => panic!("expected SuccessCount mode, got {other:?}"),
        }
        assert_eq!(spec.expr, dice(5, 1, 10, vec![]));
    }

    #[test]
    fn rejects_duplicate_success_rule_across_groups() {
        // `success` is shared parser state (one RollSpec, not per-DiceGroup); a
        // second cs/cf anywhere in the expression must error rather than silently
        // overwrite the first rule (last-write-wins data loss).
        match parse("4d6cs>=5+2d8cs>=3", ParseContext::default()) {
            Err(ParseError::DuplicateSuccessRule) => {}
            other => panic!("expected DuplicateSuccessRule, got {other:?}"),
        }
    }

    #[test]
    fn parses_explode_default_target_is_die_max() {
        let spec = parse("6d6!", ParseContext::default()).unwrap();
        match spec.expr {
            Expr::Dice(g) => assert_eq!(
                g.modifiers[0],
                GroupModifier::Explode {
                    kind: ExplodeKind::Standard,
                    comp: Comparator::Gte,
                    target: 6
                }
            ),
            _ => panic!("expected dice"),
        }
    }

    #[test]
    fn parses_reroll() {
        let spec = parse("6d6r<2", ParseContext::default()).unwrap();
        match spec.expr {
            Expr::Dice(g) => assert!(matches!(
                g.modifiers[0],
                GroupModifier::Reroll {
                    once: false,
                    comp: Comparator::Lt,
                    target: 2
                }
            )),
            _ => panic!("expected dice"),
        }
    }

    #[test]
    fn parses_parentheses_and_mul() {
        assert!(matches!(
            parse("(1d4+1)*2", ParseContext::default()).unwrap().mode,
            Mode::Total(_)
        ));
    }

    #[test]
    fn rejects_empty_and_trailing() {
        assert!(parse("", ParseContext::default()).is_err());
        assert!(parse("2d6 2d6", ParseContext::default()).is_err());
    }

    #[test]
    fn rejects_zero_sides() {
        // sides < 1 must be a parse-time Err, never a constructed DieKind::Numeric
        // with a degenerate (non-positive-span) range.
        match parse("4d0", ParseContext::default()) {
            Err(ParseError::InvalidDieSides(0)) => {}
            other => panic!("expected InvalidDieSides(0), got {other:?}"),
        }
    }

    #[test]
    fn rejects_negative_sides_token_sequence() {
        // The lexer never emits a signed Int for "d-3" (Minus and Int are separate
        // tokens), so this fails as an ordinary unexpected-token error rather than
        // InvalidDieSides -- still a hard Err, never a constructed invalid DieKind.
        assert!(parse("4d-3", ParseContext::default()).is_err());
    }

    #[test]
    fn t_target_in_total_mode_sets_difficulty() {
        let spec = parse("1d20t10", ParseContext::default()).unwrap();
        match spec.mode {
            Mode::Total(c) => assert_eq!(c.difficulty, Some(10)),
            _ => panic!(),
        }
    }

    #[test]
    fn t_target_in_successcount_uses_direction_comparator() {
        let hi = parse(
            "5d10t7",
            ParseContext {
                mode: ModeKind::SuccessCount,
                direction: Direction::HighWins,
            },
        )
        .unwrap();
        match hi.mode {
            Mode::SuccessCount(c) => assert_eq!(
                c.success,
                SuccessRule::Numeric {
                    comp: Comparator::Gte,
                    target: 7
                }
            ),
            _ => panic!(),
        }
        let lo = parse(
            "5d10t7",
            ParseContext {
                mode: ModeKind::SuccessCount,
                direction: Direction::LowWins,
            },
        )
        .unwrap();
        match lo.mode {
            Mode::SuccessCount(c) => assert_eq!(
                c.success,
                SuccessRule::Numeric {
                    comp: Comparator::Lte,
                    target: 7
                }
            ),
            _ => panic!(),
        }
    }

    #[test]
    fn cs_forces_successcount_even_under_total_ambient() {
        let spec = parse(
            "5d10cs>=7",
            ParseContext {
                mode: ModeKind::Total,
                direction: Direction::HighWins,
            },
        )
        .unwrap();
        assert!(matches!(spec.mode, Mode::SuccessCount(_)));
    }

    #[test]
    fn t_and_cs_collision_errors() {
        let e = parse(
            "5d10t6cs>=7",
            ParseContext {
                mode: ModeKind::SuccessCount,
                direction: Direction::HighWins,
            },
        );
        assert!(matches!(e, Err(ParseError::DuplicateSuccessRule)));
    }

    #[test]
    fn successcount_without_target_or_rule_errors() {
        // Ambient SuccessCount with neither a cs/cf rule nor a t<N> target leaves
        // no per-die comparator to build a SuccessRule from -- must hard-error
        // rather than silently default (the (None, None) arm in `parse`).
        let e = parse(
            "5d10",
            ParseContext {
                mode: ModeKind::SuccessCount,
                direction: Direction::HighWins,
            },
        );
        assert!(matches!(e, Err(ParseError::Unexpected(_))));
    }

    #[test]
    fn e_token_sets_expertise_under_successcount() {
        let spec = parse(
            "4d6t5e3",
            ParseContext {
                mode: ModeKind::SuccessCount,
                direction: Direction::HighWins,
            },
        )
        .unwrap();
        match spec.mode {
            Mode::SuccessCount(c) => assert_eq!(c.expertise, 3),
            other => panic!("expected SuccessCount, got {other:?}"),
        }
    }

    #[test]
    fn e_token_is_discarded_under_total_ambient_without_error() {
        // A stray e<N> where the mode can't use it must NOT fail the roll.
        let spec = parse("1d20t10e3", ParseContext::default()).unwrap(); // ambient Total
        match spec.mode {
            Mode::Total(c) => assert_eq!(c.difficulty, Some(10)),
            other => panic!("expected Total, got {other:?}"),
        }
    }

    #[test]
    fn duplicate_e_token_errors() {
        let e = parse(
            "4d6t5e3e4",
            ParseContext {
                mode: ModeKind::SuccessCount,
                direction: Direction::HighWins,
            },
        );
        assert!(matches!(e, Err(ParseError::DuplicateExpertise)));
    }

    #[test]
    fn rs_token_sets_required_successes_under_successcount() {
        let spec = parse(
            "4d6t5rs2",
            ParseContext {
                mode: ModeKind::SuccessCount,
                direction: Direction::HighWins,
            },
        )
        .unwrap();
        match spec.mode {
            Mode::SuccessCount(c) => assert_eq!(c.required_successes, Some(2)),
            other => panic!("expected SuccessCount, got {other:?}"),
        }
    }

    #[test]
    fn rs_token_is_discarded_under_total_ambient_without_error() {
        // A stray rs<N> where the mode can't use it must NOT fail the roll -- mirrors
        // e<N>'s exact silent-drop-under-Total precedent.
        let spec = parse("1d20t10rs2", ParseContext::default()).unwrap(); // ambient Total
        match spec.mode {
            Mode::Total(c) => assert_eq!(c.difficulty, Some(10)),
            other => panic!("expected Total, got {other:?}"),
        }
    }

    #[test]
    fn duplicate_rs_token_errors() {
        let e = parse(
            "4d6t5rs2rs3",
            ParseContext {
                mode: ModeKind::SuccessCount,
                direction: Direction::HighWins,
            },
        );
        assert!(matches!(e, Err(ParseError::DuplicateRequiredSuccesses)));
    }

    #[test]
    fn parses_label_onto_dice_group() {
        let spec = parse("1d12[Hope]", ParseContext::default()).unwrap();
        match spec.expr {
            Expr::Dice(g) => assert_eq!(g.label, Some("Hope".to_string())),
            _ => panic!("expected dice"),
        }
    }

    #[test]
    fn parses_two_labeled_groups() {
        let spec = parse("1d12[Hope] + 1d12[Fear]", ParseContext::default()).unwrap();
        match spec.expr {
            Expr::Bin { lhs, rhs, .. } => {
                match *lhs {
                    Expr::Dice(g) => assert_eq!(g.label, Some("Hope".to_string())),
                    _ => panic!("expected dice lhs"),
                }
                match *rhs {
                    Expr::Dice(g) => assert_eq!(g.label, Some("Fear".to_string())),
                    _ => panic!("expected dice rhs"),
                }
            }
            _ => panic!("expected Bin"),
        }
    }

    #[test]
    fn duplicate_labels_across_groups_are_not_an_error() {
        // Two groups intentionally sharing a label pool under by_label — not a parse error.
        assert!(parse("1d6[Pool] + 1d6[Pool]", ParseContext::default()).is_ok());
    }

    #[test]
    fn parses_label_onto_bare_constant() {
        // The root-cause bug: a label immediately after a bare constant (not a dice
        // group) must be consumed by `factor()`, not left as unconsumed trailing input.
        let spec = parse("3[dex]", ParseContext::default()).unwrap();
        match spec.expr {
            Expr::Const(c) => {
                assert_eq!(c.value, 3);
                assert_eq!(c.label, Some("dex".to_string()));
            }
            other => panic!("expected Const, got {other:?}"),
        }
    }

    #[test]
    fn parses_dice_group_plus_labeled_constant() {
        // A dice group followed by an additive labeled constant parses as a binary
        // expression, never as trailing input.
        let spec = parse("1d20 + 3[dex]", ParseContext::default()).unwrap();
        match spec.expr {
            Expr::Bin { lhs, rhs, .. } => {
                assert!(matches!(*lhs, Expr::Dice(_)));
                match *rhs {
                    Expr::Const(c) => {
                        assert_eq!(c.value, 3);
                        assert_eq!(c.label, Some("dex".to_string()));
                    }
                    other => panic!("expected Const rhs, got {other:?}"),
                }
            }
            other => panic!("expected Bin, got {other:?}"),
        }
    }

    #[test]
    fn unlabeled_bare_constant_has_no_label() {
        let spec = parse("3", ParseContext::default()).unwrap();
        match spec.expr {
            Expr::Const(c) => {
                assert_eq!(c.value, 3);
                assert_eq!(c.label, None);
            }
            other => panic!("expected Const, got {other:?}"),
        }
    }

    #[test]
    fn parses_min_call_with_two_bare_consts() {
        let spec = parse("min(3,5)", ParseContext::default()).unwrap();
        assert_eq!(
            spec.expr,
            Expr::Call {
                name: FnName::Min,
                args: vec![
                    Expr::Const(ConstTerm {
                        value: 3,
                        label: None
                    }),
                    Expr::Const(ConstTerm {
                        value: 5,
                        label: None
                    }),
                ],
            }
        );
    }

    #[test]
    fn parses_floor_call_wrapping_a_dice_group() {
        let spec = parse("floor(1d20/2)", ParseContext::default()).unwrap();
        match spec.expr {
            Expr::Call {
                name: FnName::Floor,
                args,
            } => {
                assert_eq!(args.len(), 1);
                assert!(matches!(args[0], Expr::Bin { op: BinOp::Div, .. }));
            }
            other => panic!("expected Call, got {other:?}"),
        }
    }

    #[test]
    fn parses_nested_min_max_calls() {
        let spec = parse("max(min(1,2),3)", ParseContext::default()).unwrap();
        assert!(matches!(
            spec.expr,
            Expr::Call {
                name: FnName::Max,
                ..
            }
        ));
    }

    #[test]
    fn rejects_min_with_wrong_arity() {
        match parse("min(3)", ParseContext::default()) {
            Err(ParseError::Unexpected(msg)) => {
                assert!(msg.contains("min"), "{msg}");
                assert!(msg.contains('2'), "{msg}");
            }
            other => panic!("expected an arity Unexpected error, got {other:?}"),
        }
    }

    #[test]
    fn rejects_floor_with_wrong_arity() {
        match parse("floor(3,4)", ParseContext::default()) {
            Err(ParseError::Unexpected(msg)) => {
                assert!(msg.contains("floor"), "{msg}");
                assert!(msg.contains('1'), "{msg}");
            }
            other => panic!("expected an arity Unexpected error, got {other:?}"),
        }
    }

    #[test]
    fn rejects_unknown_function_name() {
        match parse("foo(3)", ParseContext::default()) {
            Err(ParseError::Unexpected(msg)) => assert!(msg.contains("unknown function 'foo'")),
            other => panic!("expected an unknown-function Unexpected error, got {other:?}"),
        }
    }

    #[test]
    fn bare_ident_not_followed_by_lparen_is_not_a_function_call() {
        assert!(parse("floor", ParseContext::default()).is_err());
    }

    #[test]
    fn parses_single_tier_rung_with_value_and_label() {
        let spec = parse("4d6cs>4tr3:1[Good]", ParseContext::default()).unwrap();
        match spec.mode {
            Mode::SuccessCount(c) => assert_eq!(
                c.tiers,
                vec![Tier {
                    margin_offset: 3,
                    label: Some("Good".into()),
                    tier_value: Some(1)
                }]
            ),
            other => panic!("expected SuccessCount, got {other:?}"),
        }
    }

    #[test]
    fn parses_two_tier_rungs_appended_in_order() {
        let spec = parse("4d6cs>4tr3:1[Good]tr6:2[Great]", ParseContext::default()).unwrap();
        match spec.mode {
            Mode::SuccessCount(c) => assert_eq!(
                c.tiers,
                vec![
                    Tier {
                        margin_offset: 3,
                        label: Some("Good".into()),
                        tier_value: Some(1)
                    },
                    Tier {
                        margin_offset: 6,
                        label: Some("Great".into()),
                        tier_value: Some(2)
                    },
                ]
            ),
            other => panic!("expected SuccessCount, got {other:?}"),
        }
    }

    #[test]
    fn tr_value_and_label_are_optional() {
        let spec = parse("1d20t10tr5", ParseContext::default()).unwrap();
        match spec.mode {
            Mode::Total(c) => assert_eq!(
                c.tiers,
                vec![Tier {
                    margin_offset: 5,
                    label: None,
                    tier_value: None
                }]
            ),
            other => panic!("expected Total, got {other:?}"),
        }
    }

    #[test]
    fn no_tr_leaves_tiers_empty() {
        let spec = parse("1d20t10", ParseContext::default()).unwrap();
        match spec.mode {
            Mode::Total(c) => assert!(c.tiers.is_empty()),
            other => panic!("expected Total, got {other:?}"),
        }
    }
}
