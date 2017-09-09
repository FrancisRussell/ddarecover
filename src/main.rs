extern crate ddarecover;
extern crate ansi_escapes;

use ddarecover::block::{BlockDevice, Buffer, Request};
use ddarecover::map_file::{MapFile, SectorState};
use ddarecover::out_file::OutFile;
use std::cmp;
use std::error::Error;
use std::fs::File;
use std::ops::Range;
use std::time::Instant;
use std::path::{Path, PathBuf};
use std::io::{self, Seek, SeekFrom, Write};
use std::collections::HashMap;

const NUM_BUFFERS: usize = 256;
const READ_BATCH_SIZE: usize = 256;
const SYNC_INTERVAL: usize = 60;

struct Recover {
    block: BlockDevice,
    map_file: MapFile,
    map_file_path: PathBuf,
    out_file: OutFile,
    start: Instant,
    last_sync: Instant,
    histogram: HashMap<SectorState, u64>,
    phase_target: SectorState,
}

impl Recover {
    pub fn new(infile_path: &str, outfile_path: &str, mapfile_path: &str) -> io::Result<Recover> {
        let block = BlockDevice::open(infile_path).expect("Unable to open block device");
        let map_path = Path::new(mapfile_path);
        let map = if map_path.exists() {
            let map_file = File::open(map_path).expect("Unable to open existing map file");
            MapFile::read_from_stream(map_file).expect("Error reading map file")
        } else {
            let map = MapFile::new(block.get_size_bytes());
            map.write_to_path(map_path).expect("Unable to create new map file");
            map
        };
        assert_eq!(map.get_size_bytes(), block.get_size_bytes(), "Mismatch between device size and map file");
        let outfile_path = Path::new(outfile_path);
        let outfile = OutFile::open(outfile_path, block.get_size_bytes()).expect("Unable to open output file");

        let histogram = map.get_histogram();
        let result = Recover {
            block: block,
            map_file: map,
            map_file_path: map_path.to_path_buf(),
            out_file: outfile,
            start: Instant::now(),
            last_sync: Instant::now(),
            histogram: histogram,
            phase_target: SectorState::Untried,
        };
        Ok(result)
    }

    fn do_sync(&mut self) -> io::Result<()> {
        self.out_file.sync()?;
        self.map_file.write_to_path(&self.map_file_path)?;
        self.last_sync = Instant::now();
        Ok(())
    }

    fn print_status(&self, overwrite: bool) {
        if overwrite {
            print!("{}", ansi_escapes::EraseLines(5));
        }
        println!("Press Ctrl+C to exit.");
        println!("{:>15}: {:15} {:>15}: {:15} {:>15}: {:15}",
                 "ipos", self.format_bytes(self.map_file.get_pos()),
                 "rescued", self.get_histogram_value_formatted(SectorState::Rescued),
                 "bad", self.get_histogram_value_formatted(SectorState::Bad));

        println!("{:>15}: {:15} {:>15}: {:15} {:>15}: {:15}",
                 "non-tried", self.get_histogram_value_formatted(SectorState::Untried),
                 "non-trimmed", self.get_histogram_value_formatted(SectorState::Untrimmed),
                 "non-scraped", self.get_histogram_value_formatted(SectorState::Unscraped));

        let now = Instant::now();
        let run_time_seconds =  now.duration_since(self.start).as_secs();
        println!("{:>15}: {:15}", "run time", self.format_seconds(run_time_seconds));
    }

    fn format_bytes(&self, bytes: u64) -> String {
        let units = ["KiB", "MiB", "GiB"];
        let mut res_unit = "B";
        let mut res_bytes = bytes as f64;
        for unit in units.iter() {
            if res_bytes >= 1000000.0 {
                res_bytes /= 1024.0;
                res_unit = *unit;
            }
        }
        format!("{:.1} {}", res_bytes, res_unit)
    }

    fn format_seconds(&self, seconds: u64) -> String {
        let mut result = String::new();
        let mut value = seconds;
        for &(unit, multiple) in [("s", 60), ("m", 60), ("h", 24), ("d", usize::max_value())].iter() {
            let multiple = multiple as u64;
            result = format!(" {}{} {}", value % multiple, unit, result);
            value /= multiple;

            if value == 0 {
                break;
            }
        }
        result.trim().to_string()
    }

    fn get_histogram_value_formatted(&self, state: SectorState) -> String {
        self.format_bytes(self.get_histogram_value(state))
    }

    fn get_histogram_value(&self, state: SectorState) -> u64 {
        *self.histogram.get(&state).unwrap_or(&0)
    }

    fn update_histogram(&mut self, bytes: u64, from: SectorState, to: SectorState) {
        *self.histogram.entry(from).or_insert(0) -= bytes;
        *self.histogram.entry(to).or_insert(0) += bytes;
    }

    fn do_phase(&mut self) -> Result<(), Box<Error>> {
        let phase_target = self.phase_target;
        self.print_status(false);
        while self.get_histogram_value(phase_target) > 0 {
            self.do_pass()?;
            self.map_file.set_pos(0);
        }
        Ok(())
    }

    fn do_pass(&mut self) -> Result<(), Box<Error>> {
        let sectors_per_buffer = self.block.get_block_size_physical() / self.block.get_sector_size();
        let mut buffers: Vec<Buffer> = Vec::new();
        for _ in 0..NUM_BUFFERS {
            let buffer = self.block.create_io_buffer(sectors_per_buffer);
            buffers.push(buffer);
        }

        let mut pass_complete = false;
        while !pass_complete {
            let mut reads: Vec<Range<u64>> =
                (&self.map_file).iter_range(self.map_file.get_pos()..self.map_file.get_size())
                .filter(|r| r.tag == self.phase_target)
                .flat_map(|r| range_to_reads(&r.as_range(), &self.block))
                .take(READ_BATCH_SIZE).collect();

            pass_complete = reads.is_empty();
            while !reads.is_empty() || self.block.requests_pending() > 0 {
                while !reads.is_empty() && self.block.requests_avail() > 0 && !buffers.is_empty() {
                    let read = reads.pop().unwrap();
                    let mut buffer = buffers.pop().unwrap();
                    buffer.clear();
                    let request = Request::new(read.start, read.end - read.start, buffer);
                    self.block.submit_request(request)?;
                    let current_pos = self.map_file.get_pos();
                    self.map_file.set_pos(cmp::max(current_pos, read.end));
                }

                if (reads.is_empty() && self.block.requests_pending() > 0) || self.block.requests_avail() == 0 {
                    let request = self.block.get_completed_request()?;
                    let phase_target = self.phase_target;
                    if request.result > 0 {
                        if !request.is_data_zeros() {
                            self.out_file.seek(SeekFrom::Start(request.offset))?;
                            self.out_file.write_all(request.get_data())?;
                        }
                        self.update_histogram(request.result as u64, phase_target, SectorState::Rescued);
                        self.map_file.put(request.offset..(request.offset + request.result as u64), SectorState::Rescued);
                    } else {
                        self.update_histogram(request.size as u64, phase_target, SectorState::Bad);
                        self.map_file.put(request.offset..(request.offset + request.size), SectorState::Bad);
                    };
                    buffers.push(request.reclaim_buffer());

                    let now = Instant::now();
                    self.print_status(true);
                    if now.duration_since(self.last_sync.clone()).as_secs() >= SYNC_INTERVAL as u64 {
                        self.do_sync()?;
                    }
                }
            }
        }
        self.do_sync()?;
        Ok(())
        /*
        let end_time = Instant::now();
        let duration = end_time.duration_since(start_time);
        let duration_secs = (duration.as_secs() as f64) + (duration.subsec_nanos() as f64 * 1e-9);
        println!("Recovered: {} bytes, failed: {} bytes, duration: {:.2} seconds.", recovered, failed, duration_secs);
        println!("Recovered at {:.1} KiB/s, failed at {:.1} KiB/s, total: {:.1} KiB/s.",
                 recovered as f64 / 1024.0 / duration_secs,
                 failed as f64 / 1024.0 / duration_secs,
                 (recovered + failed) as f64 / 1024.0 / duration_secs);
        */
    }
}

fn main() {
    do_work().unwrap();
}

fn do_work() -> Result<(), Box<Error>> {
    let mut recover = Recover::new("/dev/sda", "./test.img", "./drive.map")?;
    recover.do_phase()?;
    Ok(())
}

struct ReadIter {
    start: u64,
    end: u64,
    physical_block_size: usize,
}

impl Iterator for ReadIter {
    type Item = Range<u64>;

    fn next(&mut self) -> Option<Self::Item> {
        let physical_block_size = self.physical_block_size as u64;
        if self.start < self.end {
            let read_end = cmp::min(((self.start + physical_block_size) / physical_block_size) * physical_block_size, self.end);
            let result = self.start..read_end;
            self.start = read_end;
            Some(result)
        } else {
            None
        }
    }
}

fn range_to_reads(range: &Range<u64>, block: &BlockDevice) -> ReadIter {
    let sector_size = block.get_sector_size();
    let physical_block_size = block.get_block_size_physical();
    let size_bytes = block.get_size_bytes();
    assert!(physical_block_size % sector_size == 0);

    let sector_size_u64 = sector_size as u64;
    let start = (range.start / sector_size_u64) * sector_size_u64;
    let end = cmp::max(((range.end + sector_size_u64 - 1) / sector_size_u64) * sector_size_u64, size_bytes);
    ReadIter {
        start: start,
        end: end,
        physical_block_size: physical_block_size,
    }
}
