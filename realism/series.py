# SPDX-License-Identifier: GPL-3.0-only
"""The failures series that every check of the realism report carries.

``contracts/changes/CON-23.md`` gives every check kind the catalog metric
``realism.check_failures`` with the label ``check``: the number of reasons for which
the check failed, and 0 exactly when the check passed. The checks of the other series
build those themselves, because each one reports something about the thing it judged.
"""

from __future__ import annotations

from typing import Any

FAILURES_METRIC = "realism.check_failures"

# The values of the catalog label ``check`` (catalog 0.6.0, ``messages`` added by CON-25). The
# ``host_identity`` check of RLM-10 is not among them, so it carries no failures series until a
# catalog change adds the value.
CATALOG_CHECKS = (
    "conformance",
    "parser_errors",
    "encryption",
    "content",
    "session_structure",
    "temporal",
    "mix_accuracy",
    "transport_behavior",
    "messages",
)


def failures_series(check: str, count: int, passed: bool) -> dict[str, Any]:
    """Return the ``realism.check_failures`` series of one check.

    ``count`` is the number of reasons for the failure as the table of CON-23 defines it for
    the check kind. The value is 0 for a check that passed, and at least 1 for one that did
    not, even when a check judges nothing and names no reason (the schema requires both).
    """
    if check not in CATALOG_CHECKS:
        raise ValueError(f"the metric catalog has no check kind {check!r}")
    return {
        "name": FAILURES_METRIC,
        "labels": {"check": check},
        "value": 0 if passed else max(count, 1),
    }
