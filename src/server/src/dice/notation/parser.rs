#![deny(missing_docs)]
#![deny(clippy::missing_docs_in_private_items)]

use crate::dice::notation::lexer::{describe_token, lex, Token};
use crate::dice::notation::{ModeKind, ParseContext, ParseError};
use crate::dice::spec::{
    BinOp, Comparator, ConstTerm, CritFail, CritSuccess, CritTrigger, DiceGroup, DieKind,
    Direction, ExplodeKind, Expr, FnName, GroupModifier, Mode, RollSpec, SuccessConfig,
    SuccessRule, Tier, TotalConfig,
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
    /// Crit-success trigger from an `xs<N>[:<extra>[:<counter>]]` modifier. Shared roll-level
    /// state (a second `xs` errors via `ParseError::DuplicateCritSuccess` rather than silently
    /// overwriting); consumed into `SuccessConfig.crit_success` only when the resolved mode is
    /// `SuccessCount`, mirroring `expertise`'s silent-drop under `Total`.
    crit_success: Option<CritSuccess>,
    /// Crit-fail trigger from an `xf<N>[:<lost>[:<counter>]][!]`. Same sharing/mode-gating as
    /// `crit_success`.
    crit_fail: Option<CritFail>,
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
        crit_success: None,
        crit_fail: None,
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
            crit_success: p.crit_success,
            crit_fail: p.crit_fail,
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
                        "xs" => {
                            if self.crit_success.is_some() {
                                return Err(ParseError::DuplicateCritSuccess);
                            }
                            let threshold = self.expect_int()?;
                            let extra_successes = self.optional_colon_int(1)?;
                            let positive_counter = self.optional_colon_int(1)?;
                            self.crit_success = Some(CritSuccess {
                                trigger: CritTrigger::AtLeast(threshold),
                                extra_successes,
                                positive_counter,
                            });
                        }
                        "xf" => {
                            if self.crit_fail.is_some() {
                                return Err(ParseError::DuplicateCritFail);
                            }
                            let threshold = self.expect_int()?;
                            let lost = self.optional_colon_int(1)?;
                            let negative_counter = self.optional_colon_int(1)?;
                            let allow_negative = matches!(self.peek(), Some(Token::Bang));
                            if allow_negative {
                                self.bump();
                            }
                            self.crit_fail = Some(CritFail {
                                trigger: CritTrigger::AtLeast(threshold),
                                lost,
                                negative_counter,
                                allow_negative,
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

    /// Reads an optional `:<int>` suffix, defaulting to `default` when no `Colon` token is
    /// present. Shared by `xs`/`xf`'s repeated `:<value>[:<value>]` shape: calling this twice in
    /// a row correctly reads zero, one, or two colon-prefixed values in sequence.
    fn optional_colon_int(&mut self, default: i32) -> Result<i32, ParseError> {
        if matches!(self.peek(), Some(Token::Colon)) {
            self.bump();
            self.expect_int()
        } else {
            Ok(default)
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
mod tests;
