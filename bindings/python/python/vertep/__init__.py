"""Vertep: W3C SCXML statecharts as durable, journaled actors.

Example::

    from vertep import ActorSystem

    with ActorSystem(storage="memory", clock="virtual") as system:
        light = system.spawn(CHART, type="https://example.org/charts/light", key="a")
        light.send("go")
        print(light.snapshot().configuration)

See https://volland.github.io/vertep/docs/python.html
"""

from . import _vertep
from ._errors import (
    ActivationError,
    ChartInvalid,
    DeliveryError,
    InvalidArgument,
    NotFound,
    RequestTimeout,
    StorageUnavailable,
    StoreError,
    StoreLocked,
    SystemClosed,
    TransportRefused,
    VertepError,
)
from ._system import (
    Actor,
    ActorSystem,
    Event,
    ImpurityWarning,
    JournalEntry,
    Snapshot,
    Subscription,
)

#: The version of the native module; it matches the Rust workspace's.
__version__: str = _vertep.__version__

__all__ = [
    "ActorSystem",
    "Actor",
    "Snapshot",
    "Subscription",
    "Event",
    "JournalEntry",
    "ImpurityWarning",
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
    "__version__",
]
