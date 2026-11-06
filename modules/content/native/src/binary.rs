// SPDX-License-Identifier: GPL-3.0-only
//! Binary file types: PNG, GIF, JPEG, PDF, ZIP, GZIP, and raw bytes.
//!
//! Every format has correct framing, with the checksums and lengths that its
//! specification requires, and none needs a compression library: the
//! deflate streams hold stored blocks, the GIF image data is LZW with a clear
//! code every 200 pixels, and the JPEG file carries its random bytes in
//! comment segments. PNG, GIF, PDF, ZIP, and GZIP open in a standard viewer
//! or tool. The JPEG file has the markers of a JFIF file but no image scan,
//! so a viewer shows nothing. All content derives from the seed.

use crate::prose::lang;
use crate::rng::Rng;

const fn crc_table() -> [u32; 256] {
    let mut t = [0u32; 256];
    let mut i = 0;
    while i < 256 {
        let mut c = i as u32;
        let mut k = 0;
        while k < 8 {
            c = if c & 1 == 1 {
                0xEDB8_8320 ^ (c >> 1)
            } else {
                c >> 1
            };
            k += 1;
        }
        t[i] = c;
        i += 1;
    }
    t
}

const CRC_TABLE: [u32; 256] = crc_table();

fn crc32(parts: &[&[u8]]) -> u32 {
    let mut c = 0xFFFF_FFFFu32;
    for part in parts {
        for &b in *part {
            c = CRC_TABLE[usize::from((c as u8) ^ b)] ^ (c >> 8);
        }
    }
    !c
}

fn adler32(data: &[u8]) -> u32 {
    let (mut a, mut b) = (1u32, 0u32);
    for &x in data {
        a = (a + u32::from(x)) % 65_521;
        b = (b + a) % 65_521;
    }
    (b << 16) | a
}

/// Deflate stream of stored blocks.
fn stored_blocks(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len() + 5 * (data.len() / 65_535 + 1));
    let mut chunks = data.chunks(65_535).peekable();
    if chunks.peek().is_none() {
        out.extend_from_slice(&[1, 0, 0, 0xFF, 0xFF]);
    }
    while let Some(c) = chunks.next() {
        let len = c.len() as u16;
        out.push(u8::from(chunks.peek().is_none()));
        out.extend_from_slice(&len.to_le_bytes());
        out.extend_from_slice(&(!len).to_le_bytes());
        out.extend_from_slice(c);
    }
    out
}

/// Builds with growing unit counts until the output reaches `size`, and
/// stops within one unit of it. A probe of 33 units estimates the bytes per
/// unit, so large sizes need few builds. The steps never jump past the size
/// by more than the estimate's error, because they round down.
fn fit(size: usize, mut build: impl FnMut(usize) -> Vec<u8>) -> Vec<u8> {
    const PROBE: usize = 33;
    let first = build(1);
    if first.len() >= size {
        return first;
    }
    let probe = build(PROBE);
    if probe.len() >= size {
        for units in 2..PROBE {
            let v = build(units);
            if v.len() >= size {
                return v;
            }
        }
        return probe;
    }
    let slope = ((probe.len() - first.len()) as f64 / (PROBE - 1) as f64).max(1.0);
    let mut units = PROBE;
    let mut len = probe.len();
    loop {
        units += (((size - len) as f64 / slope) as usize).max(1);
        let v = build(units);
        if v.len() >= size {
            return v;
        }
        len = v.len();
    }
}

/// ASCII prose of exactly `n` bytes, the same prefix for every `n`.
fn ascii_payload(seed: u64, n: usize) -> Vec<u8> {
    let mut text = match lang("en") {
        Some(en) => en.prose(&mut Rng::new(seed ^ 0x5A5A), n),
        None => String::new(),
    }
    .into_bytes();
    text.truncate(n);
    text
}

fn png_chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    out.extend_from_slice(kind);
    out.extend_from_slice(data);
    out.extend_from_slice(&crc32(&[kind, data]).to_be_bytes());
}

fn png(seed: u64, size: usize) -> Vec<u8> {
    let w = 16 + Rng::new(seed ^ 0x11).below(80);
    fit(size, |rows| {
        let mut r = Rng::new(seed ^ 0x12);
        let mut raw = Vec::with_capacity(rows * (1 + 3 * w));
        for y in 0..rows {
            raw.push(0);
            for x in 0..w {
                let base = (x * 255 / w + y) as u8;
                raw.push(base.wrapping_add(r.byte() % 48));
                raw.push(255 - base.wrapping_add(r.byte() % 48));
                raw.push(r.byte());
            }
        }
        let mut z = vec![0x78, 0x01];
        z.extend(stored_blocks(&raw));
        z.extend_from_slice(&adler32(&raw).to_be_bytes());
        let mut out = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
        let mut ihdr = Vec::new();
        ihdr.extend_from_slice(&(w as u32).to_be_bytes());
        ihdr.extend_from_slice(&(rows as u32).to_be_bytes());
        ihdr.extend_from_slice(&[8, 2, 0, 0, 0]);
        png_chunk(&mut out, b"IHDR", &ihdr);
        png_chunk(&mut out, b"IDAT", &z);
        png_chunk(&mut out, b"IEND", &[]);
        out
    })
}

fn gif(seed: u64, size: usize) -> Vec<u8> {
    // The width grows with the size, so the height stays below 65536.
    let w = 64 * (1 + size / 2_000_000);
    const CLEAR: u16 = 256;
    const END: u16 = 257;
    fit(size, |rows| {
        let mut r = Rng::new(seed ^ 0x21);
        let mut out = b"GIF89a".to_vec();
        out.extend_from_slice(&(w as u16).to_le_bytes());
        out.extend_from_slice(&(rows as u16).to_le_bytes());
        out.extend_from_slice(&[0xF7, 0, 0]);
        for _ in 0..768 {
            out.push(r.byte());
        }
        out.push(0x2C);
        out.extend_from_slice(&[0, 0, 0, 0]);
        out.extend_from_slice(&(w as u16).to_le_bytes());
        out.extend_from_slice(&(rows as u16).to_le_bytes());
        out.extend_from_slice(&[0, 8]);
        // LZW codes of 9 bits, packed from the least significant bit.
        let mut data = Vec::new();
        let (mut acc, mut nbits) = (0u32, 0u32);
        let mut put = |code: u16, data: &mut Vec<u8>| {
            acc |= u32::from(code) << nbits;
            nbits += 9;
            while nbits >= 8 {
                data.push(acc as u8);
                acc >>= 8;
                nbits -= 8;
            }
        };
        let pixels = w * rows;
        let mut done = 0;
        while done < pixels {
            put(CLEAR, &mut data);
            for _ in 0..200.min(pixels - done) {
                put(u16::from(r.byte()), &mut data);
            }
            done += 200;
        }
        put(END, &mut data);
        if nbits > 0 {
            data.push(acc as u8);
        }
        for block in data.chunks(255) {
            out.push(block.len() as u8);
            out.extend_from_slice(block);
        }
        out.extend_from_slice(&[0, 0x3B]);
        out
    })
}

fn jpeg(seed: u64, size: usize) -> Vec<u8> {
    fit(size, |n| {
        let mut r = Rng::new(seed ^ 0x31);
        let mut out = vec![0xFF, 0xD8];
        out.extend_from_slice(&[
            0xFF, 0xE0, 0, 16, b'J', b'F', b'I', b'F', 0, 1, 1, 0, 0, 1, 0, 1, 0, 0,
        ]);
        let mut left = n;
        while left > 0 {
            let part = left.min(65_533);
            out.extend_from_slice(&[0xFF, 0xFE]);
            out.extend_from_slice(&((part + 2) as u16).to_be_bytes());
            for _ in 0..part {
                out.push(r.byte());
            }
            left -= part;
        }
        out.extend_from_slice(&[0xFF, 0xD9]);
        out
    })
}

fn pdf(seed: u64, size: usize) -> Vec<u8> {
    fit(size, |lines| {
        let mut r = Rng::new(seed ^ 0x41);
        // A title line with a random identifier keeps short documents apart.
        let mut content = format!("BT /F1 14 Tf 50 780 Td (Document {}) Tj ET\n", r.hex(10));
        if let Some(en) = lang("en") {
            for i in 0..lines {
                let mut text = String::new();
                en.sentence(&mut r, &mut text);
                content.push_str(&format!(
                    "BT /F1 10 Tf 50 {} Td ({text}) Tj ET\n",
                    760 - (i % 72) * 10
                ));
            }
        }
        let mut out = b"%PDF-1.4\n%\xE2\xE3\xCF\xD3\n".to_vec();
        let mut offsets = Vec::new();
        let bodies = [
            "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
            "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 4 0 R /Resources << /Font << /F1 5 0 R >> >> >>".to_string(),
            format!(
                "<< /Length {} >>\nstream\n{content}endstream",
                content.len()
            ),
            "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_string(),
        ];
        for (i, body) in bodies.iter().enumerate() {
            offsets.push(out.len());
            out.extend_from_slice(format!("{} 0 obj\n{body}\nendobj\n", i + 1).as_bytes());
        }
        let xref = out.len();
        out.extend_from_slice(b"xref\n0 6\n0000000000 65535 f \n");
        for o in &offsets {
            out.extend_from_slice(format!("{o:010} 00000 n \n").as_bytes());
        }
        out.extend_from_slice(
            format!("trailer\n<< /Size 6 /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n").as_bytes(),
        );
        out
    })
}

fn zip(seed: u64, size: usize) -> Vec<u8> {
    let name = format!("report-{}.txt", Rng::new(seed ^ 0x51).hex(8)).into_bytes();
    fit(size, |n| {
        let data = ascii_payload(seed, n);
        let crc = crc32(&[&data]).to_le_bytes();
        let len = (data.len() as u32).to_le_bytes();
        let nlen = (name.len() as u16).to_le_bytes();
        // Version 2.0, no flags, method 0 (stored), time 00:00, date 2025-01-01.
        let common = [20, 0, 0, 0, 0, 0, 0, 0, 0x21, 0x5A];
        let mut out = b"PK\x03\x04".to_vec();
        out.extend_from_slice(&common);
        out.extend_from_slice(&crc);
        out.extend_from_slice(&len);
        out.extend_from_slice(&len);
        out.extend_from_slice(&nlen);
        out.extend_from_slice(&[0, 0]);
        out.extend_from_slice(&name);
        out.extend_from_slice(&data);
        let central = out.len();
        out.extend_from_slice(b"PK\x01\x02");
        out.extend_from_slice(&[20, 0]);
        out.extend_from_slice(&common);
        out.extend_from_slice(&crc);
        out.extend_from_slice(&len);
        out.extend_from_slice(&len);
        out.extend_from_slice(&nlen);
        out.extend_from_slice(&[0; 16]);
        out.extend_from_slice(&name);
        let central_len = (out.len() - central) as u32;
        out.extend_from_slice(b"PK\x05\x06");
        out.extend_from_slice(&[0, 0, 0, 0, 1, 0, 1, 0]);
        out.extend_from_slice(&central_len.to_le_bytes());
        out.extend_from_slice(&(central as u32).to_le_bytes());
        out.extend_from_slice(&[0, 0]);
        out
    })
}

fn gzip(seed: u64, size: usize) -> Vec<u8> {
    fit(size, |n| {
        let data = ascii_payload(seed, n);
        let mut out = vec![0x1F, 0x8B, 8, 0, 0, 0, 0, 0, 0, 0xFF];
        out.extend(stored_blocks(&data));
        out.extend_from_slice(&crc32(&[&data]).to_le_bytes());
        out.extend_from_slice(&(data.len() as u32).to_le_bytes());
        out
    })
}

fn raw(seed: u64, size: usize) -> Vec<u8> {
    let mut r = Rng::new(seed ^ 0x61);
    (0..size).map(|_| r.byte()).collect()
}

/// Returns a file of the given kind with at least `size` bytes, or `None`
/// for an unknown kind. The `bin` kind has exactly `size` bytes.
pub(crate) fn generate(kind: &str, seed: u64, size: usize) -> Option<Vec<u8>> {
    Some(match kind {
        "png" => png(seed, size),
        "gif" => gif(seed, size),
        "jpeg" => jpeg(seed, size),
        "pdf" => pdf(seed, size),
        "zip" => zip(seed, size),
        "gzip" => gzip(seed, size),
        "bin" => raw(seed, size),
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crc32_matches_the_check_value() {
        assert_eq!(crc32(&[b"123456789"]), 0xCBF4_3926);
    }

    #[test]
    fn adler32_matches_the_check_value() {
        assert_eq!(adler32(b"Wikipedia"), 0x11E6_0398);
    }

    #[test]
    fn stored_blocks_frame_an_empty_and_a_long_input() {
        assert_eq!(stored_blocks(&[]), [1, 0, 0, 0xFF, 0xFF]);
        let long = stored_blocks(&vec![7u8; 70_000]);
        assert_eq!(long.len(), 70_000 + 10);
        assert_eq!(long[0], 0);
    }

    #[test]
    fn every_kind_reaches_its_size() {
        for kind in crate::BINARY_KINDS {
            for size in [0, 100, 5000, 200_000] {
                let f = generate(kind, 5, size).unwrap();
                assert!(f.len() >= size, "{kind} {size}: {}", f.len());
            }
        }
        assert!(generate("exe", 1, 10).is_none());
    }
}
