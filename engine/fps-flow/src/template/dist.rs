// SPDX-License-Identifier: GPL-3.0-only
//! The `dist` functions of the template grammar (scenario contract v0.3).
//!
//! `dist.pareto`, `dist.lognormal`, `dist.exponential`, and `dist.uniform`
//! draw from the session generator and return an integer. The draws use plain
//! inverse transform sampling, and the log normal draw uses the Box-Muller
//! transform, so the crate needs no distribution library. Every call consumes
//! a fixed number of generator words, which keeps a seeded run repeatable.

use rand::{Rng, RngCore};

use super::TemplateError;
use crate::function::ValueType;

/// The default `cap` of a distribution function.
pub const DEFAULT_CAP: i64 = 2_147_483_647;

/// A distribution function of the `dist` root.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Dist {
    Pareto,
    Lognormal,
    Exponential,
    Uniform,
}

/// One parameter of a distribution function.
pub(crate) struct Param {
    pub name: &'static str,
    pub ty: ValueType,
}

const fn p(name: &'static str, ty: ValueType) -> Param {
    Param { name, ty }
}

const PARETO: &[Param] = &[
    p("alpha", ValueType::Float),
    p("scale", ValueType::Float),
    p("cap", ValueType::Int),
];
const LOGNORMAL: &[Param] = &[
    p("mu", ValueType::Float),
    p("sigma", ValueType::Float),
    p("cap", ValueType::Int),
];
const EXPONENTIAL: &[Param] = &[p("mean", ValueType::Float), p("cap", ValueType::Int)];
const UNIFORM: &[Param] = &[p("low", ValueType::Float), p("high", ValueType::Float)];

impl Dist {
    /// Looks a function up by its name after `dist.`.
    pub fn by_name(name: &str) -> Option<Dist> {
        Some(match name {
            "pareto" => Dist::Pareto,
            "lognormal" => Dist::Lognormal,
            "exponential" => Dist::Exponential,
            "uniform" => Dist::Uniform,
            _ => return None,
        })
    }

    /// The function name without the `dist.` prefix.
    pub fn name(self) -> &'static str {
        match self {
            Dist::Pareto => "pareto",
            Dist::Lognormal => "lognormal",
            Dist::Exponential => "exponential",
            Dist::Uniform => "uniform",
        }
    }

    /// The qualified name for messages.
    pub fn qualified(self) -> String {
        format!("dist.{}", self.name())
    }

    /// The parameters in positional order. The trailing `cap` is optional.
    pub fn params(self) -> &'static [Param] {
        match self {
            Dist::Pareto => PARETO,
            Dist::Lognormal => LOGNORMAL,
            Dist::Exponential => EXPONENTIAL,
            Dist::Uniform => UNIFORM,
        }
    }

    /// True when the last parameter is the optional `cap`.
    pub fn has_cap(self) -> bool {
        self != Dist::Uniform
    }

    /// Checks one argument value against the range rule of its parameter.
    pub fn check_param(self, param: &str, value: f64) -> Result<(), TemplateError> {
        let rule = match param {
            "alpha" | "scale" | "sigma" | "mean" | "cap" if !(value > 0.0 && value.is_finite()) => {
                "must be positive"
            }
            "low" | "high" if !(value >= 0.0 && value.is_finite()) => "must not be negative",
            "mu" if !value.is_finite() => "must be finite",
            _ => return Ok(()),
        };
        Err(TemplateError::DistParameter {
            function: self.qualified(),
            param: param.to_string(),
            rule,
        })
    }

    /// Draws one value. `args` holds the parameters without `cap`, in
    /// positional order, and every argument passed [`Dist::check_param`].
    pub fn draw<R: RngCore + ?Sized>(
        self,
        args: &[f64],
        cap: i64,
        rng: &mut R,
    ) -> Result<i64, TemplateError> {
        let arg = |i: usize| args.get(i).copied().unwrap_or(0.0);
        let x = match self {
            // Inverse transform: scale * u^(-1/alpha) for u in (0, 1].
            Dist::Pareto => arg(1) * open_unit(rng).powf(-1.0 / arg(0)),
            Dist::Lognormal => {
                // Box-Muller, which uses the first of its two normals and so
                // draws two words per call.
                let (u1, u2) = (open_unit(rng), rng.gen::<f64>());
                let z = (-2.0 * u1.ln()).sqrt() * (std::f64::consts::TAU * u2).cos();
                (arg(0) + arg(1) * z).exp()
            }
            Dist::Exponential => -arg(0) * open_unit(rng).ln(),
            Dist::Uniform => {
                let (low, high) = (arg(0), arg(1));
                if low > high {
                    return Err(TemplateError::UniformRange);
                }
                low + (high - low) * rng.gen::<f64>()
            }
        };
        // The arguments are checked, so x is not NaN. The float to integer
        // cast saturates, so an infinite draw lands on the cap.
        let x = x.round();
        let capped = if self.has_cap() { x.min(cap as f64) } else { x };
        Ok(capped as i64)
    }
}

/// A uniform draw from (0, 1], which keeps `ln` finite.
fn open_unit<R: RngCore + ?Sized>(rng: &mut R) -> f64 {
    1.0 - rng.gen::<f64>()
}
