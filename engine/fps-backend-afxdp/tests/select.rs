// SPDX-License-Identifier: GPL-3.0-only
//! Backend selection on the hosts that the plan names: a virtio-net KVM guest
//! with and without zero copy support, and the hosts around it.

use fps_backend_afxdp::{
    select_backend, AfXdpFacts, BackendKind, KernelFacts, SelectError, Selection, SelectionInput,
    SelectionReason, Unavailable, XdpSupport, TRANSPORT_AVAILABLE,
};

/// A virtio-net guest on a recent kernel. virtio-net runs XDP in the driver
/// with copy mode sockets, and offers zero copy only when the guest kernel and
/// the driver support it. `transport` says whether TCP over AF_XDP exists.
fn virtio_guest(zero_copy: bool, transport: bool) -> SelectionInput {
    SelectionInput {
        forced: None,
        kernel: KernelFacts::default(),
        af_xdp: AfXdpFacts {
            kernel_support: true,
            transport_available: transport,
            driver: if zero_copy {
                XdpSupport::ZeroCopy
            } else {
                XdpSupport::Native
            },
        },
    }
}

#[test]
fn virtio_net_guest_without_zero_copy_selects_af_xdp_in_native_mode() {
    let got = select_backend(&virtio_guest(false, true)).unwrap();
    assert_eq!(
        got,
        Selection {
            backend: BackendKind::AfXdp,
            reason: SelectionReason::Fastest,
            xdp: Some(XdpSupport::Native),
        }
    );
}

#[test]
fn virtio_net_guest_with_zero_copy_selects_af_xdp_in_zero_copy_mode() {
    let got = select_backend(&virtio_guest(true, true)).unwrap();
    assert_eq!(
        got,
        Selection {
            backend: BackendKind::AfXdp,
            reason: SelectionReason::Fastest,
            xdp: Some(XdpSupport::ZeroCopy),
        }
    );
}

#[test]
fn virtio_net_guest_stays_on_the_kernel_while_no_transport_carries_tcp() {
    for zero_copy in [false, true] {
        let got = select_backend(&virtio_guest(zero_copy, false)).unwrap();
        assert_eq!(got.backend, BackendKind::Kernel);
        assert_eq!(
            got.reason,
            SelectionReason::Default(Unavailable::NoTransport)
        );
        assert_eq!(got.xdp, None);
    }
}

#[test]
fn the_default_input_selects_the_kernel_backend() {
    let got = select_backend(&SelectionInput::default()).unwrap();
    assert_eq!(got.backend, BackendKind::Kernel);
    assert_eq!(
        got.reason,
        SelectionReason::Default(Unavailable::NoKernelSupport)
    );
    // The default of the transport fact follows the constant, which TRN-12 flips.
    assert_eq!(
        AfXdpFacts::default().transport_available,
        TRANSPORT_AVAILABLE
    );
}

#[test]
fn the_transport_constant_decides_whether_a_capable_host_selects_af_xdp() {
    // Facts as the agent builds them: the transport fact comes from the
    // constant, which is false until TRN-12 and then true. A forced AF_XDP
    // run is refused while it is false.
    let mut input = virtio_guest(true, TRANSPORT_AVAILABLE);
    assert_eq!(
        select_backend(&input).unwrap().backend == BackendKind::AfXdp,
        TRANSPORT_AVAILABLE
    );
    input.forced = Some(BackendKind::AfXdp);
    assert_eq!(
        select_backend(&input),
        if TRANSPORT_AVAILABLE {
            Ok(Selection {
                backend: BackendKind::AfXdp,
                reason: SelectionReason::Forced,
                xdp: Some(XdpSupport::ZeroCopy),
            })
        } else {
            Err(SelectError::ForcedUnsupported {
                backend: BackendKind::AfXdp,
                why: Unavailable::NoTransport,
            })
        }
    );
}

#[test]
fn old_kernel_without_af_xdp_uses_the_kernel_backend() {
    let mut input = virtio_guest(false, true);
    input.af_xdp.kernel_support = false;
    let got = select_backend(&input).unwrap();
    assert_eq!(got.backend, BackendKind::Kernel);
    assert_eq!(
        got.reason,
        SelectionReason::Default(Unavailable::NoKernelSupport)
    );
}

#[test]
fn generic_only_driver_does_not_win_but_can_be_forced() {
    let mut input = virtio_guest(false, true);
    input.af_xdp.driver = XdpSupport::Generic;
    let got = select_backend(&input).unwrap();
    assert_eq!(got.backend, BackendKind::Kernel);
    assert_eq!(
        got.reason,
        SelectionReason::Default(Unavailable::GenericOnly)
    );

    input.forced = Some(BackendKind::AfXdp);
    assert_eq!(
        select_backend(&input),
        Ok(Selection {
            backend: BackendKind::AfXdp,
            reason: SelectionReason::Forced,
            xdp: Some(XdpSupport::Generic),
        })
    );
}

#[test]
fn a_driver_without_xdp_is_never_selected() {
    let mut input = virtio_guest(false, true);
    input.af_xdp.driver = XdpSupport::None;
    assert_eq!(
        select_backend(&input).unwrap().reason,
        SelectionReason::Default(Unavailable::NoDriverSupport)
    );
    input.forced = Some(BackendKind::AfXdp);
    assert_eq!(
        select_backend(&input),
        Err(SelectError::ForcedUnsupported {
            backend: BackendKind::AfXdp,
            why: Unavailable::NoDriverSupport,
        })
    );
}

#[test]
fn forcing_an_unsupported_backend_is_a_typed_error_and_never_falls_back() {
    // virtio-net guest on a kernel without AF_XDP.
    let mut input = virtio_guest(false, true);
    input.af_xdp.kernel_support = false;
    input.forced = Some(BackendKind::AfXdp);
    let err = select_backend(&input).unwrap_err();
    assert_eq!(
        err,
        SelectError::ForcedUnsupported {
            backend: BackendKind::AfXdp,
            why: Unavailable::NoKernelSupport,
        }
    );
    assert_eq!(
        err.to_string(),
        "backend af_xdp was forced but is unsupported: the kernel offers no AF_XDP sockets"
    );
}

#[test]
fn forcing_the_kernel_backend_overrides_a_faster_choice() {
    let mut input = virtio_guest(true, true);
    input.forced = Some(BackendKind::Kernel);
    assert_eq!(
        select_backend(&input),
        Ok(Selection {
            backend: BackendKind::Kernel,
            reason: SelectionReason::Forced,
            xdp: None,
        })
    );
}

#[test]
fn no_working_backend_is_an_error() {
    let mut input = SelectionInput::default();
    input.kernel.transport_available = false;
    assert_eq!(select_backend(&input), Err(SelectError::NoBackend));
    input.forced = Some(BackendKind::Kernel);
    assert_eq!(
        select_backend(&input),
        Err(SelectError::ForcedUnsupported {
            backend: BackendKind::Kernel,
            why: Unavailable::NoTransport,
        })
    );
}

#[test]
fn backend_names_match_the_contract_style() {
    assert_eq!(BackendKind::Kernel.name(), "kernel");
    assert_eq!(BackendKind::AfXdp.name(), "af_xdp");
}

#[test]
fn xdp_support_orders_from_none_to_zero_copy() {
    assert!(XdpSupport::None < XdpSupport::Generic);
    assert!(XdpSupport::Generic < XdpSupport::Native);
    assert!(XdpSupport::Native < XdpSupport::ZeroCopy);
}
