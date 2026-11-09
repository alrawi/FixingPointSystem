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

/// Encodes and decodes a message and checks that nothing changed.
fn round_trip<M: prost::Message + Default + PartialEq + std::fmt::Debug>(m: &M) {
    let bytes = m.encode_to_vec();
    let back = M::decode(bytes.as_slice()).expect("decode");
    assert_eq!(&back, m);
}

#[test]
fn capture_download_messages_match_the_proto_numbers() {
    let text = proto_text();
    let rows: [(&str, &[(&str, u32)]); 8] = [
        (
            "FetchCaptureCommand",
            &[
                ("plan_id", 1),
                ("capture_id", 2),
                ("start_offset", 3),
                ("transfer_id", 4),
                ("max_chunk_bytes", 5),
            ],
        ),
        (
            "FetchCaptureResult",
            &[
                ("capture_id", 1),
                ("transfer_id", 2),
                ("total_bytes", 3),
                ("digest", 4),
                ("chunk_bytes", 5),
            ],
        ),
        (
            "UploadCaptureRequest",
            &[("open", 1), ("chunk", 2), ("end", 3)],
        ),
        (
            "UploadCaptureOpen",
            &[
                ("transfer_id", 1),
                ("session_token", 2),
                ("capture_id", 3),
                ("start_offset", 4),
            ],
        ),
        ("CaptureChunk", &[("offset", 1), ("data", 2)]),
        ("CaptureEnd", &[("total_bytes", 1), ("digest", 2)]),
        (
            "UploadCaptureResponse",
            &[("received_offset", 1), ("verified", 2)],
        ),
        ("SessionResponse", &[("fetch_capture", 9)]),
    ];
    for (name, expected) in rows {
        let fields = message_fields(&text, name);
        for (field, number) in expected {
            assert!(
                fields.contains(&((*field).to_owned(), *number)),
                "{name}.{field} = {number} is not in agent.proto"
            );
        }
    }
    let result = message_fields(&text, "CommandResult");
    assert!(result.contains(&("fetch_capture".to_owned(), 9)));
}

#[test]
fn capture_download_messages_round_trip() {
    use fps_agent::pb;
    let digest = format!("sha256:{}", "ab".repeat(32));
    round_trip(&pb::FetchCaptureCommand {
        plan_id: "run-0001/a".into(),
        capture_id: "cap-1".into(),
        start_offset: 262_144,
        transfer_id: "t-0001".into(),
        max_chunk_bytes: 65_536,
    });
    round_trip(&pb::FetchCaptureResult {
        capture_id: "cap-1".into(),
        transfer_id: "t-0001".into(),
        total_bytes: 1_000_000,
        digest: digest.clone(),
        chunk_bytes: 65_536,
    });
    let open = pb::UploadCaptureRequest {
        body: Some(pb::upload_capture_request::Body::Open(
            pb::UploadCaptureOpen {
                transfer_id: "t-0001".into(),
                session_token: "token-1".into(),
                capture_id: "cap-1".into(),
                start_offset: 262_144,
            },
        )),
    };
    let chunk = pb::UploadCaptureRequest {
        body: Some(pb::upload_capture_request::Body::Chunk(pb::CaptureChunk {
            offset: 262_144,
            data: vec![0xa5; 262_144],
        })),
    };
    let end = pb::UploadCaptureRequest {
        body: Some(pb::upload_capture_request::Body::End(pb::CaptureEnd {
            total_bytes: 1_000_000,
            digest,
        })),
    };
    for m in [&open, &chunk, &end] {
        round_trip(m);
    }
    round_trip(&pb::UploadCaptureResponse {
        received_offset: 1_000_000,
        verified: true,
    });
    // The command and its result ride the Session envelopes in arm 9.
    round_trip(&pb::SessionResponse {
        command_id: 7,
        body: Some(pb::session_response::Body::FetchCapture(
            pb::FetchCaptureCommand {
                plan_id: "p".into(),
                capture_id: "c".into(),
                transfer_id: "t".into(),
                ..Default::default()
            },
        )),
    });
    round_trip(&pb::CommandResult {
        command_id: 7,
        error: None,
        result: Some(pb::command_result::Result::FetchCapture(
            pb::FetchCaptureResult {
                capture_id: "c".into(),
                transfer_id: "t".into(),
                total_bytes: 1,
                digest: format!("sha256:{}", "00".repeat(32)),
                chunk_bytes: 262_144,
            },
        )),
    });
}

#[test]
fn capture_start_and_criterion_fields_match_the_proto_numbers() {
    let text = proto_text();
    let start = table(&[
        ("plan_id", 1),
        ("capture_id", 2),
        ("port", 3),
        ("filter", 4),
        ("snap_length", 5),
        ("max_bytes", 6),
        ("max_duration", 7),
        ("export_tls_keys", 8),
    ]);
    assert_eq!(message_fields(&text, "StartCaptureCommand"), start);
    let criterion = table(&[("metric", 1), ("op", 2), ("value", 3), ("labels", 4)]);
    assert_eq!(message_fields(&text, "Criterion"), criterion);
}

#[test]
fn export_tls_keys_and_criterion_labels_round_trip() {
    use fps_agent::pb;
    use prost::Message as _;
    let start = pb::StartCaptureCommand {
        plan_id: "p".into(),
        capture_id: "c".into(),
        port: "client0".into(),
        export_tls_keys: true,
        ..Default::default()
    };
    round_trip(&start);
    let back = pb::StartCaptureCommand::decode(start.encode_to_vec().as_slice()).unwrap();
    assert!(back.export_tls_keys);
    // A sender that leaves the field out, as an agent of 0.2.2 does, reads as false.
    let without = pb::StartCaptureCommand {
        export_tls_keys: false,
        ..start.clone()
    };
    let back = pb::StartCaptureCommand::decode(without.encode_to_vec().as_slice()).unwrap();
    assert!(!back.export_tls_keys);

    let crit = pb::Criterion {
        metric: "sessions.failed_ratio".into(),
        op: 5,
        value: 0.0,
        labels: [("side".to_owned(), "server".to_owned())].into(),
    };
    round_trip(&crit);
    let plan = pb::Plan {
        criteria: vec![crit, pb::Criterion::default()],
        ..Default::default()
    };
    round_trip(&plan);
    let back = pb::Plan::decode(plan.encode_to_vec().as_slice()).unwrap();
    assert_eq!(back.criteria[0].labels["side"], "server");
    assert!(back.criteria[1].labels.is_empty());
}

#[test]
fn engine_api_matches_the_proto_contract_version() {
    let header = format!("contract version {}.", fps_agent::capabilities::ENGINE_API);
    assert!(
        proto_text().contains(&header),
        "agent.proto does not say {header:?}"
    );
}
