// SPDX-License-Identifier: GPL-3.0-only
//! The kernel reference stack, which implements [`SessionIo`] over the TCP of
//! the operating system on the loopback interface.
//!
//! A [`KernelStack`] is the baseline that every user space candidate of TRN-10
//! must beat or match. Its client host and its server host are sockets of one
//! kernel that talk over `127.0.0.1`, driven by the reactor of the tokio
//! runtime that the workload runs on. The adapter follows the rules below.
//!
//! 1. A stack needs no driver. The kernel moves the segments and runs the
//!    timers, and [`TcpStack::drive`] only waits.
//! 2. A listened port is a logical number of the harness. Each stack binds an
//!    ephemeral loopback port for it and keeps the mapping, so the shards of a
//!    scaling run, which all listen on the same logical ports, never collide.
//! 3. [`KernelSession::abort`] sets `SO_LINGER` to zero and closes the socket,
//!    the same way as the kernel backend of the engine, and dropping a
//!    session closes it with a FIN.
//!
//! The numbers of this stack count only what the process can see. The memory
//! of a kernel socket is kernel memory, so the resident set of the process
//! does not grow with it, and the report says so in its notes.

use std::collections::HashMap;
use std::future::Future;
use std::io;
use std::net::{Ipv4Addr, SocketAddr};
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::task::{Context, Poll};

use fps_io::SessionIo;
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::net::{TcpListener, TcpSocket, TcpStream};

use crate::stack::{StackStats, TcpStack};

/// The name that the reports use for this stack.
pub const KERNEL_STACK_NAME: &str = "kernel loopback TCP";

/// Settings of a [`KernelStack`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KernelConfig {
    /// Whether `TCP_NODELAY` is on, which turns Nagle's algorithm off. The
    /// kernel backend of the engine turns it off, so `true` is the default.
    pub nodelay: bool,
    /// The send and receive buffer of every socket in bytes, or `None` to
    /// leave the sizes to the kernel, which tunes them while a session runs.
    pub buffer_bytes: Option<usize>,
    /// The accept backlog of each listened port. The kernel caps it at
    /// `net.core.somaxconn`.
    pub backlog: u32,
}

impl Default for KernelConfig {
    fn default() -> Self {
        Self {
            nodelay: true,
            buffer_bytes: None,
            backlog: 4096,
        }
    }
}

/// Counts the sockets of a stack. Dropping the guard removes one.
#[derive(Debug)]
struct SocketCount {
    open: Arc<AtomicU64>,
}

impl SocketCount {
    fn new(open: &Arc<AtomicU64>) -> Self {
        open.fetch_add(1, Ordering::Relaxed);
        Self {
            open: Arc::clone(open),
        }
    }
}

impl Drop for SocketCount {
    fn drop(&mut self) {
        self.open.fetch_sub(1, Ordering::Relaxed);
    }
}

/// A listening socket and the loopback address that it got.
struct Listener {
    socket: TcpListener,
    addr: SocketAddr,
    _count: SocketCount,
}

struct Shared {
    config: KernelConfig,
    listeners: Mutex<HashMap<u16, Arc<Listener>>>,
    open: Arc<AtomicU64>,
}

impl Shared {
    fn listeners(&self) -> MutexGuard<'_, HashMap<u16, Arc<Listener>>> {
        self.listeners
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    fn listener(&self, port: u16) -> Option<Arc<Listener>> {
        self.listeners().get(&port).cloned()
    }

    fn socket(&self) -> io::Result<TcpSocket> {
        let socket = TcpSocket::new_v4()?;
        if let Some(bytes) = self.config.buffer_bytes {
            let bytes = u32::try_from(bytes).unwrap_or(u32::MAX);
            socket.set_recv_buffer_size(bytes)?;
            socket.set_send_buffer_size(bytes)?;
        }
        Ok(socket)
    }

    fn session(&self, stream: TcpStream) -> io::Result<KernelSession> {
        if self.config.nodelay {
            stream.set_nodelay(true)?;
        }
        Ok(KernelSession {
            inner: Some(stream),
            _count: SocketCount::new(&self.open),
        })
    }
}

/// The client host and the server host of the operating system, joined by
/// the loopback interface.
///
/// [`TcpStack::listen`] must run inside a tokio runtime, because it registers
/// the socket with the reactor.
#[derive(Clone)]
pub struct KernelStack {
    shared: Arc<Shared>,
}

impl KernelStack {
    /// Creates a stack with no listened ports.
    pub fn new(config: KernelConfig) -> Self {
        Self {
            shared: Arc::new(Shared {
                config,
                listeners: Mutex::new(HashMap::new()),
                open: Arc::new(AtomicU64::new(0)),
            }),
        }
    }
}

impl Default for KernelStack {
    fn default() -> Self {
        Self::new(KernelConfig::default())
    }
}

impl TcpStack for KernelStack {
    type Session = KernelSession;

    fn name(&self) -> String {
        KERNEL_STACK_NAME.to_string()
    }

    fn notes(&self) -> Vec<String> {
        vec![
            "the client and the server host are sockets of one kernel that talk over the loopback interface of the host, in one process".to_string(),
            "the kernel runs the TCP timers and moves the segments, so there is no user space poll and idle_poll_us is zero by construction".to_string(),
            "bytes_per_socket is the growth of the resident set of the process, and kernel socket memory is not part of it".to_string(),
            "the stack has no frame counters, so frames and pump passes stay zero".to_string(),
            "every socket is a file descriptor, so the open files limit of the process caps the scaling run, and the report lists it as open_files_soft_limit".to_string(),
        ]
    }

    fn listen(&self, port: u16) -> io::Result<()> {
        let mut listeners = self.shared.listeners();
        if listeners.contains_key(&port) {
            return Err(io::Error::new(
                io::ErrorKind::AddrInUse,
                format!("port {port} is already listened on"),
            ));
        }
        let socket = self.shared.socket()?;
        socket.bind(SocketAddr::from((Ipv4Addr::LOCALHOST, 0)))?;
        let socket = socket.listen(self.shared.config.backlog)?;
        let addr = socket.local_addr()?;
        listeners.insert(
            port,
            Arc::new(Listener {
                socket,
                addr,
                _count: SocketCount::new(&self.shared.open),
            }),
        );
        Ok(())
    }

    fn connect(&self, port: u16) -> impl Future<Output = io::Result<KernelSession>> + Send {
        let shared = Arc::clone(&self.shared);
        async move {
            let Some(listener) = shared.listener(port) else {
                return Err(io::Error::new(
                    io::ErrorKind::ConnectionRefused,
                    format!("port {port} is not listened on"),
                ));
            };
            let stream = shared.socket()?.connect(listener.addr).await?;
            shared.session(stream)
        }
    }

    fn accept(&self, port: u16) -> impl Future<Output = io::Result<KernelSession>> + Send {
        let shared = Arc::clone(&self.shared);
        async move {
            let Some(listener) = shared.listener(port) else {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!("port {port} is not listened on"),
                ));
            };
            let (stream, _peer) = listener.socket.accept().await?;
            shared.session(stream)
        }
    }

    fn drive(&self) -> impl Future<Output = ()> + Send {
        std::future::pending()
    }

    fn poll_once(&self) {
        // The kernel has no poll that a caller could time.
    }

    fn stats(&self) -> StackStats {
        StackStats {
            sockets_open: self.shared.open.load(Ordering::Relaxed),
            pump_passes: 0,
            frames: 0,
            ..StackStats::default()
        }
    }
}

/// One kernel TCP session, on the client or the server side, which
/// implements [`SessionIo`].
///
/// Closing the write side with `poll_shutdown` sends a FIN. Dropping the
/// session closes it, and [`SessionIo::abort`] resets it.
#[derive(Debug)]
pub struct KernelSession {
    /// The socket, or `None` after an abort.
    inner: Option<TcpStream>,
    _count: SocketCount,
}

fn not_connected() -> io::Error {
    io::Error::new(io::ErrorKind::NotConnected, "the session was aborted")
}

impl KernelSession {
    fn pinned(&mut self) -> io::Result<Pin<&mut TcpStream>> {
        self.inner.as_mut().map(Pin::new).ok_or_else(not_connected)
    }
}

impl AsyncRead for KernelSession {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        match self.get_mut().pinned() {
            Ok(stream) => stream.poll_read(cx, buf),
            Err(error) => Poll::Ready(Err(error)),
        }
    }
}

impl AsyncWrite for KernelSession {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        data: &[u8],
    ) -> Poll<io::Result<usize>> {
        match self.get_mut().pinned() {
            Ok(stream) => stream.poll_write(cx, data),
            Err(error) => Poll::Ready(Err(error)),
        }
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        match self.get_mut().pinned() {
            Ok(stream) => stream.poll_flush(cx),
            Err(error) => Poll::Ready(Err(error)),
        }
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        match self.get_mut().pinned() {
            Ok(stream) => stream.poll_shutdown(cx),
            Err(error) => Poll::Ready(Err(error)),
        }
    }
}

impl SessionIo for KernelSession {
    fn abort(&mut self) {
        if let Some(stream) = self.inner.take() {
            // A zero linger makes close send a reset and return at once. If
            // the kernel refuses the option, the drop still closes the socket
            // with a FIN, and nothing stays open.
            let _ = stream.set_zero_linger();
            drop(stream);
        }
    }
}
