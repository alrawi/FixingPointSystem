// SPDX-License-Identifier: GPL-3.0-only
//! Golden file comparison for tests.
//!
//! A test renders its output and calls [`assert_golden!`] with a file name under
//! the crate's `tests/testdata/` directory. The helper compares the output with
//! that file and panics with a unified diff that names the file on mismatch. It
//! rewrites the file only when the environment variable `UPDATE_GOLDEN` equals
//! `1`.
//!
//! ```no_run
//! let report = "sessions 4\nerrors 0\n";
//! fps_golden::assert_golden!("report.txt", report);
//! ```
#![forbid(unsafe_code)]
#![warn(missing_docs, clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// The environment variable that switches the helper to update mode.
pub const UPDATE_ENV: &str = "UPDATE_GOLDEN";

/// Lines of unchanged context that the diff shows around each change.
const CONTEXT: usize = 3;

/// Largest product of line counts that the diff aligns line by line. Larger
/// inputs show their differing middle as one replaced block.
const MAX_ALIGN_CELLS: usize = 4_000_000;

/// Whether a comparison may rewrite the golden file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// Compare the output with the golden file and never write it.
    Compare,
    /// Write the output to the golden file when the two differ.
    Update,
}

impl Mode {
    /// Reads the mode from `UPDATE_GOLDEN` in the process environment.
    pub fn from_env() -> Self {
        Self::from_value(std::env::var(UPDATE_ENV).ok().as_deref())
    }

    /// Returns [`Mode::Update`] only for the value `1`, so a typo or an empty
    /// value never rewrites golden files.
    pub fn from_value(value: Option<&str>) -> Self {
        match value {
            Some("1") => Mode::Update,
            _ => Mode::Compare,
        }
    }
}

/// The result of a comparison that did not fail.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// The golden file already held the output.
    Matched,
    /// Update mode created or rewrote the golden file.
    Written,
}

/// Errors that a golden file comparison reports.
#[derive(Debug)]
pub enum GoldenError {
    /// The golden file does not exist and the mode is [`Mode::Compare`].
    Missing {
        /// Path of the missing golden file.
        path: PathBuf,
    },
    /// The output differs from the golden file.
    Mismatch {
        /// Path of the golden file.
        path: PathBuf,
        /// Unified diff from the golden file to the output.
        diff: String,
    },
    /// Reading or writing the golden file failed.
    Io {
        /// Path of the golden file.
        path: PathBuf,
        /// The operation that failed, for example `read`.
        op: &'static str,
        /// The underlying error.
        source: io::Error,
    },
}

impl fmt::Display for GoldenError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            GoldenError::Missing { path } => write!(
                f,
                "golden file {} does not exist; run with {UPDATE_ENV}=1 to create it",
                path.display()
            ),
            GoldenError::Mismatch { path, diff } => write!(
                f,
                "golden file {} does not match the output; run with {UPDATE_ENV}=1 to accept \
                 the output and review the change with git diff\n{diff}",
                path.display()
            ),
            GoldenError::Io { path, op, source } => {
                write!(f, "golden file {}: {op} failed: {source}", path.display())
            }
        }
    }
}

impl std::error::Error for GoldenError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            GoldenError::Io { source, .. } => Some(source),
            _ => None,
        }
    }
}

/// Compares `actual` with the golden file at `path` in the given mode.
///
/// In [`Mode::Update`] the function writes the file, and its missing parent
/// directories, only when the content differs, so unchanged files keep their
/// modification time.
pub fn check(path: &Path, actual: &[u8], mode: Mode) -> Result<Outcome, GoldenError> {
    let golden = match fs::read(path) {
        Ok(bytes) => Some(bytes),
        Err(e) if e.kind() == io::ErrorKind::NotFound => None,
        Err(source) => {
            return Err(GoldenError::Io {
                path: path.to_path_buf(),
                op: "read",
                source,
            })
        }
    };
    if golden.as_deref() == Some(actual) {
        return Ok(Outcome::Matched);
    }
    match (mode, golden) {
        (Mode::Update, _) => {
            write(path, actual)?;
            Ok(Outcome::Written)
        }
        (Mode::Compare, None) => Err(GoldenError::Missing {
            path: path.to_path_buf(),
        }),
        (Mode::Compare, Some(golden)) => Err(GoldenError::Mismatch {
            path: path.to_path_buf(),
            diff: diff(&path.display().to_string(), &golden, actual),
        }),
    }
}

fn write(path: &Path, actual: &[u8]) -> Result<(), GoldenError> {
    if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
        fs::create_dir_all(dir).map_err(|source| GoldenError::Io {
            path: path.to_path_buf(),
            op: "create directory",
            source,
        })?;
    }
    fs::write(path, actual).map_err(|source| GoldenError::Io {
        path: path.to_path_buf(),
        op: "write",
        source,
    })
}

/// Compares `actual` with the golden file at `path` in the mode that
/// `UPDATE_GOLDEN` selects, and panics with a diff that names the file when the
/// comparison fails.
#[track_caller]
pub fn assert_golden(path: impl AsRef<Path>, actual: impl AsRef<[u8]>) {
    if let Err(e) = check(path.as_ref(), actual.as_ref(), Mode::from_env()) {
        #[allow(clippy::panic)] // a failed assertion must fail the calling test
        {
            panic!("{e}");
        }
    }
}

/// Compares output with `tests/testdata/<name>` of the calling crate.
///
/// The first argument is the file name relative to `tests/testdata/`, and the
/// second is anything that implements `AsRef<[u8]>`, such as a `String` or a
/// byte vector.
#[macro_export]
macro_rules! assert_golden {
    ($name:expr, $actual:expr $(,)?) => {
        $crate::assert_golden(
            ::std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests")
                .join("testdata")
                .join($name),
            $actual,
        )
    };
}

/// Returns a unified diff from `golden` to `actual`. The header names the
/// golden file with `label`. Content that is not UTF-8 gets a summary of the
/// first differing byte instead of a line diff.
pub fn diff(label: &str, golden: &[u8], actual: &[u8]) -> String {
    match (std::str::from_utf8(golden), std::str::from_utf8(actual)) {
        (Ok(g), Ok(a)) => text_diff(label, g, a),
        _ => binary_diff(label, golden, actual),
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Op {
    Equal(usize, usize),
    Delete(usize),
    Insert(usize),
}

fn text_diff(label: &str, golden: &str, actual: &str) -> String {
    let g: Vec<&str> = golden.split_inclusive('\n').collect();
    let a: Vec<&str> = actual.split_inclusive('\n').collect();
    let ops = align(&g, &a);
    let mut out = format!("--- golden {label}\n+++ actual\n");
    for (start, end) in hunks(&ops) {
        let hunk = &ops[start..end];
        let (g0, a0) = first_positions(&ops, start);
        let g_len = hunk.iter().filter(|o| !matches!(o, Op::Insert(_))).count();
        let a_len = hunk.iter().filter(|o| !matches!(o, Op::Delete(_))).count();
        out.push_str(&format!(
            "@@ -{} +{} @@\n",
            range(g0, g_len),
            range(a0, a_len)
        ));
        for op in hunk {
            let (sign, line) = match *op {
                Op::Equal(i, _) => (' ', g[i]),
                Op::Delete(i) => ('-', g[i]),
                Op::Insert(j) => ('+', a[j]),
            };
            out.push(sign);
            out.push_str(line);
            if !line.ends_with('\n') {
                out.push_str("\n\\ No newline at end of file\n");
            }
        }
    }
    out
}

/// Formats a hunk range in unified diff notation. `start` is zero based.
fn range(start: usize, len: usize) -> String {
    match len {
        0 => format!("{start},0"),
        1 => format!("{}", start + 1),
        _ => format!("{},{len}", start + 1),
    }
}

/// Returns the zero based golden and actual line positions where the hunk that
/// starts at `ops[start]` begins.
fn first_positions(ops: &[Op], start: usize) -> (usize, usize) {
    let mut g = 0;
    let mut a = 0;
    for op in &ops[..start] {
        match op {
            Op::Equal(..) => {
                g += 1;
                a += 1;
            }
            Op::Delete(_) => g += 1,
            Op::Insert(_) => a += 1,
        }
    }
    (g, a)
}

/// Groups changed operations into hunks with [`CONTEXT`] lines around them and
/// returns each hunk as a half open range of `ops`.
fn hunks(ops: &[Op]) -> Vec<(usize, usize)> {
    let mut out: Vec<(usize, usize)> = Vec::new();
    for (k, op) in ops.iter().enumerate() {
        if matches!(op, Op::Equal(..)) {
            continue;
        }
        let start = k.saturating_sub(CONTEXT);
        let end = (k + 1 + CONTEXT).min(ops.len());
        match out.last_mut() {
            Some(last) if start <= last.1 => last.1 = end,
            _ => out.push((start, end)),
        }
    }
    out
}

/// Aligns two line lists with a longest common subsequence after trimming the
/// common prefix and suffix.
fn align(g: &[&str], a: &[&str]) -> Vec<Op> {
    let prefix = g.iter().zip(a).take_while(|(x, y)| x == y).count();
    let suffix = g[prefix..]
        .iter()
        .rev()
        .zip(a[prefix..].iter().rev())
        .take_while(|(x, y)| x == y)
        .count();
    let gm = &g[prefix..g.len() - suffix];
    let am = &a[prefix..a.len() - suffix];

    let mut ops: Vec<Op> = (0..prefix).map(|i| Op::Equal(i, i)).collect();
    if gm.len().saturating_mul(am.len()) > MAX_ALIGN_CELLS {
        ops.extend((0..gm.len()).map(|i| Op::Delete(prefix + i)));
        ops.extend((0..am.len()).map(|j| Op::Insert(prefix + j)));
    } else {
        // lcs[i][j] is the common subsequence length of gm[i..] and am[j..].
        let w = am.len() + 1;
        let mut lcs = vec![0usize; (gm.len() + 1) * w];
        for i in (0..gm.len()).rev() {
            for j in (0..am.len()).rev() {
                lcs[i * w + j] = if gm[i] == am[j] {
                    lcs[(i + 1) * w + j + 1] + 1
                } else {
                    lcs[(i + 1) * w + j].max(lcs[i * w + j + 1])
                };
            }
        }
        let (mut i, mut j) = (0, 0);
        while i < gm.len() || j < am.len() {
            if i < gm.len() && j < am.len() && gm[i] == am[j] {
                ops.push(Op::Equal(prefix + i, prefix + j));
                i += 1;
                j += 1;
            } else if j < am.len() && (i == gm.len() || lcs[i * w + j + 1] >= lcs[(i + 1) * w + j])
            {
                ops.push(Op::Insert(prefix + j));
                j += 1;
            } else {
                ops.push(Op::Delete(prefix + i));
                i += 1;
            }
        }
    }
    let (gs, as_) = (g.len() - suffix, a.len() - suffix);
    ops.extend((0..suffix).map(|k| Op::Equal(gs + k, as_ + k)));
    // Show deletions before insertions within a run of changes, as diff tools do.
    reorder_changes(&mut ops);
    ops
}

/// Sorts each run of consecutive changes so that its deletions come first.
fn reorder_changes(ops: &mut [Op]) {
    let mut k = 0;
    while k < ops.len() {
        if matches!(ops[k], Op::Equal(..)) {
            k += 1;
            continue;
        }
        let end = ops[k..]
            .iter()
            .position(|o| matches!(o, Op::Equal(..)))
            .map_or(ops.len(), |p| k + p);
        ops[k..end].sort_by_key(|o| match o {
            Op::Delete(i) => (0, *i),
            Op::Insert(j) => (1, *j),
            Op::Equal(..) => (2, 0),
        });
        k = end;
    }
}

fn binary_diff(label: &str, golden: &[u8], actual: &[u8]) -> String {
    let offset = golden
        .iter()
        .zip(actual)
        .position(|(x, y)| x != y)
        .unwrap_or(golden.len().min(actual.len()));
    let window = |b: &[u8]| -> String {
        let end = (offset + 16).min(b.len());
        b.get(offset..end)
            .unwrap_or_default()
            .iter()
            .map(|x| format!("{x:02x}"))
            .collect::<Vec<_>>()
            .join(" ")
    };
    format!(
        "--- golden {label}\n+++ actual\nbinary content differs: golden has {} bytes, actual has {} \
         bytes, first difference at offset {offset}\n-{}\n+{}\n",
        golden.len(),
        actual.len(),
        window(golden),
        window(actual)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mode_updates_only_for_one() {
        assert_eq!(Mode::from_value(Some("1")), Mode::Update);
        for v in [None, Some(""), Some("0"), Some("true"), Some("yes")] {
            assert_eq!(Mode::from_value(v), Mode::Compare, "value {v:?}");
        }
    }

    #[test]
    fn diff_replaces_one_line_with_context() {
        let d = diff("f.txt", b"a\nb\nc\nd\ne\nf\ng\n", b"a\nb\nc\nX\ne\nf\ng\n");
        assert_eq!(
            d,
            "--- golden f.txt\n+++ actual\n@@ -1,7 +1,7 @@\n a\n b\n c\n-d\n+X\n e\n f\n g\n"
        );
    }

    #[test]
    fn diff_splits_distant_changes_into_hunks() {
        let g: String = (0..20).map(|i| format!("line {i}\n")).collect();
        let a = g
            .replace("line 2\n", "two\n")
            .replace("line 17\n", "seventeen\n");
        let d = diff("f", g.as_bytes(), a.as_bytes());
        assert_eq!(d.matches("@@ ").count(), 2, "{d}");
        assert!(d.contains("@@ -1,6 +1,6 @@\n"), "{d}");
        assert!(d.contains("@@ -15,6 +15,6 @@\n"), "{d}");
    }

    #[test]
    fn diff_marks_missing_final_newline() {
        let d = diff("f", b"a\n", b"a");
        assert!(d.contains("-a\n+a\n\\ No newline at end of file\n"), "{d}");
    }

    #[test]
    fn diff_handles_insertion_into_empty() {
        let d = diff("f", b"", b"x\n");
        assert_eq!(d, "--- golden f\n+++ actual\n@@ -0,0 +1 @@\n+x\n");
    }

    #[test]
    fn diff_summarizes_binary_content() {
        let d = diff("f.bin", &[0, 1, 2, 0xff], &[0, 1, 3, 0xff, 4]);
        assert!(d.contains("golden has 4 bytes, actual has 5 bytes"), "{d}");
        assert!(d.contains("first difference at offset 2"), "{d}");
        assert!(d.contains("\n-02 ff\n+03 ff 04\n"), "{d}");
    }
}
