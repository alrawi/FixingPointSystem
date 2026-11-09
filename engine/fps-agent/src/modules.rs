// SPDX-License-Identifier: GPL-3.0-only
//! Module loading with digest checks, and the native template functions.
//!
//! The standalone mode reads each module of a plan from `<dir>/<name>`. It
//! checks the version of `manifest.yaml` and the tree digest that
//! `contracts/README.md` defines, then compiles every scenario source of the
//! plan after checking its digest, in the call scope of its module.
//!
//! First party modules implement their template functions in Rust (decision
//! D10). [`NativeFunctions`] dispatches a module call to the crate of the
//! module. A module gets its native functions only when the function catalog
//! of its manifest equals the catalog of the crate in names, parameter order,
//! types, and required parameters, because the manifest is the catalog that
//! the loader checks calls against.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use fps_flow::function::NoFunctions;
use fps_flow::manifest::Catalog;
use fps_flow::{FnCall, FnError, Manifest, ModuleSet, Scenario, TemplateFn, Value, ValueType};
use rand::RngCore;
use serde_json::Value as Json;
use sha2::{Digest, Sha256};

use crate::plan::{ModuleRef, Plan, ScenarioSource};

/// Errors of module loading.
#[derive(Debug, thiserror::Error)]
pub enum ModuleError {
    /// A file of a module tree could not be read.
    #[error("module {module}: read {path}")]
    Read {
        /// The module name.
        module: String,
        /// The file or directory.
        path: PathBuf,
        /// The cause.
        #[source]
        source: std::io::Error,
    },
    /// The module tree holds a symbolic link or a path that is not UTF-8.
    #[error("module {module}: {path} is {what}, which a module tree may not hold")]
    Forbidden {
        /// The module name.
        module: String,
        /// The offending path.
        path: PathBuf,
        /// What it is.
        what: &'static str,
    },
    /// The module directory has no readable manifest.
    #[error("module {module}: manifest")]
    Manifest {
        /// The module name.
        module: String,
        /// The cause.
        #[source]
        source: fps_flow::ManifestError,
    },
    /// The manifest names another module or version than the plan.
    #[error("module {module}: manifest has {field} {got:?}, the plan wants {want:?}")]
    Mismatch {
        /// The module name.
        module: String,
        /// The field, `name` or `version`.
        field: &'static str,
        /// The manifest value.
        got: String,
        /// The plan value.
        want: String,
    },
    /// The tree digest differs from the plan.
    #[error("module {module}: digest {actual} differs from the plan digest {want}")]
    Digest {
        /// The module name.
        module: String,
        /// The digest of the tree on disk.
        actual: String,
        /// The plan digest.
        want: String,
    },
    /// A scenario source digest differs from the plan.
    #[error("scenario {scenario}: digest {actual} differs from the plan digest {want}")]
    ScenarioDigest {
        /// The scenario ref.
        scenario: String,
        /// The digest of the source.
        actual: String,
        /// The plan digest.
        want: String,
    },
    /// A scenario source does not compile.
    #[error("scenario {scenario}: {source}")]
    Scenario {
        /// The scenario ref.
        scenario: String,
        /// The cause.
        #[source]
        source: fps_flow::LoadError,
    },
    /// A scenario defines another name than its ref.
    #[error("scenario {scenario}: the source defines scenario '{name}'")]
    ScenarioName {
        /// The scenario ref.
        scenario: String,
        /// The name in the source.
        name: String,
    },
    /// The settings of a module cannot be applied.
    #[error("module {module}: settings: {reason}")]
    Settings {
        /// The module name.
        module: String,
        /// What is wrong.
        reason: String,
    },
    /// A mix parameter cannot become a scenario variable.
    #[error("mix entry {entry}: parameter {name}: {reason}")]
    Param {
        /// The mix entry.
        entry: String,
        /// The parameter name.
        name: String,
        /// What is wrong.
        reason: String,
    },
}

/// Returns `sha256:<hex>` of `data`.
pub fn sha256_digest(data: &[u8]) -> String {
    format!("sha256:{}", hex(&Sha256::digest(data)))
}

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        // Writing to a String cannot fail.
        let _ = write!(s, "{b:02x}");
    }
    s
}

/// Returns the tree digest of the module directory `dir`.
///
/// Every regular file contributes the line `<sha256 hex>  <relative POSIX
/// path>\n`, the lines sort by the UTF-8 bytes of the path, and the digest is
/// the SHA-256 of their concatenation. A symbolic link fails.
pub fn tree_digest(module: &str, dir: &Path) -> Result<String, ModuleError> {
    let mut lines: Vec<(String, String)> = Vec::new();
    walk(module, dir, dir, &mut lines)?;
    lines.sort_by(|a, b| a.0.as_bytes().cmp(b.0.as_bytes()));
    let mut all = String::new();
    for (_, line) in &lines {
        all.push_str(line);
    }
    Ok(sha256_digest(all.as_bytes()))
}

fn walk(
    module: &str,
    root: &Path,
    dir: &Path,
    out: &mut Vec<(String, String)>,
) -> Result<(), ModuleError> {
    let read = |path: &Path, source| ModuleError::Read {
        module: module.to_owned(),
        path: path.to_path_buf(),
        source,
    };
    for entry in fs::read_dir(dir).map_err(|e| read(dir, e))? {
        let entry = entry.map_err(|e| read(dir, e))?;
        let path = entry.path();
        let kind = entry.file_type().map_err(|e| read(&path, e))?;
        if kind.is_symlink() {
            return Err(ModuleError::Forbidden {
                module: module.to_owned(),
                path,
                what: "a symbolic link",
            });
        }
        if kind.is_dir() {
            walk(module, root, &path, out)?;
        } else if kind.is_file() {
            let rel = path.strip_prefix(root).unwrap_or(&path);
            let parts: Option<Vec<&str>> =
                rel.components().map(|c| c.as_os_str().to_str()).collect();
            let Some(parts) = parts else {
                return Err(ModuleError::Forbidden {
                    module: module.to_owned(),
                    path,
                    what: "a path that is not UTF-8",
                });
            };
            let rel = parts.join("/");
            let data = fs::read(&path).map_err(|e| read(&path, e))?;
            let line = format!("{}  {rel}\n", hex(&Sha256::digest(&data)));
            out.push((rel, line));
        }
    }
    Ok(())
}

/// The modules and compiled scenarios of one plan.
#[derive(Debug)]
pub struct LoadedModules {
    /// The manifests of the plan modules.
    pub set: ModuleSet,
    /// The compiled scenario of every scenario ref, before mix parameters.
    pub scenarios: HashMap<String, Scenario>,
    /// The native template functions of the session.
    pub functions: NativeFunctions,
    /// Notes for the operator, such as a module without native functions.
    pub notes: Vec<String>,
}

/// Loads, checks, and compiles every module and scenario of `plan` from
/// `dir`.
pub fn load(plan: &Plan, dir: &Path) -> Result<LoadedModules, ModuleError> {
    let mut set = ModuleSet::default();
    for m in &plan.modules {
        set.insert(load_module(m, dir)?);
    }
    let mut notes = Vec::new();
    let functions = NativeFunctions::for_modules(&set, &plan.modules, dir, &mut notes)?;
    let mut scenarios = HashMap::new();
    for s in &plan.scenarios {
        scenarios.insert(s.r#ref.clone(), compile(s, &set)?);
    }
    Ok(LoadedModules {
        set,
        scenarios,
        functions,
        notes,
    })
}

fn load_module(m: &ModuleRef, dir: &Path) -> Result<Manifest, ModuleError> {
    let root = dir.join(&m.name);
    let manifest =
        Manifest::load(&root.join("manifest.yaml")).map_err(|source| ModuleError::Manifest {
            module: m.name.clone(),
            source,
        })?;
    if manifest.name != m.name {
        return Err(ModuleError::Mismatch {
            module: m.name.clone(),
            field: "name",
            got: manifest.name,
            want: m.name.clone(),
        });
    }
    let version = manifest_version(&root.join("manifest.yaml")).unwrap_or_default();
    if version != m.version {
        return Err(ModuleError::Mismatch {
            module: m.name.clone(),
            field: "version",
            got: version,
            want: m.version.clone(),
        });
    }
    let actual = tree_digest(&m.name, &root)?;
    if actual != m.digest {
        return Err(ModuleError::Digest {
            module: m.name.clone(),
            actual,
            want: m.digest.clone(),
        });
    }
    Ok(manifest)
}

/// Lists the modules installed under `dir`, one per subdirectory that holds a
/// `manifest.yaml`, with the name and version of the manifest and the tree
/// digest, sorted by name. The agent reports them in `Capabilities.modules`.
/// A subdirectory whose manifest or tree cannot be read is left out.
pub fn installed(dir: &Path) -> Vec<ModuleRef> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for entry in entries.flatten() {
        let root = entry.path();
        if !entry.file_type().is_ok_and(|t| t.is_dir()) {
            continue;
        }
        let path = root.join("manifest.yaml");
        let Ok(manifest) = Manifest::load(&path) else {
            continue;
        };
        let Ok(digest) = tree_digest(&manifest.name, &root) else {
            continue;
        };
        out.push(ModuleRef {
            version: manifest_version(&path).unwrap_or_default(),
            name: manifest.name,
            digest,
            settings: serde_json::Map::new(),
        });
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

/// Reads the top level `version` of a manifest, which the `fps-flow`
/// manifest reader leaves out.
fn manifest_version(path: &Path) -> Option<String> {
    let text = fs::read_to_string(path).ok()?;
    text.lines().find_map(|l| {
        let v = l.strip_prefix("version:")?.trim();
        Some(v.trim_matches(|c| c == '"' || c == '\'').to_owned())
    })
}

fn compile(s: &ScenarioSource, set: &ModuleSet) -> Result<Scenario, ModuleError> {
    let actual = sha256_digest(s.source.as_bytes());
    if actual != s.digest {
        return Err(ModuleError::ScenarioDigest {
            scenario: s.r#ref.clone(),
            actual,
            want: s.digest.clone(),
        });
    }
    let ctx = set.context(&s.module);
    let scenario =
        Scenario::parse_with(&s.source, ctx.as_ref()).map_err(|source| ModuleError::Scenario {
            scenario: s.r#ref.clone(),
            source,
        })?;
    let want = s.r#ref.split_once('/').map_or("", |(_, n)| n);
    if scenario.name != want {
        return Err(ModuleError::ScenarioName {
            scenario: s.r#ref.clone(),
            name: scenario.name,
        });
    }
    Ok(scenario)
}

/// Applies the mix parameters of `entry` to `scenario` and folds constants.
pub fn with_params(
    entry: &str,
    scenario: &Scenario,
    params: &std::collections::BTreeMap<String, Json>,
) -> Result<Scenario, ModuleError> {
    let mut s = scenario.clone();
    let mut overrides = Vec::with_capacity(params.len());
    for (name, v) in params {
        let bad = |reason: &str| ModuleError::Param {
            entry: entry.to_owned(),
            name: name.clone(),
            reason: reason.to_owned(),
        };
        let value = match v {
            Json::Bool(b) => Value::Bool(*b),
            Json::String(t) => Value::Str(t.clone()),
            Json::Number(n) => match (n.as_i64(), n.as_f64()) {
                (Some(i), _) => Value::Int(i),
                // The interpreter treats an integral number as an integer.
                (None, Some(f)) if f.fract() == 0.0 && f.abs() < 9.0e15 =>
                {
                    #[allow(clippy::cast_possible_truncation)]
                    Value::Int(f as i64)
                }
                (None, Some(f)) => Value::Float(f),
                (None, None) => return Err(bad("number out of range")),
            },
            Json::Null | Json::Array(_) | Json::Object(_) => {
                return Err(bad("only numbers, strings, and booleans bind variables"))
            }
        };
        overrides.push((name.clone(), value));
    }
    let fail = |source| ModuleError::Scenario {
        scenario: format!("{} in mix entry {entry}", scenario.name),
        source,
    };
    s.override_vars(&overrides).map_err(fail)?;
    s.fold_constants().map_err(fail)?;
    Ok(s)
}

/// The native template functions of one session, by module.
#[derive(Clone, Debug, Default)]
pub struct NativeFunctions {
    http1: Option<fps_module_http1::Http1>,
    dns: Option<fps_module_dns::Dns>,
}

impl NativeFunctions {
    /// Registers the native functions of every module in `set` whose
    /// manifest catalog equals the catalog of its crate, configured with the
    /// settings of the matching entry of `refs`. A relative path setting
    /// resolves against the directory of the module under `dir` and may not
    /// leave it. Each module that stays without native functions adds a
    /// note.
    ///
    /// Settings that the agent cannot apply are an error, because a run that
    /// ignores them would measure another test than the one configured. That
    /// covers a module without native functions and a module whose native
    /// functions take no settings.
    pub fn for_modules(
        set: &ModuleSet,
        refs: &[ModuleRef],
        dir: &Path,
        notes: &mut Vec<String>,
    ) -> Result<Self, ModuleError> {
        let mut out = Self::default();
        let settings = |name: &str| refs.iter().find(|m| m.name == name).map(|m| &m.settings);
        for (name, manifest) in &set.manifests {
            let given = settings(name).filter(|s| !s.is_empty());
            match name.as_str() {
                fps_module_http1::MODULE_NAME => {
                    let native: Vec<NativeSig> = fps_module_http1::FUNCTIONS
                        .iter()
                        .map(|f| {
                            (
                                f.name,
                                f.params
                                    .iter()
                                    .map(|p| (p.name, p.ty, p.default.is_none()))
                                    .collect(),
                            )
                        })
                        .collect();
                    match catalog_diff(&manifest.functions, &native) {
                        None => {
                            refuse_settings(name, given, "its native functions take none")?;
                            out.http1 = Some(fps_module_http1::Http1);
                        }
                        Some(d) => {
                            refuse_settings(name, given, "it has no native functions here")?;
                            notes.push(no_native(name, &d));
                        }
                    }
                }
                fps_module_dns::MODULE_NAME => {
                    let native: Vec<NativeSig> = fps_module_dns::FUNCTIONS
                        .iter()
                        .map(|f| {
                            (
                                f.name,
                                f.params
                                    .iter()
                                    .map(|p| (p.name, p.ty, p.default.is_none()))
                                    .collect(),
                            )
                        })
                        .collect();
                    match catalog_diff(&manifest.functions, &native) {
                        None => match dns_functions(dir, given) {
                            Ok(d) => out.dns = Some(d),
                            Err(DnsLoad::Settings(reason)) => {
                                return Err(ModuleError::Settings {
                                    module: name.clone(),
                                    reason,
                                })
                            }
                            Err(DnsLoad::Bundled(e)) => notes.push(format!(
                                "module dns has no native functions: bundled zone: {e}"
                            )),
                        },
                        Some(d) => {
                            refuse_settings(name, given, "it has no native functions here")?;
                            notes.push(no_native(name, &d));
                        }
                    }
                }
                _ => {
                    refuse_settings(name, given, "it has no native functions here")?;
                    if !manifest.functions.is_empty() {
                        notes.push(format!(
                            "module {name} has no native functions in this agent, so its template calls fail"
                        ));
                    }
                }
            }
        }
        notes.sort();
        Ok(out)
    }

    /// Returns the modules that have native functions.
    pub fn modules(&self) -> Vec<&'static str> {
        let mut v = Vec::new();
        if self.http1.is_some() {
            v.push(fps_module_http1::MODULE_NAME);
        }
        if self.dns.is_some() {
            v.push(fps_module_dns::MODULE_NAME);
        }
        v
    }

    /// Returns a factory that gives every session its own value.
    pub fn factory(&self) -> fps_runtime::FunctionFactory {
        let me = self.clone();
        Arc::new(move || Box::new(me.clone()))
    }
}

impl TemplateFn for NativeFunctions {
    fn render(
        &mut self,
        call: &FnCall<'_>,
        rng: &mut dyn RngCore,
        out: &mut Vec<u8>,
    ) -> Result<(), FnError> {
        match (call.module, &mut self.http1, &mut self.dns) {
            (fps_module_http1::MODULE_NAME, Some(h), _) => h.render(call, rng, out),
            (fps_module_dns::MODULE_NAME, _, Some(d)) => d.render(call, rng, out),
            _ => NoFunctions.render(call, rng, out),
        }
    }
}

/// Fails when a module that cannot apply settings got some.
fn refuse_settings(
    module: &str,
    given: Option<&serde_json::Map<String, Json>>,
    why: &str,
) -> Result<(), ModuleError> {
    match given {
        None => Ok(()),
        Some(_) => Err(ModuleError::Settings {
            module: module.to_owned(),
            reason: format!("the plan gives settings, but {why}"),
        }),
    }
}

/// Why the dns functions could not be built.
enum DnsLoad {
    /// The settings are invalid.
    Settings(String),
    /// The shipped zone does not parse.
    Bundled(fps_module_dns::ZoneError),
}

/// Builds the dns functions from the plan settings, or from the shipped zone
/// when the plan gives none. The module directory of dns is `dir/dns`.
fn dns_functions(
    dir: &Path,
    given: Option<&serde_json::Map<String, Json>>,
) -> Result<fps_module_dns::Dns, DnsLoad> {
    let Some(given) = given else {
        return fps_module_dns::Dns::bundled().map_err(DnsLoad::Bundled);
    };
    let root = dir.join(fps_module_dns::MODULE_NAME);
    let mut value = given.clone();
    // The module resolves a relative zone_file against its base, which would
    // let `..` or an absolute path leave the module directory, so the agent
    // resolves and checks the path first and hands over the result.
    if let Some(Json::String(file)) = value.get("zone_file") {
        if !file.is_empty() {
            let path =
                confined(&root, file).map_err(|e| DnsLoad::Settings(format!("zone_file: {e}")))?;
            let text = path
                .to_str()
                .ok_or_else(|| DnsLoad::Settings("zone_file: the path is not UTF-8".to_owned()))?
                .to_owned();
            value.insert("zone_file".to_owned(), Json::String(text));
        }
    }
    fps_module_dns::Dns::from_settings(&Json::Object(value), &root)
        .map_err(|e| DnsLoad::Settings(e.to_string()))
}

/// Resolves the setting path `file` against the module directory `root` and
/// returns the canonical path of an existing file inside `root`.
///
/// A path with a `..` component is refused without touching the disk. An
/// absolute path is accepted only when its canonical form lies inside `root`,
/// which also catches a symbolic link that leaves `root`, although a
/// module tree holds none after the digest check.
pub fn confined(root: &Path, file: &str) -> Result<PathBuf, String> {
    use std::path::Component;
    let given = Path::new(file);
    if given
        .components()
        .any(|c| matches!(c, Component::ParentDir))
    {
        return Err(format!(
            "{file:?} has a .. component, which could leave the module directory"
        ));
    }
    let outside = || format!("{file:?} lies outside the module directory");
    let base = root
        .canonicalize()
        .map_err(|e| format!("module directory {}: {e}", root.display()))?;
    let path = if given.is_absolute() {
        given.to_path_buf()
    } else {
        base.join(given)
    };
    let real = path.canonicalize().map_err(|e| format!("{file:?}: {e}"))?;
    if real.starts_with(&base) {
        Ok(real)
    } else {
        Err(outside())
    }
}

/// A function name and its parameters as name, type, and required.
type NativeSig = (&'static str, Vec<(&'static str, ValueType, bool)>);

/// Returns the first difference between a manifest catalog and a native
/// catalog, or `None` when they agree.
fn catalog_diff(manifest: &Catalog, native: &[NativeSig]) -> Option<String> {
    if manifest.len() != native.len() {
        return Some(format!(
            "the manifest lists {} functions and the crate {}",
            manifest.len(),
            native.len()
        ));
    }
    for (name, params) in native {
        let Some(sig) = manifest.get(*name) else {
            return Some(format!("the manifest lacks function {name}"));
        };
        let theirs: Vec<(&str, ValueType, bool)> = sig
            .params
            .iter()
            .map(|p| (p.name.as_str(), p.ty, p.required))
            .collect();
        if theirs != *params {
            return Some(format!("function {name} has other parameters"));
        }
    }
    None
}

fn no_native(module: &str, diff: &str) -> String {
    format!(
        "module {module} has no native functions, because its manifest catalog differs from the crate: {diff}"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn digest_of_bytes_is_prefixed_lowercase_hex() {
        assert_eq!(
            sha256_digest(b""),
            "sha256:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    #[test]
    fn first_party_manifests_get_their_native_functions() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../modules");
        let mut set = ModuleSet::default();
        for m in ["http1", "dns"] {
            set.insert(Manifest::load(&root.join(m).join("manifest.yaml")).unwrap());
        }
        let mut notes = Vec::new();
        let f = NativeFunctions::for_modules(&set, &[], &root, &mut notes).unwrap();
        assert_eq!(f.modules(), ["http1", "dns"], "{notes:?}");
        assert!(notes.is_empty(), "{notes:?}");

        // The example manifests list other catalogs, so they get none.
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../contracts/examples/modules");
        let mut set = ModuleSet::default();
        set.insert(Manifest::load(&root.join("dns/manifest.yaml")).unwrap());
        let f = NativeFunctions::for_modules(&set, &[], &root, &mut notes).unwrap();
        assert!(f.modules().is_empty());
        assert_eq!(notes.len(), 1, "{notes:?}");
    }

    #[test]
    fn native_functions_render_module_calls() {
        use fps_flow::function::NamedArg;
        use fps_flow::{ArgValue, CallArgs};
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../modules");
        let mut set = ModuleSet::default();
        set.insert(Manifest::load(&root.join("http1/manifest.yaml")).unwrap());
        let mut f = NativeFunctions::for_modules(&set, &[], &root, &mut Vec::new()).unwrap();
        let named = [NamedArg {
            name: "status",
            value: ArgValue::Int(204),
        }];
        let call = FnCall {
            module: "http1",
            function: "response",
            args: CallArgs {
                positional: &[],
                named: &named,
            },
        };
        let mut rng = rand_pcg::Pcg64Mcg::new(1);
        let mut out = Vec::new();
        f.render(&call, &mut rng, &mut out).unwrap();
        assert!(
            out.starts_with(b"HTTP/1.1 204"),
            "{}",
            String::from_utf8_lossy(&out)
        );
        let dns = FnCall {
            module: "dns",
            ..call
        };
        assert!(f.render(&dns, &mut rng, &mut Vec::new()).is_err());
    }

    /// A fresh module directory with a `dns` subdirectory that holds the zone
    /// files of `zones`, and the first party manifests of dns and http1.
    fn settings_dir(tag: &str, zones: &[(&str, &str)]) -> (PathBuf, ModuleSet) {
        let dir =
            std::env::temp_dir().join(format!("fps-agent-settings-{}-{tag}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("dns/zones")).unwrap();
        for (name, text) in zones {
            fs::write(dir.join("dns").join(name), text).unwrap();
        }
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../modules");
        let mut set = ModuleSet::default();
        for m in ["http1", "dns"] {
            set.insert(Manifest::load(&root.join(m).join("manifest.yaml")).unwrap());
        }
        (dir, set)
    }

    fn with_settings(name: &str, settings: Json) -> ModuleRef {
        let Json::Object(settings) = settings else {
            panic!("settings must be an object");
        };
        ModuleRef {
            name: name.into(),
            settings,
            ..ModuleRef::default()
        }
    }

    const CUSTOM_ZONE: &str = "$ORIGIN corp.test.\n$TTL 60\n@ IN SOA ns hostmaster 1 7200 3600 1209600 60\n@ IN NS ns\nns IN A 192.0.2.7\nwww IN A 192.0.2.99\n";

    /// The first A address that dns answers for `name`, or the rcode.
    fn lookup(f: &mut NativeFunctions, name: &str) -> String {
        use fps_flow::function::NamedArg;
        use fps_flow::{ArgValue, CallArgs};
        let mut rng = rand_pcg::Pcg64Mcg::new(1);
        let mut query = Vec::new();
        let named = [NamedArg {
            name: "name",
            value: ArgValue::Text(name),
        }];
        f.render(
            &FnCall {
                module: "dns",
                function: "query",
                args: CallArgs {
                    positional: &[],
                    named: &named,
                },
            },
            &mut rng,
            &mut query,
        )
        .unwrap();
        let mut response = Vec::new();
        f.render(
            &FnCall {
                module: "dns",
                function: "response",
                args: CallArgs {
                    positional: &[ArgValue::Bytes(&query)],
                    named: &[],
                },
            },
            &mut rng,
            &mut response,
        )
        .unwrap();
        let mut address = Vec::new();
        let answer = f.render(
            &FnCall {
                module: "dns",
                function: "answer_ip",
                args: CallArgs {
                    positional: &[ArgValue::Bytes(&response)],
                    named: &[],
                },
            },
            &mut rng,
            &mut address,
        );
        match answer {
            Ok(()) => String::from_utf8_lossy(&address).into_owned(),
            Err(e) => format!("error: {e}"),
        }
    }

    #[test]
    fn the_dns_zone_file_setting_selects_the_zone() {
        let (dir, set) = settings_dir("zone", &[("zones/corp.zone", CUSTOM_ZONE)]);
        let refs = [with_settings(
            "dns",
            serde_json::json!({"zone_file": "zones/corp.zone"}),
        )];
        let mut f = NativeFunctions::for_modules(&set, &refs, &dir, &mut Vec::new()).unwrap();
        assert_eq!(lookup(&mut f, "www.corp.test"), "192.0.2.99");
        // The shipped zone answers www.example.test, so the setting replaced it.
        assert_ne!(lookup(&mut f, "www.example.test"), "192.0.2.1");

        let mut bundled = NativeFunctions::for_modules(&set, &[], &dir, &mut Vec::new()).unwrap();
        assert_eq!(lookup(&mut bundled, "www.example.test"), "192.0.2.1");
        // Empty settings behave like none.
        let empty = [with_settings("dns", serde_json::json!({}))];
        let mut same = NativeFunctions::for_modules(&set, &empty, &dir, &mut Vec::new()).unwrap();
        assert_eq!(lookup(&mut same, "www.example.test"), "192.0.2.1");
        // An absolute path inside the module directory is fine.
        let inside = dir.join("dns/zones/corp.zone");
        let abs = [with_settings(
            "dns",
            serde_json::json!({"zone_file": inside.to_str().unwrap()}),
        )];
        let mut f = NativeFunctions::for_modules(&set, &abs, &dir, &mut Vec::new()).unwrap();
        assert_eq!(lookup(&mut f, "www.corp.test"), "192.0.2.99");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_setting_path_cannot_leave_the_module_directory() {
        let (dir, set) = settings_dir("escape", &[("zones/corp.zone", CUSTOM_ZONE)]);
        fs::write(dir.join("outside.zone"), CUSTOM_ZONE).unwrap();
        let outside = dir.join("outside.zone");
        for path in [
            "../outside.zone",
            "zones/../../outside.zone",
            "zones/../zones/corp.zone",
            outside.to_str().unwrap(),
            "/etc/hostname",
        ] {
            let refs = [with_settings(
                "dns",
                serde_json::json!({ "zone_file": path }),
            )];
            let e = NativeFunctions::for_modules(&set, &refs, &dir, &mut Vec::new()).unwrap_err();
            let text = e.to_string();
            assert!(matches!(e, ModuleError::Settings { .. }), "{path}: {text}");
            assert!(
                text.contains("zone_file") && (text.contains("..") || text.contains("outside")),
                "{path}: {text}"
            );
        }
        let _ = fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn a_link_that_leaves_the_module_directory_is_refused() {
        let (dir, set) = settings_dir("link", &[]);
        fs::write(dir.join("outside.zone"), CUSTOM_ZONE).unwrap();
        std::os::unix::fs::symlink(dir.join("outside.zone"), dir.join("dns/zones/link.zone"))
            .unwrap();
        let refs = [with_settings(
            "dns",
            serde_json::json!({"zone_file": "zones/link.zone"}),
        )];
        let e = NativeFunctions::for_modules(&set, &refs, &dir, &mut Vec::new()).unwrap_err();
        assert!(
            e.to_string().contains("outside the module directory"),
            "{e}"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn invalid_settings_are_errors_that_name_the_module() {
        let (dir, set) = settings_dir("invalid", &[("zones/bad.zone", "not a zone")]);
        for (module, settings, want) in [
            (
                "dns",
                serde_json::json!({"zone_file": "zones/missing.zone"}),
                "missing.zone",
            ),
            (
                "dns",
                serde_json::json!({"zone_file": "zones/bad.zone"}),
                "bad.zone",
            ),
            (
                "dns",
                serde_json::json!({"zone_file": 3}),
                "non empty string",
            ),
            (
                "dns",
                serde_json::json!({"zone": "x"}),
                "unknown dns setting",
            ),
            ("http1", serde_json::json!({"x": 1}), "settings, but"),
        ] {
            let refs = [with_settings(module, settings.clone())];
            let e = NativeFunctions::for_modules(&set, &refs, &dir, &mut Vec::new()).unwrap_err();
            let text = e.to_string();
            assert!(
                text.starts_with(&format!("module {module}: settings: ")),
                "{text}"
            );
            assert!(text.contains(want), "{settings}: {text}");
        }
        // A module whose manifest catalog differs from the crate cannot apply
        // settings, so the agent refuses them instead of ignoring them.
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../contracts/examples/modules");
        let mut example = ModuleSet::default();
        example.insert(Manifest::load(&root.join("dns/manifest.yaml")).unwrap());
        let refs = [with_settings(
            "dns",
            serde_json::json!({"zone_file": "zones/example.test.zone"}),
        )];
        let e = NativeFunctions::for_modules(&example, &refs, &dir, &mut Vec::new()).unwrap_err();
        assert!(e.to_string().contains("no native functions"), "{e}");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn integral_params_bind_as_integers() {
        let s = Scenario::parse(
            "scenario: t\nvars:\n  n: 1\n  r: 0.5\nflows:\n  - id: f\n    module: m\n    client:\n      - connect: { port: 1 }\n      - send: \"{{ vars.n }}\"\n",
        )
        .unwrap();
        let mut params = std::collections::BTreeMap::new();
        params.insert("n".to_owned(), serde_json::json!(4.0));
        params.insert("r".to_owned(), serde_json::json!(0.25));
        let out = with_params("e", &s, &params).unwrap();
        assert_eq!(out.vars.get("n"), Some(&Value::Int(4)));
        assert_eq!(out.vars.get("r"), Some(&Value::Float(0.25)));
        params.insert("missing".to_owned(), serde_json::json!(1));
        assert!(with_params("e", &s, &params).is_err());
    }
}
