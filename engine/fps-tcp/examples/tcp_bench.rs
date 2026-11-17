// SPDX-License-Identifier: GPL-3.0-only
//! Measures a user space TCP stack behind `SessionIo`.
//!
//! The program takes plain arguments, so a relay runs it without a shell. It
//! prints the report and, with `--out PATH`, writes an `fps-bench/v1` document.
//! Run it with `--help` for the flags, and in release mode for real numbers.
//!
//! ```text
//! cargo run --release -p fps-tcp --example tcp_bench -- --profile small
//! ```

use std::io::{stderr, stdout};
use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let code = fps_tcp::cli::run(&args, &mut stdout(), &mut stderr());
    ExitCode::from(u8::try_from(code).unwrap_or(1))
}
