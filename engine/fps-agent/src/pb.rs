// SPDX-License-Identifier: GPL-3.0-only
//! Generated types and gRPC stubs of `fps.agent.v1`.
//!
//! `build.rs` compiles `contracts/proto/fps/agent/v1/agent.proto` with
//! protox and tonic. The session client in [`crate::session`] uses
//! [`controller_service_client`], and tests serve a fake controller with
//! [`controller_service_server`].
#![allow(missing_docs, clippy::all, clippy::pedantic)]

tonic::include_proto!("fps.agent.v1");
