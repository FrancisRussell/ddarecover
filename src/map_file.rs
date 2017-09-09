use std::io::{self, BufRead, BufReader, BufWriter, Read, Write};
use tagged_range::{self, TaggedRange};
use std::error::Error;
use std::fmt::{self, Display};
use std::cmp;
use std::ops::Range;
use std::path::Path;
use std::fs::{self, OpenOptions};
use std::collections::HashMap;

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[repr(u8)]
pub enum SectorState {
    Untried = b'?',
    Untrimmed = b'*',
    Unscraped = b'/',
    Bad = b'-',
    Rescued = b'+',
}

#[derive(Debug)]
pub struct ParseSectorStateError {
}

impl Display for ParseSectorStateError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}", self.description())
    }
}

impl Error for ParseSectorStateError {
    fn description(&self) -> &str {
        "Unknown character when parsing sector state."
    }
}

impl SectorState {
    pub fn from_u8(c: u8) -> Result<SectorState, ParseSectorStateError> {
        use self::SectorState::*;
        for state in [Untried, Untrimmed, Unscraped, Bad, Rescued].iter() {
            if c == *state as u8 {
                return Ok(state.clone())
            }
        }
        Err(ParseSectorStateError{})
    }

    pub fn as_char(&self) -> char {
        *self as u8 as char
    }
}

#[derive(Debug)]
pub struct MapFile {
    pos: u64,
    status: u8,
    size_bytes: u64,
    sector_states: TaggedRange<SectorState>,
}

impl MapFile {
    pub fn write_to_stream<W: Write>(&self, write: W) -> io::Result<()> {
        let mut write = BufWriter::new(write);
        writeln!(&mut write, "0x{:08X}     {}", self.pos, self.status as char)?;
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

    pub fn read_from_stream<R>(read: R) -> io::Result<MapFile> where R: Read {
        let buf_reader = BufReader::new(read);
        let mut read_state = false;
        let mut pos = None;
        let mut status = None;
        let mut sector_states = TaggedRange::new();
        let mut size_bytes = 0;

        for line in buf_reader.lines() {
            let line = line?;
            if line.starts_with("#") {
                continue;
            }

            let radix = 16;
            if !read_state {
                let mut iter = line.split_whitespace();
                pos = Some(u64::from_str_radix(&iter.next().unwrap()[2..], radix).unwrap());
                status = Some(iter.next().unwrap().parse::<String>().unwrap().trim().chars().next().unwrap() as u8);
                read_state = true;
            } else {
                let mut iter = line.split_whitespace();
                let pos = u64::from_str_radix(&iter.next().unwrap()[2..], radix).unwrap();
                let size = u64::from_str_radix(&iter.next().unwrap()[2..], radix).unwrap();
                let state = SectorState::from_u8(iter.next().unwrap().chars().next().unwrap() as u8).unwrap();
                sector_states.put(pos..(pos+size), state);
                size_bytes = cmp::max(size_bytes, pos + size);
            }
        }

        let result = MapFile {
            pos: pos.unwrap(),
            status: status.unwrap(),
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
            status: b'?',
            size_bytes: size_bytes,
            sector_states: sector_states,
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
}

impl<'a> IntoIterator for &'a MapFile {
    type IntoIter = <&'a TaggedRange<SectorState> as IntoIterator>::IntoIter;
    type Item = tagged_range::Region<SectorState>;

    fn into_iter(self) -> Self::IntoIter {
        self.sector_states.into_iter()
    }
}

