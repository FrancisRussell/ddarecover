use std::fmt::{self, Display};
use std::error::Error;

#[derive(Debug)]
pub struct ParseError {
    target: String
}

impl ParseError {
    pub fn new(target: &str) -> ParseError {
        ParseError {
            target: String::from(target)
        }
    }
}

impl Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "Unable to parse {}", self.target)
    }
}

impl Error for ParseError {
    fn description(&self) -> &str {
        "Parse error."
    }
}


