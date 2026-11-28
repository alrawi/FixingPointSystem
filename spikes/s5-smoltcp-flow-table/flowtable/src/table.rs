// SPDX-License-Identifier: GPL-3.0-only
//! The flow table: one smoltcp TCP socket per flow, each in a socket set of
//! its own, behind one shared smoltcp interface, a hash map of our own, and
//! the timer wheel of [`crate::wheel`].
//!
//! Nothing calls `Interface::poll`. For a frame, the table parses the
//! 4-tuple, finds the flow, hands the frame to the device, and calls
//! `poll_ingress_single` with the one-socket set of that flow. After every
//! event on a flow (a frame, a call of the application, a timer) it calls
//! `poll_egress` until the set has nothing to send, and `poll_at` once, which
//! gives the next deadline of the socket for the wheel. smoltcp visits one
//! socket per call, whatever the number of flows.

use std::collections::HashMap;
use std::hash::{BuildHasherDefault, Hasher};
use std::time::Instant as Clock;

use smoltcp::iface::{Config, Interface, PollResult, SocketHandle, SocketSet};
use smoltcp::socket::tcp::{self, State};
use smoltcp::time::{Duration, Instant};
use smoltcp::wire::{HardwareAddress, IpAddress, IpCidr, Ipv4Address};

use crate::dev::{FrameDev, Pool};
use crate::stats::Cat;
use crate::wheel::{Entry, Wheel};

/// The multiplicative hash of rustc ("Fx"), enough for a 64 bit key.
#[derive(Default)]
pub struct Fx(u64);

impl Hasher for Fx {
    fn finish(&self) -> u64 {
        self.0
    }
    fn write(&mut self, bytes: &[u8]) {
        for b in bytes {
            self.write_u64(u64::from(*b));
        }
    }
    fn write_u64(&mut self, v: u64) {
        self.0 = (self.0.rotate_left(5) ^ v).wrapping_mul(0x51_7c_c1_b7_27_22_0a_95);
    }
}

type FlowMap = HashMap<u64, u32, BuildHasherDefault<Fx>>;

/// Settings of the sockets of a host.
#[derive(Clone, Copy, Debug)]
pub struct SocketParams {
    /// Receive buffer in bytes.
    pub rx_buffer: usize,
    /// Transmit buffer in bytes.
    pub tx_buffer: usize,
}

impl Default for SocketParams {
    fn default() -> Self {
        Self {
            rx_buffer: 1024,
            tx_buffer: 1024,
        }
    }
}

impl SocketParams {
    /// A new socket with these buffers, Nagle off and no delayed
    /// acknowledgment, as the harness of `engine/fps-tcp` uses it.
    pub fn socket(&self) -> tcp::Socket<'static> {
        let mut socket = tcp::Socket::new(
            tcp::SocketBuffer::new(vec![0u8; self.rx_buffer]),
            tcp::SocketBuffer::new(vec![0u8; self.tx_buffer]),
        );
        socket.set_nagle_enabled(false);
        socket.set_ack_delay(None);
        socket
    }
}

/// Counters of a host.
#[derive(Clone, Debug, Default)]
pub struct HostStats {
    /// Frames given to a flow.
    pub ingested: u64,
    /// Frames that matched no flow and were not a SYN for a listened port.
    pub orphans: u64,
    /// Frames that were not IPv4 TCP.
    pub malformed: u64,
    /// Frames that smoltcp wrote.
    pub egress_frames: u64,
    /// Timer entries that fired for a live flow.
    pub fires: u64,
    /// Fires after which the flow sent a frame.
    pub fires_with_frame: u64,
    /// Timer entries dropped because their flow was gone or had re-armed.
    pub stale: u64,
    /// The most flows at once.
    pub peak_flows: usize,
}

struct Flow {
    set: SocketSet<'static>,
    handle: SocketHandle,
    key: u64,
    /// The generation of the armed timer entry, if one is armed.
    gen: u32,
    /// The tick of the armed timer entry.
    armed: Option<u64>,
}

/// The tuple of a flow as this host sees it.
pub fn flow_key(remote: Ipv4Address, remote_port: u16, local_port: u16) -> u64 {
    (u64::from(u32::from_be_bytes(remote.octets())) << 32)
        | (u64::from(remote_port) << 16)
        | u64::from(local_port)
}

/// The key of an IPv4 TCP packet addressed to this host, and whether it is a
/// SYN without ACK.
fn parse(packet: &[u8]) -> Option<(u64, u16, bool)> {
    if packet.len() < 20 || packet[0] >> 4 != 4 || packet[9] != 6 {
        return None;
    }
    let ihl = usize::from(packet[0] & 0x0f) * 4;
    if ihl < 20 || packet.len() < ihl + 20 {
        return None;
    }
    let src = Ipv4Address::new(packet[12], packet[13], packet[14], packet[15]);
    let sport = u16::from_be_bytes([packet[ihl], packet[ihl + 1]]);
    let dport = u16::from_be_bytes([packet[ihl + 2], packet[ihl + 3]]);
    let flags = packet[ihl + 13];
    Some((flow_key(src, sport, dport), dport, flags & 0x12 == 0x02))
}

/// One host: a shared interface, a flow table, and a timer wheel.
pub struct Host {
    iface: Interface,
    dev: FrameDev,
    flows: Vec<Option<Flow>>,
    free: Vec<u32>,
    map: FlowMap,
    wheel: Wheel,
    due: Vec<Entry>,
    next_gen: u32,
    local: Ipv4Address,
    listen: Vec<u16>,
    params: SocketParams,
    /// Counters.
    pub stats: HostStats,
}

fn ts(now_us: i64) -> Instant {
    Instant::from_micros(now_us)
}

impl Host {
    /// A host at `local` that listens on `listen`.
    pub fn new(pool: Pool, local: Ipv4Address, listen: &[u16], params: SocketParams) -> Self {
        let mut dev = FrameDev::new(pool);
        let mut config = Config::new(HardwareAddress::Ip);
        config.random_seed = u64::from(u32::from_be_bytes(local.octets())) | 1 << 40;
        let mut iface = Interface::new(config, &mut dev, ts(0));
        iface.update_ip_addrs(|addrs| {
            addrs.push(IpCidr::new(IpAddress::Ipv4(local), 8)).ok();
        });
        Self {
            iface,
            dev,
            flows: Vec::new(),
            free: Vec::new(),
            map: FlowMap::default(),
            wheel: Wheel::new(),
            due: Vec::new(),
            next_gen: 1,
            local,
            listen: listen.to_vec(),
            params,
            stats: HostStats::default(),
        }
    }

    /// The address of this host.
    pub fn addr(&self) -> Ipv4Address {
        self.local
    }

    /// Flows in the table.
    pub fn flows(&self) -> usize {
        self.map.len()
    }

    /// Entries in the timer wheel, stale ones included.
    pub fn wheel_len(&self) -> usize {
        self.wheel.len()
    }

    fn insert_flow(&mut self, flow: Flow) -> usize {
        let key = flow.key;
        let idx = if let Some(i) = self.free.pop() {
            self.flows[i as usize] = Some(flow);
            i as usize
        } else {
            self.flows.push(Some(flow));
            self.flows.len() - 1
        };
        self.map.insert(key, idx as u32);
        self.stats.peak_flows = self.stats.peak_flows.max(self.map.len());
        idx
    }

    /// Opens a connection from `local_port` to `remote`, and returns its flow.
    pub fn connect(
        &mut self,
        now_us: i64,
        remote: Ipv4Address,
        remote_port: u16,
        local_port: u16,
    ) -> Result<usize, tcp::ConnectError> {
        let mut socket = self.params.socket();
        socket.connect(
            self.iface.context(),
            (IpAddress::Ipv4(remote), remote_port),
            local_port,
        )?;
        let mut set = SocketSet::new(Vec::new());
        let handle = set.add(socket);
        let idx = self.insert_flow(Flow {
            set,
            handle,
            key: flow_key(remote, remote_port, local_port),
            gen: 0,
            armed: None,
        });
        self.settle(now_us, idx);
        Ok(idx)
    }

    /// Takes one frame from the wire. Returns the flow that the frame
    /// reached, or `None` for a frame that no flow took or that ended its
    /// flow.
    pub fn ingest(&mut self, now_us: i64, frame: Vec<u8>) -> Option<usize> {
        let Some((key, dport, syn)) = parse(&frame) else {
            self.stats.malformed += 1;
            self.dev.recycle(frame);
            return None;
        };
        let idx = match self.map.get(&key) {
            Some(i) => *i as usize,
            None if syn && self.listen.contains(&dport) => {
                let mut socket = self.params.socket();
                socket.listen(dport).ok()?;
                let mut set = SocketSet::new(Vec::new());
                let handle = set.add(socket);
                self.insert_flow(Flow {
                    set,
                    handle,
                    key,
                    gen: 0,
                    armed: None,
                })
            }
            None => {
                self.stats.orphans += 1;
                self.dev.recycle(frame);
                return None;
            }
        };
        self.stats.ingested += 1;
        self.dev.rx.push_back(frame);
        let flow = self.flows[idx].as_mut()?;
        self.iface
            .poll_ingress_single(ts(now_us), &mut self.dev, &mut flow.set);
        self.settle(now_us, idx).then_some(idx)
    }

    /// Writes to a flow. Returns the bytes taken.
    pub fn send(&mut self, now_us: i64, idx: usize, data: &[u8]) -> usize {
        let Some(flow) = self.flows[idx].as_mut() else {
            return 0;
        };
        let n = flow
            .set
            .get_mut::<tcp::Socket>(flow.handle)
            .send_slice(data)
            .unwrap_or(0);
        self.settle(now_us, idx);
        n
    }

    /// Reads from a flow. Returns the bytes read.
    pub fn recv(&mut self, now_us: i64, idx: usize, buf: &mut [u8]) -> usize {
        let Some(flow) = self.flows[idx].as_mut() else {
            return 0;
        };
        let n = flow
            .set
            .get_mut::<tcp::Socket>(flow.handle)
            .recv_slice(buf)
            .unwrap_or(0);
        if n > 0 {
            self.settle(now_us, idx);
        }
        n
    }

    /// Closes a flow gracefully.
    pub fn close(&mut self, now_us: i64, idx: usize) {
        if let Some(flow) = self.flows[idx].as_mut() {
            flow.set.get_mut::<tcp::Socket>(flow.handle).close();
            self.settle(now_us, idx);
        }
    }

    /// The state of a flow, or `None` when the flow is gone.
    pub fn state(&self, idx: usize) -> Option<State> {
        let flow = self.flows.get(idx)?.as_ref()?;
        Some(flow.set.get::<tcp::Socket>(flow.handle).state())
    }

    /// Bytes in the transmit queue of a flow (unacknowledged or unsent).
    pub fn send_queue(&self, idx: usize) -> usize {
        self.flows
            .get(idx)
            .and_then(Option::as_ref)
            .map_or(0, |f| f.set.get::<tcp::Socket>(f.handle).send_queue())
    }

    /// Counts the flows in `state`.
    pub fn count_state(&self, state: State) -> usize {
        self.flows
            .iter()
            .flatten()
            .filter(|f| f.set.get::<tcp::Socket>(f.handle).state() == state)
            .count()
    }

    /// Hands every frame that smoltcp wrote to `f`.
    pub fn drain_out(&mut self, mut f: impl FnMut(Vec<u8>)) {
        for frame in self.dev.out.drain(..) {
            f(frame);
        }
    }

    /// Gives a frame buffer back to the pool.
    pub fn recycle(&self, buf: Vec<u8>) {
        self.dev.recycle(buf);
    }

    /// A frame buffer from the pool.
    pub fn buffer(&self) -> Vec<u8> {
        self.dev.buffer()
    }

    /// Runs what a flow has to send, removes the flow when its socket is
    /// closed, and arms the timer wheel for the next deadline of the socket.
    /// Returns whether the flow is still alive.
    fn settle(&mut self, now_us: i64, idx: usize) -> bool {
        let now = ts(now_us);
        let Some(flow) = self.flows[idx].as_mut() else {
            return false;
        };
        let before = self.dev.out.len();
        for _ in 0..64 {
            if self.iface.poll_egress(now, &mut self.dev, &mut flow.set) == PollResult::None {
                break;
            }
        }
        self.stats.egress_frames += (self.dev.out.len() - before) as u64;
        let state = flow.set.get::<tcp::Socket>(flow.handle).state();
        if state == State::Closed {
            let key = flow.key;
            self.map.remove(&key);
            self.flows[idx] = None;
            self.free.push(idx as u32);
            return false;
        }
        if let Some(at) = self.iface.poll_at(now, &flow.set) {
            // The next tick after the deadline, and not before the next tick
            // of the wheel, so a socket that is due now fires at the next step.
            let tick = (at.total_micros().max(0) as u64)
                .div_ceil(1000)
                .max(self.wheel.cur() + 1);
            // An armed entry that is not later than the deadline stays: it
            // fires early, and the settle that follows arms the real deadline.
            if flow.armed.is_none_or(|armed| tick < armed) {
                let gen = self.next_gen;
                self.next_gen += 1;
                flow.gen = gen;
                flow.armed = Some(tick);
                self.wheel.insert(idx as u32, gen, tick);
            }
        }
        true
    }

    /// Moves the timer wheel to `now_us` one tick at a time, and runs the
    /// flows whose timer fell due. Each run is recorded in `fires` when given.
    /// Returns the number of runs.
    pub fn tick(&mut self, now_us: i64, mut fires: Option<&mut Cat>) -> usize {
        let target = (now_us.max(0) as u64) / 1000;
        let mut runs = 0;
        while self.wheel.cur() < target {
            self.due.clear();
            self.wheel.step(&mut self.due);
            // The step of the wheel is in `self.due`; run it entry by entry.
            let due = std::mem::take(&mut self.due);
            for e in &due {
                let idx = e.flow as usize;
                let live = self.flows[idx].as_mut().filter(|f| f.gen == e.gen);
                let Some(flow) = live else {
                    self.stats.stale += 1;
                    continue;
                };
                flow.armed = None;
                self.stats.fires += 1;
                runs += 1;
                let t0 = fires.as_ref().map(|_| Clock::now());
                let before = self.stats.egress_frames;
                self.settle(now_us, idx);
                if self.stats.egress_frames > before {
                    self.stats.fires_with_frame += 1;
                }
                if let (Some(cat), Some(t0)) = (fires.as_deref_mut(), t0) {
                    cat.rec(t0.elapsed().as_nanos() as u64);
                }
            }
            self.due = due;
        }
        runs
    }
}

/// Helper for tests: the smoltcp duration of a number of milliseconds.
pub fn millis(ms: u64) -> Duration {
    Duration::from_millis(ms)
}
