extern crate ddarecover;

use std::fs::File;
use ddarecover::map_file::MapFile;
use ddarecover::block::BlockDevice;
use std::error::Error;

fn main() {
    do_work().unwrap();
    println!("Done!!!");
}

fn do_work() -> Result<(), Box<Error>> {
    let map_file = File::open("./drive.map").expect("Unable to open map file");
    let map = MapFile::read_from_stream(map_file).expect("Error reading map file");
    let block = BlockDevice::open("/dev/sda").expect("Unable to open block device");
    assert_eq!(map.get_size_bytes(), block.get_size_bytes()?, "Mismatch between device size and map file");
    println!("{:?}", block);
    Ok(())
}
