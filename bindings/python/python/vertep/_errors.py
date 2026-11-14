"""Typed errors: one exception class per stable error code of the command bridge."""

from __future__ import annotations

from typing import Any, Dict, List, Optional, Type

__all__ = [
    "VertepError",
    "ChartInvalid",
    "StorageUnavailable",
    "StoreLocked",
    "RequestTimeout",
    "TransportRefused",
    "NotFound",
    "InvalidArgument",
    "ActivationError",
    "DeliveryError",
    "StoreError",
    "SystemClosed",
]


# @tg: implements:: [[openspec:python-bindings#Typed Errors]]
class VertepError(Exception):
    """Base class of every error Vertep raises; ``code`` is the bridge's stable error code."""

    code: str = "Error"

    def __init__(self, message: str, *, violations: Optional[List[Dict[str, Any]]] = None):
        super().__init__(message)
        self.message = message
        #: For ``ChartInvalid``: every problem found, as ``{message, line, column, path?}``.
        self.violations: List[Dict[str, Any]] = violations or []


class ChartInvalid(VertepError):
    """A chart source failed to read or validate; see ``violations``."""

    code = "ChartInvalid"


class StorageUnavailable(VertepError):
    """The requested storage is not available in this environment."""

    code = "StorageUnavailable"


class StoreLocked(VertepError):
    """Another process or system holds the store file open."""

    code = "StoreLocked"


class RequestTimeout(VertepError):
    """No reply arrived within a request's timeout."""

    code = "RequestTimeout"


class TransportRefused(VertepError):
    """A transport refused the connection credential."""

    code = "TransportRefused"


class NotFound(VertepError):
    """A call named something that does not exist: an actor, a chart type, a subscription."""

    code = "NotFound"


class InvalidArgument(VertepError, ValueError):
    """A call's arguments are malformed."""

    code = "InvalidArgument"


class ActivationError(VertepError):
    """An actor could not be activated or replayed."""

    code = "Activation"


class DeliveryError(VertepError):
    """A message could not be delivered."""

    code = "Delivery"


class StoreError(VertepError):
    """The store failed."""

    code = "Store"


class SystemClosed(VertepError, RuntimeError):
    """The actor system was closed."""

    code = "SystemClosed"


_BY_CODE: Dict[str, Type[VertepError]] = {
    cls.code: cls
    for cls in (
        ChartInvalid,
        StorageUnavailable,
        StoreLocked,
        RequestTimeout,
        TransportRefused,
        NotFound,
        InvalidArgument,
        ActivationError,
        DeliveryError,
        StoreError,
        SystemClosed,
    )
}


def from_bridge(err: Dict[str, Any]) -> VertepError:
    """The exception for a bridge error object ``{code, message, violations?}``."""
    cls = _BY_CODE.get(err.get("code", ""), VertepError)
    return cls(str(err.get("message", "")), violations=err.get("violations"))
