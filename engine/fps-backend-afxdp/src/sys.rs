// SPDX-License-Identifier: GPL-3.0-only
//! The system calls of AF_XDP and BPF.
//!
//! No crate in the dependency policy wraps the AF_XDP socket options or the
//! `bpf` call, and `libc` has only some of the constants, so this module
//! declares the structures of `linux/if_xdp.h` and `linux/bpf.h` that it
//! needs and calls through `libc`, as decision D12 item 4 allows. It and
//! `ring` are the only modules of the crate with `unsafe` code, and every
//! block carries a `SAFETY` comment. Each function that returns a value is
//! tested against a real socket or by a unit test.

use std::ffi::CString;
use std::io;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::ptr::NonNull;

/// `SOL_XDP`, the socket level of AF_XDP options.
const SOL_XDP: libc::c_int = 283;
const AF_XDP: libc::c_int = 44;

const XDP_MMAP_OFFSETS: libc::c_int = 1;
const XDP_RX_RING: libc::c_int = 2;
const XDP_TX_RING: libc::c_int = 3;
const XDP_UMEM_REG: libc::c_int = 4;
const XDP_UMEM_FILL_RING: libc::c_int = 5;
const XDP_UMEM_COMPLETION_RING: libc::c_int = 6;
const XDP_STATISTICS: libc::c_int = 7;
const XDP_OPTIONS: libc::c_int = 8;

/// Bind flag: the kernel tells the application through a ring flag when it
/// needs a system call to make progress.
const XDP_USE_NEED_WAKEUP: u16 = 1 << 3;
const XDP_COPY: u16 = 1 << 1;
const XDP_ZEROCOPY: u16 = 1 << 2;
/// `xdp_options.flags` bit that a zero copy socket sets.
const XDP_OPTIONS_ZEROCOPY: u32 = 1;
/// Ring flag: the application must wake the kernel.
const XDP_RING_NEED_WAKEUP: u32 = 1;

const XDP_PGOFF_RX_RING: u64 = 0;
const XDP_PGOFF_TX_RING: u64 = 0x8000_0000;
const XDP_UMEM_PGOFF_FILL_RING: u64 = 0x1_0000_0000;
const XDP_UMEM_PGOFF_COMPLETION_RING: u64 = 0x1_8000_0000;

/// Offsets of one ring inside its mapping, `struct xdp_ring_offset`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct RingOffset {
    pub producer: u64,
    pub consumer: u64,
    pub desc: u64,
    pub flags: u64,
}

/// `struct xdp_mmap_offsets`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct MmapOffsets {
    pub rx: RingOffset,
    pub tx: RingOffset,
    pub fill: RingOffset,
    pub completion: RingOffset,
}

/// `struct xdp_desc`, one entry of the receive and transmit rings.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct XdpDesc {
    pub addr: u64,
    pub len: u32,
    pub options: u32,
}

/// The first version of `struct xdp_umem_reg`, which every kernel with
/// AF_XDP accepts. Later versions append flags that this crate leaves at 0.
#[repr(C)]
struct UmemReg {
    addr: u64,
    len: u64,
    chunk_size: u32,
    headroom: u32,
}

/// `struct sockaddr_xdp`.
#[repr(C)]
struct SockaddrXdp {
    family: u16,
    flags: u16,
    ifindex: u32,
    queue_id: u32,
    shared_umem_fd: u32,
}

/// `struct xdp_statistics` of kernel 5.9 and later. Older kernels fill only
/// the first three fields.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct KernelStats {
    /// Frames that the kernel dropped because the receive ring was full or
    /// the frame was too long.
    pub rx_dropped: u64,
    /// Receive descriptors that the kernel found invalid.
    pub rx_invalid_descs: u64,
    /// Transmit descriptors that the kernel found invalid.
    pub tx_invalid_descs: u64,
    /// Frames dropped because the receive ring was full.
    pub rx_ring_full: u64,
    /// Receive attempts that found the fill ring empty.
    pub rx_fill_ring_empty_descs: u64,
    /// Transmit attempts that found the transmit ring empty.
    pub tx_ring_empty_descs: u64,
}

fn cvt(ret: libc::c_int) -> io::Result<libc::c_int> {
    if ret < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(ret)
    }
}

/// The page size of the system.
pub(crate) fn page_size() -> u32 {
    // SAFETY: `sysconf` reads a system constant and has no preconditions.
    let n = unsafe { libc::sysconf(libc::_SC_PAGESIZE) };
    u32::try_from(n).ok().filter(|n| *n >= 4096).unwrap_or(4096)
}

/// The index of the interface called `name`.
pub(crate) fn if_nametoindex(name: &str) -> io::Result<u32> {
    let c = CString::new(name).map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))?;
    // SAFETY: `c` is a valid NUL terminated string that outlives the call.
    let idx = unsafe { libc::if_nametoindex(c.as_ptr()) };
    if idx == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(idx)
    }
}

/// A memory mapping that unmaps itself.
#[derive(Debug)]
pub(crate) struct Mapping {
    ptr: NonNull<u8>,
    len: usize,
}

// SAFETY: a `Mapping` owns its region exclusively. Moving it to another thread
// moves the only handle, and the region has no thread affinity.
unsafe impl Send for Mapping {}

impl Mapping {
    /// Maps `len` bytes of zeroed anonymous memory, which the kernel
    /// populates at once so the data path takes no page fault.
    pub(crate) fn anonymous(len: usize) -> io::Result<Self> {
        // SAFETY: a null hint with MAP_ANONYMOUS and fd -1 maps fresh memory
        // and touches no existing mapping.
        let p = unsafe {
            libc::mmap(
                std::ptr::null_mut(),
                len,
                libc::PROT_READ | libc::PROT_WRITE,
                libc::MAP_PRIVATE | libc::MAP_ANONYMOUS | libc::MAP_POPULATE,
                -1,
                0,
            )
        };
        Self::from_raw(p, len)
    }

    /// Maps `len` bytes of an AF_XDP ring at the page offset `offset` of `fd`.
    fn of_socket(fd: RawFd, offset: u64, len: usize) -> io::Result<Self> {
        let offset = libc::off_t::try_from(offset)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))?;
        // SAFETY: a null hint maps fresh memory, and the kernel checks `fd`,
        // the offset, and the length.
        let p = unsafe {
            libc::mmap(
                std::ptr::null_mut(),
                len,
                libc::PROT_READ | libc::PROT_WRITE,
                libc::MAP_SHARED | libc::MAP_POPULATE,
                fd,
                offset,
            )
        };
        Self::from_raw(p, len)
    }

    fn from_raw(p: *mut libc::c_void, len: usize) -> io::Result<Self> {
        if p == libc::MAP_FAILED {
            return Err(io::Error::last_os_error());
        }
        NonNull::new(p.cast::<u8>())
            .map(|ptr| Self { ptr, len })
            .ok_or_else(|| io::Error::other("mmap returned a null pointer"))
    }

    /// The first byte of the region.
    pub(crate) fn as_ptr(&self) -> *mut u8 {
        self.ptr.as_ptr()
    }

    /// The length of the region in bytes.
    pub(crate) fn len(&self) -> usize {
        self.len
    }

    /// `len` bytes at `offset`, or `None` when they do not lie inside the
    /// region.
    pub(crate) fn slice(&self, offset: u64, len: usize) -> Option<&[u8]> {
        let start = usize::try_from(offset).ok()?;
        let end = start.checked_add(len)?;
        if end > self.len {
            return None;
        }
        // SAFETY: the range lies inside the mapping, which lives as long as
        // `self`. The caller passes only offsets of frames that it owns, so
        // the kernel does not write the range while the slice exists.
        Some(unsafe { std::slice::from_raw_parts(self.ptr.as_ptr().add(start), len) })
    }

    /// Like [`Mapping::slice`], for writing.
    pub(crate) fn slice_mut(&mut self, offset: u64, len: usize) -> Option<&mut [u8]> {
        let start = usize::try_from(offset).ok()?;
        let end = start.checked_add(len)?;
        if end > self.len {
            return None;
        }
        // SAFETY: as in `slice`, and `&mut self` makes this the only slice of
        // the mapping that this process hands out.
        Some(unsafe { std::slice::from_raw_parts_mut(self.ptr.as_ptr().add(start), len) })
    }
}

impl Drop for Mapping {
    fn drop(&mut self) {
        // SAFETY: `ptr` and `len` describe a mapping that `mmap` returned and
        // that nothing else unmaps.
        unsafe {
            libc::munmap(self.ptr.as_ptr().cast(), self.len);
        }
    }
}

fn setsockopt<T>(fd: RawFd, name: libc::c_int, value: &T) -> io::Result<()> {
    // SAFETY: `value` points to `size_of::<T>()` readable bytes for the call,
    // and every `T` passed here is a plain `repr(C)` integer structure.
    cvt(unsafe {
        libc::setsockopt(
            fd,
            SOL_XDP,
            name,
            std::ptr::from_ref(value).cast(),
            std::mem::size_of::<T>() as libc::socklen_t,
        )
    })
    .map(drop)
}

/// Reads an option into a zeroed `T` and returns it with the length that the
/// kernel wrote.
fn getsockopt<T: Default>(fd: RawFd, name: libc::c_int) -> io::Result<(T, usize)> {
    let mut value = T::default();
    let mut len = std::mem::size_of::<T>() as libc::socklen_t;
    // SAFETY: `value` is a writable `T` of `len` bytes, `len` is a valid
    // pointer, and every `T` passed here accepts any bit pattern.
    cvt(unsafe {
        libc::getsockopt(
            fd,
            SOL_XDP,
            name,
            std::ptr::from_mut(&mut value).cast(),
            &mut len,
        )
    })?;
    Ok((value, len as usize))
}

/// The four rings of a socket, mapped.
pub(crate) struct MappedRings {
    pub rx: Mapping,
    pub tx: Mapping,
    pub fill: Mapping,
    pub completion: Mapping,
    pub offsets: MmapOffsets,
}

/// One AF_XDP socket, not yet bound.
#[derive(Debug)]
pub(crate) struct XskSocket {
    fd: OwnedFd,
}

impl XskSocket {
    pub(crate) fn new() -> io::Result<Self> {
        // SAFETY: `socket` has no pointer arguments.
        let fd = cvt(unsafe { libc::socket(AF_XDP, libc::SOCK_RAW | libc::SOCK_CLOEXEC, 0) })?;
        // SAFETY: `fd` is a new descriptor that nothing else owns.
        Ok(Self {
            fd: unsafe { OwnedFd::from_raw_fd(fd) },
        })
    }

    pub(crate) fn raw(&self) -> RawFd {
        self.fd.as_raw_fd()
    }

    /// Registers `umem` as the packet memory with chunks of `chunk_size`.
    pub(crate) fn register_umem(&self, umem: &Mapping, chunk_size: u32) -> io::Result<()> {
        let reg = UmemReg {
            addr: umem.as_ptr() as u64,
            len: umem.len() as u64,
            chunk_size,
            headroom: 0,
        };
        setsockopt(self.raw(), XDP_UMEM_REG, &reg)
    }

    /// Sets the entry counts of the four rings and maps them.
    pub(crate) fn map_rings(&self, ring_size: u32) -> io::Result<MappedRings> {
        let fd = self.raw();
        setsockopt(fd, XDP_UMEM_FILL_RING, &(ring_size as libc::c_int))?;
        setsockopt(fd, XDP_UMEM_COMPLETION_RING, &(ring_size as libc::c_int))?;
        setsockopt(fd, XDP_RX_RING, &(ring_size as libc::c_int))?;
        setsockopt(fd, XDP_TX_RING, &(ring_size as libc::c_int))?;
        let (off, got) = getsockopt::<MmapOffsets>(fd, XDP_MMAP_OFFSETS)?;
        if got != std::mem::size_of::<MmapOffsets>() {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "the kernel reports ring offsets without flags, which means it is older than 5.4",
            ));
        }
        let n = ring_size as usize;
        let desc = std::mem::size_of::<XdpDesc>();
        let addr = std::mem::size_of::<u64>();
        Ok(MappedRings {
            rx: Mapping::of_socket(fd, XDP_PGOFF_RX_RING, off.rx.desc as usize + n * desc)?,
            tx: Mapping::of_socket(fd, XDP_PGOFF_TX_RING, off.tx.desc as usize + n * desc)?,
            fill: Mapping::of_socket(
                fd,
                XDP_UMEM_PGOFF_FILL_RING,
                off.fill.desc as usize + n * addr,
            )?,
            completion: Mapping::of_socket(
                fd,
                XDP_UMEM_PGOFF_COMPLETION_RING,
                off.completion.desc as usize + n * addr,
            )?,
            offsets: off,
        })
    }

    /// Binds the socket to `queue` of the interface with index `ifindex`.
    pub(crate) fn bind(&self, ifindex: u32, queue: u32, mode: crate::BindMode) -> io::Result<()> {
        let mode_flag = match mode {
            crate::BindMode::Auto => 0,
            crate::BindMode::ZeroCopy => XDP_ZEROCOPY,
            crate::BindMode::Copy => XDP_COPY,
        };
        let sa = SockaddrXdp {
            family: AF_XDP as u16,
            flags: XDP_USE_NEED_WAKEUP | mode_flag,
            ifindex,
            queue_id: queue,
            shared_umem_fd: 0,
        };
        // SAFETY: `sa` is a valid `sockaddr_xdp` of the length passed.
        cvt(unsafe {
            libc::bind(
                self.raw(),
                std::ptr::from_ref(&sa).cast(),
                std::mem::size_of::<SockaddrXdp>() as libc::socklen_t,
            )
        })
        .map(drop)
    }

    /// Reads the bind mode from the socket, which is the only place that
    /// tells whether the kernel chose zero copy.
    pub(crate) fn bound_mode(&self) -> io::Result<crate::BoundMode> {
        #[repr(C)]
        #[derive(Default)]
        struct Options {
            flags: u32,
        }
        let (o, _) = getsockopt::<Options>(self.raw(), XDP_OPTIONS)?;
        Ok(if o.flags & XDP_OPTIONS_ZEROCOPY != 0 {
            crate::BoundMode::ZeroCopy
        } else {
            crate::BoundMode::Copy
        })
    }

    /// The counters that the kernel keeps for this socket.
    pub(crate) fn kernel_stats(&self) -> io::Result<KernelStats> {
        getsockopt::<KernelStats>(self.raw(), XDP_STATISTICS).map(|(s, _)| s)
    }

    /// Wakes the kernel to send, with `MSG_DONTWAIT`.
    pub(crate) fn kick_tx(&self) -> io::Result<()> {
        // SAFETY: a null buffer of length 0 and a null address are valid for
        // `sendto` on an AF_XDP socket, which reads no data from them.
        cvt(unsafe {
            libc::sendto(
                self.raw(),
                std::ptr::null(),
                0,
                libc::MSG_DONTWAIT,
                std::ptr::null(),
                0,
            )
        } as libc::c_int)
        .map(drop)
    }

    /// Wakes the kernel to receive, with `MSG_DONTWAIT`.
    pub(crate) fn kick_rx(&self) -> io::Result<()> {
        // SAFETY: a null buffer of length 0 and null address pointers are
        // valid for `recvfrom` on an AF_XDP socket, which writes nothing.
        cvt(unsafe {
            libc::recvfrom(
                self.raw(),
                std::ptr::null_mut(),
                0,
                libc::MSG_DONTWAIT,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            )
        } as libc::c_int)
        .map(drop)
    }

    /// Waits up to `timeout_ms` for a frame to arrive.
    pub(crate) fn poll_readable(&self, timeout_ms: i32) -> io::Result<bool> {
        let mut pfd = libc::pollfd {
            fd: self.raw(),
            events: libc::POLLIN,
            revents: 0,
        };
        // SAFETY: `pfd` is one valid `pollfd` and the count passed is 1.
        let n = cvt(unsafe { libc::poll(&mut pfd, 1, timeout_ms) });
        match n {
            Ok(n) => Ok(n > 0 && pfd.revents & libc::POLLIN != 0),
            Err(e) if e.kind() == io::ErrorKind::Interrupted => Ok(false),
            Err(e) => Err(e),
        }
    }
}

/// Whether a ring flag word asks for a wake up.
pub(crate) fn needs_wakeup(flags: u32) -> bool {
    flags & XDP_RING_NEED_WAKEUP != 0
}

/// Pins the calling thread to `cpu`.
pub(crate) fn pin_current_thread(cpu: usize) -> io::Result<()> {
    if cpu >= libc::CPU_SETSIZE as usize {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "cpu number too large",
        ));
    }
    // SAFETY: the set is zeroed, `cpu` is below `CPU_SETSIZE`, and
    // `sched_setaffinity` reads the set for the given size. Pid 0 means the
    // calling thread.
    cvt(unsafe {
        let mut set: libc::cpu_set_t = std::mem::zeroed();
        libc::CPU_SET(cpu, &mut set);
        libc::sched_setaffinity(0, std::mem::size_of::<libc::cpu_set_t>(), &set)
    })
    .map(drop)
}

// ---------------------------------------------------------------------------
// BPF
// ---------------------------------------------------------------------------

const BPF_MAP_CREATE: libc::c_long = 0;
const BPF_MAP_UPDATE_ELEM: libc::c_long = 2;
const BPF_PROG_LOAD: libc::c_long = 5;
const BPF_LINK_CREATE: libc::c_long = 28;
const BPF_MAP_TYPE_XSKMAP: u32 = 17;
const BPF_PROG_TYPE_XDP: u32 = 6;
const BPF_XDP: u32 = 37;
const XDP_FLAGS_SKB_MODE: u32 = 1 << 1;

/// The attributes of `BPF_MAP_CREATE`.
#[repr(C)]
#[derive(Default)]
struct MapCreate {
    map_type: u32,
    key_size: u32,
    value_size: u32,
    max_entries: u32,
    map_flags: u32,
}

/// The attributes of `BPF_MAP_UPDATE_ELEM`.
#[repr(C)]
#[derive(Default)]
struct MapUpdate {
    map_fd: u32,
    pad: u32,
    key: u64,
    value: u64,
    flags: u64,
}

/// The attributes of `BPF_PROG_LOAD` up to `expected_attach_type`.
#[repr(C)]
#[derive(Default)]
struct ProgLoad {
    prog_type: u32,
    insn_cnt: u32,
    insns: u64,
    license: u64,
    log_level: u32,
    log_size: u32,
    log_buf: u64,
    kern_version: u32,
    prog_flags: u32,
    prog_name: [u8; 16],
    prog_ifindex: u32,
    expected_attach_type: u32,
}

/// The attributes of `BPF_LINK_CREATE` for an XDP program.
#[repr(C)]
#[derive(Default)]
struct LinkCreate {
    prog_fd: u32,
    target_ifindex: u32,
    attach_type: u32,
    flags: u32,
}

/// One BPF instruction, `struct bpf_insn`.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Insn {
    code: u8,
    regs: u8,
    off: i16,
    imm: i32,
}

const fn insn(code: u8, dst: u8, src: u8, off: i16, imm: i32) -> Insn {
    Insn {
        code,
        regs: (src << 4) | (dst & 0x0f),
        off,
        imm,
    }
}

/// The program that sends every frame of queue `n` to the socket at index
/// `n` of the XSK map and passes the frame to the kernel stack when the map
/// has no socket for that queue:
///
/// ```text
/// r2 = *(u32 *)(r1 + 16)        // ctx->rx_queue_index
/// r1 = map_fd ll                // pseudo map fd
/// r3 = XDP_PASS                 // action when the lookup fails
/// call bpf_redirect_map
/// exit
/// ```
pub(crate) fn redirect_program(map_fd: RawFd) -> [Insn; 6] {
    const BPF_LDX_MEM_W: u8 = 0x61;
    const BPF_LD_IMM64: u8 = 0x18;
    const BPF_MOV64_IMM: u8 = 0xb7;
    const BPF_CALL: u8 = 0x85;
    const BPF_EXIT: u8 = 0x95;
    const BPF_PSEUDO_MAP_FD: u8 = 1;
    const BPF_FUNC_REDIRECT_MAP: i32 = 51;
    const XDP_PASS: i32 = 2;
    const RX_QUEUE_INDEX_OFFSET: i16 = 16;
    [
        insn(BPF_LDX_MEM_W, 2, 1, RX_QUEUE_INDEX_OFFSET, 0),
        insn(BPF_LD_IMM64, 1, BPF_PSEUDO_MAP_FD, 0, map_fd),
        insn(0, 0, 0, 0, 0),
        insn(BPF_MOV64_IMM, 3, 0, 0, XDP_PASS),
        insn(BPF_CALL, 0, 0, 0, BPF_FUNC_REDIRECT_MAP),
        insn(BPF_EXIT, 0, 0, 0, 0),
    ]
}

/// Runs a `bpf` command that returns a new file descriptor.
fn bpf<T>(cmd: libc::c_long, attr: &T) -> io::Result<OwnedFd> {
    // SAFETY: `attr` points to `size_of::<T>()` readable bytes, a prefix of
    // `union bpf_attr` that the kernel zero extends, and the pointers inside
    // it are valid for the call as the callers document.
    let ret = unsafe {
        libc::syscall(
            libc::SYS_bpf,
            cmd,
            std::ptr::from_ref(attr),
            std::mem::size_of::<T>(),
        )
    };
    if ret < 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: only the commands that return a new file descriptor use this
    // function, and the descriptor is not owned by anything else.
    Ok(unsafe { OwnedFd::from_raw_fd(ret as RawFd) })
}

/// Runs a `bpf` command that returns 0 and no descriptor.
fn bpf_unit<T>(cmd: libc::c_long, attr: &T) -> io::Result<()> {
    // SAFETY: as in `bpf`.
    let ret = unsafe {
        libc::syscall(
            libc::SYS_bpf,
            cmd,
            std::ptr::from_ref(attr),
            std::mem::size_of::<T>(),
        )
    };
    if ret < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

/// An XSK map that sends frames to the sockets of a port.
#[derive(Debug)]
pub(crate) struct XskMap {
    fd: OwnedFd,
}

impl XskMap {
    /// Creates a map with `entries` slots, one per queue number.
    pub(crate) fn new(entries: u32) -> io::Result<Self> {
        let attr = MapCreate {
            map_type: BPF_MAP_TYPE_XSKMAP,
            key_size: 4,
            value_size: 4,
            max_entries: entries,
            map_flags: 0,
        };
        Ok(Self {
            fd: bpf(BPF_MAP_CREATE, &attr)?,
        })
    }

    /// Stores the socket `sock` at `queue`.
    pub(crate) fn insert(&self, queue: u32, sock: &XskSocket) -> io::Result<()> {
        let key = queue;
        let value = sock.raw() as u32;
        let attr = MapUpdate {
            map_fd: self.fd.as_raw_fd() as u32,
            pad: 0,
            key: std::ptr::from_ref(&key) as u64,
            value: std::ptr::from_ref(&value) as u64,
            flags: 0,
        };
        bpf_unit(BPF_MAP_UPDATE_ELEM, &attr)
    }

    pub(crate) fn raw(&self) -> RawFd {
        self.fd.as_raw_fd()
    }
}

/// How the program is attached to the device.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum XdpMode {
    /// In the driver, before the kernel allocates a buffer.
    Native,
    /// In the generic hook after the kernel allocated a buffer, which every
    /// device supports and which is slower.
    Generic,
}

/// A loaded program attached to an interface through a BPF link, which the
/// kernel detaches when the last descriptor of the link closes.
#[derive(Debug)]
pub(crate) struct Attached {
    _link: OwnedFd,
    _prog: OwnedFd,
    _map: XskMap,
    pub mode: XdpMode,
}

/// The error of an XDP program load, with the verifier log if the kernel
/// wrote one.
#[derive(Debug)]
pub(crate) struct LoadError {
    /// The step that failed, `load XDP program` or `attach XDP program`.
    pub step: &'static str,
    pub source: io::Error,
    pub log: String,
}

/// Loads the redirect program for `map` and attaches it to `ifindex`.
///
/// Native mode comes first, and a device without native XDP support gets the
/// generic mode.
pub(crate) fn attach_redirect(ifindex: u32, map: XskMap) -> Result<Attached, LoadError> {
    let insns = redirect_program(map.raw());
    let license = b"GPL\0";
    let mut load = ProgLoad {
        prog_type: BPF_PROG_TYPE_XDP,
        insn_cnt: insns.len() as u32,
        insns: insns.as_ptr() as u64,
        license: license.as_ptr() as u64,
        ..ProgLoad::default()
    };
    load.prog_name[..11].copy_from_slice(b"fps_xsk_rdr");
    let prog = match bpf(BPF_PROG_LOAD, &load) {
        Ok(fd) => fd,
        Err(source) => {
            // Load again with a log buffer, which costs nothing on success
            // and says why the verifier refused the program.
            let mut buf = vec![0u8; 4096];
            load.log_level = 1;
            load.log_size = buf.len() as u32;
            load.log_buf = buf.as_mut_ptr() as u64;
            let _ = bpf(BPF_PROG_LOAD, &load);
            let end = buf.iter().position(|b| *b == 0).unwrap_or(buf.len());
            let log = String::from_utf8_lossy(&buf[..end]).trim().to_owned();
            return Err(LoadError {
                step: "load XDP program",
                source,
                log,
            });
        }
    };
    let link = |flags: u32| {
        bpf(
            BPF_LINK_CREATE,
            &LinkCreate {
                prog_fd: prog.as_raw_fd() as u32,
                target_ifindex: ifindex,
                attach_type: BPF_XDP,
                flags,
            },
        )
    };
    let (link, mode) = match link(0) {
        Ok(fd) => (fd, XdpMode::Native),
        Err(e) if e.raw_os_error() == Some(libc::EOPNOTSUPP) => match link(XDP_FLAGS_SKB_MODE) {
            Ok(fd) => (fd, XdpMode::Generic),
            Err(source) => {
                return Err(LoadError {
                    step: "attach XDP program",
                    source,
                    log: String::new(),
                })
            }
        },
        Err(source) => {
            return Err(LoadError {
                step: "attach XDP program",
                source,
                log: String::new(),
            })
        }
    };
    Ok(Attached {
        _link: link,
        _prog: prog,
        _map: map,
        mode,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_redirect_program_encodes_the_documented_instructions() {
        let p = redirect_program(7);
        assert_eq!(std::mem::size_of::<Insn>(), 8);
        // r2 = *(u32 *)(r1 + 16)
        assert_eq!(
            p[0],
            Insn {
                code: 0x61,
                regs: 0x12,
                off: 16,
                imm: 0
            }
        );
        // r1 = map fd 7 as a pseudo map reference, two slots wide
        assert_eq!(
            p[1],
            Insn {
                code: 0x18,
                regs: 0x11,
                off: 0,
                imm: 7
            }
        );
        assert_eq!(
            p[2],
            Insn {
                code: 0,
                regs: 0,
                off: 0,
                imm: 0
            }
        );
        // r3 = XDP_PASS
        assert_eq!(
            p[3],
            Insn {
                code: 0xb7,
                regs: 0x03,
                off: 0,
                imm: 2
            }
        );
        // call bpf_redirect_map, then exit
        assert_eq!(
            p[4],
            Insn {
                code: 0x85,
                regs: 0,
                off: 0,
                imm: 51
            }
        );
        assert_eq!(
            p[5],
            Insn {
                code: 0x95,
                regs: 0,
                off: 0,
                imm: 0
            }
        );
    }

    #[test]
    fn struct_sizes_match_the_kernel_headers() {
        assert_eq!(std::mem::size_of::<RingOffset>(), 32);
        assert_eq!(std::mem::size_of::<MmapOffsets>(), 128);
        assert_eq!(std::mem::size_of::<XdpDesc>(), 16);
        assert_eq!(std::mem::size_of::<UmemReg>(), 24);
        assert_eq!(std::mem::size_of::<SockaddrXdp>(), 16);
        assert_eq!(std::mem::size_of::<KernelStats>(), 48);
        assert_eq!(std::mem::size_of::<ProgLoad>(), 72);
    }

    #[test]
    fn mappings_are_zeroed_and_bounds_checked() {
        let mut m = Mapping::anonymous(8192).unwrap();
        assert_eq!(m.len(), 8192);
        assert_eq!(m.slice(0, 8192).unwrap().iter().copied().max(), Some(0));
        m.slice_mut(4096, 4).unwrap().copy_from_slice(&[1, 2, 3, 4]);
        assert_eq!(m.slice(4097, 2), Some(&[2u8, 3][..]));
        assert!(m.slice(8190, 3).is_none());
        assert!(m.slice(u64::MAX, 1).is_none());
        assert!(m.slice(1, usize::MAX).is_none());
        assert!(m.slice_mut(8192, 1).is_none());
    }

    #[test]
    fn the_page_size_and_interface_lookup_work() {
        assert!(page_size().is_power_of_two());
        assert!(if_nametoindex("lo").unwrap() >= 1);
        let e = if_nametoindex("no-such-if0").unwrap_err();
        assert_eq!(e.raw_os_error(), Some(libc::ENODEV));
        assert_eq!(
            if_nametoindex("a\0b").unwrap_err().kind(),
            io::ErrorKind::InvalidInput
        );
    }

    #[test]
    fn the_wakeup_flag_is_bit_zero() {
        assert!(needs_wakeup(1));
        assert!(needs_wakeup(3));
        assert!(!needs_wakeup(0));
        assert!(!needs_wakeup(2));
    }

    #[test]
    fn a_thread_can_pin_itself_to_an_allowed_cpu() {
        let t = std::thread::spawn(|| {
            // The first CPU of the current affinity set always exists.
            let cpu = (0..libc::CPU_SETSIZE as usize).find(|c| {
                let mut set: libc::cpu_set_t = unsafe { std::mem::zeroed() };
                // SAFETY: the set is zeroed and large enough for the call.
                unsafe {
                    libc::sched_getaffinity(0, std::mem::size_of::<libc::cpu_set_t>(), &mut set);
                    libc::CPU_ISSET(*c, &set)
                }
            });
            pin_current_thread(cpu.unwrap()).unwrap();
            pin_current_thread(libc::CPU_SETSIZE as usize)
                .unwrap_err()
                .kind()
        });
        assert_eq!(t.join().unwrap(), io::ErrorKind::InvalidInput);
    }
}
