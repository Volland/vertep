## MODIFIED Requirements

### Requirement: Macrostep Execution
The interpreter SHALL process one external event per macrostep and SHALL run to completion: after each microstep it SHALL take enabled eventless transitions, then dequeue internal events one at a time, until no eventless transition is enabled and the internal queue is empty (W3C SCXML Appendix D, `mainEventLoop`). The microstep budget SHALL charge every selection that fires transitions and every internal event consumed without enabling any transition.

#### Scenario: Internal events before the next external event
- **WHEN** processing external event `e1` raises internal events `i1` and `i2`, and external event `e2` is queued
- **THEN** `i1` and `i2` are processed in that order before `e2`

#### Scenario: Eventless transitions take priority over internal events
- **WHEN** after a microstep both an eventless transition is enabled and the internal queue holds an event
- **THEN** the eventless transition is taken first

#### Scenario: Unbounded eventless loop is cut by a budget
- **WHEN** a chart's eventless transitions form a cycle that never stabilizes
- **THEN** the interpreter stops after the configured microstep budget and reports a budget-exceeded error instead of running forever

#### Scenario: Error loop without transitions is cut by the budget
- **WHEN** a failing eventless guard raises `error.execution` on every round and no transition handles it
- **THEN** each consumed error event is charged to the budget and the macrostep ends with a budget-exceeded error and its partial trace
