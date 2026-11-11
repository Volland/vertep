//! Trust-boundary actors for Vertep: DID identities and resolution, key management, verified
//! origins, DID-scoped capability grants, the `vt-id:` identity ontology module, the
//! protocol-neutral relay envelope, and the message-layer cryptography of TSP (feature `tsp`)
//! and DIDComm v2 (feature `didcomm`).
//!
//! The pieces line up with the three layers of the design:
//!
//! - **Identity**: [`did`], [`resolver`] (`did:key`, `did:peer`, `did:web`, and `did:webvh` with
//!   feature `webvh`, behind a registry applications extend or override, with a bounded cache),
//!   [`directory`] (a DID address resolves to routes and keys), [`keys`] (the [`KeyManager`]
//!   seam), [`identity`] (actor DIDs, provisioning per actor type, rotation with a grace period).
//! - **Authority**: [`origin`] (verified origins and the inbound admission pipeline run before a
//!   journal append), [`grants`] (inbound and outbound grant checks), [`credential`] (grants as
//!   verifiable credentials).
//! - **Relay messages**: [`envelope`] (the JSON-LD event envelope), `tsp` and `didcomm`
//!   (sealing and opening messages through the key manager, forward wrapping, routed mode,
//!   relationships), behind the protocol seam [`relay`].
//! - **Runtime integration** (`vertep-runtime`): [`federation`] wires a node; [`outbox`] is the
//!   resolver of `did:` addresses and the relay transports (SCXML Event I/O Processors), with
//!   outbound grants, transport selection, fallback, retries and `error.communication`;
//!   [`inbox`] holds the relay mailboxes of local DID actors, fed by pickups from mediators,
//!   intermediaries and the node's own inbound queue ([`net`]), acknowledged after the journal
//!   append. `testkit` (feature `testkit`) has an in-process mediator and intermediary.
//!
//! Identity data is projected into RDF with [`ontology`] and validated by its SHACL shapes. See
//! `tg.md/views/distribution.md` for the architecture.
//!
//! @tg: implements:: [[decisions#Identity, mailbox and transport layers]] {note: "DID directory, relay mailboxes and relay transports on the runtime traits"}
//! @tg: implements:: [[decisions#Grants enforced before the journal]]
//! @tg: implements:: [[decisions#TSP before DIDComm across trust boundaries]]
//! @tg: implements:: [[decisions#Modular vt ontology on RDF 1.2]] {confidence: partial, note: "the vt-id identity module and its shapes"}

pub mod credential;
pub mod did;
#[cfg(feature = "didcomm")]
pub mod didcomm;
pub mod directory;
pub mod envelope;
pub mod error;
pub mod federation;
pub mod grants;
pub mod identity;
pub mod inbox;
pub mod keys;
pub mod net;
pub mod ontology;
pub mod origin;
pub mod outbox;
pub mod relay;
pub mod resolver;
#[cfg(all(feature = "testkit", any(feature = "didcomm", feature = "tsp")))]
pub mod testkit;
pub mod time;
#[cfg(feature = "tsp")]
pub mod tsp;
pub mod vocab;

pub use credential::{CredentialError, issue_grant_credential, verify_grant_credential};
pub use did::{Did, DidDocument, Service, ServiceEndpoint, VerificationMethod, is_did_address};
pub use directory::{DidDirectory, ResolvedAddress, Route, TransportKind};
pub use envelope::RelayEnvelope;
pub use error::{CommunicationError, CommunicationFailure};
pub use federation::{Federation, FederationBuilder, Identities, ProvisionError, RuntimeClock};
pub use grants::{Grant, GrantRefusal, GrantSet};
pub use identity::{ActorIdentity, DidMethodChoice, IdentityConfig, KeyRef};
pub use inbox::{PickupBinding, PickupReport, Recipients, RelayInbox, RelayMailbox};
pub use keys::{FileKeyManager, InMemoryKeyManager, KeyError, KeyManager, KeyType, PublicKey};
pub use net::{DropBox, LocalNetwork, NetError, PickupSource, RelayEndpoint, RelayNetwork};
pub use origin::{
    Admitted, AnonymousPolicy, InboundPolicy, InboundRecord, OpenedMessage, Refusal,
    VerifiedOrigin, admit,
};
pub use outbox::{OutboundOutcome, OutboundRecord, RelayOutbox, RetryPolicy};
pub use relay::{LocalActor, RelayProtocol, SendContext, SendError};
pub use resolver::{
    CachingResolver, DidResolver, HttpFetcher, Resolution, ResolutionError, ResolverRegistry,
};
pub use time::{Clock, ManualClock, SystemClock, Timestamp};
