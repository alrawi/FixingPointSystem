// SPDX-License-Identifier: GPL-3.0-only
//! Backend selection at boot (AGT-22).
//!
//! `fps_backend_afxdp::select_backend` (TRN-14) picks the transport backend
//! from a [`SelectionInput`] of its own, because [`HostCapabilities`] lives in
//! this crate and the backend crates cannot depend on it. This module builds
//! that input from the detection and maps the answer to the proto types that
//! `Register` carries.
//!
//! | `SelectionInput` field | Source |
//! |---|---|
//! | `forced` | the operator, through `--backend` of `fps-agent connect` |
//! | `kernel` | `KernelFacts::default()`, which the kernel backend crate decides |
//! | `af_xdp.kernel_support` | [`HostCapabilities::af_xdp`], where only a `Supported` kernel counts and `Unknown` does not |
//! | `af_xdp.transport_available` | `AfXdpFacts::default()`, which copies `fps_backend_afxdp::TRANSPORT_AVAILABLE`, and `true` when the operator forces AF_XDP (AGT-24) |
//! | `af_xdp.driver` | the weakest port, see [`weakest_driver`] |
//!
//! The selection reads facts that detection already collected. It opens no
//! AF_XDP socket and probes no device, so it needs no privilege and runs
//! the same on every host. The mode that a bound socket reads back at run
//! time stays with the queue that bound it. What this module reports before
//! any queue exists is the driver table hint of CON-32.
//!
//! # Reported fields
//!
//! | Field | Value |
//! |---|---|
//! | `Capabilities.kernel_release` | [`HostCapabilities::kernel`] `.release` |
//! | `Capabilities.selected_backend` | [`backend_of`] the selected [`BackendKind`] |
//! | `PortInfo.xdp_driver_mode` | [`xdp_mode_of`] the driver table of the port when the selected backend is AF_XDP, else `XDP_MODE_UNSPECIFIED` |
//!
//! A port reports its own driver table mode, so a mixed host shows which port
//! is the slow one, while the selection uses the weakest port because every
//! port of a run must keep up.

use fps_backend_afxdp::{
    select_backend, AfXdpFacts, BackendKind, KernelFacts, SelectError, Selection, SelectionInput,
    SelectionReason, XdpSupport,
};

use crate::capabilities::{DetectedPort, HostCapabilities};
use crate::proto::{Backend, XdpMode};

/// Returns the best XDP mode that the driver table gives one port.
///
/// A port with a driver that is not in the table of native drivers still takes
/// the generic hook, which every device supports. A port with no driver name
/// (a bridge, or a device without a `uevent`) is not probed, so it is
/// [`XdpSupport::None`].
pub fn port_support(port: &DetectedPort) -> XdpSupport {
    if port.xdp.zero_copy {
        XdpSupport::ZeroCopy
    } else if port.xdp.native {
        XdpSupport::Native
    } else if port.info.driver.is_empty() {
        XdpSupport::None
    } else {
        XdpSupport::Generic
    }
}

/// Returns the best XDP mode that every port offers: the weakest port decides.
///
/// A host with no port offers [`XdpSupport::None`], because no AF_XDP queue
/// can bind.
pub fn weakest_driver(ports: &[DetectedPort]) -> XdpSupport {
    ports
        .iter()
        .map(port_support)
        .min()
        .unwrap_or(XdpSupport::None)
}

/// Builds the input of `select_backend` from the detection.
///
/// `forced` is the backend that the operator asks for, or `None` to let the
/// rules choose. The transport facts come from the defaults of the backend
/// crates, so TRN-31 turns AF_XDP on by changing a constant of its own crate.
///
/// An operator who forces AF_XDP gets it before that flip (AGT-24): the
/// datapath of TRN-27 is a transport that exists, and the constant only keeps
/// AF_XDP from becoming the default of every host before the own stack has
/// loss recovery and congestion control. So a forced AF_XDP run sets
/// `transport_available`, and the other checks (kernel support and driver)
/// still decide. Without a forced backend the facts stay as they are.
pub fn selection_input(host: &HostCapabilities, forced: Option<BackendKind>) -> SelectionInput {
    let defaults = AfXdpFacts::default();
    SelectionInput {
        forced,
        kernel: KernelFacts::default(),
        af_xdp: AfXdpFacts {
            kernel_support: host.af_xdp.is_supported(),
            transport_available: defaults.transport_available || forced == Some(BackendKind::AfXdp),
            driver: weakest_driver(&host.ports),
        },
    }
}

/// Selects the transport backend for `host`.
///
/// # Errors
///
/// Returns [`SelectError::ForcedUnsupported`] when `forced` names a backend
/// that this host cannot run, and [`SelectError::NoBackend`] when nothing is
/// forced and no backend can run. A forced backend never falls back to the
/// other one.
pub fn select(
    host: &HostCapabilities,
    forced: Option<BackendKind>,
) -> Result<Selection, SelectError> {
    select_backend(&selection_input(host, forced))
}

/// Maps a selected backend to the proto enum.
pub fn backend_of(kind: BackendKind) -> Backend {
    match kind {
        BackendKind::Kernel => Backend::Kernel,
        BackendKind::AfXdp => Backend::AfXdp,
        // BackendKind is non exhaustive. A backend that this agent cannot name
        // is reported as not specified, which the contract defines as "not
        // reported".
        _ => Backend::Unspecified,
    }
}

/// Maps the XDP support of a port to the proto enum. [`XdpSupport::None`] is
/// `XDP_MODE_UNSPECIFIED`.
pub fn xdp_mode_of(support: XdpSupport) -> XdpMode {
    match support {
        XdpSupport::None => XdpMode::Unspecified,
        XdpSupport::Generic => XdpMode::Generic,
        XdpSupport::Native => XdpMode::Native,
        XdpSupport::ZeroCopy => XdpMode::ZeroCopy,
    }
}

/// Reads the value of `--backend`: `kernel` or `af_xdp`.
pub fn parse_backend(name: &str) -> Option<BackendKind> {
    [BackendKind::Kernel, BackendKind::AfXdp]
        .into_iter()
        .find(|k| k.name() == name)
}

/// Says in one sentence why the selection chose its backend, for the log and
/// for `fps-agent capabilities`.
pub fn reason_text(reason: SelectionReason) -> String {
    match reason {
        SelectionReason::Forced => "forced by the operator".to_owned(),
        SelectionReason::Fastest => "AF_XDP is the fastest backend that works".to_owned(),
        SelectionReason::Default(why) => {
            format!("the kernel backend is the default, and AF_XDP was not chosen: {why}")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capabilities::{driver_xdp, AfXdpSupport, DriverXdp, KernelVersion, XdpEvidence};
    use crate::proto::PortInfo;
    use fps_backend_afxdp::{Unavailable, TRANSPORT_AVAILABLE};

    fn port(name: &str, driver: &str, kernel: &KernelVersion) -> DetectedPort {
        DetectedPort {
            info: PortInfo {
                name: name.to_owned(),
                driver: driver.to_owned(),
                speed_mbps: 10_000,
                mac: "52:54:00:00:00:01".to_owned(),
                mtu: 1500,
                xdp_driver_mode: XdpMode::Unspecified,
            },
            numa_node: None,
            xdp: driver_xdp(driver, kernel),
        }
    }

    /// A fake host. Nothing reads `/proc` or `/sys`, and no test here needs
    /// root or a network card.
    fn host(release: &str, af_xdp: AfXdpSupport, drivers: &[&str]) -> HostCapabilities {
        let kernel = KernelVersion::parse(release).expect("test release parses");
        let ports = drivers
            .iter()
            .enumerate()
            .map(|(i, d)| port(&format!("eth{i}"), d, &kernel))
            .collect();
        HostCapabilities {
            online_cpus: vec![0, 1, 2, 3],
            numa_nodes: Vec::new(),
            memory_total_bytes: 8 << 30,
            memory_available_bytes: 7 << 30,
            kernel,
            af_xdp,
            ports,
        }
    }

    const SUPPORTED: AfXdpSupport = AfXdpSupport::Supported(XdpEvidence::KernelConfig);
    const UNSUPPORTED: AfXdpSupport = AfXdpSupport::Unsupported(XdpEvidence::KernelConfig);

    #[test]
    fn the_input_takes_the_transport_facts_from_the_backend_crates() {
        let h = host("6.12.0", SUPPORTED, &["ice"]);
        let input = selection_input(&h, None);
        assert_eq!(input.forced, None);
        assert_eq!(input.kernel, KernelFacts::default());
        assert!(input.af_xdp.kernel_support);
        assert_eq!(
            input.af_xdp.transport_available,
            AfXdpFacts::default().transport_available
        );
        assert_eq!(input.af_xdp.transport_available, TRANSPORT_AVAILABLE);
        assert_eq!(input.af_xdp.driver, XdpSupport::ZeroCopy);
    }

    #[test]
    fn only_a_supported_kernel_counts_as_af_xdp_support() {
        for (support, expected) in [
            (SUPPORTED, true),
            (UNSUPPORTED, false),
            (AfXdpSupport::Unknown, false),
        ] {
            let h = host("6.12.0", support, &["ice"]);
            assert_eq!(selection_input(&h, None).af_xdp.kernel_support, expected);
        }
    }

    #[test]
    fn the_forced_backend_reaches_the_input() {
        let h = host("6.12.0", SUPPORTED, &["ice"]);
        assert_eq!(
            selection_input(&h, Some(BackendKind::AfXdp)).forced,
            Some(BackendKind::AfXdp)
        );
        assert_eq!(
            selection_input(&h, Some(BackendKind::Kernel)).forced,
            Some(BackendKind::Kernel)
        );
    }

    #[test]
    fn a_port_takes_the_best_mode_its_driver_table_gives() {
        let k = KernelVersion::parse("6.12.0").unwrap();
        assert_eq!(port_support(&port("a", "ice", &k)), XdpSupport::ZeroCopy);
        // Zero copy of virtio_net starts with Linux 6.11.
        assert_eq!(
            port_support(&port("a", "virtio_net", &k)),
            XdpSupport::ZeroCopy
        );
        let old = KernelVersion::parse("6.8.0-45-generic").unwrap();
        assert_eq!(
            port_support(&port("a", "virtio_net", &old)),
            XdpSupport::Native
        );
        assert_eq!(port_support(&port("a", "veth", &old)), XdpSupport::Native);
        assert_eq!(
            port_support(&port("a", "e1000e", &old)),
            XdpSupport::Generic
        );
        assert_eq!(port_support(&port("br0", "", &old)), XdpSupport::None);
    }

    #[test]
    fn the_weakest_port_decides_the_driver() {
        let k = KernelVersion::parse("6.12.0").unwrap();
        let ice = port("p0", "ice", &k);
        let virtio_old = {
            let old = KernelVersion::parse("6.8.0").unwrap();
            port("p1", "virtio_net", &old)
        };
        let e1000e = port("p2", "e1000e", &k);
        let bridge = port("br0", "", &k);

        assert_eq!(weakest_driver(&[]), XdpSupport::None, "no port, no queue");
        assert_eq!(
            weakest_driver(std::slice::from_ref(&ice)),
            XdpSupport::ZeroCopy
        );
        assert_eq!(
            weakest_driver(&[ice.clone(), virtio_old.clone()]),
            XdpSupport::Native
        );
        assert_eq!(
            weakest_driver(&[virtio_old.clone(), e1000e.clone(), ice.clone()]),
            XdpSupport::Generic,
            "one generic only port holds the run back, whatever the order"
        );
        assert_eq!(
            weakest_driver(&[ice, virtio_old, e1000e, bridge]),
            XdpSupport::None,
            "a port that no table knows is not offered to AF_XDP"
        );
    }

    #[test]
    fn a_mixed_host_selects_from_its_weakest_port() {
        // The ice port could use zero copy, the e1000e port only the generic
        // hook, so the input says Generic.
        let h = host("6.12.0", SUPPORTED, &["ice", "e1000e"]);
        let input = selection_input(&h, None);
        assert_eq!(input.af_xdp.driver, XdpSupport::Generic);
        // And the answer is the kernel backend, with the generic policy as
        // the reason when a transport exists, or the missing transport.
        let sel = select(&h, None).unwrap();
        assert_eq!(sel.backend, BackendKind::Kernel);
        let expected = if TRANSPORT_AVAILABLE {
            Unavailable::GenericOnly
        } else {
            Unavailable::NoTransport
        };
        assert_eq!(sel.reason, SelectionReason::Default(expected));
    }

    #[test]
    fn a_capable_host_selects_the_backend_that_the_transport_constant_allows() {
        let h = host("6.12.0", SUPPORTED, &["ice", "ice"]);
        let sel = select(&h, None).unwrap();
        if TRANSPORT_AVAILABLE {
            assert_eq!(sel.backend, BackendKind::AfXdp);
            assert_eq!(sel.reason, SelectionReason::Fastest);
            assert_eq!(sel.xdp, Some(XdpSupport::ZeroCopy));
        } else {
            assert_eq!(sel.backend, BackendKind::Kernel);
            assert_eq!(
                sel.reason,
                SelectionReason::Default(Unavailable::NoTransport)
            );
            assert_eq!(sel.xdp, None);
        }
    }

    #[test]
    fn a_host_without_af_xdp_selects_the_kernel_backend() {
        for support in [UNSUPPORTED, AfXdpSupport::Unknown] {
            let h = host("6.8.0", support, &["virtio_net"]);
            let sel = select(&h, None).unwrap();
            assert_eq!(sel.backend, BackendKind::Kernel);
            assert_eq!(
                sel.reason,
                SelectionReason::Default(Unavailable::NoKernelSupport)
            );
        }
    }

    #[test]
    fn the_kernel_backend_can_be_forced_on_any_host() {
        for h in [
            host("6.12.0", SUPPORTED, &["ice"]),
            host("6.8.0", UNSUPPORTED, &["e1000e"]),
            host("6.8.0", AfXdpSupport::Unknown, &[]),
        ] {
            let sel = select(&h, Some(BackendKind::Kernel)).unwrap();
            assert_eq!(sel.backend, BackendKind::Kernel);
            assert_eq!(sel.reason, SelectionReason::Forced);
        }
    }

    #[test]
    fn a_forced_af_xdp_without_kernel_support_fails_and_does_not_fall_back() {
        for support in [UNSUPPORTED, AfXdpSupport::Unknown] {
            let h = host("6.8.0", support, &["virtio_net"]);
            let err = select(&h, Some(BackendKind::AfXdp)).unwrap_err();
            assert_eq!(
                err,
                SelectError::ForcedUnsupported {
                    backend: BackendKind::AfXdp,
                    why: Unavailable::NoKernelSupport,
                }
            );
            let message = err.to_string();
            assert!(message.contains("af_xdp"), "{message}");
            assert!(message.contains("forced"), "{message}");
            assert!(message.contains("no AF_XDP sockets"), "{message}");
        }
    }

    #[test]
    fn a_forced_af_xdp_on_a_host_without_a_driver_fails() {
        // A supported kernel and a host without ports. A forced AF_XDP run
        // has a transport (AGT-24), so the driver is the cause, whatever the
        // transport constant says.
        let h = host("6.8.0", SUPPORTED, &[]);
        let err = select(&h, Some(BackendKind::AfXdp)).unwrap_err();
        assert_eq!(
            err,
            SelectError::ForcedUnsupported {
                backend: BackendKind::AfXdp,
                why: Unavailable::NoDriverSupport,
            }
        );
        // A bridge has no driver, so no queue could bind to it.
        let h = host("6.12.0", SUPPORTED, &[""]);
        let err = select(&h, Some(BackendKind::AfXdp)).unwrap_err();
        assert_eq!(
            err.to_string(),
            "backend af_xdp was forced but is unsupported: the driver takes no XDP program"
        );
    }

    #[test]
    fn a_forced_af_xdp_runs_before_the_transport_constant_is_flipped() {
        // The test holds before and after TRN-31, because a forced run does
        // not read the constant.
        let h = host("6.12.0", SUPPORTED, &["ice"]);
        let input = selection_input(&h, Some(BackendKind::AfXdp));
        assert!(input.af_xdp.transport_available);
        let sel = select(&h, Some(BackendKind::AfXdp)).unwrap();
        assert_eq!(sel.backend, BackendKind::AfXdp);
        assert_eq!(sel.reason, SelectionReason::Forced);
        assert_eq!(sel.xdp, Some(XdpSupport::ZeroCopy));
        // A forced run on a generic only host is the operator's call.
        let h = host("6.12.0", SUPPORTED, &["e1000e"]);
        let sel = select(&h, Some(BackendKind::AfXdp)).unwrap();
        assert_eq!(sel.backend, BackendKind::AfXdp);
        assert_eq!(sel.xdp, Some(XdpSupport::Generic));
    }

    #[test]
    fn without_a_forced_af_xdp_the_transport_fact_is_the_constant() {
        let h = host("6.12.0", SUPPORTED, &["ice"]);
        for forced in [None, Some(BackendKind::Kernel)] {
            assert_eq!(
                selection_input(&h, forced).af_xdp.transport_available,
                TRANSPORT_AVAILABLE,
                "{forced:?}"
            );
        }
        if !TRANSPORT_AVAILABLE {
            // The default selection stays the kernel backend until TRN-31.
            let sel = select(&h, None).unwrap();
            assert_eq!(sel.backend, BackendKind::Kernel);
            assert_eq!(
                sel.reason,
                SelectionReason::Default(Unavailable::NoTransport)
            );
        }
    }

    #[test]
    fn an_af_xdp_selection_reports_the_driver_table_mode_of_every_port() {
        // Hand built selection, so the test holds before and after TRN-12
        // turns the transport constant on.
        let mut h = host("6.12.0", SUPPORTED, &["ice", "e1000e", ""]);
        let old = KernelVersion::parse("6.8.0").unwrap();
        h.ports.push(port("eth9", "virtio_net", &old));
        let selection = Selection {
            backend: BackendKind::AfXdp,
            reason: SelectionReason::Forced,
            xdp: Some(XdpSupport::None),
        };
        let caps = h.capabilities_for(&selection);
        assert_eq!(caps.selected_backend, Backend::AfXdp);
        assert!(caps.backends.contains(&Backend::AfXdp));
        assert_eq!(caps.kernel_release, "6.12.0");
        let modes: Vec<XdpMode> = caps.ports.iter().map(|p| p.xdp_driver_mode).collect();
        assert_eq!(
            modes,
            [
                XdpMode::ZeroCopy,
                XdpMode::Generic,
                XdpMode::Unspecified,
                XdpMode::Native
            ]
        );
    }

    #[test]
    fn a_kernel_selection_reports_no_xdp_mode_and_the_plain_ports() {
        let h = host("6.12.0", SUPPORTED, &["ice", "virtio_net"]);
        let selection = Selection {
            backend: BackendKind::Kernel,
            reason: SelectionReason::Forced,
            xdp: None,
        };
        let caps = h.capabilities_for(&selection);
        assert_eq!(caps.selected_backend, Backend::Kernel);
        assert!(caps
            .ports
            .iter()
            .all(|p| p.xdp_driver_mode == XdpMode::Unspecified));
        let plain: Vec<PortInfo> = h.ports.iter().map(|p| p.info.clone()).collect();
        assert_eq!(caps.ports, plain, "nothing else of a port changes");
    }

    #[test]
    fn a_host_where_no_backend_runs_reports_no_selection() {
        // Not reachable with the real kernel facts, which are always
        // available, so the report path is exercised through `fill`.
        let h = host("6.8.0", UNSUPPORTED, &["virtio_net"]);
        let caps = h.fill(None);
        assert_eq!(caps.selected_backend, Backend::Unspecified);
        assert_eq!(caps.kernel_release, "6.8.0");
    }

    #[test]
    fn the_enums_map_to_the_proto_numbers() {
        assert_eq!(backend_of(BackendKind::Kernel), Backend::Kernel);
        assert_eq!(backend_of(BackendKind::AfXdp), Backend::AfXdp);
        assert_eq!(xdp_mode_of(XdpSupport::None), XdpMode::Unspecified);
        assert_eq!(xdp_mode_of(XdpSupport::Generic), XdpMode::Generic);
        assert_eq!(xdp_mode_of(XdpSupport::Native), XdpMode::Native);
        assert_eq!(xdp_mode_of(XdpSupport::ZeroCopy), XdpMode::ZeroCopy);
        // The numbers match the contract enum (CON-32).
        assert_eq!(xdp_mode_of(XdpSupport::Native).number(), 2);
        assert_eq!(backend_of(BackendKind::AfXdp).number(), 2);
    }

    #[test]
    fn the_backend_flag_takes_the_names_of_the_backend_crate() {
        assert_eq!(parse_backend("kernel"), Some(BackendKind::Kernel));
        assert_eq!(parse_backend("af_xdp"), Some(BackendKind::AfXdp));
        for bad in ["", "AF_XDP", "dpdk", "afxdp", "kernel "] {
            assert_eq!(parse_backend(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn the_reason_names_the_cause() {
        assert_eq!(
            reason_text(SelectionReason::Forced),
            "forced by the operator"
        );
        let text = reason_text(SelectionReason::Default(Unavailable::NoKernelSupport));
        assert!(
            text.contains("default") && text.contains("no AF_XDP sockets"),
            "{text}"
        );
    }

    #[test]
    fn the_driver_table_types_stay_what_the_mapping_assumes() {
        // port_support reads both flags, so a zero copy driver that the table
        // forgot to list as native still wins over Native.
        let k = KernelVersion::parse("6.12.0").unwrap();
        let mut p = port("a", "e1000e", &k);
        p.xdp = DriverXdp {
            native: false,
            zero_copy: true,
        };
        assert_eq!(port_support(&p), XdpSupport::ZeroCopy);
    }
}
