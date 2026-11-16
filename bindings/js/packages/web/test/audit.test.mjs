// Offline verification of an explanation bundle: the committed fixture exported by
// crates/vertep-agent/tests/bundle_fixture.rs, and tampered copies of it.
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { test } from "node:test";
import { workspace } from "./helpers.mjs";
import { VertepError, counterfactualBundle, verifyBundle } from "../dist/index.js";

const FIXTURE = readFileSync(join(workspace, "..", "..", "crates", "vertep-agent", "tests", "fixtures", "review-bundle.json"), "utf8");
const bundle = () => JSON.parse(FIXTURE);

// The fixture's chart with a rejection that escalates instead.
const ESCALATING = `<scxml xmlns="http://www.w3.org/2005/07/scxml" xmlns:vt="https://w3id.org/vertep/vt#" version="1.0" datamodel="cel" initial="idle"><datamodel><data id="ok" expr="true"/></datamodel><state id="idle"><transition event="start" target="review"/></state>
  <state id="review" vt:decide="d" vt:candidates="approve reject">
    <transition event="approve" cond="ctx.ok" target="approved"/>
    <transition event="reject" target="escalated"/>
  </state>
  <state id="approved"/><state id="rejected"/><state id="escalated"/></scxml>`;

const invalid = (pattern) => (e) => e instanceof VertepError && e.code === "BundleInvalid" && pattern.test(e.message);

// @tg: verifies:: [[openspec:decision-records#Explanation Bundle#A bundle verifies offline]] {level: integration}
test("a bundle verifies offline", async () => {
  for (const given of [FIXTURE, new TextEncoder().encode(FIXTURE), bundle()]) {
    const v = await verifyBundle(given);
    assert.equal(v.actor, "https://example.org/agents/a1");
    assert.deepEqual([v.from, v.to, v.deciderCalls], [1, 3, 0]);
    assert.deepEqual(v.explanations.map((x) => x.position), [1, 2, 3]);
  }
  const last = (await verifyBundle(FIXTURE)).explanations[2];
  assert.equal(last.input, "decision");
  assert.equal(last.decision.chosen, "reject");
  assert.deepEqual(last.state_after, ["rejected"]);
});

// @tg: verifies:: [[openspec:decision-records#Explanation Bundle#An altered entry is detected]] {level: integration}
test("an altered entry is detected", async () => {
  const b = bundle();
  b.entries[1].ts += 1;
  await assert.rejects(verifyBundle(b), invalid(/entry 2/));
});

// @tg: verifies:: [[openspec:decision-records#Explanation Bundle#A missing chart is detected]] {level: integration}
test("a missing chart is detected", async () => {
  const b = bundle();
  const [version] = Object.keys(b.charts);
  b.charts = {};
  await assert.rejects(verifyBundle(b), invalid(new RegExp(version)));
});

test("text that is not JSON is an argument error", async () => {
  await assert.rejects(verifyBundle("{"), (e) => e instanceof VertepError && e.code === "InvalidArgument");
});

test("a counterfactual replay of a bundle", async () => {
  const r = await counterfactualBundle(FIXTURE, ESCALATING);
  assert.equal(r.decisions, "recorded");
  assert.equal(r.first_divergence, 3);
  assert.deepEqual(r.steps[2].original_state, ["rejected"]);
  assert.deepEqual(r.steps[2].counterfactual_state, ["escalated"]);
});
