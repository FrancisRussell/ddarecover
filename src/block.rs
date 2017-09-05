use libc::{self, c_int, c_uint};
use nix;
use std::ffi::CString;
use std::error::Error;
use libaio::{self, aio_context_t};
use std::ptr;

#[derive(Debug)]
pub struct BlockDevice {
    fd: c_int,
    block_size: usize,
    num_blocks: u64,
    context: aio_context_t,
}

mod ioctl {
    use libc::c_uint;
    pub const BLK: c_uint = 0x12;
    pub const GETSIZE64: c_uint = 114;
    pub const PBSZGET: c_uint = 123;
}

const MAX_EVENTS: c_int = 20;

impl BlockDevice {
    pub fn open(path: &str) -> Result<BlockDevice, Box<Error>> {
        let path = CString::new(path)?;
        let fd = unsafe { libc::open(path.as_ptr(), libc::O_RDONLY | libc::O_DIRECT) };
        if fd == -1 {
            return Self::fail_errno();
        }
        let mut block_size: c_uint = 0;
        let ioc = ioc!(nix::sys::ioctl::NONE, ioctl::BLK, ioctl::PBSZGET, 0);
        if unsafe { libc::ioctl(fd, ioc, &mut block_size as *mut c_uint) } == -1 {
            return Self::fail_errno();
        }
        let ioc = ior!(ioctl::BLK, ioctl::GETSIZE64, 8);
        let mut size_bytes: u64 = 0;
        if unsafe { libc::ioctl(fd, ioc, &mut size_bytes as *mut u64) } == -1 {
            return Self::fail_errno();
        }
        let num_blocks = size_bytes / (block_size as u64);
        assert_eq!(size_bytes % (block_size as u64), 0, "Device size is not multiple of block size!");

        let mut context: aio_context_t = ptr::null_mut();
        if unsafe { libaio::io_setup(MAX_EVENTS, &mut context as *mut aio_context_t) } == -1 {
            return Self::fail_errno();
        }

        let result = BlockDevice {
            fd: fd,
            block_size: block_size as usize,
            num_blocks: num_blocks,
            context: context,
        };
        Ok(result)
    }

    pub fn get_block_size(&self) -> Result<usize, nix::Error> {
        Ok(self.block_size)
    }

    pub fn get_size_blocks(&self) -> Result<u64, nix::Error> {
        Ok(self.num_blocks)
    }

    pub fn get_size_bytes(&self) -> Result<u64, nix::Error> {
        Ok(self.num_blocks as u64 * self.block_size as u64)
    }

    fn fail_errno<T>() -> Result<T, Box<Error>> {
        Err(Box::new(nix::Error::last()))
    }
}

impl Drop for BlockDevice {
    fn drop(&mut self) {
        unsafe {
            libaio::io_destroy(self.context);
            libc::close(self.fd)
        };
    }
}
