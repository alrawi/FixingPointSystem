// SPDX-License-Identifier: GPL-3.0-only
//! The parts of a module manifest that the scenario loader needs.
//!
//! The loader checks module calls against the `functions` catalog of the
//! manifest, as `tools/contracts/validate.py` does. It reads only the name,
//! the type, `requires`, `functions`, and `scenarios`, and it leaves the rest
//! of the manifest to the schema validation of the controller.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde_yaml::Value as Yaml;

use crate::function::ValueType;

/// Errors that reading a manifest reports.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ManifestError {
    /// The manifest file could not be read.
    #[error("reading {path}")]
    Read {
        /// The manifest file.
        path: PathBuf,
        /// The underlying error.
        #[source]
        source: std::io::Error,
    },
    /// The manifest is not valid YAML or lacks a field the loader needs.
    #[error("{}{reason}", .path.as_ref().map(|p| format!("{}: ", p.display())).unwrap_or_default())]
    Invalid {
        /// The manifest file, when it was read from one.
        path: Option<PathBuf>,
        /// What is wrong.
        reason: String,
    },
}

fn invalid(reason: impl Into<String>) -> ManifestError {
    ManifestError::Invalid {
        path: None,
        reason: reason.into(),
    }
}

/// One parameter of a module template function.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Param {
    /// Parameter name.
    pub name: String,
    /// Declared type.
    pub ty: ValueType,
    /// False when the manifest gives a default.
    pub required: bool,
}

/// The signature of one module template function. Every module call renders
/// bytes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FunctionSig {
    /// Function name without the module prefix.
    pub name: String,
    /// Parameters in declaration order.
    pub params: Vec<Param>,
}

/// The template functions that one manifest declares, by name.
pub type Catalog = HashMap<String, FunctionSig>;

/// The manifest fields that the loader reads.
#[derive(Clone, Debug, PartialEq)]
pub struct Manifest {
    /// Module name.
    pub name: String,
    /// Module type, such as `app`, `strike`, or `fuzzer`.
    pub module_type: String,
    /// Names of the required modules.
    pub requires: Vec<String>,
    /// The template function catalog.
    pub functions: Catalog,
    /// Scenario files relative to the module directory.
    pub scenarios: Vec<PathBuf>,
}

fn parse_type(s: &str) -> Option<ValueType> {
    Some(match s {
        "int" => ValueType::Int,
        "float" => ValueType::Float,
        "boolean" => ValueType::Boolean,
        "text" => ValueType::Text,
        "bytes" => ValueType::Bytes,
        "any" => ValueType::Any,
        _ => return None,
    })
}

fn str_field<'y>(v: &'y Yaml, key: &str) -> Result<&'y str, ManifestError> {
    v.get(key)
        .and_then(Yaml::as_str)
        .ok_or_else(|| invalid(format!("missing '{key}'")))
}

impl Manifest {
    /// Reads `manifest.yaml` at `path`.
    pub fn load(path: &Path) -> Result<Self, ManifestError> {
        let text = std::fs::read_to_string(path).map_err(|source| ManifestError::Read {
            path: path.to_path_buf(),
            source,
        })?;
        Self::parse(&text).map_err(|e| match e {
            ManifestError::Invalid { path: None, reason } => ManifestError::Invalid {
                path: Some(path.to_path_buf()),
                reason,
            },
            other => other,
        })
    }

    /// Parses manifest text.
    pub fn parse(text: &str) -> Result<Self, ManifestError> {
        let doc: Yaml =
            serde_yaml::from_str(text).map_err(|e| invalid(format!("invalid YAML: {e}")))?;
        let name = str_field(&doc, "name")?.to_string();
        let module_type = str_field(&doc, "type")?.to_string();
        let mut requires = Vec::new();
        for r in doc
            .get("requires")
            .and_then(Yaml::as_sequence)
            .map(Vec::as_slice)
            .unwrap_or_default()
        {
            requires.push(str_field(r, "module")?.to_string());
        }
        let mut functions = Catalog::new();
        for f in doc
            .get("functions")
            .and_then(Yaml::as_sequence)
            .map(Vec::as_slice)
            .unwrap_or_default()
        {
            let fname = str_field(f, "name")?.to_string();
            let mut params = Vec::new();
            for p in f
                .get("params")
                .and_then(Yaml::as_sequence)
                .map(Vec::as_slice)
                .unwrap_or_default()
            {
                let ty = str_field(p, "type")?;
                params.push(Param {
                    name: str_field(p, "name")?.to_string(),
                    ty: parse_type(ty).ok_or_else(|| {
                        invalid(format!("function '{fname}' has unknown type '{ty}'"))
                    })?,
                    required: p.get("default").is_none(),
                });
            }
            functions.insert(
                fname.clone(),
                FunctionSig {
                    name: fname,
                    params,
                },
            );
        }
        let scenarios = doc
            .get("scenarios")
            .and_then(Yaml::as_sequence)
            .map(Vec::as_slice)
            .unwrap_or_default()
            .iter()
            .filter_map(Yaml::as_str)
            .map(PathBuf::from)
            .collect();
        Ok(Manifest {
            name,
            module_type,
            requires,
            functions,
            scenarios,
        })
    }
}

/// The manifests of every installed module, by name.
#[derive(Clone, Debug, Default)]
pub struct ModuleSet {
    /// Manifests by module name.
    pub manifests: HashMap<String, Manifest>,
}

impl ModuleSet {
    /// Reads `manifest.yaml` from every directory under `root` that has one.
    pub fn load_dir(root: &Path) -> Result<Self, ManifestError> {
        let mut set = ModuleSet::default();
        let entries = std::fs::read_dir(root).map_err(|source| ManifestError::Read {
            path: root.to_path_buf(),
            source,
        })?;
        for entry in entries {
            let entry = entry.map_err(|source| ManifestError::Read {
                path: root.to_path_buf(),
                source,
            })?;
            let path = entry.path().join("manifest.yaml");
            if path.is_file() {
                set.insert(Manifest::load(&path)?);
            }
        }
        Ok(set)
    }

    /// Adds a manifest, replacing one with the same name.
    pub fn insert(&mut self, m: Manifest) {
        self.manifests.insert(m.name.clone(), m);
    }

    /// Returns the call scope of a scenario that module `name` ships.
    pub fn context(&self, name: &str) -> Option<ModuleContext> {
        self.manifests
            .get(name)
            .map(|m| ModuleContext::new(m, self))
    }
}

/// The modules that the scenarios of one module may name, with their
/// function catalogs. It mirrors the `callable_modules` of the Python
/// validator.
#[derive(Clone, Debug, PartialEq)]
pub struct ModuleContext {
    /// The module that ships the scenario.
    pub module: String,
    /// Its module type.
    pub module_type: String,
    /// The module itself and every required module, each with its catalog,
    /// or `None` when its manifest is not available.
    pub callable: HashMap<String, Option<Catalog>>,
}

impl ModuleContext {
    /// Builds the context of `module`, finding required modules in `set`.
    pub fn new(module: &Manifest, set: &ModuleSet) -> Self {
        let mut callable = HashMap::new();
        for r in &module.requires {
            callable.insert(r.clone(), set.manifests.get(r).map(|m| m.functions.clone()));
        }
        callable.insert(module.name.clone(), Some(module.functions.clone()));
        ModuleContext {
            module: module.name.clone(),
            module_type: module.module_type.clone(),
            callable,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_function_catalog() {
        let m = Manifest::parse(
            "name: a\ntype: app\nrequires: [ { module: b } ]\nfunctions:\n  - name: f\n    params:\n      - { name: x, type: int }\n      - { name: y, type: text, default: z }\n",
        )
        .expect("manifest");
        let f = &m.functions["f"];
        assert_eq!(f.params[0].ty, ValueType::Int);
        assert!(f.params[0].required);
        assert!(!f.params[1].required);
        let mut set = ModuleSet::default();
        set.insert(m);
        let ctx = set.context("a").expect("context");
        assert!(ctx.callable["a"].is_some());
        assert_eq!(ctx.callable["b"], None);
    }

    #[test]
    fn rejects_unknown_parameter_types() {
        let bad =
            "name: a\ntype: app\nfunctions: [ { name: f, params: [ { name: x, type: list } ] } ]\n";
        assert!(matches!(
            Manifest::parse(bad),
            Err(ManifestError::Invalid { .. })
        ));
    }
}
