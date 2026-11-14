"""Quickstart: one virtual actor, a durable delayed send, and time travel.

An order actor is addressed by its chart type and key. Paying it schedules a delayed `ship`
event, which is a durable timer in the journal and fires 200 ms later as an ordinary journaled
input. The virtual clock makes the run deterministic: time moves only when `advance` says so.

Illustrates the python-bindings capability (Actor Creation And Event Sending From Python,
Explicit Time, Journal Inspection And Time Travel).

Run with `python examples/python/quickstart.py` after `pip install vertep` (or
`maturin develop` in bindings/python).
"""

# @tg: implements:: [[openspec:python-bindings#Runnable Examples]]
from vertep import ActorSystem

ORDER = """<scxml xmlns="http://www.w3.org/2005/07/scxml" version="1.0" initial="open">
  <state id="open">
    <transition event="pay" target="paid"><send event="ship" delay="200ms"/></transition>
  </state>
  <state id="paid"><transition event="ship" target="shipped"/></state>
  <state id="shipped"/>
</scxml>"""

ORDER_TYPE = "https://example.org/charts/order"

with ActorSystem(storage="memory", clock="virtual") as system:
    # A virtual actor exists for every key: no create call, the first message activates it.
    order = system.spawn(ORDER, type=ORDER_TYPE, key="42")
    print("actor", order.address)
    print("start:", order.snapshot().configuration)

    order.send("pay")
    print("after pay:", order.snapshot().configuration)
    print("pending timers:", [(t["event"], t["due"]) for t in system.pending_timers()])

    system.advance(0.1)
    print("after 100 ms:", order.snapshot().configuration)
    system.advance(0.1)
    print("after 200 ms:", order.snapshot().configuration)

    print("journal:")
    for entry in order.journal():
        taken = [t.split("#")[1] for t in entry["transitions"]]
        print(f"  {entry['position']} {entry['event']['name']:<5} -> {taken}")

    # Time travel: the state after entry 1, rebuilt from the journal.
    print("as of entry 1:", order.as_of(position=1).configuration)
