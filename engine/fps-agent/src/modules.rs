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

use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use fps_flow::function::NoFunctions;
use fps_flow::manifest::Catalog;
use fps_flow::{FnCall, FnError, Manifest, ModuleSet, Scenario, TemplateFn, Value, ValueType};
use fps_runtime::{ExtensionError, ExtensionSet};
use fps_wasm::Limits as WasmLimits;
use rand::RngCore;
use serde_json::Value as Json;
use sha2::{Digest, Sha256};

use crate::plan::{ModuleRef, Plan, ScenarioSource};
use crate::trust::{ModulePolicy, TrustError};

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
    /// The signature of the module does not satisfy the trust store of the
    /// agent (AGT-04). `--dev-modules` accepts it with a warning.
    #[error("module {module}: {source}")]
    Untrusted {
        /// The module name.
        module: String,
        /// Why the signature is refused.
        #[source]
        source: TrustError,
    },
    /// The `extension.wasm` of a module does not compile, or its catalog
    /// differs from the manifest (AGT-11). The plan is refused, because a run
    /// without the extension would measure another test than the one
    /// configured.
    #[error("module {module}: {source}")]
    Extension {
        /// The module name.
        module: String,
        /// The cause.
        #[source]
        source: ExtensionError,
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
    Ok(read_tree(module, dir, false)?.digest)
}

/// The file of a module tree that holds its manifest.
const MANIFEST_FILE: &str = "manifest.yaml";

/// The bytes of the two files of a module tree that the agent uses after the
/// digest, as the digest pass read them.
#[derive(Default)]
struct Kept {
    /// `manifest.yaml` in the root of the tree.
    manifest: Option<Vec<u8>>,
    /// `extension.wasm` in the root of the tree.
    extension: Option<Vec<u8>>,
}

/// What one pass over a module tree gives: the tree digest and, when the pass
/// kept them, the bytes of `manifest.yaml` and `extension.wasm` that went into
/// that digest.
struct Tree {
    digest: String,
    kept: Kept,
}

/// Reads every file of the module tree `dir` once and digests it. With
/// `keep`, the pass also returns the bytes of the files `manifest.yaml` and
/// `extension.wasm` in the root of the tree, so the caller parses and compiles
/// the bytes that the digest covers and reads neither file a second time.
fn read_tree(module: &str, dir: &Path, keep: bool) -> Result<Tree, ModuleError> {
    let mut lines: Vec<(String, String)> = Vec::new();
    let mut kept = Kept::default();
    walk(module, dir, dir, &mut lines, keep.then_some(&mut kept))?;
    lines.sort_by(|a, b| a.0.as_bytes().cmp(b.0.as_bytes()));
    let mut all = String::new();
    for (_, line) in &lines {
        all.push_str(line);
    }
    Ok(Tree {
        digest: sha256_digest(all.as_bytes()),
        kept,
    })
}

fn walk(
    module: &str,
    root: &Path,
    dir: &Path,
    out: &mut Vec<(String, String)>,
    mut kept: Option<&mut Kept>,
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
            walk(module, root, &path, out, kept.as_deref_mut())?;
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
            if let Some(kept) = kept.as_deref_mut() {
                if rel == fps_runtime::EXTENSION_FILE {
                    kept.extension = Some(data);
                } else if rel == MANIFEST_FILE {
                    kept.manifest = Some(data);
                }
            }
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
    /// The compiled WebAssembly extensions of the plan modules that ship an
    /// `extension.wasm`, or `None` when no module does. Every flow of the
    /// plan shares the set through `FlowRef::extensions`.
    pub extensions: Option<Arc<ExtensionSet>>,
    /// Notes for the operator, such as a module without native functions.
    pub notes: Vec<String>,
    /// Warnings about modules that the agent loaded without a valid
    /// signature, see [`ModulePolicy`].
    pub warnings: Vec<String>,
}

/// Loads, checks, and compiles every module and scenario of `plan` from
/// `dir`.
///
/// Every module passes two checks before anything of it is compiled or
/// registered. The tree digest of its directory must equal the plan digest,
/// which no setting waives, and `policy` judges the signature of the plan
/// over that digest (AGT-04). The `extension.wasm` of a module is part of its
/// tree, so both checks cover it. Only after they pass does the agent compile
/// the extension (AGT-11), and an extension that does not compile or whose
/// catalog differs from the manifest refuses the plan. A plan whose modules
/// ship no extension never starts a WebAssembly engine.
///
/// The agent reads `manifest.yaml` and `extension.wasm` once, in the pass that
/// digests the module tree, and parses and compiles those bytes. A writer that
/// replaces either file after the digest check changes nothing that runs,
/// because the files are not read again (AGT-16 for the extension, AGT-17 for
/// the manifest).
pub fn load(plan: &Plan, dir: &Path, policy: &ModulePolicy) -> Result<LoadedModules, ModuleError> {
    load_with(plan, dir, policy, &mut |_| {})
}

/// [`load`] with a hook that runs for every module after its tree digest
/// matched the plan and before anything of it is compiled. A test uses it to
/// change the module directory at the one moment that a time of check to time
/// of use flaw would matter. It is not part of the stable interface.
#[doc(hidden)]
pub fn load_with(
    plan: &Plan,
    dir: &Path,
    policy: &ModulePolicy,
    after_digest: &mut dyn FnMut(&str),
) -> Result<LoadedModules, ModuleError> {
    let mut set = ModuleSet::default();
    let mut warnings = Vec::new();
    let mut extensions: Option<ExtensionSet> = None;
    let mut extended: Vec<String> = Vec::new();
    for m in &plan.modules {
        let (manifest, extension_bytes) = load_module(m, dir)?;
        after_digest(&m.name);
        // The digest of the directory equals m.digest here, so the signature
        // that covers m.digest covers the bytes that the agent runs: the
        // extension below is compiled from the bytes that the digest covered.
        let verdict = policy
            .judge(&m.name, &m.digest, &m.signature_key_id, &m.signature)
            .map_err(|source| ModuleError::Untrusted {
                module: m.name.clone(),
                source,
            })?;
        warnings.extend(verdict);
        if load_extension(m, &manifest, extension_bytes, &mut extensions)? {
            extended.push(m.name.clone());
        }
        set.insert(manifest);
    }
    if policy.store.is_none() && !plan.modules.is_empty() {
        warnings.push(format!(
            "no --module-trust-store: the signatures of {} module(s) are not required and not checked",
            plan.modules.len()
        ));
    }
    let mut notes = Vec::new();
    // The extension serves the calls of a module without native functions, so
    // the registry adds no note that its calls fail.
    let functions = NativeFunctions::for_modules(&set, &plan.modules, dir, &extended, &mut notes)?;
    for name in &extended {
        notes.push(format!("module {name} runs its WebAssembly extension"));
    }
    notes.sort();
    let mut scenarios = HashMap::new();
    for s in &plan.scenarios {
        scenarios.insert(s.r#ref.clone(), compile(s, &set)?);
    }
    Ok(LoadedModules {
        set,
        scenarios,
        functions,
        extensions: extensions.map(Arc::new),
        notes,
        warnings,
    })
}

/// Compiles `bytes`, the `extension.wasm` that the digest of module `m`
/// covered, into `set` and creates the set at the first extension. Returns
/// whether the module has an extension, that is, whether `bytes` is `Some`.
fn load_extension(
    m: &ModuleRef,
    manifest: &Manifest,
    bytes: Option<Vec<u8>>,
    set: &mut Option<ExtensionSet>,
) -> Result<bool, ModuleError> {
    // A module without an extension starts no engine.
    let Some(bytes) = bytes else {
        return Ok(false);
    };
    let fail = |source| ModuleError::Extension {
        module: m.name.clone(),
        source,
    };
    if set.is_none() {
        *set = Some(ExtensionSet::new().map_err(fail)?);
    }
    let Some(set) = set.as_mut() else {
        return Ok(false);
    };
    set.load_module_bytes(manifest, &bytes, WasmLimits::default())
        .map_err(fail)?;
    Ok(true)
}

/// The note of a module whose template calls fail for lack of native
/// functions.
fn no_native_functions(name: &str) -> String {
    format!("module {name} has no native functions in this agent, so its template calls fail")
}

/// Reads the module tree of `m` once, parses the manifest from the bytes that
/// the pass digested, checks its name and version against the plan and the
/// digest against the plan, and returns the manifest with the bytes of the
/// `extension.wasm` of the same pass, if the module has one.
///
/// The manifest is part of the tree, so the digest covers it. Parsing the
/// digested bytes means that a manifest swapped after the digest pass is never
/// parsed, and that the catalog the agent compiles scenarios against is the
/// one that the plan digest, and the signature over it, name.
fn load_module(m: &ModuleRef, dir: &Path) -> Result<(Manifest, Option<Vec<u8>>), ModuleError> {
    let root = dir.join(&m.name);
    let Tree { digest, kept } = read_tree(&m.name, &root, true)?;
    let path = root.join(MANIFEST_FILE);
    let (manifest, version) =
        parse_manifest(&path, kept.manifest).map_err(|source| ModuleError::Manifest {
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
    if version != m.version {
        return Err(ModuleError::Mismatch {
            module: m.name.clone(),
            field: "version",
            got: version,
            want: m.version.clone(),
        });
    }
    if digest != m.digest {
        return Err(ModuleError::Digest {
            module: m.name.clone(),
            actual: digest,
            want: m.digest.clone(),
        });
    }
    Ok((manifest, kept.extension))
}

/// Parses the manifest from `bytes`, the content of the file `path` as the
/// digest pass read it, and returns it with its top level `version`. A tree
/// without the file gives the error of a failed read, as `Manifest::load`
/// does.
fn parse_manifest(
    path: &Path,
    bytes: Option<Vec<u8>>,
) -> Result<(Manifest, String), fps_flow::ManifestError> {
    use fps_flow::ManifestError;
    let read = |source| ManifestError::Read {
        path: path.to_path_buf(),
        source,
    };
    let bytes = bytes.ok_or_else(|| read(std::io::ErrorKind::NotFound.into()))?;
    let text = String::from_utf8(bytes).map_err(|e| {
        read(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            e.utf8_error(),
        ))
    })?;
    let manifest = Manifest::parse(&text).map_err(|e| match e {
        ManifestError::Invalid { path: None, reason } => ManifestError::Invalid {
            path: Some(path.to_path_buf()),
            reason,
        },
        other => other,
    })?;
    Ok((manifest, version_of(&text).unwrap_or_default()))
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
        // One pass gives the digest and the manifest bytes that it covers.
        let label = entry.file_name().to_string_lossy().into_owned();
        let Ok(Tree { digest, kept }) = read_tree(&label, &root, true) else {
            continue;
        };
        let Ok((manifest, version)) = parse_manifest(&root.join(MANIFEST_FILE), kept.manifest)
        else {
            continue;
        };
        out.push(ModuleRef {
            version,
            name: manifest.name,
            digest,
            settings: serde_json::Map::new(),
            ..Default::default()
        });
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

/// Returns the top level `version` of manifest text, which the `fps-flow`
/// manifest reader leaves out.
fn version_of(text: &str) -> Option<String> {
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
    // A scenario instance runner accepts the spawned flows and the `spawn`
    // action that the single flow loader refuses.
    let scenario = Scenario::parse_instance(&s.source, ctx.as_ref()).map_err(|source| {
        ModuleError::Scenario {
            scenario: s.r#ref.clone(),
            source,
        }
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

/// A native module as the registry holds it: its template functions and the
/// means to copy them, because every session gets its own value.
trait NativeModule: TemplateFn + std::fmt::Debug + Send + Sync {
    fn duplicate(&self) -> Box<dyn NativeModule>;
}

impl<T: TemplateFn + Clone + std::fmt::Debug + Send + Sync + 'static> NativeModule for T {
    fn duplicate(&self) -> Box<dyn NativeModule> {
        Box::new(self.clone())
    }
}

impl Clone for Box<dyn NativeModule> {
    fn clone(&self) -> Self {
        self.duplicate()
    }
}

/// What a constructor of the registry gets.
struct BuildCtx<'a> {
    /// The directory of the installed modules, `<dir>/<module>` being the
    /// module tree.
    dir: &'a Path,
    /// The non empty settings of the plan entry, if any.
    given: Option<&'a serde_json::Map<String, Json>>,
}

/// Why a constructor failed.
enum BuildError {
    /// The settings are invalid. The plan is refused.
    Settings(String),
    /// The module cannot get native functions in this run. The module stays
    /// without them and the run notes it.
    Unavailable(String),
}

/// One first party module of the agent.
struct Registration {
    /// The module name, which `MODULE_NAME` of its crate defines.
    name: &'static str,
    /// The function catalog of the crate, to compare with the manifest.
    catalog: fn() -> Vec<NativeSig>,
    /// Builds the native functions from the plan settings.
    build: fn(&BuildCtx<'_>) -> Result<Box<dyn NativeModule>, BuildError>,
}

/// Lists the catalog of a crate whose `FUNCTIONS` use the `FunctionSpec`
/// shape that every first party module shares.
macro_rules! catalog_of {
    ($krate:ident) => {
        $krate::FUNCTIONS
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
            .collect()
    };
}

/// Registers a module whose native functions are a unit value that takes no
/// settings. A plan that gives settings to such a module is refused.
macro_rules! stateless {
    ($krate:ident, $ty:ident) => {
        Registration {
            name: $krate::MODULE_NAME,
            catalog: || catalog_of!($krate),
            build: |ctx| {
                if ctx.given.is_some() {
                    return Err(BuildError::Settings(
                        "the plan gives settings, but its native functions take none".to_owned(),
                    ));
                }
                Ok(Box::new($krate::$ty))
            },
        }
    };
}

/// The first party modules of the agent. A new module adds one line here and
/// one dependency in `Cargo.toml`. A module that reads settings gets a
/// constructor like [`build_dns`] instead of `stateless!`.
static REGISTRY: &[Registration] = &[
    stateless!(fps_module_http1, Http1),
    Registration {
        name: fps_module_dns::MODULE_NAME,
        catalog: || catalog_of!(fps_module_dns),
        build: build_dns,
    },
    stateless!(fps_module_content, Content),
    stateless!(fps_module_smtp, Smtp),
    stateless!(fps_module_imap, Imap),
    stateless!(fps_module_pop3, Pop3),
    stateless!(fps_module_ftp, Ftp),
    stateless!(fps_module_smb2, Smb2),
    // Http2 holds the HPACK tables of one connection side, so it is a value
    // with state and not a unit value. Every session clones its own copy.
    Registration {
        name: fps_module_http2::MODULE_NAME,
        catalog: || catalog_of!(fps_module_http2),
        build: |ctx| {
            if ctx.given.is_some() {
                return Err(BuildError::Settings(
                    "the plan gives settings, but its native functions take none".to_owned(),
                ));
            }
            Ok(Box::new(fps_module_http2::Http2::default()))
        },
    },
    // Ssh holds the keys and the packet counters of one session side, so every
    // session side gets a fresh value, see [`SshSide`].
    Registration {
        name: fps_module_ssh::MODULE_NAME,
        catalog: || catalog_of!(fps_module_ssh),
        build: |ctx| {
            if ctx.given.is_some() {
                return Err(BuildError::Settings(
                    "the plan gives settings, but its native functions take none".to_owned(),
                ));
            }
            Ok(Box::new(SshSide::default()))
        },
    },
    // SipRtp holds the RTP stream and the pacing clock of one flow side. The
    // runtime asks the factory once per flow side, so the signaling and the
    // media flow of a call, and both ends of each, get their own value. It is
    // `Clone`, and the registry value itself never renders, so a clone starts
    // from nothing.
    Registration {
        name: fps_module_sip_rtp::MODULE_NAME,
        catalog: || catalog_of!(fps_module_sip_rtp),
        build: |ctx| {
            if ctx.given.is_some() {
                return Err(BuildError::Settings(
                    "the plan gives settings, but its native functions take none".to_owned(),
                ));
            }
            Ok(Box::new(fps_module_sip_rtp::SipRtp::default()))
        },
    },
];

/// The ssh functions of one session side. `fps_module_ssh::Ssh` is not
/// `Clone`, because a copy would share the keys and the packet counters of one
/// SSH session between two sessions. The registry clones its value for every
/// session side (see [`NativeFunctions::factory`]), so a clone of this wrapper
/// starts a new session with a fresh `Ssh` and carries nothing of the original.
#[derive(Default)]
struct SshSide(fps_module_ssh::Ssh);

impl Clone for SshSide {
    fn clone(&self) -> Self {
        Self::default()
    }
}

impl std::fmt::Debug for SshSide {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // The keys of the session stay out of debug output.
        f.debug_struct("SshSide").finish_non_exhaustive()
    }
}

impl TemplateFn for SshSide {
    fn render(
        &mut self,
        call: &FnCall<'_>,
        rng: &mut dyn RngCore,
        out: &mut Vec<u8>,
    ) -> Result<(), FnError> {
        self.0.render(call, rng, out)
    }
}

fn build_dns(ctx: &BuildCtx<'_>) -> Result<Box<dyn NativeModule>, BuildError> {
    match dns_functions(ctx.dir, ctx.given) {
        Ok(d) => Ok(Box::new(d)),
        Err(DnsLoad::Settings(reason)) => Err(BuildError::Settings(reason)),
        Err(DnsLoad::Bundled(e)) => Err(BuildError::Unavailable(format!(
            "module dns has no native functions: bundled zone: {e}"
        ))),
    }
}

/// The native template functions of one session, by module.
#[derive(Clone, Debug, Default)]
pub struct NativeFunctions {
    modules: BTreeMap<&'static str, Box<dyn NativeModule>>,
}

impl NativeFunctions {
    /// Registers the native functions of every module in `set` that the
    /// registry knows and whose manifest catalog equals the catalog of its
    /// crate, configured with the settings of the matching entry of `refs`.
    /// A relative path setting resolves against the directory of the module
    /// under `dir` and may not leave it. Each module that stays without
    /// native functions adds a note, except a module named in `extended`,
    /// whose calls its WebAssembly extension serves.
    ///
    /// Settings that the agent cannot apply are an error, because a run that
    /// ignores them would measure another test than the one configured. That
    /// covers a module without native functions, extension or not, and a
    /// module whose native functions take no settings.
    pub fn for_modules(
        set: &ModuleSet,
        refs: &[ModuleRef],
        dir: &Path,
        extended: &[String],
        notes: &mut Vec<String>,
    ) -> Result<Self, ModuleError> {
        let mut out = Self::default();
        let settings = |name: &str| refs.iter().find(|m| m.name == name).map(|m| &m.settings);
        for (name, manifest) in &set.manifests {
            let given = settings(name).filter(|s| !s.is_empty());
            let Some(entry) = REGISTRY.iter().find(|r| r.name == name.as_str()) else {
                refuse_settings(name, given, "it has no native functions here")?;
                if !manifest.functions.is_empty() && !extended.contains(name) {
                    notes.push(no_native_functions(name));
                }
                continue;
            };
            if let Some(d) = catalog_diff(&manifest.functions, &(entry.catalog)()) {
                refuse_settings(name, given, "it has no native functions here")?;
                notes.push(no_native(name, &d));
                continue;
            }
            match (entry.build)(&BuildCtx { dir, given }) {
                Ok(m) => {
                    out.modules.insert(entry.name, m);
                }
                Err(BuildError::Settings(reason)) => {
                    return Err(ModuleError::Settings {
                        module: name.clone(),
                        reason,
                    })
                }
                Err(BuildError::Unavailable(note)) => notes.push(note),
            }
        }
        out.add_required(set, dir, notes);
        notes.sort();
        Ok(out)
    }

    /// Registers the native functions of the modules that a plan module
    /// requires and the plan does not list.
    ///
    /// The plan names the modules of its scenarios only, so a plan with an
    /// smtp scenario lists smtp but not content, whose functions the scenario
    /// calls through `requires`. The registry knows such a module by name,
    /// and the plan gives it no settings, so the agent registers it with
    /// none. When its manifest is installed under `dir`, the catalog must
    /// equal the catalog of the crate, as for a listed module, or the module
    /// stays without native functions and the run notes it.
    fn add_required(&mut self, set: &ModuleSet, dir: &Path, notes: &mut Vec<String>) {
        let mut wanted: Vec<(&str, &str)> = set
            .manifests
            .values()
            .flat_map(|m| m.requires.iter().map(|r| (r.as_str(), m.name.as_str())))
            .filter(|(r, _)| !set.manifests.contains_key(*r))
            .collect();
        wanted.sort_unstable();
        wanted.dedup_by_key(|(r, _)| *r);
        for (name, by) in wanted {
            let Some(entry) = REGISTRY.iter().find(|r| r.name == name) else {
                continue;
            };
            if let Ok(manifest) = Manifest::load(&dir.join(name).join("manifest.yaml")) {
                if let Some(d) = catalog_diff(&manifest.functions, &(entry.catalog)()) {
                    notes.push(no_native(name, &d));
                    continue;
                }
            }
            if let Ok(m) = (entry.build)(&BuildCtx { dir, given: None }) {
                self.modules.insert(entry.name, m);
                notes.push(format!(
                    "module {name}, which {by} requires, runs the native functions of this agent"
                ));
            }
        }
    }

    /// Returns the modules that have native functions, in the order of the
    /// registry.
    pub fn modules(&self) -> Vec<&'static str> {
        REGISTRY
            .iter()
            .map(|r| r.name)
            .filter(|n| self.modules.contains_key(n))
            .collect()
    }

    /// Returns the names of every module that the registry knows, in its
    /// order.
    pub fn registered() -> Vec<&'static str> {
        REGISTRY.iter().map(|r| r.name).collect()
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
        match self.modules.get_mut(call.module) {
            Some(m) => m.render(call, rng, out),
            None => NoFunctions.render(call, rng, out),
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

    /// The registered modules, each of which must have a manifest under
    /// `root`.
    fn registered_with_manifest(root: &Path) -> Vec<&'static str> {
        NativeFunctions::registered()
            .into_iter()
            .inspect(|m| {
                assert!(
                    root.join(m).join("manifest.yaml").exists(),
                    "module {m} has no manifest"
                );
            })
            .collect()
    }

    #[test]
    fn first_party_manifests_get_their_native_functions() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../modules");
        let mut set = ModuleSet::default();
        let present = registered_with_manifest(&root);
        for m in &present {
            set.insert(Manifest::load(&root.join(m).join("manifest.yaml")).unwrap());
        }
        let mut notes = Vec::new();
        let f = NativeFunctions::for_modules(&set, &[], &root, &[], &mut notes).unwrap();
        assert_eq!(f.modules(), present, "{notes:?}");
        assert!(notes.is_empty(), "{notes:?}");

        // The example manifests list other catalogs, so they get none.
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../contracts/examples/modules");
        let mut set = ModuleSet::default();
        set.insert(Manifest::load(&root.join("dns/manifest.yaml")).unwrap());
        let f = NativeFunctions::for_modules(&set, &[], &root, &[], &mut notes).unwrap();
        assert!(f.modules().is_empty());
        assert_eq!(notes.len(), 1, "{notes:?}");
    }

    #[test]
    fn the_registry_covers_every_first_party_module() {
        assert_eq!(
            NativeFunctions::registered(),
            [
                "http1", "dns", "content", "smtp", "imap", "pop3", "ftp", "smb2", "http2", "ssh",
                "sip_rtp"
            ]
        );
    }

    /// AGT-17. The factory gives every session side its own ssh value. A call
    /// on one value moves its state, and a value that the factory makes later
    /// starts from nothing, as the key exchange of a new session needs.
    #[test]
    fn every_session_side_gets_its_own_ssh_value() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../modules");
        let mut set = ModuleSet::default();
        set.insert(Manifest::load(&root.join("ssh/manifest.yaml")).unwrap());
        let f = NativeFunctions::for_modules(&set, &[], &root, &[], &mut Vec::new()).unwrap();
        assert_eq!(f.modules(), ["ssh"]);
        let factory = f.factory();
        let (mut a, mut b) = (factory(), factory());
        let mut rng = rand_pcg::Pcg64Mcg::new(7);
        let call = |function| FnCall {
            module: "ssh",
            function,
            args: fps_flow::CallArgs {
                positional: &[],
                named: &[],
            },
        };
        // `ecdh_init` makes side `a` a client in the middle of a key exchange.
        // Its `newkeys` then fails because the exchange is unfinished, while a
        // side that has done nothing fails because it has no role yet. The two
        // errors differ, so side `b` shares nothing with side `a`.
        a.render(&call("ecdh_init"), &mut rng, &mut Vec::new())
            .expect("ecdh_init");
        let e = a
            .render(&call("newkeys"), &mut rng, &mut Vec::new())
            .expect_err("the exchange is unfinished")
            .to_string();
        assert!(e.contains("before the key exchange finished"), "{e}");
        let e = b
            .render(&call("newkeys"), &mut rng, &mut Vec::new())
            .expect_err("no exchange started")
            .to_string();
        assert!(e.contains("wrong order"), "{e}");
        // A value that the factory makes after `a` ran starts fresh too.
        let mut c = factory();
        let e = c
            .render(&call("newkeys"), &mut rng, &mut Vec::new())
            .expect_err("no exchange started")
            .to_string();
        assert!(e.contains("wrong order"), "{e}");
    }

    /// AGT-20. The factory gives every flow side its own sip_rtp value: the RTP
    /// stream that one side started does not exist on a value that the factory
    /// makes later, so the media flows of two calls never share a sequence.
    #[test]
    fn every_flow_side_gets_its_own_sip_rtp_value() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../modules");
        let mut set = ModuleSet::default();
        set.insert(Manifest::load(&root.join("sip_rtp/manifest.yaml")).unwrap());
        let f = NativeFunctions::for_modules(&set, &[], &root, &[], &mut Vec::new()).unwrap();
        assert_eq!(f.modules(), ["sip_rtp"]);
        let factory = f.factory();
        let (mut a, mut b) = (factory(), factory());
        let mut rng = rand_pcg::Pcg64Mcg::new(7);
        let args = [
            fps_flow::ArgValue::Text("PCMU"),
            fps_flow::ArgValue::Int(20),
        ];
        let call = || FnCall {
            module: "sip_rtp",
            function: "rtp",
            args: fps_flow::CallArgs {
                positional: &args,
                named: &[],
            },
        };
        let (mut one, mut two, mut three) = (Vec::new(), Vec::new(), Vec::new());
        a.render(&call(), &mut rng, &mut one).expect("first packet");
        a.render(&call(), &mut rng, &mut two)
            .expect("second packet");
        // The sequence number of an RTP header sits in bytes 2 and 3 and moves
        // by one per packet on one side.
        let seq = |p: &[u8]| u16::from_be_bytes([p[2], p[3]]);
        assert_eq!(seq(&two), seq(&one).wrapping_add(1));
        // Another side starts a stream of its own, and so does a later value.
        b.render(&call(), &mut rng, &mut three).expect("other side");
        assert_ne!(seq(&three), seq(&two), "the sides share a stream");
        let mut four = Vec::new();
        factory()
            .render(&call(), &mut rng, &mut four)
            .expect("later");
        assert_ne!(seq(&four), seq(&two), "a later value shares a stream");
    }

    /// Every function of every first party manifest reaches the crate of its
    /// module. A call without arguments may fail on its arguments, but never
    /// with the error of a module that nobody provides.
    #[test]
    fn each_first_party_function_resolves() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../modules");
        let mut set = ModuleSet::default();
        for m in registered_with_manifest(&root) {
            set.insert(Manifest::load(&root.join(m).join("manifest.yaml")).unwrap());
        }
        let mut f = NativeFunctions::for_modules(&set, &[], &root, &[], &mut Vec::new()).unwrap();
        let mut rng = rand_pcg::Pcg64Mcg::new(1);
        let mut resolved = 0;
        for (module, manifest) in &set.manifests {
            assert!(
                !manifest.functions.is_empty(),
                "{module} lists no functions"
            );
            for name in manifest.functions.keys() {
                let call = FnCall {
                    module,
                    function: name,
                    args: fps_flow::CallArgs {
                        positional: &[],
                        named: &[],
                    },
                };
                if let Err(e) = f.render(&call, &mut rng, &mut Vec::new()) {
                    assert!(
                        !e.to_string().contains("no module provides"),
                        "{module}.{name} does not resolve: {e}"
                    );
                }
                resolved += 1;
            }
        }
        assert!(resolved >= 7, "{resolved}");
        // A module that the registry does not know still fails every call.
        let call = FnCall {
            module: "unknown",
            function: "x",
            args: fps_flow::CallArgs {
                positional: &[],
                named: &[],
            },
        };
        let e = f.render(&call, &mut rng, &mut Vec::new()).unwrap_err();
        assert!(e.to_string().contains("no module provides"), "{e}");
    }

    #[test]
    fn stateless_modules_refuse_settings_and_extension_modules_add_no_note() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../modules");
        let mut set = ModuleSet::default();
        for m in ["smtp", "content"] {
            set.insert(Manifest::load(&root.join(m).join("manifest.yaml")).unwrap());
        }
        for m in ["smtp", "content"] {
            let refs = [with_settings(m, serde_json::json!({"x": 1}))];
            let e =
                NativeFunctions::for_modules(&set, &refs, &root, &[], &mut Vec::new()).unwrap_err();
            assert!(
                e.to_string()
                    .starts_with(&format!("module {m}: settings: the plan gives settings")),
                "{e}"
            );
        }
        // A module outside the registry gets a note, unless its extension
        // serves it. Settings stay refused in both cases.
        let examples =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../contracts/examples/modules");
        let mut other = ModuleSet::default();
        other.insert(Manifest::load(&examples.join("http1/manifest.yaml")).unwrap());
        let mut other_name = Manifest::load(&examples.join("http1/manifest.yaml")).unwrap();
        other_name.name = "community".to_owned();
        let mut set = ModuleSet::default();
        set.insert(other_name);
        let mut notes = Vec::new();
        NativeFunctions::for_modules(&set, &[], &root, &[], &mut notes).unwrap();
        assert_eq!(notes, [no_native_functions("community")]);
        let mut notes = Vec::new();
        let ext = ["community".to_owned()];
        NativeFunctions::for_modules(&set, &[], &root, &ext, &mut notes).unwrap();
        assert!(notes.is_empty(), "{notes:?}");
        let refs = [with_settings("community", serde_json::json!({"x": 1}))];
        let e =
            NativeFunctions::for_modules(&set, &refs, &root, &ext, &mut Vec::new()).unwrap_err();
        assert!(e.to_string().contains("no native functions"), "{e}");
    }

    #[test]
    fn a_required_module_that_the_plan_omits_gets_its_native_functions() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../modules");
        let mut set = ModuleSet::default();
        for m in ["smtp", "imap", "pop3", "ftp"] {
            set.insert(Manifest::load(&root.join(m).join("manifest.yaml")).unwrap());
        }
        let mut notes = Vec::new();
        let f = NativeFunctions::for_modules(&set, &[], &root, &[], &mut notes).unwrap();
        assert_eq!(f.modules(), ["content", "smtp", "imap", "pop3", "ftp"]);
        // One note, although four modules require content.
        assert_eq!(notes.len(), 1, "{notes:?}");
        assert!(notes[0].starts_with("module content, which "), "{notes:?}");

        // A directory without the content manifest still gets the module,
        // and one whose manifest differs from the crate does not.
        let dir = std::env::temp_dir().join(format!("fps-agent-required-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("content")).unwrap();
        let f = NativeFunctions::for_modules(&set, &[], &dir, &[], &mut Vec::new()).unwrap();
        assert!(f.modules().contains(&"content"));
        fs::write(
            dir.join("content/manifest.yaml"),
            "name: content\ntype: app\nfunctions:\n  - { name: other, params: [] }\n",
        )
        .unwrap();
        let mut notes = Vec::new();
        let f = NativeFunctions::for_modules(&set, &[], &dir, &[], &mut notes).unwrap();
        assert!(!f.modules().contains(&"content"), "{notes:?}");
        assert!(
            notes
                .iter()
                .any(|n| n.contains("module content has no native")),
            "{notes:?}"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn native_functions_render_module_calls() {
        use fps_flow::function::NamedArg;
        use fps_flow::{ArgValue, CallArgs};
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../modules");
        let mut set = ModuleSet::default();
        set.insert(Manifest::load(&root.join("http1/manifest.yaml")).unwrap());
        let mut f = NativeFunctions::for_modules(&set, &[], &root, &[], &mut Vec::new()).unwrap();
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
        let mut f = NativeFunctions::for_modules(&set, &refs, &dir, &[], &mut Vec::new()).unwrap();
        assert_eq!(lookup(&mut f, "www.corp.test"), "192.0.2.99");
        // The shipped zone answers www.example.test, so the setting replaced it.
        assert_ne!(lookup(&mut f, "www.example.test"), "192.0.2.1");

        let mut bundled =
            NativeFunctions::for_modules(&set, &[], &dir, &[], &mut Vec::new()).unwrap();
        assert_eq!(lookup(&mut bundled, "www.example.test"), "192.0.2.1");
        // Empty settings behave like none.
        let empty = [with_settings("dns", serde_json::json!({}))];
        let mut same =
            NativeFunctions::for_modules(&set, &empty, &dir, &[], &mut Vec::new()).unwrap();
        assert_eq!(lookup(&mut same, "www.example.test"), "192.0.2.1");
        // An absolute path inside the module directory is fine.
        let inside = dir.join("dns/zones/corp.zone");
        let abs = [with_settings(
            "dns",
            serde_json::json!({"zone_file": inside.to_str().unwrap()}),
        )];
        let mut f = NativeFunctions::for_modules(&set, &abs, &dir, &[], &mut Vec::new()).unwrap();
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
            let e =
                NativeFunctions::for_modules(&set, &refs, &dir, &[], &mut Vec::new()).unwrap_err();
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
        let e = NativeFunctions::for_modules(&set, &refs, &dir, &[], &mut Vec::new()).unwrap_err();
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
            let e =
                NativeFunctions::for_modules(&set, &refs, &dir, &[], &mut Vec::new()).unwrap_err();
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
        let e =
            NativeFunctions::for_modules(&example, &refs, &dir, &[], &mut Vec::new()).unwrap_err();
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
