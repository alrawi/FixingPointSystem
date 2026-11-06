// SPDX-License-Identifier: GPL-3.0-only
//! The module settings of `dns`, which a test configuration sets under
//! `modules.dns.settings` (decision D12 item 22).
//!
//! The manifest describes them with a JSON Schema. The only setting is
//! `zone_file`, the path of a zone file that the server answers from. A
//! relative path resolves against the directory that the caller names, which
//! is the module directory for the fixture peer. Without settings the server
//! answers from the zone that the module ships.
//!
//! The zone file uses the subset of RFC 1035 master file syntax that
//! [`Zone::parse`] documents. The file must also hold an SOA record, which
//! gives the zone its origin.

use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::wire::{Name, RData};
use crate::zone::{Zone, ZoneError};
use crate::Dns;

/// Why the settings of the module cannot be applied.
#[derive(Debug, thiserror::Error)]
pub enum SettingsError {
    /// The settings value is not a JSON object.
    #[error("dns settings must be an object")]
    NotAnObject,
    /// The settings name a key that the manifest does not describe.
    #[error("unknown dns setting {0:?}, the only setting is zone_file")]
    UnknownKey(String),
    /// `zone_file` is not a non empty string.
    #[error("dns setting zone_file must be a non empty string")]
    BadZoneFile,
    /// The zone file cannot be read.
    #[error("zone file {}: {source}", path.display())]
    Read {
        /// The path that failed.
        path: PathBuf,
        /// The operating system error.
        source: std::io::Error,
    },
    /// The zone file is malformed.
    #[error("zone file {}: {source}", path.display())]
    Parse {
        /// The path that failed.
        path: PathBuf,
        /// The parse error with its line.
        source: ZoneError,
    },
    /// The zone file has no SOA record, so the zone has no origin.
    #[error("zone file {}: the zone has no SOA record", path.display())]
    NoSoa {
        /// The path that failed.
        path: PathBuf,
    },
}

/// The validated settings of the `dns` module.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Settings {
    /// Zone file that the server answers from, as the configuration wrote
    /// it. `None` selects the zone that the module ships.
    pub zone_file: Option<PathBuf>,
}

impl Settings {
    /// Reads the settings from the JSON value that the plan carries. The
    /// check matches the schema of the manifest: an object with the optional
    /// string `zone_file` and no other key.
    pub fn from_json(value: &Value) -> Result<Self, SettingsError> {
        let object = value.as_object().ok_or(SettingsError::NotAnObject)?;
        let mut settings = Settings::default();
        for (key, v) in object {
            match key.as_str() {
                "zone_file" => match v.as_str() {
                    Some(s) if !s.is_empty() => settings.zone_file = Some(PathBuf::from(s)),
                    _ => return Err(SettingsError::BadZoneFile),
                },
                other => return Err(SettingsError::UnknownKey(other.to_string())),
            }
        }
        Ok(settings)
    }

    /// Builds the zone these settings select. A relative `zone_file` resolves
    /// against `base`.
    pub fn zone(&self, base: &Path) -> Result<Zone, SettingsError> {
        let Some(file) = &self.zone_file else {
            return Zone::bundled().map_err(|source| SettingsError::Parse {
                path: PathBuf::from("zones/example.test.zone"),
                source,
            });
        };
        let path = base.join(file);
        let text = std::fs::read_to_string(&path).map_err(|source| SettingsError::Read {
            path: path.clone(),
            source,
        })?;
        let zone = Zone::parse(&text, &Name::root()).map_err(|source| SettingsError::Parse {
            path: path.clone(),
            source,
        })?;
        if !zone
            .records()
            .iter()
            .any(|r| matches!(r.data, RData::Soa { .. }))
        {
            return Err(SettingsError::NoSoa { path });
        }
        Ok(zone)
    }
}

impl Dns {
    /// Creates the functions from the settings value of a test
    /// configuration. A relative `zone_file` resolves against `base`. An
    /// empty object selects the zone that the module ships.
    pub fn from_settings(value: &Value, base: &Path) -> Result<Self, SettingsError> {
        Settings::from_json(value)?.zone(base).map(Dns::new)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn temp_zone(name: &str, text: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("fps-dns-settings-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(name);
        std::fs::write(&path, text).unwrap();
        path
    }

    const ZONE: &str = "$ORIGIN corp.test.\n$TTL 60\n@ IN SOA ns hostmaster 1 7200 3600 1209600 60\n@ IN NS ns\nns IN A 192.0.2.7\nwww IN A 192.0.2.99\n";

    #[test]
    fn empty_settings_select_the_bundled_zone() {
        let dns = Dns::from_settings(&json!({}), Path::new(".")).unwrap();
        assert_eq!(dns.zone().origin(), Zone::bundled().unwrap().origin());
        assert_eq!(
            dns.zone().records().len(),
            Zone::bundled().unwrap().records().len()
        );
    }

    #[test]
    fn zone_file_selects_that_zone_and_resolves_relative_paths() {
        let path = temp_zone("corp.zone", ZONE);
        let base = path.parent().unwrap();
        let dns = Dns::from_settings(&json!({"zone_file": "corp.zone"}), base).unwrap();
        assert_eq!(dns.zone().origin(), &Name::parse("corp.test").unwrap());
        let abs = Dns::from_settings(&json!({"zone_file": path}), Path::new("/nowhere")).unwrap();
        assert_eq!(abs.zone().records().len(), 4);
    }

    #[test]
    fn answers_come_from_the_configured_zone_only() {
        let path = temp_zone("answers.zone", ZONE);
        let dns = Dns::from_settings(&json!({"zone_file": path}), Path::new(".")).unwrap();
        let a = Name::parse("www.corp.test").unwrap();
        let found = dns.zone().lookup(&a, 1);
        assert_eq!(found.rcode, Some(crate::Rcode::NoError));
        assert_eq!(
            found.answers.first().map(|r| r.data.clone()),
            Some(RData::A(std::net::Ipv4Addr::new(192, 0, 2, 99)))
        );
        let bundled_name = Name::parse("www.example.test").unwrap();
        assert_eq!(
            dns.zone().lookup(&bundled_name, 1).rcode,
            Some(crate::Rcode::Refused)
        );
    }

    #[test]
    fn malformed_settings_are_rejected() {
        let base = Path::new(".");
        for (v, want) in [
            (json!([]), "must be an object"),
            (json!({"zone": "x"}), "unknown dns setting"),
            (json!({"zone_file": 3}), "non empty string"),
            (json!({"zone_file": ""}), "non empty string"),
        ] {
            let e = Dns::from_settings(&v, base).unwrap_err().to_string();
            assert!(e.contains(want), "{v}: {e}");
        }
    }

    #[test]
    fn a_missing_zone_file_names_the_path() {
        let e = Dns::from_settings(
            &json!({"zone_file": "absent.zone"}),
            Path::new("/nonexistent"),
        )
        .unwrap_err()
        .to_string();
        assert!(e.contains("/nonexistent/absent.zone"), "{e}");
    }

    #[test]
    fn a_malformed_zone_names_the_file_and_line() {
        let path = temp_zone(
            "bad.zone",
            "$TTL 60\n@ IN SOA ns h 1 2 3 4 5\nwww IN A 999.0.0.1\n",
        );
        let e = Dns::from_settings(&json!({"zone_file": path}), Path::new("."))
            .unwrap_err()
            .to_string();
        assert!(e.contains("bad.zone") && e.contains("line 3"), "{e}");
    }

    #[test]
    fn a_zone_without_soa_is_rejected() {
        let path = temp_zone("nosoa.zone", "$TTL 60\nwww.corp.test. IN A 192.0.2.1\n");
        let e = Dns::from_settings(&json!({"zone_file": path}), Path::new("."))
            .unwrap_err()
            .to_string();
        assert!(e.contains("no SOA record"), "{e}");
    }
}
