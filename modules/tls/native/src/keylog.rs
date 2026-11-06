// SPDX-License-Identifier: GPL-3.0-only
//! Key log export in the NSS format (SSLKEYLOGFILE).

use std::fs::{File, OpenOptions};
use std::io::{self, Write};
use std::path::Path;
use std::sync::{Mutex, MutexGuard};

use rustls::KeyLog;

enum Sink {
    Memory(Vec<u8>),
    File(File),
}

/// Writes the secrets of every handshake as NSS key log lines.
///
/// A line has the form `<label> <client random in hex> <secret in hex>`, for
/// example `CLIENT_TRAFFIC_SECRET_0`. Wireshark and tshark read the file
/// through the preference `tls.keylog_file`. One log can serve many
/// connectors and acceptors, and each line is one write on a file opened in
/// append mode, so processes that share a file do not interleave lines.
///
/// The log holds the keys of the sessions it saw. Treat a key log file like a
/// private key.
pub struct NssKeyLog {
    sink: Mutex<Sink>,
    error: Mutex<Option<io::Error>>,
}

impl std::fmt::Debug for NssKeyLog {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // The secrets stay out of debug output.
        f.debug_struct("NssKeyLog").finish_non_exhaustive()
    }
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

impl NssKeyLog {
    /// Creates a log that keeps its lines in memory, which [`NssKeyLog::contents`] returns.
    pub fn in_memory() -> Self {
        Self {
            sink: Mutex::new(Sink::Memory(Vec::new())),
            error: Mutex::new(None),
        }
    }

    /// Creates a log that appends to the file at `path`, which it creates when missing.
    pub fn to_file(path: impl AsRef<Path>) -> io::Result<Self> {
        let file = OpenOptions::new().create(true).append(true).open(path)?;
        Ok(Self {
            sink: Mutex::new(Sink::File(file)),
            error: Mutex::new(None),
        })
    }

    /// Creates a file log at the path in the environment variable `SSLKEYLOGFILE`.
    ///
    /// Returns `None` when the variable is unset or empty. This reads the
    /// process environment, so only the edge of a process calls it.
    pub fn from_env() -> io::Result<Option<Self>> {
        match std::env::var_os("SSLKEYLOGFILE") {
            Some(path) if !path.is_empty() => Self::to_file(path).map(Some),
            _ => Ok(None),
        }
    }

    /// Returns the lines of an in memory log, and an empty string for a file log.
    pub fn contents(&self) -> String {
        match &*lock(&self.sink) {
            Sink::Memory(bytes) => String::from_utf8_lossy(bytes).into_owned(),
            Sink::File(_) => String::new(),
        }
    }

    /// Returns the first write error since the last call. A key log cannot
    /// fail a handshake, so a failed write waits here for the caller.
    pub fn take_error(&self) -> Option<io::Error> {
        lock(&self.error).take()
    }

    /// Formats one NSS key log line, including the newline.
    pub fn format_line(label: &str, client_random: &[u8], secret: &[u8]) -> String {
        let mut line =
            String::with_capacity(label.len() + 2 + 2 * (client_random.len() + secret.len()) + 1);
        line.push_str(label);
        line.push(' ');
        push_hex(&mut line, client_random);
        line.push(' ');
        push_hex(&mut line, secret);
        line.push('\n');
        line
    }
}

fn push_hex(out: &mut String, bytes: &[u8]) {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    for b in bytes {
        out.push(char::from(DIGITS[usize::from(b >> 4)]));
        out.push(char::from(DIGITS[usize::from(b & 0x0f)]));
    }
}

impl KeyLog for NssKeyLog {
    fn log(&self, label: &str, client_random: &[u8], secret: &[u8]) {
        let line = Self::format_line(label, client_random, secret);
        let result = match &mut *lock(&self.sink) {
            Sink::Memory(bytes) => {
                bytes.extend_from_slice(line.as_bytes());
                Ok(())
            }
            Sink::File(file) => file.write_all(line.as_bytes()),
        };
        if let Err(e) = result {
            let mut slot = lock(&self.error);
            if slot.is_none() {
                *slot = Some(e);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lines_follow_the_nss_format() {
        let log = NssKeyLog::in_memory();
        log.log("CLIENT_RANDOM", &[0x01, 0xab], &[0xff, 0x00, 0x10]);
        assert_eq!(log.contents(), "CLIENT_RANDOM 01ab ff0010\n");
        assert!(log.take_error().is_none());
    }

    #[test]
    fn file_log_appends_lines() {
        let dir = std::env::temp_dir().join(format!("fps-tls-keylog-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("keys.log");
        let _ = std::fs::remove_file(&path);
        let log = NssKeyLog::to_file(&path).unwrap();
        log.log("A", &[1], &[2]);
        let again = NssKeyLog::to_file(&path).unwrap();
        again.log("B", &[3], &[4]);
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "A 01 02\nB 03 04\n"
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
