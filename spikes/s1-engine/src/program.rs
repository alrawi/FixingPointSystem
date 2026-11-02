//! Scenario loader and flow program compiler.
//!
//! Each side of a flow compiles into a flat list of operations with jumps, so
//! the interpreter runs loops and branches without recursion or per call
//! allocation.

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::time::Duration;

use anyhow::{anyhow, bail, Context, Result};
use serde_yaml::Value as Y;

use crate::template::{Template, Value, Vars};

pub const DEFAULT_TIMEOUT: Duration = Duration::from_millis(10_000);

#[derive(Clone, Debug, PartialEq)]
pub enum StopRule {
    Pattern(Vec<u8>),
    Bytes(Template),
    MinBytes(Template),
}

#[derive(Clone, Debug, PartialEq)]
pub enum Delay {
    Fixed(Template),
    Exponential { mean_ms: f64 },
    Uniform { min_ms: f64, max_ms: f64 },
    LogNormal { mu: f64, sigma: f64, unit_ms: f64 },
    Pareto { alpha: f64, scale_ms: f64 },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CloseMode {
    Fin,
    Rst,
    Half,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Op {
    Send(Template),
    Expect {
        rule: StopRule,
        timeout: Duration,
    },
    Delay(Delay),
    Set(Vec<(String, Template)>),
    /// Push a counter; jump to `end + 1` when the count is zero.
    LoopStart {
        count: Template,
        end: usize,
    },
    /// Decrement the top counter; jump to `start + 1` while it stays positive.
    LoopEnd {
        start: usize,
    },
    /// Pick an arm by cumulative probability and jump to its first op.
    Branch {
        arms: Vec<(f64, usize)>,
    },
    Jump(usize),
    Close(CloseMode),
}

/// One side of a flow: how the session opens, then the body.
#[derive(Clone, Debug)]
pub struct Program {
    pub port: Template,
    pub host: Option<Template>,
    pub ops: Vec<Op>,
}

#[derive(Clone, Debug)]
pub struct Flow {
    pub id: String,
    pub module: String,
    pub client: Program,
    pub server: Option<Program>,
}

#[derive(Clone, Debug)]
pub struct Scenario {
    pub name: String,
    pub vars: Vars,
    /// Flows in execution order (a topological order of `after`).
    pub flows: Vec<Flow>,
}

impl Scenario {
    pub fn load(path: &Path) -> Result<Self> {
        let text =
            std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        Self::parse(&text).with_context(|| format!("compiling {}", path.display()))
    }

    pub fn parse(text: &str) -> Result<Self> {
        let doc: Y = serde_yaml::from_str(text)?;
        let name = doc
            .get("scenario")
            .and_then(Y::as_str)
            .ok_or_else(|| anyhow!("missing 'scenario'"))?;
        let mut vars = Vars::new();
        if let Some(Y::Mapping(m)) = doc.get("vars") {
            for (k, v) in m {
                let k = k
                    .as_str()
                    .ok_or_else(|| anyhow!("variable names must be strings"))?;
                vars.insert(k.to_string(), Value::from_yaml(v)?);
            }
        }
        let flows_y = doc
            .get("flows")
            .and_then(Y::as_sequence)
            .ok_or_else(|| anyhow!("missing 'flows'"))?;
        let mut flows = Vec::new();
        let mut after: HashMap<String, Vec<String>> = HashMap::new();
        for f in flows_y {
            let (flow, deps) = compile_flow(f)?;
            if after.contains_key(&flow.id) {
                bail!("duplicate flow id '{}'", flow.id);
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

    pub fn override_vars(&mut self, overrides: &[(String, Value)]) -> Result<()> {
        for (k, v) in overrides {
            if !self.vars.contains_key(k) {
                bail!("scenario '{}' has no variable '{k}'", self.name);
            }
            self.vars.insert(k.clone(), v.clone());
        }
        Ok(())
    }
}

fn topo_order(flows: Vec<Flow>, after: &HashMap<String, Vec<String>>) -> Result<Vec<Flow>> {
    let mut done: HashSet<String> = HashSet::new();
    let mut pending = flows;
    let mut ordered = Vec::new();
    while !pending.is_empty() {
        let before = pending.len();
        let mut next = Vec::new();
        for f in pending {
            let deps = &after[&f.id];
            for d in deps {
                if !after.contains_key(d) {
                    bail!("flow '{}' depends on unknown flow '{d}'", f.id);
                }
            }
            if deps.iter().all(|d| done.contains(d)) {
                done.insert(f.id.clone());
                ordered.push(f);
            } else {
                next.push(f);
            }
        }
        if next.len() == before {
            bail!(
                "dependency cycle among flows {:?}",
                next.iter().map(|f| &f.id).collect::<Vec<_>>()
            );
        }
        pending = next;
    }
    Ok(ordered)
}

fn compile_flow(f: &Y) -> Result<(Flow, Vec<String>)> {
    let id = f
        .get("id")
        .and_then(Y::as_str)
        .ok_or_else(|| anyhow!("flow without 'id'"))?
        .to_string();
    let ctx = || format!("flow '{id}'");
    let module = f
        .get("module")
        .and_then(Y::as_str)
        .unwrap_or("")
        .to_string();
    if let Some(l4) = f.get("l4").and_then(Y::as_str) {
        if l4 != "tcp" {
            bail!(
                "{}: l4 '{l4}' is outside spike S1, which runs TCP only",
                ctx()
            );
        }
    }
    if f.get("transport").is_some() {
        bail!("{}: transport modules are outside spike S1", ctx());
    }
    if f.get("spawned").and_then(Y::as_bool).unwrap_or(false) {
        bail!("{}: spawned flows are outside spike S1", ctx());
    }
    let deps = match f.get("after") {
        None => vec![],
        Some(Y::String(s)) => vec![s.clone()],
        Some(Y::Sequence(s)) => s
            .iter()
            .filter_map(|x| x.as_str().map(String::from))
            .collect(),
        Some(other) => bail!("{}: bad 'after' {other:?}", ctx()),
    };
    let client_y = f
        .get("client")
        .and_then(Y::as_sequence)
        .ok_or_else(|| anyhow!("{}: missing client", ctx()))?;
    let client = compile_side(client_y, "connect").with_context(|| format!("{}: client", ctx()))?;
    let server = match f.get("server").and_then(Y::as_sequence) {
        Some(s) => Some(compile_side(s, "accept").with_context(|| format!("{}: server", ctx()))?),
        None => None,
    };
    Ok((
        Flow {
            id,
            module,
            client,
            server,
        },
        deps,
    ))
}

fn single_key(action: &Y) -> Result<(&str, &Y)> {
    let m = action
        .as_mapping()
        .ok_or_else(|| anyhow!("an action must be a mapping"))?;
    if m.len() != 1 {
        bail!("an action must have exactly one key, got {}", m.len());
    }
    let (k, v) = m.iter().next().unwrap();
    Ok((
        k.as_str()
            .ok_or_else(|| anyhow!("action keys must be strings"))?,
        v,
    ))
}

fn compile_side(actions: &[Y], opener: &str) -> Result<Program> {
    let first = actions.first().ok_or_else(|| anyhow!("no actions"))?;
    let (kind, body) = single_key(first)?;
    if kind != opener {
        bail!("actions must start with '{opener}', found '{kind}'");
    }
    let port = Template::from_yaml_count(
        body.get("port")
            .ok_or_else(|| anyhow!("{opener} needs a port"))?,
    )?;
    let host = match body.get("host").and_then(Y::as_str) {
        Some(h) => Some(Template::compile(h)?),
        None => None,
    };
    let mut ops = Vec::new();
    compile_actions(&actions[1..], &mut ops)?;
    Ok(Program { port, host, ops })
}

fn num(v: &Y, key: &str) -> Result<f64> {
    v.get(key)
        .and_then(Y::as_f64)
        .ok_or_else(|| anyhow!("missing number '{key}'"))
}

fn compile_actions(actions: &[Y], ops: &mut Vec<Op>) -> Result<()> {
    for (i, action) in actions.iter().enumerate() {
        compile_action(action, ops).with_context(|| format!("action {}", i + 1))?;
    }
    Ok(())
}

fn compile_action(action: &Y, ops: &mut Vec<Op>) -> Result<()> {
    let (kind, body) = single_key(action)?;
    match kind {
        "send" => ops.push(Op::Send(Template::compile(
            body.as_str()
                .ok_or_else(|| anyhow!("send takes a string"))?,
        )?)),
        "expect" => {
            let timeout = body
                .get("timeout_ms")
                .and_then(Y::as_u64)
                .map(Duration::from_millis)
                .unwrap_or(DEFAULT_TIMEOUT);
            let rule = if let Some(p) = body.get("pattern").and_then(Y::as_str) {
                if p.is_empty() {
                    bail!("expect pattern is empty");
                }
                StopRule::Pattern(p.as_bytes().to_vec())
            } else if let Some(b) = body.get("bytes") {
                StopRule::Bytes(Template::from_yaml_count(b)?)
            } else if let Some(b) = body.get("min_bytes") {
                StopRule::MinBytes(Template::from_yaml_count(b)?)
            } else {
                bail!("expect needs pattern, bytes, or min_bytes");
            };
            ops.push(Op::Expect { rule, timeout });
        }
        "delay" => {
            let d = match body.get("dist").and_then(Y::as_str) {
                None => Delay::Fixed(Template::from_yaml_count(
                    body.get("ms").ok_or_else(|| anyhow!("delay needs ms"))?,
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
                    unit_ms: match body.get("unit").and_then(Y::as_str).unwrap_or("s") {
                        "s" => 1000.0,
                        "ms" => 1.0,
                        u => bail!("unknown delay unit '{u}'"),
                    },
                },
                Some("pareto") => Delay::Pareto {
                    alpha: num(body, "alpha")?,
                    scale_ms: num(body, "scale_ms")?,
                },
                Some(d) => bail!("unknown delay distribution '{d}'"),
            };
            ops.push(Op::Delay(d));
        }
        "set" => {
            let m = body
                .as_mapping()
                .ok_or_else(|| anyhow!("set takes a mapping"))?;
            let mut binds = Vec::new();
            for (k, v) in m {
                let k = k
                    .as_str()
                    .ok_or_else(|| anyhow!("set keys must be strings"))?;
                let v = match v {
                    Y::String(s) => Template::compile(s)?,
                    other => Template::literal(Value::from_yaml(other)?.to_string()),
                };
                binds.push((k.to_string(), v));
            }
            ops.push(Op::Set(binds));
        }
        "loop" => {
            let count = Template::from_yaml_count(
                body.get("count")
                    .ok_or_else(|| anyhow!("loop needs count"))?,
            )?;
            let body_y = body
                .get("do")
                .and_then(Y::as_sequence)
                .ok_or_else(|| anyhow!("loop needs do"))?;
            let start = ops.len();
            ops.push(Op::LoopStart {
                count,
                end: usize::MAX,
            });
            compile_actions(body_y, ops)?;
            let end = ops.len();
            ops.push(Op::LoopEnd { start });
            if let Op::LoopStart { end: e, .. } = &mut ops[start] {
                *e = end;
            }
        }
        "branch" => {
            let arms_y = body
                .as_sequence()
                .ok_or_else(|| anyhow!("branch takes a list"))?;
            let at = ops.len();
            ops.push(Op::Branch { arms: vec![] });
            let mut arms = Vec::new();
            let mut jumps = Vec::new();
            let mut cumulative = 0.0;
            for arm in arms_y {
                cumulative += num(arm, "p")?;
                arms.push((cumulative, ops.len()));
                let do_y = arm
                    .get("do")
                    .and_then(Y::as_sequence)
                    .ok_or_else(|| anyhow!("branch arm needs do"))?;
                compile_actions(do_y, ops)?;
                jumps.push(ops.len());
                ops.push(Op::Jump(usize::MAX));
            }
            if (cumulative - 1.0).abs() > 1e-6 {
                bail!("branch probabilities sum to {cumulative}, not 1");
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
            other => bail!("unknown close mode {other:?}"),
        })),
        "connect" | "accept" => bail!("'{kind}' may only open a side"),
        "spawn" | "raw" => bail!("'{kind}' is outside spike S1"),
        other => bail!("unknown action '{other}'"),
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
        assert!(Scenario::parse(cycle).is_err());
        let udp = "scenario: t\nflows:\n  - { id: a, module: m, l4: udp, client: [ { connect: { port: 1 } } ] }\n";
        assert!(Scenario::parse(udp)
            .unwrap_err()
            .to_string()
            .contains("TCP only"));
        let no_connect = "scenario: t\nflows:\n  - { id: a, module: m, client: [ { send: x } ] }\n";
        assert!(Scenario::parse(no_connect).is_err());
        let bad_branch = "scenario: t\nflows:\n  - { id: a, module: m, client: [ { connect: { port: 1 } }, { branch: [ { p: 0.5, do: [ { send: x } ] }, { p: 0.4, do: [ { send: y } ] } ] } ] }\n";
        assert!(Scenario::parse(bad_branch).is_err());
    }

    #[test]
    fn example_scenarios_compile() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../contracts/examples/modules/http1/scenarios");
        for name in ["web_get.yaml", "web_browse.yaml"] {
            Scenario::load(&root.join(name)).unwrap();
        }
    }
}
