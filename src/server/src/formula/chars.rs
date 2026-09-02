//! The character classes both grammars in this crate accept — the formula
//! tokenizer (`lexer`) and the notation-template rewriter (`template`) — in
//! ONE declaration, twin of the client package's `chars.ts`. The two grammars
//! accept the same identifier characters, so a per-module copy is a forked
//! decision; the grammars differ ABOVE this layer (what may follow an
//! identifier, which words are reserved, how a malformed reference fails), and
//! nothing here encodes any of that.

#![deny(missing_docs)]
#![deny(clippy::missing_docs_in_private_items)]

/// True for an ASCII decimal digit.
pub(crate) fn is_digit(c: char) -> bool {
    c.is_ascii_digit()
}

/// True for a character that may start an identifier: ASCII letter or `_`.
pub(crate) fn is_word_start(c: char) -> bool {
    c.is_ascii_alphabetic() || c == '_'
}

/// True for a character that may continue an identifier: letter, digit or `_`.
pub(crate) fn is_word_char(c: char) -> bool {
    is_word_start(c) || is_digit(c)
}
