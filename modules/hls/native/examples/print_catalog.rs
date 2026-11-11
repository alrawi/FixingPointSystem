// SPDX-License-Identifier: GPL-3.0-only
//! Prints the template function catalog of the module in the syntax of the
//! `functions` list of `modules/hls/manifest.yaml`.
//!
//! ```text
//! cargo run -p fps-module-hls --example print_catalog
//! ```

fn main() {
    print!("{}", fps_module_hls::catalog::manifest_functions());
}
