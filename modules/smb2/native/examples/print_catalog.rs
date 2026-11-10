// SPDX-License-Identifier: GPL-3.0-only
//! Prints the template function catalog of the module in the syntax of the
//! `functions` list of `modules/smb2/manifest.yaml`.
//!
//! ```text
//! cargo run -p fps-module-smb2 --example print_catalog
//! ```

fn main() {
    print!("{}", fps_module_smb2::catalog::manifest_functions());
}
