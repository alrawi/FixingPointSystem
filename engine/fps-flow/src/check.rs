// SPDX-License-Identifier: GPL-3.0-only
//! Semantic rules of a scenario document.
//!
//! The rules and their order follow `check_scenario` in
//! `tools/contracts/validate.py`, so both validators give every scenario the
//! same verdict. The checks walk the YAML document and tolerate structure
//! that the compiler rejects later, as the Python checks tolerate what its
//! JSON Schema step rejects. The first error stops the check.

use std::collections::{HashMap, HashSet};

use serde_yaml::Value as Yaml;

use crate::error::{LoadError, Warning};
use crate::function::ValueType;
use crate::location::{FlowRef, Location, Side, Step};
use crate::manifest::ModuleContext;
use crate::template::{CheckScope, CheckWarning, Template};

/// Actions that only some module types may use, as in
/// `RESERVED_ACTION_TYPES` of the Python validator.
const RESERVED_ACTIONS: &[(&str, &[&str])] = &[("raw", &["fuzzer", "strike"])];

/// Upper bound of the UDP payload over IPv4, and the default `max_datagram`.
pub const MAX_DATAGRAM: u64 = 65507;

/// Keys of an action body that hold templates, as in `_template_fields`.
const TEMPLATE_KEYS: &[&str] = &["host", "port", "bytes", "min_bytes", "count", "ms"];

/// The action key and body of a mapping with exactly one entry.
fn single(action: &Yaml) -> Option<(&str, &Yaml)> {
    let m = action.as_mapping()?;
    if m.len() != 1 {
        return None;
    }
    let (k, v) = m.iter().next()?;
    Some((k.as_str()?, v))
}

fn seq(v: Option<&Yaml>) -> &[Yaml] {
    v.and_then(Yaml::as_sequence)
        .map(Vec::as_slice)
        .unwrap_or_default()
}

/// The type of a variable value as YAML loads it, as in `value_type`.
fn value_type(v: &Yaml) -> ValueType {
    match v {
        Yaml::Bool(_) => ValueType::Boolean,
        Yaml::Number(n) if n.is_i64() || n.is_u64() => ValueType::Int,
        Yaml::Number(_) => ValueType::Float,
        _ => ValueType::Text,
    }
}

fn int(v: Option<&Yaml>) -> Option<i64> {
    match v {
        Some(Yaml::Number(n)) => n.as_i64(),
        _ => None,
    }
}

fn number(v: Option<&Yaml>) -> f64 {
    match v {
        Some(Yaml::Number(n)) => n.as_f64().unwrap_or(0.0),
        _ => 0.0,
    }
}

/// Visits every action of a list, loop bodies and branch arms included, with
/// its step path.
fn walk<'y>(actions: &'y [Yaml], path: &mut Vec<Step>, f: &mut dyn FnMut(&[Step], &str, &'y Yaml)) {
    for (i, action) in actions.iter().enumerate() {
        let Some((kind, body)) = single(action) else {
            continue;
        };
        path.push(Step::Action {
            index: i,
            kind: Some(kind.to_string()),
        });
        f(path, kind, body);
        if kind == "loop" {
            walk(seq(body.get("do")), path, f);
        }
        if let Some(arms) = body.as_sequence().filter(|_| kind == "branch") {
            for (j, arm) in arms.iter().enumerate() {
                if arm.is_mapping() {
                    path.push(Step::Arm(j));
                    walk(seq(arm.get("do")), path, f);
                    path.pop();
                }
            }
        }
        path.pop();
    }
}

struct Ctx<'a> {
    vars: HashMap<String, ValueType>,
    profiles: HashSet<String>,
    modules: Option<&'a ModuleContext>,
    warnings: &'a mut Vec<Warning>,
}

/// Where one flow side sits, for building locations.
#[derive(Clone)]
struct SideAt<'y> {
    index: usize,
    id: Option<&'y str>,
    side: Side,
}

impl SideAt<'_> {
    fn at(&self, path: &[Step], field: Option<&str>) -> Box<Location> {
        Box::new(self.loc(path, field))
    }

    fn loc(&self, path: &[Step], field: Option<&str>) -> Location {
        Location {
            flow: Some(FlowRef {
                index: Some(self.index),
                id: self.id.map(String::from),
            }),
            side: Some(self.side),
            actions: path.to_vec(),
            field: field.map(String::from),
            ..Location::default()
        }
    }
}

fn flow_at(index: usize, id: Option<&str>, field: &str) -> Box<Location> {
    Box::new(Location {
        flow: Some(FlowRef {
            index: Some(index),
            id: id.map(String::from),
        }),
        field: Some(field.to_string()),
        ..Location::default()
    })
}

fn with_at(e: LoadError, at: Location) -> LoadError {
    let mut e = e;
    if let Some(slot) = e.location_mut() {
        *slot = at;
    }
    e
}

/// Checks the semantic rules of a parsed scenario document. `modules` is the
/// call scope of the module that ships the scenario; without it, flow
/// modules and module calls stay unchecked, as in the Python validator.
pub(crate) fn check_document(
    doc: &Yaml,
    modules: Option<&ModuleContext>,
    warnings: &mut Vec<Warning>,
) -> Result<(), LoadError> {
    let flows = seq(doc.get("flows"));
    let mut vars: HashMap<String, ValueType> = HashMap::new();
    if let Some(m) = doc.get("vars").and_then(Yaml::as_mapping) {
        for (k, v) in m {
            if let Some(k) = k.as_str() {
                vars.insert(k.to_string(), value_type(v));
            }
        }
    }
    // Variables that a set action binds are visible to every template. Their
    // type depends on the rendered text, so they are any unless declared.
    for flow in flows {
        for side in [Side::Client, Side::Server] {
            walk(
                seq(flow.get(side.key())),
                &mut Vec::new(),
                &mut |_, kind, body| {
                    if let (true, Some(m)) = (kind == "set", body.as_mapping()) {
                        for k in m.keys().filter_map(Yaml::as_str) {
                            vars.entry(k.to_string()).or_insert(ValueType::Any);
                        }
                    }
                },
            );
        }
    }

    let ids: Vec<Option<&str>> = flows
        .iter()
        .map(|f| f.get("id").and_then(Yaml::as_str))
        .collect();
    let mut seen = HashSet::new();
    for (i, id) in ids.iter().enumerate() {
        if !seen.insert(*id) {
            return Err(LoadError::DuplicateFlow {
                at: flow_at(i, *id, "id"),
                id: id.unwrap_or_default().to_string(),
            });
        }
    }
    let by_id: HashMap<&str, &Yaml> = ids
        .iter()
        .zip(flows)
        .filter_map(|(id, f)| id.map(|id| (id, f)))
        .collect();

    let profiles = doc
        .get("profiles")
        .and_then(Yaml::as_mapping)
        .map(|m| {
            m.keys()
                .filter_map(Yaml::as_str)
                .map(String::from)
                .collect()
        })
        .unwrap_or_default();
    let mut ctx = Ctx {
        vars,
        profiles,
        modules,
        warnings,
    };
    let mut deps: Vec<(&str, Vec<&str>)> = Vec::new();
    let mut spawned_targets: HashSet<&str> = HashSet::new();
    for (i, flow) in flows.iter().enumerate() {
        let id = ids[i];
        let after: Vec<&str> = match flow.get("after") {
            Some(Yaml::String(s)) => vec![s.as_str()],
            Some(Yaml::Sequence(s)) => s.iter().filter_map(Yaml::as_str).collect(),
            _ => vec![],
        };
        for dep in &after {
            if Some(*dep) == id {
                return Err(LoadError::SelfDependency {
                    at: flow_at(i, id, "after"),
                    flow: dep.to_string(),
                });
            }
            if !by_id.contains_key(dep) {
                return Err(LoadError::UnknownFlow {
                    at: flow_at(i, id, "after"),
                    flow: id.unwrap_or_default().to_string(),
                    dependency: dep.to_string(),
                });
            }
        }
        if let Some(id) = id {
            deps.push((id, after));
        }
        if let Some(m) = modules {
            let module = flow
                .get("module")
                .and_then(Yaml::as_str)
                .unwrap_or_default();
            if !m.callable.contains_key(module) {
                return Err(LoadError::ForeignFlowModule {
                    at: flow_at(i, id, "module"),
                    module: module.to_string(),
                    owner: m.module.clone(),
                });
            }
        }
        for side in [Side::Client, Side::Server] {
            let actions = seq(flow.get(side.key()));
            if let Some(first) = actions.first() {
                let opens = first
                    .as_mapping()
                    .is_some_and(|m| m.contains_key(side.opener()));
                if !opens {
                    let found = single(first).map(|(k, _)| k).unwrap_or("?").to_string();
                    let at = SideAt { index: i, id, side };
                    return Err(LoadError::MissingOpener {
                        at: at.at(
                            &[Step::Action {
                                index: 0,
                                kind: None,
                            }],
                            None,
                        ),
                        side,
                        found,
                    });
                }
            }
        }
        let udp = flow.get("l4").and_then(Yaml::as_str) == Some("udp");
        if udp && flow.get("transport").is_some() {
            return Err(LoadError::TlsUpgradeOnUdp {
                at: flow_at(i, id, "transport"),
            });
        }
        for side in [Side::Client, Side::Server] {
            let at = SideAt { index: i, id, side };
            let actions = seq(flow.get(side.key()));
            let mut result = Ok(());
            walk(actions, &mut Vec::new(), &mut |path, kind, body| {
                if result.is_ok() {
                    result = check_action(&at, path, kind, body, udp, &by_id, modules);
                    if kind == "spawn" {
                        if let Some(t) = body.get("flow").and_then(Yaml::as_str) {
                            spawned_targets.insert(t);
                        }
                    }
                }
            });
            result?;
            check_templates(&mut ctx, &at, actions, &mut Vec::new(), false)?;
        }
    }

    for (i, flow) in flows.iter().enumerate() {
        let spawned = flow.get("spawned").and_then(Yaml::as_bool) == Some(true);
        if spawned && !ids[i].is_some_and(|id| spawned_targets.contains(id)) {
            ctx.warnings.push(Warning {
                at: Location {
                    flow: Some(FlowRef {
                        index: Some(i),
                        id: ids[i].map(String::from),
                    }),
                    ..Location::default()
                },
                message: format!(
                    "flow '{}' is marked spawned but no action spawns it",
                    ids[i].unwrap_or_default()
                ),
            });
        }
    }

    // Cycle detection over `after`, ignoring the dependencies reported above.
    let mut done: HashSet<&str> = HashSet::new();
    let mut pending = deps;
    while !pending.is_empty() {
        let before = pending.len();
        pending.retain(|(id, after)| {
            if after.iter().all(|d| done.contains(d)) {
                done.insert(id);
                false
            } else {
                true
            }
        });
        if pending.len() == before {
            return Err(LoadError::Cycle {
                at: Box::default(),
                flows: pending.into_iter().map(|(id, _)| id.to_string()).collect(),
            });
        }
    }
    Ok(())
}

fn check_action(
    at: &SideAt<'_>,
    path: &[Step],
    kind: &str,
    body: &Yaml,
    udp: bool,
    by_id: &HashMap<&str, &Yaml>,
    modules: Option<&ModuleContext>,
) -> Result<(), LoadError> {
    match kind {
        "spawn" if body.is_mapping() => {
            let target = body.get("flow").and_then(Yaml::as_str).unwrap_or_default();
            match by_id.get(target) {
                None => {
                    return Err(LoadError::UnknownSpawnTarget {
                        at: at.at(path, None),
                        flow: target.to_string(),
                    })
                }
                Some(f) if f.get("spawned").and_then(Yaml::as_bool) != Some(true) => {
                    return Err(LoadError::NotSpawned {
                        at: at.at(path, None),
                        flow: target.to_string(),
                    })
                }
                Some(_) => {}
            }
        }
        "branch" => {
            if let Some(arms) = body.as_sequence() {
                let sum: f64 = arms
                    .iter()
                    .filter(|a| a.is_mapping())
                    .map(|a| number(a.get("p")))
                    .sum();
                if (sum - 1.0).abs() > 1e-6 {
                    return Err(LoadError::BranchSum {
                        at: at.at(path, None),
                        sum,
                    });
                }
            }
        }
        "delay" if body.get("dist").and_then(Yaml::as_str) == Some("uniform") => {
            if number(body.get("min_ms")) > number(body.get("max_ms")) {
                return Err(LoadError::UniformRange {
                    at: at.at(path, None),
                });
            }
        }
        "expect" if body.is_mapping() => check_expect(at, path, body, udp)?,
        "tls_upgrade" if udp => {
            return Err(LoadError::TlsUpgradeOnUdp {
                at: at.at(path, None),
            })
        }
        _ => {}
    }
    for (action, types) in RESERVED_ACTIONS {
        let module_type = modules.map(|m| m.module_type.as_str());
        if kind == *action && !module_type.is_some_and(|t| types.contains(&t)) {
            return Err(LoadError::ReservedAction {
                at: at.at(path, None),
                action: kind.to_string(),
                types,
            });
        }
    }
    Ok(())
}

/// Datagram rules of `expect`. On UDP one expect consumes one datagram, so a
/// fixed size rule must fit within `max_datagram`.
fn check_expect(at: &SideAt<'_>, path: &[Step], body: &Yaml, udp: bool) -> Result<(), LoadError> {
    if !udp {
        if body.get("max_datagram").is_some() {
            return Err(LoadError::MaxDatagramOnTcp {
                at: at.at(path, Some("max_datagram")),
            });
        }
        return Ok(());
    }
    let limit = int(body.get("max_datagram")).map_or(MAX_DATAGRAM, |l| l.max(0) as u64);
    for field in ["bytes", "min_bytes"] {
        if let Some(size) = int(body.get(field)) {
            if size > 0 && size as u64 > limit {
                return Err(LoadError::DatagramLimit {
                    at: at.at(path, Some(field)),
                    field,
                    size: size as u64,
                    limit,
                });
            }
        }
    }
    if let Some(p) = body.get("pattern").and_then(Yaml::as_str) {
        if p.len() as u64 > limit {
            return Err(LoadError::DatagramLimit {
                at: at.at(path, Some("pattern")),
                field: "pattern",
                size: p.len() as u64,
                limit,
            });
        }
    }
    Ok(())
}

/// The template fields of an action, as `_template_fields` yields them.
fn template_fields<'y>(kind: &str, body: &'y Yaml) -> Vec<(Option<&'y str>, &'y str)> {
    match (kind, body) {
        ("send", Yaml::String(s)) => vec![(None, s.as_str())],
        ("connect" | "accept" | "expect" | "loop" | "delay", Yaml::Mapping(m)) => TEMPLATE_KEYS
            .iter()
            .filter_map(|k| Some((Some(*k), m.get(*k)?.as_str()?)))
            .collect(),
        ("set", Yaml::Mapping(m)) => m
            .iter()
            .filter_map(|(k, v)| Some((Some(k.as_str()?), v.as_str()?)))
            .collect(),
        _ => vec![],
    }
}

/// Checks the templates of one action list in order and returns whether an
/// expect always ran by its end. A loop body may run zero times unless its
/// count is a positive integer, and a branch guarantees an expect only when
/// every arm does. This mirrors `_check_side_templates`.
fn check_templates(
    ctx: &mut Ctx<'_>,
    at: &SideAt<'_>,
    actions: &[Yaml],
    path: &mut Vec<Step>,
    mut seen: bool,
) -> Result<bool, LoadError> {
    for (i, action) in actions.iter().enumerate() {
        let Some((kind, body)) = single(action) else {
            continue;
        };
        path.push(Step::Action {
            index: i,
            kind: Some(kind.to_string()),
        });
        for (field, text) in template_fields(kind, body) {
            let here = || at.loc(path, field);
            let t = Template::compile(text).map_err(|e| with_at(e.into(), here()))?;
            let mut found = Vec::new();
            let scope = CheckScope {
                vars: &ctx.vars,
                modules: ctx.modules,
                profiles: &ctx.profiles,
                last: seen,
            };
            t.check(scope, &mut found)
                .map_err(|e| with_at(e.into(), here()))?;
            ctx.warnings.extend(found.into_iter().map(|w| Warning {
                at: here(),
                message: match w {
                    CheckWarning::UncheckedModule { .. } => w.to_string(),
                },
            }));
        }
        match kind {
            "expect" => seen = true,
            "loop" if body.is_mapping() => {
                let inner = check_templates(ctx, at, seq(body.get("do")), path, seen)?;
                let runs = int(body.get("count")).is_some_and(|n| n >= 1);
                seen = seen || (inner && runs);
            }
            "branch" => {
                if let Some(arms) = body.as_sequence() {
                    let mut all = !arms.is_empty();
                    let mut any_arm = false;
                    for (j, arm) in arms.iter().enumerate() {
                        if !arm.is_mapping() {
                            continue;
                        }
                        any_arm = true;
                        path.push(Step::Arm(j));
                        let r = check_templates(ctx, at, seq(arm.get("do")), path, seen);
                        path.pop();
                        all &= r?;
                    }
                    seen = seen || (any_arm && all);
                }
            }
            _ => {}
        }
        path.pop();
    }
    Ok(seen)
}
