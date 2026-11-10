# SPDX-License-Identifier: GPL-3.0-only
"""Tests of the pure Python KS tests against known values.

The reference values come from the critical value tables of the KS test and from
``scipy.stats`` (``kstwo.sf``, ``ks_2samp(method="exact")``), which the runner does not
depend on. The p values of the tests below are written out so that a regression shows up
without scipy.
"""

from __future__ import annotations

import math
import random

import pytest
from hypothesis import given
from hypothesis import strategies as st

from realism import ks


@pytest.mark.parametrize(
    ("n", "d", "p"),
    [
        # Table critical values: D at alpha 0.05 and 0.01 gives that p value.
        (10, 0.40925, 0.0499964523),
        (10, 0.48893, 0.0100003714),
        (20, 0.29408, 0.0499941603),
        (20, 0.35241, 0.0100002702),
        (30, 0.24170, 0.0500052522),
        # Further scipy values, the last two in the tail.
        (50, 0.1, 0.6623112705),
        (50, 0.2, 0.0314387778),
        (100, 0.13, 0.0620923407),
        (100, 0.2, 0.0005551927),
        (7, 0.5, 0.0383917415),
        (5, 0.3, 0.664),
    ],
)
def test_one_sample_exact_p_values_match_known_values(n: int, d: float, p: float) -> None:
    got, method = ks.one_sample_p(d, n)
    assert method == "exact"
    assert got == pytest.approx(p, rel=1e-6)


@pytest.mark.parametrize(
    ("n", "d", "p"),
    [(1000, 0.06, 0.001428597887), (5000, 0.03, 0.00024159234), (500, 0.05, 0.15866260)],
)
def test_one_sample_asymptotic_p_values_stay_close_to_the_exact_ones(
    n: int, d: float, p: float
) -> None:
    got, method = ks.one_sample_p(d, n)
    assert method == "asymptotic"
    assert got == pytest.approx(p, rel=0.02)


def test_the_kolmogorov_distribution_has_its_classic_critical_values() -> None:
    assert ks.kolmogorov_sf(1.3581) == pytest.approx(0.05, abs=2e-4)
    assert ks.kolmogorov_sf(1.6276) == pytest.approx(0.01, abs=1e-4)
    assert ks.kolmogorov_sf(0.0) == 1.0
    assert ks.kolmogorov_sf(10.0) == pytest.approx(0.0, abs=1e-80)


def test_edges_of_the_one_sample_p_value() -> None:
    assert ks.one_sample_p(0.0, 20)[0] == 1.0
    assert ks.one_sample_p(1.0, 20)[0] == 0.0
    # Above n * d of 30 the exact matrix would be large, and the tail is below 3e-8.
    p, method = ks.one_sample_p(0.5, 100)
    assert method == "asymptotic"
    assert 0.0 <= p < 1e-8


def test_one_sample_against_a_uniform_target() -> None:
    result = ks.ks_one_sample([0.1, 0.2, 0.35, 0.9, 0.95], lambda x: min(1.0, max(0.0, x)))
    assert result.statistic == pytest.approx(0.3)
    assert result.p_value == pytest.approx(0.664)
    assert (result.n, result.m, result.method) == (5, None, "exact")


def test_two_sample_exact_p_values_match_scipy() -> None:
    a = [0.1, 0.4, 0.7, 1.1, 1.9, 2.3]
    b = [0.5, 0.9, 1.3, 1.4, 2.8, 3.1, 3.3]
    result = ks.ks_two_sample(a, b)
    assert result.statistic == pytest.approx(0.4285714285714286)
    assert result.p_value == pytest.approx(0.4335664335664336, rel=1e-9)
    assert (result.n, result.m, result.method) == (6, 7, "exact")
    # Disjoint samples: D is 1 and the p value is 2 / C(10, 5).
    disjoint = ks.ks_two_sample([1, 2, 3, 4, 5], [6, 7, 8, 9, 10])
    assert disjoint.statistic == 1.0
    assert disjoint.p_value == pytest.approx(2 / 252, rel=1e-12)
    first = list(range(1, 41))
    second = [x + 0.5 + (x % 3) for x in range(1, 31)]
    assert ks.ks_two_sample(first, second).p_value == pytest.approx(0.3104742295811527, rel=1e-9)


def test_two_sample_uses_the_asymptotic_p_value_for_large_samples() -> None:
    rng = random.Random(5)
    a = [rng.gauss(0, 1) for _ in range(300)]
    b = [rng.gauss(0.5, 1) for _ in range(400)]
    result = ks.ks_two_sample(a, b)
    assert result.method == "asymptotic"
    assert result.p_value < 1e-6
    same = ks.ks_two_sample(a, [rng.gauss(0, 1) for _ in range(400)])
    assert same.p_value > 0.01


def test_ties_are_counted_at_their_value() -> None:
    # F1 jumps to 2/3 at 1 and F2 to 1/3, so D is 1/3 at 1 and again at 2 after the second jump.
    h, n, m = ks.two_sample_statistic([1, 1, 2], [1, 2, 2])
    assert h / (n * m) == pytest.approx(1 / 3)
    assert ks.two_sample_statistic([5, 5, 5], [5, 5])[0] == 0
    assert ks.ks_two_sample([5, 5, 5], [5, 5]).p_value == 1.0


def test_the_constant_test_is_exact() -> None:
    same = ks.ks_one_sample_constant([2.0] * 10, 2.0, 0.0)
    assert (same.statistic, same.p_value, same.method) == (0.0, 1.0, "constant")
    within = ks.ks_one_sample_constant([2.0, 2.004, 1.996], 2.0, 0.005)
    assert within.p_value == 1.0
    off = ks.ks_one_sample_constant([2.0] * 7 + [3.0] * 3, 2.0, 0.0)
    assert off.statistic == pytest.approx(0.3)
    assert off.p_value == 0.0


def test_empty_samples_are_refused() -> None:
    with pytest.raises(ValueError, match="empty"):
        ks.ks_one_sample([], lambda x: x)
    with pytest.raises(ValueError, match="empty"):
        ks.ks_one_sample_constant([], 1.0, 0.0)
    with pytest.raises(ValueError, match="at least one"):
        ks.ks_two_sample([1.0], [])


def test_the_test_keeps_its_level_on_samples_from_the_null() -> None:
    """About 1% of exponential samples reject at alpha 0.01 and almost all shifted ones do."""
    rng = random.Random(20260110)
    cdf = lambda x: 0.0 if x <= 0 else -math.expm1(-x)  # noqa: E731
    trials = 200
    rejected = sum(
        ks.ks_one_sample([rng.expovariate(1.0) for _ in range(50)], cdf).p_value < 0.01
        for _ in range(trials)
    )
    assert rejected / trials < 0.03
    shifted = sum(
        ks.ks_one_sample([rng.expovariate(1.0 / 1.8) for _ in range(50)], cdf).p_value < 0.01
        for _ in range(trials)
    )
    assert shifted / trials > 0.5


@given(
    st.lists(st.floats(0.0, 1.0), min_size=1, max_size=60),
    st.lists(st.floats(0.0, 1.0), min_size=1, max_size=60),
)
def test_two_sample_statistics_and_p_values_are_well_formed(a: list[float], b: list[float]) -> None:
    result = ks.ks_two_sample(a, b)
    assert 0.0 <= result.statistic <= 1.0
    assert 0.0 <= result.p_value <= 1.0
    swapped = ks.ks_two_sample(b, a)
    assert swapped.statistic == pytest.approx(result.statistic)
    assert swapped.p_value == pytest.approx(result.p_value, abs=1e-9)


@given(st.lists(st.floats(0.0, 1.0), min_size=1, max_size=80))
def test_one_sample_p_value_decreases_with_the_statistic(sample: list[float]) -> None:
    n = len(sample)
    low = ks.one_sample_p(0.2, n)[0]
    high = ks.one_sample_p(0.5, n)[0]
    assert 0.0 <= high <= low <= 1.0
