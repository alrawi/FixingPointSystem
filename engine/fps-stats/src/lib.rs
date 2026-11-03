// SPDX-License-Identifier: GPL-3.0-only
//! Counters and histograms.
//!
//! Each worker records into its own statistics shard, and the agent merges
//! the shards into snapshots named after `contracts/metrics/catalog.yaml`. The
//! crate is empty until ENG-09 implements statistics collection.
#![forbid(unsafe_code)]
#![warn(missing_docs, clippy::unwrap_used, clippy::expect_used, clippy::panic)]
