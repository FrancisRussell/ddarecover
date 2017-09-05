extern crate ddarecover;

use std::fs::File;
use std::io;
use ddarecover::map_file::MapFile;
use ddarecover::block::BlockDevice;
use std::error::Error;

fn main() {
    do_work().unwrap();
    println!("Done!!!");
}

fn do_work() -> Result<(), Box<Error>> {
    let map_file = File::open("./drive.map")?;
    let mut map = MapFile::read_from_stream(map_file)?;
    let mut block = BlockDevice::open("/dev/sda")?;
    println!("{:?}", block);
    Ok(())
}
