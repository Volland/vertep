//! Agent wallets for Vertep.
//!
//! A wallet receives, keeps, presents, issues and verifies verifiable credentials for one
//! identity. It is **protocol-based**: every exchange is a journaled run of a role chart
//! (issuer, holder or verifier) whose consent step is a `vt:decide` state. It is **pluggable**:
//! exchange protocols, credential formats, stores, consent policies and status resolvers are
//! traits in a [`plugin::WalletRegistry`]. And it is **verified**: every protocol maps its
//! messages onto one exchange vocabulary ([`vocab::Step`]), the conformance kit ([`testkit`])
//! records histories under an adversarial network, and the Lean checker `vertep-wallet` replays
//! them against the exchange model whose safety theorems are proven in `lean/vertep-lean`.
//!
//! - [`wallet::Wallet`]: the operations of the three roles; [`store::SealedStore`]: credentials
//!   encrypted at rest; [`consent`]: rule policies and deciders.
//! - [`exchange::Exchanges`]: the engine running role charts in an agent host.
//! - [`format`]: Data Integrity (`eddsa-jcs-2022`, `eddsa-rdfc-2022`) and SD-JWT VC.
//! - [`protocol`]: the Vertep Credential Exchange 1.0, and OpenID4VCI and OpenID4VP (feature
//!   `oid4vc`).
//! - [`status`]: W3C Bitstring Status List.
//!
//! The history format is specified in `docs/exchange-history.md`.
//!
//! @tg: implements:: [[decisions#The wallet is its own crate over federation, agents and the runtime]]
//! @tg: implements:: [[decisions#Wallets are virtual actors and exchanges are role charts]]
//! @tg: implements:: [[decisions#Wallet plugins behind five interfaces registered by IRI]]
//! @tg: implements:: [[decisions#One exchange vocabulary with value-free histories]]
//! @tg: implements:: [[decisions#Plugin conformance is trace refinement judged in Lean]]
//! @tg: implements:: [[decisions#Consent is a decide state with policy-computed candidates]]
//! @tg: implements:: [[decisions#Credentials are sealed under a wallet content key]]
//! @tg: implements:: [[decisions#Credential formats are implemented over the key manager]]
//! @tg: implements:: [[decisions#The Vertep Credential Exchange is a versioned protocol inside TSP]]
//! @tg: implements:: [[decisions#OpenID4VC endpoints are bindings of the role charts]]
//! @tg: implements:: [[decisions#Status lists are published credentials with a bounded cache]]
//! @tg: implements:: [[decisions#Grant credentials are wallet credentials]]

pub mod consent;
pub mod credential;
pub mod error;
pub mod exchange;
pub mod format;
pub mod grants;
pub mod history;
pub mod jose;
pub mod message;
pub mod plugin;
pub mod protocol;
pub mod status;
pub mod store;
#[cfg(feature = "testkit")]
pub mod testkit;
pub mod vocab;
pub mod wallet;

pub use credential::{Credential, CredentialMeta, Presentation, PresentationRequest};
pub use error::WalletError;
pub use exchange::{Exchanges, Offer, Wire};
pub use history::{History, Line};
pub use plugin::WalletRegistry;
pub use vocab::{Role, Step};
pub use wallet::{Wallet, WalletConfig};
