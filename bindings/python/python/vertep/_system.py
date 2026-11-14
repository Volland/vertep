"""The Python API: ``ActorSystem``, ``Actor`` and ``Snapshot`` over the native command bridge."""

from __future__ import annotations

import json
import warnings
from collections import deque
from dataclasses import dataclass
from typing import Any, Callable, Deque, Dict, List, Mapping, Optional, Tuple, Union

from . import _vertep
from ._errors import InvalidArgument, SystemClosed, VertepError, from_bridge

__all__ = ["ActorSystem", "Actor", "Snapshot", "Subscription", "Event", "JournalEntry"]

#: An event: its name, or ``{"type": name, **payload}``.
Event = Union[str, Mapping[str, Any]]
#: A journal entry as the bridge reports it (``position``, ``kind``, ``event``, ``transitions``,
#: ``timers``, ``sends``, ``timestamp``, ``digest``, ...).
JournalEntry = Dict[str, Any]


@dataclass(frozen=True)
# @tg: implements:: [[openspec:python-bindings#Snapshots And Subscriptions]]
class Snapshot:
    """An actor's state after a macrostep."""

    #: The actor's address IRI.
    address: str
    #: The journal position the state is after (0 before the first entry).
    position: int
    #: The chart version IRI.
    version: str
    #: Active state ids in document order.
    configuration: Tuple[str, ...]
    #: The datamodel's context, as plain Python values.
    context: Any
    #: Whether the session terminated.
    terminated: bool

    def matches(self, state: str) -> bool:
        """Whether state ``state`` is active."""
        return state in self.configuration

    @staticmethod
    def _from(d: Mapping[str, Any]) -> "Snapshot":
        return Snapshot(
            address=d["address"],
            position=d["position"],
            version=d["version"],
            configuration=tuple(d["configuration"]),
            context=d["context"],
            terminated=d["terminated"],
        )


# @tg: implements:: [[openspec:python-bindings#Snapshots And Subscriptions]]
class Subscription:
    """A live subscription; ``cancel()`` (or leaving a ``with`` block) stops it."""

    def __init__(self, system: "ActorSystem", sub_id: int):
        self._system = system
        self._id = sub_id

    def cancel(self) -> None:
        """Stops delivery. Cancelling twice, or after the system closed, does nothing."""
        system, self._system = self._system, None  # type: ignore[assignment]
        if system is not None and not system.closed:
            system._unsubscribe(self._id)

    def __enter__(self) -> "Subscription":
        return self

    def __exit__(self, *exc: object) -> None:
        self.cancel()


def _event(event: Event, data: Mapping[str, Any]) -> Dict[str, Any]:
    """The bridge's event object: ``{"type": name, **payload}``."""
    if isinstance(event, str):
        out: Dict[str, Any] = {"type": event}
    elif isinstance(event, Mapping):
        out = dict(event)
        if "type" not in out and "name" not in out:
            raise InvalidArgument("an event mapping needs a 'type'")
    else:
        raise InvalidArgument("an event is a name or a mapping with a 'type'")
    for key in ("type", "name"):
        if key in data:
            raise InvalidArgument(f"{key!r} cannot be a payload field; pass the event as a mapping")
    out.update(data)
    return out


# @tg: implements:: [[openspec:python-bindings#Actor Creation And Event Sending From Python]]
# @tg: implements:: [[openspec:python-bindings#Journal Inspection And Time Travel]]
class Actor:
    """A virtual actor: addressed by chart type and key, activated by its first message."""

    def __init__(self, system: "ActorSystem", address: str):
        self._system = system
        #: The actor's address IRI, ``<type>/<key>``.
        self.address = address

    def __repr__(self) -> str:
        return f"Actor({self.address!r})"

    def send(
        self, event: Event, /, *, message_id: Optional[str] = None, **data: Any
    ) -> str:
        """Sends an event and runs everything that falls due now; returns the message id.

        ``actor.send("add", qty=2)`` and ``actor.send({"type": "add", "qty": 2})`` are the same
        event. A repeated ``message_id`` is a duplicate and is not applied twice.
        """
        args: Dict[str, Any] = {"to": self.address, "event": _event(event, data)}
        if message_id is not None:
            args["messageId"] = message_id
        return self._system._call("send", args)["messageId"]

    def snapshot(self) -> Snapshot:
        """The actor's current state."""
        return Snapshot._from(self._system._call("snapshot", {"address": self.address}))

    def can(self, event: Event, /, **data: Any) -> bool:
        """Whether the event would take a transition now, without sending it."""
        args = {"address": self.address, "event": _event(event, data)}
        return bool(self._system._call("can", args)["can"])

    def matches(self, state: str) -> bool:
        """Whether state ``state`` is active now."""
        return self.snapshot().matches(state)

    def journal(self, *, start: int = 0) -> List[JournalEntry]:
        """The journal entries from position ``start``: one per accepted event or timer."""
        return self._system._call("journal", {"address": self.address, "from": start})

    def as_of(
        self, *, position: Optional[int] = None, time: Optional[float] = None
    ) -> Snapshot:
        """The state after journal entry ``position``, or at ``time`` (seconds since the epoch).

        Rebuilt by replaying the journal; the live actor is untouched.
        """
        if (position is None) == (time is None):
            raise InvalidArgument("as_of needs exactly one of position and time")
        args: Dict[str, Any] = {"address": self.address}
        if position is not None:
            args["position"] = position
        else:
            args["time"] = int(round(time * 1000))  # type: ignore[operator]
        return Snapshot._from(self._system._call("asOf", args))

    def subscribe(
        self,
        callback: Callable[[Snapshot], None],
        *,
        transitions: Optional[Callable[[Dict[str, Any]], None]] = None,
    ) -> Subscription:
        """Calls ``callback(snapshot)`` once per macrostep that changed the configuration or
        the context, in journal order.

        Callbacks run in the thread that drives the system, during ``send``, ``wait`` or
        ``pump``. With ``transitions=``, ``transitions(notice)`` is also called for every
        macrostep that took a transition, with ``position``, ``event`` and ``transitions``.
        """
        return self._system._subscribe(self.address, callback, transitions)


# @tg: implements:: [[openspec:python-bindings#Actor Creation And Event Sending From Python]]
# @tg: implements:: [[openspec:python-bindings#Explicit Time]]
# @tg: implements:: [[openspec:python-bindings#Durable File Storage]]
# @tg: implements:: [[openspec:python-bindings#Python Guards And Actions]]
class ActorSystem:
    """An actor system on one store: register charts, spawn actors, send events.

    ``storage="memory"`` keeps everything in memory. ``storage="file"`` keeps the journal in a
    tiramemsu file at ``path``, the same file format the Rust runtime and ``@vertep/web`` use,
    so a journal written here replays there.

    Time is explicit. With ``clock="system"`` (the default) timers fire while you call
    :meth:`wait`, :meth:`send` or :meth:`pump`. With ``clock="virtual"`` time moves only when
    you call :meth:`advance`, which is deterministic and what tests want.

    Use it from the thread that created it, and close it (or use ``with``) to release the
    store file::

        with ActorSystem(storage="memory") as system:
            order = system.spawn(ORDER_SCXML, type="https://example.org/order", key="42")
            order.send("pay")
            print(order.snapshot().configuration)
    """

    def __init__(
        self,
        storage: str = "memory",
        *,
        path: Optional[str] = None,
        clock: str = "system",
        start: float = 0.0,
        dev: bool = False,
        idle_timeout: Optional[float] = None,
        guards: Optional[Mapping[str, Callable[[Any, Any], Any]]] = None,
        actions: Optional[Mapping[str, Callable[[Any, Any], Any]]] = None,
    ):
        """
        :param storage: ``"memory"`` or ``"file"``; there is no default fallback between them.
        :param path: The tiramemsu file, for ``storage="file"`` (a path or ``os.PathLike``).
        :param clock: ``"system"`` or ``"virtual"``.
        :param start: The virtual clock's start, in seconds since the epoch (``clock="virtual"``).
        :param dev: Replays each macrostep from the journal and reports a guard or action whose
            second evaluation differs (an :class:`ImpurityWarning`).
        :param idle_timeout: Seconds of idleness after which virtual actors deactivate.
        :param guards: ``{iri: fn(context, event) -> bool}``: guards a chart names by IRI.
        :param actions: ``{iri: fn(context, event) -> dict | None}``: actions a chart names by IRI;
            the result may hold ``context`` (fields to change), ``raise`` and ``send`` (lists of
            events).
        """
        if clock not in ("system", "virtual"):
            raise InvalidArgument('clock is "system" or "virtual"')
        if storage not in ("memory", "file"):
            raise InvalidArgument('storage must be chosen explicitly: "memory" or "file"')
        if storage == "file" and path is None:
            raise InvalidArgument('storage="file" needs a path')
        options: Dict[str, Any] = {"storage": storage, "dev": dev}
        if path is not None:
            import os

            options["path"] = os.fspath(path)
        if idle_timeout is not None:
            options["idleTimeoutMs"] = int(round(idle_timeout * 1000))
        try:
            self._core = _vertep.Core(
                json.dumps(options),
                dict(guards) if guards else None,
                dict(actions) if actions else None,
                int(round(start * 1000)) if clock == "virtual" else None,
            )
        except ValueError as e:
            raise _from_value_error(e) from None
        self._subs: Dict[int, Tuple[str, Callable[..., None]]] = {}
        self._queue: Deque[Dict[str, Any]] = deque()
        self._dispatching = False
        self._closed = False

    # -- lifecycle ---------------------------------------------------------------------------

    @property
    def closed(self) -> bool:
        """Whether :meth:`close` was called."""
        return self._closed

    def close(self) -> None:
        """Closes the system and releases its store file. Closing twice does nothing."""
        if not self._closed:
            self._closed = True
            self._subs.clear()
            self._core.close()

    def __enter__(self) -> "ActorSystem":
        return self

    def __exit__(self, *exc: object) -> None:
        self.close()

    def __repr__(self) -> str:
        return f"ActorSystem({'closed' if self._closed else 'open'})"

    # -- charts and actors -------------------------------------------------------------------

    def register(
        self, source: str, *, type: Optional[str] = None, format: Optional[str] = None
    ) -> Dict[str, str]:
        """Validates and registers a chart; returns ``{"type": ..., "version": ...}``.

        ``source`` is SCXML, YAML or an RDF chart graph (``format`` is ``"scxml"``, ``"yaml"``,
        ``"turtle"``, ``"ntriples"``, ``"nquads"`` or ``"trig"``; sniffed when absent). The chart
        is content-addressed: ``version`` is its version IRI. ``type`` is the chart type IRI
        actors are addressed under (the version IRI when absent).

        :raises ChartInvalid: with every violation in ``.violations``.
        """
        return self._call("register", _chart_args(source, type, format))

    def spawn(
        self,
        source: Optional[str] = None,
        *,
        type: Optional[str] = None,
        key: Optional[str] = None,
        format: Optional[str] = None,
    ) -> Actor:
        """Registers ``source`` (if given) and returns the actor of that chart type and ``key``.

        Without ``source`` the chart type must be registered already. Without ``key`` a fresh one
        is generated. A virtual actor exists for every key: the first message activates it.
        """
        if source is None:
            if type is None:
                raise InvalidArgument("spawn needs a chart source or a registered type")
            args: Dict[str, Any] = {"type": type}
        else:
            args = _chart_args(source, type, format)
        if key is not None:
            args["key"] = key
        return Actor(self, self._call("spawn", args)["address"])

    def actor(self, type: str, key: str) -> Actor:
        """The actor of a registered chart ``type`` and ``key``."""
        return Actor(self, self._call("virtual", {"type": type, "key": key})["address"])

    def actors(self) -> List[str]:
        """The addresses of every actor that has a journal in this store."""
        return self._call("actors", {})

    # -- the graph ---------------------------------------------------------------------------

    def query(self, sparql: str, *, at: Optional[float] = None) -> Dict[str, Any]:
        """Runs a SPARQL query on the state graph.

        Returns ``{"variables": [...], "rows": [{var: N-Triples term}]}`` for SELECT,
        ``{"boolean": b}`` for ASK and ``{"triples": [...]}`` for CONSTRUCT. With ``at=`` (seconds
        since the epoch) the query sees the graph as it was then; that needs ``storage="file"``.
        """
        args: Dict[str, Any] = {"sparql": sparql}
        if at is not None:
            args["time"] = int(round(at * 1000))
        return self._call("query", args)

    def export_journal(self) -> bytes:
        """The journal file's bytes (``storage="file"`` only), importable by the other bindings."""
        self._check_open()
        try:
            return bytes(self._core.export())
        except ValueError as e:
            raise _from_value_error(e) from None

    # -- time --------------------------------------------------------------------------------

    def now(self) -> float:
        """The system's time, in seconds since the epoch (virtual time on the virtual clock)."""
        self._check_open()
        return self._core.now() / 1000.0

    def pending_timers(self) -> List[Dict[str, Any]]:
        """Durable timers that have not fired: ``{due, actor, event}``, ``due`` in milliseconds."""
        return self._call("pendingTimers", {})

    def pump(self) -> None:
        """Runs everything that is due now and delivers subscription callbacks."""
        self._check_open()
        self._deliver(self._core.pump())

    def wait(self, seconds: float) -> None:
        """Lets ``seconds`` of system time pass, firing timers as they fall due.

        On the system clock this sleeps (other Python threads keep running; Ctrl-C interrupts).
        On the virtual clock it advances time at once, as :meth:`advance` does.
        """
        self._check_open()
        if seconds < 0:
            raise InvalidArgument("seconds must not be negative")
        self._deliver(self._core.wait(int(round(seconds * 1000))))

    def advance(self, seconds: float) -> None:
        """Moves a virtual clock forward by ``seconds``, firing every timer that falls due."""
        if self._core.wall_clock:
            raise InvalidArgument('advance needs clock="virtual"; use wait on the system clock')
        self.wait(seconds)

    # -- internals ---------------------------------------------------------------------------

    def _check_open(self) -> None:
        if self._closed:
            raise SystemClosed("the actor system is closed")

    def _call(self, op: str, args: Dict[str, Any]) -> Any:
        self._check_open()
        try:
            reply = json.loads(self._core.command(op, json.dumps(args)))
        except ValueError as e:  # malformed JSON never comes from our own encoder
            raise _from_value_error(e) from None
        # notifications raised by the command (snapshots of the macrostep it ran)
        self._deliver(self._core.pump())
        if "err" in reply:
            raise from_bridge(reply["err"])
        return reply["ok"]

    def _subscribe(
        self,
        address: str,
        on_snapshot: Callable[[Snapshot], None],
        on_transitions: Optional[Callable[[Dict[str, Any]], None]],
    ) -> Subscription:
        ids: List[int] = []
        sub = self._call("subscribe", {"address": address, "kind": "snapshot"})["id"]
        self._subs[sub] = ("snapshot", on_snapshot)
        ids.append(sub)
        if on_transitions is not None:
            tsub = self._call("subscribe", {"address": address, "kind": "transitions"})["id"]
            self._subs[tsub] = ("transition", on_transitions)
            ids.append(tsub)
        return _Subscriptions(self, ids)

    def _unsubscribe(self, sub_id: int) -> None:
        self._subs.pop(sub_id, None)
        try:
            self._call("unsubscribe", {"id": sub_id})
        except VertepError:
            pass

    def _deliver(self, notes_json: str) -> None:
        self._queue.extend(json.loads(notes_json))
        if self._dispatching:  # a callback sent an event: the outer loop drains the queue
            return
        self._dispatching = True
        try:
            while self._queue:
                note = self._queue.popleft()
                kind = note.get("kind")
                if kind == "impurity":
                    warnings.warn(note["message"], ImpurityWarning, stacklevel=4)
                    continue
                entry = self._subs.get(note.get("sub", -1))
                if entry is None:
                    continue
                if kind == "snapshot":
                    entry[1](Snapshot._from(note["snapshot"]))
                elif kind == "transition":
                    entry[1](
                        {k: note[k] for k in ("address", "position", "event", "transitions")}
                    )
        finally:
            self._dispatching = False


class _Subscriptions(Subscription):
    """A subscription made of one or two bridge subscriptions."""

    def __init__(self, system: ActorSystem, ids: List[int]):
        super().__init__(system, ids[0])
        self._ids = ids

    def cancel(self) -> None:
        system, self._system = self._system, None  # type: ignore[assignment]
        if system is not None and not system.closed:
            for i in self._ids:
                system._unsubscribe(i)


class ImpurityWarning(UserWarning):
    """A guard or action gave a different result when replayed (``dev=True``)."""


def _chart_args(source: str, type: Optional[str], fmt: Optional[str]) -> Dict[str, Any]:
    if not isinstance(source, str):
        raise InvalidArgument("a chart source is a string (SCXML, YAML or RDF text)")
    args: Dict[str, Any] = {"source": source}
    if type is not None:
        args["type"] = type
    if fmt is not None:
        args["format"] = fmt
    return args


def _from_value_error(e: ValueError) -> VertepError:
    """A bridge error that surfaced as a ``ValueError`` carrying the error object as JSON."""
    try:
        return from_bridge(json.loads(str(e)))
    except (ValueError, AttributeError):
        return InvalidArgument(str(e))
