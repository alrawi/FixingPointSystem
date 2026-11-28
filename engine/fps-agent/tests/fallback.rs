// SPDX-License-Identifier: GPL-3.0-only
//! The kernel fallback of an agent that nobody forced to AF_XDP (AGT-29).
//!
//! The tests run on the fake system root `testdata/virtio-guest` (kernel 6.8,
//! AF_XDP sockets, native XDP on two virtio-net ports), so none of them needs
//! root or a network card, and none opens an AF_XDP socket. The real
//! `TRANSPORT_AVAILABLE` stays `false`: the tests ask the selection for the
//! answer that it will give after the flip of TRN-31 through
//! `select_with_transport`, which sets the fact for one call.
//!
//! An unforced agent then selects AF_XDP with the reason `Fastest`, and a
//! forced one with the reason `Forced`. `prepare_selected` gets
//! `fall_back = reason != Forced`, as `fps-agent connect` sets it. A plan that
//! the datapath refuses runs on the kernel backend of the first and is refused
//! with its cause by the second.
//!
//! The acceptance plans are the test configurations
//! `contracts/examples/tests/http_smoke.yaml` and `enterprise_mix_10g.yaml`,
//! turned into agent plans the way the controller does: ports from the
//! locations, host pools, the mix, and one worker.

use std::path::Path;

use fps_agent::assemble::Sides;
use fps_agent::backend::BackendError;
use fps_agent::plan::Plan;
use fps_agent::run::{prepare_selected, Prepared, RunError};
use fps_agent::select::select_with_transport;
use fps_agent::tls::KeyTap;
use fps_agent::trust::ModulePolicy;
use fps_agent::{detect, HostCapabilities, SystemRoot};
use fps_backend_afxdp::{BackendKind, Selection, SelectionReason, TRANSPORT_AVAILABLE};
use serde_json::{json, Value as Json};

mod common;
use common::{contracts, modules_of, plan_in, plan_of, served_plan};

fn host(name: &str) -> HostCapabilities {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("testdata")
        .join(name);
    match detect(&SystemRoot::new(root)) {
        Ok(h) => h,
        Err(e) => panic!("detect {name}: {e}"),
    }
}

/// The selection of an agent on the guest after the flip of TRN-31: nobody
/// forced it, and AF_XDP is the fastest backend that works.
fn unforced() -> Selection {
    let s = select_with_transport(&host("virtio-guest"), None, true).expect("a selection");
    assert_eq!(s.backend, BackendKind::AfXdp);
    assert_ne!(s.reason, SelectionReason::Forced);
    s
}

/// The selection of an agent that the operator forced to AF_XDP.
fn forced() -> Selection {
    let s = select_with_transport(&host("virtio-guest"), Some(BackendKind::AfXdp), true)
        .expect("a selection");
    assert_eq!(s.backend, BackendKind::AfXdp);
    assert_eq!(s.reason, SelectionReason::Forced);
    s
}

fn prepare(plan: &Json, selection: &Selection, sides: Sides) -> Result<Prepared, Box<RunError>> {
    prepare_in(
        plan,
        selection,
        sides,
        &contracts().join("examples/modules"),
    )
}

fn prepare_in(
    plan: &Json,
    selection: &Selection,
    sides: Sides,
    modules: &Path,
) -> Result<Prepared, Box<RunError>> {
    prepare_selected(
        Plan::parse(&plan.to_string()).expect("the plan parses"),
        modules,
        sides,
        &ModulePolicy::default(),
        &KeyTap::new(),
        Some(selection.backend),
        selection.reason != SelectionReason::Forced,
    )
    .map_err(Box::new)
}

fn refusal(e: &RunError) -> &BackendError {
    match e {
        RunError::Backend(b) => b,
        other => panic!("not a backend refusal: {other}"),
    }
}

#[test]
fn the_real_transport_constant_stays_off_and_the_tests_ask_for_the_answer_after_the_flip() {
    // Nothing here flips the constant of fps-backend-afxdp.
    const { assert!(!TRANSPORT_AVAILABLE) };
    let guest = host("virtio-guest");
    let now = select_with_transport(&guest, None, TRANSPORT_AVAILABLE).unwrap();
    assert_eq!(now.backend, BackendKind::Kernel);
    assert_eq!(unforced().backend, BackendKind::AfXdp);
}

#[test]
fn a_plan_in_the_shape_of_the_datapath_runs_on_af_xdp_for_both_agents() {
    for selection in [unforced(), forced()] {
        let p = prepare(&served_plan(), &selection, Sides::Both).expect("the plan prepares");
        assert_eq!(p.backend(), BackendKind::AfXdp);
        assert_eq!(p.fallback(), None);
    }
}

#[test]
fn an_unforced_agent_runs_http_smoke_on_the_kernel_backend_and_a_forced_agent_refuses_it() {
    // Both ports of http_smoke.yaml are `local/lo`, so the datapath, which
    // wants an interface of its own for each arm, refuses the plan.
    let plan = plan_of("http_smoke.yaml");
    let p = prepare(&plan, &unforced(), Sides::Both).expect("the unforced agent runs the plan");
    assert_eq!(p.backend(), BackendKind::Kernel);
    let cause = p.fallback().expect("the cause of the fallback");
    assert!(
        cause.starts_with("backend af_xdp: ") && cause.contains("same interface lo"),
        "{cause}"
    );

    let e = prepare(&plan, &forced(), Sides::Both).expect_err("a forced agent refuses the plan");
    assert!(matches!(refusal(&e), BackendError::Plan(_)), "{e}");
    assert_eq!(
        e.to_string(),
        cause,
        "the forced refusal names the same cause"
    );
}

#[test]
fn the_enterprise_mix_runs_on_af_xdp_for_an_unforced_and_a_forced_agent() {
    // enterprise_mix_10g.yaml has two interfaces, one client pool, and one
    // server pool, which is the shape of the datapath. Its entries web_browse
    // and web_get would both listen on tcp 8080 of the server hosts, so the
    // example gives web_browse the server_port 8081 (bug #692).
    let plan = plan_of("enterprise_mix_10g.yaml");
    assert_eq!(plan["mix"][0]["params"]["server_port"], 8081);
    for selection in [unforced(), forced()] {
        let p = prepare(&plan, &selection, Sides::Both).expect("the plan prepares");
        assert_eq!(p.backend(), BackendKind::AfXdp);
        assert_eq!(p.fallback(), None);
    }
}

#[test]
fn the_enterprise_mix_runs_on_the_kernel_backend() {
    // The plan of an agent that runs on the kernel backend names that
    // backend; a plan that asks for AF_XDP is a mismatch on such an agent.
    let mut plan = plan_of("enterprise_mix_10g.yaml");
    plan["backend"] = "BACKEND_KERNEL".into();
    let guest = host("virtio-guest");
    let kernel = select_with_transport(&guest, None, false).expect("a selection");
    assert_eq!(kernel.backend, BackendKind::Kernel);
    let p = prepare(&plan, &kernel, Sides::Both).expect("the plan prepares");
    assert_eq!(p.backend(), BackendKind::Kernel);
}

#[test]
fn the_enterprise_mix_without_its_second_port_is_refused_on_every_backend() {
    let mut plan = plan_of("enterprise_mix_10g.yaml");
    plan["mix"][0].as_object_mut().unwrap().remove("params");
    for selection in [unforced(), forced()] {
        let e = prepare(&plan, &selection, Sides::Both).expect_err("the plan is refused");
        let text = e.to_string();
        assert!(
            text.contains("mix entries web_browse and web_get both listen on tcp 172.16.0.2:8080"),
            "{text}"
        );
        assert!(
            text.contains(
                "give one entry its own `server_port` in its params or its own `server_pool`"
            ),
            "{text}"
        );
        assert!(matches!(*e, RunError::Plan(_)), "{text}");
    }
}

/// One refused shape: what it changes in the plan, the sides of the agent,
/// and a word of the cause.
struct Shape {
    what: &'static str,
    change: fn(&mut Json),
    sides: Sides,
    cause: &'static str,
}

fn shapes() -> Vec<Shape> {
    vec![
        Shape {
            what: "client_sim topology",
            change: |p| p["topology"] = "TOPOLOGY_CLIENT_SIM".into(),
            sides: Sides::Both,
            cause: "TOPOLOGY_APP_SIM",
        },
        Shape {
            what: "server_sim topology",
            change: |p| p["topology"] = "TOPOLOGY_SERVER_SIM".into(),
            sides: Sides::Both,
            cause: "TOPOLOGY_APP_SIM",
        },
        Shape {
            what: "an agent that runs the client side only",
            change: |_| {},
            sides: Sides::Client,
            cause: "both arms",
        },
        Shape {
            what: "an agent that runs the server side only",
            change: |_| {},
            sides: Sides::Server,
            cause: "both arms",
        },
        Shape {
            what: "both pools on one interface",
            change: |p| p["ports"][1]["interfaceName"] = "veth-c0".into(),
            sides: Sides::Both,
            cause: "same interface veth-c0",
        },
        Shape {
            what: "a port without an interface",
            change: |p| p["ports"][1]["interfaceName"] = "".into(),
            sides: Sides::Both,
            cause: "names no interface",
        },
        Shape {
            what: "two families",
            change: |p| {
                p["hostPools"][1]["ipv4Prefix"] = "".into();
                p["hostPools"][1]["ipv6Prefix"] = "fd00:2::/64".into();
            },
            sides: Sides::Both,
            cause: "a lane carries one family",
        },
        Shape {
            what: "a pool with both prefixes",
            change: |p| p["hostPools"][0]["ipv6Prefix"] = "fd00:1::/64".into(),
            sides: Sides::Both,
            cause: "IPv4 alone",
        },
        Shape {
            what: "two VLANs",
            change: |p| p["hostPools"][0]["vlan"] = 7.into(),
            sides: Sides::Both,
            cause: "VLAN 7 and 0",
        },
        Shape {
            what: "an unknown OS profile",
            change: |p| p["hostPools"][1]["osProfile"] = "beos".into(),
            sides: Sides::Both,
            cause: "os_profile 'beos'",
        },
        Shape {
            what: "a second client pool in the mix",
            change: |p| {
                let mut more = p["hostPools"][0].clone();
                more["name"] = "more".into();
                more["ipv4Prefix"] = "17.0.0.0/24".into();
                p["hostPools"].as_array_mut().unwrap().push(more);
                p["mix"][1]["clientPool"] = "more".into();
                for (n, w) in p["workers"].as_array_mut().unwrap().iter_mut().enumerate() {
                    let hosts = json!({ "pool": "more", "offset": n * 127, "count": 127 });
                    w["hosts"].as_array_mut().unwrap().push(hosts);
                }
            },
            sides: Sides::Both,
            cause: "2 Client host pools",
        },
    ]
}

#[test]
fn an_unforced_agent_runs_each_refused_shape_on_the_kernel_backend() {
    for shape in shapes() {
        let mut plan = served_plan();
        (shape.change)(&mut plan);
        let p = prepare(&plan, &unforced(), shape.sides)
            .unwrap_or_else(|e| panic!("{}: {e}", shape.what));
        assert_eq!(p.backend(), BackendKind::Kernel, "{}", shape.what);
        let cause = p
            .fallback()
            .unwrap_or_else(|| panic!("{}: no cause", shape.what));
        assert!(cause.contains(shape.cause), "{}: {cause}", shape.what);
    }
}

#[test]
fn a_forced_agent_refuses_each_refused_shape_with_the_cause() {
    for shape in shapes() {
        let mut plan = served_plan();
        (shape.change)(&mut plan);
        let e = prepare(&plan, &forced(), shape.sides)
            .expect_err(&format!("{}: a forced agent runs the plan", shape.what));
        assert!(e.to_string().contains(shape.cause), "{}: {e}", shape.what);
        assert!(
            matches!(
                refusal(&e),
                BackendError::Unsupported(_) | BackendError::Plan(_)
            ),
            "{}: {e}",
            shape.what
        );
    }
}

#[test]
fn a_dhcp_client_pool_runs_on_af_xdp_and_the_kernel_backend_refuses_it() {
    // The datapath leases the addresses of a client pool (AGT-23, TRN-37), so
    // an unforced agent and a forced one both end on AF_XDP with no fallback,
    // and the kernel backend refuses the plan naming dhcp4.
    let mut plan = served_plan();
    plan["hostPools"][0]["ipv4Prefix"] = "".into();
    plan["hostPools"][0]["dhcp4"] = json!({});
    for selection in [unforced(), forced()] {
        let p = prepare(&plan, &selection, Sides::Both).expect("a DHCP client pool runs");
        assert_eq!((p.backend(), p.fallback()), (BackendKind::AfXdp, None));
    }
    let guest = host("virtio-guest");
    let kernel = select_with_transport(&guest, None, false).expect("a selection");
    let mut on_kernel = plan.clone();
    on_kernel["backend"] = "BACKEND_KERNEL".into();
    let e = prepare(&on_kernel, &kernel, Sides::Both).expect_err("the kernel leases nothing");
    assert!(e.to_string().contains("dhcp4"), "{e}");
}

#[test]
fn a_dhcp_server_pool_is_refused_by_the_datapath_and_the_kernel_backend() {
    let mut plan = served_plan();
    plan["hostPools"][1]["ipv4Prefix"] = "".into();
    plan["hostPools"][1]["dhcp4"] = json!({});
    let e = prepare(&plan, &forced(), Sides::Both).expect_err("a server pool does not lease");
    assert!(e.to_string().contains("dhcp4"), "{e}");
    let e = prepare(&plan, &unforced(), Sides::Both).expect_err("the kernel leases nothing");
    assert!(e.to_string().contains("dhcp4"), "{e}");
    assert!(e.to_string().contains("fell back from AF_XDP"), "{e}");
}

#[test]
fn the_kernel_refusal_of_a_dhcp_plan_carries_the_cause_of_the_fallback() {
    // A DHCP plan of a shape that the datapath refuses falls back to the
    // kernel backend, which cannot lease. The error names dhcp4 and says why
    // the plan left AF_XDP (AGT-23).
    for shape in shapes() {
        let mut plan = served_plan();
        plan["hostPools"][0]["ipv4Prefix"] = "".into();
        plan["hostPools"][0]["dhcp4"] = json!({});
        (shape.change)(&mut plan);
        let e = prepare(&plan, &unforced(), shape.sides)
            .expect_err(&format!("{}: a DHCP pool does not run", shape.what));
        let text = e.to_string();
        // The cause is what the forced agent reports for the same plan.
        let cause = prepare(&plan, &forced(), shape.sides)
            .expect_err(&format!("{}: a forced agent refuses", shape.what))
            .to_string();
        assert!(text.contains("dhcp4"), "{}: {text}", shape.what);
        assert!(
            text.contains("fell back from AF_XDP") && text.contains(&cause),
            "{}: the error carries the cause of the fallback {cause}: {text}",
            shape.what
        );
    }
}

/// What an unforced agent does with a file of `contracts/examples/tests/`
/// that does not end on AF_XDP.
enum Outcome {
    /// It runs on the kernel backend, and reports this cause of the fallback.
    FallsBack(&'static str),
    /// The unforced selection runs it on AF_XDP with no fallback, and the
    /// kernel selection refuses it with this text (a DHCP client pool).
    AfXdpAndKernelRefuses(&'static str),
}

const LO: &str = "host pools clients and servers sit on the same interface lo";

/// The files that do not end on AF_XDP, each with its cause. A file that is
/// not in this table must prepare on the kernel selection and end on AF_XDP
/// without a fallback on the unforced one.
const EXPECTED: &[(&str, Outcome, &str)] = &[
    (
        "dhcp_clients.yaml",
        Outcome::AfXdpAndKernelRefuses(
            "host pool clients asks for dhcp4, and the kernel backend leases no address",
        ),
        "the clients lease their address, which only the datapath does (AGT-23, bug #708)",
    ),
    (
        "dns_zone.yaml",
        Outcome::FallsBack(LO),
        "both ports are local/lo, so the datapath has no interface for each arm",
    ),
    (
        "filtered_criteria.yaml",
        Outcome::FallsBack(LO),
        "both ports are local/lo, so the datapath has no interface for each arm",
    ),
    (
        "http1_first_party.yaml",
        Outcome::FallsBack(LO),
        "both ports are local/lo, so the datapath has no interface for each arm",
    ),
    (
        "http_smoke.yaml",
        Outcome::FallsBack(LO),
        "both ports are local/lo, so the datapath has no interface for each arm",
    ),
    (
        "seedless_smoke.yaml",
        Outcome::FallsBack(LO),
        "both ports are local/lo, so the datapath has no interface for each arm",
    ),
    (
        "port_locations.yaml",
        Outcome::FallsBack("the mix uses 2 Client host pools"),
        "two client pools in the mix, and the datapath serves one per side",
    ),
];

/// Every file of `contracts/examples/tests/` prepares on the kernel selection
/// and on the unforced AF_XDP selection of the guest, or is in `EXPECTED`
/// with its cause. A new example is covered without a change here.
#[test]
fn every_example_test_prepares_on_the_kernel_and_the_unforced_selection() {
    let mut names: Vec<String> = std::fs::read_dir(contracts().join("examples/tests"))
        .expect("the examples")
        .map(|e| e.expect("entry").file_name().into_string().expect("name"))
        .filter(|n| n.ends_with(".yaml"))
        .collect();
    names.sort();
    for (file, _, _) in EXPECTED {
        assert!(names.iter().any(|n| n == file), "{file} is not an example");
    }
    let guest = host("virtio-guest");
    let kernel = select_with_transport(&guest, None, false).expect("a selection");
    assert_eq!(kernel.backend, BackendKind::Kernel);

    for name in &names {
        let modules = modules_of(name);
        let afxdp_plan = plan_in(name, &modules);
        let mut kernel_plan = afxdp_plan.clone();
        kernel_plan["backend"] = "BACKEND_KERNEL".into();
        let on_kernel = prepare_in(&kernel_plan, &kernel, Sides::Both, &modules);
        let on_unforced = prepare_in(&afxdp_plan, &unforced(), Sides::Both, &modules);
        let expected = EXPECTED.iter().find(|(f, _, _)| f == name).map(|e| &e.1);
        match expected {
            None => {
                let k = on_kernel.unwrap_or_else(|e| panic!("{name} on kernel: {e}"));
                assert_eq!(k.backend(), BackendKind::Kernel, "{name}");
                let u = on_unforced.unwrap_or_else(|e| panic!("{name} unforced: {e}"));
                assert_eq!(
                    (u.backend(), u.fallback()),
                    (BackendKind::AfXdp, None),
                    "{name}: a new example that falls back or is refused needs a row in EXPECTED"
                );
            }
            Some(Outcome::FallsBack(cause)) => {
                let k = on_kernel.unwrap_or_else(|e| panic!("{name} on kernel: {e}"));
                assert_eq!(k.backend(), BackendKind::Kernel, "{name}");
                let u = on_unforced.unwrap_or_else(|e| panic!("{name} unforced: {e}"));
                assert_eq!(u.backend(), BackendKind::Kernel, "{name}");
                let got = u.fallback().unwrap_or_else(|| panic!("{name}: no cause"));
                assert!(
                    got.starts_with("backend af_xdp: ") && got.contains(cause),
                    "{name}: {got}"
                );
            }
            Some(Outcome::AfXdpAndKernelRefuses(text)) => {
                let e = on_kernel
                    .err()
                    .unwrap_or_else(|| panic!("{name} kernel runs"));
                assert!(e.to_string().contains(text), "{name} kernel: {e}");
                let u = on_unforced.unwrap_or_else(|e| panic!("{name} unforced: {e}"));
                assert_eq!(
                    (u.backend(), u.fallback()),
                    (BackendKind::AfXdp, None),
                    "{name}"
                );
            }
        }
    }
}
