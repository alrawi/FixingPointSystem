// SPDX-License-Identifier: GPL-3.0-only
//! Typed locations of loader errors.
//!
//! A [`Location`] names the file, the flow, the side, and the path of action
//! indices through loops and branch arms. [`Location::pointer`] renders the
//! JSON pointer that `tools/contracts/validate.py` prints for the same spot,
//! so tests compare the two validators finding by finding.

use std::fmt;
use std::path::PathBuf;

/// One side of a flow.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Side {
    /// The `client` action list.
    Client,
    /// The `server` action list.
    Server,
}

impl Side {
    /// The key of the side in the scenario file.
    pub fn key(self) -> &'static str {
        match self {
            Side::Client => "client",
            Side::Server => "server",
        }
    }

    /// The action that must open the side.
    pub fn opener(self) -> &'static str {
        match self {
            Side::Client => "connect",
            Side::Server => "accept",
        }
    }
}

impl fmt::Display for Side {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.key())
    }
}

/// One step of the path from a side to an action.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Step {
    /// The action at a zero based index of its list, with its kind when the
    /// action is a mapping with one key.
    Action {
        /// Zero based index in the enclosing action list.
        index: usize,
        /// The action key, such as `send` or `loop`.
        kind: Option<String>,
    },
    /// The arm at a zero based index of the enclosing `branch`.
    Arm(usize),
}

/// The flow that an error belongs to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FlowRef {
    /// Zero based index in the `flows` list.
    pub index: Option<usize>,
    /// The flow id, when the flow has one.
    pub id: Option<String>,
}

/// Where a loader error occurred, from the file inward.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Location {
    /// The scenario file, when the scenario was read from one.
    pub file: Option<PathBuf>,
    /// The scenario variable, for errors in `vars`.
    pub var: Option<String>,
    /// The flow.
    pub flow: Option<FlowRef>,
    /// The side of the flow.
    pub side: Option<Side>,
    /// Action indices from the side to the action, outermost first.
    pub actions: Vec<Step>,
    /// The field of the action or the flow, such as `port` or `after`.
    pub field: Option<String>,
}

impl Location {
    /// Returns the zero based index of the innermost action.
    pub fn action_index(&self) -> Option<usize> {
        self.actions.iter().rev().find_map(|s| match s {
            Step::Action { index, .. } => Some(*index),
            Step::Arm(_) => None,
        })
    }

    /// Returns the JSON pointer into the scenario document, in the form that
    /// `tools/contracts/validate.py` reports, for example
    /// `/flows/0/client/2/loop/do/0/send`.
    pub fn pointer(&self) -> String {
        let mut out = String::new();
        if let Some(var) = &self.var {
            out.push_str("/vars/");
            out.push_str(var);
        }
        if let Some(FlowRef { index: Some(i), .. }) = &self.flow {
            out.push_str(&format!("/flows/{i}"));
            if let Some(side) = self.side {
                out.push('/');
                out.push_str(side.key());
            }
            let mut after_action = false;
            for step in &self.actions {
                match step {
                    Step::Action { index, kind } => {
                        if after_action {
                            out.push_str("/do");
                        }
                        out.push_str(&format!("/{index}"));
                        if let Some(k) = kind {
                            out.push('/');
                            out.push_str(k);
                        }
                        after_action = true;
                    }
                    Step::Arm(j) => {
                        out.push_str(&format!("/{j}/do"));
                        after_action = false;
                    }
                }
            }
        }
        if let Some(field) = &self.field {
            out.push('/');
            out.push_str(field);
        }
        out
    }

    pub(crate) fn is_empty(&self) -> bool {
        *self == Location::default()
    }
}

impl fmt::Display for Location {
    /// Prints the location for people, for example
    /// `web_get.yaml: flow 'page': client: action 3: loop: action 1: port`.
    /// Action numbers count from 1, so they match the order in the file.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut parts: Vec<String> = Vec::new();
        if let Some(file) = &self.file {
            parts.push(file.display().to_string());
        }
        if let Some(var) = &self.var {
            parts.push(format!("variable '{var}'"));
        }
        match &self.flow {
            Some(FlowRef { id: Some(id), .. }) => parts.push(format!("flow '{id}'")),
            Some(FlowRef {
                index: Some(i),
                id: None,
            }) => parts.push(format!("flow {}", i + 1)),
            Some(FlowRef {
                index: None,
                id: None,
            }) => parts.push("flow".into()),
            None => {}
        }
        if let Some(side) = self.side {
            parts.push(side.key().into());
        }
        let mut loop_open = false;
        for step in &self.actions {
            match step {
                Step::Action { index, kind } => {
                    if loop_open {
                        parts.push("loop".into());
                    }
                    parts.push(format!("action {}", index + 1));
                    loop_open = kind.as_deref() == Some("loop");
                }
                Step::Arm(j) => {
                    parts.push(format!("branch arm {}", j + 1));
                    loop_open = false;
                }
            }
        }
        if let Some(field) = &self.field {
            parts.push(field.clone());
        }
        f.write_str(&parts.join(": "))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn nested() -> Location {
        Location {
            file: Some("web.yaml".into()),
            var: None,
            flow: Some(FlowRef {
                index: Some(1),
                id: Some("page".into()),
            }),
            side: Some(Side::Client),
            actions: vec![
                Step::Action {
                    index: 2,
                    kind: Some("loop".into()),
                },
                Step::Action {
                    index: 0,
                    kind: Some("branch".into()),
                },
                Step::Arm(1),
                Step::Action {
                    index: 0,
                    kind: Some("send".into()),
                },
            ],
            field: None,
        }
    }

    #[test]
    fn pointer_matches_the_python_validator() {
        assert_eq!(
            nested().pointer(),
            "/flows/1/client/2/loop/do/0/branch/1/do/0/send"
        );
        let field = Location {
            flow: Some(FlowRef {
                index: Some(0),
                id: None,
            }),
            field: Some("after".into()),
            ..Location::default()
        };
        assert_eq!(field.pointer(), "/flows/0/after");
    }

    #[test]
    fn display_counts_actions_from_one() {
        assert_eq!(
            nested().to_string(),
            "web.yaml: flow 'page': client: action 3: loop: action 1: branch arm 2: action 1"
        );
        assert_eq!(nested().action_index(), Some(0));
    }
}
