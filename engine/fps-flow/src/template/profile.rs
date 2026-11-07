// SPDX-License-Identifier: GPL-3.0-only
//! Behavior profiles of a scenario (scenario contract v0.3).
//!
//! A profile is a named list of entries. The reference `profiles.<name>`
//! renders each entry as `Name: value` and CRLF, in declaration order, so a
//! module function can take the text as a block of header lines.

use std::collections::HashMap;

use super::Value;

/// The profiles of a scenario by name.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Profiles {
    by_name: HashMap<String, Vec<(String, Value)>>,
}

impl Profiles {
    /// Creates an empty set.
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a profile with its entries in declaration order.
    pub fn insert(&mut self, name: impl Into<String>, entries: Vec<(String, Value)>) {
        self.by_name.insert(name.into(), entries);
    }

    /// Returns true when the scenario declares the profile.
    pub fn contains(&self, name: &str) -> bool {
        self.by_name.contains_key(name)
    }

    /// Returns true when no profile exists.
    pub fn is_empty(&self) -> bool {
        self.by_name.is_empty()
    }

    /// Renders a profile as `Name: value` lines that end in CRLF.
    pub fn render(&self, name: &str) -> Option<String> {
        let entries = self.by_name.get(name)?;
        let mut out = String::new();
        for (key, value) in entries {
            out.push_str(key);
            out.push_str(": ");
            out.push_str(&value.to_string());
            out.push_str("\r\n");
        }
        Some(out)
    }
}
