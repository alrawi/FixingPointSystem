// SPDX-License-Identifier: GPL-3.0-only
//! The data ports that the TCP flows of the instances of one worker share
//! (ENG-26).
//!
//! A server flow that is not the entry flow binds the port that its program
//! renders and accepts one connection. Two concurrent instances that draw the
//! same port used to bind in turn, but the client of the second instance
//! already learned the port and connected to the listener of the first, which
//! accepted one connection and reset the other (AGT-18).
//!
//! One listener now serves a port for as long as any instance waits on it.
//! Every connection goes to the waiter that expects the address it came from,
//! see [`crate::rendezvous`], so a connection that reaches the listener
//! before its own instance has bound still finds that instance. The listener
//! closes, and the port is free, when its last waiter has its connection or
//! gives up.

use std::cell::RefCell;
use std::collections::HashMap;
use std::io;
use std::net::{IpAddr, SocketAddr};
use std::rc::Rc;

use fps_backend_kernel::{KernelListener, KernelStream, ListenConfig};
use tokio::sync::Notify;
use tokio::time::{sleep, sleep_until, Instant};

use crate::instance::{OPEN_GRACE, RETRY_FIRST, RETRY_MAX};
use crate::rendezvous::Rendezvous;

/// The listeners of the data ports of one worker.
#[derive(Clone)]
pub(crate) struct DataPorts {
    inner: Rc<Inner>,
}

struct Inner {
    listen: ListenConfig,
    ports: RefCell<HashMap<SocketAddr, Rc<Port>>>,
}

struct Port {
    waiting: RefCell<Rendezvous<KernelStream>>,
    /// Wakes the task of the port when a waiter came or went.
    changed: Notify,
}

/// Removes a waiter when its `accept` ends, however it ends.
struct Registered<'a> {
    port: &'a Port,
    id: u64,
}

impl Drop for Registered<'_> {
    fn drop(&mut self) {
        self.port.waiting.borrow_mut().cancel(self.id);
        self.port.changed.notify_one();
    }
}

impl DataPorts {
    /// Returns the ports of a worker, which bind with `listen`.
    pub(crate) fn new(listen: ListenConfig) -> Self {
        Self {
            inner: Rc::new(Inner {
                listen,
                ports: RefCell::new(HashMap::new()),
            }),
        }
    }

    /// Returns the connection that `expect` dials to `addr`, or any
    /// connection when `expect` is `None`.
    ///
    /// The call binds the port when no instance of the worker holds it, and
    /// retries a bind that finds the port in use for [`OPEN_GRACE`], because
    /// a socket of another process or of the previous run may still hold it.
    pub(crate) async fn accept(
        &self,
        addr: SocketAddr,
        expect: Option<IpAddr>,
    ) -> io::Result<KernelStream> {
        let deadline = Instant::now() + OPEN_GRACE;
        let mut wait = RETRY_FIRST;
        let (port, id, rx) = loop {
            let shared = self.inner.ports.borrow().get(&addr).map(Rc::clone);
            if let Some(port) = shared {
                let (id, rx) = port.waiting.borrow_mut().register(expect);
                break (port, id, rx);
            }
            match KernelListener::bind(addr, &self.inner.listen) {
                Ok(listener) => {
                    let port = Rc::new(Port {
                        waiting: RefCell::new(Rendezvous::new()),
                        changed: Notify::new(),
                    });
                    let (id, rx) = port.waiting.borrow_mut().register(expect);
                    self.inner.ports.borrow_mut().insert(addr, Rc::clone(&port));
                    tokio::task::spawn_local(serve(
                        Rc::clone(&self.inner),
                        addr,
                        Rc::clone(&port),
                        listener,
                    ));
                    break (port, id, rx);
                }
                Err(e)
                    if e.io_kind() == Some(io::ErrorKind::AddrInUse)
                        && Instant::now() + wait < deadline =>
                {
                    sleep(wait).await;
                    wait = (wait * 2).min(RETRY_MAX);
                }
                Err(e) => {
                    return Err(io::Error::new(
                        e.io_kind().unwrap_or(io::ErrorKind::Other),
                        e,
                    ))
                }
            }
        };
        // A connection that already waited completes the receiver now.
        port.changed.notify_one();
        let _registered = Registered { port: &port, id };
        rx.await
            .map_err(|_| io::Error::new(io::ErrorKind::ConnectionAborted, "the data port closed"))
    }
}

/// Accepts the connections of one port and hands them to the waiters, until
/// nobody waits and no connection waits for anybody.
async fn serve(inner: Rc<Inner>, addr: SocketAddr, port: Rc<Port>, listener: KernelListener) {
    loop {
        let deadline = {
            let mut waiting = port.waiting.borrow_mut();
            waiting.expire(Instant::now());
            if waiting.is_idle() {
                // Removing the entry and dropping the listener happen in one
                // step, so a later waiter binds the port again.
                inner.ports.borrow_mut().remove(&addr);
                return;
            }
            waiting.next_deadline()
        };
        tokio::select! {
            accepted = listener.accept() => match accepted {
                Ok((stream, peer)) => {
                    port.waiting.borrow_mut().offer(peer.ip(), stream, Instant::now());
                }
                // One failed accept, such as EMFILE, leaves the listener usable.
                Err(_) => tokio::task::yield_now().await,
            },
            () = port.changed.notified() => {}
            () = async {
                match deadline {
                    Some(d) => sleep_until(d).await,
                    None => std::future::pending().await,
                }
            } => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::{TcpSocket, TcpStream};
    use tokio::task::LocalSet;

    use super::*;

    async fn dial(from: u8, to: SocketAddr) -> TcpStream {
        let socket = TcpSocket::new_v4().expect("a socket");
        socket
            .bind(SocketAddr::from(([127, 0, 0, from], 0)))
            .expect("a source address");
        socket.connect(to).await.expect("the port accepts")
    }

    fn free_port() -> SocketAddr {
        let l = std::net::TcpListener::bind(("127.0.0.1", 0)).expect("a free port");
        l.local_addr().expect("an address")
    }

    /// The tests tell the connections apart by the address that dialed.
    fn peer_ip(stream: &KernelStream) -> IpAddr {
        stream.peer_addr().expect("a peer").ip()
    }

    #[tokio::test(flavor = "current_thread")]
    async fn two_waiters_on_one_port_each_get_their_own_connection() {
        LocalSet::new()
            .run_until(async {
                let ports = DataPorts::new(ListenConfig::default());
                let addr = free_port();
                let one = tokio::task::spawn_local({
                    let ports = ports.clone();
                    async move { ports.accept(addr, Some(IpAddr::from([127, 0, 0, 2]))).await }
                });
                let two = tokio::task::spawn_local({
                    let ports = ports.clone();
                    async move { ports.accept(addr, Some(IpAddr::from([127, 0, 0, 3]))).await }
                });
                tokio::task::yield_now().await;
                // The second host dials first, to the listener of the first waiter.
                let _b = dial(3, addr).await;
                let _a = dial(2, addr).await;
                let (one, two) = (one.await.expect("task"), two.await.expect("task"));
                assert_eq!(peer_ip(&one.expect("first")), IpAddr::from([127, 0, 0, 2]));
                assert_eq!(peer_ip(&two.expect("second")), IpAddr::from([127, 0, 0, 3]));
                // Both are served, so the port is free again.
                tokio::time::sleep(Duration::from_millis(10)).await;
                assert!(ports.inner.ports.borrow().is_empty());
                std::net::TcpListener::bind(addr).expect("the port is free");
            })
            .await;
    }

    #[tokio::test(flavor = "current_thread")]
    async fn a_connection_that_comes_before_its_waiter_waits_for_it() {
        LocalSet::new()
            .run_until(async {
                let ports = DataPorts::new(ListenConfig::default());
                let addr = free_port();
                let first = tokio::task::spawn_local({
                    let ports = ports.clone();
                    async move { ports.accept(addr, Some(IpAddr::from([127, 0, 0, 2]))).await }
                });
                tokio::task::yield_now().await;
                // The host of a second instance dials before that instance
                // waits, and the first waiter must not take the connection.
                let mut early = dial(3, addr).await;
                tokio::time::sleep(Duration::from_millis(20)).await;
                assert!(!first.is_finished());
                let late = ports
                    .accept(addr, Some(IpAddr::from([127, 0, 0, 3])))
                    .await
                    .expect("the early connection");
                assert_eq!(peer_ip(&late), IpAddr::from([127, 0, 0, 3]));
                let _a = dial(2, addr).await;
                assert_eq!(
                    peer_ip(&first.await.expect("task").expect("first")),
                    IpAddr::from([127, 0, 0, 2])
                );
                // The early connection was never reset.
                early.write_all(b"x").await.expect("still open");
                let mut buf = [0u8; 1];
                let read = tokio::time::timeout(Duration::from_millis(20), early.read(&mut buf));
                assert!(read.await.is_err(), "no reset or close arrived");
            })
            .await;
    }

    #[tokio::test(flavor = "current_thread")]
    async fn a_waiter_that_gives_up_frees_the_port() {
        LocalSet::new()
            .run_until(async {
                let ports = DataPorts::new(ListenConfig::default());
                let addr = free_port();
                let waiting = tokio::time::timeout(
                    Duration::from_millis(20),
                    ports.accept(addr, Some(IpAddr::from([127, 0, 0, 2]))),
                )
                .await;
                assert!(waiting.is_err());
                tokio::time::sleep(Duration::from_millis(10)).await;
                assert!(ports.inner.ports.borrow().is_empty());
                std::net::TcpListener::bind(addr).expect("the port is free");
            })
            .await;
    }

    #[tokio::test(flavor = "current_thread")]
    async fn a_bind_waits_for_a_port_that_another_process_holds() {
        LocalSet::new()
            .run_until(async {
                let holder = std::net::TcpListener::bind(("127.0.0.1", 0)).expect("a holder");
                let addr = holder.local_addr().expect("an address");
                let ports = DataPorts::new(ListenConfig::default());
                let accept = tokio::task::spawn_local({
                    let ports = ports.clone();
                    async move { ports.accept(addr, None).await }
                });
                tokio::time::sleep(Duration::from_millis(40)).await;
                drop(holder);
                let mut dialed = None;
                for _ in 0..200 {
                    if let Ok(s) = TcpStream::connect(addr).await {
                        dialed = Some(s);
                        break;
                    }
                    tokio::time::sleep(Duration::from_millis(5)).await;
                }
                assert!(dialed.is_some(), "the port came free");
                assert!(accept.await.expect("task").is_ok());
            })
            .await;
    }
}
