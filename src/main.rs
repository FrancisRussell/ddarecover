extern crate ddarecover;

use ddarecover::block::{BlockDevice, Buffer, Request};
use ddarecover::map_file::{MapFile, SectorState};
use ddarecover::out_file::OutFile;
use std::cmp;
use std::error::Error;
use std::fs::File;
use std::ops::Range;
use std::time::Instant;
use std::path::Path;
use std::io::{Seek, SeekFrom, Write};

const NUM_BUFFERS: usize = 256;
const READ_BATCH_SIZE: usize = 256;
const SYNC_INTERVAL: usize = 60;

fn main() {
    do_work().unwrap();
}

fn do_work() -> Result<(), Box<Error>> {
    let mut block = BlockDevice::open("/dev/sda").expect("Unable to open block device");
    let map_path = Path::new("./drive.map");
    let mut map = if map_path.exists() {
        let map_file = File::open(map_path).expect("Unable to open existing map file");
        MapFile::read_from_stream(map_file).expect("Error reading map file")
    } else {
        let map = MapFile::new(block.get_size_bytes());
        map.write_to_path(map_path).expect("Unable to create new map file");
        map
    };
    assert_eq!(map.get_size_bytes(), block.get_size_bytes(), "Mismatch between device size and map file");
    let out_file_path = Path::new("./test.img");
    let mut out_file = OutFile::open(out_file_path, block.get_size_bytes()).expect("Unable to open output file");

    let sectors_per_buffer = block.get_block_size_physical() / block.get_sector_size();
    let mut buffers: Vec<Buffer> = Vec::new();
    for _ in 0..NUM_BUFFERS {
        let buffer = block.create_io_buffer(sectors_per_buffer);
        buffers.push(buffer);
    }

    let mut recovered: u64 = 0;
    let mut failed: u64 = 0;
    let mut phase_complete = false;
    let start_time = Instant::now();
    let mut last_sync = Instant::now();
    println!("Starting...");
    while !phase_complete {
        let reads: Vec<Range<u64>> =
            (&map).iter_range(map.get_pos()..map.get_size())
            .filter(|r| r.tag == SectorState::Untried)
            .flat_map(|r| range_to_reads(&r.as_range(), &block))
            .take(READ_BATCH_SIZE).collect();

        phase_complete = reads.is_empty();
        for read in reads {
            if block.requests_avail() > 0 && !buffers.is_empty() {
                let request = Request::new(read.start, read.end - read.start, buffers.pop().unwrap());
                block.submit_request(request)?;
                map.set_pos(read.end);
            } else {
                let request = block.get_completed_request()?;
                if request.result > 0 {
                    recovered += request.result as u64;
                    out_file.seek(SeekFrom::Start(request.offset))?;
                    out_file.write_all(request.get_data())?;
                    map.put(request.offset..(request.offset + request.result as u64), SectorState::Rescued);
                } else {
                    failed += request.size;
                    map.put(request.offset..(request.offset + request.size), SectorState::Bad);
                };
                println!("Request: {:?}", request);
                buffers.push(request.reclaim_buffer());

                let now = Instant::now();
                if now.duration_since(last_sync.clone()).as_secs() >= SYNC_INTERVAL as u64 {
                    println!("Performing sync...");
                    out_file.sync()?;
                    map.write_to_path(&map_path)?;
                    last_sync = now;
                    println!("Sync complete.");

                    let end_time = Instant::now();
                    let duration = end_time.duration_since(start_time);
                    let duration_secs = (duration.as_secs() as f64) + (duration.subsec_nanos() as f64 * 1e-9);
                    println!("Recovered: {} bytes, failed: {} bytes, duration: {:.2} seconds.", recovered, failed, duration_secs);
                    println!("Recovered at {:.1} KiB/s, failed at {:.1} KiB/s, total: {:.1} KiB/s.",
                             recovered as f64 / 1024.0 / duration_secs,
                             failed as f64 / 1024.0 / duration_secs,
                             (recovered + failed) as f64 / 1024.0 / duration_secs);
                }
            }
        }
    }
    let end_time = Instant::now();
    let duration = end_time.duration_since(start_time);
    let duration_secs = (duration.as_secs() as f64) + (duration.subsec_nanos() as f64 * 1e-9);
    println!("Recovered: {} bytes, failed: {} bytes, duration: {:.2} seconds.", recovered, failed, duration_secs);
    println!("Recovered at {:.1} KiB/s, failed at {:.1} KiB/s, total: {:.1} KiB/s.",
             recovered as f64 / 1024.0 / duration_secs,
             failed as f64 / 1024.0 / duration_secs,
             (recovered + failed) as f64 / 1024.0 / duration_secs);

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
