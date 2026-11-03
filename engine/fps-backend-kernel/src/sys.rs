// SPDX-License-Identifier: GPL-3.0-only
//! Socket options that `socket2` does not expose.
//!
//! `socket2` 0.6 has no setter for `IP_BIND_ADDRESS_NO_PORT` or
//! `IP_LOCAL_PORT_RANGE`, so this module calls `setsockopt` through `libc`.
//! It is the only module of the crate that contains `unsafe` code.

use std::io;

use socket2::Socket;

use crate::PortRange;

/// `IP_LOCAL_PORT_RANGE` from `linux/in.h`, added in Linux 6.3. The `libc`
/// crate does not define it yet.
#[cfg(target_os = "linux")]
const IP_LOCAL_PORT_RANGE: libc::c_int = 51;

/// Defers the choice of the source port from `bind` to `connect`, so a port is
/// shared by sessions with different destinations.
pub(crate) fn set_bind_address_no_port(sock: &Socket) -> io::Result<()> {
    #[cfg(target_os = "linux")]
    {
        set_u32(sock, libc::IPPROTO_IP, libc::IP_BIND_ADDRESS_NO_PORT, 1)
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = sock;
        Err(unsupported("IP_BIND_ADDRESS_NO_PORT"))
    }
}

/// Limits the source port that the kernel picks for this socket to `range`.
///
/// Kernels before 6.3 reject the option with `ENOPROTOOPT`.
pub(crate) fn set_local_port_range(sock: &Socket, range: PortRange) -> io::Result<()> {
    #[cfg(target_os = "linux")]
    {
        set_u32(sock, libc::IPPROTO_IP, IP_LOCAL_PORT_RANGE, range.encode())
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (sock, range);
        Err(unsupported("IP_LOCAL_PORT_RANGE"))
    }
}

#[cfg(not(target_os = "linux"))]
fn unsupported(option: &str) -> io::Error {
    io::Error::new(
        io::ErrorKind::Unsupported,
        format!("{option} exists only on Linux"),
    )
}

#[cfg(target_os = "linux")]
fn set_u32(sock: &Socket, level: libc::c_int, name: libc::c_int, value: u32) -> io::Result<()> {
    use std::os::fd::AsRawFd;

    // Both options take a 32 bit integer, so the length always fits.
    const LEN: libc::socklen_t = std::mem::size_of::<u32>() as libc::socklen_t;
    let fd = sock.as_raw_fd();
    let ptr: *const libc::c_void = std::ptr::from_ref(&value).cast();
    // SAFETY: `fd` is an open socket because `sock` owns it for the duration
    // of the call, `ptr` points to `value`, which lives on the stack until the
    // call returns, and `LEN` equals the size of `value`, so the kernel reads
    // only initialized memory that this function owns.
    let rc = unsafe { libc::setsockopt(fd, level, name, ptr, LEN) };
    if rc == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}
