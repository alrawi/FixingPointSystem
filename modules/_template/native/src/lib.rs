// SPDX-License-Identifier: GPL-3.0-only
//! Template for the native crate of a first party module.
//!
//! Copy `modules/_template/` to `modules/<name>/`, rename the package to
//! `fps-module-<name>`, and implement the template functions and hooks that
//! `contracts/extension/README.md` defines. The crate also keeps the workspace glob
//! `modules/*/native` matching before the first real module exists.
#![forbid(unsafe_code)]
#![warn(missing_docs)]

/// Name of the module that this crate implements.
pub const MODULE_NAME: &str = "template";

#[cfg(test)]
mod tests {
    use super::MODULE_NAME;

    #[test]
    fn module_name_is_set() {
        assert_eq!(MODULE_NAME, "template");
    }
}
