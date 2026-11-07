// SPDX-License-Identifier: GPL-3.0-only
//! The bounded pcapng ring on disk.
//!
//! The ring is a directory of files named `<prefix>-<index>.pcapng`. The writer
//! appends whole sessions to the newest file and starts a new file when the
//! next session would pass the size limit. When the file count passes its
//! limit, the oldest file goes, together with its key log. A session never
//! spans two files, so every file decrypts on its own.

use std::collections::VecDeque;
use std::fs::{self, File, OpenOptions};
use std::io::{self, BufWriter, Write};
use std::path::{Path, PathBuf};

use crate::pcapng;
use crate::CaptureError;

/// Where the TLS key log of the sessions goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum KeyLogMode {
    /// A decryption secrets block in the pcapng file, ahead of the packets of its session.
    Embedded,
    /// A file `<capture>.keys` in the NSS key log format next to each capture file.
    Sidecar,
    /// Both, which lets Wireshark read the file alone and lets other tools read the log.
    #[default]
    Both,
}

impl KeyLogMode {
    fn embedded(self) -> bool {
        matches!(self, Self::Embedded | Self::Both)
    }
    fn sidecar(self) -> bool {
        matches!(self, Self::Sidecar | Self::Both)
    }
}

/// Where the ring lives and how large it may grow.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RingConfig {
    /// The directory of the ring files. The capture creates it when missing.
    pub dir: PathBuf,
    /// The file name prefix, which must not contain a path separator.
    pub prefix: String,
    /// The size at which the writer starts a new file. A file may pass it only
    /// when a single session is larger, and sessions are bounded by
    /// [`crate::CaptureConfig::max_session_bytes`].
    pub file_bytes: u64,
    /// The number of files that the ring keeps, which must be at least 1.
    pub max_files: usize,
}

impl RingConfig {
    /// A ring in `dir` with 64 MiB files, eight of them.
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self {
            dir: dir.into(),
            prefix: "fps-capture".into(),
            file_bytes: 64 << 20,
            max_files: 8,
        }
    }

    pub(crate) fn validate(&self) -> Result<(), CaptureError> {
        if self.max_files == 0 {
            return Err(CaptureError::InvalidConfig(
                "ring needs at least one file".into(),
            ));
        }
        if self.prefix.is_empty() || self.prefix.contains(['/', '\\']) {
            return Err(CaptureError::InvalidConfig(
                "ring prefix must be a plain file name".into(),
            ));
        }
        Ok(())
    }
}

struct RingFile {
    index: u64,
    path: PathBuf,
    keys: PathBuf,
    bytes: u64,
}

struct Current {
    out: BufWriter<File>,
    keys: Option<File>,
}

/// What one write did.
#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct Appended {
    /// Files that the write opened.
    pub opened: u64,
    /// Files that the write removed from the ring.
    pub evicted: u64,
}

pub(crate) struct Ring {
    cfg: RingConfig,
    mode: KeyLogMode,
    files: VecDeque<RingFile>,
    current: Option<Current>,
    next_index: u64,
    header: Vec<u8>,
}

fn file_name(prefix: &str, index: u64) -> String {
    format!("{prefix}-{index:06}.pcapng")
}

fn keys_path(path: &Path) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(".keys");
    PathBuf::from(name)
}

fn io_err(what: &str, path: &Path, source: io::Error) -> CaptureError {
    CaptureError::Io {
        what: format!("{what} {}", path.display()),
        source,
    }
}

impl Ring {
    /// Opens the ring directory and adopts files of an earlier run with the same prefix.
    pub fn open(cfg: RingConfig, mode: KeyLogMode) -> Result<Self, CaptureError> {
        cfg.validate()?;
        fs::create_dir_all(&cfg.dir).map_err(|e| io_err("create", &cfg.dir, e))?;
        let mut found = Vec::new();
        let entries = fs::read_dir(&cfg.dir).map_err(|e| io_err("read", &cfg.dir, e))?;
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            let Some(index) = name
                .strip_prefix(&format!("{}-", cfg.prefix))
                .and_then(|r| r.strip_suffix(".pcapng"))
                .and_then(|n| n.parse::<u64>().ok())
            else {
                continue;
            };
            let bytes = entry.metadata().map(|m| m.len()).unwrap_or(0);
            let path = entry.path();
            found.push(RingFile {
                index,
                keys: keys_path(&path),
                path,
                bytes,
            });
        }
        found.sort_by_key(|f| f.index);
        let next_index = found.last().map_or(0, |f| f.index + 1);
        let mut header = pcapng::section_header(concat!("fps-capture ", env!("CARGO_PKG_VERSION")));
        header.extend_from_slice(&pcapng::interface_description("fps-synthetic"));
        let mut ring = Self {
            cfg,
            mode,
            files: found.into(),
            current: None,
            next_index,
            header,
        };
        // Adopted files are closed. The bound holds from the first write on.
        ring.evict();
        Ok(ring)
    }

    /// The paths of the files in the ring, oldest first.
    pub fn paths(&self) -> Vec<PathBuf> {
        self.files.iter().map(|f| f.path.clone()).collect()
    }

    fn evict(&mut self) -> u64 {
        let mut evicted = 0;
        while self.files.len() > self.cfg.max_files {
            if let Some(old) = self.files.pop_front() {
                // A failed removal leaves a stray file, which the next open adopts.
                let _ = fs::remove_file(&old.path);
                let _ = fs::remove_file(&old.keys);
                evicted += 1;
            }
        }
        evicted
    }

    fn open_next(&mut self) -> Result<u64, CaptureError> {
        self.finish_current()?;
        let path = self
            .cfg
            .dir
            .join(file_name(&self.cfg.prefix, self.next_index));
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .map_err(|e| io_err("create", &path, e))?;
        file.write_all(&self.header)
            .map_err(|e| io_err("write", &path, e))?;
        self.files.push_back(RingFile {
            index: self.next_index,
            keys: keys_path(&path),
            path,
            bytes: self.header.len() as u64,
        });
        self.next_index += 1;
        self.current = Some(Current {
            out: BufWriter::new(file),
            keys: None,
        });
        Ok(self.evict())
    }

    fn finish_current(&mut self) -> Result<(), CaptureError> {
        if let (Some(mut cur), Some(file)) = (self.current.take(), self.files.back()) {
            cur.out
                .flush()
                .map_err(|e| io_err("flush", &file.path, e))?;
        }
        Ok(())
    }

    /// Appends one session. `packets` holds its encoded packet blocks, and
    /// `keys` holds its NSS key log lines, which may be empty.
    ///
    /// Returns `Ok(None)` when the session cannot fit into a file of its own,
    /// which the caller counts as dropped.
    pub fn append(&mut self, packets: &[u8], keys: &str) -> Result<Option<Appended>, CaptureError> {
        let secrets = if self.mode.embedded() && !keys.is_empty() {
            pcapng::decryption_secrets(keys.as_bytes())
        } else {
            Vec::new()
        };
        let size = (secrets.len() + packets.len()) as u64;
        if self.header.len() as u64 + size > self.cfg.file_bytes {
            return Ok(None);
        }
        let mut done = Appended::default();
        let fits = match (&self.current, self.files.back()) {
            (Some(_), Some(file)) => file.bytes + size <= self.cfg.file_bytes,
            _ => false,
        };
        if !fits {
            let before = self.files.len();
            done.evicted = self.open_next()?;
            done.opened = 1;
            debug_assert!(self.files.len() <= before + 1);
        }
        let (Some(cur), Some(file)) = (self.current.as_mut(), self.files.back_mut()) else {
            return Ok(None);
        };
        cur.out
            .write_all(&secrets)
            .map_err(|e| io_err("write", &file.path, e))?;
        cur.out
            .write_all(packets)
            .map_err(|e| io_err("write", &file.path, e))?;
        cur.out
            .flush()
            .map_err(|e| io_err("flush", &file.path, e))?;
        file.bytes += size;
        if self.mode.sidecar() && !keys.is_empty() {
            if cur.keys.is_none() {
                cur.keys = Some(
                    OpenOptions::new()
                        .create(true)
                        .append(true)
                        .open(&file.keys)
                        .map_err(|e| io_err("create", &file.keys, e))?,
                );
            }
            if let Some(out) = cur.keys.as_mut() {
                out.write_all(keys.as_bytes())
                    .map_err(|e| io_err("write", &file.keys, e))?;
            }
        }
        Ok(Some(done))
    }

    /// Flushes the newest file.
    pub fn flush(&mut self) -> Result<(), CaptureError> {
        if let (Some(cur), Some(file)) = (self.current.as_mut(), self.files.back()) {
            cur.out
                .flush()
                .map_err(|e| io_err("flush", &file.path, e))?;
        }
        Ok(())
    }

    /// Flushes and closes the newest file.
    pub fn close(&mut self) -> Result<(), CaptureError> {
        self.finish_current()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("ring-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    fn cfg(dir: &Path, file_bytes: u64, max_files: usize) -> RingConfig {
        RingConfig {
            dir: dir.into(),
            prefix: "t".into(),
            file_bytes,
            max_files,
        }
    }

    #[test]
    fn rotation_keeps_the_file_count_and_removes_the_oldest_with_its_keys() {
        let dir = temp("rotate");
        let mut ring = Ring::open(cfg(&dir, 1_000, 3), KeyLogMode::Both).unwrap();
        let chunk = vec![0u8; 450];
        for _ in 0..10 {
            ring.append(&chunk, "CLIENT_RANDOM aa bb\n")
                .unwrap()
                .unwrap();
            assert!(ring.paths().len() <= 3);
        }
        ring.close().unwrap();
        let mut names: Vec<_> = fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        names.sort();
        // Each file holds one session of 450 bytes plus its secrets block, so
        // ten sessions make ten files, of which the last three remain.
        assert_eq!(
            names.len(),
            6,
            "three captures and three key logs: {names:?}"
        );
        assert!(names[0].starts_with("t-000007") && names[4].starts_with("t-000009"));
        for p in ring.paths() {
            assert!(fs::metadata(&p).unwrap().len() <= 1_000);
        }
    }

    #[test]
    fn an_oversized_session_is_refused_without_touching_the_ring() {
        let dir = temp("oversize");
        let mut ring = Ring::open(cfg(&dir, 500, 2), KeyLogMode::Embedded).unwrap();
        assert!(ring.append(&vec![0u8; 600], "").unwrap().is_none());
        assert!(ring.paths().is_empty());
    }

    #[test]
    fn a_new_run_adopts_existing_files_and_continues_the_numbering() {
        let dir = temp("adopt");
        let mut ring = Ring::open(cfg(&dir, 1_000, 2), KeyLogMode::Embedded).unwrap();
        for _ in 0..3 {
            ring.append(&vec![0u8; 700], "").unwrap();
        }
        ring.close().unwrap();
        let again = Ring::open(cfg(&dir, 1_000, 2), KeyLogMode::Embedded).unwrap();
        assert_eq!(again.paths().len(), 2);
        assert_eq!(again.next_index, 3);
        let smaller = Ring::open(cfg(&dir, 1_000, 1), KeyLogMode::Embedded).unwrap();
        assert_eq!(smaller.paths().len(), 1);
    }

    #[test]
    fn invalid_configurations_are_refused() {
        let dir = temp("invalid");
        assert!(Ring::open(cfg(&dir, 1_000, 0), KeyLogMode::Both).is_err());
        let mut c = cfg(&dir, 1_000, 1);
        c.prefix = "a/b".into();
        assert!(Ring::open(c, KeyLogMode::Both).is_err());
    }
}
