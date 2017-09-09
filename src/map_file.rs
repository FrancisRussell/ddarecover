use parse_error::ParseError;
use phase::Phase;
use std::cmp;
use std::collections::HashMap;
use std::fs::{self, OpenOptions};
use std::io::{self, BufRead, BufReader, BufWriter, Read, Write};
use std::ops::Range;
use std::path::Path;
use std::str::FromStr;
use tagged_range::{self, TaggedRange};
use combine::{self, Stream, Parser};
use std::error::Error;

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[repr(u8)]
pub enum SectorState {
    Untried = b'?',
    Untrimmed = b'*',
    Unscraped = b'/',
    Bad = b'-',
    Rescued = b'+',
}

impl SectorState {
    pub fn from_char(c: char) -> Result<SectorState, ParseError> {
        use self::SectorState::*;
        for state in [Untried, Untrimmed, Unscraped, Bad, Rescued].iter() {
            if c as u8 == *state as u8 {
                return Ok(state.clone())
            }
        }
        Err(ParseError::new("sector state"))
    }

    pub fn as_char(&self) -> char {
        *self as u8 as char
    }
}

#[derive(Debug)]
pub struct MapFile {
    pos: u64,
    status: Phase,
    pass: usize,
    size_bytes: u64,
    sector_states: TaggedRange<SectorState>,
}

impl MapFile {
    pub fn write_to_stream<W: Write>(&self, write: W) -> io::Result<()> {
        let mut write = BufWriter::new(write);
        writeln!(&mut write, "0x{:08X}     {}     {}", self.pos, self.status.as_char(), self.pass)?;
        for region in self.sector_states.into_iter() {
            writeln!(&mut write, "0x{:08X}  0x{:08X}  {}", region.start, region.length, region.tag.as_char())?;
        }
        Ok(())
    }

    pub fn write_to_path(&self, path: &Path) -> io::Result<()> {
        let mut tmp_path = path.to_path_buf();
        tmp_path.set_extension("ddarescue-tmp");
        {
            let mut file = OpenOptions::new()
                    .create_new(true)
                    .write(true)
                    .open(&tmp_path)?;
            self.write_to_stream(&mut file)?;
            file.flush()?;
            file.sync_all()?;
        }
        fs::rename(tmp_path, path)?;
        Ok(())
    }

    pub fn get_size_bytes(&self) -> u64 {
        self.size_bytes
    }

    pub fn get_pass(&self) -> usize {
        self.pass
    }

    pub fn set_pass(&mut self, pass: usize) {
        self.pass = pass;
    }

    pub fn next_pass(&mut self) {
        self.pass += 1;
    }

    pub fn read_from_stream<R>(read: R) -> Result<MapFile, Box<Error>> where R: Read {
        let buf_reader = BufReader::new(read);
        let mut read_state = false;
        let mut pos = None;
        let mut status = None;
        let mut pass = None;
        let mut sector_states = TaggedRange::new();
        let mut size_bytes = 0;

        for line in buf_reader.lines() {
            let line = line?;
            if line.starts_with("#") {
                continue;
            }

            if !read_state {
                let mut parser = (combine::parser(Self::parse_hex_value),
                              combine::skip_many1(combine::char::space()),
                              combine::any().and_then(Phase::from_char),
                              combine::skip_many1(combine::char::space()),
                              combine::many1::<String, _>(combine::char::digit()).and_then(|x| usize::from_str(x.as_str()))
                              );
                let parsed = parser.parse(line.as_str()).map_err(|err| err.map_range(|s| s.to_string()))?.0;

                pos = Some(parsed.0);
                status = Some(parsed.2);
                pass = Some(parsed.4);
                read_state = true;
            } else {
                let mut parser = (combine::parser(Self::parse_hex_value),
                              combine::skip_many1(combine::char::space()),
                              combine::parser(Self::parse_hex_value),
                              combine::skip_many1(combine::char::space()),
                              combine::any().and_then(SectorState::from_char)
                              );
                let parsed = parser.parse(line.as_str()).map_err(|err| err.map_range(|s| s.to_string()))?.0;
                let pos = parsed.0;
                let size = parsed.2;
                let state = parsed.4;
                sector_states.put(pos..(pos+size), state);
                size_bytes = cmp::max(size_bytes, pos + size);
            }
        }

        let result = MapFile {
            pos: pos.unwrap(),
            status: status.unwrap(),
            pass: pass.unwrap(),
            sector_states: sector_states,
            size_bytes: size_bytes,
        };
        Ok(result)
    }

    pub fn new(size_bytes: u64) -> MapFile {
        let mut sector_states = TaggedRange::new();
        sector_states.put(0..size_bytes, SectorState::Untried);
        MapFile {
            pos: 0,
            status: Phase::Copying,
            size_bytes: size_bytes,
            sector_states: sector_states,
            pass: 1,
        }
    }

    pub fn put(&mut self, range: Range<u64>, state: SectorState) {
        self.sector_states.put(range, state);
    }

    pub fn iter<'a>(&'a self) -> tagged_range::Iter<'a, SectorState> {
        self.into_iter()
    }

    pub fn iter_range<'a>(&'a self, range: Range<u64>) -> tagged_range::Iter<'a, SectorState> {
        self.sector_states.iter_range(range)
    }

    pub fn get_pos(&self) -> u64 {
        self.pos
    }

    pub fn set_pos(&mut self, pos: u64) {
        self.pos = pos;
    }

    pub fn get_phase(&self) -> Phase {
        self.status
    }

    pub fn set_phase(&mut self, phase: &Phase) {
        self.status = *phase;
    }

    pub fn get_size(&self) -> u64 {
        self.size_bytes
    }

    pub fn get_histogram(&self) -> HashMap<SectorState, u64> {
        let mut result = HashMap::new();
        for region in self.sector_states.iter() {
            *result.entry(region.tag).or_insert(0) += region.length;
        }
        result
    }

    fn parse_hex_value<I: Stream<Item = char>>(input: I) -> combine::ParseResult<u64, I> {
        let prefix = combine::char::string("0x");
        let digits = combine::combinator::many1(combine::char::hex_digit());
        let value = digits.and_then(|x: String| u64::from_str_radix(x.as_str(), 16));
        let mut token = prefix.with(value);
        token.parse_stream(input)
    }
}

impl<'a> IntoIterator for &'a MapFile {
    type IntoIter = <&'a TaggedRange<SectorState> as IntoIterator>::IntoIter;
    type Item = tagged_range::Region<SectorState>;

    fn into_iter(self) -> Self::IntoIter {
        self.sector_states.into_iter()
    }
}

