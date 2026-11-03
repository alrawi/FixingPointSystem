// SPDX-License-Identifier: GPL-3.0-only
//! The narrow wrapper around `ip(8)` that port setup uses.
//!
//! Port setup talks to the kernel over rtnetlink through iproute2. Every
//! command goes through [`IpRunner`], which runs the `ip` program with an
//! argument vector and no shell, so a value can never become a second command.
//! The module [`crate::port`] builds every argument vector from validated
//! names and addresses. [`FakeIp`] records commands and replays scripted
//! answers, so component tests run without root.

use std::ffi::OsString;
use std::process::{Command, Stdio};

/// A failed `ip` command.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("ip {args}: {detail}")]
pub struct CommandError {
    /// The arguments, joined by spaces.
    pub args: String,
    /// The exit status and the standard error output, or the spawn error.
    pub detail: String,
}

/// Runs `ip` with an argument vector and returns its standard output.
pub trait IpRunner {
    /// Runs `ip ARGS...` and returns the standard output on exit status 0.
    fn ip(&mut self, args: &[&str]) -> Result<String, CommandError>;
}

/// The real runner, which executes the `ip` program from `PATH` or from a
/// fixed path.
#[derive(Debug, Clone)]
pub struct IpCommand {
    program: OsString,
}

impl Default for IpCommand {
    fn default() -> Self {
        Self::new()
    }
}

impl IpCommand {
    /// A runner for the `ip` program found in `PATH`.
    pub fn new() -> Self {
        Self {
            program: OsString::from("ip"),
        }
    }

    /// A runner for the `ip` program at `program`, such as `/usr/sbin/ip`.
    pub fn with_program(program: impl Into<OsString>) -> Self {
        Self {
            program: program.into(),
        }
    }
}

impl IpRunner for IpCommand {
    fn ip(&mut self, args: &[&str]) -> Result<String, CommandError> {
        let joined = args.join(" ");
        let out = Command::new(&self.program)
            .args(args)
            .stdin(Stdio::null())
            .output()
            .map_err(|e| CommandError {
                args: joined.clone(),
                detail: format!("cannot run ip: {e}"),
            })?;
        if !out.status.success() {
            let stderr = String::from_utf8_lossy(&out.stderr);
            return Err(CommandError {
                args: joined,
                detail: format!("{}: {}", out.status, stderr.trim()),
            });
        }
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    }
}

/// One scripted answer of [`FakeIp`].
#[derive(Debug, Clone)]
struct Rule {
    prefix: String,
    answer: Result<String, String>,
    remaining: Option<usize>,
}

/// A fake runner that records every command and answers from a script.
///
/// A rule matches a command whose arguments, joined by spaces, start with the
/// rule's prefix. The newest matching rule wins, so a test can override an
/// earlier answer. A command without a matching rule succeeds with empty
/// output.
#[derive(Debug, Clone, Default)]
pub struct FakeIp {
    rules: Vec<Rule>,
    calls: Vec<String>,
}

impl FakeIp {
    /// A fake without rules.
    pub fn new() -> Self {
        Self::default()
    }

    /// Answers every command that starts with `prefix` with `stdout`.
    pub fn ok(&mut self, prefix: &str, stdout: &str) -> &mut Self {
        self.push(prefix, Ok(stdout.to_owned()), None)
    }

    /// Fails every command that starts with `prefix` with `stderr`.
    pub fn fail(&mut self, prefix: &str, stderr: &str) -> &mut Self {
        self.push(prefix, Err(stderr.to_owned()), None)
    }

    /// Answers the next `times` commands that start with `prefix` with
    /// `stdout`, and then lets older rules answer.
    pub fn ok_times(&mut self, prefix: &str, stdout: &str, times: usize) -> &mut Self {
        self.push(prefix, Ok(stdout.to_owned()), Some(times))
    }

    fn push(
        &mut self,
        prefix: &str,
        answer: Result<String, String>,
        remaining: Option<usize>,
    ) -> &mut Self {
        self.rules.push(Rule {
            prefix: prefix.to_owned(),
            answer,
            remaining,
        });
        self
    }

    /// Every command so far, with its arguments joined by spaces.
    pub fn calls(&self) -> &[String] {
        &self.calls
    }
}

impl IpRunner for FakeIp {
    fn ip(&mut self, args: &[&str]) -> Result<String, CommandError> {
        let joined = args.join(" ");
        self.calls.push(joined.clone());
        let rule = self
            .rules
            .iter_mut()
            .rev()
            .find(|r| joined.starts_with(&r.prefix) && r.remaining != Some(0));
        let Some(rule) = rule else {
            return Ok(String::new());
        };
        if let Some(n) = rule.remaining.as_mut() {
            *n -= 1;
        }
        rule.answer.clone().map_err(|detail| CommandError {
            args: joined,
            detail,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fake_prefers_newest_rule_and_counts_one_shot_rules() {
        let mut ip = FakeIp::new();
        ip.ok("link show", "old")
            .ok("link show dev", "new")
            .ok_times("link show dev x", "once", 1);
        assert_eq!(ip.ip(&["link", "show", "dev", "x"]).unwrap(), "once");
        assert_eq!(ip.ip(&["link", "show", "dev", "x"]).unwrap(), "new");
        assert_eq!(ip.ip(&["link", "show"]).unwrap(), "old");
        assert_eq!(ip.ip(&["route"]).unwrap(), "");
        ip.fail("route", "boom");
        let err = ip.ip(&["route", "add"]).unwrap_err();
        assert_eq!(err.args, "route add");
        assert_eq!(err.detail, "boom");
        assert_eq!(ip.calls().len(), 5);
    }

    #[test]
    fn real_runner_reports_a_missing_program() {
        let mut ip = IpCommand::with_program("/nonexistent/fps-ip");
        let err = ip.ip(&["link"]).unwrap_err();
        assert!(err.detail.starts_with("cannot run ip"), "{err}");
    }
}
