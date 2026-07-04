use crate::dice::notation::ParseError;
use crate::dice::spec::RollSpec;

pub fn parse(_input: &str) -> Result<RollSpec, ParseError> {
    Err(ParseError::Empty)
}
