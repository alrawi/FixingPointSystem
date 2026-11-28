// SPDX-License-Identifier: GPL-3.0-only
//! Both ends of N connections over an in memory wire, through the phases of
//! a short session. Each phase records the cost of its events.

use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;
use std::time::Instant;

use smoltcp::socket::tcp::State;
use smoltcp::wire::Ipv4Address;

use crate::dev::Pool;
use crate::stats::{rss_bytes, Cat};
use crate::table::{Host, SocketParams};

/// Port of the server.
pub const SERVER_PORT: u16 = 80;
/// First client port.
pub const FIRST_PORT: u16 = 1024;
/// Bytes in a request and in a response.
pub const MSG: usize = 64;

/// The cost of the events of one phase.
#[derive(Clone, Debug, Default)]
pub struct Phase {
    /// A frame given to a flow, with the sending that it caused.
    pub ingest: Cat,
    /// A write or a read of the application, with the sending that it caused.
    pub app: Cat,
    /// A timer that fired for a live flow.
    pub fire: Cat,
    /// One tick of the wheel, with the fires inside it.
    pub tick: Cat,
    /// Wall time of the whole phase in ns.
    pub wall_ns: u64,
}

/// What a run measured and checked.
#[derive(Clone, Debug, Default)]
pub struct Report {
    /// Connections.
    pub n: usize,
    /// Handshake: connect, SYN, SYN-ACK, ACK.
    pub handshake: Phase,
    /// Request and response on every flow, as one burst per round.
    pub burst: Phase,
    /// One request and response at a time, with all flows resident.
    pub single: Phase,
    /// The first transmission of a request is lost on every flow; the
    /// retransmission timer recovers it.
    pub loss: Phase,
    /// Close from the client side and the wait through TIME-WAIT.
    pub close: Phase,
    /// Resident memory per socket (both ends counted) after the handshake.
    pub bytes_per_socket: f64,
    /// Fires after which a flow sent a frame, in the loss phase (client side).
    pub retransmit_fires: u64,
    /// Frames the loss phase dropped.
    pub dropped: u64,
    /// Timer entries that fired for a live flow, both hosts, whole run.
    pub fires: u64,
    /// Of those, the fires after which the flow sent a frame, both hosts.
    pub fires_with_frame: u64,
    /// Frames the server host took in total.
    pub server_ingested: u64,
    /// Frames that matched no flow, both hosts.
    pub orphans: u64,
    /// Stale wheel entries dropped, both hosts.
    pub stale: u64,
    /// Entries left in the wheels after the close phase.
    pub wheel_left: usize,
    /// Flows left after the close phase.
    pub flows_left: usize,
    /// Failed checks, empty when every check held.
    pub failures: Vec<String>,
}

struct Rig {
    client: Host,
    server: Host,
    /// Frames from the client to the server.
    c2s: VecDeque<Vec<u8>>,
    /// Frames from the server to the client.
    s2c: VecDeque<Vec<u8>>,
    now: i64,
    /// Client flow of each connection.
    cflows: Vec<usize>,
    /// Connection of each client flow.
    conn_of: Vec<u32>,
    /// Requests sent and responses received, per connection.
    sent: Vec<u32>,
    received: Vec<u32>,
    /// Drop every frame that the client writes, while set.
    drop_c2s: bool,
    dropped: u64,
    failures: Vec<String>,
}

/// The bytes of request `round` on connection `conn`. The server echoes them.
fn payload(conn: usize, round: usize) -> [u8; MSG] {
    let mut p = [0u8; MSG];
    for (i, b) in p.iter_mut().enumerate() {
        *b = (conn.wrapping_mul(31) ^ round.wrapping_mul(7) ^ i) as u8;
    }
    p
}

struct Xorshift(u64);

impl Xorshift {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn shuffled(&mut self, n: usize) -> Vec<usize> {
        let mut v: Vec<usize> = (0..n).collect();
        for i in (1..n).rev() {
            let j = (self.next() % (i as u64 + 1)) as usize;
            v.swap(i, j);
        }
        v
    }
}

impl Rig {
    fn new(n: usize, params: SocketParams) -> Self {
        let pool: Pool = Rc::new(RefCell::new(Vec::new()));
        let c = Ipv4Address::new(10, 0, 0, 1);
        let s = Ipv4Address::new(10, 0, 0, 2);
        Self {
            client: Host::new(pool.clone(), c, &[], params),
            server: Host::new(pool, s, &[SERVER_PORT], params),
            c2s: VecDeque::new(),
            s2c: VecDeque::new(),
            now: 1_000,
            cflows: Vec::with_capacity(n),
            conn_of: Vec::with_capacity(n),
            sent: vec![0; n],
            received: vec![0; n],
            drop_c2s: false,
            dropped: 0,
            failures: Vec::new(),
        }
    }

    fn fail(&mut self, msg: String) {
        if self.failures.len() < 20 {
            self.failures.push(msg);
        }
    }

    /// Moves the frames that the hosts wrote onto the wire.
    fn collect(&mut self) {
        let mut out = Vec::new();
        self.client.drain_out(|f| out.push(f));
        for f in out {
            if self.drop_c2s {
                self.dropped += 1;
                self.client.recycle(f);
            } else {
                self.c2s.push_back(f);
            }
        }
        let s2c = &mut self.s2c;
        self.server.drain_out(|f| s2c.push_back(f));
    }

    /// Runs the wire until it is quiet. The server application echoes every
    /// request, the client application checks every response.
    fn pump(&mut self, ph: &mut Phase) {
        loop {
            self.collect();
            if let Some(f) = self.c2s.pop_front() {
                let t0 = Instant::now();
                let idx = self.server.ingest(self.now, f);
                ph.ingest.rec(t0.elapsed().as_nanos() as u64);
                if let Some(idx) = idx {
                    self.server_app(idx, ph);
                }
            } else if let Some(f) = self.s2c.pop_front() {
                let t0 = Instant::now();
                let idx = self.client.ingest(self.now, f);
                ph.ingest.rec(t0.elapsed().as_nanos() as u64);
                if let Some(idx) = idx {
                    self.client_app(idx, ph);
                }
            } else {
                break;
            }
        }
    }

    fn server_app(&mut self, idx: usize, ph: &mut Phase) {
        let state = self.server.state(idx);
        if state != Some(State::Established) && state != Some(State::CloseWait) {
            return;
        }
        let mut buf = [0u8; MSG];
        let t0 = Instant::now();
        let got = self.server.recv(self.now, idx, &mut buf);
        if got == MSG {
            self.server.send(self.now, idx, &buf);
            ph.app.rec(t0.elapsed().as_nanos() as u64);
        } else if got > 0 {
            self.fail(format!("server read {got} bytes, not {MSG}"));
        }
        if state == Some(State::CloseWait) {
            self.server.close(self.now, idx);
        }
    }

    fn client_app(&mut self, idx: usize, ph: &mut Phase) {
        if self.client.state(idx) != Some(State::Established) {
            return;
        }
        let mut buf = [0u8; MSG];
        let t0 = Instant::now();
        let got = self.client.recv(self.now, idx, &mut buf);
        if got == 0 {
            return;
        }
        ph.app.rec(t0.elapsed().as_nanos() as u64);
        let conn = self.conn_of[idx] as usize;
        let round = self.received[conn] as usize;
        self.received[conn] += 1;
        if got != MSG || buf != payload(conn, round) {
            self.fail(format!(
                "conn {conn} round {round}: wrong response ({got} bytes)"
            ));
        }
    }

    fn request(&mut self, conn: usize, ph: &mut Phase) {
        let round = self.sent[conn] as usize;
        self.sent[conn] += 1;
        let t0 = Instant::now();
        let n = self
            .client
            .send(self.now, self.cflows[conn], &payload(conn, round));
        ph.app.rec(t0.elapsed().as_nanos() as u64);
        if n != MSG {
            self.fail(format!("conn {conn}: send took {n} bytes"));
        }
    }

    /// Advances the virtual clock by one millisecond and ticks both wheels.
    fn tick_ms(&mut self, ph: &mut Phase) {
        self.now += 1000;
        let t0 = Instant::now();
        self.client.tick(self.now, Some(&mut ph.fire));
        ph.tick.rec(t0.elapsed().as_nanos() as u64);
        let t0 = Instant::now();
        self.server.tick(self.now, Some(&mut ph.fire));
        ph.tick.rec(t0.elapsed().as_nanos() as u64);
    }

    fn all_answered(&self) -> bool {
        self.sent == self.received
    }
}

/// Runs `n` connections through the phases. `rounds` is the number of burst
/// rounds, `single` the number of isolated round trips. `n` is at most 64,000.
pub fn run(n: usize, rounds: usize, single: usize) -> Report {
    if n == 0 || n > 64_000 {
        return Report {
            n,
            failures: vec!["n is 1 to 64,000".into()],
            ..Report::default()
        };
    }
    let params = SocketParams::default();
    let mut rig = Rig::new(n, params);
    let mut rng = Xorshift(0x9e37_79b9_7f4a_7c15);
    let mut rep = Report {
        n,
        ..Report::default()
    };
    let server_addr = rig.server.addr();

    // Handshake: every client connects, then the wire runs.
    let rss0 = rss_bytes();
    let t_phase = Instant::now();
    let mut ph = Phase::default();
    for i in 0..n {
        let t0 = Instant::now();
        let Ok(idx) = rig
            .client
            .connect(rig.now, server_addr, SERVER_PORT, FIRST_PORT + i as u16)
        else {
            rig.fail(format!("connect {i} failed"));
            rep.failures = rig.failures;
            return rep;
        };
        ph.app.rec(t0.elapsed().as_nanos() as u64);
        if rig.conn_of.len() <= idx {
            rig.conn_of.resize(idx + 1, 0);
        }
        rig.conn_of[idx] = i as u32;
        rig.cflows.push(idx);
    }
    rig.pump(&mut ph);
    ph.wall_ns = t_phase.elapsed().as_nanos() as u64;
    rep.bytes_per_socket = rss_bytes().saturating_sub(rss0) as f64 / (2 * n) as f64;
    let (ce, se) = (
        rig.client.count_state(State::Established),
        rig.server.count_state(State::Established),
    );
    if ce != n || se != n {
        rig.fail(format!(
            "after the handshake {ce} client and {se} server flows are established, not {n}"
        ));
    }
    rep.handshake = ph;

    // Burst: every flow sends one request, in random order, then the wire runs.
    let t_phase = Instant::now();
    let mut ph = Phase::default();
    for _ in 0..rounds {
        for conn in rng.shuffled(n) {
            rig.request(conn, &mut ph);
        }
        rig.pump(&mut ph);
        rig.tick_ms(&mut ph);
    }
    ph.wall_ns = t_phase.elapsed().as_nanos() as u64;
    if !rig.all_answered() {
        rig.fail("burst: not every request was answered".into());
    }
    rep.burst = ph;

    // Single: one request at a time, with all flows resident.
    let t_phase = Instant::now();
    let mut ph = Phase::default();
    for conn in rng.shuffled(n).into_iter().take(single) {
        rig.request(conn, &mut ph);
        rig.pump(&mut ph);
    }
    rig.tick_ms(&mut ph);
    ph.wall_ns = t_phase.elapsed().as_nanos() as u64;
    if !rig.all_answered() {
        rig.fail("single: not every request was answered".into());
    }
    rep.single = ph;

    // Loss: the first transmission of every request is dropped. The
    // retransmission timer of every flow has to recover it, with no frame
    // arriving to drive the socket.
    let t_phase = Instant::now();
    let mut ph = Phase::default();
    let fires0 = rig.client.stats.fires_with_frame;
    rig.drop_c2s = true;
    for conn in rng.shuffled(n) {
        rig.request(conn, &mut ph);
    }
    rig.collect();
    rig.drop_c2s = false;
    let mut ticks = 0;
    while !rig.all_answered() && ticks < 10_000 {
        rig.tick_ms(&mut ph);
        rig.pump(&mut ph);
        ticks += 1;
    }
    ph.wall_ns = t_phase.elapsed().as_nanos() as u64;
    if !rig.all_answered() {
        rig.fail(format!(
            "loss: not every request was recovered after {ticks} ms"
        ));
    }
    rep.dropped = rig.dropped;
    rep.retransmit_fires = rig.client.stats.fires_with_frame - fires0;
    rep.loss = ph;

    // Close: the client closes every flow and the wheel runs through
    // TIME-WAIT (ten seconds in smoltcp).
    let t_phase = Instant::now();
    let mut ph = Phase::default();
    for conn in 0..n {
        let t0 = Instant::now();
        rig.client.close(rig.now, rig.cflows[conn]);
        ph.app.rec(t0.elapsed().as_nanos() as u64);
    }
    rig.pump(&mut ph);
    let mut ticks = 0;
    while (rig.client.flows() > 0 || rig.server.flows() > 0) && ticks < 15_000 {
        rig.tick_ms(&mut ph);
        rig.pump(&mut ph);
        ticks += 1;
    }
    ph.wall_ns = t_phase.elapsed().as_nanos() as u64;
    rep.flows_left = rig.client.flows() + rig.server.flows();
    if rep.flows_left > 0 {
        rig.fail(format!(
            "close: {} flows left after {ticks} ms",
            rep.flows_left
        ));
    }
    rep.close = ph;

    rep.fires = rig.server.stats.fires + rig.client.stats.fires;
    rep.fires_with_frame = rig.server.stats.fires_with_frame + rig.client.stats.fires_with_frame;
    rep.server_ingested = rig.server.stats.ingested;
    rep.orphans = rig.server.stats.orphans + rig.client.stats.orphans;
    rep.stale = rig.server.stats.stale + rig.client.stats.stale;
    rep.wheel_left = rig.server.wheel_len() + rig.client.wheel_len();
    rep.failures = rig.failures;
    rep
}
