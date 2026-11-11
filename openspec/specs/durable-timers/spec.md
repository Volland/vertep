# durable-timers Specification

## Purpose
Makes SCXML delayed sends and Orleans-style reminders durable: a timer is recorded in the journal by the macrostep that schedules it, survives crashes and deactivation, and fires as an ordinary journaled input event.

## Requirements

### Requirement: Delayed Send Is Journaled
A `<send>` with a `delay` or `delayexpr` SHALL be recorded as a timer in the journal entry of the macrostep that executed it, with its send id, due time, target and event, and SHALL NOT be held only in memory.

#### Scenario: Timer recorded with its macrostep
- **WHEN** a macrostep executes `<send event="remind" delay="5s" id="r1"/>`
- **THEN** its journal entry contains a timer with send id `r1` and a due time 5 s after the macrostep's journaled timestamp

#### Scenario: Timer survives a crash
- **WHEN** the node crashes after the entry was appended and before the timer is due
- **THEN** after restart the timer still fires at its due time

### Requirement: Timer Fires As Input
When a timer becomes due, the runtime SHALL deliver its event to the target actor's mailbox with a message id derived from the timer's send id, and the event SHALL be processed and journaled like any other external input.

#### Scenario: Due timer delivered
- **WHEN** the timer `r1` becomes due
- **THEN** the actor receives `remind` as an external event and appends a journal entry for it

#### Scenario: Timer fires once
- **WHEN** the timer service delivers a due timer twice because of a retry
- **THEN** the actor processes the event once, because both deliveries carry the same message id

### Requirement: Timer Cancellation
`<cancel sendid="...">` SHALL cancel a pending timer by recording the cancellation in the journal entry of the cancelling macrostep, and a cancelled timer SHALL NOT fire.

#### Scenario: Cancel before due
- **WHEN** a macrostep executes `<cancel sendid="r1"/>` before `r1` is due
- **THEN** the cancellation is journaled and `remind` is never delivered

#### Scenario: Cancel after fired
- **WHEN** a cancellation is processed after the timer already fired
- **THEN** the cancellation has no effect and raises no error

### Requirement: Timers Reactivate Virtual Actors
A due timer for a virtual actor that is not active SHALL cause its activation, so that a deactivated actor needs no running process to keep its timers.

#### Scenario: Reminder wakes an idle actor
- **WHEN** a virtual actor schedules a timer for 1 hour and is then deactivated for idleness
- **THEN** after 1 hour it is activated and processes the timer event

### Requirement: Timer Time Source
Due times SHALL be computed from the journaled timestamp of the scheduling macrostep, and the time at which a timer fired SHALL be recorded in the entry that processes it, so that replay never reads a clock.

#### Scenario: Replay reproduces due times
- **WHEN** an actor's journal is replayed
- **THEN** every timer it scheduled gets the same due time as originally, and replay does not deliver timers that already fired

### Requirement: Recurring Reminders
A chart SHALL be able to declare a `vt:reminder` with a name, a period and an event, which the runtime SHALL fire at each period as a timer, journaling each occurrence, until a `vt:reminder` cancellation for that name is executed or the actor reaches a final state.

#### Scenario: Periodic reminder
- **WHEN** an actor registers reminder `poll` with a period of 10 min
- **THEN** it receives the reminder event about every 10 min, each occurrence as a separate journaled input

#### Scenario: Reminder stops at final
- **WHEN** the actor reaches a top-level final state
- **THEN** its reminders are cancelled and no further occurrences fire

### Requirement: Missed Timers After Downtime
When the runtime starts after downtime, it SHALL fire every timer whose due time has passed, in order of due time, and SHALL fire a recurring reminder at most once for all occurrences missed during the downtime.

#### Scenario: Catching up after downtime
- **WHEN** the node was down while two one-shot timers and five occurrences of one reminder became due
- **THEN** after start the two timers fire in due-time order and the reminder fires once
