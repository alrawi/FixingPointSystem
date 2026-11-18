// SPDX-License-Identifier: GPL-3.0-only
//! The kernel reference stack on an impaired link in network namespaces
//! (TRN-22).
//!
//! The scenarios need root, `ip`, `tc`, and for two of them the `sch_netem`
//! module. As a normal user the binary runs itself again through `sudo -n`,
//! which GitHub hosted runners and the owner's hosts allow, and it skips with a
//! message when that is not possible or when the kernel lacks a qdisc.
//! `CI=true` or `FPS_REQUIRE_NETEM=1` turns a skip for missing root, sudo, or
//! `ip` into a failure, so a runner that cannot run the scenarios does not
//! pass silently. A kernel without the qdisc modules (`sch_netem`, `sch_tbf`)
//! is a property of the runner image that a package cannot repair, so only
//! `FPS_REQUIRE_NETEM=1` turns that skip into a failure, and a workflow sets
//! it once the image has the modules.
//!
//! The target has its own `main` (`harness = false`), because the server and
//! the client of a transfer are this binary again, started with `ip netns
//! exec` and the flag `--netns-peer`, and the default test harness would
//! refuse that flag.

use std::path::PathBuf;
use std::process::{Command, ExitCode};
use std::time::Duration;

use fps_tcp::impair::ImpairConfig;
use fps_tcp::netns::{self, NetnsError, PEER_FLAG};
use fps_tcp::workload::{LossConfig, LossRun};

fn explicit() -> bool {
    std::env::var("FPS_REQUIRE_NETEM").is_ok_and(|v| v == "1")
}

fn required() -> bool {
    std::env::var("CI").is_ok_and(|v| v == "true") || explicit()
}

/// A scenario that the kernel cannot run for lack of a qdisc.
fn skip_qdisc(what: &str, why: &str) -> Result<(), String> {
    if explicit() {
        Err(format!(
            "{what} cannot run here and FPS_REQUIRE_NETEM=1: {why}"
        ))
    } else {
        eprintln!("skipping {what}: {why}");
        Ok(())
    }
}

/// A scenario that cannot run here is a skip, or a failure where the
/// environment requires it. Returns the exit code to stop with, if any.
fn skip(what: &str, why: &str) -> Result<(), String> {
    if required() {
        Err(format!("{what} cannot run here and CI requires it: {why}"))
    } else {
        eprintln!("skipping {what}: {why}");
        Ok(())
    }
}

fn exe() -> PathBuf {
    std::env::current_exe().expect("the path of this binary")
}

fn config(bytes: u64) -> LossConfig {
    LossConfig {
        bytes,
        timeout: Duration::from_secs(60),
        port: 9300,
        ..LossConfig::default()
    }
}

fn transfer(tag: &str, link: &ImpairConfig, bytes: u64) -> Result<LossRun, NetnsError> {
    let prefix = format!("fpstcpt{}{tag}", std::process::id() % 100_000);
    netns::kernel_transfer(&exe(), &prefix, link, &config(bytes), 1500, None, None)
}

fn leftover_namespaces() -> bool {
    Command::new("ip")
        .args(["netns", "list"])
        .output()
        .is_ok_and(|o| String::from_utf8_lossy(&o.stdout).contains("fpstcpt"))
}

/// A qdisc that the kernel lacks ends a scenario as a skip.
fn missing_qdisc(error: &NetnsError) -> bool {
    match error {
        NetnsError::NetemMissing(_) => true,
        NetnsError::Command { detail, .. } => detail.contains("kind is unknown"),
        NetnsError::Peer(_) => false,
    }
}

fn rate_limit_caps_the_goodput() -> Result<(), String> {
    let link = ImpairConfig {
        rate_bps: 100_000_000,
        queue_bytes: 256 * 1024,
        ..ImpairConfig::default()
    };
    match transfer("r", &link, 2 * 1024 * 1024) {
        Ok(run) => {
            if !run.completed {
                return Err(format!("the rated transfer did not complete: {run:?}"));
            }
            let goodput = run.goodput_mbit_per_s();
            // 100 Mbit/s of link carries less goodput, because headers and the
            // token bucket take a share; the cap must hold either way.
            if !(10.0..=102.0).contains(&goodput) {
                return Err(format!(
                    "goodput {goodput} Mbit/s is not a rate limited value"
                ));
            }
            if leftover_namespaces() {
                return Err("the namespaces were not removed".into());
            }
            println!("ok rate_limit_caps_the_goodput ({goodput:.1} Mbit/s)");
            Ok(())
        }
        Err(error) if missing_qdisc(&error) => {
            skip_qdisc("the rate limit scenario", &error.to_string())
        }
        Err(error) => Err(error.to_string()),
    }
}

fn delay_is_on_both_directions() -> Result<(), String> {
    let link = ImpairConfig {
        delay_us: 20_000,
        ..ImpairConfig::default()
    };
    match transfer("d", &link, 256 * 1024) {
        Ok(run) => {
            if !run.completed {
                return Err(format!("the delayed transfer did not complete: {run:?}"));
            }
            // The data takes 20 ms and the confirmation 20 ms.
            if run.elapsed < Duration::from_millis(38) {
                return Err(format!("elapsed {:?} is below one round trip", run.elapsed));
            }
            println!("ok delay_is_on_both_directions ({:?})", run.elapsed);
            Ok(())
        }
        Err(error) if missing_qdisc(&error) => skip_qdisc("the delay scenario", &error.to_string()),
        Err(error) => Err(error.to_string()),
    }
}

fn loss_makes_the_kernel_retransmit() -> Result<(), String> {
    let link = ImpairConfig {
        loss_ppm: 20_000,
        delay_us: 5_000,
        ..ImpairConfig::default()
    };
    match transfer("l", &link, 2 * 1024 * 1024) {
        Ok(run) => {
            if !run.completed || run.retransmits == 0 {
                return Err(format!(
                    "expected a completed transfer with retransmits: {run:?}"
                ));
            }
            println!(
                "ok loss_makes_the_kernel_retransmit ({} segments)",
                run.retransmits
            );
            Ok(())
        }
        Err(error) if missing_qdisc(&error) => skip_qdisc("the loss scenario", &error.to_string()),
        Err(error) => Err(error.to_string()),
    }
}

fn run_again_as_root() -> Result<(), String> {
    let sudo = Command::new("sudo").args(["-n", "true"]).status();
    if !sudo.is_ok_and(|s| s.success()) {
        return skip(
            "the namespace scenarios",
            "not root and no passwordless sudo",
        );
    }
    // sudo drops the environment, so the child learns the requirement here.
    let flag = format!("FPS_REQUIRE_NETEM={}", u8::from(required()));
    let status = Command::new("sudo")
        .args(["-n", "env", &flag])
        .arg(exe())
        .status()
        .map_err(|e| e.to_string())?;
    if status.success() {
        Ok(())
    } else {
        Err("the scenarios failed under sudo".into())
    }
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) == Some(PEER_FLAG) {
        let code = netns::peer_main(&args[1..]);
        return ExitCode::from(u8::try_from(code).unwrap_or(1));
    }
    let outcome = if netns::is_root() {
        if Command::new("ip").arg("-V").output().is_err() {
            skip("the namespace scenarios", "ip is not installed")
        } else {
            rate_limit_caps_the_goodput()
                .and_then(|()| delay_is_on_both_directions())
                .and_then(|()| loss_makes_the_kernel_retransmit())
        }
    } else {
        run_again_as_root()
    };
    match outcome {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("netns test failed: {message}");
            ExitCode::FAILURE
        }
    }
}
