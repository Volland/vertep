//! IRIs of the wallet, its built-in plugins, the Vertep Credential Exchange, and the exchange
//! vocabulary every protocol maps its messages onto.

use serde::{Deserialize, Serialize};

/// The wallet namespace: built-in plugin IRIs and ontology terms.
pub const WALLET_NS: &str = "https://w3id.org/vertep/wallet#";
/// The Vertep Credential Exchange 1.0 namespace: its message types.
pub const EXCHANGE_NS: &str = "https://w3id.org/vertep/exchange/1.0#";
/// The chart types of the wallet actor and the role charts.
pub const CHARTS: &str = "https://w3id.org/vertep/wallet/charts/";

/// Built-in exchange protocol: the Vertep Credential Exchange 1.0 (over TSP or any wire).
pub const PROTOCOL_VCE: &str = "https://w3id.org/vertep/wallet#protocol/vce-1.0";
/// Built-in exchange protocol: OpenID4VCI 1.0, pre-authorised code flow.
pub const PROTOCOL_OID4VCI: &str = "https://w3id.org/vertep/wallet#protocol/oid4vci-1.0";
/// Built-in exchange protocol: OpenID4VP 1.0 with DCQL and `direct_post`.
pub const PROTOCOL_OID4VP: &str = "https://w3id.org/vertep/wallet#protocol/oid4vp-1.0";

/// Built-in format: VCDM 2.0 with Data Integrity `eddsa-jcs-2022`.
pub const FORMAT_DI_JCS: &str = "https://w3id.org/vertep/wallet#format/vc-di-eddsa-jcs-2022";
/// Built-in format: VCDM 2.0 with Data Integrity `eddsa-rdfc-2022`.
pub const FORMAT_DI_RDFC: &str = "https://w3id.org/vertep/wallet#format/vc-di-eddsa-rdfc-2022";
/// Built-in format: SD-JWT VC with EdDSA.
pub const FORMAT_SD_JWT: &str = "https://w3id.org/vertep/wallet#format/dc+sd-jwt";

/// Built-in credential store: sealed records.
pub const STORE_SEALED: &str = "https://w3id.org/vertep/wallet#store/sealed";
/// Built-in consent policy: rules (default: refuse).
pub const CONSENT_RULES: &str = "https://w3id.org/vertep/wallet#consent/rules";
/// Built-in status resolver: W3C Bitstring Status List.
pub const STATUS_BITSTRING: &str = "https://w3id.org/vertep/wallet#status/bitstring";

/// The exchange vocabulary: the abstract steps every protocol maps each message onto
/// (`credential-exchange` requirement `Exchange Vocabulary`).
// @tg: implements:: [[openspec:credential-exchange#Exchange Vocabulary]]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Step {
    /// An issuer offers a credential with its nonce.
    Offer,
    /// A holder proves possession of a key over the issuer's nonce.
    ProvePossession,
    /// An issuer issues a credential bound to a key.
    Issue,
    /// A holder stores a credential.
    Store,
    /// A verifier requests a presentation with its nonce, audience and requested claims.
    RequestPresentation,
    /// A holder consents to disclose claims to a verifier for a nonce.
    Consent,
    /// A holder presents a credential.
    Present,
    /// A verifier accepts or refuses a presentation.
    Verify,
    /// A party refuses.
    Refuse,
    /// An issuer revokes a credential.
    Revoke,
}

impl Step {
    /// The step's name in histories.
    pub fn as_str(self) -> &'static str {
        match self {
            Step::Offer => "offer",
            Step::ProvePossession => "prove-possession",
            Step::Issue => "issue",
            Step::Store => "store",
            Step::RequestPresentation => "request-presentation",
            Step::Consent => "consent",
            Step::Present => "present",
            Step::Verify => "verify",
            Step::Refuse => "refuse",
            Step::Revoke => "revoke",
        }
    }

    /// Every step.
    pub const ALL: [Step; 10] = [
        Step::Offer,
        Step::ProvePossession,
        Step::Issue,
        Step::Store,
        Step::RequestPresentation,
        Step::Consent,
        Step::Present,
        Step::Verify,
        Step::Refuse,
        Step::Revoke,
    ];
}

/// The three roles of an exchange.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    /// Issues credentials.
    Issuer,
    /// Holds and presents credentials.
    Holder,
    /// Requests and verifies presentations.
    Verifier,
}

impl Role {
    /// The role's name.
    pub fn as_str(self) -> &'static str {
        match self {
            Role::Issuer => "issuer",
            Role::Holder => "holder",
            Role::Verifier => "verifier",
        }
    }
}
