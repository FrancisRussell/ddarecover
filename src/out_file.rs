use std::cmp;
use std::fs::{File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::ops::Range;
use std::path::Path;

#[derive(Debug)]
pub struct OutFile {
    file: File,
}

impl OutFile {
    pub fn open(path: &Path, size_bytes: u64) -> io::Result<OutFile> {
        let file = if !path.exists() {
            let file = OpenOptions::new()
                .create_new(true)
                .read(true)
                .write(true)
                .open(path)?;
            file.set_len(size_bytes)?;
            file
        } else {
            OpenOptions::new()
                .read(true)
                .write(true)
                .create(false)
                .truncate(false)
                .open(path)?
        };

        let meta = file.metadata()?;
        if meta.len() != size_bytes {
            panic!("Output file size does not match required length");
        }

        let res = OutFile {
            file: file,
        };
        Ok(res)
    }

    pub fn sync(&mut self) -> io::Result<()> {
        self.file.flush()?;
        self.file.sync_all()
    }

    pub fn is_range_zero(&mut self, range: Range<u64>) -> io::Result<bool> {
        let mut data = vec![0u8; 65536];
        self.seek(SeekFrom::Start(range.start))?;
        let mut remaining = range.end - range.start;
        while remaining > 0 {
            let read_size = cmp::min(remaining, data.len() as u64);
            let data = &mut data[0..read_size as usize];
            self.read_exact(&mut data[..])?;
            for value in &data[..] {
                if *value != 0 {
                    return Ok(false);
                }
            }
            remaining -= read_size;
        }
        Ok(true)
    }
}

impl Read for OutFile {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.file.read(buf)
    }
}

impl Write for OutFile {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.file.write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.file.flush()
    }
}

impl Seek for OutFile {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        self.file.seek(pos)
    }
}

impl Drop for OutFile {
    fn drop(&mut self) {
        self.sync().unwrap()
    }
}
