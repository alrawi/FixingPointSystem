// SPDX-License-Identifier: GPL-3.0-only
//! Static checks of a compiled template against its scope.
//!
//! [`Template::compile`] checks the grammar and the core functions with every
//! variable typed `any`. [`Template::check`] repeats the type checks with the
//! declared variable types, checks the placement of `last`, and checks module
//! calls against the manifest function catalogs. The rules and their order
//! follow `_check_node` in `tools/contracts/validate.py`.

use std::collections::{HashMap, HashSet};

use super::{Expr, ModuleCall, Template, TemplateError};
use crate::function::ValueType;
use crate::manifest::ModuleContext;

/// What a template may refer to at one point of a flow program.
#[derive(Clone, Copy, Debug)]
pub struct CheckScope<'a> {
    /// Every visible variable with its value type, `any` when only a run time
    /// value fixes the type.
    pub vars: &'a HashMap<String, ValueType>,
    /// The callable modules, or `None` to leave module calls unchecked.
    pub modules: Option<&'a ModuleContext>,
    /// The names of the profiles that the scenario declares.
    pub profiles: &'a HashSet<String>,
    /// True when an `expect` earlier on the same side always runs first.
    pub last: bool,
}

/// A problem that [`Template::check`] finds.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum CheckError {
    /// A type, arity, parameter, or variable error.
    #[error(transparent)]
    Template(#[from] TemplateError),
    /// `last` appears where no `expect` always ran before.
    #[error("'last' needs an expect that always runs earlier on the same side")]
    LastWithoutExpect,
    /// A call names a module that is neither the owner nor a required module.
    #[error("module '{module}' is neither '{owner}' nor a required module")]
    ForeignModule {
        /// The called module.
        module: String,
        /// The module that ships the scenario.
        owner: String,
    },
    /// The manifest of the called module declares no such function.
    #[error("module '{module}' declares no template function '{function}' in its manifest")]
    UnknownModuleFunction {
        /// Module name.
        module: String,
        /// Function name.
        function: String,
    },
}

/// A finding that does not reject the scenario.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CheckWarning {
    /// The manifest of a required module is not available, so its calls
    /// stay unchecked.
    UncheckedModule {
        /// Module name.
        module: String,
        /// Function name.
        function: String,
    },
}

impl std::fmt::Display for CheckWarning {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CheckWarning::UncheckedModule { module, function } => write!(
                f,
                "cannot check '{module}.{function}', the manifest of module '{module}' is not available"
            ),
        }
    }
}

struct Checker<'a, 'w> {
    scope: CheckScope<'a>,
    warnings: &'w mut Vec<CheckWarning>,
}

fn arg_type(
    function: &str,
    param: &str,
    expected: ValueType,
    got: ValueType,
) -> Result<(), CheckError> {
    if expected.accepts(got) {
        Ok(())
    } else {
        Err(TemplateError::ArgumentType {
            function: function.into(),
            param: param.into(),
            expected,
            got,
        }
        .into())
    }
}

impl Checker<'_, '_> {
    fn node(&mut self, e: &Expr) -> Result<ValueType, CheckError> {
        Ok(match e {
            Expr::Const(c) => c.value_type(),
            Expr::Var(name) => *self
                .scope
                .vars
                .get(name)
                .ok_or_else(|| TemplateError::UndefinedVariable(name.clone()))?,
            Expr::SessionId => ValueType::Int,
            Expr::SessionFlow => ValueType::Text,
            Expr::Profile(name) => {
                if !self.scope.profiles.contains(name) {
                    return Err(TemplateError::UndefinedProfile(name.clone()).into());
                }
                ValueType::Text
            }
            Expr::Dist(d, args) => {
                for (arg, param) in args.iter().zip(d.params()) {
                    let t = self.node(arg)?;
                    arg_type(&d.qualified(), param.name, param.ty, t)?;
                }
                ValueType::Int
            }
            Expr::Last => {
                if !self.scope.last {
                    return Err(CheckError::LastWithoutExpect);
                }
                ValueType::Bytes
            }
            Expr::Fill(n) => {
                let t = self.node(n)?;
                arg_type("fill", "n", ValueType::Int, t)?;
                ValueType::Text
            }
            Expr::Hex(s) => {
                let t = self.node(s)?;
                arg_type("hex", "s", ValueType::Text, t)?;
                ValueType::Bytes
            }
            Expr::RandInt(a, b) => {
                let ta = self.node(a)?;
                let tb = self.node(b)?;
                arg_type("rand_int", "a", ValueType::Int, ta)?;
                arg_type("rand_int", "b", ValueType::Int, tb)?;
                ValueType::Int
            }
            Expr::Call(call) => {
                self.call(call)?;
                ValueType::Bytes
            }
        })
    }

    fn call(&mut self, call: &ModuleCall) -> Result<(), CheckError> {
        let positional = call
            .positional
            .iter()
            .map(|e| self.node(e))
            .collect::<Result<Vec<_>, _>>()?;
        let named = call
            .named
            .iter()
            .map(|(k, e)| self.node(e).map(|t| (k.as_str(), t)))
            .collect::<Result<Vec<_>, _>>()?;
        let Some(ctx) = self.scope.modules else {
            return Ok(());
        };
        let Some(catalog) = ctx.callable.get(&call.module) else {
            return Err(CheckError::ForeignModule {
                module: call.module.clone(),
                owner: ctx.module.clone(),
            });
        };
        let Some(catalog) = catalog else {
            self.warnings.push(CheckWarning::UncheckedModule {
                module: call.module.clone(),
                function: call.function.clone(),
            });
            return Ok(());
        };
        let Some(sig) = catalog.get(&call.function) else {
            return Err(CheckError::UnknownModuleFunction {
                module: call.module.clone(),
                function: call.function.clone(),
            });
        };
        let qualified = format!("{}.{}", call.module, call.function);
        if positional.len() > sig.params.len() {
            return Err(TemplateError::Arity {
                name: qualified,
                max: sig.params.len(),
                got: positional.len(),
            }
            .into());
        }
        let mut bound: Vec<Option<ValueType>> = vec![None; sig.params.len()];
        for (slot, t) in bound.iter_mut().zip(&positional) {
            *slot = Some(*t);
        }
        for (name, t) in named {
            let Some(i) = sig.params.iter().position(|p| p.name == name) else {
                return Err(TemplateError::UnknownParameter {
                    function: qualified,
                    param: name.into(),
                }
                .into());
            };
            if bound[i].is_some() {
                return Err(TemplateError::DuplicateParameter {
                    function: qualified,
                    param: name.into(),
                }
                .into());
            }
            bound[i] = Some(t);
        }
        for (p, t) in sig.params.iter().zip(bound) {
            match t {
                None if p.required => {
                    return Err(TemplateError::MissingParameter {
                        function: qualified,
                        param: p.name.clone(),
                    }
                    .into())
                }
                None => {}
                Some(t) => arg_type(&qualified, &p.name, p.ty, t)?,
            }
        }
        Ok(())
    }
}

impl Template {
    /// Checks the template against `scope` and appends warnings, such as a
    /// call into a module whose manifest is missing, to `warnings`.
    pub fn check(
        &self,
        scope: CheckScope<'_>,
        warnings: &mut Vec<CheckWarning>,
    ) -> Result<(), CheckError> {
        let mut c = Checker { scope, warnings };
        for seg in &self.segs {
            if let super::Seg::Expr(e) = seg {
                c.node(e)?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::{Manifest, ModuleSet};

    fn ctx() -> ModuleContext {
        let mut set = ModuleSet::default();
        set.insert(
            Manifest::parse(
                "name: m\ntype: app\nrequires: [ { module: ghost } ]\nfunctions:\n  - name: f\n    params:\n      - { name: n, type: int }\n      - { name: s, type: text, default: x }\n",
            )
            .expect("manifest"),
        );
        set.context("m").expect("context")
    }

    fn check(src: &str, last: bool) -> (Result<(), CheckError>, Vec<CheckWarning>) {
        let vars: HashMap<String, ValueType> = [
            ("n".to_string(), ValueType::Int),
            ("host".to_string(), ValueType::Text),
            ("bound".to_string(), ValueType::Any),
        ]
        .into();
        let profiles: HashSet<String> = ["chrome".to_string()].into();
        let ctx = ctx();
        let mut warnings = Vec::new();
        let t = Template::compile(src).expect("compile");
        let r = t.check(
            CheckScope {
                vars: &vars,
                modules: Some(&ctx),
                profiles: &profiles,
                last,
            },
            &mut warnings,
        );
        (r, warnings)
    }

    #[test]
    fn accepts_typed_calls() {
        for src in [
            "{{ m.f(vars.n) }}",
            "{{ m.f(1, s=vars.host) }}",
            "{{ m.f(n=vars.bound) }}",
            "{{ fill(vars.n) }}{{ rand_int(0, vars.bound) }}",
            "{{ m.f(session.id, s=session.flow) }}{{ last }}",
        ] {
            let (r, w) = check(src, true);
            assert_eq!(r, Ok(()), "{src}");
            assert!(w.is_empty());
        }
    }

    type Want = fn(&CheckError) -> bool;

    #[test]
    fn rejects_what_the_python_validator_rejects() {
        let cases: [(&str, Want); 9] = [
            ("{{ vars.nope }}", |e| {
                matches!(e, CheckError::Template(TemplateError::UndefinedVariable(_)))
            }),
            ("{{ fill(vars.host) }}", |e| {
                matches!(e, CheckError::Template(TemplateError::ArgumentType { .. }))
            }),
            ("{{ last }}", |e| matches!(e, CheckError::LastWithoutExpect)),
            ("{{ other.f(1) }}", |e| {
                matches!(e, CheckError::ForeignModule { .. })
            }),
            ("{{ m.g(1) }}", |e| {
                matches!(e, CheckError::UnknownModuleFunction { .. })
            }),
            ("{{ m.f() }}", |e| {
                matches!(
                    e,
                    CheckError::Template(TemplateError::MissingParameter { .. })
                )
            }),
            ("{{ m.f(1, 'a', 3) }}", |e| {
                matches!(e, CheckError::Template(TemplateError::Arity { .. }))
            }),
            ("{{ m.f(1, k=2) }}", |e| {
                matches!(
                    e,
                    CheckError::Template(TemplateError::UnknownParameter { .. })
                )
            }),
            ("{{ m.f(vars.host) }}", |e| {
                matches!(e, CheckError::Template(TemplateError::ArgumentType { .. }))
            }),
        ];
        for (src, want) in cases {
            let (r, _) = check(src, false);
            let e = r.expect_err(src);
            assert!(want(&e), "{src}: {e:?}");
        }
    }

    #[test]
    fn warns_about_modules_without_manifest() {
        let (r, w) = check("{{ ghost.x(1) }}", false);
        assert_eq!(r, Ok(()));
        assert_eq!(
            w,
            [CheckWarning::UncheckedModule {
                module: "ghost".into(),
                function: "x".into()
            }]
        );
    }
}
