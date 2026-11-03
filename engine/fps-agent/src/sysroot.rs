// SPDX-License-Identifier: GPL-3.0-only
//! Reads files under an injectable system root.
//!
//! Detection never opens an absolute path. It names files relative to the
//! root, such as `proc/meminfo`, and the root is `/` on a real host and a
//! directory under `testdata/` in tests.

use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

use crate::error::DetectError;

/// A directory that stands in for `/` during detection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SystemRoot {
    root: PathBuf,
}

impl SystemRoot {
    /// Returns the root of the running host, which is `/`.
    pub fn host() -> Self {
        Self::new("/")
    }

    /// Returns a root at `path`, for example a fake root under `testdata/`.
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { root: path.into() }
    }

    /// Returns the directory that stands in for `/`.
    pub fn as_path(&self) -> &Path {
        &self.root
    }

    /// Returns the full path of `rel`, which names a file without a leading slash.
    pub fn path(&self, rel: &str) -> PathBuf {
        self.root.join(rel.trim_start_matches('/'))
    }

    /// Reads a file that must exist.
    pub(crate) fn read(&self, rel: &str) -> Result<String, DetectError> {
        let path = self.path(rel);
        std::fs::read_to_string(&path).map_err(|source| DetectError::Read { path, source })
    }

    /// Reads a file and returns `None` when it does not exist.
    ///
    /// Sysfs attributes such as `speed` of a link that is down fail with
    /// `EINVAL`, and attributes that the caller does not have permission to
    /// read fail with `EACCES`. Both count as absent, because they say nothing
    /// about the host that a missing file would not.
    pub(crate) fn read_optional(&self, rel: &str) -> Result<Option<String>, DetectError> {
        let path = self.path(rel);
        match std::fs::read_to_string(&path) {
            Ok(text) => Ok(Some(text)),
            Err(e) if is_absent(&e) => Ok(None),
            Err(source) => Err(DetectError::Read { path, source }),
        }
    }

    /// Lists the entry names of a directory in sorted order, or returns an
    /// empty list when the directory does not exist.
    pub(crate) fn list_dir(&self, rel: &str) -> Result<Vec<String>, DetectError> {
        let path = self.path(rel);
        let entries = match std::fs::read_dir(&path) {
            Ok(entries) => entries,
            Err(e) if is_absent(&e) => return Ok(Vec::new()),
            Err(source) => return Err(DetectError::Read { path, source }),
        };
        let mut names = Vec::new();
        for entry in entries {
            let entry = entry.map_err(|source| DetectError::Read {
                path: path.clone(),
                source,
            })?;
            if let Some(name) = entry.file_name().to_str() {
                names.push(name.to_owned());
            }
        }
        names.sort();
        Ok(names)
    }

    /// Reports whether `rel` exists.
    pub(crate) fn exists(&self, rel: &str) -> bool {
        self.path(rel).exists()
    }

    /// Scans a file line by line and returns the first line for which `f`
    /// returns `Some`. Returns `Ok(None)` when the file does not exist or no
    /// line matches. The scan streams, so large files such as
    /// `/proc/kallsyms` never sit in memory whole.
    pub(crate) fn find_line<T>(
        &self,
        rel: &str,
        mut f: impl FnMut(&str) -> Option<T>,
    ) -> Result<Option<T>, DetectError> {
        let path = self.path(rel);
        let file = match std::fs::File::open(&path) {
            Ok(file) => file,
            Err(e) if is_absent(&e) => return Ok(None),
            Err(source) => return Err(DetectError::Read { path, source }),
        };
        for line in BufReader::new(file).lines() {
            let line = line.map_err(|source| DetectError::Read {
                path: path.clone(),
                source,
            })?;
            if let Some(found) = f(&line) {
                return Ok(Some(found));
            }
        }
        Ok(None)
    }
}

/// Reports whether an error means that the file says nothing. The standard
/// library maps `EINVAL`, which sysfs returns for `speed` of a link that is
/// down, to `InvalidInput`.
fn is_absent(e: &std::io::Error) -> bool {
    use std::io::ErrorKind;
    matches!(
        e.kind(),
        ErrorKind::NotFound | ErrorKind::PermissionDenied | ErrorKind::InvalidInput
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn path_strips_leading_slash() {
        let root = SystemRoot::new("/fake");
        assert_eq!(
            root.path("/proc/meminfo"),
            PathBuf::from("/fake/proc/meminfo")
        );
        assert_eq!(
            root.path("proc/meminfo"),
            PathBuf::from("/fake/proc/meminfo")
        );
    }

    #[test]
    fn missing_files_are_absent_not_errors() {
        let root = SystemRoot::new("/nonexistent-fps-root");
        assert!(matches!(root.read_optional("proc/meminfo"), Ok(None)));
        assert!(matches!(root.list_dir("sys/class/net"), Ok(v) if v.is_empty()));
        assert!(matches!(
            root.find_line("proc/kallsyms", |_| Some(())),
            Ok(None)
        ));
        assert!(matches!(
            root.read("proc/meminfo"),
            Err(DetectError::Read { .. })
        ));
    }
}
