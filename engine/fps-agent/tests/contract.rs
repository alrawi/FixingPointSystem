// SPDX-License-Identifier: GPL-3.0-only
//! Contract test that compares the Rust mirrors in `fps_agent::proto` with
//! `contracts/proto/fps/agent/v1/agent.proto`.
//!
//! The test reads the proto file directly, as rule 10 of the coding standards
//! asks. Each mirror is destructured without `..`, so a field added to or
//! removed from a Rust struct fails to compile until the table here changes,
//! and the table is then compared with the proto field names and numbers.

use std::path::PathBuf;

use fps_agent::proto::{Backend, Capabilities, ModuleRef, PortInfo};

fn proto_text() -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../contracts/proto/fps/agent/v1/agent.proto");
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
}

/// Returns the body of `message NAME { ... }` or `enum NAME { ... }`.
fn block<'a>(text: &'a str, kind: &str, name: &str) -> &'a str {
    let head = format!("{kind} {name} {{");
    let start = text
        .find(&head)
        .unwrap_or_else(|| panic!("{head} not in agent.proto"));
    let body = &text[start + head.len()..];
    let end = body
        .find("\n}")
        .unwrap_or_else(|| panic!("{head} is not closed"));
    &body[..end]
}

/// Returns the `(name, number)` pairs of the fields of a message, ignoring
/// comments. A field line ends with `name = number;`.
fn message_fields(text: &str, name: &str) -> Vec<(String, u32)> {
    entries(block(text, "message", name))
}

fn entries(body: &str) -> Vec<(String, u32)> {
    body.lines()
        .map(str::trim)
        .filter(|l| !l.starts_with("//") && l.ends_with(';') && l.contains(" = "))
        .map(|l| {
            let (lhs, rhs) = l
                .trim_end_matches(';')
                .rsplit_once(" = ")
                .unwrap_or_default();
            let field = lhs.split_whitespace().last().unwrap_or_default().to_owned();
            let number = rhs
                .trim()
                .parse()
                .unwrap_or_else(|_| panic!("bad number in {l:?}"));
            (field, number)
        })
        .collect()
}

fn table(rows: &[(&str, u32)]) -> Vec<(String, u32)> {
    rows.iter().map(|&(n, i)| (n.to_owned(), i)).collect()
}

#[test]
fn capabilities_mirrors_the_proto_message() {
    let Capabilities {
        agent_version: _,
        engine_api: _,
        backends: _,
        cores: _,
        memory_bytes: _,
        ports: _,
        modules: _,
        labels: _,
    } = Capabilities::default();
    let rust = table(&[
        ("agent_version", 1),
        ("engine_api", 2),
        ("backends", 3),
        ("cores", 4),
        ("memory_bytes", 5),
        ("ports", 6),
        ("modules", 7),
        ("labels", 8),
    ]);
    assert_eq!(message_fields(&proto_text(), "Capabilities"), rust);
}

#[test]
fn port_info_mirrors_the_proto_message() {
    let PortInfo {
        name: _,
        driver: _,
        speed_mbps: _,
        mac: _,
        mtu: _,
    } = PortInfo::default();
    let rust = table(&[
        ("name", 1),
        ("driver", 2),
        ("speed_mbps", 3),
        ("mac", 4),
        ("mtu", 5),
    ]);
    assert_eq!(message_fields(&proto_text(), "PortInfo"), rust);
}

#[test]
fn module_ref_mirrors_the_proto_message() {
    let ModuleRef {
        name: _,
        version: _,
        digest: _,
        signature: _,
        signature_key_id: _,
        settings: _,
    } = ModuleRef::default();
    let rust = table(&[
        ("name", 1),
        ("version", 2),
        ("digest", 3),
        ("signature", 4),
        ("signature_key_id", 5),
        ("settings", 6),
    ]);
    assert_eq!(message_fields(&proto_text(), "ModuleRef"), rust);
}

#[test]
fn backend_mirrors_the_proto_enum() {
    let all = [
        Backend::Unspecified,
        Backend::Kernel,
        Backend::AfXdp,
        Backend::Dpdk,
    ];
    for b in all {
        // An exhaustive match fails to compile when a variant is added.
        match b {
            Backend::Unspecified | Backend::Kernel | Backend::AfXdp | Backend::Dpdk => {}
        }
    }
    let rust: Vec<(String, u32)> = all
        .iter()
        .map(|b| {
            (
                b.proto_name().to_owned(),
                u32::try_from(b.number()).unwrap_or(u32::MAX),
            )
        })
        .collect();
    assert_eq!(entries(block(&proto_text(), "enum", "Backend")), rust);
}

#[test]
fn engine_api_matches_the_proto_contract_version() {
    let header = format!("contract version {}.", fps_agent::capabilities::ENGINE_API);
    assert!(
        proto_text().contains(&header),
        "agent.proto does not say {header:?}"
    );
}
