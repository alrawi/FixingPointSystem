// SPDX-License-Identifier: GPL-3.0-only
//! Backend selection at boot.
//!
//! [`select_backend`] picks the fastest transport backend that works on a host
//! from a small set of facts, and says why. The facts are a type of this
//! module, [`SelectionInput`], because the detected host capabilities
//! (`HostCapabilities`) live in `fps-agent`, which depends on the backends. The
//! agent builds a [`SelectionInput`] from its detection and passes it in
//! (work package AGT-22). This module depends on nothing, so it builds and
//! tests on every platform.
//!
//! The rules are these.
//!
//! - The default is the kernel backend. It works on every host.
//! - AF_XDP wins only when it works: the kernel offers AF_XDP sockets, the
//!   driver runs XDP natively (copy mode or zero copy), and a transport really
//!   carries TCP over AF_XDP ([`TRANSPORT_AVAILABLE`]). Nothing does before
//!   TRN-12, so the answer today is the kernel backend on every host.
//! - Generic XDP, which runs after the kernel allocated a buffer, is the
//!   slowest mode, so it never wins by itself. An operator can still force it.
//! - A forced backend is honoured when it works and refused with a
//!   [`SelectError`] when it does not. The selection never falls back from a
//!   forced backend, because a silent fallback would make a benchmark
//!   measure the wrong thing.

/// Whether a transport that carries TCP over AF_XDP exists in the engine.
///
/// It is `false` until TRN-12 (user space TCP over AF_XDP) lands, because
/// [`crate::FrameIo`] moves frames but nothing above it speaks TCP. TRN-12
/// sets it to `true`. The agent copies it into
/// [`AfXdpFacts::transport_available`] through [`AfXdpFacts::default`].
pub const TRANSPORT_AVAILABLE: bool = false;

/// A transport backend that the selection can name.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum BackendKind {
    /// Kernel sockets, `fps-backend-kernel`.
    Kernel,
    /// AF_XDP sockets with a user space transport, this crate.
    AfXdp,
}

impl BackendKind {
    /// The name that operators and logs use: `kernel` or `af_xdp`.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Kernel => "kernel",
            Self::AfXdp => "af_xdp",
        }
    }
}

/// The best XDP mode that the driver of a port offers, from the least to the
/// most capable. The order of the variants is the order of the modes, so
/// comparisons work.
///
/// The variants map to the contract enum `XdpMode` of the agent protocol:
/// [`XdpSupport::Generic`] to `GENERIC`, [`XdpSupport::Native`] to `NATIVE`,
/// and [`XdpSupport::ZeroCopy`] to `ZERO_COPY`. [`XdpSupport::None`] is
/// `UNSPECIFIED`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub enum XdpSupport {
    /// The device takes no XDP program, or the agent did not probe it.
    #[default]
    None,
    /// Only the generic hook, which every device has and which is slowest.
    Generic,
    /// The program runs in the driver and sockets use copy mode, which a
    /// virtio-net guest offers without zero copy support.
    Native,
    /// The program runs in the driver and sockets use zero copy.
    ZeroCopy,
}

/// What the agent knows about the kernel backend.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct KernelFacts {
    /// Whether the kernel backend carries sessions on this host. It does
    /// everywhere, see `fps_backend_kernel::TRANSPORT_AVAILABLE`.
    pub transport_available: bool,
}

impl Default for KernelFacts {
    fn default() -> Self {
        Self {
            transport_available: true,
        }
    }
}

/// What the agent knows about AF_XDP on this host.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AfXdpFacts {
    /// Whether the running kernel offers AF_XDP sockets.
    pub kernel_support: bool,
    /// Whether a transport carries TCP over AF_XDP, see
    /// [`TRANSPORT_AVAILABLE`].
    pub transport_available: bool,
    /// The best XDP mode that every port of the run offers, so the weakest
    /// port decides.
    pub driver: XdpSupport,
}

impl Default for AfXdpFacts {
    /// No kernel support and no driver support, with
    /// [`TRANSPORT_AVAILABLE`] as the transport fact.
    fn default() -> Self {
        Self {
            kernel_support: false,
            transport_available: TRANSPORT_AVAILABLE,
            driver: XdpSupport::None,
        }
    }
}

/// The input of [`select_backend`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SelectionInput {
    /// The backend that the operator forces, or `None` to let the selection
    /// choose.
    pub forced: Option<BackendKind>,
    /// The facts about the kernel backend.
    pub kernel: KernelFacts,
    /// The facts about AF_XDP.
    pub af_xdp: AfXdpFacts,
}

/// Why a backend cannot run on a host.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum Unavailable {
    /// The kernel offers no AF_XDP sockets.
    #[error("the kernel offers no AF_XDP sockets")]
    NoKernelSupport,
    /// No transport carries TCP over this backend yet.
    #[error("no transport carries TCP over this backend")]
    NoTransport,
    /// The driver takes no XDP program.
    #[error("the driver takes no XDP program")]
    NoDriverSupport,
    /// The driver offers only the generic XDP hook, which is too slow to
    /// select without an operator's request.
    #[error("the driver offers only generic XDP, which the selection policy does not choose on its own (S3 measured only native copy and zero copy; TRN-12 revisits it)")]
    GenericOnly,
}

/// Why the selection chose a backend.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SelectionReason {
    /// The operator forced the backend and it works.
    Forced,
    /// AF_XDP works and is the fastest backend.
    Fastest,
    /// The kernel backend is the default, and AF_XDP was not chosen for the
    /// reason given.
    Default(Unavailable),
}

/// The backend that the selection chose.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Selection {
    /// The chosen backend.
    pub backend: BackendKind,
    /// Why it was chosen.
    pub reason: SelectionReason,
    /// The XDP mode that the driver offers, which is the mode that the agent
    /// expects the queues to bind in. It is `Some` only for
    /// [`BackendKind::AfXdp`]. The bound mode that a socket reports at run
    /// time is the one that the agent reports to the controller.
    pub xdp: Option<XdpSupport>,
}

/// A forced backend that cannot run, or a host where no backend can.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum SelectError {
    /// The operator forced a backend that this host cannot run.
    #[error("backend {} was forced but is unsupported: {why}", .backend.name())]
    ForcedUnsupported {
        /// The forced backend.
        backend: BackendKind,
        /// Why it cannot run.
        why: Unavailable,
    },
    /// The kernel backend cannot run, and AF_XDP was not chosen either.
    #[error("no backend can run on this host")]
    NoBackend,
}

/// Selects the transport backend for a run.
///
/// # Errors
///
/// Returns [`SelectError::ForcedUnsupported`] when the operator forced a
/// backend that this host cannot run, and [`SelectError::NoBackend`] when
/// nothing is forced and not even the kernel backend can run.
pub fn select_backend(input: &SelectionInput) -> Result<Selection, SelectError> {
    let af_xdp = af_xdp_check(&input.af_xdp);
    match input.forced {
        Some(BackendKind::Kernel) => {
            if input.kernel.transport_available {
                Ok(Selection {
                    backend: BackendKind::Kernel,
                    reason: SelectionReason::Forced,
                    xdp: None,
                })
            } else {
                Err(SelectError::ForcedUnsupported {
                    backend: BackendKind::Kernel,
                    why: Unavailable::NoTransport,
                })
            }
        }
        Some(BackendKind::AfXdp) => match af_xdp {
            Ok(_) | Err(Unavailable::GenericOnly) => Ok(Selection {
                backend: BackendKind::AfXdp,
                reason: SelectionReason::Forced,
                xdp: Some(input.af_xdp.driver),
            }),
            Err(why) => Err(SelectError::ForcedUnsupported {
                backend: BackendKind::AfXdp,
                why,
            }),
        },
        None => match af_xdp {
            Ok(mode) => Ok(Selection {
                backend: BackendKind::AfXdp,
                reason: SelectionReason::Fastest,
                xdp: Some(mode),
            }),
            Err(why) if input.kernel.transport_available => Ok(Selection {
                backend: BackendKind::Kernel,
                reason: SelectionReason::Default(why),
                xdp: None,
            }),
            Err(_) => Err(SelectError::NoBackend),
        },
    }
}

/// Checks AF_XDP in the order of the cheapest cause to fix. `Ok` carries the
/// mode that wins over kernel sockets. `Err(GenericOnly)` is a backend that
/// works but does not win.
fn af_xdp_check(facts: &AfXdpFacts) -> Result<XdpSupport, Unavailable> {
    if !facts.kernel_support {
        return Err(Unavailable::NoKernelSupport);
    }
    if !facts.transport_available {
        return Err(Unavailable::NoTransport);
    }
    match facts.driver {
        XdpSupport::None => Err(Unavailable::NoDriverSupport),
        XdpSupport::Generic => Err(Unavailable::GenericOnly),
        mode @ (XdpSupport::Native | XdpSupport::ZeroCopy) => Ok(mode),
    }
}
