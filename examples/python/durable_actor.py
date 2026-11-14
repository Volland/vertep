"""A durable actor: close the process's view of it, reopen the file, and nothing is lost.

With `storage="file"` the journal is a tiramemsu file, the format the Rust runtime and
`@vertep/web` use. The second system below starts on a different virtual clock and still
rebuilds the same state: replay reads journaled timestamps, never a clock. The state graph is
queryable with SPARQL.

Illustrates the python-bindings capability (Durable File Storage, Explicit Time).

Run with `python examples/python/durable_actor.py`.
"""

# @tg: implements:: [[openspec:python-bindings#Runnable Examples]]
import tempfile
from pathlib import Path

from vertep import ActorSystem

ORDER = """<scxml xmlns="http://www.w3.org/2005/07/scxml" version="1.0" initial="open">
  <state id="open">
    <transition event="pay" target="paid"><send event="ship" delay="3600s"/></transition>
  </state>
  <state id="paid"><transition event="ship" target="shipped"/></state>
  <state id="shipped"/>
</scxml>"""

ORDER_TYPE = "https://example.org/charts/order"

with tempfile.TemporaryDirectory() as directory:
    journal = Path(directory) / "orders.db"

    with ActorSystem(storage="file", path=journal, clock="virtual", start=0) as system:
        order = system.spawn(ORDER, type=ORDER_TYPE, key="7")
        order.send("pay")
        print("first run:", order.snapshot().configuration, "timers:", len(system.pending_timers()))
    # The system is closed: the file is released and the process holds nothing of the actor.

    # A day later, by a different clock, in a fresh system on the same file. The durable timer
    # was journaled and is overdue, so it fires once, as the system starts.
    with ActorSystem(storage="file", path=journal, clock="virtual", start=86_400) as system:
        system.register(ORDER, type=ORDER_TYPE)
        order = system.actor(ORDER_TYPE, "7")
        snapshot = order.snapshot()
        print("reopened:", snapshot.configuration, "at position", snapshot.position)
        print("timers left:", len(system.pending_timers()))

        # The state graph, as SPARQL sees it now.
        rows = system.query(
            "SELECT (COUNT(*) AS ?n) WHERE { GRAPH ?g { ?s ?p ?o } }"
        )["rows"]
        print("graph holds triples:", int(rows[0]["n"].split('"')[1]) > 0)
        print("journal file is SQLite:", system.export_journal()[:15].decode())
