"""The Python API against the real native module. Each test proves one scenario of the
`python-bindings` capability."""

import os
import threading
import time
import warnings

import pytest

import vertep
from vertep import ActorSystem

from conftest import CART, CART_TYPE, GUARDED, GUARDED_TYPE, ORDER, ORDER_TYPE


# @tg: verifies:: [[openspec:python-bindings#Native Extension Module#Version matches the workspace]] {level: unit}
def test_version_matches_the_workspace():
    root = os.path.join(os.path.dirname(__file__), "..", "..", "..", "Cargo.toml")
    with open(root) as f:
        line = next(l for l in f if l.startswith("version = "))
    assert vertep.__version__ == line.split('"')[1]


# @tg: verifies:: [[openspec:python-bindings#Actor Creation And Event Sending From Python#Create from SCXML text]] {level: integration}
def test_create_from_scxml_text(system):
    order = system.spawn(ORDER, type=ORDER_TYPE, key="1")
    s = order.snapshot()
    assert order.address == f"{ORDER_TYPE}/1"
    assert s.configuration == ("open",)
    assert s.matches("open") and not s.matches("paid")
    assert s.version.startswith("urn:vertep:chart:sha256:")
    assert s.position == 0 and not s.terminated


# @tg: verifies:: [[openspec:python-bindings#Actor Creation And Event Sending From Python#Payload as keyword arguments]] {level: integration}
def test_payload_as_keyword_arguments(system):
    cart = system.spawn(CART, type=CART_TYPE, key="a")
    cart.send("add", qty=2)
    cart.send({"type": "add", "qty": 1})
    assert cart.snapshot().context == {"items": 3}
    assert cart.snapshot().configuration == ("filled",)


# @tg: verifies:: [[openspec:python-bindings#Actor Creation And Event Sending From Python#Repeated message id is applied once]] {level: integration}
def test_repeated_message_id_is_applied_once(system):
    cart = system.spawn(CART, type=CART_TYPE, key="a")
    assert cart.send("add", qty=2, message_id="m-1") == "m-1"
    cart.send("add", qty=2, message_id="m-1")
    assert cart.snapshot().context == {"items": 2}
    assert len(cart.journal()) == 1


# @tg: verifies:: [[openspec:python-bindings#Actor Creation And Event Sending From Python#Registered type activates by key]] {level: integration}
def test_registered_type_activates_by_key(system):
    reg = system.register(ORDER, type=ORDER_TYPE)
    assert reg["type"] == ORDER_TYPE
    a = system.actor(ORDER_TYPE, "x")
    a.send("pay")
    assert system.actor(ORDER_TYPE, "x").snapshot().configuration == ("paid",)
    assert system.actors() == [f"{ORDER_TYPE}/x"]
    assert system.spawn(type=ORDER_TYPE, key="y").snapshot().configuration == ("open",)


# @tg: verifies:: [[openspec:python-bindings#Snapshots And Subscriptions#One snapshot per changing macrostep]] {level: integration}
def test_one_snapshot_per_changing_macrostep(system):
    cart = system.spawn(CART, type=CART_TYPE, key="a")
    seen, moves = [], []
    sub = cart.subscribe(seen.append, transitions=moves.append)
    cart.send("add", qty=2)
    cart.send("add", qty=1)
    cart.send("pay")  # no transition takes it: journaled, no snapshot
    cart.send("checkout")
    assert [s.position for s in seen] == [1, 2, 4]
    assert [m["position"] for m in moves] == [1, 2, 4]
    assert moves[0]["event"]["name"] == "add"
    sub.cancel()
    cart.send("add", qty=1)  # a fourth change after cancelling: nobody hears it
    assert len(seen) == 3 and len(moves) == 3


# @tg: verifies:: [[openspec:python-bindings#Snapshots And Subscriptions#Callbacks may send events]] {level: integration}
def test_callbacks_may_send_events(system):
    cart = system.spawn(CART, type=CART_TYPE, key="a")
    sent = []

    def on_snapshot(s):
        sent.append(s.position)
        if s.matches("filled") and s.position == 1:
            cart.send("checkout")

    cart.subscribe(on_snapshot)
    cart.send("add", qty=1)
    assert cart.snapshot().configuration == ("ordered",)
    assert sent == [1, 2]


# @tg: verifies:: [[openspec:python-bindings#Journal Inspection And Time Travel#As of a position]] {level: integration}
def test_as_of_a_position(system):
    cart = system.spawn(CART, type=CART_TYPE, key="a")
    cart.send("add", qty=2)
    cart.send("add", qty=1)
    then = cart.as_of(position=1)
    assert then.configuration == ("filled",) and then.context == {"items": 2}
    assert cart.as_of(position=0).configuration == ("empty",)
    entries = cart.journal()
    assert [e["position"] for e in entries] == [1, 2]
    assert entries[0]["event"]["name"] == "add" and entries[0]["transitions"]
    assert [e["position"] for e in cart.journal(start=2)] == [2]
    with pytest.raises(vertep.InvalidArgument):
        cart.as_of()


# @tg: verifies:: [[openspec:python-bindings#Journal Inspection And Time Travel#Can does not send]] {level: integration}
def test_can_does_not_send(system):
    cart = system.spawn(CART, type=CART_TYPE, key="a")
    assert cart.can("add", qty=1) is False  # nothing is activated before the first message
    cart.send("add", qty=1)
    assert cart.can("add", qty=1) is True
    assert cart.can("checkout") is True
    assert cart.can("refund") is False
    assert cart.snapshot().position == 1


# @tg: verifies:: [[openspec:python-bindings#Journal Inspection And Time Travel#Graph query]] {level: integration}
def test_graph_query(system):
    cart = system.spawn(CART, type=CART_TYPE, key="a")
    cart.send("add", qty=2)
    ask = system.query("ASK { GRAPH ?g { ?s ?p ?o } }")
    assert ask == {"boolean": True}
    rows = system.query("SELECT ?g WHERE { GRAPH ?g { ?s ?p ?o } } LIMIT 1")
    assert rows["variables"] == ["g"] and len(rows["rows"]) == 1
    with pytest.raises(vertep.InvalidArgument):  # the memory store keeps no history
        system.query("ASK { ?s ?p ?o }", at=1.0)


# @tg: verifies:: [[openspec:python-bindings#Explicit Time#Virtual clock fires timers when advanced]] {level: integration}
def test_virtual_clock_fires_timers_when_advanced(system):
    order = system.spawn(ORDER, type=ORDER_TYPE, key="1")
    order.send("pay")
    assert order.snapshot().configuration == ("paid",)
    assert [t["event"] for t in system.pending_timers()] == ["ship"]
    system.advance(0.1)
    assert order.snapshot().configuration == ("paid",)
    system.advance(0.2)
    assert order.snapshot().configuration == ("shipped",)
    assert system.pending_timers() == []
    assert system.now() == pytest.approx(1_000.3)
    assert [e["position"] for e in order.journal()] == [1, 2]


# @tg: verifies:: [[openspec:python-bindings#Explicit Time#Journal replays under a different clock]] {level: integration}
def test_journal_replays_under_a_different_clock(tmp_path):
    db = tmp_path / "order.db"
    with ActorSystem(storage="file", path=db, clock="virtual", start=50.0) as s:
        order = s.spawn(ORDER, type=ORDER_TYPE, key="1")
        order.send("pay")
        s.advance(1)
        before = order.snapshot()
    with ActorSystem(storage="file", path=db, clock="virtual", start=9_999.0) as s:
        s.register(ORDER, type=ORDER_TYPE)
        after = s.actor(ORDER_TYPE, "1").snapshot()
    assert after == before and after.configuration == ("shipped",)


# @tg: verifies:: [[openspec:python-bindings#Explicit Time#System clock waits in real time]] {level: integration}
def test_system_clock_waits_in_real_time():
    with ActorSystem(storage="memory") as s:
        order = s.spawn(ORDER, type=ORDER_TYPE, key="1")
        order.send("pay")
        t0 = time.monotonic()
        s.wait(0.5)
        elapsed = time.monotonic() - t0
        assert order.snapshot().configuration == ("shipped",)
        assert 0.45 <= elapsed < 2.0
        with pytest.raises(vertep.InvalidArgument):
            s.advance(1)  # a system clock cannot be advanced


# @tg: verifies:: [[openspec:python-bindings#Durable File Storage#Journal survives reopening]] {level: integration}
def test_journal_survives_reopening(tmp_path):
    db = os.fspath(tmp_path / "cart.db")
    with ActorSystem(storage="file", path=db, clock="virtual") as s:
        cart = s.spawn(CART, type=CART_TYPE, key="a")
        cart.send("add", qty=2)
    with ActorSystem(storage="file", path=db, clock="virtual") as s:
        s.register(CART, type=CART_TYPE)
        cart = s.actor(CART_TYPE, "a")
        assert cart.snapshot().context == {"items": 2}
        assert s.actors() == [f"{CART_TYPE}/a"]
        assert s.query("ASK { GRAPH ?g { ?s ?p ?o } }") == {"boolean": True}


# @tg: verifies:: [[openspec:python-bindings#Durable File Storage#File is exclusive and closing releases it]] {level: integration}
def test_file_is_exclusive_and_closing_releases_it(tmp_path):
    db = tmp_path / "x.db"
    first = ActorSystem(storage="file", path=db, clock="virtual")
    with pytest.raises(vertep.StoreLocked):
        ActorSystem(storage="file", path=db, clock="virtual")
    first.close()
    first.close()  # closing twice does nothing
    ActorSystem(storage="file", path=db, clock="virtual").close()


# @tg: verifies:: [[openspec:python-bindings#Durable File Storage#Export journal bytes]] {level: integration}
def test_export_journal_bytes(tmp_path):
    with ActorSystem(storage="file", path=tmp_path / "a.db", clock="virtual") as s:
        s.spawn(CART, type=CART_TYPE, key="a").send("add", qty=1)
        data = s.export_journal()
    assert isinstance(data, bytes) and data.startswith(b"SQLite format 3\x00")
    with ActorSystem(storage="memory") as m:
        with pytest.raises(vertep.InvalidArgument):
            m.export_journal()


# @tg: verifies:: [[openspec:python-bindings#Typed Errors#Invalid chart lists its violations]] {level: unit}
def test_invalid_chart_lists_its_violations(system):
    with pytest.raises(vertep.ChartInvalid) as e:
        system.register("<scxml")
    assert isinstance(e.value, vertep.VertepError) and e.value.code == "ChartInvalid"
    assert e.value.violations and e.value.violations[0]["line"] == 1
    bad_target = ORDER.replace('target="paid"', 'target="nowhere"')
    with pytest.raises(vertep.ChartInvalid):
        system.register(bad_target)


# @tg: verifies:: [[openspec:python-bindings#Typed Errors#Misuse raises a typed error]] {level: unit}
def test_misuse_raises_a_typed_error(system):
    with pytest.raises(vertep.NotFound):
        system.spawn(type="https://example.org/charts/missing", key="1")
    with pytest.raises(vertep.InvalidArgument):
        system.spawn(ORDER, type="not an iri", key="1")
    with pytest.raises(vertep.InvalidArgument):
        ActorSystem(storage="disk")
    with pytest.raises(vertep.InvalidArgument):
        ActorSystem(storage="file")
    order = system.spawn(ORDER, type=ORDER_TYPE, key="1")
    with pytest.raises(vertep.InvalidArgument):
        order.send("pay", type="x")


# @tg: verifies:: [[openspec:python-bindings#Typed Errors#Closed system raises]] {level: unit}
def test_closed_system_raises():
    s = ActorSystem(storage="memory", clock="virtual")
    a = s.spawn(ORDER, type=ORDER_TYPE, key="1")
    s.close()
    assert s.closed
    with pytest.raises(vertep.SystemClosed):
        a.send("pay")
    with pytest.raises(vertep.SystemClosed):
        s.pump()


# @tg: verifies:: [[openspec:python-bindings#Python Guards And Actions#Guard and action bound by IRI]] {level: integration}
def test_guard_and_action_bound_by_iri():
    calls = []

    def positive(context, event):
        calls.append(event)
        return event["amount"] > 0

    def record(context, event):
        return {
            "context": {"total": event["amount"]},
            "raise": [{"type": "receipt", "amount": event["amount"]}],
        }

    with ActorSystem(
        storage="memory",
        clock="virtual",
        guards={"vt:guard/order/positive": positive},
        actions={"vt:action/order/record": record},
    ) as s:
        a = s.spawn(GUARDED, type=GUARDED_TYPE, key="1")
        a.send("pay", amount=0)
        assert a.snapshot().configuration == ("idle",)  # the guard refused
        a.send("pay", amount=5)
        snap = a.snapshot()
        # the raised `receipt` reached the final state, which ends the session
        assert snap.terminated and snap.configuration == ()
        assert snap.context == {"total": 5}
        assert calls  # the guard ran in Python


# @tg: verifies:: [[openspec:python-bindings#Python Guards And Actions#Impure guard is reported in dev mode]] {level: integration}
def test_impure_guard_is_reported_in_dev_mode():
    n = {"calls": 0}

    def flaky(context, event):
        n["calls"] += 1
        return n["calls"] == 1

    with ActorSystem(
        storage="memory",
        clock="virtual",
        dev=True,
        guards={"vt:guard/order/positive": flaky},
        actions={"vt:action/order/record": lambda c, e: None},
    ) as s:
        a = s.spawn(GUARDED, type=GUARDED_TYPE, key="1")
        with pytest.warns(vertep.ImpurityWarning, match="impure"):
            a.send("pay", amount=1)


# @tg: verifies:: [[openspec:python-bindings#Python Guards And Actions#Callback must be callable]] {level: unit}
def test_callback_must_be_callable():
    with pytest.raises(ValueError):
        ActorSystem(guards={"vt:guard/x": 3})  # type: ignore[dict-item]


# @tg: verifies:: [[openspec:python-bindings#Native Extension Module#Another thread is refused]] {level: unit}
def test_another_thread_is_refused(system):
    err = []

    def use():
        try:
            system.pump()
        except BaseException as e:  # noqa: BLE001
            err.append(e)

    t = threading.Thread(target=use)
    t.start()
    t.join()
    assert err and isinstance(err[0], BaseException)


# @tg: verifies:: [[openspec:python-bindings#Runnable Examples#Examples print their recorded output]] {level: e2e}
@pytest.mark.parametrize("name", ["quickstart", "cart", "durable_actor", "guards_actions"])
def test_examples_print_their_recorded_output(name):
    import subprocess
    import sys

    examples = os.path.join(os.path.dirname(__file__), "..", "..", "..", "examples", "python")
    run = subprocess.run(
        [sys.executable, "-I", os.path.join(examples, f"{name}.py")],
        capture_output=True,
        text=True,
        timeout=60,
    )
    assert run.returncode == 0, run.stderr
    with open(os.path.join(examples, f"{name}.expected-output.txt")) as f:
        assert run.stdout == f.read()


# @tg: verifies:: [[openspec:python-bindings#Package Publishing#Tag must match the package version]] {level: unit}
def test_tag_must_match_the_package_version():
    import subprocess
    import sys

    script = os.path.join(os.path.dirname(__file__), "..", "scripts", "check_version.py")
    good = subprocess.run([sys.executable, "-I", script, f"v{vertep.__version__}"], capture_output=True, text=True)
    assert good.returncode == 0, good.stderr
    for bad in ("v9.9.9", "0.1.5-nonsense", "main"):
        run = subprocess.run([sys.executable, "-I", script, bad], capture_output=True, text=True)
        assert run.returncode != 0 and "does not match" in run.stderr


# @tg: verifies:: [[openspec:python-bindings#Package Publishing#Wheels are smoke-tested before upload]] {level: e2e}
def test_the_release_smoke_script_passes_on_the_installed_package():
    # the script the release workflow runs on every freshly built wheel
    import subprocess
    import sys

    script = os.path.join(os.path.dirname(__file__), "..", "scripts", "smoke.py")
    run = subprocess.run([sys.executable, "-I", script], capture_output=True, text=True)
    assert run.returncode == 0, run.stderr
    assert run.stdout.strip() == f"vertep {vertep.__version__} ok"
