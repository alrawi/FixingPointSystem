// SPDX-License-Identifier: GPL-3.0-only
//! Scenario loader and flow program compiler.
//!
//! Each side of a flow compiles into a flat list of operations with jumps, so
//! the interpreter runs loops and branches without recursion or per call
//! allocation.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde_yaml::Value as Yaml;

use crate::template::{Template, TemplateError, Value, Vars};

/// Timeout of an `expect` that names none.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_millis(10_000);

/// Errors that loading or compiling a scenario reports.
///
/// The `at` field names where the error occurred, from the outermost scope
/// inward, for example `web_get.yaml: flow 'page': client: action 2`.
#[derive(Debug, thiserror::Error)]
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
        at: String,
        /// Message of the YAML parser.
        message: String,
    },
    /// A required field is absent or has the wrong type.
    #[error("{}missing '{field}'", prefix(.at))]
    Missing {
        /// Location of the error.
        at: String,
        /// The field name.
        field: &'static str,
    },
    /// A field holds a value that the grammar rejects.
    #[error("{}{reason}", prefix(.at))]
    Invalid {
        /// Location of the error.
        at: String,
        /// What is wrong.
        reason: String,
    },
    /// The scenario uses a feature that this engine version does not run yet.
    #[error("{}{feature} is not supported by this engine version", prefix(.at))]
    Unsupported {
        /// Location of the error.
        at: String,
        /// The feature.
        feature: String,
    },
    /// A template failed to compile.
    #[error("{}{error}", prefix(.at))]
    Template {
        /// Location of the error.
        at: String,
        /// The template error.
        error: TemplateError,
    },
    /// Two flows share one id.
    #[error("{}duplicate flow id '{id}'", prefix(.at))]
    DuplicateFlow {
        /// Location of the error.
        at: String,
        /// The repeated id.
        id: String,
    },
    /// An `after` entry names a flow that does not exist.
    #[error("{}flow '{flow}' depends on unknown flow '{dependency}'", prefix(.at))]
    UnknownFlow {
        /// Location of the error.
        at: String,
        /// The dependent flow.
        flow: String,
        /// The missing dependency.
        dependency: String,
    },
    /// The `after` relation has a cycle.
    #[error("{}dependency cycle among flows {flows:?}", prefix(.at))]
    Cycle {
        /// Location of the error.
        at: String,
        /// The flows that cannot be ordered.
        flows: Vec<String>,
    },
    /// An override names a variable that the scenario does not declare.
    #[error("{}scenario has no variable '{name}'", prefix(.at))]
    UnknownVariable {
        /// Location of the error.
        at: String,
        /// The variable name.
        name: String,
    },
}

fn prefix(at: &str) -> String {
    if at.is_empty() {
        String::new()
    } else {
        format!("{at}: ")
    }
}

impl LoadError {
    /// Prepends an enclosing scope to the location of the error.
    fn within(mut self, scope: impl std::fmt::Display) -> Self {
        let at = match &mut self {
            LoadError::Read { .. } => return self,
            LoadError::Syntax { at, .. }
            | LoadError::Missing { at, .. }
            | LoadError::Invalid { at, .. }
            | LoadError::Unsupported { at, .. }
            | LoadError::Template { at, .. }
            | LoadError::DuplicateFlow { at, .. }
            | LoadError::UnknownFlow { at, .. }
            | LoadError::Cycle { at, .. }
            | LoadError::UnknownVariable { at, .. } => at,
        };
        *at = if at.is_empty() {
            scope.to_string()
        } else {
            format!("{scope}: {at}")
        };
        self
    }
}

impl From<TemplateError> for LoadError {
    fn from(error: TemplateError) -> Self {
        LoadError::Template {
            at: String::new(),
            error,
        }
    }
}

fn missing(field: &'static str) -> LoadError {
    LoadError::Missing {
        at: String::new(),
        field,
    }
}

fn invalid(reason: impl Into<String>) -> LoadError {
    LoadError::Invalid {
        at: String::new(),
        reason: reason.into(),
    }
}

fn unsupported(feature: impl Into<String>) -> LoadError {
    LoadError::Unsupported {
        at: String::new(),
        feature: feature.into(),
    }
}

/// The condition that ends an `expect`.
#[derive(Clone, Debug, PartialEq)]
pub enum StopRule {
    /// Consume through the first occurrence of the bytes.
    Pattern(Vec<u8>),
    /// Consume exactly this many bytes.
    Bytes(Template),
    /// Wait until at least this many bytes arrived, then consume what arrived.
    MinBytes(Template),
}

/// A think time between actions.
#[derive(Clone, Debug, PartialEq)]
pub enum Delay {
    /// A fixed number of milliseconds.
    Fixed(Template),
    /// An exponential distribution with the given mean.
    Exponential {
        /// Mean in milliseconds.
        mean_ms: f64,
    },
    /// A uniform distribution over `[min_ms, max_ms)`.
    Uniform {
        /// Lower bound in milliseconds.
        min_ms: f64,
        /// Upper bound in milliseconds.
        max_ms: f64,
    },
    /// A log normal distribution.
    LogNormal {
        /// Mean of the underlying normal distribution.
        mu: f64,
        /// Standard deviation of the underlying normal distribution.
        sigma: f64,
        /// Milliseconds per sampled unit, 1000 for seconds or 1 for milliseconds.
        unit_ms: f64,
    },
    /// A Pareto distribution.
    Pareto {
        /// Shape parameter.
        alpha: f64,
        /// Scale, the smallest value, in milliseconds.
        scale_ms: f64,
    },
}

/// How a `close` action ends the session.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CloseMode {
    /// Shut down the write side and drain until the peer closes.
    Fin,
    /// Abort with a reset.
    Rst,
    /// Shut down the write side and continue.
    Half,
}

/// One operation of a compiled program.
#[derive(Clone, Debug, PartialEq)]
pub enum Op {
    /// Render and write a template.
    Send(Template),
    /// Read until the stop rule holds or the timeout passes.
    Expect {
        /// The stop rule.
        rule: StopRule,
        /// The timeout.
        timeout: Duration,
    },
    /// Wait for a sampled think time.
    Delay(Delay),
    /// Bind session variables to rendered templates.
    Set(Vec<(String, Template)>),
    /// Push a counter, and jump to `end + 1` when the count is zero.
    LoopStart {
        /// The iteration count.
        count: Template,
        /// Index of the matching [`Op::LoopEnd`].
        end: usize,
    },
    /// Decrement the top counter, and jump to `start + 1` while it stays positive.
    LoopEnd {
        /// Index of the matching [`Op::LoopStart`].
        start: usize,
    },
    /// Pick an arm by cumulative probability and jump to its first op.
    Branch {
        /// Pairs of cumulative probability and first op index.
        arms: Vec<(f64, usize)>,
    },
    /// Continue at the given op index.
    Jump(usize),
    /// End the session.
    Close(CloseMode),
}

/// One side of a flow, which is how the session opens and then the body.
#[derive(Clone, Debug)]
pub struct Program {
    /// Port of `connect` or `accept`.
    pub port: Template,
    /// Host of `connect`, when given.
    pub host: Option<Template>,
    /// The body.
    pub ops: Vec<Op>,
}

/// The transport protocol of a flow.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum L4 {
    /// A TCP byte stream.
    Tcp,
    /// UDP datagrams.
    Udp,
}

/// One flow of a scenario.
#[derive(Clone, Debug)]
pub struct Flow {
    /// Flow id.
    pub id: String,
    /// Module that owns the flow.
    pub module: String,
    /// Transport protocol.
    pub l4: L4,
    /// The client side.
    pub client: Program,
    /// The server side, when the scenario emulates the server.
    pub server: Option<Program>,
}

/// A compiled scenario.
#[derive(Clone, Debug)]
pub struct Scenario {
    /// Scenario name.
    pub name: String,
    /// Declared variables and their values.
    pub vars: Vars,
    /// Flows in execution order, a topological order of `after`.
    pub flows: Vec<Flow>,
}

impl Scenario {
    /// Reads and compiles a scenario file.
    pub fn load(path: &Path) -> Result<Self, LoadError> {
        let text = std::fs::read_to_string(path).map_err(|source| LoadError::Read {
            path: path.to_path_buf(),
            source,
        })?;
        Self::parse(&text).map_err(|e| e.within(path.display()))
    }

    /// Compiles scenario text.
    pub fn parse(text: &str) -> Result<Self, LoadError> {
        let doc: Yaml = serde_yaml::from_str(text).map_err(|e| LoadError::Syntax {
            at: String::new(),
            message: e.to_string(),
        })?;
        let name = doc
            .get("scenario")
            .and_then(Yaml::as_str)
            .ok_or_else(|| missing("scenario"))?;
        let mut vars = Vars::new();
        if let Some(Yaml::Mapping(m)) = doc.get("vars") {
            for (k, v) in m {
                let k = k
                    .as_str()
                    .ok_or_else(|| invalid("variable names must be strings"))?;
                let v = Value::from_yaml(v).map_err(|e| LoadError::from(e).within(k))?;
                vars.insert(k.to_string(), v);
            }
        }
        let flows_y = doc
            .get("flows")
            .and_then(Yaml::as_sequence)
            .ok_or_else(|| missing("flows"))?;
        let mut flows = Vec::new();
        let mut after: HashMap<String, Vec<String>> = HashMap::new();
        for f in flows_y {
            let (flow, deps) = compile_flow(f)?;
            if after.contains_key(&flow.id) {
                return Err(LoadError::DuplicateFlow {
                    at: String::new(),
                    id: flow.id,
                });
            }
            after.insert(flow.id.clone(), deps);
            flows.push(flow);
        }
        let flows = topo_order(flows, &after)?;
        Ok(Scenario {
            name: name.to_string(),
            vars,
            flows,
        })
    }

    /// Replaces the values of declared variables.
    pub fn override_vars(&mut self, overrides: &[(String, Value)]) -> Result<(), LoadError> {
        for (k, v) in overrides {
            if !self.vars.contains_key(k) {
                return Err(LoadError::UnknownVariable {
                    at: format!("scenario '{}'", self.name),
                    name: k.clone(),
                });
            }
            self.vars.insert(k.clone(), v.clone());
        }
        Ok(())
    }
}

fn topo_order(
    flows: Vec<Flow>,
    after: &HashMap<String, Vec<String>>,
) -> Result<Vec<Flow>, LoadError> {
    let mut done: HashSet<String> = HashSet::new();
    let mut pending = flows;
    let mut ordered = Vec::new();
    while !pending.is_empty() {
        let before = pending.len();
        let mut next = Vec::new();
        for f in pending {
            let deps = after.get(&f.id).map(Vec::as_slice).unwrap_or_default();
            if let Some(d) = deps.iter().find(|d| !after.contains_key(*d)) {
                return Err(LoadError::UnknownFlow {
                    at: String::new(),
                    flow: f.id.clone(),
                    dependency: d.clone(),
                });
            }
            if deps.iter().all(|d| done.contains(d)) {
                done.insert(f.id.clone());
                ordered.push(f);
            } else {
                next.push(f);
            }
        }
        if next.len() == before {
            return Err(LoadError::Cycle {
                at: String::new(),
                flows: next.into_iter().map(|f| f.id).collect(),
            });
        }
        pending = next;
    }
    Ok(ordered)
}

fn compile_flow(f: &Yaml) -> Result<(Flow, Vec<String>), LoadError> {
    let id = f
        .get("id")
        .and_then(Yaml::as_str)
        .ok_or_else(|| missing("id").within("flow"))?
        .to_string();
    compile_flow_body(f, id.clone()).map_err(|e| e.within(format!("flow '{id}'")))
}

fn compile_flow_body(f: &Yaml, id: String) -> Result<(Flow, Vec<String>), LoadError> {
    let module = f
        .get("module")
        .and_then(Yaml::as_str)
        .unwrap_or("")
        .to_string();
    let l4 = match f.get("l4").and_then(Yaml::as_str) {
        None | Some("tcp") => L4::Tcp,
        Some("udp") => L4::Udp,
        Some(other) => return Err(invalid(format!("unknown l4 '{other}'"))),
    };
    if f.get("transport").is_some() {
        return Err(unsupported("a transport module"));
    }
    if f.get("spawned").and_then(Yaml::as_bool).unwrap_or(false) {
        return Err(unsupported("a spawned flow"));
    }
    let deps = match f.get("after") {
        None => vec![],
        Some(Yaml::String(s)) => vec![s.clone()],
        Some(Yaml::Sequence(s)) => s
            .iter()
            .filter_map(|x| x.as_str().map(String::from))
            .collect(),
        Some(other) => return Err(invalid(format!("bad 'after' {other:?}"))),
    };
    let client_y = f
        .get("client")
        .and_then(Yaml::as_sequence)
        .ok_or_else(|| missing("client"))?;
    let client = compile_side(client_y, "connect").map_err(|e| e.within("client"))?;
    let server = match f.get("server").and_then(Yaml::as_sequence) {
        Some(s) => Some(compile_side(s, "accept").map_err(|e| e.within("server"))?),
        None => None,
    };
    Ok((
        Flow {
            id,
            module,
            l4,
            client,
            server,
        },
        deps,
    ))
}

fn single_key(action: &Yaml) -> Result<(&str, &Yaml), LoadError> {
    let m = action
        .as_mapping()
        .ok_or_else(|| invalid("an action must be a mapping"))?;
    let mut entries = m.iter();
    match (entries.next(), entries.next()) {
        (Some((k, v)), None) => Ok((
            k.as_str()
                .ok_or_else(|| invalid("action keys must be strings"))?,
            v,
        )),
        _ => Err(invalid(format!(
            "an action must have exactly one key, got {}",
            m.len()
        ))),
    }
}

fn compile_side(actions: &[Yaml], opener: &'static str) -> Result<Program, LoadError> {
    let first = actions.first().ok_or_else(|| invalid("no actions"))?;
    let (kind, body) = single_key(first)?;
    if kind != opener {
        return Err(invalid(format!(
            "actions must start with '{opener}', found '{kind}'"
        )));
    }
    let port = Template::from_yaml_count(body.get("port").ok_or_else(|| missing("port"))?)
        .map_err(|e| LoadError::from(e).within(opener))?;
    let host = match body.get("host").and_then(Yaml::as_str) {
        Some(h) => Some(Template::compile(h).map_err(|e| LoadError::from(e).within(opener))?),
        None => None,
    };
    let mut ops = Vec::new();
    compile_actions(&actions[1..], &mut ops, 2)?;
    Ok(Program { port, host, ops })
}

fn num(v: &Yaml, key: &'static str) -> Result<f64, LoadError> {
    v.get(key)
        .and_then(Yaml::as_f64)
        .ok_or_else(|| missing(key))
}

/// Compiles a list of actions. `first` is the one based number of the first
/// action in its list, which keeps error locations aligned with the file.
fn compile_actions(actions: &[Yaml], ops: &mut Vec<Op>, first: usize) -> Result<(), LoadError> {
    for (i, action) in actions.iter().enumerate() {
        compile_action(action, ops).map_err(|e| e.within(format!("action {}", first + i)))?;
    }
    Ok(())
}

fn compile_action(action: &Yaml, ops: &mut Vec<Op>) -> Result<(), LoadError> {
    let (kind, body) = single_key(action)?;
    match kind {
        "send" => ops.push(Op::Send(Template::compile(
            body.as_str()
                .ok_or_else(|| invalid("send takes a string"))?,
        )?)),
        "expect" => {
            let timeout = body
                .get("timeout_ms")
                .and_then(Yaml::as_u64)
                .map(Duration::from_millis)
                .unwrap_or(DEFAULT_TIMEOUT);
            let rule = if let Some(p) = body.get("pattern").and_then(Yaml::as_str) {
                if p.is_empty() {
                    return Err(invalid("expect pattern is empty"));
                }
                StopRule::Pattern(p.as_bytes().to_vec())
            } else if let Some(b) = body.get("bytes") {
                StopRule::Bytes(Template::from_yaml_count(b)?)
            } else if let Some(b) = body.get("min_bytes") {
                StopRule::MinBytes(Template::from_yaml_count(b)?)
            } else {
                return Err(invalid("expect needs pattern, bytes, or min_bytes"));
            };
            ops.push(Op::Expect { rule, timeout });
        }
        "delay" => {
            let d = match body.get("dist").and_then(Yaml::as_str) {
                None => Delay::Fixed(Template::from_yaml_count(
                    body.get("ms").ok_or_else(|| missing("ms"))?,
                )?),
                Some("exponential") => Delay::Exponential {
                    mean_ms: num(body, "mean_ms")?,
                },
                Some("uniform") => Delay::Uniform {
                    min_ms: num(body, "min_ms")?,
                    max_ms: num(body, "max_ms")?,
                },
                Some("lognormal") => Delay::LogNormal {
                    mu: num(body, "mu")?,
                    sigma: num(body, "sigma")?,
                    unit_ms: match body.get("unit").and_then(Yaml::as_str).unwrap_or("s") {
                        "s" => 1000.0,
                        "ms" => 1.0,
                        u => return Err(invalid(format!("unknown delay unit '{u}'"))),
                    },
                },
                Some("pareto") => Delay::Pareto {
                    alpha: num(body, "alpha")?,
                    scale_ms: num(body, "scale_ms")?,
                },
                Some(d) => return Err(invalid(format!("unknown delay distribution '{d}'"))),
            };
            ops.push(Op::Delay(d));
        }
        "set" => {
            let m = body
                .as_mapping()
                .ok_or_else(|| invalid("set takes a mapping"))?;
            let mut binds = Vec::new();
            for (k, v) in m {
                let k = k
                    .as_str()
                    .ok_or_else(|| invalid("set keys must be strings"))?;
                let v = match v {
                    Yaml::String(s) => Template::compile(s)?,
                    other => Template::literal(Value::from_yaml(other)?.to_string()),
                };
                binds.push((k.to_string(), v));
            }
            ops.push(Op::Set(binds));
        }
        "loop" => {
            let count =
                Template::from_yaml_count(body.get("count").ok_or_else(|| missing("count"))?)?;
            let body_y = body
                .get("do")
                .and_then(Yaml::as_sequence)
                .ok_or_else(|| missing("do"))?;
            let start = ops.len();
            ops.push(Op::LoopStart {
                count,
                end: usize::MAX,
            });
            compile_actions(body_y, ops, 1).map_err(|e| e.within("loop"))?;
            let end = ops.len();
            ops.push(Op::LoopEnd { start });
            if let Some(Op::LoopStart { end: e, .. }) = ops.get_mut(start) {
                *e = end;
            }
        }
        "branch" => {
            let arms_y = body
                .as_sequence()
                .ok_or_else(|| invalid("branch takes a list"))?;
            let at = ops.len();
            ops.push(Op::Branch { arms: vec![] });
            let mut arms = Vec::new();
            let mut jumps = Vec::new();
            let mut cumulative = 0.0;
            for (i, arm) in arms_y.iter().enumerate() {
                let scope = format!("branch arm {}", i + 1);
                cumulative += num(arm, "p").map_err(|e| e.within(&scope))?;
                arms.push((cumulative, ops.len()));
                let do_y = arm
                    .get("do")
                    .and_then(Yaml::as_sequence)
                    .ok_or_else(|| missing("do").within(&scope))?;
                compile_actions(do_y, ops, 1).map_err(|e| e.within(&scope))?;
                jumps.push(ops.len());
                ops.push(Op::Jump(usize::MAX));
            }
            if (cumulative - 1.0_f64).abs() > 1e-6 {
                return Err(invalid(format!(
                    "branch probabilities sum to {cumulative}, not 1"
                )));
            }
            let end = ops.len();
            for j in jumps {
                ops[j] = Op::Jump(end);
            }
            ops[at] = Op::Branch { arms };
        }
        "close" => ops.push(Op::Close(match body.as_str() {
            Some("fin") => CloseMode::Fin,
            Some("rst") => CloseMode::Rst,
            Some("half") => CloseMode::Half,
            other => return Err(invalid(format!("unknown close mode {other:?}"))),
        })),
        "connect" | "accept" => return Err(invalid(format!("'{kind}' may only open a side"))),
        "spawn" | "raw" => return Err(unsupported(format!("the '{kind}' action"))),
        other => return Err(invalid(format!("unknown action '{other}'"))),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const LOOPED: &str = r#"
scenario: t
vars: { n: 2, port: 9000 }
flows:
  - id: a
    module: m
    client:
      - connect: { port: "{{ vars.port }}" }
      - loop:
          count: "{{ vars.n }}"
          do:
            - send: "x"
            - branch:
                - { p: 0.25, do: [ { delay: { ms: 1 } } ] }
                - { p: 0.75, do: [ { send: "y" } ] }
      - close: fin
"#;

    #[test]
    fn compiles_loops_and_branches_to_jumps() {
        let s = Scenario::parse(LOOPED).unwrap();
        let ops = &s.flows[0].client.ops;
        assert!(matches!(ops[0], Op::LoopStart { end: 7, .. }));
        assert!(matches!(&ops[2], Op::Branch { arms } if arms == &vec![(0.25, 3), (1.0, 5)]));
        assert_eq!(ops[4], Op::Jump(7));
        assert_eq!(ops[6], Op::Jump(7));
        assert_eq!(ops[7], Op::LoopEnd { start: 0 });
        assert_eq!(ops[8], Op::Close(CloseMode::Fin));
    }

    #[test]
    fn orders_flows_by_after() {
        let text = r#"
scenario: t
flows:
  - { id: b, module: m, after: a, client: [ { connect: { port: 1 } } ] }
  - { id: a, module: m, client: [ { connect: { port: 1 } } ] }
"#;
        let s = Scenario::parse(text).unwrap();
        assert_eq!(
            s.flows.iter().map(|f| f.id.as_str()).collect::<Vec<_>>(),
            ["a", "b"]
        );
    }

    #[test]
    fn rejects_out_of_scope_and_bad_input() {
        let cycle = "scenario: t\nflows:\n  - { id: a, module: m, after: b, client: [ { connect: { port: 1 } } ] }\n  - { id: b, module: m, after: a, client: [ { connect: { port: 1 } } ] }\n";
        assert!(matches!(
            Scenario::parse(cycle),
            Err(LoadError::Cycle { .. })
        ));
        let spawned = "scenario: t\nflows:\n  - { id: a, module: m, spawned: true, client: [ { connect: { port: 1 } } ] }\n";
        assert!(matches!(
            Scenario::parse(spawned),
            Err(LoadError::Unsupported { .. })
        ));
        let no_connect = "scenario: t\nflows:\n  - { id: a, module: m, client: [ { send: x } ] }\n";
        assert!(Scenario::parse(no_connect).is_err());
        let bad_branch = "scenario: t\nflows:\n  - { id: a, module: m, client: [ { connect: { port: 1 } }, { branch: [ { p: 0.5, do: [ { send: x } ] }, { p: 0.4, do: [ { send: y } ] } ] } ] }\n";
        assert!(Scenario::parse(bad_branch).is_err());
    }

    #[test]
    fn records_udp_flows() {
        let udp = "scenario: t\nflows:\n  - { id: a, module: m, l4: udp, client: [ { connect: { port: 1 } } ] }\n";
        assert_eq!(Scenario::parse(udp).unwrap().flows[0].l4, L4::Udp);
        let sctp = "scenario: t\nflows:\n  - { id: a, module: m, l4: sctp, client: [ { connect: { port: 1 } } ] }\n";
        assert!(matches!(
            Scenario::parse(sctp),
            Err(LoadError::Invalid { .. })
        ));
    }

    #[test]
    fn error_names_flow_side_and_action() {
        let text = "scenario: t\nflows:\n  - { id: a, module: m, client: [ { connect: { port: 1 } }, { send: x }, { loop: { count: 2, do: [ { send: \"{{ nope() }}\" } ] } } ] }\n";
        let err = Scenario::parse(text).unwrap_err();
        assert_eq!(
            err.to_string(),
            "flow 'a': client: action 3: loop: action 1: unknown function 'nope'"
        );
    }

    #[test]
    fn override_vars_rejects_undeclared_names() {
        let mut s = Scenario::parse(LOOPED).unwrap();
        s.override_vars(&[("n".into(), Value::Int(9))]).unwrap();
        assert_eq!(s.vars["n"], Value::Int(9));
        assert!(matches!(
            s.override_vars(&[("other".into(), Value::Int(1))]),
            Err(LoadError::UnknownVariable { .. })
        ));
    }
}
