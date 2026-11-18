// SPDX-License-Identifier: GPL-3.0-only
//! OS profiles in the handshake (TRN-28): the SYN and the SYN-ACK of a host
//! with a profile, compared byte for byte with the table of the Transport
//! page. The expected bytes below are written out by hand from that table and
//! do not come from the code under test.

use std::task::Waker;

use super::*;
use crate::pools::Pools;
use crate::profile::{LINUX6, MACOS14, WINDOWS11};

const SERVER_PORT: u16 = 443;
/// The first byte of the IP header in an Ethernet frame without a tag.
const IP: usize = 14;
/// The first byte of the TCP header in such a frame.
const TCP: usize = IP + wire::IP_LEN;
const MSS_1460: [u8; 4] = [2, 4, 5, 0xB4];

/// The three frames of a handshake and the two connections it made.
struct Handshake {
    syn: Vec<u8>,
    syn_ack: Vec<u8>,
    ack: Vec<u8>,
    client: Host,
    server: Host,
    c: (u32, u32),
    s: (u32, u32),
}

fn handshake_with(client: Host, server: Host) -> Handshake {
    let (mut client, mut server) = (client, server);
    let pools = Pools::sized("10.1.0.0/16", 1, "10.2.0.0/16", 1, None).expect("pools");
    let client_ip = pools.client.addresses().expect("addresses")[0];
    let server_ip = pools.server.addresses().expect("addresses")[0];
    server.listen(SERVER_PORT).expect("listen");
    let now = 5_000_000;
    let c = client
        .open(client_ip, server_ip, SERVER_PORT, now)
        .expect("open");
    client.flush_dirty(now);
    let syn = client.out.queue.pop_front().expect("a SYN");
    server.input(&syn, now);
    server.flush_dirty(now);
    let syn_ack = server.out.queue.pop_front().expect("a SYN-ACK");
    client.input(&syn_ack, now);
    client.flush_dirty(now);
    let ack = client.out.queue.pop_front().expect("an ACK");
    server.input(&ack, now);
    let mut ticket = None;
    let s = server
        .accept(SERVER_PORT, &mut ticket, Waker::noop())
        .expect("accept")
        .expect("a connection is queued");
    Handshake {
        syn,
        syn_ack,
        ack,
        client,
        server,
        c,
        s,
    }
}

fn host(side: Side, cfg: OwnConfig, mtu: usize, profile: Option<OsProfile>) -> Host {
    let pools = Pools::sized("10.1.0.0/16", 1, "10.2.0.0/16", 1, None).expect("pools");
    let built = pools.stations().expect("stations");
    let station = match side {
        Side::Client => built.client,
        Side::Server => built.server,
    };
    Host::new(station, cfg, mtu).with_profile(profile)
}

#[derive(Clone, Copy)]
enum Side {
    Client,
    Server,
}

/// A client with `client` and a server with `server`, at an MTU of 1500.
fn pair(client: Option<OsProfile>, server: Option<OsProfile>) -> Handshake {
    handshake_with(
        host(Side::Client, OwnConfig::default(), 1500, client),
        host(Side::Server, OwnConfig::default(), 1500, server),
    )
}

fn ttl(frame: &[u8]) -> u8 {
    frame[IP + 8]
}

fn dont_fragment(frame: &[u8]) -> bool {
    frame[IP + 6] & 0x40 != 0
}

/// The window field of the TCP header.
fn window(frame: &[u8]) -> u16 {
    u16::from_be_bytes([frame[TCP + 14], frame[TCP + 15]])
}

/// The raw option bytes of the TCP header, with the padding.
fn options(frame: &[u8]) -> &[u8] {
    let end = TCP + usize::from(frame[TCP + 12] >> 4) * 4;
    &frame[TCP + wire::TCP_LEN..end]
}

/// The timestamp value and echo of the first timestamp option.
fn timestamp(frame: &[u8]) -> (u32, u32) {
    wire::parse_frame(frame)
        .expect("a valid frame")
        .header
        .opts
        .timestamp
        .expect("a timestamp option")
}

/// `bytes` with the four bytes at `at` and `at + 4` set to the timestamp.
fn with_timestamp(mut bytes: Vec<u8>, at: usize, value: u32, echo: u32) -> Vec<u8> {
    bytes[at..at + 4].copy_from_slice(&value.to_be_bytes());
    bytes[at + 4..at + 8].copy_from_slice(&echo.to_be_bytes());
    bytes
}

/// Option bytes from pieces, to keep the expectations readable.
fn cat(parts: &[&[u8]]) -> Vec<u8> {
    parts.concat()
}

// The table of the Transport page, as bytes. Windows 11 sends no timestamp:
//   mss, nop, ws 8, nop, nop, sok
fn windows_options() -> Vec<u8> {
    cat(&[&MSS_1460, &[1, 3, 3, 8], &[1, 1, 4, 2]])
}

// Linux 6: mss, sok, ts, nop, ws 7. The timestamp bytes are zero here.
fn linux_options() -> Vec<u8> {
    cat(&[&MSS_1460, &[4, 2, 8, 10], &[0; 8], &[1, 3, 3, 7]])
}

// macOS 14: mss, nop, ws 6, nop, nop, ts, sok, eol, and one byte of padding.
fn macos_options() -> Vec<u8> {
    cat(&[
        &MSS_1460,
        &[1, 3, 3, 6],
        &[1, 1, 8, 10],
        &[0; 8],
        &[4, 2, 0, 0],
    ])
}

/// The offset of the timestamp bytes in the option layouts above.
const LINUX_TS_AT: usize = 8;
const MACOS_TS_AT: usize = 12;

#[test]
fn the_syn_of_windows11_matches_the_table() {
    let h = pair(Some(WINDOWS11), None);
    assert_eq!(ttl(&h.syn), 128);
    assert!(dont_fragment(&h.syn));
    assert_eq!(window(&h.syn), 64240);
    assert_eq!(options(&h.syn), windows_options());
}

#[test]
fn the_syn_of_linux6_matches_the_table() {
    let h = pair(Some(LINUX6), None);
    assert_eq!(ttl(&h.syn), 64);
    assert!(dont_fragment(&h.syn));
    assert_eq!(window(&h.syn), 64240);
    let (value, echo) = timestamp(&h.syn);
    assert_eq!(echo, 0, "a SYN echoes nothing");
    assert_eq!(
        options(&h.syn),
        with_timestamp(linux_options(), LINUX_TS_AT, value, echo)
    );
}

#[test]
fn the_syn_of_macos14_matches_the_table() {
    let h = pair(Some(MACOS14), None);
    assert_eq!(ttl(&h.syn), 64);
    assert!(dont_fragment(&h.syn));
    assert_eq!(window(&h.syn), 65535);
    let (value, echo) = timestamp(&h.syn);
    assert_eq!(echo, 0);
    assert_eq!(
        options(&h.syn),
        with_timestamp(macos_options(), MACOS_TS_AT, value, echo)
    );
    assert_eq!(options(&h.syn).len() % 4, 0);
}

#[test]
fn the_syn_ack_of_each_profile_matches_the_table() {
    // The client offers every option, so the server answers with all it has.
    let h = pair(None, Some(WINDOWS11));
    assert_eq!(ttl(&h.syn_ack), 128);
    assert!(dont_fragment(&h.syn_ack));
    assert_eq!(window(&h.syn_ack), 64240);
    assert_eq!(options(&h.syn_ack), windows_options());

    let h = pair(None, Some(LINUX6));
    assert_eq!(ttl(&h.syn_ack), 64);
    assert_eq!(window(&h.syn_ack), 64240);
    let (value, echo) = timestamp(&h.syn_ack);
    assert_eq!(echo, timestamp(&h.syn).0, "the SYN-ACK echoes the SYN");
    assert_eq!(
        options(&h.syn_ack),
        with_timestamp(linux_options(), LINUX_TS_AT, value, echo)
    );

    let h = pair(None, Some(MACOS14));
    assert_eq!(ttl(&h.syn_ack), 64);
    assert_eq!(window(&h.syn_ack), 65535);
    let (value, echo) = timestamp(&h.syn_ack);
    assert_eq!(echo, timestamp(&h.syn).0);
    assert_eq!(
        options(&h.syn_ack),
        with_timestamp(macos_options(), MACOS_TS_AT, value, echo)
    );
}

#[test]
fn a_syn_ack_leaves_out_what_the_peer_did_not_offer_with_its_padding() {
    // A Windows client offers no timestamp and a macOS server answers
    // without one: its order, minus the timestamp and the padding before it.
    let h = pair(Some(WINDOWS11), Some(MACOS14));
    assert_eq!(
        options(&h.syn_ack),
        cat(&[&MSS_1460, &[1, 3, 3, 6], &[4, 2, 0, 0]])
    );
    // A Linux server answers it as the kernel does: nop nop sok, nop ws.
    let h = pair(Some(WINDOWS11), Some(LINUX6));
    assert_eq!(
        options(&h.syn_ack),
        cat(&[&MSS_1460, &[1, 1, 4, 2], &[1, 3, 3, 7]])
    );
    // A client without window scaling and SACK gets only the MSS back, and
    // the window of the SYN-ACK is still the profile's.
    let client = host(
        Side::Client,
        OwnConfig {
            window_scale: false,
            timestamps: false,
            sack: false,
            ..OwnConfig::default()
        },
        1500,
        None,
    );
    let h = handshake_with(
        client,
        host(Side::Server, OwnConfig::default(), 1500, Some(WINDOWS11)),
    );
    assert_eq!(options(&h.syn_ack), MSS_1460);
    assert_eq!(window(&h.syn_ack), 64240);
}

#[test]
fn every_frame_of_a_host_carries_the_ttl_of_its_profile() {
    let h = pair(Some(WINDOWS11), Some(LINUX6));
    assert_eq!(ttl(&h.syn), 128);
    assert_eq!(ttl(&h.ack), 128, "the handshake ACK is not a SYN");
    assert_eq!(ttl(&h.syn_ack), 64);
    // A reset to a port that nobody listens on also shows the profile.
    let mut server = host(Side::Server, OwnConfig::default(), 1500, Some(WINDOWS11));
    let client = host(Side::Client, OwnConfig::default(), 1500, None);
    let mut client = client;
    let pools = Pools::sized("10.1.0.0/16", 1, "10.2.0.0/16", 1, None).expect("pools");
    let (cip, sip) = (
        pools.client.addresses().expect("addresses")[0],
        pools.server.addresses().expect("addresses")[0],
    );
    client.open(cip, sip, 9, 0).expect("open");
    client.flush_dirty(0);
    let syn = client.out.queue.pop_front().expect("a SYN");
    server.input(&syn, 0);
    let rst = server.out.queue.pop_front().expect("a RST");
    assert_eq!(rst[TCP + 13] & wire::flags::RST, wire::flags::RST);
    assert_eq!(ttl(&rst), 128);
}

#[test]
fn a_host_without_a_profile_is_unchanged() {
    let h = pair(None, None);
    assert_eq!(ttl(&h.syn), 64);
    assert_eq!(window(&h.syn), 65535);
    let (value, _) = timestamp(&h.syn);
    assert_eq!(
        options(&h.syn),
        with_timestamp(linux_options_with_shift(5), LINUX_TS_AT, value, 0),
        "the default offer is the kernel order and a shift from the buffer"
    );
}

/// The kernel order with the window scale shift `shift`.
fn linux_options_with_shift(shift: u8) -> Vec<u8> {
    cat(&[&MSS_1460, &[4, 2, 8, 10], &[0; 8], &[1, 3, 3, shift]])
}

#[test]
fn the_kernel_order_of_linux6_is_what_the_stack_always_wrote() {
    // The profile has no order of its own writer: with every option it gives
    // the same bytes as no profile, apart from the shift and the window.
    let with = pair(Some(LINUX6), None);
    let without = pair(None, None);
    let (value, _) = timestamp(&with.syn);
    assert_eq!(
        options(&with.syn),
        with_timestamp(linux_options_with_shift(7), LINUX_TS_AT, value, 0)
    );
    assert_eq!(options(&without.syn).len(), options(&with.syn).len());
}

#[test]
fn the_profile_sets_the_scales_and_the_options_of_both_connections() {
    let mut h = pair(Some(WINDOWS11), Some(MACOS14));
    let (c, s) = (h.c, h.s);
    let client = h.client.conn(c.0).expect("client connection");
    // Each side scales its window by its own shift and reads the other's.
    assert_eq!((client.rcv_shift(), client.snd_shift()), (8, 6));
    // Windows 11 sends no timestamp, so the connection has none.
    assert!(client.has(O_WS) && client.has(O_SACK) && !client.has(O_TS));
    let server = h.server.conn(s.0).expect("server connection");
    assert_eq!((server.rcv_shift(), server.snd_shift()), (6, 8));
    assert!(server.has(O_WS) && server.has(O_SACK) && !server.has(O_TS));
}

#[test]
fn a_profile_beats_the_option_fields_of_the_configuration() {
    // The configuration turns every option off and Linux 6 turns them on.
    let off = OwnConfig {
        window_scale: false,
        timestamps: false,
        sack: false,
        ..OwnConfig::default()
    };
    let h = handshake_with(
        host(Side::Client, off.clone(), 1500, Some(LINUX6)),
        host(Side::Server, off, 1500, Some(LINUX6)),
    );
    assert_eq!(options(&h.syn).len(), 20);
    // And the other way round: Windows 11 sends no timestamp whatever the
    // configuration says.
    let on = OwnConfig::default();
    let h = handshake_with(
        host(Side::Client, on.clone(), 1500, Some(WINDOWS11)),
        host(Side::Server, on, 1500, None),
    );
    assert_eq!(options(&h.syn), windows_options());
}

#[test]
fn the_mss_of_a_profile_only_lowers_to_the_link() {
    // MTU 9000: the profile's 1460 stays, as `TCP_MAXSEG` keeps it in the
    // kernel backend. MTU 1000: the link decides, 960.
    for (mtu, mss) in [(9000, 1460u16), (1500, 1460), (1000, 960)] {
        let h = handshake_with(
            host(Side::Client, OwnConfig::default(), mtu, Some(WINDOWS11)),
            host(Side::Server, OwnConfig::default(), mtu, Some(WINDOWS11)),
        );
        for frame in [&h.syn, &h.syn_ack] {
            let parsed = wire::parse_frame(frame).expect("a valid frame");
            assert_eq!(parsed.header.mss, Some(mss), "mtu {mtu}");
        }
    }
}

#[test]
fn the_syn_window_never_promises_more_than_the_buffer() {
    let small = OwnConfig {
        rcv_buffer: 8192,
        ..OwnConfig::default()
    };
    let h = handshake_with(
        host(Side::Client, small, 1500, Some(MACOS14)),
        host(Side::Server, OwnConfig::default(), 1500, None),
    );
    assert_eq!(window(&h.syn), 8192);
}

#[test]
fn the_parser_reads_the_syns_of_every_profile() {
    for profile in [WINDOWS11, LINUX6, MACOS14] {
        let h = pair(Some(profile), Some(profile));
        for frame in [&h.syn, &h.syn_ack] {
            let parsed = wire::parse_frame(frame).expect("a valid frame");
            assert_eq!(parsed.header.mss, Some(1460), "{}", profile.name);
            assert_eq!(
                parsed.header.opts.wscale,
                Some(profile.window_scale),
                "{}",
                profile.name
            );
            assert!(parsed.header.opts.sack_permitted, "{}", profile.name);
            assert_eq!(
                parsed.header.opts.timestamp.is_some(),
                profile.timestamps,
                "{}",
                profile.name
            );
        }
    }
}
