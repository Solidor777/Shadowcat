//! This module defines the authored nameplate and attribute-bar configuration of an actor.
//!
//! A bar's `current` and `max` are `Formula`s the server evaluates over the token's host
//! document and delivers as plain numbers through the `"bars"` derived channel
//! (`scene::bars`); the client never receives the formula source or the host's `system` band.

// Ratchet: every item in this module must carry a doc comment, enforced by
// the two crate-level deny attributes this module declares.
#![deny(missing_docs)]
#![deny(clippy::missing_docs_in_private_items)]

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::data::engine::combat::Formula;
use crate::data::engine::geometry::validate_hex_color;

/// The most bars one actor, or one token's override, may author. A fixed sizing constant, not
/// derived from anything: a nameplate plus a handful of bars is the working set a canvas can show.
pub const MAX_BAR_ATTRIBUTES: usize = 4;

/// The longest bar label, in characters.
pub const MAX_BAR_LABEL_CHARS: usize = 64;

/// Who a nameplate or bar is delivered to. Delivery is decided server-side, per recipient, so a
/// value a recipient is not entitled to never leaves the server.
///
/// `Hover` and `Owner` deliver identically (to the token's effective owner and the GM); they
/// differ only in how the owner's client reveals the value. `Always` delivers to everyone who
/// can read the token.
///
/// # Examples
///
/// ```
/// use shadowcat::data::engine::BarDisplayMode;
///
/// assert_eq!(BarDisplayMode::default(), BarDisplayMode::Hover);
/// assert!(BarDisplayMode::Always.delivered_to(false));
/// assert!(!BarDisplayMode::Owner.delivered_to(false));
/// assert!(BarDisplayMode::Owner.delivered_to(true));
/// assert!(!BarDisplayMode::None.delivered_to(true));
/// ```
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/engine/")]
#[serde(rename_all = "snake_case")]
pub enum BarDisplayMode {
    /// Delivered to nobody, the owner and GM included: an authored value that is never surfaced.
    None,
    /// Delivered to the effective owner and the GM; their client shows it while the token is
    /// hovered or selected.
    #[default]
    Hover,
    /// Delivered to the effective owner and the GM; their client shows it always.
    Owner,
    /// Delivered to every recipient who can read the token.
    Always,
}

impl BarDisplayMode {
    /// Whether a recipient receives a value gated by this mode. `trusted` is true for the
    /// token's effective owner and the GM (`Access::can_see` at the `OwnerOrGm` tier).
    ///
    /// # Examples
    ///
    /// ```
    /// use shadowcat::data::engine::BarDisplayMode;
    ///
    /// assert!(BarDisplayMode::Hover.delivered_to(true));
    /// assert!(!BarDisplayMode::Hover.delivered_to(false));
    /// ```
    pub fn delivered_to(self, trusted: bool) -> bool {
        match self {
            BarDisplayMode::None => false,
            BarDisplayMode::Hover | BarDisplayMode::Owner => trusted,
            BarDisplayMode::Always => true,
        }
    }
}

/// One authored attribute bar: a labelled, coloured fill whose `current` and `max` are evaluated
/// server-side.
///
/// # Examples
///
/// ```
/// use shadowcat::data::engine::combat::Formula;
/// use shadowcat::data::engine::{BarAttribute, BarDisplayMode};
///
/// let bar = BarAttribute {
///     label: "HP".to_string(),
///     current: Formula::Text("hp.value".to_string()),
///     max: Formula::Number(10.0),
///     color: "#cc2222".to_string(),
///     mode: BarDisplayMode::Always,
/// };
/// assert!(bar.validate("bars[0]").is_ok());
/// ```
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/engine/")]
#[serde(deny_unknown_fields)]
pub struct BarAttribute {
    /// Display label; non-empty, at most `MAX_BAR_LABEL_CHARS` characters.
    pub label: String,
    /// The filled amount: a number, or formula source evaluated over the token's host document.
    pub current: Formula,
    /// The full amount, evaluated like `current`.
    pub max: Formula,
    /// Fill colour as a css `#rrggbb` string.
    pub color: String,
    /// Who receives this bar; absent means `BarDisplayMode::Hover`.
    #[serde(default)]
    pub mode: BarDisplayMode,
}

impl BarAttribute {
    /// Ingress validation: a non-empty bounded label, both formulas finite or parseable, and a
    /// `#rrggbb` colour. `at` prefixes every error with the bar's position.
    ///
    /// # Examples
    ///
    /// ```
    /// use shadowcat::data::engine::combat::Formula;
    /// use shadowcat::data::engine::{BarAttribute, BarDisplayMode};
    ///
    /// let bar = BarAttribute {
    ///     label: String::new(),
    ///     current: Formula::Number(1.0),
    ///     max: Formula::Number(1.0),
    ///     color: "#ffffff".to_string(),
    ///     mode: BarDisplayMode::Hover,
    /// };
    /// assert!(bar.validate("bars[0]").is_err());
    /// ```
    pub fn validate(&self, at: &str) -> Result<(), String> {
        if self.label.is_empty() {
            return Err(format!("{at}.label must not be empty"));
        }
        if self.label.chars().count() > MAX_BAR_LABEL_CHARS {
            return Err(format!(
                "{at}.label exceeds {MAX_BAR_LABEL_CHARS} characters"
            ));
        }
        self.current.validate(&format!("{at}.current"))?;
        self.max.validate(&format!("{at}.max"))?;
        validate_hex_color(&self.color).map_err(|m| format!("{at}.color: {m}"))
    }
}

/// Validates an authored bar list: at most `MAX_BAR_ATTRIBUTES` entries (a longer list is
/// refused, never truncated), each passing `BarAttribute::validate`. `at` names the list in
/// errors.
///
/// # Examples
///
/// ```
/// use shadowcat::data::engine::combat::Formula;
/// use shadowcat::data::engine::{validate_bars, BarAttribute, BarDisplayMode};
///
/// let bar = BarAttribute {
///     label: "HP".to_string(),
///     current: Formula::Number(1.0),
///     max: Formula::Number(1.0),
///     color: "#ffffff".to_string(),
///     mode: BarDisplayMode::Hover,
/// };
/// assert!(validate_bars(&[bar.clone()], "bars").is_ok());
/// assert!(validate_bars(&vec![bar; 5], "bars").is_err());
/// ```
pub fn validate_bars(bars: &[BarAttribute], at: &str) -> Result<(), String> {
    if bars.len() > MAX_BAR_ATTRIBUTES {
        return Err(format!("{at} holds more than {MAX_BAR_ATTRIBUTES} bars"));
    }
    bars.iter()
        .enumerate()
        .try_for_each(|(i, b)| b.validate(&format!("{at}[{i}]")))
}
