"""Guards and actions written in Python, bound by IRI.

A chart on the `rust` datamodel names its guards and actions by IRI; Python functions
registered under those IRIs run inside the step. A guard gets `(context, event)` and returns a
bool; an action gets the same and returns what it changes: `context` fields and events to
`raise` or `send`. Both must be pure: the journal is replayed, and `dev=True` replays every
macrostep to report a guard or action that answers differently the second time.

Illustrates the python-bindings capability (Python Guards And Actions).

Run with `python examples/python/guards_actions.py`.
"""

# @tg: implements:: [[openspec:python-bindings#Runnable Examples]]
import warnings

from vertep import ActorSystem, ImpurityWarning

CHART = """<scxml xmlns="http://www.w3.org/2005/07/scxml" version="1.0" initial="idle" datamodel="rust">
  <datamodel><data id="total" expr="0"/></datamodel>
  <state id="idle">
    <transition event="pay" cond="vt:guard/order/positive" target="paid">
      <foreach array="vt:action/order/record" item="_vt_emit">
        <send eventexpr="_vt_emit.event" targetexpr="_vt_emit.target" typeexpr="_vt_emit.type" delayexpr="_vt_emit.delay"><content expr="_vt_emit.data"/></send>
      </foreach>
    </transition>
  </state>
  <state id="paid"><transition event="receipt" target="done"/></state>
  <final id="done"/>
</scxml>"""

TYPE = "https://example.org/charts/payment"


def positive(context, event):
    return event["amount"] > 0


def record(context, event):
    # What the action changes, and the event it raises (its fields are the payload).
    return {
        "context": {"total": context["total"] + event["amount"]},
        "raise": [{"type": "receipt", "amount": event["amount"]}],
    }


with ActorSystem(
    storage="memory",
    clock="virtual",
    guards={"vt:guard/order/positive": positive},
    actions={"vt:action/order/record": record},
) as system:
    payment = system.spawn(CHART, type=TYPE, key="1")
    payment.send("pay", amount=0)
    print("pay 0 :", payment.snapshot().configuration, "(the guard refused)")
    payment.send("pay", amount=25)
    snapshot = payment.snapshot()
    print("pay 25:", snapshot.configuration, "terminated:", snapshot.terminated, "total:", snapshot.context["total"])

# An impure guard answers differently when the journal is replayed; development mode says so.
calls = []


def flaky(context, event):
    calls.append(event)
    return len(calls) == 1


with ActorSystem(
    storage="memory",
    clock="virtual",
    dev=True,
    guards={"vt:guard/order/positive": flaky},
    actions={"vt:action/order/record": lambda context, event: None},
) as system:
    payment = system.spawn(CHART, type=TYPE, key="2")
    with warnings.catch_warnings(record=True) as caught:
        warnings.simplefilter("always")
        payment.send("pay", amount=1)
    for w in caught:
        if issubclass(w.category, ImpurityWarning):
            print("dev mode:", str(w.message).split(":")[0])
