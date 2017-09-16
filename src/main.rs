extern crate ansi_escapes;
extern crate ctrlc;
extern crate ddarecover;
extern crate getopts;
extern crate nix;

use ddarecover::block::{BlockDevice, Buffer, Request};
use ddarecover::map_file::{MapFile, SectorState};
use ddarecover::out_file::OutFile;
use getopts::Options;
use std::env;
use std::cmp;
use std::collections::{VecDeque, HashMap};
use std::error::Error;
use std::fs::File;
use std::io::{self, Seek, SeekFrom, Write};
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

const READ_BATCH_SIZE: usize = 128;
const SYNC_INTERVAL: usize = 5 * 60;
const REFRESH_INTERVAL: f32 = 0.5;

#[derive(Debug)]
struct Stats {
    good: u64,
    bad: u64,
    requests: u64,
}

impl Stats {
    pub fn new() -> Stats {
        Stats {
            good: 0,
            bad: 0,
            requests: 0,
        }
    }
}

#[derive(Debug)]
struct Recover {
    block: BlockDevice,
    map_file: MapFile,
    map_file_path: PathBuf,
    out_file: OutFile,
    start: Instant,
    last_sync: Instant,
    last_success: Option<Instant>,
    last_print: Option<Instant>,
    histogram: HashMap<SectorState, u64>,
    buffer_cache: Vec<Buffer>,
    should_run_flag: Arc<AtomicBool>,
    stats: Stats,
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
        let should_run_flag = Arc::new(AtomicBool::new(true));
        let result = Recover {
            block: block,
            map_file: map,
            map_file_path: map_path.to_path_buf(),
            out_file: outfile,
            start: Instant::now(),
            last_sync: Instant::now(),
            last_success: None,
            last_print: None,
            histogram: histogram,
            buffer_cache: Vec::new(),
            should_run_flag: should_run_flag.clone(),
            stats: Stats::new(),
        };
        ctrlc::set_handler(move || {
            should_run_flag.store(false, Ordering::SeqCst);
        }).expect("Error setting Ctrl-C handler");
        Ok(result)
    }

    fn should_run(&self) -> bool {
        self.should_run_flag.load(Ordering::SeqCst)
    }

    fn do_sync(&mut self) -> io::Result<()> {
        self.out_file.sync()?;
        self.map_file.write_to_path(&self.map_file_path)?;
        self.last_sync = Instant::now();
        Ok(())
    }

    fn update_status(&mut self) {
        let now = Instant::now();
        match self.last_print {
            None => {
                self.print_status(false);
                self.last_print = Some(now);
            },
            Some(previous) => {
                let duration = now.duration_since(previous);
                let seconds = duration.as_secs() as f32 + duration.subsec_nanos() as f32 * 1e-9;
                if seconds > REFRESH_INTERVAL {
                    self.print_status(true);
                    self.last_print = Some(now);
                }
            },
        }
    }

    fn print_status(&self, overwrite: bool) {
        if overwrite {
            print!("{}{}", ansi_escapes::CursorLeft, ansi_escapes::CursorUp(7));
        }
        println!("Press Ctrl+C to exit.{}\n{}",ansi_escapes::EraseEndLine, ansi_escapes::EraseEndLine);
        println!("{:>13}: {:19}{}", "Phase",
                 format!("{} (pass {})", self.map_file.get_phase().name(), self.map_file.get_pass()),
                 ansi_escapes::EraseEndLine);
        println!("{:>13}: {:19} {:>13}: {:19} {:>13}: {:19}{}",
                 "ipos", self.format_bytes_with_percentage(self.map_file.get_pos()),
                 "rescued", self.get_histogram_value_formatted(SectorState::Rescued),
                 "bad", self.get_histogram_value_formatted(SectorState::Bad),
                 ansi_escapes::EraseEndLine);

        println!("{:>13}: {:19} {:>13}: {:19} {:>13}: {:19}{}",
                 "non-tried", self.get_histogram_value_formatted(SectorState::Untried),
                 "non-trimmed", self.get_histogram_value_formatted(SectorState::Untrimmed),
                 "non-scraped", self.get_histogram_value_formatted(SectorState::Unscraped),
                 ansi_escapes::EraseEndLine);

        let now = Instant::now();
        let elapsed = now.duration_since(self.start).as_secs();

        let good = self.stats.good;
        let bad = self.stats.bad;
        let total = self.stats.good + self.stats.bad;
        let bytes_remaining = self.get_histogram_value(SectorState::Untried)
            + self.get_histogram_value(SectorState::Untrimmed)
            + self.get_histogram_value(SectorState::Unscraped);
        let seconds_remaining = if total > 0 {
            bytes_remaining * elapsed / total
        } else {
            0
        };

        println!("{:>13}: {:19} {:>13}: {:19} {:>13}: {:19}",
                 "read rate", self.format_rate(good, elapsed),
                 "error rate", self.format_rate(bad, elapsed),
                 "total rate", self.format_rate(total, elapsed));

        let last_success = match self.last_success {
            None => String::from("never"),
            Some(time) => format!("{} ago", self.format_seconds(now.duration_since(time).as_secs())),
        };
        println!("{:>13}: {:19} {:>13}: {:19} {:>13}: {:19}",
                 "run time", self.format_seconds(elapsed),
                 "last success", last_success,
                 "remaining", self.format_seconds(seconds_remaining));
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
        format!("{:.0} {}", res_bytes, res_unit)
    }

    fn format_bytes_with_percentage(&self, bytes: u64) -> String {
        let percentage = (bytes as f64) * 100.0 / (self.map_file.get_size() as f64);
        format!("{} ({:.1}%)", self.format_bytes(bytes), percentage)
    }

    fn format_rate(&self, bytes: u64, seconds: u64) -> String {
        if bytes == 0 || seconds > 0 {
            let rate = if seconds > 0 {
                bytes / seconds
            } else {
                0
            };
            format!("{}/s", self.format_bytes(rate))
        } else {
            String::from("inf")
        }
    }

    fn format_seconds(&self, seconds: u64) -> String {
        let mut value = seconds;
        let mut elements = Vec::new();
        for &(unit, multiple) in [("s", 60), ("m", 60), ("h", 24), ("d", usize::max_value())].iter() {
            let multiple = multiple as u64;
            elements.push(format!("{}{}", value % multiple, unit));
            value /= multiple;

            if value == 0 {
                break;
            }
        }
        let max_time_components = 2;
        elements.reverse();
        elements.truncate(max_time_components);
        elements.join(" ")
    }

    fn get_histogram_value_formatted(&self, state: SectorState) -> String {
        self.format_bytes_with_percentage(self.get_histogram_value(state))
    }


    fn get_histogram_value(&self, state: SectorState) -> u64 {
        *self.histogram.get(&state).unwrap_or(&0)
    }

    fn update_histogram(&mut self, bytes: u64, from: SectorState, to: SectorState) {
        *self.histogram.entry(from).or_insert(0) -= bytes;
        *self.histogram.entry(to).or_insert(0) += bytes;
    }

    fn do_phase(&mut self) -> Result<(), Box<Error>> {
        self.map_file.set_pass(1);
        match self.map_file.get_phase().target_sectors() {
            Some(phase_target) => {
                while self.get_histogram_value(phase_target) > 0 && self.should_run() {
                    self.do_pass(&phase_target)?;
                    if self.is_pass_complete() {
                        self.map_file.set_pos(0);
                        self.map_file.next_pass();
                    }
                }
            },
            None => {},
        }
        Ok(())
    }

    fn do_phases(&mut self) -> Result<(), Box<Error>> {
        self.update_status();
        let mut finished = false;
        while !finished && self.should_run() {
            if self.is_phase_complete() {
                let current_phase = self.map_file.get_phase();
                match current_phase.next() {
                    Some(phase) => {
                        self.map_file.set_phase(&phase);
                    },
                    None => finished = true,
                }
            } else {
                self.do_phase()?;
            }
        }
        self.do_sync()?;
        Ok(())
    }

    fn is_pass_complete(&self) -> bool {
        let current_phase = self.map_file.get_phase();
        match current_phase.target_sectors() {
            Some(phase_target) => {
                (&self.map_file).iter_range(self.map_file.get_pos()..self.map_file.get_size())
                .filter(|r| r.tag == phase_target).next().is_none()
            },
            None => true,
        }
    }

    fn is_phase_complete(&self) -> bool {
        let current_phase = self.map_file.get_phase();
        match current_phase.target_sectors() {
            Some(phase_target) => {
                (&self.map_file).iter_range(0..self.map_file.get_size())
                .filter(|r| r.tag == phase_target).next().is_none()
            },
            None => true,
        }
    }

    fn get_cleared_buffer(&mut self) -> Buffer {
        let sectors_per_buffer = self.block.get_block_size_physical() / self.block.get_sector_size();
        let mut buffer = match self.buffer_cache.pop() {
            Some(buffer) => buffer,
            None => self.block.create_io_buffer(sectors_per_buffer),
        };
        buffer.clear();
        buffer
    }

    fn recycle_buffer(&mut self, buffer: Buffer) {
        self.buffer_cache.push(buffer)
    }

    fn try_drain_request(&mut self, phase_target: &SectorState) -> Result<(), Box<Error>> {
        if self.block.requests_pending() > 0 {
            let request = match self.block.get_completed_request() {
                Ok(r) => r,
                Err(nix::Error::Sys(nix::Errno::EINTR)) => return Ok(()),
                Err(err) => return Err(Box::new(err)),
            };
            self.stats.requests += 1;
            if request.result > 0 {
                let request_result = request.result as u64;
                if !request.is_data_zeros() {
                    self.out_file.seek(SeekFrom::Start(request.offset))?;
                    self.out_file.write_all(request.get_data())?;
                }
                self.update_histogram(request_result, *phase_target, SectorState::Rescued);
                self.map_file.put(request.offset..(request.offset + request_result), SectorState::Rescued);
                self.last_success = Some(Instant::now());
                self.stats.good += request_result;
            } else {
                self.update_histogram(request.size as u64, *phase_target, SectorState::Bad);
                self.map_file.put(request.offset..(request.offset + request.size), SectorState::Bad);
                self.stats.bad += request.size;
            };
            self.recycle_buffer(request.reclaim_buffer());
        }
        Ok(())
    }

    fn do_pass(&mut self, phase_target: &SectorState) -> Result<(), Box<Error>> {
        let mut pass_complete = false;
        while !pass_complete && self.should_run() {
            let mut reads: VecDeque<Range<u64>> =
                (&self.map_file).iter_range(self.map_file.get_pos()..self.map_file.get_size())
                .filter(|r| r.tag == *phase_target)
                .flat_map(|r| range_to_reads(&r.as_range(), &self.block))
                .take(READ_BATCH_SIZE).collect();

            pass_complete = reads.is_empty();
            while !reads.is_empty() && self.should_run() {
                if self.block.requests_avail() > 0 {
                    let read = reads.pop_front().unwrap();
                    let buffer = self.get_cleared_buffer();
                    let request = Request::new(read.start, read.end - read.start, buffer);
                    self.block.submit_request(request)?;
                    let current_start = self.map_file.get_pos();
                    self.map_file.set_pos(cmp::max(current_start, read.end));
                }
                if self.block.requests_avail() == 0 {
                    self.try_drain_request(phase_target)?;
                    self.update_status();
                }
                let now = Instant::now();
                if now.duration_since(self.last_sync.clone()).as_secs() >= SYNC_INTERVAL as u64 {
                    self.do_sync()?;
                }
            }
        }
        while self.block.requests_pending() > 0 {
            self.try_drain_request(phase_target)?;
            self.update_status();
        }
        Ok(())
    }
}

fn print_usage(program: &str, opts: &Options) {
    println!("{}", opts.usage(&format!("Usage: {} -i input_device -o output_file -m map_file", program)));
}

fn main() {
    do_work().unwrap();
}

fn do_work() -> Result<(), Box<Error>> {
    let args : Vec<String> = env::args().collect();
    let program = &args[0];

    let mut opts = Options::new();
    opts.optflag("h", "help", "Show usage.");
    opts.reqopt("i", "input", "Input device (required).", "FILE");
    opts.reqopt("o", "output", "Output file (required).", "FILE");
    opts.reqopt("m", "map", "Map file (required).", "FILE");

    let matches = match opts.parse(&args[1..]) {
        Ok(m) => { m }
        Err(e) => {
            println!("Error: {}", e.description());
            print_usage(&program, &opts);
            return Ok(())
        },
    };

    let free_args = &matches.free;
    let needed_args = 0;
    if matches.opt_present("h") || free_args.len() != needed_args {
        print_usage(&program, &opts);
        return Ok(());
    }

    let input = matches.opt_str("i").unwrap();
    let output = matches.opt_str("o").unwrap();
    let map = matches.opt_str("m").unwrap();

    let mut recover = Recover::new(input.as_str(), output.as_str(), map.as_str())?;
    recover.do_phases()?;
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
    let end = cmp::min(((range.end + sector_size_u64 - 1) / sector_size_u64) * sector_size_u64, size_bytes);
    ReadIter {
        start: start,
        end: end,
        physical_block_size: physical_block_size,
    }
}
