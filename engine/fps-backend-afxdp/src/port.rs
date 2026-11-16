// SPDX-License-Identifier: GPL-3.0-only
//! Opening a port: one XSK map, one socket per queue, and the XDP program.

use std::sync::Arc;

use crate::config::PortConfig;
use crate::error::AfXdpError;
use crate::queue::{AfXdpQueue, QueueInfo};
use crate::sys::{self, XdpMode, XskMap};

/// What [`open_port`] did, as the kernel reports it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PortInfo {
    /// The interface name.
    pub interface: String,
    /// The interface index.
    pub ifindex: u32,
    /// Whether the XDP program runs in the driver or in the generic hook.
    pub xdp_mode: XdpMode,
    /// One entry per queue, in the order of [`PortConfig::queues`], with the
    /// bind mode read from each socket.
    pub queues: Vec<QueueInfo>,
}

/// An open port and its queues. Dropping the last queue detaches the XDP
/// program, so the kernel stack sees every frame again.
#[derive(Debug)]
pub struct OpenedPort {
    /// What was opened.
    pub info: PortInfo,
    /// One queue per entry of [`PortConfig::queues`], in the same order. Give
    /// each to its own worker, see [`crate::Workers`].
    pub queues: Vec<AfXdpQueue>,
}

/// Opens every queue of `config` and attaches the XDP program that steers
/// frames to them.
///
/// The program sends a frame that arrives on queue `n` to the socket of queue
/// `n`, and passes it to the kernel stack when no socket serves that queue, so
/// queues that the port does not open keep working. The program attaches in
/// the driver when the device supports it and in the generic hook otherwise.
/// Each queue binds with the requested [`crate::BindMode`], and the mode that
/// the socket reports comes back in [`PortInfo::queues`].
///
/// The call needs `CAP_NET_ADMIN`, `CAP_NET_RAW`, and `CAP_BPF` or `CAP_SYS_ADMIN`
/// in the network namespace of the interface, and it fails when the interface
/// already has an XDP program.
///
/// # Errors
///
/// Returns [`AfXdpError::Config`] for a refused setting, and
/// [`AfXdpError::Syscall`] or [`AfXdpError::Program`] for the step that the
/// kernel refused. A request for [`crate::BindMode::ZeroCopy`] on a driver
/// without zero copy fails at the step `bind`, which
/// [`AfXdpError::is_zero_copy_unsupported`] recognizes.
pub fn open_port(config: &PortConfig) -> Result<OpenedPort, AfXdpError> {
    config.validate(sys::page_size())?;
    let interface = config.interface.as_str();
    let port_fail = |step: &'static str| {
        move |source: std::io::Error| AfXdpError::Syscall {
            step,
            interface: interface.to_owned(),
            queue: None,
            source,
        }
    };
    let ifindex = sys::if_nametoindex(interface).map_err(port_fail("look up the interface"))?;
    let entries = config
        .queues
        .iter()
        .max()
        .map_or(1, |m| m.saturating_add(1));
    let map = XskMap::new(entries).map_err(port_fail("create the XSK map"))?;
    let mut queues = Vec::with_capacity(config.queues.len());
    let mut infos = Vec::with_capacity(config.queues.len());
    for &queue_id in &config.queues {
        let q = AfXdpQueue::open(interface, ifindex, queue_id, &config.queue, &map)?;
        infos.push(QueueInfo {
            queue_id,
            requested: config.queue.mode,
            bound: q.bind_mode().map_err(port_fail("read XDP_OPTIONS"))?,
        });
        queues.push(q);
    }
    let attached = sys::attach_redirect(ifindex, map).map_err(|e| {
        if e.log.is_empty() {
            AfXdpError::Syscall {
                step: e.step,
                interface: interface.to_owned(),
                queue: None,
                source: e.source,
            }
        } else {
            AfXdpError::Program {
                interface: interface.to_owned(),
                detail: format!("{}: {}", e.source, e.log),
            }
        }
    })?;
    let xdp_mode = attached.mode;
    let attached = Arc::new(attached);
    for q in &mut queues {
        q.keep_attached(Arc::clone(&attached));
    }
    Ok(OpenedPort {
        info: PortInfo {
            interface: interface.to_owned(),
            ifindex,
            xdp_mode,
            queues: infos,
        },
        queues,
    })
}
