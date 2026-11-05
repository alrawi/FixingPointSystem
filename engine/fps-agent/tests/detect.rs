// SPDX-License-Identifier: GPL-3.0-only
//! Component tests of capability detection against fake system roots.
//!
//! Each directory under `testdata/` stands in for `/`. The files are
//! synthetic and hold only the lines that detection reads.
//!
//! | Root | Host | What it exercises |
//! |---|---|---|
//! | `virtio-guest` | KVM guest, two virtio-net interfaces, 4 vCPUs, one NUMA node, kernel 6.8 | driver from the virtio `uevent`, speed `-1`, AF_XDP from the kernel config, no zero copy before 6.11 |
//! | `host-no-xdp` | e1000e NIC and a bridge, kernel 5.4 without `CONFIG_XDP_SOCKETS` | AF_XDP unsupported, cores from `proc/cpuinfo`, no NUMA directory, uppercase MAC, interface without a device |
//! | `two-numa` | two NUMA nodes, 16 CPUs, two ice ports on node 1, kernel 6.1 without a config file | per node CPUs and memory, port NUMA node, AF_XDP from `proc/kallsyms`, zero copy hint |

use std::path::PathBuf;

use fps_agent::capabilities::{AfXdpSupport, DriverXdp, XdpEvidence};
use fps_agent::proto::{Backend, PortInfo};
use fps_agent::{detect, DetectError, HostCapabilities, SystemRoot};

fn root(name: &str) -> SystemRoot {
    SystemRoot::new(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("testdata")
            .join(name),
    )
}

fn detect_fixture(name: &str) -> HostCapabilities {
    match detect(&root(name)) {
        Ok(host) => host,
        Err(e) => panic!("detect {name}: {e}"),
    }
}

const GIB: u64 = 1024 * 1024 * 1024;

#[test]
fn virtio_guest_reports_virtio_net_ports_and_af_xdp() {
    let host = detect_fixture("virtio-guest");
    let caps = host.to_capabilities();

    assert_eq!(caps.cores, 4);
    assert_eq!(caps.memory_bytes, 7 * GIB);
    assert_eq!(caps.backends, vec![Backend::Kernel, Backend::AfXdp]);
    assert_eq!(caps.engine_api, "0.1.0");
    assert_eq!(caps.agent_version, env!("CARGO_PKG_VERSION"));
    assert!(caps.modules.is_empty() && caps.labels.is_empty());

    assert_eq!(
        caps.ports,
        vec![
            PortInfo {
                name: "eth0".into(),
                driver: "virtio_net".into(),
                speed_mbps: 0,
                mac: "52:54:00:12:34:56".into(),
                mtu: 1500,
            },
            PortInfo {
                name: "eth1".into(),
                driver: "virtio_net".into(),
                speed_mbps: 0,
                mac: "52:54:00:12:34:57".into(),
                mtu: 1500,
            },
        ]
    );

    assert_eq!(host.kernel.release, "6.8.0-45-generic");
    assert_eq!((host.kernel.major, host.kernel.minor), (6, 8));
    assert_eq!(
        host.af_xdp,
        AfXdpSupport::Supported(XdpEvidence::KernelConfig)
    );
    assert_eq!(host.numa_nodes.len(), 1);
    for port in &host.ports {
        assert_eq!(port.numa_node, None);
        // virtio_net gained AF_XDP zero copy in 6.11, so a 6.8 guest copies.
        assert_eq!(
            port.xdp,
            DriverXdp {
                native: true,
                zero_copy: false
            }
        );
    }
}

#[test]
fn host_without_xdp_offers_only_the_kernel_backend() {
    let host = detect_fixture("host-no-xdp");
    let caps = host.to_capabilities();

    assert_eq!(
        host.af_xdp,
        AfXdpSupport::Unsupported(XdpEvidence::KernelConfig)
    );
    assert_eq!(caps.backends, vec![Backend::Kernel]);
    assert_eq!((host.kernel.major, host.kernel.minor), (5, 4));

    // No sys/devices/system/cpu/online, so cores come from proc/cpuinfo.
    assert_eq!(host.online_cpus, vec![0, 1]);
    assert_eq!(caps.cores, 2);
    // No node directory, so the host reports no NUMA nodes.
    assert!(host.numa_nodes.is_empty());
    assert_eq!(host.memory_total_bytes, 16_314_612 * 1024);
    assert_eq!(caps.memory_bytes, 12_000_000 * 1024);

    let names: Vec<&str> = caps.ports.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(
        names,
        ["br0", "eno1"],
        "loopback is not a traffic interface"
    );
    let br0 = &host.ports[0];
    assert_eq!(br0.info.driver, "", "a bridge has no device and no driver");
    assert_eq!(br0.info.speed_mbps, 0);
    let eno1 = &host.ports[1];
    assert_eq!(eno1.info.driver, "e1000e");
    assert_eq!(eno1.info.speed_mbps, 1000);
    assert_eq!(eno1.info.mtu, 9000);
    assert_eq!(eno1.info.mac, "a0:36:9f:00:00:01");
    assert_eq!(eno1.numa_node, None, "numa_node -1 means no node");
    assert_eq!(eno1.xdp, DriverXdp::default());
}

#[test]
fn two_numa_host_reports_nodes_and_port_locality() {
    let host = detect_fixture("two-numa");
    let caps = host.to_capabilities();

    assert_eq!(caps.cores, 16);
    assert_eq!(host.numa_nodes.len(), 2);
    let node0 = &host.numa_nodes[0];
    let node1 = &host.numa_nodes[1];
    assert_eq!((node0.id, node1.id), (0, 1));
    assert_eq!(node0.cpus, (0..8).collect::<Vec<u32>>());
    assert_eq!(node1.cpus, (8..16).collect::<Vec<u32>>());
    assert_eq!(node0.memory_bytes, 32_919_816 * 1024);
    assert_eq!(node1.memory_bytes, 32_919_816 * 1024);
    let node_cpus: usize = host.numa_nodes.iter().map(|n| n.cpus.len()).sum();
    assert_eq!(node_cpus, host.online_cpus.len());

    // No boot config, so the kallsyms entry for xsk_create decides.
    assert_eq!(
        host.af_xdp,
        AfXdpSupport::Supported(XdpEvidence::KernelSymbols)
    );
    assert_eq!(caps.backends, vec![Backend::Kernel, Backend::AfXdp]);

    let summary: Vec<(&str, &str, u32, Option<u32>, bool)> = host
        .ports
        .iter()
        .map(|p| {
            (
                p.info.name.as_str(),
                p.info.driver.as_str(),
                p.info.speed_mbps,
                p.numa_node,
                p.xdp.zero_copy,
            )
        })
        .collect();
    assert_eq!(
        summary,
        [
            ("eno1", "igb", 1000, Some(0), false),
            ("ens1f0", "ice", 25000, Some(1), true),
            ("ens1f1", "ice", 25000, Some(1), true),
        ]
    );
}

#[test]
fn empty_root_reports_no_cpus() {
    let missing = root("does-not-exist");
    match detect(&missing) {
        Err(DetectError::NoCpus { .. }) => {}
        other => panic!("expected NoCpus, got {other:?}"),
    }
}

#[test]
fn the_running_host_detects_without_error() {
    // An integration check on the CI runner or developer machine. Sandboxes
    // without a Linux /proc skip it.
    let host = SystemRoot::host();
    if !host.path("proc/meminfo").exists() {
        return;
    }
    match detect(&host) {
        Ok(found) => {
            let caps = found.to_capabilities();
            assert!(caps.cores >= 1);
            assert!(caps.memory_bytes > 0);
            assert!(caps.backends.contains(&Backend::Kernel));
        }
        Err(e) => panic!("detect on the running host: {e}"),
    }
}

#[test]
fn an_old_kernel_is_refused_with_an_error_that_names_the_port_range_option() {
    // host-no-xdp reports 5.4.0-150-custom.
    let err = match fps_agent::require_kernel(&root("host-no-xdp")) {
        Ok(kernel) => panic!("kernel {} passed the check", kernel.release),
        Err(e) => e,
    };
    let text = err.to_string();
    assert!(
        matches!(
            &err,
            fps_agent::KernelError::TooOld { release, required: (6, 3) }
                if release == "5.4.0-150-custom"
        ),
        "{err:?}"
    );
    assert!(text.contains("IP_LOCAL_PORT_RANGE"), "{text}");
    assert!(text.contains("Linux 6.3 or newer"), "{text}");
    assert!(text.contains("5.4.0-150-custom"), "{text}");
}

#[test]
fn kernels_from_6_3_on_pass_and_6_2_does_not() {
    use fps_agent::capabilities::KernelVersion;
    // virtio-guest reports 6.8.0-45-generic, two-numa reports 6.1.0-18-amd64.
    assert!(fps_agent::require_kernel(&root("virtio-guest")).is_ok());
    assert!(fps_agent::require_kernel(&root("two-numa")).is_err());
    for (release, passes) in [
        ("6.3.0", true),
        ("6.18.44-fc-v236", true),
        ("7.0.1", true),
        ("6.2.16-generic", false),
        ("5.15.0-91-generic", false),
    ] {
        let kernel = match KernelVersion::parse(release) {
            Some(k) => k,
            None => panic!("release {release} does not parse"),
        };
        assert_eq!(
            kernel.require_ip_local_port_range().is_ok(),
            passes,
            "{release}"
        );
    }
}

#[test]
fn a_missing_kernel_release_is_a_detect_error_not_a_pass() {
    let err = fps_agent::require_kernel(&SystemRoot::new("/nonexistent-fps-root"));
    assert!(matches!(err, Err(fps_agent::KernelError::Detect(_))));
}
