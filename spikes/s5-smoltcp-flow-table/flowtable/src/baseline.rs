// SPDX-License-Identifier: GPL-3.0-only
//! The interface loop of smoltcp over one socket set per host, as the harness
//! of `engine/fps-tcp` uses it. Every event calls `Interface::poll` and
//! `Interface::poll_at`, and both visit every socket of the set.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::time::Instant;

use smoltcp::iface::{Config, Interface, SocketHandle, SocketSet};
use smoltcp::socket::tcp::{self, State};
use smoltcp::time::Instant as SmolInstant;
use smoltcp::wire::{HardwareAddress, IpAddress, IpCidr, Ipv4Address};

use crate::dev::{FrameDev, Pool};
use crate::scenario::{FIRST_PORT, MSG, SERVER_PORT};
use crate::stats::Cat;
use crate::table::SocketParams;

struct Side {
    iface: Interface,
    dev: FrameDev,
    set: SocketSet<'static>,
}

impl Side {
    fn new(pool: Pool, addr: Ipv4Address) -> Self {
        let mut dev = FrameDev::new(pool);
        let mut config = Config::new(HardwareAddress::Ip);
        config.random_seed = u64::from(u32::from_be_bytes(addr.octets())) | 1 << 40;
        let mut iface = Interface::new(config, &mut dev, SmolInstant::from_micros(0));
        iface.update_ip_addrs(|a| {
            a.push(IpCidr::new(IpAddress::Ipv4(addr), 8)).ok();
        });
        Self {
            iface,
            dev,
            set: SocketSet::new(Vec::new()),
        }
    }

    /// One turn of the loop: poll, then ask for the next deadline.
    fn turn(&mut self, now_us: i64) {
        let now = SmolInstant::from_micros(now_us);
        self.iface.poll(now, &mut self.dev, &mut self.set);
        std::hint::black_box(self.iface.poll_at(now, &self.set));
    }
}

/// What the baseline measured.
#[derive(Clone, Debug, Default)]
pub struct BaselineReport {
    /// Connections.
    pub n: usize,
    /// One `poll` plus `poll_at` of one side per event, in isolated round trips.
    pub turn: Cat,
    /// Failed checks.
    pub failures: Vec<String>,
}

/// Opens `n` connections over two socket sets (the server has `n` listening
/// sockets on one port), then measures `single` isolated request and
/// response round trips: each event is one turn of the loop of the side that
/// has something to do.
pub fn run(n: usize, single: usize) -> BaselineReport {
    let pool: Pool = Rc::new(RefCell::new(Vec::new()));
    let ca = Ipv4Address::new(10, 0, 0, 1);
    let sa = Ipv4Address::new(10, 0, 0, 2);
    let params = SocketParams::default();
    let mut c = Side::new(pool.clone(), ca);
    let mut s = Side::new(pool, sa);
    let mut rep = BaselineReport {
        n,
        ..BaselineReport::default()
    };
    let now = 1_000i64;

    let mut chandles: Vec<SocketHandle> = Vec::with_capacity(n);
    for _ in 0..n {
        let mut sock = params.socket();
        if sock.listen(SERVER_PORT).is_err() {
            rep.failures.push("listen failed".into());
            return rep;
        }
        s.set.add(sock);
    }
    for i in 0..n {
        let mut sock = params.socket();
        let port = FIRST_PORT + i as u16;
        if sock
            .connect(c.iface.context(), (IpAddress::Ipv4(sa), SERVER_PORT), port)
            .is_err()
        {
            rep.failures.push("connect failed".into());
            return rep;
        }
        chandles.push(c.set.add(sock));
    }

    // Handshake in bulk: each turn handles all queued frames.
    for _ in 0..8 {
        c.turn(now);
        s.dev.rx.extend(c.dev.out.drain(..));
        s.turn(now);
        c.dev.rx.extend(s.dev.out.drain(..));
    }
    c.turn(now);
    s.dev.rx.extend(c.dev.out.drain(..));
    s.turn(now);
    let established = c
        .set
        .iter()
        .filter(|(_, x)| matches!(x, smoltcp::socket::Socket::Tcp(t) if t.state() == State::Established))
        .count();
    if established != n {
        rep.failures
            .push(format!("{established} of {n} client sockets established"));
    }
    // The server socket that serves each client port.
    let mut by_port: HashMap<u16, SocketHandle> = HashMap::new();
    for (h, x) in s.set.iter() {
        let smoltcp::socket::Socket::Tcp(t) = x;
        if let Some(r) = t.remote_endpoint() {
            by_port.insert(r.port, h);
        }
    }

    // Isolated round trips on a spread of connections.
    let step = (n / single.max(1)).max(1);
    let mut done = 0;
    for i in (0..n).step_by(step).take(single) {
        let port = FIRST_PORT + i as u16;
        let req = [i as u8; MSG];
        if c.set
            .get_mut::<tcp::Socket>(chandles[i])
            .send_slice(&req)
            .is_err()
        {
            rep.failures.push(format!("baseline conn {i}: send failed"));
            return rep;
        }
        let t0 = Instant::now();
        c.turn(now);
        rep.turn.rec(t0.elapsed().as_nanos() as u64);
        let mut guard = 0;
        let mut replied = false;
        loop {
            let mut moved = false;
            if !c.dev.out.is_empty() {
                s.dev.rx.extend(c.dev.out.drain(..));
                let t0 = Instant::now();
                s.turn(now);
                rep.turn.rec(t0.elapsed().as_nanos() as u64);
                moved = true;
                let Some(sh) = by_port.get(&port) else {
                    rep.failures
                        .push(format!("baseline conn {i}: no server socket"));
                    return rep;
                };
                let sock = s.set.get_mut::<tcp::Socket>(*sh);
                let mut buf = [0u8; MSG];
                if !replied {
                    if let Ok(MSG) = sock.recv_slice(&mut buf) {
                        let _ = sock.send_slice(&buf);
                        replied = true;
                        let t0 = Instant::now();
                        s.turn(now);
                        rep.turn.rec(t0.elapsed().as_nanos() as u64);
                    }
                }
            }
            if !s.dev.out.is_empty() {
                c.dev.rx.extend(s.dev.out.drain(..));
                let t0 = Instant::now();
                c.turn(now);
                rep.turn.rec(t0.elapsed().as_nanos() as u64);
                moved = true;
            }
            if !moved {
                break;
            }
            guard += 1;
            if guard > 16 {
                break;
            }
        }
        let mut buf = [0u8; MSG];
        let got = c
            .set
            .get_mut::<tcp::Socket>(chandles[i])
            .recv_slice(&mut buf)
            .unwrap_or(0);
        if got != MSG || buf != req {
            rep.failures.push(format!("baseline conn {i}: bad echo"));
        } else {
            done += 1;
        }
    }
    if done == 0 {
        rep.failures
            .push("baseline: no round trip completed".into());
    }
    rep
}
