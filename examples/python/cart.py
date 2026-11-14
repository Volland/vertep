"""A cart on the CEL datamodel: subscriptions, the journal and `as_of`.

`add` carries a quantity, `checkout` needs a non-empty cart. A subscriber hears one snapshot
per macrostep that changed the configuration or the context; an event no transition takes
(`pay` here) is journaled and changes nothing, so nobody is told.

The same chart and the same calls run in JavaScript: see examples/browser-js.

Illustrates the python-bindings capability (Snapshots And Subscriptions, Journal Inspection
And Time Travel).

Run with `python examples/python/cart.py`.
"""

# @tg: implements:: [[openspec:python-bindings#Runnable Examples]]
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
    cart = system.spawn(CART, type="https://example.org/charts/cart", key="alice")
    print("actor", cart.address)

    def heard(snapshot):
        print(f"snapshot @{snapshot.position}: {list(snapshot.configuration)} items={snapshot.context['items']:g}")

    subscription = cart.subscribe(heard)

    # An event is its name plus its payload's fields.
    cart.send("add", qty=2)
    cart.send("add", qty=1)
    cart.send("pay")  # no transition takes it: journaled, no new snapshot
    print("can checkout:", cart.can("checkout"))
    cart.send("checkout")
    subscription.cancel()

    now = cart.snapshot()
    print("now:", list(now.configuration), "matches('ordered') =", now.matches("ordered"))

    # Every accepted event is a journal entry, with the transitions it took.
    print("journal:")
    for entry in cart.journal():
        data = entry["event"]["data"]
        taken = [t.split("#")[1] for t in entry["transitions"]]
        print(f"  {entry['position']} {entry['event']['name']:<8} {data!s:<15} -> {taken}")

    then = cart.as_of(position=1)
    print("as of entry 1:", list(then.configuration), f"items = {then.context['items']:g}")
