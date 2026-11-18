// SPDX-License-Identifier: GPL-3.0-only
//! Component tests of the backend selection and its report (AGT-22).
//!
//! The tests run on the fake system roots of `testdata/` and on the binary,
//! so none of them needs root or a network card, and none opens an AF_XDP
//! socket.
//!
//! | Root | Host | What it exercises |
//! |---|---|---|
//! | `virtio-guest` | kernel 6.8 with `CONFIG_XDP_SOCKETS=y`, two virtio-net ports | AF_XDP offered, native XDP without zero copy |
//! | `virtio-guest-no-xdp` | the same guest without `CONFIG_XDP_SOCKETS` | a forced AF_XDP backend is unsupported |
//! | `host-no-xdp` | kernel 5.4, e1000e and a bridge | no AF_XDP, a port that no table knows |
//! | `two-numa` | kernel 6.1, one igb and two ice ports | the weakest port decides |

use std::path::PathBuf;
use std::process::Command;

use fps_agent::proto::{Backend, XdpMode};
use fps_agent::select::{select, selection_input};
use fps_agent::{detect, HostCapabilities, SystemRoot};
use fps_backend_afxdp::{
    BackendKind, SelectError, SelectionReason, Unavailable, XdpSupport, TRANSPORT_AVAILABLE,
};

fn testdata(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("testdata")
        .join(name)
}

fn host(name: &str) -> HostCapabilities {
    match detect(&SystemRoot::new(testdata(name))) {
        Ok(h) => h,
        Err(e) => panic!("detect {name}: {e}"),
    }
}

#[test]
fn the_capabilities_report_the_kernel_release_and_the_selected_backend() {
    for (name, release) in [
        ("virtio-guest", "6.8.0-45-generic"),
        ("virtio-guest-no-xdp", "6.8.0-45-generic"),
        ("host-no-xdp", "5.4.0-150-custom"),
        ("two-numa", "6.1.0-18-amd64"),
    ] {
        let caps = host(name).to_capabilities();
        assert_eq!(caps.kernel_release, release, "{name}");
        assert!(
            caps.backends.contains(&caps.selected_backend),
            "{name}: the selected backend is one entry of backends"
        );
        assert_ne!(caps.selected_backend, Backend::Unspecified, "{name}");
        for port in &caps.ports {
            assert_eq!(
                port.xdp_driver_mode,
                XdpMode::Unspecified,
                "{name}: the kernel backend reports no XDP mode"
            );
        }
    }
}

#[test]
fn a_host_selects_the_kernel_backend_until_a_transport_carries_tcp_over_af_xdp() {
    let guest = host("virtio-guest");
    let selection = select(&guest, None).expect("a default selection works");
    if TRANSPORT_AVAILABLE {
        // TRN-12 flipped the constant: the guest has native XDP and AF_XDP.
        assert_eq!(selection.backend, BackendKind::AfXdp);
        assert_eq!(selection.xdp, Some(XdpSupport::Native));
        let caps = guest.capabilities_for(&selection);
        assert_eq!(caps.selected_backend, Backend::AfXdp);
        assert!(caps
            .ports
            .iter()
            .all(|p| p.xdp_driver_mode == XdpMode::Native));
    } else {
        assert_eq!(selection.backend, BackendKind::Kernel);
        assert_eq!(
            selection.reason,
            SelectionReason::Default(Unavailable::NoTransport)
        );
        assert_eq!(
            guest.to_capabilities().selected_backend,
            Backend::Kernel,
            "the report follows the selection"
        );
    }
}

#[test]
fn the_input_of_each_fixture_follows_its_ports() {
    // virtio_net on 6.8 has native XDP and no zero copy.
    assert_eq!(
        selection_input(&host("virtio-guest"), None).af_xdp.driver,
        XdpSupport::Native
    );
    // One igb port (native, no zero copy in the table) and two ice ports (zero
    // copy): the weakest port decides, so the input says Native.
    assert_eq!(
        selection_input(&host("two-numa"), None).af_xdp.driver,
        XdpSupport::Native
    );
    // The bridge has no driver, so the weakest port has no XDP at all.
    let old = selection_input(&host("host-no-xdp"), None);
    assert_eq!(old.af_xdp.driver, XdpSupport::None);
    assert!(!old.af_xdp.kernel_support);
    assert!(
        selection_input(&host("virtio-guest"), None)
            .af_xdp
            .kernel_support
    );
}

#[test]
fn a_forced_backend_that_the_host_cannot_run_is_an_error_and_never_a_fallback() {
    for name in ["virtio-guest-no-xdp", "host-no-xdp"] {
        let err = select(&host(name), Some(BackendKind::AfXdp))
            .expect_err("AF_XDP is not offered on this host");
        assert_eq!(
            err,
            SelectError::ForcedUnsupported {
                backend: BackendKind::AfXdp,
                why: Unavailable::NoKernelSupport,
            },
            "{name}"
        );
    }
    // The kernel backend is forced without trouble on the same hosts.
    for name in ["virtio-guest-no-xdp", "host-no-xdp", "virtio-guest"] {
        let selection = select(&host(name), Some(BackendKind::Kernel)).expect("kernel works");
        assert_eq!(selection.backend, BackendKind::Kernel, "{name}");
        assert_eq!(selection.reason, SelectionReason::Forced, "{name}");
    }
}

#[test]
fn a_forced_af_xdp_works_before_the_transport_constant_is_flipped() {
    // AGT-24: a forced AF_XDP run no longer meets NoTransport. The guest has
    // AF_XDP sockets and native XDP, so the selection takes it, whatever
    // TRANSPORT_AVAILABLE says, and the report follows.
    let guest = host("virtio-guest");
    let selection = select(&guest, Some(BackendKind::AfXdp)).expect("forced AF_XDP works");
    assert_eq!(selection.backend, BackendKind::AfXdp);
    assert_eq!(selection.reason, SelectionReason::Forced);
    assert_eq!(selection.xdp, Some(XdpSupport::Native));
    let caps = guest.capabilities_for(&selection);
    assert_eq!(caps.selected_backend, Backend::AfXdp);
    assert!(caps
        .ports
        .iter()
        .all(|p| p.xdp_driver_mode == XdpMode::Native));
    // The default selection stays the kernel backend until TRN-31.
    if !TRANSPORT_AVAILABLE {
        let default = select(&guest, None).expect("a default selection works");
        assert_eq!(default.backend, BackendKind::Kernel);
    }
    // A host without AF_XDP sockets still refuses it.
    let err = select(&host("virtio-guest-no-xdp"), Some(BackendKind::AfXdp))
        .expect_err("no AF_XDP sockets");
    assert_eq!(
        err,
        SelectError::ForcedUnsupported {
            backend: BackendKind::AfXdp,
            why: Unavailable::NoKernelSupport,
        }
    );
}

#[test]
fn a_forced_backend_reaches_the_report_when_it_works() {
    let guest = host("virtio-guest");
    let selection = select(&guest, Some(BackendKind::Kernel)).expect("kernel works");
    let caps = guest.capabilities_for(&selection);
    assert_eq!(caps.selected_backend, Backend::Kernel);
    assert_eq!(caps.kernel_release, "6.8.0-45-generic");
}

fn connect(root: &str, backend: &[&str]) -> std::process::Output {
    let modules =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../contracts/examples/modules");
    match Command::new(env!("CARGO_BIN_EXE_fps-agent"))
        .args(["connect", "--controller", "http://127.0.0.1:1"])
        .args(["--agent-id", "select-test", "--modules"])
        .arg(modules)
        .arg("--root")
        .arg(testdata(root))
        .args(["--redial-limit", "1"])
        .args(backend)
        .output()
    {
        Ok(out) => out,
        Err(e) => panic!("run fps-agent: {e}"),
    }
}

#[test]
fn the_boot_fails_with_a_clear_message_for_an_unsupported_forced_backend() {
    let out = connect("virtio-guest-no-xdp", &["--backend", "af_xdp"]);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(2), "{stderr}");
    assert!(
        stderr.contains(
            "backend selection failed: backend af_xdp was forced but is unsupported: \
             the kernel offers no AF_XDP sockets"
        ),
        "{stderr}"
    );
    assert!(!stderr.contains("session lost"), "dialed anyway: {stderr}");
    assert!(
        !stderr.contains("gave up redialing"),
        "dialed anyway: {stderr}"
    );
}

#[test]
fn a_forced_backend_that_works_lets_the_agent_dial() {
    // The selection passes and the agent goes on to the dial, which fails
    // after the redial limit with exit code 3 and not with the code 2 of a
    // failed selection.
    let out = connect("virtio-guest", &["--backend", "kernel"]);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(3), "{stderr}");
    assert!(
        stderr.contains("selected the transport backend"),
        "{stderr}"
    );
    assert!(stderr.contains(r#""backend":"kernel""#), "{stderr}");
    assert!(stderr.contains("forced by the operator"), "{stderr}");
}

#[test]
fn a_forced_af_xdp_backend_lets_the_agent_dial_before_the_flip() {
    // Before AGT-24 this boot failed with "no transport carries TCP over this
    // backend". Now the selection passes and the agent goes on to the dial.
    let out = connect("virtio-guest", &["--backend", "af_xdp"]);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(3), "{stderr}");
    assert!(stderr.contains(r#""backend":"af_xdp""#), "{stderr}");
    assert!(stderr.contains("forced by the operator"), "{stderr}");
    assert!(!stderr.contains("backend selection failed"), "{stderr}");
}

#[test]
fn an_unknown_backend_name_is_a_usage_error() {
    let out = connect("virtio-guest", &["--backend", "dpdk"]);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(2), "{stderr}");
    assert!(stderr.contains("unknown backend"), "{stderr}");
}

#[test]
fn the_capabilities_command_prints_the_release_and_the_selection() {
    let out = match Command::new(env!("CARGO_BIN_EXE_fps-agent"))
        .arg("capabilities")
        .arg("--root")
        .arg(testdata("virtio-guest"))
        .output()
    {
        Ok(out) => out,
        Err(e) => panic!("run fps-agent: {e}"),
    };
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(out.status.success(), "{stdout}");
    assert!(
        stdout.contains("kernel_release 6.8.0-45-generic\n"),
        "{stdout}"
    );
    assert!(
        stdout.contains("selected_backend BACKEND_KERNEL\n")
            || stdout.contains("selected_backend BACKEND_AF_XDP\n"),
        "{stdout}"
    );
    assert!(stdout.contains("selection_reason "), "{stdout}");
}
