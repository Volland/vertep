"""A smoke test for an installed wheel: one actor, one event. Run it as `python -I smoke.py` from
a fresh environment, so nothing is imported from the checkout."""

import vertep

CHART = """<scxml xmlns="http://www.w3.org/2005/07/scxml" version="1.0" initial="a">
  <state id="a"><transition event="go" target="b"/></state>
  <state id="b"/>
</scxml>"""

with vertep.ActorSystem(storage="memory", clock="virtual") as system:
    actor = system.spawn(CHART, type="https://example.org/charts/smoke", key="1")
    actor.send("go")
    assert actor.snapshot().configuration == ("b",), actor.snapshot()
print("vertep", vertep.__version__, "ok")
