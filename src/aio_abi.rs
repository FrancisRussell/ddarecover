use libc::{c_int, c_long, c_void, int16_t, int64_t, timespec, uint16_t, uint32_t, uint64_t};

#[allow(non_camel_case_types)]
pub enum aio_context {}

#[allow(non_camel_case_types)]
pub type aio_context_t = *mut aio_context;

#[repr(C)]
#[allow(non_camel_case_types)]
#[derive(Clone, Copy, Debug)]
pub struct iocb {
    pub data: uint64_t,
    pub key: uint32_t,
    pub reserved1: uint32_t,

    pub lio_opcode: uint16_t,
    pub reqprio: int16_t,
    pub fildes: uint32_t,

    pub buf: uint64_t,
    pub nbytes: uint64_t,
    pub offset: int64_t,

    pub reserved2: uint64_t,
    pub flags: uint32_t,

    pub resfd: uint32_t
}

#[repr(C)]
#[allow(non_camel_case_types)]
#[derive(Clone, Copy, Debug)]
pub struct io_event {
    pub data: uint64_t,
    pub obj: uint64_t,
    pub res: int64_t,
    pub res2: int64_t,
}

impl io_event {
    pub fn new() -> io_event {
        io_event{
            data: 0,
            obj: 0,
            res: 0,
            res2: 0,
        }
    }
}

#[allow(non_camel_case_types)]
pub enum iocb_cmd {
    IOCB_CMD_PREAD = 0,
    IOCB_CMD_PWRITE = 1,
    IOCB_CMD_FSYNC = 2,
    IOCB_CMD_FDSYNC = 3,
    IOCB_CMD_PREADX = 4,
    IOCB_CMD_POLL = 5,
    IOCB_CMD_NOOP = 6,
    IOCB_CMD_PREADV = 7,
    IOCB_CMD_PWRITEV = 8,
}

impl iocb {
    pub fn new() -> iocb {
        iocb {
            data: 0,
            key: 0,
            reserved1: 0,
            lio_opcode: iocb_cmd::IOCB_CMD_NOOP as u16,
            reqprio: 0,
            fildes: 0,
            buf: 0,
            nbytes: 0,
            offset: 0,
            reserved2: 0,
            flags: 0,
            resfd: 0,
        }
    }
}

pub fn io_prep_pread(iocb: &mut iocb, fd: uint32_t, buf: *mut c_void, count: uint64_t, offset: int64_t) {
    iocb.fildes = fd;
    iocb.lio_opcode = iocb_cmd::IOCB_CMD_PREAD as u16;
    iocb.reqprio = 0;
    iocb.buf = buf as u64;
    iocb.nbytes = count;
    iocb.offset = offset;
}

#[link(name = "aio")]
extern "C" {
    pub fn io_setup(maxevents: c_int, ctxp: *mut aio_context_t) -> c_int;
    pub fn io_submit(ctx_id: aio_context_t, nr: c_long, iocbpp: *mut *mut iocb) -> c_int;
    pub fn io_getevents(ctx_id: aio_context_t, min_nr: c_long, nr: c_long, events: *mut io_event, timeout: *mut timespec) -> c_int;
    pub fn io_destroy(ctx_id: aio_context_t) -> c_int;
}
