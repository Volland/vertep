"""Decision records for an auditor: verify an agent's explanation bundle offline, and replay it
under another chart. No actor system, decider or network is involved: everything comes from the
bundle, which an agent host exports (``AgentHost::export_bundle`` in Rust)."""

from __future__ import annotations

import json
from dataclasses import dataclass
from typing import Any, Dict, Mapping, Optional, Tuple, Union

from . import _vertep
from ._errors import from_bridge

__all__ = ["verify_bundle", "counterfactual_bundle", "VerifiedBundle", "Explanation", "Bundle"]

#: A bundle: its JSON text (``str`` or ``bytes``) or the parsed object.
Bundle = Union[str, bytes, Mapping[str, Any]]
#: One explanation, in the canonical JSON of ``vertep-agent`` (what ``vt explain --json`` prints
#: and the bundle's digests cover): ``actor``, ``position``, ``ts``, ``input``, ``chart_version``,
#: ``state_before``, ``state_after``, ``entered``, ``exited``, ``effects``, ``decision``, and
#: ``replay: "recorded"`` when erased data was replayed.
Explanation = Dict[str, Any]


@dataclass(frozen=True)
class VerifiedBundle:
    """What a bundle that verified explains."""

    #: The actor IRI.
    actor: str
    #: The first position explained.
    from_position: int
    #: The last position explained.
    to_position: int
    #: How many deciders verification called: always 0.
    decider_calls: int
    #: The recomputed explanations, positions ``from_position`` to ``to_position``.
    explanations: Tuple[Explanation, ...]

    def explanation(self, position: int) -> Optional[Explanation]:
        """The explanation of ``position``, if the bundle explains it."""
        return next((x for x in self.explanations if x["position"] == position), None)


def _text(bundle: Bundle) -> str:
    if isinstance(bundle, bytes):
        return bundle.decode("utf-8")
    if isinstance(bundle, str):
        return bundle
    return json.dumps(bundle)


def _ok(reply: str) -> Any:
    r = json.loads(reply)
    if "err" in r:
        raise from_bridge(r["err"])
    return r["ok"]


# @tg: implements:: [[openspec:decision-records#Explanation Bundle]] {note: "python API"}
def verify_bundle(bundle: Bundle) -> VerifiedBundle:
    """Verifies an explanation bundle offline and returns the explanations it recomputed.

    Every chart is re-hashed, every entry checked against the manifest, the journal replayed and
    each explanation recomputed and compared with its digest; no decider is called.

    Raises ``BundleInvalid`` naming the altered entry, the missing chart or the wrong digest, and
    ``InvalidArgument`` for text that is not JSON.
    """
    v = _ok(_vertep.verify_bundle(_text(bundle)))
    return VerifiedBundle(
        actor=v["actor"],
        from_position=v["from"],
        to_position=v["to"],
        decider_calls=v["deciderCalls"],
        explanations=tuple(v["explanations"]),
    )


# @tg: implements:: [[openspec:decision-records#Counterfactual Replay]] {confidence: partial, note: "python API, from a bundle"}
def counterfactual_bundle(bundle: Bundle, chart: str, *, format: Optional[str] = None) -> Dict[str, Any]:
    """Verifies ``bundle``, then replays its journal under ``chart`` (a chart source; its format
    is guessed when ``format`` is not given) with the decisions the journal recorded.

    Returns the counterfactual report: ``original_version``, ``counterfactual_version``,
    ``decisions``, ``steps`` (each with ``original_state``, ``counterfactual_state``, the
    dispositions and whether they changed), ``first_divergence`` and ``changed_dispositions``.
    The bundle is only read.
    """
    spec: Dict[str, Any] = {"source": chart}
    if format is not None:
        spec["format"] = format
    report: Dict[str, Any] = _ok(_vertep.counterfactual_bundle(_text(bundle), json.dumps(spec)))
    return report
