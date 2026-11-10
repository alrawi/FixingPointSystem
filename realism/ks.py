# SPDX-License-Identifier: GPL-3.0-only
"""Kolmogorov Smirnov tests in pure Python.

The runner needs only the standard library, so this module implements the one sample
and the two sample test itself. Both return the statistic ``D`` (the largest distance
between two cumulative distribution functions) and the p value, which is the
probability that a sample of the same size from the null distribution has a statistic
of at least ``D``.

**One sample.** The statistic is computed against a continuous cumulative distribution
function, or against a step function for a constant target. The p value is exact
(Marsaglia, Tsang, and Wang, 2003) for up to :data:`EXACT_ONE_SAMPLE_MAX` observations
and asymptotic above that, using the Kolmogorov distribution with the correction of
Stephens (1970), ``lambda = (sqrt(n) + 0.12 + 0.11 / sqrt(n)) * D``.

**Two sample.** The p value is exact (a count of lattice paths, Hodges 1958) while
``n * m`` stays within :data:`EXACT_TWO_SAMPLE_MAX_PRODUCT`, and asymptotic above that,
with the effective size ``n * m / (n + m)`` in the correction.

The exact p values assume that the observations have no ties. With ties, as in byte
sizes, they are approximate, and the one sample p value against a continuous target is
conservative for discrete data, so the test then rejects less often than its level.

A p value below :data:`SMALL_P` is replaced by the asymptotic value, because ``1 - cdf``
loses its digits there and the asymptotic series keeps its relative accuracy.
"""

from __future__ import annotations

import math
from collections.abc import Callable, Sequence
from dataclasses import dataclass

EXACT_ONE_SAMPLE_MAX = 100
EXACT_TWO_SAMPLE_MAX_PRODUCT = 10_000
# The exact one sample algorithm multiplies matrices of order 2 * n * d + 1. Above this
# n * d the p value is below 3e-8 for the sizes that use the exact path.
_EXACT_ONE_SAMPLE_MAX_ND = 30.0
SMALL_P = 1e-9


@dataclass(frozen=True)
class KsResult:
    """Outcome of one KS test."""

    statistic: float
    p_value: float
    n: int
    m: int | None
    method: str

    def to_dict(self) -> dict[str, object]:
        """Return the result as a JSON compatible mapping."""
        return {
            "statistic": self.statistic,
            "p_value": self.p_value,
            "method": self.method,
        }


def kolmogorov_sf(x: float) -> float:
    """Return ``P(K >= x)`` for the Kolmogorov distribution."""
    if x <= 0.27:
        return 1.0
    total = 0.0
    for j in range(1, 200):
        term = math.exp(-2.0 * j * j * x * x)
        total += term if j % 2 else -term
        if term < 1e-18:
            break
    return min(1.0, max(0.0, 2.0 * total))


def _asymptotic_p(statistic: float, effective_n: float) -> float:
    root = math.sqrt(effective_n)
    return kolmogorov_sf((root + 0.12 + 0.11 / root) * statistic)


def _mat_mul(a: list[list[float]], b: list[list[float]]) -> list[list[float]]:
    m = len(a)
    out = [[0.0] * m for _ in range(m)]
    for i in range(m):
        row_a, row_out = a[i], out[i]
        for k in range(m):
            x = row_a[k]
            if x == 0.0:
                continue
            row_b = b[k]
            for j in range(m):
                row_out[j] += x * row_b[j]
    return out


def _mat_pow(
    a: list[list[float]], exp_a: int, n: int, centre: int
) -> tuple[list[list[float]], int]:
    """Return ``a ** n`` as a matrix and a power of ten, rescaled against overflow."""
    if n == 1:
        return a, exp_a
    half, exp_half = _mat_pow(a, exp_a, n // 2, centre)
    square = _mat_mul(half, half)
    exp_square = 2 * exp_half
    if n % 2:
        square = _mat_mul(a, square)
        exp_square += exp_a
    if square[centre][centre] > 1e140:
        square = [[x * 1e-140 for x in row] for row in square]
        exp_square += 140
    return square, exp_square


def _mtw_cdf(n: int, d: float) -> float:
    """Return ``P(D_n < d)`` exactly (Marsaglia, Tsang, Wang 2003)."""
    k = int(n * d) + 1
    m = 2 * k - 1
    h = k - n * d
    mat = [[1.0 if i - j + 1 >= 0 else 0.0 for j in range(m)] for i in range(m)]
    for i in range(m):
        mat[i][0] -= h ** (i + 1)
        mat[m - 1][i] -= h ** (m - i)
    mat[m - 1][0] += (2 * h - 1) ** m if 2 * h - 1 > 0 else 0.0
    for i in range(m):
        for j in range(m):
            if i - j + 1 > 0:
                for g in range(1, i - j + 2):
                    mat[i][j] /= g
    power, exponent = _mat_pow(mat, 0, n, k - 1)
    s = power[k - 1][k - 1]
    for i in range(1, n + 1):
        s = s * i / n
        if s < 1e-140:
            s *= 1e140
            exponent -= 140
    return s * 10.0**exponent


def one_sample_p(statistic: float, n: int) -> tuple[float, str]:
    """Return the p value of a one sample statistic and the method that gave it."""
    if statistic <= 0.0:
        return 1.0, "exact" if n <= EXACT_ONE_SAMPLE_MAX else "asymptotic"
    if statistic >= 1.0:
        return 0.0, "exact"
    if n <= EXACT_ONE_SAMPLE_MAX and n * statistic <= _EXACT_ONE_SAMPLE_MAX_ND:
        p = 1.0 - _mtw_cdf(n, statistic)
        if p >= SMALL_P:
            return min(1.0, p), "exact"
    return _asymptotic_p(statistic, n), "asymptotic"


def ks_one_sample(sample: Sequence[float], cdf: Callable[[float], float]) -> KsResult:
    """Test a sample against a continuous cumulative distribution function."""
    n = len(sample)
    if n == 0:
        raise ValueError("the sample is empty")
    ordered = sorted(sample)
    d = 0.0
    for i, x in enumerate(ordered):
        f = cdf(x)
        d = max(d, (i + 1) / n - f, f - i / n)
    p, method = one_sample_p(d, n)
    return KsResult(d, p, n, None, method)


def ks_one_sample_constant(sample: Sequence[float], value: float, tolerance: float) -> KsResult:
    """Test a sample against a constant, where observations within ``tolerance`` equal it.

    The target is a step function, so the statistic is the larger of the shares of
    observations below and above the constant. The p value is 1 when every observation
    equals the constant and 0 otherwise, because no sample of a constant has a spread.
    """
    n = len(sample)
    if n == 0:
        raise ValueError("the sample is empty")
    below = sum(1 for x in sample if x < value - tolerance)
    above = sum(1 for x in sample if x > value + tolerance)
    d = max(below, above) / n
    return KsResult(d, 1.0 if below + above == 0 else 0.0, n, None, "constant")


def two_sample_statistic(a: Sequence[float], b: Sequence[float]) -> tuple[int, int, int]:
    """Return ``(h, n, m)`` where the statistic is ``h / (n * m)``, ties handled."""
    xs, ys = sorted(a), sorted(b)
    n, m = len(xs), len(ys)
    i = j = 0
    best = 0
    while i < n and j < m:
        x = min(xs[i], ys[j])
        while i < n and xs[i] == x:
            i += 1
        while j < m and ys[j] == x:
            j += 1
        best = max(best, abs(i * m - j * n))
    return best, n, m


def _exact_two_sample_p(h: int, n: int, m: int) -> float:
    """Return ``P(D >= h / (n * m))`` by counting lattice paths inside the band."""
    if h == 0:
        return 1.0
    prev = [0.0] * (m + 1)
    for i in range(n + 1):
        cur = [0.0] * (m + 1)
        for j in range(m + 1):
            if abs(i * m - j * n) >= h:
                continue
            if i == 0 and j == 0:
                cur[j] = 1.0
                continue
            value = 0.0
            if i > 0:
                value += i / (i + j) * prev[j]
            if j > 0:
                value += j / (i + j) * cur[j - 1]
            cur[j] = value
        prev = cur
    return min(1.0, max(0.0, 1.0 - prev[m]))


def two_sample_p(h: int, n: int, m: int) -> tuple[float, str]:
    """Return the p value of a two sample statistic ``h / (n * m)`` and its method."""
    if h == 0:
        return 1.0, "exact"
    if n * m <= EXACT_TWO_SAMPLE_MAX_PRODUCT:
        p = _exact_two_sample_p(h, n, m)
        if p >= SMALL_P:
            return p, "exact"
    return _asymptotic_p(h / (n * m), n * m / (n + m)), "asymptotic"


def ks_two_sample(a: Sequence[float], b: Sequence[float]) -> KsResult:
    """Test whether two samples come from the same continuous distribution."""
    if not a or not b:
        raise ValueError("both samples need at least one observation")
    h, n, m = two_sample_statistic(a, b)
    p, method = two_sample_p(h, n, m)
    return KsResult(h / (n * m), p, n, m, method)
