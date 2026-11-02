// SPDX-License-Identifier: GPL-3.0-only
//! Match, mismatch, and update tests for the golden file helper.

use std::fs;
use std::path::PathBuf;

use fps_golden::{check, GoldenError, Mode, Outcome};

/// Returns a fresh directory for one test under Cargo's temporary target directory.
fn scratch(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join("fps-golden")
        .join(name);
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn match_passes_against_committed_testdata() {
    fps_golden::assert_golden!("report.txt", "sessions 4\nerrors 0\n");
}

#[test]
fn match_leaves_the_file_untouched_in_update_mode() {
    let path = scratch("match-update").join("out.txt");
    fs::write(&path, "same\n").unwrap();
    let before = fs::metadata(&path).unwrap().modified().unwrap();
    assert_eq!(
        check(&path, b"same\n", Mode::Update).unwrap(),
        Outcome::Matched
    );
    assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), before);
}

#[test]
fn mismatch_names_the_golden_file_and_shows_a_diff() {
    let path = scratch("mismatch").join("out.txt");
    fs::write(&path, "sessions 4\nerrors 0\n").unwrap();
    let err = check(&path, b"sessions 4\nerrors 1\n", Mode::Compare).unwrap_err();
    assert!(matches!(err, GoldenError::Mismatch { .. }), "{err:?}");
    let msg = err.to_string();
    assert!(
        msg.contains(&format!("golden file {}", path.display())),
        "{msg}"
    );
    assert!(msg.contains("UPDATE_GOLDEN=1"), "{msg}");
    assert!(msg.contains("-errors 0\n+errors 1\n"), "{msg}");
    assert_eq!(fs::read_to_string(&path).unwrap(), "sessions 4\nerrors 0\n");
}

#[test]
fn mismatch_panics_in_assert_golden() {
    let path = scratch("mismatch-panic").join("out.txt");
    fs::write(&path, "a\n").unwrap();
    let p = path.clone();
    let result = std::panic::catch_unwind(move || fps_golden::assert_golden(&p, "b\n"));
    let payload = result.unwrap_err();
    let msg = payload.downcast_ref::<String>().unwrap();
    assert!(msg.contains(&path.display().to_string()), "{msg}");
    assert!(msg.contains("-a\n+b\n"), "{msg}");
}

#[test]
fn missing_file_fails_in_compare_mode() {
    let path = scratch("missing").join("out.txt");
    let err = check(&path, b"x\n", Mode::Compare).unwrap_err();
    assert!(matches!(err, GoldenError::Missing { .. }), "{err:?}");
    assert!(err.to_string().contains(&path.display().to_string()));
    assert!(!path.exists());
}

#[test]
fn update_rewrites_a_differing_file() {
    let path = scratch("update").join("out.txt");
    fs::write(&path, "old\n").unwrap();
    assert_eq!(
        check(&path, b"new\n", Mode::Update).unwrap(),
        Outcome::Written
    );
    assert_eq!(fs::read_to_string(&path).unwrap(), "new\n");
    assert_eq!(
        check(&path, b"new\n", Mode::Compare).unwrap(),
        Outcome::Matched
    );
}

#[test]
fn update_creates_a_missing_file_and_its_directories() {
    let path = scratch("create").join("nested/dir/out.bin");
    assert_eq!(
        check(&path, &[0, 1, 2], Mode::Update).unwrap(),
        Outcome::Written
    );
    assert_eq!(fs::read(&path).unwrap(), vec![0, 1, 2]);
}
