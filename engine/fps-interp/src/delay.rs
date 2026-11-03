// SPDX-License-Identifier: GPL-3.0-only
//! Think time sampling for the `delay` action.

use std::time::Duration;

use fps_flow::program::Delay;
use rand::Rng;
use rand_distr::{Distribution, Exp, LogNormal, Pareto};

/// The longest single think time. Heavy tailed distributions such as Pareto
/// can sample values far beyond any test duration, so we cap every sample.
pub const MAX_DELAY: Duration = Duration::from_secs(3600);

/// Samples a think time from a distribution of the `delay` action.
///
/// A fixed delay takes its milliseconds from a template, so the caller
/// renders it and passes the value as `fixed_ms`, which other distributions
/// ignore. The sample is capped at [`MAX_DELAY`]. Parameters that the
/// distribution rejects, such as a non positive mean, return an error message.
pub fn sample<R: Rng + ?Sized>(
    delay: &Delay,
    fixed_ms: i64,
    rng: &mut R,
) -> Result<Duration, String> {
    let ms = match delay {
        Delay::Fixed(_) => {
            if fixed_ms < 0 {
                return Err(format!("delay ms is negative, got {fixed_ms}"));
            }
            // Milliseconds below 2^53 convert exactly, and larger values hit the cap.
            fixed_ms as f64
        }
        Delay::Exponential { mean_ms } => {
            if !(*mean_ms > 0.0 && mean_ms.is_finite()) {
                return Err(format!(
                    "exponential mean_ms must be positive, got {mean_ms}"
                ));
            }
            Exp::new(1.0 / mean_ms)
                .map_err(|e| format!("exponential delay: {e}"))?
                .sample(rng)
        }
        Delay::Uniform { min_ms, max_ms } => {
            if !(min_ms.is_finite() && max_ms.is_finite() && *min_ms >= 0.0 && max_ms >= min_ms) {
                return Err(format!(
                    "uniform delay needs 0 <= min_ms <= max_ms, got {min_ms} and {max_ms}"
                ));
            }
            if max_ms > min_ms {
                rng.gen_range(*min_ms..*max_ms)
            } else {
                *min_ms
            }
        }
        Delay::LogNormal { mu, sigma, unit_ms } => {
            // rand_distr 0.4 accepts a negative sigma, so we check it here.
            if !(mu.is_finite() && sigma.is_finite() && *sigma >= 0.0) {
                return Err(format!(
                    "lognormal delay needs a finite mu and sigma >= 0, got {mu} and {sigma}"
                ));
            }
            LogNormal::new(*mu, *sigma)
                .map_err(|e| format!("lognormal delay: {e}"))?
                .sample(rng)
                * unit_ms
        }
        Delay::Pareto { alpha, scale_ms } => Pareto::new(*scale_ms, *alpha)
            .map_err(|e| format!("pareto delay: {e}"))?
            .sample(rng),
    };
    let secs = ms / 1000.0;
    if secs.is_nan() {
        return Err("delay sample is not a number".into());
    }
    Ok(Duration::from_secs_f64(
        secs.clamp(0.0, MAX_DELAY.as_secs_f64()),
    ))
}

#[cfg(test)]
mod tests {
    use fps_flow::Template;
    use rand_pcg::Pcg64Mcg;

    use super::*;

    const N: usize = 20_000;

    fn mean_ms(d: &Delay) -> f64 {
        let mut rng = Pcg64Mcg::new(0x1234_5678_9abc_def0_1234_5678_9abc_def3);
        let total: f64 = (0..N)
            .map(|_| sample(d, 0, &mut rng).unwrap().as_secs_f64() * 1000.0)
            .sum();
        total / N as f64
    }

    fn close(got: f64, want: f64, rel: f64) {
        assert!(
            (got - want).abs() <= rel * want,
            "mean {got} is not within {rel} of {want}"
        );
    }

    #[test]
    fn fixed_uses_the_rendered_value() {
        let mut rng = Pcg64Mcg::new(3);
        let d = Delay::Fixed(Template::literal("ignored"));
        assert_eq!(
            sample(&d, 250, &mut rng).unwrap(),
            Duration::from_millis(250)
        );
        assert!(sample(&d, -1, &mut rng).is_err());
    }

    #[test]
    fn sample_means_match_the_distributions() {
        close(mean_ms(&Delay::Exponential { mean_ms: 50.0 }), 50.0, 0.03);
        close(
            mean_ms(&Delay::Uniform {
                min_ms: 10.0,
                max_ms: 30.0,
            }),
            20.0,
            0.02,
        );
        // The mean of a log normal is exp(mu + sigma^2 / 2) in its unit.
        let (mu, sigma) = (1.0_f64, 0.5_f64);
        close(
            mean_ms(&Delay::LogNormal {
                mu,
                sigma,
                unit_ms: 1.0,
            }),
            (mu + sigma * sigma / 2.0).exp(),
            0.03,
        );
        // The mean of a Pareto is alpha * scale / (alpha - 1) for alpha > 1.
        close(
            mean_ms(&Delay::Pareto {
                alpha: 3.0,
                scale_ms: 10.0,
            }),
            15.0,
            0.05,
        );
    }

    #[test]
    fn samples_stay_in_their_support() {
        let mut rng = Pcg64Mcg::new(11);
        for _ in 0..1000 {
            let u = sample(
                &Delay::Uniform {
                    min_ms: 5.0,
                    max_ms: 6.0,
                },
                0,
                &mut rng,
            )
            .unwrap();
            assert!(u >= Duration::from_millis(5) && u < Duration::from_millis(6));
            let p = sample(
                &Delay::Pareto {
                    alpha: 0.5,
                    scale_ms: 2.0,
                },
                0,
                &mut rng,
            )
            .unwrap();
            assert!(p >= Duration::from_millis(2) && p <= MAX_DELAY);
        }
        let degenerate = Delay::Uniform {
            min_ms: 7.0,
            max_ms: 7.0,
        };
        assert_eq!(
            sample(&degenerate, 0, &mut rng).unwrap(),
            Duration::from_millis(7)
        );
    }

    #[test]
    fn invalid_parameters_are_errors() {
        let mut rng = Pcg64Mcg::new(5);
        for d in [
            Delay::Exponential { mean_ms: 0.0 },
            Delay::Uniform {
                min_ms: 5.0,
                max_ms: 1.0,
            },
            Delay::LogNormal {
                mu: 0.0,
                sigma: -1.0,
                unit_ms: 1.0,
            },
            Delay::Pareto {
                alpha: 0.0,
                scale_ms: 1.0,
            },
        ] {
            assert!(sample(&d, 0, &mut rng).is_err(), "{d:?}");
        }
    }
}
