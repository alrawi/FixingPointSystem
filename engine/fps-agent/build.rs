// SPDX-License-Identifier: GPL-3.0-only
//! Generates the Rust types and the gRPC stubs of
//! `contracts/proto/fps/agent/v1/agent.proto`.
//!
//! protox compiles the proto file in pure Rust, so the build needs no protoc
//! binary. The well known types map to `prost-types`.

use std::path::PathBuf;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR")?);
    let proto_root = manifest.join("../../contracts/proto");
    let file = proto_root.join("fps/agent/v1/agent.proto");
    println!("cargo:rerun-if-changed={}", file.display());
    let fds = protox::compile([&file], [&proto_root])?;
    tonic_prost_build::configure()
        .build_client(true)
        .build_server(true)
        .compile_fds(fds)?;
    Ok(())
}
