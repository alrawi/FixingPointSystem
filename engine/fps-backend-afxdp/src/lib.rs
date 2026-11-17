// SPDX-License-Identifier: GPL-3.0-only
//! AF_XDP packet input and output of the engine.
//!
//! The crate gives a user space transport whole Ethernet frames in batches
//! on one queue of a network device. [`FrameIo`] is the seam. It has batched
//! receive and send calls that lend frame memory and never block, so the
//! transport above it, for example a user space TCP stack, runs on one thread
//! per queue without locks.
//!
//! [`open_port`] implements the seam for AF_XDP. It opens one socket per
//! queue, each with its own UMEM and the four rings (fill, completion,
//! receive, and transmit), attaches a small XDP program that sends the frames
//! of each queue to its socket, and returns an [`AfXdpQueue`] per queue.
//! A queue binds in zero copy mode when the driver offers it and falls back
//! to copy mode otherwise, and [`AfXdpQueue::bind_mode`] reads the mode that
//! the socket really runs in. [`Workers`] runs one thread per queue, pinned
//! to a CPU if asked. [`memory::frame_pair`] is an in memory fake with the
//! same trait for tests above this crate.
//!
//! [`select_backend`] chooses between this backend and the kernel backend at
//! boot from facts that the agent passes in, and refuses a forced backend that
//! cannot run.
//!
//! The defaults of [`QueueConfig`] come from spike S3. The crate depends only
//! on `libc`. It needs Linux 5.4 or newer for AF_XDP with wake up flags, 5.9 for the BPF link
//! that attaches the program, and root or the capabilities that
//! [`open_port`] lists.
//!
//! The crate denies `unsafe` code except in `sys`, which calls the kernel,
//! and `ring`, which shares memory with it. Every block says why it is safe.
#![deny(unsafe_code)]
#![warn(missing_docs, clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod config;
mod error;
mod frame;
pub mod memory;
mod select;
mod worker;

#[cfg(all(target_os = "linux", target_pointer_width = "64"))]
mod port;
#[cfg(all(target_os = "linux", target_pointer_width = "64"))]
mod queue;
#[cfg(all(target_os = "linux", target_pointer_width = "64"))]
#[allow(unsafe_code)]
mod ring;
#[cfg(all(target_os = "linux", target_pointer_width = "64"))]
#[allow(unsafe_code)]
mod sys;

pub use config::{BindMode, BoundMode, ConfigError, PortConfig, QueueConfig, XDP_PACKET_HEADROOM};
pub use error::AfXdpError;
pub use frame::{FrameIo, QueueStats};
pub use select::{
    select_backend, AfXdpFacts, BackendKind, KernelFacts, SelectError, Selection, SelectionInput,
    SelectionReason, Unavailable, XdpSupport, TRANSPORT_AVAILABLE,
};
pub use worker::{Idle, QueueLoop, WorkerOptions, WorkerReport, Workers};

#[cfg(all(target_os = "linux", target_pointer_width = "64"))]
pub use port::{open_port, OpenedPort, PortInfo};
#[cfg(all(target_os = "linux", target_pointer_width = "64"))]
pub use queue::{AfXdpQueue, QueueInfo};
#[cfg(all(target_os = "linux", target_pointer_width = "64"))]
pub use sys::{KernelStats, XdpMode};
