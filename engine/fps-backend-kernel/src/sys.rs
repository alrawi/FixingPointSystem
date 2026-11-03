// SPDX-License-Identifier: GPL-3.0-only
//! Socket options that `socket2` does not expose.
//!
//! `socket2` 0.6 has no setter for `IP_BIND_ADDRESS_NO_PORT`,
//! `IP_LOCAL_PORT_RANGE`, or `TCP_WINDOW_CLAMP`, so this module calls
//! `setsockopt` through `libc`. It is the only module of the crate that
//! contains `unsafe` code.

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

/// Bounds the receive window of the socket, and with it the window scale
/// that the handshake advertises, to `clamp` bytes.
pub(crate) fn set_window_clamp(sock: &Socket, clamp: u32) -> io::Result<()> {
    #[cfg(target_os = "linux")]
    {
        set_u32(sock, libc::IPPROTO_TCP, libc::TCP_WINDOW_CLAMP, clamp)
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (sock, clamp);
        Err(unsupported("TCP_WINDOW_CLAMP"))
    }
}

/// Returns the `TCP_WINDOW_CLAMP` of the socket, for the tests of the crate.
#[cfg(all(test, target_os = "linux"))]
pub(crate) fn window_clamp(sock: &Socket) -> io::Result<u32> {
    use std::os::fd::AsRawFd;

    let mut value: u32 = 0;
    let mut len = LEN_U32;
    let ptr: *mut libc::c_void = std::ptr::from_mut(&mut value).cast();
    // SAFETY: `sock` owns the open socket `fd` for the duration of the call,
    // `ptr` points to `value`, which lives on the stack until the call
    // returns, and `len` tells the kernel that it may write at most the 4
    // bytes of `value`. The kernel writes a 32 bit integer for this option.
    let rc = unsafe {
        libc::getsockopt(
            sock.as_raw_fd(),
            libc::IPPROTO_TCP,
            libc::TCP_WINDOW_CLAMP,
            ptr,
            &mut len,
        )
    };
    if rc == 0 {
        Ok(value)
    } else {
        Err(io::Error::last_os_error())
    }
}

/// The length of a 32 bit option value, which always fits `socklen_t`.
#[cfg(target_os = "linux")]
const LEN_U32: libc::socklen_t = std::mem::size_of::<u32>() as libc::socklen_t;

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

    // Every option of this module takes a 32 bit integer.
    const LEN: libc::socklen_t = LEN_U32;
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
