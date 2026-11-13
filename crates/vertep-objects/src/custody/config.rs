//! The custody configuration of a durable object: its custodians with their roles, and the
//! replication quorum, read from and written to the object's DID document.

use core::fmt;
use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};
use vertep_federation::{DidDocument, Service, ServiceEndpoint};

/// The service type that declares a durable object's custodians in its DID document.
pub const CUSTODY_SERVICE: &str = "VertepCustody";
/// The URI of the endpoint that carries the quorum and the configuration version.
pub const CUSTODY_PARAMETERS: &str = "urn:vertep:custody";

/// What a custodian may do.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    /// May activate the object, write its journal and read its entries.
    Host,
    /// Stores, orders and verifies encrypted entries; cannot read them.
    Keeper,
}

impl Role {
    /// `host` or `keeper`.
    pub fn as_str(self) -> &'static str {
        match self {
            Role::Host => "host",
            Role::Keeper => "keeper",
        }
    }
}

/// One custodian.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Custodian {
    /// The custodian's DID.
    pub id: String,
    /// Its role.
    pub role: Role,
}

impl Custodian {
    /// A host.
    pub fn host(id: impl Into<String>) -> Self {
        Custodian {
            id: id.into(),
            role: Role::Host,
        }
    }
    /// A keeper.
    pub fn keeper(id: impl Into<String>) -> Self {
        Custodian {
            id: id.into(),
            role: Role::Keeper,
        }
    }
}

/// Why a custody declaration is invalid.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ConfigError {
    /// The quorum is not a majority of the custodian set.
    QuorumBelowMajority {
        /// The declared quorum.
        quorum: usize,
        /// The number of custodians.
        custodians: usize,
    },
    /// The quorum exceeds the custodian set.
    QuorumAboveSet {
        /// The declared quorum.
        quorum: usize,
        /// The number of custodians.
        custodians: usize,
    },
    /// No custodian is a host, so nobody could ever write.
    NoHost,
    /// A custodian is listed twice.
    Duplicate(String),
    /// The DID document declares no custody service, or an unreadable one.
    Missing(String),
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ConfigError::QuorumBelowMajority { quorum, custodians } => write!(
                f,
                "invalid custody declaration: a quorum of {quorum} is not a majority of {custodians} custodians"
            ),
            ConfigError::QuorumAboveSet { quorum, custodians } => write!(
                f,
                "invalid custody declaration: a quorum of {quorum} exceeds {custodians} custodians"
            ),
            ConfigError::NoHost => write!(f, "invalid custody declaration: no custodian is a host"),
            ConfigError::Duplicate(d) => write!(f, "invalid custody declaration: {d} listed twice"),
            ConfigError::Missing(m) => write!(f, "no custody declaration: {m}"),
        }
    }
}

impl std::error::Error for ConfigError {}

/// A durable object's custodian set and quorum, with a version that increases on every change.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CustodyConfig {
    /// The configuration version, from 1.
    pub version: u64,
    /// The custodians, in declaration order.
    pub custodians: Vec<Custodian>,
    /// How many custodians must accept an entry or promise an epoch.
    pub quorum: usize,
}

impl CustodyConfig {
    /// A validated configuration at version 1.
    ///
    /// # Errors
    ///
    /// [`ConfigError`] when the quorum is not a majority, a custodian is listed twice or no
    /// custodian is a host.
    // @tg: implements:: [[openspec:object-custody#Custodian Set In The DID Document]]
    pub fn new(custodians: Vec<Custodian>, quorum: usize) -> Result<Self, ConfigError> {
        let c = CustodyConfig {
            version: 1,
            custodians,
            quorum,
        };
        c.validate()?;
        Ok(c)
    }

    /// A configuration whose quorum is the smallest majority.
    ///
    /// # Errors
    ///
    /// As [`CustodyConfig::new`].
    pub fn majority(custodians: Vec<Custodian>) -> Result<Self, ConfigError> {
        let q = custodians.len() / 2 + 1;
        CustodyConfig::new(custodians, q)
    }

    /// Checks the rules every configuration obeys.
    ///
    /// # Errors
    ///
    /// [`ConfigError`].
    pub fn validate(&self) -> Result<(), ConfigError> {
        let n = self.custodians.len();
        if self.quorum > n {
            return Err(ConfigError::QuorumAboveSet {
                quorum: self.quorum,
                custodians: n,
            });
        }
        if self.quorum * 2 <= n {
            return Err(ConfigError::QuorumBelowMajority {
                quorum: self.quorum,
                custodians: n,
            });
        }
        let mut seen = BTreeSet::new();
        for c in &self.custodians {
            if !seen.insert(&c.id) {
                return Err(ConfigError::Duplicate(c.id.clone()));
            }
        }
        if !self.custodians.iter().any(|c| c.role == Role::Host) {
            return Err(ConfigError::NoHost);
        }
        Ok(())
    }

    /// The role of `did`, when it is a custodian.
    pub fn role_of(&self, did: &str) -> Option<Role> {
        self.custodians.iter().find(|c| c.id == did).map(|c| c.role)
    }

    /// True when `did` is a host.
    pub fn is_host(&self, did: &str) -> bool {
        self.role_of(did) == Some(Role::Host)
    }

    /// The hosts' DIDs.
    pub fn hosts(&self) -> impl Iterator<Item = &str> {
        self.custodians
            .iter()
            .filter(|c| c.role == Role::Host)
            .map(|c| c.id.as_str())
    }

    /// True when the custodians among `ids` form a quorum.
    pub fn is_quorum<'a>(&self, ids: impl IntoIterator<Item = &'a str>) -> bool {
        let ids: BTreeSet<&str> = ids.into_iter().collect();
        self.custodians
            .iter()
            .filter(|c| ids.contains(c.id.as_str()))
            .count()
            >= self.quorum
    }

    /// The custody service entry for a DID document: one endpoint per custodian (its DID, with
    /// `accept` naming the role) and one [`CUSTODY_PARAMETERS`] endpoint with the quorum and
    /// version.
    pub fn to_service(&self, did: &str) -> Service {
        let mut endpoints: Vec<ServiceEndpoint> = self
            .custodians
            .iter()
            .map(|c| ServiceEndpoint {
                uri: c.id.clone(),
                routing_keys: Vec::new(),
                accept: vec![c.role.as_str().to_owned()],
            })
            .collect();
        endpoints.push(ServiceEndpoint {
            uri: CUSTODY_PARAMETERS.into(),
            routing_keys: Vec::new(),
            accept: vec![
                format!("quorum={}", self.quorum),
                format!("version={}", self.version),
            ],
        });
        Service {
            id: format!("{did}#custody"),
            types: vec![CUSTODY_SERVICE.into()],
            endpoints,
        }
    }

    /// Reads and validates the custody declaration of `doc`.
    ///
    /// # Errors
    ///
    /// [`ConfigError::Missing`] when there is none or it is unreadable; the validation errors of
    /// [`CustodyConfig::validate`].
    // @tg: implements:: [[openspec:object-custody#Custodian Set In The DID Document]]
    pub fn from_document(doc: &DidDocument) -> Result<Self, ConfigError> {
        let svc = doc
            .service
            .iter()
            .find(|s| s.has_type(CUSTODY_SERVICE))
            .ok_or_else(|| ConfigError::Missing(format!("{} has no {CUSTODY_SERVICE}", doc.id)))?;
        let mut custodians = Vec::new();
        let mut quorum = None;
        let mut version = 1;
        for ep in &svc.endpoints {
            if ep.uri == CUSTODY_PARAMETERS {
                for a in &ep.accept {
                    if let Some(q) = a.strip_prefix("quorum=") {
                        quorum = Some(
                            q.parse::<usize>()
                                .map_err(|e| ConfigError::Missing(format!("quorum: {e}")))?,
                        );
                    } else if let Some(v) = a.strip_prefix("version=") {
                        version = v
                            .parse::<u64>()
                            .map_err(|e| ConfigError::Missing(format!("version: {e}")))?;
                    }
                }
                continue;
            }
            let role = match ep.accept.first().map(String::as_str) {
                Some("host") => Role::Host,
                Some("keeper") => Role::Keeper,
                other => {
                    return Err(ConfigError::Missing(format!(
                        "custodian {} has role {other:?}",
                        ep.uri
                    )));
                }
            };
            custodians.push(Custodian {
                id: ep.uri.clone(),
                role,
            });
        }
        let quorum = quorum.unwrap_or(custodians.len() / 2 + 1);
        let c = CustodyConfig {
            version,
            custodians,
            quorum,
        };
        c.validate()?;
        Ok(c)
    }
}
