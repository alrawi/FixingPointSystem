// SPDX-License-Identifier: GPL-3.0-only
//! The Huffman coding of HPACK string literals (RFC 7541 section 5.2).
//!
//! The code table lives in `huffman_table.rs`. Encoding looks symbols up in
//! the table, and decoding walks a binary tree that the first call builds out
//! of the same table, so the two directions cannot disagree about the code.

use std::fmt;
use std::sync::OnceLock;

use crate::huffman_table::CODES;

/// The symbol that marks the end of a string. A string must not contain it.
const EOS: usize = 256;

/// Why a Huffman coded string is invalid.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HuffmanError {
    /// The string contains the end of string symbol.
    Eos,
    /// The padding after the last symbol is longer than 7 bits.
    PaddingTooLong,
    /// The padding is not a prefix of the end of string symbol, which is a
    /// run of one bits.
    PaddingNotOnes,
}

impl fmt::Display for HuffmanError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            HuffmanError::Eos => "huffman string contains the end of string symbol",
            HuffmanError::PaddingTooLong => "huffman padding is longer than 7 bits",
            HuffmanError::PaddingNotOnes => "huffman padding is not a run of one bits",
        })
    }
}

impl std::error::Error for HuffmanError {}

/// Returns the number of octets that `data` takes when Huffman coded.
pub fn encoded_len(data: &[u8]) -> usize {
    let bits: usize = data
        .iter()
        .map(|b| usize::from(CODES[usize::from(*b)].1))
        .sum();
    bits.div_ceil(8)
}

/// Appends the Huffman coding of `data`, padded with one bits to an octet
/// boundary.
pub fn encode(data: &[u8], out: &mut Vec<u8>) {
    let mut acc: u64 = 0;
    let mut bits: u32 = 0;
    for b in data {
        let (code, len) = CODES[usize::from(*b)];
        acc = (acc << len) | u64::from(code);
        bits += u32::from(len);
        while bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
        }
        // Keep only the bits that wait for output, so the shift cannot overflow.
        acc &= (1u64 << bits) - 1;
    }
    if bits > 0 {
        let pad = 8 - bits;
        out.push(((acc << pad) | ((1u64 << pad) - 1)) as u8);
    }
}

/// The decoding tree. A child value above zero is the index of an inner node,
/// a value of zero means that no code leads there, and a value below zero is a
/// leaf whose symbol is `-value - 1`.
fn tree() -> &'static [[i32; 2]] {
    static TREE: OnceLock<Vec<[i32; 2]>> = OnceLock::new();
    TREE.get_or_init(|| {
        let mut nodes: Vec<[i32; 2]> = vec![[0, 0]];
        for (symbol, (code, len)) in CODES.iter().enumerate() {
            let mut at = 0usize;
            for i in (0..*len).rev() {
                let bit = ((code >> i) & 1) as usize;
                if i == 0 {
                    nodes[at][bit] = -(symbol as i32) - 1;
                } else {
                    if nodes[at][bit] == 0 {
                        nodes.push([0, 0]);
                        nodes[at][bit] = (nodes.len() - 1) as i32;
                    }
                    at = nodes[at][bit] as usize;
                }
            }
        }
        nodes
    })
}

/// Decodes a Huffman coded string and appends it to `out`.
pub fn decode(data: &[u8], out: &mut Vec<u8>) -> Result<(), HuffmanError> {
    let tree = tree();
    let mut at = 0usize;
    // Bits read since the last complete symbol, and whether all were ones.
    let mut pending = 0u32;
    let mut ones = true;
    for byte in data {
        for i in (0..8).rev() {
            let bit = usize::from((byte >> i) & 1);
            pending += 1;
            ones &= bit == 1;
            let next = tree[at][bit];
            if next < 0 {
                let symbol = (-next - 1) as usize;
                if symbol == EOS {
                    return Err(HuffmanError::Eos);
                }
                out.push(symbol as u8);
                at = 0;
                pending = 0;
                ones = true;
            } else if next == 0 {
                // The code is complete, so this cannot happen.
                return Err(HuffmanError::PaddingNotOnes);
            } else {
                at = next as usize;
            }
        }
    }
    if pending > 7 {
        return Err(HuffmanError::PaddingTooLong);
    }
    if !ones {
        return Err(HuffmanError::PaddingNotOnes);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(s: &str) -> Vec<u8> {
        let s: String = s.split_whitespace().collect();
        (0..s.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
            .collect()
    }

    #[test]
    fn the_code_is_complete_and_prefix_free() {
        // A prefix free code has a Kraft sum of at most 1, and equality means
        // that every bit string either is a code or starts with one.
        let kraft: u128 = CODES
            .iter()
            .map(|(_, l)| 1u128 << (30 - u32::from(*l)))
            .sum();
        assert_eq!(kraft, 1u128 << 30);
        for (i, (a, la)) in CODES.iter().enumerate() {
            assert!(
                *a < (1u64 << la) as u32 || *la == 32,
                "code {i} fits its length"
            );
            for (j, (b, lb)) in CODES.iter().enumerate() {
                if i != j && la <= lb {
                    assert_ne!(*a, b >> (lb - la), "code {i} is a prefix of code {j}");
                }
            }
        }
        assert_eq!(CODES[EOS], (0x3fff_ffff, 30));
    }

    #[test]
    fn appendix_c_strings() {
        // The Huffman coded strings of RFC 7541 Appendix C.4 and C.6.
        for (plain, coded) in [
            ("www.example.com", "f1e3 c2e5 f23a 6ba0 ab90 f4ff"),
            ("no-cache", "a8eb 1064 9cbf"),
            ("custom-key", "25a8 49e9 5ba9 7d7f"),
            ("custom-value", "25a8 49e9 5bb8 e8b4 bf"),
            ("302", "6402"),
            ("private", "aec3 771a 4b"),
            (
                "Mon, 21 Oct 2013 20:13:21 GMT",
                "d07a be94 1054 d444 a820 0595 040b 8166 e082 a62d 1bff",
            ),
            (
                "https://www.example.com",
                "9d29 ad17 1863 c78f 0b97 c8e9 ae82 ae43 d3",
            ),
        ] {
            let want = hex(coded);
            let mut out = Vec::new();
            encode(plain.as_bytes(), &mut out);
            assert_eq!(out, want, "encode {plain}");
            assert_eq!(
                encoded_len(plain.as_bytes()),
                want.len(),
                "length of {plain}"
            );
            let mut back = Vec::new();
            decode(&want, &mut back).unwrap();
            assert_eq!(back, plain.as_bytes(), "decode {plain}");
        }
    }

    #[test]
    fn every_octet_round_trips() {
        let all: Vec<u8> = (0..=255).collect();
        let mut coded = Vec::new();
        encode(&all, &mut coded);
        let mut back = Vec::new();
        decode(&coded, &mut back).unwrap();
        assert_eq!(back, all);
        assert_eq!(encoded_len(&all), coded.len());
    }

    #[test]
    fn invalid_padding_and_eos_are_errors() {
        let mut out = Vec::new();
        // 'a' is 00011, so 00011 followed by zeros is padding that is not ones.
        assert_eq!(
            decode(&[0b0001_1000], &mut out),
            Err(HuffmanError::PaddingNotOnes)
        );
        // Eight one bits after a symbol are longer than 7 bits of padding.
        assert_eq!(
            decode(&[0b0001_1111, 0xff], &mut out),
            Err(HuffmanError::PaddingTooLong)
        );
        // Four octets of ones hold the end of string symbol.
        assert_eq!(
            decode(&[0xff, 0xff, 0xff, 0xff], &mut out),
            Err(HuffmanError::Eos)
        );
        assert!(decode(&[], &mut out).is_ok());
    }
}
