use std::io::{self, BufRead, BufReader, BufWriter, Read, Write};
const BLOCK_SIZE: usize = 512;

#[derive(Clone, Copy, Debug)]
#[repr(u8)]
enum SectorState {
    Untried = b'?',
    Untrimmed = b'*',
    Unscraped = b'/',
    Bad = b'-',
    Rescued = b'+',
}

#[derive(Debug)]
pub struct MapFile {
    pos: u64,
    status: u8,
    block_size: usize,
    sector_states: Vec<u8>,
}

impl MapFile {
    pub fn write_to_stream<W: Write>(&self, write: W) -> io::Result<()> {
        let mut write = BufWriter::new(write);
        writeln!(&mut write, "0x{:08X}     {}", self.pos, self.status as char)?;
        let mut current = None;
        let mut size = 0;
        let mut start = 0;

        for (idx, state) in self.sector_states.iter().enumerate() {
            if current == Some(*state) {
                size += self.block_size;
            } else {
                if size > 0 {
                    writeln!(&mut write, "0x{:08X}  0x{:08X}  {}", start, size, current.unwrap() as char)?;
                }
                start = idx * self.block_size;
                size = self.block_size;
                current = Some(*state);
            }
        }
        if size > 0 {
            writeln!(&mut write, "0x{:08X}  0x{:08X}  {}", start, size, current.unwrap() as char)?;
        }
        Ok(())
    }

    pub fn read_from_stream<R>(read: R) -> io::Result<MapFile> where R: Read {
        let buf_reader = BufReader::new(read);
        let block_size = BLOCK_SIZE;
        let mut read_state = false;
        let mut pos = None;
        let mut status = None;
        let mut states = Vec::new();

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
                let state = iter.next().unwrap().chars().next().unwrap() as u8;

                states.resize(((size + pos) / (block_size as u64)) as usize, SectorState::Untried as u8);
                assert_eq!((size + pos) % (block_size as u64), 0);
                let start = pos / (block_size as u64);
                assert_eq!(start * (block_size as u64), pos);
                let count = size / (block_size as u64);
                assert_eq!(count * (block_size as u64), size);
                for i in start..(start + count) {
                    states[i as usize] = state;
                }
            }
        }

        let result = MapFile {
            pos: pos.unwrap(),
            status: status.unwrap(),
            block_size: block_size,
            sector_states: states,
        };
        Ok(result)
    }
}

