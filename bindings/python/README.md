<p align="center">
  <a href="https://volland.github.io/vertep/"><img src="https://volland.github.io/vertep/assets/img/logo.svg" alt="Vertep" width="120"></a>
</p>

<h1 align="center">vertep</h1>

<p align="center">W3C SCXML statecharts as journaled actors, from Python</p>

<p align="center">
  <a href="https://pypi.org/project/vertep/"><img src="https://img.shields.io/pypi/v/vertep.svg" alt="PyPI"></a>
  <a href="https://github.com/Volland/vertep"><img src="https://img.shields.io/badge/github-Volland%2Fvertep-black.svg" alt="GitHub"></a>
  <a href="https://github.com/Volland/vertep/blob/main/LICENSE-MIT"><img src="https://img.shields.io/pypi/l/vertep.svg" alt="license"></a>
</p>

Vertep runs W3C SCXML statecharts as event-sourced actors. This package is the Python binding of the same Rust core that backs the Rust crates and the `@vertep/web` npm package: one chart, one journal format and one set of semantics in all three.

- Charts in SCXML, YAML or RDF, validated and content-addressed exactly as in the Rust runtime
- One journal entry per accepted event, with the transitions it took; time travel with `as_of`
- Durable timers, and a journal in a file that the Rust runtime and `@vertep/web` open as well
- The state graph queryable with SPARQL
- Guards and actions written in Python, and the CEL datamodel built in
- An explicit clock: real time for services, a virtual clock for deterministic tests

## Install

```sh
pip install vertep
```

CPython 3.9 or later. Wheels for Linux, macOS and Windows; no Rust toolchain needed.

## Use

```python
from vertep import ActorSystem

CART = """<scxml xmlns="http://www.w3.org/2005/07/scxml" version="1.0"
       datamodel="cel" name="cart" initial="empty">
  <datamodel><data id="items" expr="0"/></datamodel>
  <state id="empty">
    <transition event="add" target="filled"><assign location="items" expr="ctx.items + ev.data.qty"/></transition>
  </state>
  <state id="filled">
    <transition event="add"><assign location="items" expr="ctx.items + ev.data.qty"/></transition>
    <transition event="checkout" cond="ctx.items > 0" target="ordered"/>
  </state>
  <state id="ordered"/>
</scxml>"""

with ActorSystem(storage="memory", clock="virtual") as system:
    # A virtual actor is addressed by chart type and key; the first message activates it.
    cart = system.spawn(CART, type="https://example.org/charts/cart", key="alice")

    cart.subscribe(lambda s: print(s.position, s.configuration, s.context))

    cart.send("add", qty=2)       # an event is its name plus payload fields
    cart.send("add", qty=1)
    cart.send("checkout")

    print(cart.snapshot().matches("ordered"))   # True
    print(cart.as_of(position=1).context)       # {'items': 2.0}: the state after entry 1
    for entry in cart.journal():                # every event, with the transitions it took
        print(entry["position"], entry["event"]["name"])
```

A durable actor keeps its journal in a file and survives a restart, including timers that fell due while it was away:

```python
with ActorSystem(storage="file", path="orders.db") as system:
    order = system.spawn(ORDER, type="https://example.org/charts/order", key="42")
    order.send("pay")             # schedules `ship` in 200 ms, durably
    system.wait(0.5)              # real time passes; the timer fires
```

An auditor checks an agent's explanation bundle (exported by a Rust agent host) offline, with no actor system or decider:

```python
import vertep
v = vertep.verify_bundle(open("review-bundle.json").read())  # raises BundleInvalid if tampered
for x in v.explanations:
    print(x["position"], x["state_before"], "->", x["state_after"])
```

More in the repository: [examples/python](https://github.com/Volland/vertep/tree/main/examples/python) (a quick start, a cart, a durable actor, Python guards and actions) and the [documentation](https://volland.github.io/vertep/docs/python.html).

## Things to know

- A system belongs to the thread that created it, and the API is synchronous (no `asyncio` yet).
- CEL integers arrive as Python floats, as they do in JavaScript.
- Not yet bound: request/reply, the WebSocket transport, agent hosts (only bundle verification is), federation and the cluster. Use the Rust crates for those.

## License

MIT or Apache-2.0, at your option.
