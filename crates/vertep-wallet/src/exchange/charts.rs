//! The generic role charts and the wallet chart (SCXML, CEL datamodel). Protocols reuse them
//! through [`crate::plugin::ExchangeProtocol::role_chart`]; each maps its wire messages onto the
//! same events.

use crate::vocab::{CHARTS, Role};

/// The issuer role chart: one instance per offer, final after its one issuance.
// @tg: implements:: [[openspec:credential-exchange#Issuance At Most Once Per Offer]]
pub const ISSUER: &str = include_str!("charts/issuer.scxml");
/// The holder role chart.
pub const HOLDER: &str = include_str!("charts/holder.scxml");
/// The verifier role chart.
pub const VERIFIER: &str = include_str!("charts/verifier.scxml");
/// The wallet actor's chart.
pub const WALLET: &str = include_str!("charts/wallet.scxml");

/// The generic chart of `role`.
// @tg: implements:: [[openspec:credential-exchange#Role Charts]]
pub fn generic(role: Role) -> &'static str {
    match role {
        Role::Issuer => ISSUER,
        Role::Holder => HOLDER,
        Role::Verifier => VERIFIER,
    }
}

/// The chart type IRI of `role`'s chart.
pub fn chart_type(role: Role) -> String {
    format!("{CHARTS}{}", role.as_str())
}

/// The wallet chart's type IRI.
pub fn wallet_type() -> String {
    format!("{CHARTS}wallet")
}

/// Every built-in chart with its name, for validation and certification.
pub fn all() -> [(&'static str, &'static str); 4] {
    [
        ("issuer", ISSUER),
        ("holder", HOLDER),
        ("verifier", VERIFIER),
        ("wallet", WALLET),
    ]
}
