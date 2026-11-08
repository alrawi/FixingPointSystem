# SPDX-License-Identifier: GPL-3.0-only
"""Python client of the FixingPointSystem controller API. It needs only the standard library."""

from . import _generated as types
from ._generated import API_VERSION
from .client import (
    ApiError,
    AuthenticationError,
    Client,
    FixingPointError,
    ForbiddenError,
    NotFoundError,
    RunFailedError,
    TransportError,
    WaitTimeoutError,
)

__version__ = "0.1.0"

__all__ = [
    "API_VERSION",
    "ApiError",
    "AuthenticationError",
    "Client",
    "FixingPointError",
    "ForbiddenError",
    "NotFoundError",
    "RunFailedError",
    "TransportError",
    "WaitTimeoutError",
    "__version__",
    "types",
]
