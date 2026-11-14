"""Shared charts and fixtures."""

import pytest

from vertep import ActorSystem

ORDER_TYPE = "https://example.org/charts/order"

#: Paying schedules a durable `ship` 200 ms later.
ORDER = """<scxml xmlns="http://www.w3.org/2005/07/scxml" version="1.0" initial="open">
  <state id="open">
    <transition event="pay" target="paid"><send event="ship" delay="200ms"/></transition>
  </state>
  <state id="paid"><transition event="ship" target="shipped"/></state>
  <state id="shipped"/>
</scxml>"""

CART_TYPE = "https://example.org/charts/cart"

#: A cart on the CEL datamodel: `add` carries a quantity, `checkout` needs a non-empty cart.
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

#: Named guards and actions bound from Python (the `rust` datamodel).
GUARDED_TYPE = "https://example.org/charts/guarded"
GUARDED = """<scxml xmlns="http://www.w3.org/2005/07/scxml" version="1.0" initial="idle" datamodel="rust">
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


@pytest.fixture
def system():
    """A virtual-clock, in-memory system with the order chart registered."""
    with ActorSystem(storage="memory", clock="virtual", start=1_000.0) as s:
        yield s
