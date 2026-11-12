// SPDX-License-Identifier: GPL-3.0-only
//! Scenario loader and flow program compiler.
//!
//! Loading runs the semantic rules of `tools/contracts/validate.py` first, in
//! [`crate::check`], and then compiles each side of a flow into a flat list
//! of operations with jumps, so the interpreter runs loops and branches
//! without recursion or per call allocation. [`Scenario::fold_constants`] folds the run variables into the
//! templates, and [`Scenario::dump`] prints the compiled programs.

use std::collections::{HashMap, HashSet};
use std::fmt::Write;
use std::path::Path;
use std::time::Duration;

use serde_yaml::Value as Yaml;

use crate::check::check_document;
use crate::error::{invalid, missing, unsupported};
pub use crate::error::{LoadError, Warning};
use crate::location::Side;
use crate::manifest::ModuleContext;
use crate::template::{Profiles, Template, TemplateError, Value, Vars};

/// Timeout of an `expect` that names none.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_millis(10_000);

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

/// The part that a side plays in the handshake of a `tls_upgrade`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TlsRole {
    /// Start the handshake with a ClientHello.
    Client,
    /// Wait for the ClientHello and answer it.
    Server,
}

/// A compiled `tls_upgrade` action.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TlsUpgrade {
    /// The name of the TLS profile of the run, which the engine resolves.
    pub profile: String,
    /// The role in the handshake, or `None` for the role that matches the
    /// side: the client side connects, and the server side accepts.
    pub role: Option<TlsRole>,
    /// The server name to verify and send as SNI, for the client role. `None`
    /// leaves the choice to the engine.
    pub server_name: Option<String>,
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
    /// Start the same side of the named flow, which declares `spawned: true`.
    /// Only [`Scenario::parse_instance`] compiles this op.
    Spawn(String),
    /// Wrap the transport of the running session in TLS, so every later
    /// action runs over TLS.
    UpgradeTls(TlsUpgrade),
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
    /// Flow ids that must complete before this flow starts.
    pub after: Vec<String>,
    /// True when the flow starts only through a `spawn` action.
    pub spawned: bool,
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
    /// Reads, checks, and compiles a scenario file without a module
    /// context, so flow modules and module calls stay unchecked.
    pub fn load(path: &Path) -> Result<Self, LoadError> {
        Self::load_with(path, None)
    }

    /// Reads, checks, and compiles a scenario file that the module of
    /// `modules` ships. Module calls are checked against its function
    /// catalogs.
    pub fn load_with(path: &Path, modules: Option<&ModuleContext>) -> Result<Self, LoadError> {
        let text = read(path)?;
        Self::parse_with(&text, modules).map_err(|e| e.in_file(path))
    }

    /// Checks and compiles scenario text without a module context.
    pub fn parse(text: &str) -> Result<Self, LoadError> {
        Self::parse_with(text, None)
    }

    /// Checks and compiles scenario text in a module context. The result
    /// runs on this engine version, so features that it does not run yet,
    /// such as `spawn`, fail with [`LoadError::Unsupported`].
    pub fn parse_with(text: &str, modules: Option<&ModuleContext>) -> Result<Self, LoadError> {
        build(text, modules, Mode::Session, &mut Vec::new())
    }

    /// Reads, checks, and compiles a scenario file for a scenario instance
    /// runner, as [`Scenario::parse_instance`] describes.
    pub fn load_instance(path: &Path, modules: Option<&ModuleContext>) -> Result<Self, LoadError> {
        let text = read(path)?;
        Self::parse_instance(&text, modules).map_err(|e| e.in_file(path))
    }

    /// Checks and compiles scenario text for a runner that executes whole
    /// scenario instances. Unlike [`Scenario::parse_with`], it accepts flows
    /// with `spawned: true` and compiles the `spawn` action to [`Op::Spawn`],
    /// so a runner that executes flows one by one never sees them.
    pub fn parse_instance(text: &str, modules: Option<&ModuleContext>) -> Result<Self, LoadError> {
        build(text, modules, Mode::Instance, &mut Vec::new())
    }

    /// Applies the contract rules to scenario text and returns the warnings.
    ///
    /// The verdict equals the verdict of `tools/contracts/validate.py` for
    /// the semantic rules, so features that the engine does not run yet pass
    /// when the contract allows them.
    pub fn check(text: &str, modules: Option<&ModuleContext>) -> Result<Vec<Warning>, LoadError> {
        let mut warnings = Vec::new();
        build(text, modules, Mode::Check, &mut warnings)?;
        Ok(warnings)
    }

    /// Reads a scenario file and applies [`Scenario::check`].
    pub fn check_file(
        path: &Path,
        modules: Option<&ModuleContext>,
    ) -> Result<Vec<Warning>, LoadError> {
        let text = read(path)?;
        Self::check(&text, modules).map_err(|e| e.in_file(path))
    }

    /// Replaces the values of declared variables.
    pub fn override_vars(&mut self, overrides: &[(String, Value)]) -> Result<(), LoadError> {
        for (k, v) in overrides {
            if !self.vars.contains_key(k) {
                return Err(LoadError::UnknownVariable {
                    at: Box::default(),
                    name: k.clone(),
                });
            }
            self.vars.insert(k.clone(), v.clone());
        }
        Ok(())
    }

    /// Returns the names that any `set` action of the scenario binds. Their
    /// values differ between sessions.
    pub fn set_targets(&self) -> HashSet<&str> {
        let mut out = HashSet::new();
        for p in self.programs() {
            for op in &p.ops {
                if let Op::Set(binds) = op {
                    out.extend(binds.iter().map(|(k, _)| k.as_str()));
                }
            }
        }
        out
    }

    fn programs(&self) -> impl Iterator<Item = &Program> {
        self.flows
            .iter()
            .flat_map(|f| std::iter::once(&f.client).chain(f.server.as_ref()))
    }

    /// Folds the scenario variables into every template.
    ///
    /// The engine calls this once per run, after [`Scenario::override_vars`].
    /// Variables that a `set` action binds stay references, so the folded
    /// scenario renders the same bytes as the original in every session.
    pub fn fold_constants(&mut self) -> Result<(), LoadError> {
        let targets = self.set_targets();
        let constants: Vars = self
            .vars
            .iter()
            .filter(|(k, _)| !targets.contains(k.as_str()))
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        for (i, flow) in self.flows.iter_mut().enumerate() {
            let id = flow.id.clone();
            let at = |e: LoadError, side| e.in_side(side).in_flow(Some(i), Some(&id));
            fold_flow(flow, &|t| t.fold_vars(&constants), at)?;
        }
        Ok(())
    }

    /// Returns a human readable dump of the compiled scenario for golden
    /// tests. Variables appear in name order and ops with their indices.
    pub fn dump(&self) -> String {
        let mut out = String::new();
        let _ = writeln!(out, "scenario {}", self.name);
        let mut names: Vec<&String> = self.vars.keys().collect();
        names.sort();
        for name in names {
            let _ = writeln!(out, "  var {name} = {:?}", self.vars[name]);
        }
        for flow in &self.flows {
            let l4 = match flow.l4 {
                L4::Tcp => "tcp",
                L4::Udp => "udp",
            };
            let _ = writeln!(out, "flow {} module {} l4 {l4}", flow.id, flow.module);
            dump_program(&mut out, "client", &flow.client);
            if let Some(server) = &flow.server {
                dump_program(&mut out, "server", server);
            }
        }
        out
    }
}

/// Rewrites every template of a flow with `f`, naming the side of a failure.
fn fold_flow(
    flow: &mut Flow,
    f: &Fold<'_>,
    at: impl Fn(LoadError, Side) -> LoadError,
) -> Result<(), LoadError> {
    fold_program(&mut flow.client, f).map_err(|e| at(e, Side::Client))?;
    if let Some(server) = &mut flow.server {
        fold_program(server, f).map_err(|e| at(e, Side::Server))?;
    }
    Ok(())
}

/// A rewrite of one template.
type Fold<'a> = dyn Fn(&Template) -> Result<Template, TemplateError> + 'a;

fn fold_template(t: &mut Template, f: &Fold<'_>) -> Result<(), LoadError> {
    *t = f(t)?;
    Ok(())
}

fn fold_program(p: &mut Program, constants: &Fold<'_>) -> Result<(), LoadError> {
    fold_template(&mut p.port, constants)?;
    if let Some(host) = &mut p.host {
        fold_template(host, constants)?;
    }
    for (i, op) in p.ops.iter_mut().enumerate() {
        let r = match op {
            Op::Send(t)
            | Op::Expect {
                rule: StopRule::Bytes(t) | StopRule::MinBytes(t),
                ..
            }
            | Op::Delay(Delay::Fixed(t))
            | Op::LoopStart { count: t, .. } => fold_template(t, constants),
            Op::Set(binds) => binds
                .iter_mut()
                .try_for_each(|(_, t)| fold_template(t, constants)),
            _ => Ok(()),
        };
        r.map_err(|e| e.at_field(&format!("op {i}")))?;
    }
    Ok(())
}

const DUMP_OP: usize = 6;

fn dump_program(out: &mut String, side: &str, p: &Program) {
    let _ = writeln!(out, "  {side}");
    let _ = writeln!(out, "    port");
    p.port.write_dump(out, DUMP_OP);
    if let Some(host) = &p.host {
        let _ = writeln!(out, "    host");
        host.write_dump(out, DUMP_OP);
    }
    for (i, op) in p.ops.iter().enumerate() {
        let _ = write!(out, "    {i} ");
        match op {
            Op::Send(t) => {
                let _ = writeln!(out, "send");
                t.write_dump(out, DUMP_OP);
            }
            Op::Expect { rule, timeout } => {
                let ms = timeout.as_millis();
                match rule {
                    StopRule::Pattern(p) => {
                        let _ = writeln!(
                            out,
                            "expect pattern \"{}\" timeout {ms} ms",
                            p.escape_ascii()
                        );
                    }
                    StopRule::Bytes(t) | StopRule::MinBytes(t) => {
                        let kind = if matches!(rule, StopRule::Bytes(_)) {
                            "bytes"
                        } else {
                            "min_bytes"
                        };
                        let _ = writeln!(out, "expect {kind} timeout {ms} ms");
                        t.write_dump(out, DUMP_OP);
                    }
                }
            }
            Op::Delay(Delay::Fixed(t)) => {
                let _ = writeln!(out, "delay fixed");
                t.write_dump(out, DUMP_OP);
            }
            Op::Delay(d) => {
                let _ = writeln!(out, "delay {d:?}");
            }
            Op::Set(binds) => {
                let _ = writeln!(out, "set");
                for (name, t) in binds {
                    let _ = writeln!(out, "      {name} =");
                    t.write_dump(out, DUMP_OP + 2);
                }
            }
            Op::LoopStart { count, end } => {
                let _ = writeln!(out, "loop end {end}");
                count.write_dump(out, DUMP_OP);
            }
            Op::LoopEnd { start } => {
                let _ = writeln!(out, "loop_end start {start}");
            }
            Op::Branch { arms } => {
                let arms: Vec<String> = arms.iter().map(|(p, at)| format!("{p}->{at}")).collect();
                let _ = writeln!(out, "branch {}", arms.join(" "));
            }
            Op::Jump(to) => {
                let _ = writeln!(out, "jump {to}");
            }
            Op::Close(mode) => {
                let _ = writeln!(out, "close {}", format!("{mode:?}").to_lowercase());
            }
            Op::Spawn(flow) => {
                let _ = writeln!(out, "spawn {flow}");
            }
            Op::UpgradeTls(u) => {
                let _ = writeln!(
                    out,
                    "tls_upgrade profile {} role {:?} server_name {:?}",
                    u.profile, u.role, u.server_name
                );
            }
        }
    }
}

fn read(path: &Path) -> Result<String, LoadError> {
    std::fs::read_to_string(path).map_err(|source| LoadError::Read {
        path: path.to_path_buf(),
        source,
    })
}

/// What a compiled scenario is for.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    /// Only the verdict counts, so every contract feature passes.
    Check,
    /// Flows run one by one, so `spawn` and spawned flows fail.
    Session,
    /// A scenario instance runner runs the flows, so `spawn` compiles.
    Instance,
}

impl Mode {
    /// Reject the features that this engine version does not run yet.
    fn runnable(self) -> bool {
        self != Mode::Check
    }
}

/// Reads the `profiles` root. The compiler folds each reference into the
/// text that it renders, so a compiled scenario carries no profile lookup.
fn read_profiles(doc: &Yaml) -> Result<Profiles, LoadError> {
    let mut profiles = Profiles::new();
    let Some(Yaml::Mapping(m)) = doc.get("profiles") else {
        return Ok(profiles);
    };
    for (name, entries) in m {
        let name = name
            .as_str()
            .ok_or_else(|| invalid("profile names must be strings"))?;
        let entries = entries
            .as_mapping()
            .filter(|e| !e.is_empty())
            .ok_or_else(|| invalid(format!("profile '{name}' needs at least one entry")))?;
        let mut list = Vec::with_capacity(entries.len());
        for (key, value) in entries {
            let key = key
                .as_str()
                .filter(|k| valid_entry_name(k))
                .ok_or_else(|| invalid(format!("bad entry name in profile '{name}'")))?;
            let value = Value::from_yaml(value)
                .map_err(|e| LoadError::from(e).at_field(&format!("profiles.{name}.{key}")))?;
            if matches!(&value, Value::Str(s) if s.contains(['\r', '\n'])) {
                return Err(invalid(format!(
                    "entry '{key}' of profile '{name}' holds a line break"
                )));
            }
            list.push((key.to_string(), value));
        }
        profiles.insert(name, list);
    }
    Ok(profiles)
}

/// The entry name pattern `[A-Za-z0-9][A-Za-z0-9_-]*` of the schema.
fn valid_entry_name(name: &str) -> bool {
    let mut chars = name.chars();
    chars.next().is_some_and(|c| c.is_ascii_alphanumeric())
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

/// Checks and compiles scenario text. In [`Mode::Check`] the compiler
/// accepts the contract features that this engine does not run yet and
/// leaves them out of the programs, so only the verdict counts.
fn build(
    text: &str,
    modules: Option<&ModuleContext>,
    mode: Mode,
    warnings: &mut Vec<Warning>,
) -> Result<Scenario, LoadError> {
    let doc: Yaml = serde_yaml::from_str(text).map_err(|e| LoadError::Syntax {
        at: Box::default(),
        message: e.to_string(),
    })?;
    check_document(&doc, modules, warnings)?;
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
            let v = Value::from_yaml(v).map_err(|e| LoadError::from(e).in_var(k))?;
            vars.insert(k.to_string(), v);
        }
    }
    let flows_y = doc
        .get("flows")
        .and_then(Yaml::as_sequence)
        .ok_or_else(|| missing("flows"))?;
    let mut flows = Vec::new();
    let mut after: HashMap<String, Vec<String>> = HashMap::new();
    for (i, f) in flows_y.iter().enumerate() {
        let (flow, deps) = compile_flow(f, mode).map_err(|e| e.in_flow(Some(i), None))?;
        if after.contains_key(&flow.id) {
            return Err(LoadError::DuplicateFlow {
                at: Box::default(),
                id: flow.id,
            }
            .in_flow(Some(i), None)
            .at_field("id"));
        }
        after.insert(flow.id.clone(), deps);
        flows.push(flow);
    }
    let mut flows = topo_order(flows, &after)?;
    let profiles = read_profiles(&doc)?;
    if !profiles.is_empty() {
        for (i, flow) in flows.iter_mut().enumerate() {
            let id = flow.id.clone();
            let at = |e: LoadError, side| e.in_side(side).in_flow(Some(i), Some(&id));
            fold_flow(flow, &|t| t.fold_profiles(&profiles), at)?;
        }
    }
    Ok(Scenario {
        name: name.to_string(),
        vars,
        flows,
    })
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
                    at: Box::default(),
                    flow: f.id.clone(),
                    dependency: d.clone(),
                }
                .in_flow(None, Some(&f.id))
                .at_field("after"));
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
                at: Box::default(),
                flows: next.into_iter().map(|f| f.id).collect(),
            });
        }
        pending = next;
    }
    Ok(ordered)
}

fn compile_flow(f: &Yaml, mode: Mode) -> Result<(Flow, Vec<String>), LoadError> {
    let id = f
        .get("id")
        .and_then(Yaml::as_str)
        .ok_or_else(|| missing("id"))?
        .to_string();
    compile_flow_body(f, id.clone(), mode).map_err(|e| e.in_flow(None, Some(&id)))
}

fn compile_flow_body(f: &Yaml, id: String, mode: Mode) -> Result<(Flow, Vec<String>), LoadError> {
    let runnable = mode.runnable();
    let module = f
        .get("module")
        .and_then(Yaml::as_str)
        .unwrap_or("")
        .to_string();
    let l4 = match f.get("l4").and_then(Yaml::as_str) {
        None | Some("tcp") => L4::Tcp,
        Some("udp") => L4::Udp,
        Some(other) => return Err(invalid(format!("unknown l4 '{other}'")).at_field("l4")),
    };
    let transport = match f.get("transport") {
        None => None,
        Some(_) if l4 == L4::Udp => {
            return Err(LoadError::TlsUpgradeOnUdp { at: Box::default() }.at_field("transport"));
        }
        Some(body) => Some(transport(body).map_err(|e| e.at_field("transport"))?),
    };
    let spawned = f.get("spawned").and_then(Yaml::as_bool).unwrap_or(false);
    if mode == Mode::Session && spawned {
        return Err(unsupported("a spawned flow").at_field("spawned"));
    }
    let deps = match f.get("after") {
        None => vec![],
        Some(Yaml::String(s)) => vec![s.clone()],
        Some(Yaml::Sequence(s)) => s
            .iter()
            .filter_map(|x| x.as_str().map(String::from))
            .collect(),
        Some(other) => return Err(invalid(format!("bad 'after' {other:?}")).at_field("after")),
    };
    let c = Compiler {
        runnable,
        spawn: mode == Mode::Instance,
        transport,
    };
    let client_y = f
        .get("client")
        .and_then(Yaml::as_sequence)
        .ok_or_else(|| missing("client"))?;
    let client = c
        .side(client_y, Side::Client)
        .map_err(|e| e.in_side(Side::Client))?;
    let server = match f.get("server").and_then(Yaml::as_sequence) {
        Some(s) => Some(
            c.side(s, Side::Server)
                .map_err(|e| e.in_side(Side::Server))?,
        ),
        None => None,
    };
    Ok((
        Flow {
            id,
            module,
            l4,
            after: deps.clone(),
            spawned,
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

fn num(v: &Yaml, key: &'static str) -> Result<f64, LoadError> {
    v.get(key)
        .and_then(Yaml::as_f64)
        .ok_or_else(|| missing(key))
}

/// Compiles the body of a `tls_upgrade` action.
fn tls_upgrade(body: &Yaml) -> Result<TlsUpgrade, LoadError> {
    let map = body
        .as_mapping()
        .ok_or_else(|| invalid("tls_upgrade takes a mapping"))?;
    for key in map.keys() {
        match key.as_str() {
            Some("profile" | "role" | "server_name") => {}
            other => return Err(invalid(format!("unknown tls_upgrade field {other:?}"))),
        }
    }
    let text = |field: &str| -> Result<Option<String>, LoadError> {
        match body.get(field) {
            None => Ok(None),
            Some(Yaml::String(s)) if !s.is_empty() => Ok(Some(s.clone())),
            Some(_) => Err(invalid(format!("{field} takes a non empty string")).at_field(field)),
        }
    };
    let profile = text("profile")?.ok_or_else(|| missing("profile"))?;
    let role = match text("role")?.as_deref() {
        None => None,
        Some("client") => Some(TlsRole::Client),
        Some("server") => Some(TlsRole::Server),
        Some(r) => return Err(invalid(format!("unknown tls role '{r}'")).at_field("role")),
    };
    Ok(TlsUpgrade {
        profile,
        role,
        server_name: text("server_name")?,
    })
}

/// Compiles the flow keyword `transport`, which stands for a `tls_upgrade`
/// with the role of the side. Only the module `tls` exists, and its profile is
/// required.
fn transport(body: &Yaml) -> Result<TlsUpgrade, LoadError> {
    let map = body
        .as_mapping()
        .ok_or_else(|| invalid("transport takes a mapping"))?;
    for key in map.keys() {
        match key.as_str() {
            Some("module" | "profile" | "server_name") => {}
            other => return Err(invalid(format!("unknown transport field {other:?}"))),
        }
    }
    match body.get("module").and_then(Yaml::as_str) {
        Some("tls") => {}
        Some(other) => {
            return Err(invalid(format!("unknown transport module '{other}'")).at_field("module"))
        }
        None => return Err(missing("module")),
    }
    // The fields are the profile and the server name of a tls_upgrade, and the
    // role stays unset, so each side plays its own.
    tls_upgrade(&Yaml::Mapping(
        map.iter()
            .filter(|(k, _)| k.as_str() != Some("module"))
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect(),
    ))
}

fn template(v: &Yaml, field: &str) -> Result<Template, LoadError> {
    let t = match v {
        Yaml::String(s) => Template::compile(s),
        other => Template::from_yaml_count(other),
    };
    t.map_err(|e| LoadError::from(e).at_field(field))
}

/// Compiles the action lists of one flow.
struct Compiler {
    /// Reject the features that this engine version does not run yet.
    runnable: bool,
    /// Compile `spawn` to [`Op::Spawn`] for a scenario instance runner.
    spawn: bool,
    /// The upgrade that the flow keyword `transport` stands for, as the
    /// client side runs it. Both sides of the flow start with it.
    transport: Option<TlsUpgrade>,
}

impl Compiler {
    fn side(&self, actions: &[Yaml], side: Side) -> Result<Program, LoadError> {
        let opener = side.opener();
        let first = actions.first().ok_or_else(|| invalid("no actions"))?;
        let (kind, body) = single_key(first).map_err(|e| e.in_action(0, None))?;
        if kind != opener {
            return Err(LoadError::MissingOpener {
                at: Box::default(),
                side,
                found: kind.to_string(),
            }
            .in_action(0, None));
        }
        let open = |e: LoadError| e.in_action(0, Some(opener));
        let port = template(
            body.get("port").ok_or_else(|| open(missing("port")))?,
            "port",
        )
        .map_err(open)?;
        let host = match body.get("host").and_then(Yaml::as_str) {
            Some(h) => {
                Some(Template::compile(h).map_err(|e| open(LoadError::from(e).at_field("host")))?)
            }
            None => None,
        };
        let mut ops = Vec::new();
        if let Some(upgrade) = &self.transport {
            // The flow keyword `transport` is sugar for a tls_upgrade right
            // after the opener. The server name is for the client handshake.
            let mut upgrade = upgrade.clone();
            if side == Side::Server {
                upgrade.server_name = None;
            }
            ops.push(Op::UpgradeTls(upgrade));
        }
        self.actions(actions, 1, &mut ops)?;
        Ok(Program { port, host, ops })
    }

    /// Compiles `actions[from..]`, naming each action by its index in the
    /// list.
    fn actions(&self, actions: &[Yaml], from: usize, ops: &mut Vec<Op>) -> Result<(), LoadError> {
        for (i, action) in actions.iter().enumerate().skip(from) {
            let kind = action
                .as_mapping()
                .filter(|m| m.len() == 1)
                .and_then(|m| m.keys().next())
                .and_then(Yaml::as_str);
            self.action(action, ops).map_err(|e| e.in_action(i, kind))?;
        }
        Ok(())
    }

    fn action(&self, action: &Yaml, ops: &mut Vec<Op>) -> Result<(), LoadError> {
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
                        return Err(invalid("expect pattern is empty").at_field("pattern"));
                    }
                    StopRule::Pattern(p.as_bytes().to_vec())
                } else if let Some(b) = body.get("bytes") {
                    StopRule::Bytes(template(b, "bytes")?)
                } else if let Some(b) = body.get("min_bytes") {
                    StopRule::MinBytes(template(b, "min_bytes")?)
                } else {
                    return Err(invalid("expect needs pattern, bytes, or min_bytes"));
                };
                ops.push(Op::Expect { rule, timeout });
            }
            "delay" => {
                let d = match body.get("dist").and_then(Yaml::as_str) {
                    None => Delay::Fixed(template(
                        body.get("ms").ok_or_else(|| missing("ms"))?,
                        "ms",
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
                        Yaml::String(s) => Template::compile(s),
                        other => Value::from_yaml(other).map(|v| Template::literal(v.to_string())),
                    }
                    .map_err(|e| LoadError::from(e).at_field(k))?;
                    binds.push((k.to_string(), v));
                }
                ops.push(Op::Set(binds));
            }
            "loop" => {
                let count = template(body.get("count").ok_or_else(|| missing("count"))?, "count")?;
                let body_y = body
                    .get("do")
                    .and_then(Yaml::as_sequence)
                    .ok_or_else(|| missing("do"))?;
                let start = ops.len();
                ops.push(Op::LoopStart {
                    count,
                    end: usize::MAX,
                });
                self.actions(body_y, 0, ops)?;
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
                    cumulative += num(arm, "p").map_err(|e| e.in_arm(i))?;
                    arms.push((cumulative, ops.len()));
                    let do_y = arm
                        .get("do")
                        .and_then(Yaml::as_sequence)
                        .ok_or_else(|| missing("do").in_arm(i))?;
                    self.actions(do_y, 0, ops).map_err(|e| e.in_arm(i))?;
                    jumps.push(ops.len());
                    ops.push(Op::Jump(usize::MAX));
                }
                if (cumulative - 1.0_f64).abs() > 1e-6 {
                    return Err(LoadError::BranchSum {
                        at: Box::default(),
                        sum: cumulative,
                    });
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
            "tls_upgrade" => ops.push(Op::UpgradeTls(tls_upgrade(body)?)),
            "connect" | "accept" => return Err(invalid(format!("'{kind}' may only open a side"))),
            "spawn" if self.spawn => {
                let flow = body
                    .get("flow")
                    .and_then(Yaml::as_str)
                    .ok_or_else(|| missing("flow"))?;
                ops.push(Op::Spawn(flow.to_string()));
            }
            "spawn" if self.runnable => return Err(unsupported("the 'spawn' action")),
            // The contract check accepts this action, and the programs leave
            // it out until the instance runner runs it.
            "spawn" => {
                body.get("flow")
                    .and_then(Yaml::as_str)
                    .ok_or_else(|| missing("flow"))?;
            }
            // The bytes are a literal segment that no template parser reads, so
            // `{{` in them stays as written. The contract check admits the
            // action only in strike and fuzzer modules.
            "raw" => {
                let hex = body
                    .get("hex")
                    .and_then(Yaml::as_str)
                    .ok_or_else(|| missing("hex"))?;
                let bytes = crate::template::decode_hex(hex)
                    .map_err(|e| LoadError::from(e).at_field("hex"))?;
                ops.push(Op::Send(Template::literal(bytes)));
            }
            other => return Err(invalid(format!("unknown action '{other}'"))),
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::{Manifest, ModuleSet};

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
    fn compiles_tls_upgrade() {
        let text = r#"
scenario: t
flows:
  - id: a
    module: m
    client:
      - connect: { port: 25 }
      - tls_upgrade: { profile: strict, server_name: mail.example.test }
      - tls_upgrade: { profile: p, role: server }
"#;
        let s = Scenario::parse(text).unwrap();
        let ops = &s.flows[0].client.ops;
        assert_eq!(
            ops[0],
            Op::UpgradeTls(TlsUpgrade {
                profile: "strict".into(),
                role: None,
                server_name: Some("mail.example.test".into()),
            })
        );
        assert!(matches!(&ops[1], Op::UpgradeTls(u) if u.role == Some(TlsRole::Server)));
        for bad in [
            "{ role: client }",
            "{ profile: p, role: both }",
            "{ profile: p, extra: 1 }",
            "p",
        ] {
            let text = text.replace("{ profile: strict, server_name: mail.example.test }", bad);
            assert!(Scenario::parse(&text).is_err(), "{bad}");
        }
    }

    #[test]
    fn transport_desugars_to_a_leading_tls_upgrade_on_both_sides() {
        let text = r#"
scenario: t
flows:
  - id: a
    module: m
    transport: { module: tls, profile: strict, server_name: www.example.test }
    client:
      - connect: { port: 443 }
      - loop: { count: 2, do: [ { send: "x" } ] }
    server:
      - accept: { port: 443 }
      - send: "y"
  - id: b
    module: m
    client: [ { connect: { port: 80 } }, { send: "z" } ]
"#;
        let s = Scenario::parse(text).unwrap();
        let client = &s.flows[0].client.ops;
        let server = &s.flows[0].server.as_ref().unwrap().ops;
        assert_eq!(
            client[0],
            Op::UpgradeTls(TlsUpgrade {
                profile: "strict".into(),
                role: None,
                server_name: Some("www.example.test".into()),
            })
        );
        assert_eq!(
            server[0],
            Op::UpgradeTls(TlsUpgrade {
                profile: "strict".into(),
                role: None,
                server_name: None,
            })
        );
        // The jump targets of the body account for the extra leading op.
        assert!(matches!(&client[1], Op::LoopStart { end: 3, .. }));
        assert!(matches!(&s.flows[1].client.ops[0], Op::Send(_)));
        // Explicit upgrades follow the sugar.
        let both = text.replace(
            "client:\n      - connect: { port: 443 }",
            "client:\n      - connect: { port: 443 }\n      - tls_upgrade: { profile: inner }",
        );
        let s = Scenario::parse(&both).unwrap();
        assert!(matches!(&s.flows[0].client.ops[1], Op::UpgradeTls(u) if u.profile == "inner"));
    }

    #[test]
    fn transport_rejects_bad_bodies_and_udp() {
        let text = r#"
scenario: t
flows:
  - id: a
    module: m
    transport: { module: tls, profile: p }
    client: [ { connect: { port: 1 } } ]
"#;
        for bad in [
            "{ profile: p }",
            "{ module: tls }",
            "{ module: quic, profile: p }",
            "{ module: tls, profile: '' }",
            "{ module: tls, profile: p, role: client }",
            "tls",
        ] {
            let text = text.replace("{ module: tls, profile: p }", bad);
            assert!(Scenario::parse(&text).is_err(), "{bad}");
        }
        let udp = text.replace("module: m\n", "module: m\n    l4: udp\n");
        let err = Scenario::parse(&udp).unwrap_err();
        assert_eq!(err.kind(), "TlsUpgradeOnUdp");
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
    fn instance_mode_compiles_spawn_and_keeps_flow_relations() {
        let text = r#"
scenario: t
flows:
  - id: ctl
    module: m
    client:
      - connect: { port: 21 }
      - spawn: { flow: data }
      - close: fin
  - { id: data, module: m, spawned: true, client: [ { connect: { port: 20 } } ] }
  - { id: tail, module: m, after: [ctl, data], client: [ { connect: { port: 1 } } ] }
"#;
        assert!(matches!(
            Scenario::parse(text),
            Err(LoadError::Unsupported { .. })
        ));
        let s = Scenario::parse_instance(text, None).unwrap();
        assert_eq!(s.flows[0].client.ops[0], Op::Spawn("data".into()));
        assert!(s.flows[1].spawned && !s.flows[0].spawned);
        assert_eq!(s.flows[2].after, ["ctl", "data"]);
        assert!(s.dump().contains("0 spawn data"));
        let raw = "scenario: t\nflows:\n  - { id: a, module: strike, client: [ { connect: { port: 1 } }, { raw: { hex: '00' } } ] }\n";
        assert!(matches!(
            Scenario::parse_instance(raw, None),
            Err(LoadError::ReservedAction { .. })
        ));
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

    /// A context for a module of the given type, which has no functions.
    fn context_of(module_type: &str) -> ModuleContext {
        let manifest = Manifest::parse(&format!("name: m\ntype: {module_type}\n")).unwrap();
        let mut set = ModuleSet::default();
        set.insert(manifest);
        set.context("m").unwrap()
    }

    /// The bytes of the static sends of the client program of the first flow.
    fn sent(s: &Scenario) -> Vec<Vec<u8>> {
        s.flows[0]
            .client
            .ops
            .iter()
            .filter_map(|op| match op {
                Op::Send(t) => Some(t.as_static().expect("static send").to_vec()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn raw_compiles_to_a_literal_send_in_strike_and_fuzzer_modules() {
        // The bytes hold `{{` and `}}`, a NUL, and a byte that is no UTF-8, and
        // none of them goes through the template parser.
        let text = "scenario: t\nflows:\n  - id: a\n    module: m\n    client:\n      - connect: { port: 1 }\n      - raw: { hex: '7b7b2276617273227d7d00ff' }\n      - raw: { hex: 0D0A }\n";
        for module_type in ["strike", "fuzzer"] {
            let ctx = context_of(module_type);
            Scenario::check(text, Some(&ctx)).unwrap();
            for s in [
                Scenario::parse_with(text, Some(&ctx)).unwrap(),
                Scenario::parse_instance(text, Some(&ctx)).unwrap(),
            ] {
                assert_eq!(
                    sent(&s),
                    [b"{{\"vars\"}}\x00\xff".to_vec(), b"\r\n".to_vec()],
                    "{module_type}"
                );
            }
        }
    }

    #[test]
    fn raw_inside_a_loop_and_a_branch_arm_compiles() {
        let text = "scenario: t\nflows:\n  - id: a\n    module: m\n    client:\n      - connect: { port: 1 }\n      - loop: { count: 3, do: [ { raw: { hex: '41' } } ] }\n      - branch:\n          - { p: 1.0, do: [ { raw: { hex: '4243' } } ] }\n";
        let s = Scenario::parse_with(text, Some(&context_of("strike"))).unwrap();
        assert_eq!(sent(&s), [b"A".to_vec(), b"BC".to_vec()]);
    }

    #[test]
    fn raw_fails_to_load_outside_strike_and_fuzzer_modules() {
        let text = "scenario: t\nflows:\n  - id: a\n    module: m\n    client:\n      - connect: { port: 1 }\n      - loop: { count: 2, do: [ { raw: { hex: '00' } } ] }\n";
        for module_type in ["app", "transport", "host", "content", "tunnel", "evasion"] {
            let ctx = context_of(module_type);
            for result in [
                Scenario::parse_with(text, Some(&ctx)),
                Scenario::parse_instance(text, Some(&ctx)),
            ] {
                let err = result.expect_err(module_type);
                assert!(matches!(err, LoadError::ReservedAction { .. }), "{err:?}");
                assert_eq!(err.kind(), "ReservedAction");
                assert!(err.to_string().contains("'raw' is reserved"), "{err}");
            }
        }
        // Without a module context no module type admits the action.
        assert!(matches!(
            Scenario::parse(text),
            Err(LoadError::ReservedAction { .. })
        ));
    }

    #[test]
    fn raw_with_bad_hex_names_the_field() {
        for hex in ["'0'", "'zz'"] {
            let text = format!("scenario: t\nflows:\n  - id: a\n    module: m\n    client:\n      - connect: {{ port: 1 }}\n      - raw: {{ hex: {hex} }}\n");
            let ctx = context_of("strike");
            let err = Scenario::parse_with(&text, Some(&ctx)).expect_err(hex);
            assert!(err.to_string().contains("hex"), "{err}");
        }
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
    fn fold_constants_keeps_variables_that_set_binds() {
        let text = r#"
scenario: t
vars: { token: abc, port: 9000 }
flows:
  - id: a
    module: m
    client:
      - connect: { port: "{{ vars.port }}" }
      - send: "{{ vars.token }}"
  - id: b
    module: m
    after: a
    client:
      - connect: { port: "{{ vars.port }}" }
      - set: { token: "{{ rand_int(1, 9) }}" }
"#;
        let mut s = Scenario::parse(text).unwrap();
        s.override_vars(&[("port".into(), Value::Int(9100))])
            .unwrap();
        s.fold_constants().unwrap();
        let a = &s.flows[0].client;
        assert_eq!(a.port.as_static(), Some(&b"9100"[..]));
        assert!(matches!(&a.ops[0], Op::Send(t) if t.variables() == ["token"]));
        assert!(s.dump().contains("set\n      token =\n"));
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
