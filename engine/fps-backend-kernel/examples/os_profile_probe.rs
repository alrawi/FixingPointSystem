// SPDX-License-Identifier: GPL-3.0-only
//! Opens kernel backend sessions with an operating system profile.
//!
//! The namespace capture test `tests/netns/profiles.sh` runs this program in
//! the client and server namespaces of the QA-01 harness and checks the
//! handshakes in the capture. Each subcommand takes the name of a built in
//! profile, and `serve` and `connect` also take `none`, which keeps the
//! kernel defaults for the baseline session of the test.
//!
//! ```text
//! os_profile_probe sysctls PROFILE              print name=value per sysctl
//! os_profile_probe expect PROFILE               print the expected handshake
//! os_profile_probe serve PROFILE ADDR:PORT N    accept N sessions, then exit
//! os_profile_probe connect PROFILE SOURCE DEST  open one session from SOURCE
//! ```
//!
//! A session sends one line, shuts its write side down, and reads until end
//! of stream. The server answers every line with `ok`.

use std::error::Error;
use std::net::{IpAddr, SocketAddr};
use std::process::ExitCode;

use fps_backend_kernel::{
    read_proc_sysctl, ConnectConfig, Connector, KernelListener, ListenConfig, OsProfile,
    SocketOptions,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

type Result<T> = std::result::Result<T, Box<dyn Error>>;

const USAGE: &str = "usage: os_profile_probe sysctls|expect PROFILE\n       \
                     os_profile_probe serve PROFILE ADDR:PORT SESSIONS\n       \
                     os_profile_probe connect PROFILE SOURCE DEST";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("os_profile_probe: {e}");
            let mut cause = e.source();
            while let Some(c) = cause {
                eprintln!("  caused by: {c}");
                cause = c.source();
            }
            ExitCode::FAILURE
        }
    }
}

fn run(args: &[String]) -> Result<()> {
    let (command, name, rest) = match args {
        [command, name, rest @ ..] => (command.as_str(), name.as_str(), rest),
        _ => return Err(USAGE.into()),
    };
    if name == "none" {
        return match (command, rest) {
            ("serve", [address, sessions]) => {
                let config = ListenConfig::default();
                runtime()?.block_on(serve(&config, address.parse()?, sessions.parse()?))
            }
            ("connect", [source, destination]) => {
                let config = ConnectConfig::default();
                runtime()?.block_on(connect(&config, source.parse()?, destination.parse()?))
            }
            _ => Err(USAGE.into()),
        };
    }
    let profile = OsProfile::builtin(name)?;
    let socket = profile.socket_options(SocketOptions::default());
    match (command, rest) {
        ("sysctls", []) => {
            for (key, value) in profile.sysctls() {
                println!("{key}={value}");
            }
            Ok(())
        }
        ("expect", []) => {
            let flag = |on: bool| u8::from(on);
            println!("TTL={}", profile.socket.ttl);
            println!("MSS={}", profile.socket.mss);
            println!("WSCALE={}", profile.reference.window_scale);
            println!("SACK={}", flag(profile.system.sack));
            println!("TS={}", flag(profile.system.timestamps));
            println!("OPTIONS={}", profile.reference.options.join(","));
            Ok(())
        }
        ("serve", [address, sessions]) => {
            warn_on_sysctls(&profile);
            let config = ListenConfig {
                socket,
                ..ListenConfig::default()
            };
            runtime()?.block_on(serve(&config, address.parse()?, sessions.parse()?))
        }
        ("connect", [source, destination]) => {
            warn_on_sysctls(&profile);
            let config = ConnectConfig {
                socket,
                ..ConnectConfig::default()
            };
            runtime()?.block_on(connect(&config, source.parse()?, destination.parse()?))
        }
        _ => Err(USAGE.into()),
    }
}

fn runtime() -> Result<tokio::runtime::Runtime> {
    Ok(tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?)
}

/// Warns on standard error when the namespace does not hold the sysctls of
/// the profile, because the handshake then differs from the profile.
fn warn_on_sysctls(profile: &OsProfile) {
    for m in profile.sysctl_mismatches(read_proc_sysctl) {
        eprintln!(
            "os_profile_probe: {} is {}, profile {} needs {}",
            m.name,
            m.found.as_deref().unwrap_or("unreadable"),
            profile.name,
            m.want
        );
    }
}

async fn serve(config: &ListenConfig, address: SocketAddr, sessions: usize) -> Result<()> {
    let listener = KernelListener::bind(address, config)?;
    println!("listening on {}", listener.local_addr());
    for _ in 0..sessions {
        let (mut io, _peer) = listener.accept().await?;
        let mut line = Vec::new();
        io.read_to_end(&mut line).await?;
        io.write_all(b"ok\n").await?;
        io.shutdown().await?;
    }
    Ok(())
}

async fn connect(config: &ConnectConfig, source: IpAddr, destination: SocketAddr) -> Result<()> {
    let connector = Connector::new(config.clone());
    let mut io = connector.connect(source, destination).await?;
    io.write_all(b"hello\n").await?;
    io.shutdown().await?;
    let mut answer = Vec::new();
    io.read_to_end(&mut answer).await?;
    if answer != b"ok\n" {
        return Err(format!("unexpected answer {answer:?}").into());
    }
    println!("session from {source} to {destination} done");
    Ok(())
}
