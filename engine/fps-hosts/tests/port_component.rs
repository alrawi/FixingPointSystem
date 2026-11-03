// SPDX-License-Identifier: GPL-3.0-only
//! Component tests of kernel port setup and teardown against the recording
//! `ip` fake, so they need no root.

use fps_hosts::{cleanup, setup, FakeIp, HostPool, InterfaceAddr, PoolSpec, PortError, PortSpec};

const NS: &str = "fps-port-t1";
const LINK: &str = r#"[{"ifindex":3,"ifname":"eth1","flags":["BROADCAST","MULTICAST","UP","LOWER_UP"],"mtu":1500,"operstate":"UP"}]"#;
const MGMT_ROUTE: &str = r#"[{"dst":"default","gateway":"192.0.2.1","dev":"mgmt0"}]"#;
const LINK_LOCAL: &str = r#"[{"ifname":"eth1","addr_info":[{"family":"inet6","local":"fe80::1","prefixlen":64,"scope":"link"}]}]"#;
const NS_PRESENT: &str = r#"[{"name":"fps-port-t1"},{"name":"other"}]"#;

/// A pool of 20,000 hosts whose gateway lies outside the prefix, as in the
/// routed harness test.
fn spec() -> PortSpec {
    let pool = HostPool::parse(&PoolSpec {
        name: "clients",
        ipv4: Some("10.10.0.0/16"),
        count: 20_000,
        gateway: Some("10.201.0.1"),
        ..PoolSpec::default()
    })
    .unwrap();
    let mut s = PortSpec::new("t1", "eth1", &pool, pool.range()).unwrap();
    s.address = Some("10.201.0.2/24".parse::<InterfaceAddr>().unwrap());
    s.mtu = Some(9000);
    s
}

/// Answers for a healthy host whose management interface is mgmt0.
fn healthy_host() -> FakeIp {
    let mut ip = FakeIp::new();
    ip.ok("-j netns list", "[]")
        .ok("-j link show dev eth1", LINK)
        .ok("-j -4 route show default", MGMT_ROUTE)
        .ok("-j -6 route show default", "[]")
        .ok("-j addr show dev eth1", LINK_LOCAL);
    let exec = format!("netns exec {NS} sysctl -n ");
    for (key, value) in [
        ("net.ipv4.ip_local_port_range", "1024\t65535\n"),
        ("net.ipv4.tcp_tw_reuse", "1\n"),
        ("net.ipv4.tcp_rmem", "4096\t131072\t16777216\n"),
        ("net.ipv4.tcp_wmem", "4096\t16384\t16777216\n"),
        ("net.core.wmem_max", "16777216\n"),
        ("net.ipv6.conf.all.disable_ipv6", "1\n"),
        ("net.ipv6.conf.default.disable_ipv6", "1\n"),
    ] {
        ip.ok(&format!("{exec}{key}"), value);
    }
    // An older kernel keeps net.core.rmem_max global, so the namespace lacks it.
    ip.fail(&format!("{exec}net.core.rmem_max"), "cannot stat");
    ip
}

/// Lets the namespace appear after `netns add` and vanish after `netns del`.
/// The fake answers with the newest unspent rule, so the answers stack in
/// reverse order. Teardown first sees the namespace, and the check after the
/// delete sees none.
fn namespace_lifecycle(ip: &mut FakeIp) {
    ip.ok_times("-j netns list", "[]", 1)
        .ok_times("-j netns list", NS_PRESENT, 1);
}

#[test]
fn setup_issues_the_documented_commands_in_order() {
    let mut ip = healthy_host();
    let port = setup(&spec(), &mut ip).unwrap();
    assert_eq!(port.namespace(), NS);
    assert_eq!(port.skipped_sysctls(), ["net.core.rmem_max"]);
    assert_eq!(port.original().mtu, 1500);
    assert!(port.original().up);
    let calls = ip.calls();
    let head = [
        "-j netns list",
        "-j link show dev eth1",
        "-j -4 route show default",
        "-j -6 route show default",
        "-j addr show dev eth1",
        "netns add fps-port-t1",
        "-n fps-port-t1 link set dev lo up",
    ];
    assert_eq!(&calls[..head.len()], head);
    let sysctl_end = calls
        .iter()
        .position(|c| c.ends_with("netns fps-port-t1"))
        .unwrap();
    assert!(calls[head.len()..sysctl_end]
        .iter()
        .all(|c| c.starts_with("netns exec fps-port-t1 sysctl ")));
    assert!(calls.contains(&format!(
        "netns exec {NS} sysctl -q -w net.ipv4.ip_local_port_range=1024 65535"
    )));
    assert!(!calls.iter().any(|c| c.contains("-w net.core.rmem_max")));
    let tail: Vec<&str> = calls[sysctl_end..].iter().map(String::as_str).collect();
    assert_eq!(
        tail,
        [
            "link set dev eth1 netns fps-port-t1",
            "-n fps-port-t1 link set dev eth1 mtu 9000 up",
            "-n fps-port-t1 addr add 10.201.0.2/24 dev eth1",
            "-n fps-port-t1 route add local 10.10.0.1/32 dev lo",
            "-n fps-port-t1 route add local 10.10.0.2/31 dev lo",
            "-n fps-port-t1 route add local 10.10.0.4/30 dev lo",
            "-n fps-port-t1 route add local 10.10.0.8/29 dev lo",
            "-n fps-port-t1 route add local 10.10.0.16/28 dev lo",
            "-n fps-port-t1 route add local 10.10.0.32/27 dev lo",
            "-n fps-port-t1 route add local 10.10.0.64/26 dev lo",
            "-n fps-port-t1 route add local 10.10.0.128/25 dev lo",
            "-n fps-port-t1 route add local 10.10.1.0/24 dev lo",
            "-n fps-port-t1 route add local 10.10.2.0/23 dev lo",
            "-n fps-port-t1 route add local 10.10.4.0/22 dev lo",
            "-n fps-port-t1 route add local 10.10.8.0/21 dev lo",
            "-n fps-port-t1 route add local 10.10.16.0/20 dev lo",
            "-n fps-port-t1 route add local 10.10.32.0/19 dev lo",
            "-n fps-port-t1 route add local 10.10.64.0/21 dev lo",
            "-n fps-port-t1 route add local 10.10.72.0/22 dev lo",
            "-n fps-port-t1 route add local 10.10.76.0/23 dev lo",
            "-n fps-port-t1 route add local 10.10.78.0/27 dev lo",
            "-n fps-port-t1 route add local 10.10.78.32/32 dev lo",
            "-n fps-port-t1 -4 route add default via 10.201.0.1 dev eth1 onlink",
        ]
    );
}

#[test]
fn setup_changes_nothing_outside_the_port_namespace_but_the_test_interface() {
    let mut ip = healthy_host();
    let mut s = spec();
    s.vlan = Some(100);
    let port = setup(&s, &mut ip).unwrap();
    namespace_lifecycle(&mut ip);
    port.teardown(&mut ip).unwrap();
    let pid = std::process::id().to_string();
    for c in ip.calls() {
        let in_port = c.starts_with("-n fps-port-t1 ") || c.starts_with("netns exec fps-port-t1 ");
        let read_only = c.starts_with("-j ");
        let namespace = c == "netns add fps-port-t1" || c == "netns del fps-port-t1";
        let move_out = c == "link set dev eth1 netns fps-port-t1";
        let restore = c == "link set dev eth1 mtu 1500 up";
        assert!(
            in_port || read_only || namespace || move_out || restore,
            "command outside the port namespace: {c}"
        );
        assert!(
            !c.contains("mgmt0"),
            "command names the management interface: {c}"
        );
    }
    let calls = ip.calls();
    assert!(
        calls.contains(&"-n fps-port-t1 link add link eth1 name vlan100 type vlan id 100".into())
    );
    assert!(calls.contains(&"-n fps-port-t1 link del dev vlan100".into()));
    assert!(calls.contains(&format!("-n fps-port-t1 link set dev eth1 netns {pid}")));
}

fn refused(ip: &mut FakeIp, s: &PortSpec) -> String {
    let err = setup(s, ip).unwrap_err();
    assert!(
        !ip.calls().iter().any(|c| c.starts_with("netns add")),
        "setup created a namespace before refusing"
    );
    match err {
        PortError::Management { reason, .. } => reason,
        other => panic!("want a management refusal, got {other}"),
    }
}

#[test]
fn setup_refuses_interfaces_that_look_like_management_interfaces() {
    let mut ip = healthy_host();
    ip.ok(
        "-j -4 route show default",
        r#"[{"dst":"default","gateway":"192.0.2.1","dev":"eth1"}]"#,
    );
    assert!(refused(&mut ip, &spec()).contains("default route"));

    let mut ip = healthy_host();
    ip.ok(
        "-j addr show dev eth1",
        r#"[{"ifname":"eth1","addr_info":[{"family":"inet","local":"198.51.100.7","scope":"global"}]}]"#,
    );
    assert!(refused(&mut ip, &spec()).contains("198.51.100.7"));

    let mut ip = healthy_host();
    ip.ok(
        "-j link show dev eth1",
        r#"[{"ifname":"eth1","flags":["UP"],"mtu":1500,"master":"br0"}]"#,
    );
    assert!(refused(&mut ip, &spec()).contains("br0"));

    let mut ip = healthy_host();
    let mut s = spec();
    s.interface = "lo".into();
    ip.ok(
        "-j link show dev lo",
        r#"[{"ifname":"lo","flags":["LOOPBACK","UP"],"mtu":65536}]"#,
    );
    assert!(refused(&mut ip, &s).contains("loopback"));

    let mut ip = healthy_host();
    let mut s = spec();
    s.management = vec!["eth1".into()];
    assert!(refused(&mut ip, &s).contains("management"));
}

#[test]
fn setup_refuses_an_existing_namespace_and_a_missing_interface() {
    let mut ip = healthy_host();
    ip.ok("-j netns list", NS_PRESENT);
    assert!(matches!(
        setup(&spec(), &mut ip),
        Err(PortError::NamespaceExists { .. })
    ));
    let mut ip = healthy_host();
    ip.fail("-j link show dev eth1", "Device \"eth1\" does not exist.");
    assert!(matches!(
        setup(&spec(), &mut ip),
        Err(PortError::InterfaceNotFound { .. })
    ));
    assert!(!ip.calls().iter().any(|c| c.starts_with("netns add")));
}

#[test]
fn failed_step_rolls_back_and_returns_the_first_error() {
    let mut ip = healthy_host();
    namespace_lifecycle(&mut ip);
    // The preflight must see no namespace, so its answer goes on top.
    ip.ok_times("-j netns list", "[]", 1);
    ip.fail(
        "-n fps-port-t1 -4 route add default",
        "Nexthop has invalid gateway",
    );
    let err = setup(&spec(), &mut ip).unwrap_err();
    assert!(
        matches!(
            err,
            PortError::Step {
                step: "add default route",
                ..
            }
        ),
        "{err}"
    );
    let calls = ip.calls();
    let pid = std::process::id().to_string();
    let rollback = calls
        .iter()
        .position(|c| c.ends_with("route add default via 10.201.0.1 dev eth1 onlink"))
        .unwrap();
    let rest: Vec<&str> = calls[rollback + 1..].iter().map(String::as_str).collect();
    assert_eq!(
        rest,
        [
            "-n fps-port-t1 link show dev eth1",
            "-n fps-port-t1 link set dev eth1 down",
            &format!("-n fps-port-t1 link set dev eth1 netns {pid}"),
            "link set dev eth1 mtu 1500 up",
            "-j netns list",
            "netns del fps-port-t1",
            "-j netns list",
        ]
    );
}

#[test]
fn sysctl_that_does_not_take_its_value_fails_setup() {
    let mut ip = healthy_host();
    namespace_lifecycle(&mut ip);
    ip.ok_times("-j netns list", "[]", 1);
    ip.fail("-n fps-port-t1 link show dev eth1", "does not exist");
    ip.ok(
        &format!("netns exec {NS} sysctl -n net.ipv4.tcp_tw_reuse"),
        "2\n",
    );
    match setup(&spec(), &mut ip).unwrap_err() {
        PortError::SysctlMismatch { key, want, got, .. } => {
            assert_eq!(key, "net.ipv4.tcp_tw_reuse");
            assert_eq!(want, "1");
            assert_eq!(got, "2");
        }
        other => panic!("want a sysctl mismatch, got {other}"),
    }
    assert!(ip
        .calls()
        .iter()
        .any(|c| c == "netns del fps-port-t1" || c == "-j netns list"));
}

#[test]
fn teardown_keeps_the_namespace_when_the_interface_cannot_move_back() {
    let mut ip = healthy_host();
    let port = setup(&spec(), &mut ip).unwrap();
    ip.fail("-n fps-port-t1 link set dev eth1 netns", "Invalid argument");
    let err = port.teardown(&mut ip).unwrap_err();
    assert!(matches!(err, PortError::Teardown { .. }), "{err}");
    assert!(!ip.calls().iter().any(|c| c == "netns del fps-port-t1"));
}

#[test]
fn cleanup_is_a_no_op_without_the_namespace_and_removes_remains_otherwise() {
    let mut ip = healthy_host();
    cleanup(&spec(), &mut ip).unwrap();
    assert_eq!(ip.calls(), ["-j netns list"]);

    let mut ip = healthy_host();
    ip.ok_times("-j netns list", "[]", 1)
        .ok_times("-j netns list", NS_PRESENT, 2);
    ip.fail("-n fps-port-t1 link show dev eth1", "does not exist");
    cleanup(&spec(), &mut ip).unwrap();
    let calls = ip.calls();
    assert!(calls.contains(&"netns del fps-port-t1".to_owned()));
    assert!(!calls.iter().any(|c| c.contains("mtu 1500")));
}
