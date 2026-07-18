use crate::dice::notation::lexer::{describe_token, lex, Token};
use crate::dice::notation::{ModeKind, ParseContext, ParseError};
use crate::dice::spec::{
    BinOp, Comparator, ConstTerm, DiceGroup, DieKind, Direction, ExplodeKind, Expr, GroupModifier,
    Mode, RollSpec, SuccessConfig, SuccessRule, TotalConfig,
};

/// Recursion depth (via `expr`/`term`/`factor`'s mutual calls, e.g. through
/// nested `(...)` groups or a `-` chain) is bounded ONLY by input length --
/// there is no explicit depth counter or recursion-depth cap in this parser.
/// A caller exposing `parse` to untrusted input relies entirely on its own
/// length cap to keep worst-case nesting (and therefore stack usage) bounded.
/// Chat's `MAX_MESSAGE_CHARS = 4096` (`chat/mod.rs`) caps a single formula at
/// well under that, so worst-case nesting is roughly ~2k levels (each `(` or
/// unary `-` costs at least 2 input chars) -- a few thousand light recursive-
/// descent frames, safe on all three target OSes' default thread stacks.
/// Re-evaluate with an explicit depth counter if any future caller ever
/// exposes this parser to longer untrusted input than that.
struct P {
    toks: Vec<Token>,
    pos: usize,
    success: Option<SuccessRule>,
    /// Mode-agnostic `t<N>` target (design's notation pillar, §10): resolves to
    /// `TotalConfig::difficulty` in Total mode, or a direction-derived
    /// `SuccessRule` in SuccessCount mode.
    t_target: Option<i32>,
    /// Roll-level expertise budget from an `e<N>` token (design §8/R4). Shared state,
    /// not per-`DiceGroup`; applied only when the resolved mode is SuccessCount.
    expertise: Option<u32>,
}

/// Recursive-descent parser: `expr := term (('+'|'-') term)*`;
/// `term := factor (('*'|'/') factor)*`; `factor := '(' expr ')' | '-' factor | dice | int`;
/// a dice factor is `int 'd' int modifier*`. `ctx` supplies the ambient mode/
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
            required_successes: None,
            tiers: vec![],
            crit_success: None,
            crit_fail: None,
            expertise: p.expertise.unwrap_or(0),
        })
    } else {
        Mode::Total(TotalConfig {
            difficulty: p.t_target,
            tiers: vec![],
        })
    };
    Ok(RollSpec {
        expr,
        direction: ctx.direction,
        mode,
    })
}

impl P {
    fn peek(&self) -> Option<&Token> {
        self.toks.get(self.pos)
    }

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

    fn expect_int(&mut self) -> Result<i32, ParseError> {
        match self.bump() {
            Some(Token::Int(n)) => Ok(n),
            other => Err(ParseError::Unexpected(format!(
                "expected a number, found {}",
                describe_token(other.as_ref())
            ))),
        }
    }

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
                    // Adaptation: reject a non-positive sides count at parse time.
                    // `DieKind::Numeric { min, max }` with `min > max` (or a
                    // non-positive span) is only `debug_assert!`-guarded deep in
                    // `rng::roll_uniform`, a no-op in release builds. Never construct
                    // that variant here for an invalid range (docs/TODO.md
                    // "Server / dice (M11a)" min>max validation gap).
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
            other => Err(ParseError::Unexpected(format!(
                "expected a number or dice expression, found {}",
                describe_token(other)
            ))),
        }
    }

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
                        "cf" => {
                            // Failure-counting parsed as success on the inverted comparator
                            // (single count path in M11a; dedicated fail-count is M11b).
                            let (comp, target) = self.cmp_target_required()?;
                            if self.success.is_some() {
                                return Err(ParseError::DuplicateSuccessRule);
                            }
                            self.success = Some(SuccessRule::Numeric {
                                comp: invert(comp),
                                target,
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
        // with a degenerate (non-positive-span) range (Adaptation 1 / TODO.md
        // "Server / dice (M11a)" min>max validation gap).
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
        // R4: a stray e<N> where the mode can't use it must NOT fail the roll.
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
        // The exact failing case from the e2e report: a dice group followed by an
        // additive labeled constant must no longer error as trailing input.
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
}
