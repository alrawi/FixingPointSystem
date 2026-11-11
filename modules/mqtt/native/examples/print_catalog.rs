// SPDX-License-Identifier: GPL-3.0-only
//! Prints the template function catalog of the module in the syntax of the
//! `functions` list of `modules/mqtt/manifest.yaml`.
//!
//! ```text
//! cargo run -p fps-module-mqtt --example print_catalog
//! ```

fn main() {
    print!("{}", fps_module_mqtt::catalog::manifest_functions());
}
