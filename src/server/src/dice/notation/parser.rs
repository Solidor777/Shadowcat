use crate::dice::notation::lexer::{lex, Token};
use crate::dice::notation::ParseError;
use crate::dice::spec::{
    BinOp, Comparator, DiceGroup, DieKind, Direction, ExplodeKind, Expr, GroupModifier, Mode,
    RollSpec, SuccessConfig, SuccessRule, TotalConfig,
};

struct P {
    toks: Vec<Token>,
    pos: usize,
    success: Option<SuccessRule>,
}

/// Recursive-descent parser: `expr := term (('+'|'-') term)*`;
/// `term := factor (('*'|'/') factor)*`; `factor := '(' expr ')' | '-' factor | dice | int`;
/// a dice factor is `int 'd' int modifier*`.
pub fn parse(input: &str) -> Result<RollSpec, ParseError> {
    let toks = lex(input)?;
    if toks.is_empty() {
        return Err(ParseError::Empty);
    }
    let mut p = P {
        toks,
        pos: 0,
        success: None,
    };
    let expr = p.expr()?;
    if p.pos != p.toks.len() {
        return Err(ParseError::Trailing(format!("{:?}", p.toks[p.pos])));
    }
    let mode = match p.success {
        Some(rule) => Mode::SuccessCount(SuccessConfig {
            success: rule,
            required_successes: None,
            tiers: vec![],
            crit_success: None,
            crit_fail: None,
        }),
        None => Mode::Total(TotalConfig {
            difficulty: None,
            tiers: vec![],
        }),
    };
    Ok(RollSpec {
        expr,
        direction: Direction::HighWins,
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

    fn expect_int(&mut self) -> Result<i32, ParseError> {
        match self.bump() {
            Some(Token::Int(n)) => Ok(n),
            other => Err(ParseError::Unexpected(format!("{other:?}, expected int"))),
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
                    other => Err(ParseError::Unexpected(format!("{other:?}, expected )"))),
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
                    Ok(Expr::Dice(DiceGroup {
                        count: n as u32,
                        kind: DieKind::Numeric { min: 1, max: sides },
                        modifiers,
                    }))
                } else {
                    Ok(Expr::Const(n))
                }
            }
            other => Err(ParseError::Unexpected(format!("{other:?}"))),
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
                            self.success = Some(SuccessRule { comp, target });
                        }
                        "cf" => {
                            // Failure-counting parsed as success on the inverted comparator
                            // (single count path in M11a; dedicated fail-count is M11b).
                            let (comp, target) = self.cmp_target_required()?;
                            if self.success.is_some() {
                                return Err(ParseError::DuplicateSuccessRule);
                            }
                            self.success = Some(SuccessRule {
                                comp: invert(comp),
                                target,
                            });
                        }
                        other => return Err(ParseError::Unexpected(format!("modifier {other}"))),
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
                "{other:?}, expected comparator/int"
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
    use crate::dice::notation::ParseError;
    use crate::dice::spec::*;

    fn dice(count: u32, min: i32, max: i32, mods: Vec<GroupModifier>) -> Expr {
        Expr::Dice(DiceGroup {
            count,
            kind: DieKind::Numeric { min, max },
            modifiers: mods,
        })
    }

    #[test]
    fn parses_keep_highest_plus_const() {
        let spec = parse("4d6kh3+2").unwrap();
        assert!(matches!(spec.mode, Mode::Total(_)));
        assert_eq!(
            spec.expr,
            Expr::Bin {
                op: BinOp::Add,
                lhs: Box::new(dice(4, 1, 6, vec![GroupModifier::KeepHighest(3)])),
                rhs: Box::new(Expr::Const(2)),
            }
        );
    }

    #[test]
    fn parses_success_pool() {
        let spec = parse("5d10cs>=7").unwrap();
        match &spec.mode {
            Mode::SuccessCount(cfg) => assert_eq!(
                cfg.success,
                SuccessRule {
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
        match parse("4d6cs>=5+2d8cs>=3") {
            Err(ParseError::DuplicateSuccessRule) => {}
            other => panic!("expected DuplicateSuccessRule, got {other:?}"),
        }
    }

    #[test]
    fn parses_explode_default_target_is_die_max() {
        let spec = parse("6d6!").unwrap();
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
        let spec = parse("6d6r<2").unwrap();
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
        assert!(matches!(parse("(1d4+1)*2").unwrap().mode, Mode::Total(_)));
    }

    #[test]
    fn rejects_empty_and_trailing() {
        assert!(parse("").is_err());
        assert!(parse("2d6 2d6").is_err());
    }

    #[test]
    fn rejects_zero_sides() {
        // sides < 1 must be a parse-time Err, never a constructed DieKind::Numeric
        // with a degenerate (non-positive-span) range (Adaptation 1 / TODO.md
        // "Server / dice (M11a)" min>max validation gap).
        match parse("4d0") {
            Err(ParseError::InvalidDieSides(0)) => {}
            other => panic!("expected InvalidDieSides(0), got {other:?}"),
        }
    }

    #[test]
    fn rejects_negative_sides_token_sequence() {
        // The lexer never emits a signed Int for "d-3" (Minus and Int are separate
        // tokens), so this fails as an ordinary unexpected-token error rather than
        // InvalidDieSides -- still a hard Err, never a constructed invalid DieKind.
        assert!(parse("4d-3").is_err());
    }
}
