//! The `vt:` runtime terms, as declared by `vertep-rdf/ontology/vt-runtime.ttl`.
//!
//! The storage layer's own terms (`vt:JournalEntry`, `vt:actor`, `vt:position`, `vt:head`,
//! `vt:fencingEpoch`, `vt:projectedPosition`, graph `vt:projection`) are re-exported from
//! [`vertep_store::vocab`], so both crates write the same IRIs.

use oxrdf::NamedNode;

pub use vertep_store::vocab::{
    ACTOR, FENCING_EPOCH, HEAD, JOURNAL_ENTRY, POSITION, PROJECTED_POSITION, PROJECTION_GRAPH,
    RDF_TYPE, VT,
};

macro_rules! terms {
    ($($(#[$m:meta])* $name:ident = $local:literal;)*) => {
        $($(#[$m])* pub const $name: &str = concat!("https://w3id.org/vertep/vt#", $local);)*
        /// Every runtime term this module names, for checks against the ontology file.
        pub const ALL: &[&str] = &[$($name),*];
    };
}

terms! {
    /// `vt:Address`.
    ADDRESS = "Address";
    /// `vt:Actor`.
    ACTOR_CLASS = "Actor";
    /// `vt:VirtualActor`.
    VIRTUAL_ACTOR = "VirtualActor";
    /// `vt:InvokedActor`.
    INVOKED_ACTOR = "InvokedActor";
    /// `vt:StatelessWorker`.
    STATELESS_WORKER = "StatelessWorker";
    /// `vt:chartType`.
    CHART_TYPE = "chartType";
    /// `vt:key`.
    KEY = "key";
    /// `vt:parent`.
    PARENT = "parent";
    /// `vt:status`.
    STATUS = "status";
    /// `vt:Running`.
    RUNNING = "Running";
    /// `vt:Terminated`.
    TERMINATED = "Terminated";
    /// `vt:inState`.
    IN_STATE = "inState";
    /// `vt:datamodelState`.
    DATAMODEL_STATE = "datamodelState";
    /// `vt:tookTransition`.
    TOOK_TRANSITION = "tookTransition";
    /// `vt:causedBy`.
    CAUSED_BY = "causedBy";
    /// `vt:Mailbox`.
    MAILBOX = "Mailbox";
    /// `vt:Message`.
    MESSAGE = "Message";
    /// `vt:recipient`.
    RECIPIENT = "recipient";
    /// `vt:sequence`.
    SEQUENCE = "sequence";
    /// `vt:envelope`.
    ENVELOPE = "envelope";
    /// `vt:Activation`.
    ACTIVATION = "Activation";
    /// `vt:InputEntry`.
    INPUT_ENTRY = "InputEntry";
    /// `vt:MigrationEntry`.
    MIGRATION_ENTRY = "MigrationEntry";
    /// `vt:ForkEntry`.
    FORK_ENTRY = "ForkEntry";
    /// `vt:Macrostep`.
    MACROSTEP = "Macrostep";
    /// `vt:InputEvent`.
    INPUT_EVENT = "InputEvent";
    /// `vt:OutgoingSend`.
    OUTGOING_SEND = "OutgoingSend";
    /// `vt:chartVersion`.
    CHART_VERSION = "chartVersion";
    /// `vt:messageId`.
    MESSAGE_ID = "messageId";
    /// `vt:timestamp`.
    TIMESTAMP = "timestamp";
    /// `vt:configurationDigest`.
    CONFIGURATION_DIGEST = "configurationDigest";
    /// `vt:terminatedHere`.
    TERMINATED_HERE = "terminatedHere";
    /// `vt:unhandled`.
    UNHANDLED = "unhandled";
    /// `vt:input`.
    INPUT = "input";
    /// `vt:send`.
    SEND = "send";
    /// `vt:schedules`.
    SCHEDULES = "schedules";
    /// `vt:cancels`.
    CANCELS = "cancels";
    /// `vt:firedTimer`.
    FIRED_TIMER = "firedTimer";
    /// `vt:invokes`.
    INVOKES = "invokes";
    /// `vt:eventName` (shared with the chart module).
    EVENT_NAME = "eventName";
    /// `vt:eventType`.
    EVENT_TYPE = "eventType";
    /// `vt:sendId` (shared with the chart module).
    SEND_ID = "sendId";
    /// `vt:invokeId` (shared with the chart module).
    INVOKE_ID = "invokeId";
    /// `vt:sendTarget` (shared with the chart module).
    SEND_TARGET = "sendTarget";
    /// `vt:order` (shared with the chart module).
    ORDER = "order";
    /// `vt:origin`.
    ORIGIN = "origin";
    /// `vt:originType`.
    ORIGIN_TYPE = "originType";
    /// `vt:payload`.
    PAYLOAD = "payload";
    /// `vt:sendType`.
    SEND_TYPE = "sendType";
    /// `vt:correlationId`.
    CORRELATION_ID = "correlationId";
    /// `vt:answers`.
    ANSWERS = "answers";
    /// `vt:requests`.
    REQUESTS = "requests";
    /// `vt:Snapshot`.
    SNAPSHOT = "Snapshot";
    /// `vt:snapshotOf`.
    SNAPSHOT_OF = "snapshotOf";
    /// `vt:sessionState`.
    SESSION_STATE = "sessionState";
    /// `vt:runtimeState`.
    RUNTIME_STATE = "runtimeState";
    /// `vt:Timer`.
    TIMER = "Timer";
    /// `vt:Reminder`.
    REMINDER = "Reminder";
    /// `vt:dueAt`.
    DUE_AT = "dueAt";
    /// `vt:period`.
    PERIOD = "period";
    /// `vt:reminderName`.
    REMINDER_NAME = "reminderName";
    /// `vt:scheduledBy`.
    SCHEDULED_BY = "scheduledBy";
    /// `vt:dispatched`.
    DISPATCHED = "dispatched";
    /// `vt:dispatchedThrough`.
    DISPATCHED_THROUGH = "dispatchedThrough";
    /// `vt:Migration`.
    MIGRATION = "Migration";
    /// `vt:migration`.
    MIGRATION_P = "migration";
    /// `vt:fromVersion`.
    FROM_VERSION = "fromVersion";
    /// `vt:toVersion`.
    TO_VERSION = "toVersion";
    /// `vt:snapshotBefore`.
    SNAPSHOT_BEFORE = "snapshotBefore";
    /// `vt:snapshotAfter`.
    SNAPSHOT_AFTER = "snapshotAfter";
    /// `vt:snapshot`.
    SNAPSHOT_P = "snapshot";
    /// `vt:forkOf`.
    FORK_OF = "forkOf";
    /// `vt:forkPosition`.
    FORK_POSITION = "forkPosition";
    /// `vt:DeadLetter`.
    DEAD_LETTER = "DeadLetter";
    /// `vt:receiver`.
    RECEIVER = "receiver";
    /// `vt:sender`.
    SENDER = "sender";
    /// `vt:reason`.
    REASON = "reason";
    /// `vt:recordedAt`.
    RECORDED_AT = "recordedAt";
}

/// The type IRI of `vt:request` sends: a send of this type (with `;timeout=<ms>` appended when
/// a timeout is declared) is a request whose correlation id the runtime assigns.
pub const REQUEST_SEND_TYPE: &str = "https://w3id.org/vertep/vt#request";
/// The type IRI of `vt:reminder` registrations (with `;name=..;period=..` or `;cancel`).
pub const REMINDER_SEND_TYPE: &str = "https://w3id.org/vertep/vt#reminder";
/// The type IRI of self-migration requests (with `;to=<version>;migration=<iri>`).
// @tg: implements:: [[openspec:chart-versioning#Self-Migration Requests]]
pub const MIGRATE_SEND_TYPE: &str = "https://w3id.org/vertep/vt#migrate";
/// The event name of a reply to a `vt:request`; the runtime copies the request's correlation id
/// onto every `vt:reply` sent while the request is being handled.
pub const REPLY_EVENT: &str = "vt:reply";
/// The event a request's timeout timer raises.
pub const TIMEOUT_EVENT: &str = "error.timeout";
/// The event raised by the `error` unhandled-event policy.
pub const UNHANDLED_EVENT: &str = "error.unhandled";

/// A [`NamedNode`] for one of the constants above.
pub fn node(iri: &str) -> NamedNode {
    NamedNode::new_unchecked(iri)
}
