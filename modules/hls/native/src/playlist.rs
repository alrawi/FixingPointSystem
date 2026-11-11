// SPDX-License-Identifier: GPL-3.0-only
//! Master and media playlists (RFC 8216) and the readers that a player needs.

use std::fmt::Write as _;

use crate::ladder::Rung;

/// The codec string of the video of a rung, by picture height.
fn video_codec(height: u32) -> &'static str {
    match height {
        0..=360 => "avc1.42c01e",
        361..=720 => "avc1.4d401f",
        _ => "avc1.640028",
    }
}

/// The file name of segment `index`.
pub fn segment_name(index: u64) -> String {
    format!("seg{index:05}.ts")
}

/// Renders the master playlist of a ladder. The rung `i` has its media playlist at
/// `v<i>/index.m3u8`, and its peak bandwidth is 15% above its average.
pub fn master(rungs: &[Rung]) -> String {
    let mut out = String::from("#EXTM3U\n#EXT-X-VERSION:6\n#EXT-X-INDEPENDENT-SEGMENTS\n");
    for (i, r) in rungs.iter().enumerate() {
        let peak = r.bits_per_second + r.bits_per_second * 15 / 100;
        let _ = writeln!(
            out,
            "#EXT-X-STREAM-INF:BANDWIDTH={peak},AVERAGE-BANDWIDTH={},RESOLUTION={}x{},FRAME-RATE=30.000,CODECS=\"{},mp4a.40.2\"",
            r.bits_per_second,
            r.width,
            r.height,
            video_codec(r.height)
        );
        let _ = writeln!(out, "v{i}/index.m3u8");
    }
    out
}

/// The parameters of a media playlist.
#[derive(Clone, Copy, Debug)]
pub struct Media {
    /// Segments of a VOD stream.
    pub segments: u64,
    /// Duration of each segment in milliseconds.
    pub segment_ms: u64,
    /// Number of the first listed segment.
    pub first: u64,
    /// Segments that a live list names, or 0 for every segment from `first` on.
    pub count: u64,
    /// Whether the list is live, which has no end marker.
    pub live: bool,
}

/// Renders a media playlist.
pub fn media(m: &Media) -> Result<String, String> {
    let last = if m.live {
        if m.count == 0 {
            return Err("a live playlist needs a count of at least 1".into());
        }
        m.first + m.count
    } else {
        if m.segments == 0 {
            return Err("a VOD playlist needs at least one segment".into());
        }
        if m.count == 0 {
            m.segments
        } else {
            (m.first + m.count).min(m.segments)
        }
    };
    if m.first >= last {
        return Err(format!(
            "the playlist from segment {} names no segment",
            m.first
        ));
    }
    let target = m.segment_ms.div_ceil(1000);
    let mut out = String::from("#EXTM3U\n#EXT-X-VERSION:3\n");
    let _ = writeln!(out, "#EXT-X-TARGETDURATION:{target}");
    let _ = writeln!(out, "#EXT-X-MEDIA-SEQUENCE:{}", m.first);
    if !m.live {
        out.push_str("#EXT-X-PLAYLIST-TYPE:VOD\n");
    }
    for i in m.first..last {
        let _ = writeln!(
            out,
            "#EXTINF:{}.{:03},",
            m.segment_ms / 1000,
            m.segment_ms % 1000
        );
        let _ = writeln!(out, "{}", segment_name(i));
    }
    if !m.live {
        out.push_str("#EXT-X-ENDLIST\n");
    }
    Ok(out)
}

fn uris(playlist: &[u8]) -> Result<Vec<&str>, String> {
    let text = std::str::from_utf8(playlist).map_err(|_| "the playlist is not text".to_owned())?;
    if !text.starts_with("#EXTM3U") {
        return Err("the text is not a playlist, it lacks #EXTM3U".into());
    }
    Ok(text
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .collect())
}

/// The URI line `index` of a playlist. A negative index counts from the end, so -1 is the last
/// line, which is the newest segment of a live playlist.
pub fn uri(playlist: &[u8], index: i64) -> Result<String, String> {
    let all = uris(playlist)?;
    let at = if index < 0 {
        all.len().checked_sub(index.unsigned_abs() as usize)
    } else {
        usize::try_from(index).ok()
    };
    at.and_then(|i| all.get(i))
        .map(|s| (*s).to_owned())
        .ok_or_else(|| format!("the playlist has {} URIs, and no URI {index}", all.len()))
}

/// The number of URI lines of a playlist.
pub fn count(playlist: &[u8]) -> Result<usize, String> {
    uris(playlist).map(|u| u.len())
}

/// The request path of `uri` relative to the playlist at `base`.
pub fn resolve(base: &str, uri: &str) -> Result<String, String> {
    if uri.is_empty() || uri.bytes().any(|b| b <= b' ' || b == 0x7f) {
        return Err(format!(
            "'{uri}' is not a URI that a request line can carry"
        ));
    }
    if uri.contains("://") {
        return Err(format!(
            "'{uri}' is an absolute URL, and the module serves paths"
        ));
    }
    if uri.starts_with('/') {
        return Ok(uri.to_owned());
    }
    if !base.starts_with('/') {
        return Err(format!("the base '{base}' is not an absolute path"));
    }
    let dir = &base[..=base.rfind('/').unwrap_or(0)];
    Ok(format!("{dir}{uri}"))
}

/// The value of the `Content-Length` header of an HTTP message.
pub fn content_length(message: &[u8]) -> Result<u64, String> {
    let end = message
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .ok_or_else(|| "the message has no end of header block".to_owned())?;
    let head = std::str::from_utf8(&message[..end])
        .map_err(|_| "the header block is not text".to_owned())?;
    for line in head.split("\r\n").skip(1) {
        if let Some((name, value)) = line.split_once(':') {
            if name.eq_ignore_ascii_case("content-length") {
                return value
                    .trim()
                    .parse()
                    .map_err(|_| format!("Content-Length '{}' is not a number", value.trim()));
            }
        }
    }
    Err("the message has no Content-Length header".into())
}

/// The variant and the segment of a request line such as `GET /v1/seg00007.ts HTTP/1.1`.
pub fn parse_segment_request(request: &[u8]) -> Result<(i64, u64), String> {
    let line_end = request
        .windows(2)
        .position(|w| w == b"\r\n")
        .unwrap_or(request.len());
    let line = std::str::from_utf8(&request[..line_end])
        .map_err(|_| "the request line is not text".to_owned())?;
    let mut parts = line.split(' ');
    let (Some("GET"), Some(path)) = (parts.next(), parts.next()) else {
        return Err(format!("'{line}' is not a GET request line"));
    };
    // The rung directory and the file name are the last two parts, so a prefix such as /live is fine.
    let mut parts = path.rsplit('/');
    let (Some(name), Some(dir)) = (parts.next(), parts.next()) else {
        return Err(format!(
            "'{path}' is not a segment path such as /v1/seg00007.ts"
        ));
    };
    let variant = dir
        .strip_prefix('v')
        .and_then(|d| d.parse::<i64>().ok())
        .ok_or_else(|| format!("'{path}' has no rung directory such as v1"))?;
    let index = name
        .strip_prefix("seg")
        .and_then(|n| n.strip_suffix(".ts"))
        .and_then(|n| n.parse::<u64>().ok())
        .ok_or_else(|| format!("'{path}' is not a segment name such as seg00007.ts"))?;
    Ok((variant, index))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ladder::parse;

    #[test]
    fn master_lists_every_rung() {
        let r = parse("400000@640x360,800000@960x540").unwrap();
        let m = master(&r);
        assert!(m.starts_with("#EXTM3U\n#EXT-X-VERSION:6\n"));
        assert!(m.contains("BANDWIDTH=460000,AVERAGE-BANDWIDTH=400000,RESOLUTION=640x360"));
        assert!(m.contains("AVERAGE-BANDWIDTH=800000,RESOLUTION=960x540"));
        assert!(m.contains("\nv0/index.m3u8\n"));
        assert!(m.ends_with("\nv1/index.m3u8\n"));
        assert_eq!(count(m.as_bytes()).unwrap(), 2);
        assert_eq!(uri(m.as_bytes(), 1).unwrap(), "v1/index.m3u8");
    }

    #[test]
    fn vod_playlist_has_all_segments_and_an_end() {
        let p = media(&Media {
            segments: 3,
            segment_ms: 6000,
            first: 0,
            count: 0,
            live: false,
        })
        .unwrap();
        assert_eq!(
            p,
            "#EXTM3U\n#EXT-X-VERSION:3\n#EXT-X-TARGETDURATION:6\n#EXT-X-MEDIA-SEQUENCE:0\n\
             #EXT-X-PLAYLIST-TYPE:VOD\n#EXTINF:6.000,\nseg00000.ts\n#EXTINF:6.000,\nseg00001.ts\n\
             #EXTINF:6.000,\nseg00002.ts\n#EXT-X-ENDLIST\n"
        );
    }

    #[test]
    fn live_playlist_slides_and_has_no_end() {
        let p = media(&Media {
            segments: 0,
            segment_ms: 2500,
            first: 7,
            count: 3,
            live: true,
        })
        .unwrap();
        assert!(p.contains("#EXT-X-TARGETDURATION:3\n#EXT-X-MEDIA-SEQUENCE:7\n"));
        assert!(p.contains("#EXTINF:2.500,\nseg00007.ts\n"));
        assert!(p.ends_with("seg00009.ts\n"));
        assert!(!p.contains("ENDLIST") && !p.contains("PLAYLIST-TYPE"));
        assert_eq!(uri(p.as_bytes(), 2).unwrap(), "seg00009.ts");
    }

    #[test]
    fn playlists_reject_nonsense() {
        let base = Media {
            segments: 3,
            segment_ms: 6000,
            first: 0,
            count: 0,
            live: false,
        };
        assert!(media(&Media {
            segments: 0,
            ..base
        })
        .is_err());
        assert!(media(&Media { first: 3, ..base }).is_err());
        assert!(media(&Media { live: true, ..base }).is_err());
        assert!(uri(b"hello", 0).is_err());
        assert!(uri(b"#EXTM3U\na.ts\n", 1).is_err());
        assert!(uri(b"#EXTM3U\na.ts\n", -2).is_err());
    }

    #[test]
    fn a_negative_index_counts_from_the_end() {
        let p = b"#EXTM3U\n#EXTINF:2.0,\na.ts\n#EXTINF:2.0,\nb.ts\n#EXTINF:2.0,\nc.ts\n";
        assert_eq!(uri(p, -1).unwrap(), "c.ts");
        assert_eq!(uri(p, -3).unwrap(), "a.ts");
        assert!(uri(p, -4).is_err());
    }

    #[test]
    fn resolve_joins_relative_uris() {
        assert_eq!(
            resolve("/v1/index.m3u8", "seg00001.ts").unwrap(),
            "/v1/seg00001.ts"
        );
        assert_eq!(
            resolve("/index.m3u8", "v0/index.m3u8").unwrap(),
            "/v0/index.m3u8"
        );
        assert_eq!(resolve("/a/b.m3u8", "/c/d.ts").unwrap(), "/c/d.ts");
        assert!(resolve("a.m3u8", "b.ts").is_err());
        assert!(resolve("/a.m3u8", "http://x.test/b.ts").is_err());
        assert!(resolve("/a.m3u8", "b c.ts").is_err());
    }

    #[test]
    fn content_length_reads_the_header() {
        let m = b"HTTP/1.1 200 OK\r\nServer: x\r\ncontent-length: 1234\r\n\r\nbody";
        assert_eq!(content_length(m).unwrap(), 1234);
        assert!(content_length(b"HTTP/1.1 200 OK\r\n\r\n").is_err());
        assert!(content_length(b"HTTP/1.1 200 OK\r\nContent-Length: x\r\n\r\n").is_err());
        assert!(content_length(b"HTTP/1.1 200 OK\r\n").is_err());
    }

    #[test]
    fn segment_requests_parse() {
        let r = b"GET /v2/seg00017.ts HTTP/1.1\r\nHost: h\r\n\r\n";
        assert_eq!(parse_segment_request(r).unwrap(), (2, 17));
        let live = b"GET /live/v1/seg00003.ts HTTP/1.1\r\n\r\n";
        assert_eq!(parse_segment_request(live).unwrap(), (1, 3));
        for bad in [
            &b"POST /v2/seg00017.ts HTTP/1.1\r\n"[..],
            b"GET /seg00017.ts HTTP/1.1\r\n",
            b"GET /x2/seg00017.ts HTTP/1.1\r\n",
            b"GET /v2/index.m3u8 HTTP/1.1\r\n",
        ] {
            assert!(parse_segment_request(bad).is_err());
        }
    }
}
