// SPDX-License-Identifier: GPL-3.0-only
//! Errors of the scenario loader.

use std::path::{Path, PathBuf};

use crate::location::{FlowRef, Location, Side, Step};
use crate::template::{CheckError, TemplateError};

/// Errors that loading, checking, or compiling a scenario reports.
///
/// Every variant except [`LoadError::Read`] carries a [`Location`] in `at`,
/// which names the file, the flow, the side, and the action path. The
/// variants after [`LoadError::UnknownVariable`] are the semantic rules of
/// `tools/contracts/validate.py`.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum LoadError {
    /// The scenario file could not be read.
    #[error("reading {path}")]
    Read {
        /// The scenario file.
        path: PathBuf,
        /// The underlying error.
        #[source]
        source: std::io::Error,
    },
    /// The text is not valid YAML.
    #[error("{}invalid YAML: {message}", prefix(.at))]
    Syntax {
        /// Location of the error.
        at: Box<Location>,
        /// Message of the YAML parser.
        message: String,
    },
    /// A required field is absent or has the wrong type.
    #[error("{}missing '{field}'", prefix(.at))]
    Missing {
        /// Location of the error.
        at: Box<Location>,
        /// The field name.
        field: &'static str,
    },
    /// A field holds a value that the grammar rejects.
    #[error("{}{reason}", prefix(.at))]
    Invalid {
        /// Location of the error.
        at: Box<Location>,
        /// What is wrong.
        reason: String,
    },
    /// The scenario uses a feature that this engine version does not run yet.
    #[error("{}{feature} is not supported by this engine version", prefix(.at))]
    Unsupported {
        /// Location of the error.
        at: Box<Location>,
        /// The feature.
        feature: String,
    },
    /// A template failed to compile or to type check.
    #[error("{}{error}", prefix(.at))]
    Template {
        /// Location of the error.
        at: Box<Location>,
        /// The template error.
        error: TemplateError,
    },
    /// Two flows share one id.
    #[error("{}duplicate flow id '{id}'", prefix(.at))]
    DuplicateFlow {
        /// Location of the error.
        at: Box<Location>,
        /// The repeated id.
        id: String,
    },
    /// An `after` entry names a flow that does not exist.
    #[error("{}flow '{flow}' depends on unknown flow '{dependency}'", prefix(.at))]
    UnknownFlow {
        /// Location of the error.
        at: Box<Location>,
        /// The dependent flow.
        flow: String,
        /// The missing dependency.
        dependency: String,
    },
    /// The `after` relation has a cycle.
    #[error("{}dependency cycle among flows {flows:?}", prefix(.at))]
    Cycle {
        /// Location of the error.
        at: Box<Location>,
        /// The flows that cannot be ordered.
        flows: Vec<String>,
    },
    /// An override names a variable that the scenario does not declare.
    #[error("{}scenario has no variable '{name}'", prefix(.at))]
    UnknownVariable {
        /// Location of the error.
        at: Box<Location>,
        /// The variable name.
        name: String,
    },
    /// An `after` entry names its own flow.
    #[error("{}flow '{flow}' depends on itself", prefix(.at))]
    SelfDependency {
        /// Location of the error.
        at: Box<Location>,
        /// The flow.
        flow: String,
    },
    /// A flow names a module that is neither the owner nor a required module.
    #[error("{}flow module '{module}' is neither '{owner}' nor a required module", prefix(.at))]
    ForeignFlowModule {
        /// Location of the error.
        at: Box<Location>,
        /// The module of the flow.
        module: String,
        /// The module that ships the scenario.
        owner: String,
    },
    /// A side does not start with its opening action.
    #[error("{}{side} actions must start with '{}', found '{found}'", prefix(.at), .side.opener())]
    MissingOpener {
        /// Location of the error.
        at: Box<Location>,
        /// The side.
        side: Side,
        /// The first action found.
        found: String,
    },
    /// A `spawn` names a flow that does not exist.
    #[error("{}spawn of unknown flow '{flow}'", prefix(.at))]
    UnknownSpawnTarget {
        /// Location of the error.
        at: Box<Location>,
        /// The named flow.
        flow: String,
    },
    /// A `spawn` names a flow without `spawned: true`.
    #[error("{}flow '{flow}' must declare spawned: true", prefix(.at))]
    NotSpawned {
        /// Location of the error.
        at: Box<Location>,
        /// The named flow.
        flow: String,
    },
    /// The arm probabilities of a `branch` do not sum to 1.
    #[error("{}branch probabilities sum to {sum}, not 1", prefix(.at))]
    BranchSum {
        /// Location of the error.
        at: Box<Location>,
        /// The sum.
        sum: f64,
    },
    /// A uniform delay has `min_ms` above `max_ms`.
    #[error("{}uniform delay has min_ms greater than max_ms", prefix(.at))]
    UniformRange {
        /// Location of the error.
        at: Box<Location>,
    },
    /// An action is reserved for other module types.
    #[error("{}'{action}' is reserved for module types {types:?}", prefix(.at))]
    ReservedAction {
        /// Location of the error.
        at: Box<Location>,
        /// The action.
        action: String,
        /// The module types that may use it.
        types: &'static [&'static str],
    },
    /// `max_datagram` appears on a TCP flow.
    #[error("{}max_datagram applies only to UDP flows", prefix(.at))]
    MaxDatagramOnTcp {
        /// Location of the error.
        at: Box<Location>,
    },
    /// `tls_upgrade` appears in a UDP flow.
    #[error("{}tls_upgrade applies only to TCP flows", prefix(.at))]
    TlsUpgradeOnUdp {
        /// Location of the error.
        at: Box<Location>,
    },
    /// An `expect` of a UDP flow cannot fit one datagram.
    #[error("{}{field} {size} exceeds the datagram limit of {limit} bytes", prefix(.at))]
    DatagramLimit {
        /// Location of the error.
        at: Box<Location>,
        /// `bytes`, `min_bytes`, or `pattern`, whose size is its length.
        field: &'static str,
        /// The size that the expect needs.
        size: u64,
        /// The datagram limit.
        limit: u64,
    },
    /// `last` appears where no `expect` always ran before on the same side.
    #[error("{}'last' needs an expect that always runs earlier on the same side", prefix(.at))]
    LastWithoutExpect {
        /// Location of the error.
        at: Box<Location>,
    },
    /// A template calls a module that is neither the owner nor a required module.
    #[error("{}module '{module}' is neither '{owner}' nor a required module", prefix(.at))]
    ForeignModule {
        /// Location of the error.
        at: Box<Location>,
        /// The called module.
        module: String,
        /// The module that ships the scenario.
        owner: String,
    },
    /// A template calls a function that the module manifest does not declare.
    #[error("{}module '{module}' declares no template function '{function}' in its manifest", prefix(.at))]
    UnknownModuleFunction {
        /// Location of the error.
        at: Box<Location>,
        /// Module name.
        module: String,
        /// Function name.
        function: String,
    },
}

fn prefix(at: &Location) -> String {
    if at.is_empty() {
        String::new()
    } else {
        format!("{at}: ")
    }
}

/// A finding that does not reject the scenario.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Warning {
    /// Where the finding applies.
    pub at: Location,
    /// What the finding says.
    pub message: String,
}

impl std::fmt::Display for Warning {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.at.is_empty() {
            f.write_str(&self.message)
        } else {
            write!(f, "{}: {}", self.at, self.message)
        }
    }
}

impl LoadError {
    /// Returns the location of the error. A read error has none.
    pub fn location(&self) -> Option<&Location> {
        Some(match self {
            LoadError::Read { .. } => return None,
            LoadError::Syntax { at, .. }
            | LoadError::Missing { at, .. }
            | LoadError::Invalid { at, .. }
            | LoadError::Unsupported { at, .. }
            | LoadError::Template { at, .. }
            | LoadError::DuplicateFlow { at, .. }
            | LoadError::UnknownFlow { at, .. }
            | LoadError::Cycle { at, .. }
            | LoadError::UnknownVariable { at, .. }
            | LoadError::SelfDependency { at, .. }
            | LoadError::ForeignFlowModule { at, .. }
            | LoadError::MissingOpener { at, .. }
            | LoadError::UnknownSpawnTarget { at, .. }
            | LoadError::NotSpawned { at, .. }
            | LoadError::BranchSum { at, .. }
            | LoadError::UniformRange { at }
            | LoadError::ReservedAction { at, .. }
            | LoadError::MaxDatagramOnTcp { at }
            | LoadError::TlsUpgradeOnUdp { at }
            | LoadError::DatagramLimit { at, .. }
            | LoadError::LastWithoutExpect { at }
            | LoadError::ForeignModule { at, .. }
            | LoadError::UnknownModuleFunction { at, .. } => at,
        })
    }

    /// Returns the name of the variant, for tests and reports that compare
    /// error kinds across validators.
    pub fn kind(&self) -> &'static str {
        match self {
            LoadError::Read { .. } => "Read",
            LoadError::Syntax { .. } => "Syntax",
            LoadError::Missing { .. } => "Missing",
            LoadError::Invalid { .. } => "Invalid",
            LoadError::Unsupported { .. } => "Unsupported",
            LoadError::Template { .. } => "Template",
            LoadError::DuplicateFlow { .. } => "DuplicateFlow",
            LoadError::UnknownFlow { .. } => "UnknownFlow",
            LoadError::Cycle { .. } => "Cycle",
            LoadError::UnknownVariable { .. } => "UnknownVariable",
            LoadError::SelfDependency { .. } => "SelfDependency",
            LoadError::ForeignFlowModule { .. } => "ForeignFlowModule",
            LoadError::MissingOpener { .. } => "MissingOpener",
            LoadError::UnknownSpawnTarget { .. } => "UnknownSpawnTarget",
            LoadError::NotSpawned { .. } => "NotSpawned",
            LoadError::BranchSum { .. } => "BranchSum",
            LoadError::UniformRange { .. } => "UniformRange",
            LoadError::ReservedAction { .. } => "ReservedAction",
            LoadError::MaxDatagramOnTcp { .. } => "MaxDatagramOnTcp",
            LoadError::TlsUpgradeOnUdp { .. } => "TlsUpgradeOnUdp",
            LoadError::DatagramLimit { .. } => "DatagramLimit",
            LoadError::LastWithoutExpect { .. } => "LastWithoutExpect",
            LoadError::ForeignModule { .. } => "ForeignModule",
            LoadError::UnknownModuleFunction { .. } => "UnknownModuleFunction",
        }
    }

    pub(crate) fn location_mut(&mut self) -> Option<&mut Location> {
        Some(match self {
            LoadError::Read { .. } => return None,
            LoadError::Syntax { at, .. }
            | LoadError::Missing { at, .. }
            | LoadError::Invalid { at, .. }
            | LoadError::Unsupported { at, .. }
            | LoadError::Template { at, .. }
            | LoadError::DuplicateFlow { at, .. }
            | LoadError::UnknownFlow { at, .. }
            | LoadError::Cycle { at, .. }
            | LoadError::UnknownVariable { at, .. }
            | LoadError::SelfDependency { at, .. }
            | LoadError::ForeignFlowModule { at, .. }
            | LoadError::MissingOpener { at, .. }
            | LoadError::UnknownSpawnTarget { at, .. }
            | LoadError::NotSpawned { at, .. }
            | LoadError::BranchSum { at, .. }
            | LoadError::UniformRange { at }
            | LoadError::ReservedAction { at, .. }
            | LoadError::MaxDatagramOnTcp { at }
            | LoadError::TlsUpgradeOnUdp { at }
            | LoadError::DatagramLimit { at, .. }
            | LoadError::LastWithoutExpect { at }
            | LoadError::ForeignModule { at, .. }
            | LoadError::UnknownModuleFunction { at, .. } => at,
        })
    }

    fn edit(mut self, f: impl FnOnce(&mut Location)) -> Self {
        if let Some(at) = self.location_mut() {
            f(at);
        }
        self
    }

    /// Sets the file of the location.
    pub(crate) fn in_file(self, path: &Path) -> Self {
        self.edit(|at| at.file = Some(path.to_path_buf()))
    }

    /// Sets the flow of the location, keeping an index or id already known.
    pub(crate) fn in_flow(self, index: Option<usize>, id: Option<&str>) -> Self {
        self.edit(|at| {
            let f = at.flow.get_or_insert(FlowRef {
                index: None,
                id: None,
            });
            f.index = f.index.or(index);
            if f.id.is_none() {
                f.id = id.map(String::from);
            }
        })
    }

    /// Sets the side of the location.
    pub(crate) fn in_side(self, side: Side) -> Self {
        self.edit(|at| at.side = Some(side))
    }

    /// Prepends an action step, because errors travel outward.
    pub(crate) fn in_action(self, index: usize, kind: Option<&str>) -> Self {
        self.edit(|at| {
            at.actions.insert(
                0,
                Step::Action {
                    index,
                    kind: kind.map(String::from),
                },
            )
        })
    }

    /// Prepends a branch arm step.
    pub(crate) fn in_arm(self, index: usize) -> Self {
        self.edit(|at| at.actions.insert(0, Step::Arm(index)))
    }

    /// Sets the field of the location unless an inner scope set one.
    pub(crate) fn at_field(self, field: &str) -> Self {
        self.edit(|at| {
            if at.field.is_none() {
                at.field = Some(field.to_string());
            }
        })
    }

    /// Sets the variable of the location.
    pub(crate) fn in_var(self, name: &str) -> Self {
        self.edit(|at| at.var = Some(name.to_string()))
    }
}

impl From<TemplateError> for LoadError {
    fn from(error: TemplateError) -> Self {
        LoadError::Template {
            at: Box::default(),
            error,
        }
    }
}

impl From<CheckError> for LoadError {
    fn from(error: CheckError) -> Self {
        let at = Box::default();
        match error {
            CheckError::Template(error) => LoadError::Template { at, error },
            CheckError::LastWithoutExpect => LoadError::LastWithoutExpect { at },
            CheckError::ForeignModule { module, owner } => {
                LoadError::ForeignModule { at, module, owner }
            }
            CheckError::UnknownModuleFunction { module, function } => {
                LoadError::UnknownModuleFunction {
                    at,
                    module,
                    function,
                }
            }
        }
    }
}

pub(crate) fn missing(field: &'static str) -> LoadError {
    LoadError::Missing {
        at: Box::default(),
        field,
    }
}

pub(crate) fn invalid(reason: impl Into<String>) -> LoadError {
    LoadError::Invalid {
        at: Box::default(),
        reason: reason.into(),
    }
}

pub(crate) fn unsupported(feature: impl Into<String>) -> LoadError {
    LoadError::Unsupported {
        at: Box::default(),
        feature: feature.into(),
    }
}
