"""Offline verification of an explanation bundle from Python: the committed fixture exported by
`crates/vertep-agent/tests/bundle_fixture.rs`, and tampered copies of it."""

import copy
import json
import os

import pytest

import vertep
from vertep import BundleInvalid, InvalidArgument

FIXTURE = os.path.join(
    os.path.dirname(__file__), "..", "..", "..", "crates", "vertep-agent", "tests", "fixtures", "review-bundle.json"
)

#: The fixture's chart with a rejection that escalates instead.
ESCALATING = """<scxml xmlns="http://www.w3.org/2005/07/scxml" xmlns:vt="https://w3id.org/vertep/vt#" version="1.0" datamodel="cel" initial="idle"><datamodel><data id="ok" expr="true"/></datamodel><state id="idle"><transition event="start" target="review"/></state>
  <state id="review" vt:decide="d" vt:candidates="approve reject">
    <transition event="approve" cond="ctx.ok" target="approved"/>
    <transition event="reject" target="escalated"/>
  </state>
  <state id="approved"/><state id="rejected"/><state id="escalated"/></scxml>"""


@pytest.fixture
def bundle():
    with open(FIXTURE) as f:
        return json.load(f)


# @tg: verifies:: [[openspec:decision-records#Explanation Bundle#A bundle verifies offline]] {level: integration}
def test_a_bundle_verifies_offline(bundle):
    with open(FIXTURE) as f:
        text = f.read()
    for given in (text, text.encode(), bundle):
        v = vertep.verify_bundle(given)
        assert v.actor == "https://example.org/agents/a1"
        assert (v.from_position, v.to_position) == (1, 3)
        assert v.decider_calls == 0
        assert [x["position"] for x in v.explanations] == [1, 2, 3]
    last = v.explanation(3)
    assert last["input"] == "decision"
    assert last["decision"]["chosen"] == "reject"
    assert last["state_before"] == ["review"] and last["state_after"] == ["rejected"]


# @tg: verifies:: [[openspec:decision-records#Explanation Bundle#An altered entry is detected]] {level: integration}
def test_an_altered_entry_is_detected(bundle):
    tampered = copy.deepcopy(bundle)
    tampered["entries"][1]["ts"] += 1
    with pytest.raises(BundleInvalid, match="entry 2"):
        vertep.verify_bundle(tampered)


# @tg: verifies:: [[openspec:decision-records#Explanation Bundle#A missing chart is detected]] {level: integration}
def test_a_missing_chart_is_detected(bundle):
    version = next(iter(bundle["charts"]))
    tampered = copy.deepcopy(bundle)
    tampered["charts"] = {}
    with pytest.raises(BundleInvalid) as e:
        vertep.verify_bundle(tampered)
    assert version in str(e.value) and e.value.code == "BundleInvalid"


def test_text_that_is_not_json_is_an_argument_error():
    with pytest.raises(InvalidArgument):
        vertep.verify_bundle("{")


def test_counterfactual_replay_of_a_bundle(bundle):
    report = vertep.counterfactual_bundle(bundle, ESCALATING)
    assert report["decisions"] == "recorded"
    assert report["first_divergence"] == 3
    step = report["steps"][2]
    assert step["original_state"] == ["rejected"] and step["counterfactual_state"] == ["escalated"]
    # the bundle is only read: it still verifies
    assert vertep.verify_bundle(bundle).decider_calls == 0
