pub mod lexer;
pub mod parser;

pub use parser::parse;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ParseError {
    Empty,
    Unexpected(String),
    Trailing(String),
}
