use map_file::SectorState;
use parse_error::ParseError;
use phase::Phase::*;
use std::{iter, slice};

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[repr(u8)]
pub enum Phase {
    Copying = b'?',
    Trimming = b'*',
    Scraping = b'/',
    Retrying = b'-',
    Filling = b'F',
    Generating = b'G',
    Finished = b'+',
}

static PHASES: [Phase; 5] = [Copying, Trimming, Scraping, Retrying, Finished];

impl Phase {
    pub fn from_char(c: char) -> Result<Phase, ParseError> {
        for state in Self::values() {
            if c as u8 == state as u8 {
                return Ok(state.clone())
            }
        }
        Err(ParseError::new("phase"))
    }

    pub fn as_char(&self) -> char {
        *self as u8 as char
    }

    fn values() -> iter::Cloned<slice::Iter<'static, Phase>> {
        PHASES.iter().cloned()
    }

    pub fn next(&self) -> Option<Self> {
        match *self {
            Copying => Some(Trimming),
            Trimming => Some(Scraping),
            Scraping => Some(Retrying),
            Retrying => Some(Finished),
            Filling => None,
            Generating => None,
            Finished => None,
        }

    }

    pub fn target_sectors(&self) -> Option<SectorState> {
        match *self {
            Copying => Some(SectorState::Untried),
            Trimming => Some(SectorState::Untrimmed),
            Scraping => Some(SectorState::Unscraped),
            Retrying => Some(SectorState::Bad),
            Filling => None,
            Generating => None,
            Finished => None,
        }
    }

    pub fn name(&self) -> String {
        format!("{:?}", self)
    }
}
