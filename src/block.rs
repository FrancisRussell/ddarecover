use aio_abi::{self, aio_context_t, io_event, iocb};
use libc::{self, c_int, c_uint, c_void};
use nix;
use num::cast;
use std::collections::BTreeMap;
use std::error::Error;
use std::ptr;
use std::slice;
use std::fs::{File, OpenOptions};
use std::os::unix::fs::OpenOptionsExt;
use std::os::unix::io::AsRawFd;

const MAX_EVENTS: usize = 32;

// Meaning of block/sector sizes:
//
// physical block size - true physical block size of hardware
// sector size - minimum size of reads and writes supported by hardware (potentially smaller than
// physical block size)
// (logical) block size - a value that may actually vary across partitions, depending on whether a filesystem
// is mounted
//
// Alignment for O_DIRECT:
//
// In 2.4: buffer offset and transfer size must be multiples of logical block size.
// In 2.5: buffer offset and transfer size must be multiples of sector size.
//
// See https://lists.gt.net/linux/kernel/350775

#[derive(Debug)]
pub struct BlockDevice {
    block_size_physical: usize,
    context: aio_context_t,
    file: File,
    iocbs: Vec<(bool, iocb)>,
    requests: BTreeMap<usize, Request>,
    sector_size: usize,
    size_bytes: u64,
}

mod ioctl {
    use libc::c_uint;
    pub const BLK: c_uint = 0x12;
    pub const SSZGET: c_uint = 104;
    pub const GETSIZE64: c_uint = 114;
    pub const PBSZGET: c_uint = 123;
}

#[derive(Debug)]
pub struct Request {
    pub offset: u64,
    pub size: u64,
    pub buffer: Buffer,
    pub result: isize,
}

impl Request {
    pub fn new(offset: u64, size: u64, buffer: Buffer) -> Request {
        assert!(cast::<usize, u64>(buffer.size).unwrap() >= size, "Supplied buffer is too small");
        Request {
            offset: offset,
            size: size,
            buffer: buffer,
            result: -1,
        }
    }

    pub fn get_data(&self) -> &[u8] {
        if self.result < 0 {
            &[]
        } else {
            &self.buffer.as_slice()[0..cast::<isize, usize>(self.result).unwrap()]
        }
    }

    pub fn reclaim_buffer(self) -> Buffer {
        self.buffer
    }

    pub fn is_data_zeros(&self) -> bool {
        for c in self.get_data() {
            if *c != 0 {
                return false;
            }
        }
        return true;
    }
}

#[derive(Debug)]
pub struct Buffer {
    alignment: usize,
    size: usize,
    data: *mut c_void,
}

impl Buffer {
    pub fn allocate_aligned(size: usize, alignment: usize) -> Buffer {
        let buffer = unsafe { libc::memalign(alignment, size) };
        if buffer.is_null() {
            panic!("Buffer allocation failed!");
        }
        Buffer {
            alignment: alignment,
            size: size,
            data: buffer,
        }
    }

    pub fn len(&self) -> usize {
        self.size
    }

    pub fn as_slice(&self) -> &[u8] {
        unsafe {
            let data = self.data as *const u8;
            slice::from_raw_parts(data, self.size)
        }
    }

    pub fn clear(&mut self) {
        unsafe {
            libc::memset(self.data, 0, self.size);
        }
    }
}

impl Drop for Buffer {
    fn drop(&mut self) {
        unsafe {
            libc::free(self.data);
        }
    }
}

impl BlockDevice {
    pub fn open(path: &str) -> Result<BlockDevice, Box<Error>> {
        let file = OpenOptions::new()
            .read(true)
            .write(false)
            .custom_flags(libc::O_DIRECT)
            .open(path)?;
        let fd = file.as_raw_fd();
        let block_size_physical = Self::query_block_size_physical(fd)?;
        let sector_size = Self::query_sector_size(fd)?;
        let size_bytes = Self::query_size_bytes(fd)?;
        let iocbs = vec![(false, iocb::new()); MAX_EVENTS];
        let mut context: aio_context_t = ptr::null_mut();
        if unsafe { aio_abi::io_setup(cast::<usize, i32>(iocbs.len()).unwrap(), &mut context as *mut aio_context_t) } == -1 {
            return Err(Box::new(Self::fail_errno()));
        }

        let result = BlockDevice {
            context: context,
            block_size_physical: cast::<u32, usize>(block_size_physical).unwrap(),
            file: file,
            iocbs: iocbs,
            requests: BTreeMap::new(),
            size_bytes: size_bytes,
            sector_size: cast::<u32, usize>(sector_size).unwrap(),
        };
        Ok(result)
    }

    fn get_fd(&self) -> c_int {
        self.file.as_raw_fd()
    }

    fn query_block_size_physical(fd: c_int) -> Result<c_uint, nix::Error> {
        let mut block_size_physical: c_uint = 0;
        let ioc = ioc!(nix::sys::ioctl::NONE, ioctl::BLK, ioctl::PBSZGET, 0);
        if unsafe { libc::ioctl(fd, ioc, &mut block_size_physical as *mut c_uint) } == -1 {
            Err(Self::fail_errno())
        } else {
            Ok(block_size_physical)
        }
    }

    fn query_sector_size(fd: c_int) -> Result<c_uint, nix::Error> {
        let mut sector_size: c_uint = 0;
        let ioc = ioc!(nix::sys::ioctl::NONE, ioctl::BLK, ioctl::SSZGET, 0);
        if unsafe { libc::ioctl(fd, ioc, &mut sector_size as *mut c_uint) } == -1 {
            Err(Self::fail_errno())
        } else {
            Ok(sector_size)
        }
    }

    fn query_size_bytes(fd: c_int) -> Result<u64, nix::Error> {
        let mut size_bytes: u64 = 0;
        let ioc = ior!(ioctl::BLK, ioctl::GETSIZE64, 8);
        if unsafe { libc::ioctl(fd, ioc, &mut size_bytes as *mut u64) } == -1 {
            Err(Self::fail_errno())
        } else {
            Ok(size_bytes)
        }
    }

    pub fn submit_request(&mut self, req: Request) -> Result<(), nix::Error> {
        assert!(self.requests_avail() > 0);
        let fd = self.get_fd();
        let slot = self.find_slot();
        let iocb = &mut self.iocbs[slot];
        iocb.0 = true;
        aio_abi::io_prep_pread(&mut iocb.1, fd, req.buffer.data, req.size, cast::<u64, i64>(req.offset).unwrap());
        iocb.1.data = cast::<usize, u64>(slot).unwrap();
        let iocb_ptr = &mut iocb.1 as *mut iocb;
        let mut iocb_list = [iocb_ptr];
        let res = unsafe {
            aio_abi::io_submit(self.context, cast::<usize, i64>(iocb_list.len()).unwrap(), &mut iocb_list[0] as *mut *mut iocb)
        };
        if res < 0 {
            let errno = nix::Errno::from_i32(-res);
            Err(nix::Error::Sys(errno))
        } else {
            self.requests.insert(slot, req);
            Ok(())
        }
    }

    pub fn get_completed_request(&mut self) -> Result<Request, nix::Error> {
        assert!(self.requests_pending() > 0);
        let mut event = io_event::new();
        let res = unsafe {
            aio_abi::io_getevents(self.context, 1, 1, &mut event as *mut io_event, ptr::null_mut())
        };
        if res < 0 {
            let errno = nix::Errno::from_i32(-res);
            Err(nix::Error::Sys(errno))
        } else {
            let slot = cast::<u64, usize>(event.data).unwrap();
            let  &mut (ref mut used, _) = self.iocbs.get_mut(slot).expect("iocb maps to invalid slot");
            *used = false;
            let mut req = self.requests.remove(&slot).unwrap();
            req.result = cast::<i64, isize>(event.res).unwrap();
            return Ok(req);
        }
    }

    fn find_slot(&self) -> usize {
        for (idx, &(used, _)) in self.iocbs.iter().enumerate() {
            if !used {
                return idx;
            }
        }
        panic!("No free slot");
    }

    pub fn get_block_size_physical(&self) -> usize {
        self.block_size_physical
    }

    pub fn get_sector_size(&self) -> usize {
        self.sector_size
    }

    pub fn get_size_bytes(&self) -> u64 {
        self.size_bytes
    }

    fn fail_errno() -> nix::Error {
        nix::Error::last()
    }

    pub fn max_requests(&self) -> usize {
        self.iocbs.len()
    }

    pub fn requests_avail(&self) -> usize {
        self.iocbs.iter().filter(|r| !r.0).count()
    }

    pub fn requests_pending(&self) -> usize {
        self.iocbs.iter().filter(|r| r.0).count()
    }

    pub fn create_io_buffer(&self, sectors: usize) -> Buffer {
        let bytes = sectors * self.sector_size;
        Buffer::allocate_aligned(bytes, self.sector_size)
    }
}

impl Drop for BlockDevice {
    fn drop(&mut self) {
        unsafe {
            aio_abi::io_destroy(self.context);
        };
    }
}
