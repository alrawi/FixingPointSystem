// SPDX-License-Identifier: GPL-3.0-only
//! A minimal pcapng writer.
//!
//! The functions return the bytes of one little endian block. A file is a
//! section header, one interface description, and then packet blocks and
//! decryption secrets blocks in any order.

/// Block type of the section header block.
pub const SHB_TYPE: u32 = 0x0A0D_0D0A;
/// Block type of the interface description block.
pub const IDB_TYPE: u32 = 0x0000_0001;
/// Block type of the enhanced packet block.
pub const EPB_TYPE: u32 = 0x0000_0006;
/// Block type of the decryption secrets block.
pub const DSB_TYPE: u32 = 0x0000_000A;
/// Secrets type of a TLS key log in the NSS format, the ASCII bytes "TLSK".
pub const SECRETS_TLS_KEY_LOG: u32 = 0x544c_534b;
/// Link type of Ethernet.
pub const LINKTYPE_ETHERNET: u16 = 1;
/// The snap length that the interface description announces.
pub const SNAPLEN: u32 = 262_144;

const BYTE_ORDER_MAGIC: u32 = 0x1A2B_3C4D;
const OPT_ENDOFOPT: u16 = 0;
const OPT_IF_NAME: u16 = 2;
const OPT_IF_TSRESOL: u16 = 9;
const OPT_SHB_USERAPPL: u16 = 4;
/// The `if_tsresol` value for nanosecond timestamps.
const TSRESOL_NANOS: u8 = 9;

fn pad4(len: usize) -> usize {
    (4 - len % 4) % 4
}

fn block(block_type: u32, body: &[u8]) -> Vec<u8> {
    let padded = body.len() + pad4(body.len());
    // A block length is 32 bits by definition of the format. The callers
    // bound their bodies far below that.
    let total = u32::try_from(12 + padded).unwrap_or(u32::MAX);
    let mut out = Vec::with_capacity(12 + padded);
    out.extend_from_slice(&block_type.to_le_bytes());
    out.extend_from_slice(&total.to_le_bytes());
    out.extend_from_slice(body);
    out.resize(8 + padded, 0);
    out.extend_from_slice(&total.to_le_bytes());
    out
}

fn option(out: &mut Vec<u8>, code: u16, value: &[u8]) {
    out.extend_from_slice(&code.to_le_bytes());
    // Option values here are short constants.
    out.extend_from_slice(&u16::try_from(value.len()).unwrap_or(u16::MAX).to_le_bytes());
    out.extend_from_slice(value);
    out.resize(out.len() + pad4(value.len()), 0);
}

fn end_of_options(out: &mut Vec<u8>) {
    out.extend_from_slice(&OPT_ENDOFOPT.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
}

/// Returns the section header block, with the writing application as an option.
pub fn section_header(application: &str) -> Vec<u8> {
    let mut body = Vec::new();
    body.extend_from_slice(&BYTE_ORDER_MAGIC.to_le_bytes());
    body.extend_from_slice(&1u16.to_le_bytes());
    body.extend_from_slice(&0u16.to_le_bytes());
    // The section length is unspecified, which is -1.
    body.extend_from_slice(&(-1i64).to_le_bytes());
    option(&mut body, OPT_SHB_USERAPPL, application.as_bytes());
    end_of_options(&mut body);
    block(SHB_TYPE, &body)
}

/// Returns the interface description block of the one Ethernet interface,
/// with nanosecond timestamps.
pub fn interface_description(name: &str) -> Vec<u8> {
    let mut body = Vec::new();
    body.extend_from_slice(&LINKTYPE_ETHERNET.to_le_bytes());
    body.extend_from_slice(&0u16.to_le_bytes());
    body.extend_from_slice(&SNAPLEN.to_le_bytes());
    option(&mut body, OPT_IF_NAME, name.as_bytes());
    option(&mut body, OPT_IF_TSRESOL, &[TSRESOL_NANOS]);
    end_of_options(&mut body);
    block(IDB_TYPE, &body)
}

/// Appends an enhanced packet block on interface 0 to `out`.
///
/// `nanos` counts nanoseconds since the Unix epoch.
pub fn push_enhanced_packet(out: &mut Vec<u8>, nanos: u64, data: &[u8]) {
    let len = u32::try_from(data.len()).unwrap_or(u32::MAX);
    let mut body = Vec::with_capacity(20 + data.len());
    body.extend_from_slice(&0u32.to_le_bytes());
    body.extend_from_slice(&((nanos >> 32) as u32).to_le_bytes());
    body.extend_from_slice(&(nanos as u32).to_le_bytes());
    body.extend_from_slice(&len.to_le_bytes());
    body.extend_from_slice(&len.to_le_bytes());
    body.extend_from_slice(data);
    out.extend_from_slice(&block(EPB_TYPE, &body));
}

/// Returns a decryption secrets block that holds TLS key log lines in the NSS format.
pub fn decryption_secrets(nss_lines: &[u8]) -> Vec<u8> {
    let mut body = Vec::with_capacity(8 + nss_lines.len());
    body.extend_from_slice(&SECRETS_TLS_KEY_LOG.to_le_bytes());
    body.extend_from_slice(
        &u32::try_from(nss_lines.len())
            .unwrap_or(u32::MAX)
            .to_le_bytes(),
    );
    body.extend_from_slice(nss_lines);
    body.resize(body.len() + pad4(nss_lines.len()), 0);
    end_of_options(&mut body);
    block(DSB_TYPE, &body)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn u32_at(b: &[u8], at: usize) -> u32 {
        u32::from_le_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]])
    }

    #[test]
    fn every_block_is_aligned_and_framed_by_equal_lengths() {
        let mut all = vec![
            section_header("fps-capture test"),
            interface_description("synthetic"),
        ];
        let mut packet = Vec::new();
        push_enhanced_packet(&mut packet, 1_700_000_000_123_456_789, &[1, 2, 3, 4, 5]);
        all.push(packet);
        all.push(decryption_secrets(b"CLIENT_RANDOM aa bb\n"));
        for b in &all {
            assert_eq!(b.len() % 4, 0);
            assert_eq!(u32_at(b, 4) as usize, b.len());
            assert_eq!(u32_at(b, b.len() - 4) as usize, b.len());
        }
        assert_eq!(u32_at(&all[0], 0), SHB_TYPE);
        assert_eq!(u32_at(&all[1], 0), IDB_TYPE);
        assert_eq!(u32_at(&all[2], 0), EPB_TYPE);
        assert_eq!(u32_at(&all[3], 0), DSB_TYPE);
    }

    #[test]
    fn packet_timestamp_splits_into_high_and_low_words() {
        let nanos: u64 = (7 << 32) | 9;
        let mut out = Vec::new();
        push_enhanced_packet(&mut out, nanos, &[0; 4]);
        assert_eq!(u32_at(&out, 8), 0);
        assert_eq!(u32_at(&out, 12), 7);
        assert_eq!(u32_at(&out, 16), 9);
        assert_eq!(u32_at(&out, 20), 4);
    }
}
