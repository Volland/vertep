//! The `vt-mediator` configuration: a TOML file, overridden by `VT_MEDIATOR_*` environment
//! variables, and the mediator's identity, created on first start and kept in the data
//! directory with its keys.
//!
//! ```toml
//! listen = "0.0.0.0:8700"                      # VT_MEDIATOR_LISTEN
//! public_url = "https://mediator.example/tsp"  # VT_MEDIATOR_PUBLIC_URL, the TSP endpoint
//! data_dir = "/var/lib/vt-mediator"            # VT_MEDIATOR_DATA_DIR
//! did = "peer"                                 # or "web:mediator.example" or
//!                                              # "webvh:mediator.example[:path]"; VT_MEDIATOR_DID
//! shards = 4                                   # mailbox shard files (one writer each)
//! direct_delivery = true
//! allow = []                                   # VIDs allowed to register; empty: open
//!
//! [limits]
//! max_messages = 10000
//! max_bytes = 67108864
//! retention_secs = 1209600
//! max_batch = 100
//! ```

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::Deserialize;
use vertep_federation::{
    ActorIdentity, DidMethodChoice, FileKeyManager, IdentityConfig, KeyManager, Service,
    ServiceEndpoint,
};

use crate::mediator::{Limits, MediatorOptions, RegistrationPolicy};

/// Mailbox limits as configured.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct LimitsConfig {
    /// The most messages per mailbox.
    pub max_messages: u64,
    /// The most bytes per mailbox.
    pub max_bytes: u64,
    /// How long a message waits before it expires, in seconds.
    pub retention_secs: u64,
    /// The most messages in one batch.
    pub max_batch: usize,
}

impl Default for LimitsConfig {
    fn default() -> Self {
        let l = Limits::default();
        LimitsConfig {
            max_messages: l.max_messages,
            max_bytes: l.max_bytes,
            retention_secs: u64::try_from(l.retention_ms / 1000).unwrap_or(0),
            max_batch: l.max_batch,
        }
    }
}

/// The configuration of `vt-mediator`.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct MediatorConfig {
    /// The socket address to listen on.
    pub listen: String,
    /// The public URL of the TSP endpoint, published in the DID document; the live endpoint is
    /// this URL with `/live` appended.
    pub public_url: String,
    /// Where keys, the identity and the mailbox shard files live.
    pub data_dir: PathBuf,
    /// `peer` for a `did:peer:2`, or `web:<host>[:<path>]` for a `did:web` served by this
    /// mediator.
    pub did: String,
    /// Mailbox shard files.
    pub shards: usize,
    /// Try the recipient's own endpoint before storing at the last hop.
    pub direct_delivery: bool,
    /// VIDs allowed to register; empty means open registration.
    pub allow: Vec<String>,
    /// Mailbox limits.
    pub limits: LimitsConfig,
}

impl Default for MediatorConfig {
    fn default() -> Self {
        MediatorConfig {
            listen: "0.0.0.0:8700".into(),
            public_url: "http://localhost:8700/tsp".into(),
            data_dir: PathBuf::from("vt-mediator-data"),
            did: "peer".into(),
            shards: 4,
            direct_delivery: true,
            allow: Vec::new(),
            limits: LimitsConfig::default(),
        }
    }
}

/// Why the configuration or identity could not be loaded.
#[derive(Debug)]
pub struct ConfigError(pub String);

impl std::fmt::Display for ConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "configuration: {}", self.0)
    }
}

impl std::error::Error for ConfigError {}

impl MediatorConfig {
    /// Reads `path` (when given) and applies the environment overrides.
    ///
    /// # Errors
    ///
    /// [`ConfigError`] when the file cannot be read or parsed, or a value is invalid.
    pub fn load(path: Option<&Path>) -> Result<Self, ConfigError> {
        let mut c = match path {
            Some(p) => {
                let text = std::fs::read_to_string(p)
                    .map_err(|e| ConfigError(format!("{}: {e}", p.display())))?;
                Self::parse(&text)?
            }
            None => MediatorConfig::default(),
        };
        c.apply_env(|k| std::env::var(k).ok());
        c.validate()?;
        Ok(c)
    }

    /// Parses TOML text.
    ///
    /// # Errors
    ///
    /// [`ConfigError`] for invalid TOML or unknown keys.
    pub fn parse(text: &str) -> Result<Self, ConfigError> {
        toml::from_str(text).map_err(|e| ConfigError(e.to_string()))
    }

    /// Applies `VT_MEDIATOR_LISTEN`, `VT_MEDIATOR_PUBLIC_URL`, `VT_MEDIATOR_DATA_DIR`,
    /// `VT_MEDIATOR_DID` and `VT_MEDIATOR_SHARDS` from `env`.
    pub fn apply_env(&mut self, env: impl Fn(&str) -> Option<String>) {
        if let Some(v) = env("VT_MEDIATOR_LISTEN") {
            self.listen = v;
        }
        if let Some(v) = env("VT_MEDIATOR_PUBLIC_URL") {
            self.public_url = v;
        }
        if let Some(v) = env("VT_MEDIATOR_DATA_DIR") {
            self.data_dir = PathBuf::from(v);
        }
        if let Some(v) = env("VT_MEDIATOR_DID") {
            self.did = v;
        }
        if let Some(n) = env("VT_MEDIATOR_SHARDS").and_then(|v| v.parse().ok()) {
            self.shards = n;
        }
    }

    fn validate(&self) -> Result<(), ConfigError> {
        url::Url::parse(&self.public_url)
            .map_err(|e| ConfigError(format!("public_url {}: {e}", self.public_url)))?;
        if self.shards == 0 {
            return Err(ConfigError("shards must be at least 1".into()));
        }
        if self.did != "peer" && !self.did.starts_with("web:") && !self.did.starts_with("webvh:") {
            return Err(ConfigError(format!(
                "did must be \"peer\", \"web:<host>\" or \"webvh:<host>[:<path>]\", not {}",
                self.did
            )));
        }
        Ok(())
    }

    /// The mediator options this configuration gives.
    pub fn options(&self) -> MediatorOptions {
        MediatorOptions {
            limits: Limits {
                max_messages: self.limits.max_messages,
                max_bytes: self.limits.max_bytes,
                retention_ms: i64::try_from(self.limits.retention_secs.saturating_mul(1000))
                    .unwrap_or(i64::MAX),
                max_batch: self.limits.max_batch,
            },
            policy: if self.allow.is_empty() {
                RegistrationPolicy::Open
            } else {
                RegistrationPolicy::Allow(self.allow.iter().cloned().collect::<BTreeSet<_>>())
            },
            direct_delivery: self.direct_delivery,
        }
    }

    /// The directory of the mailbox shard files.
    pub fn mailbox_dir(&self) -> PathBuf {
        self.data_dir.join("mailboxes")
    }

    /// The key manager over `<data_dir>/keys`.
    ///
    /// # Errors
    ///
    /// [`ConfigError`] when the directory cannot be created.
    pub fn key_manager(&self) -> Result<Arc<dyn KeyManager>, ConfigError> {
        Ok(Arc::new(
            FileKeyManager::open(self.data_dir.join("keys"))
                .map_err(|e| ConfigError(e.to_string()))?,
        ))
    }

    /// The mediator's identity from `<data_dir>/identity.json`, created (with new keys in `km`)
    /// on first use.
    ///
    /// # Errors
    ///
    /// [`ConfigError`] when it cannot be read, created or saved.
    pub fn identity(&self, km: &dyn KeyManager) -> Result<ActorIdentity, ConfigError> {
        let path = self.data_dir.join("identity.json");
        if path.exists() {
            let text = std::fs::read(&path)
                .map_err(|e| ConfigError(format!("{}: {e}", path.display())))?;
            return serde_json::from_slice(&text)
                .map_err(|e| ConfigError(format!("{}: {e}", path.display())));
        }
        let services = vec![Service {
            id: "#tsp".into(),
            types: vec!["TSPTransport".into()],
            endpoints: vec![ServiceEndpoint::uri(&self.public_url)],
        }];
        std::fs::create_dir_all(&self.data_dir).map_err(|e| ConfigError(e.to_string()))?;
        let identity = if let Some(rest) = self.did.strip_prefix("webvh:") {
            self.create_webvh(km, rest, services)?
        } else {
            let method = match self.did.strip_prefix("web:") {
                Some(host) => DidMethodChoice::Hosted {
                    did: format!("did:web:{host}"),
                    services,
                },
                None => DidMethodChoice::Peer { services },
            };
            ActorIdentity::provision(km, &IdentityConfig::new(method))
                .map_err(|e| ConfigError(e.to_string()))?
        };
        let json = serde_json::to_vec_pretty(&identity).map_err(|e| ConfigError(e.to_string()))?;
        std::fs::write(&path, json).map_err(|e| ConfigError(format!("{}: {e}", path.display())))?;
        Ok(identity)
    }
}

impl MediatorConfig {
    /// The directory of the mediator's own `did:webvh` log (`did = "webvh:..."`).
    pub fn webvh_dir(&self) -> PathBuf {
        self.data_dir.join("webvh")
    }

    /// Creates the mediator's `did:webvh` (`<host>[:<path>]`) with pre-rotation: its identity,
    /// its genesis log entry, and the names of its update keys, kept in [`Self::webvh_dir`].
    // @tg: implements:: [[openspec:webvh-hosting#Mediator Under did:webvh]]
    // @tg: implements:: [[decisions#The mediator creates and serves its own did:webvh]]
    fn create_webvh(
        &self,
        km: &dyn KeyManager,
        spec: &str,
        services: Vec<Service>,
    ) -> Result<ActorIdentity, ConfigError> {
        let mut parts = spec.split(':');
        let host = parts
            .next()
            .filter(|h| !h.is_empty())
            .ok_or_else(|| ConfigError(format!("did {}: no host", self.did)))?
            .replace("%3A", ":")
            .replace("%3a", ":");
        let path: Vec<&str> = parts.collect();
        let options = vertep_webvh::CreateOptions::new(host)
            .path(&path.join(":"))
            .pre_rotation(true);
        let err = |e: &dyn std::fmt::Display| ConfigError(format!("did:webvh: {e}"));
        let mut identity =
            vertep_webvh::document::provision(km, &options, services).map_err(|e| err(&e))?;
        let doc = vertep_webvh::document::identity_document(&identity, km).map_err(|e| err(&e))?;
        let keys = self.key_manager()?;
        let mut author = vertep_webvh::Author::generate(keys, true).map_err(|e| err(&e))?;
        let log = author.create(&options, &doc).map_err(|e| err(&e))?;
        vertep_webvh::document::bind(&mut identity, log.did());
        let dir = self.webvh_dir();
        std::fs::create_dir_all(&dir).map_err(|e| ConfigError(e.to_string()))?;
        let write = |name: &str, text: &str| {
            let p = dir.join(name);
            std::fs::write(&p, text).map_err(|e| ConfigError(format!("{}: {e}", p.display())))
        };
        write("did.jsonl", &log.jsonl())?;
        write(
            "update-keys.json",
            &serde_json::to_string_pretty(author.update_keys())
                .map_err(|e| ConfigError(e.to_string()))?,
        )?;
        Ok(identity)
    }

    /// The mediator's own `did:webvh` log, when it has one: the URL path of its directory
    /// (`/.well-known` or `/<path>`) and the log.
    ///
    /// # Errors
    ///
    /// [`ConfigError`] when the log cannot be read.
    pub fn webvh_log(&self) -> Result<Option<(String, vertep_webvh::DidLog)>, ConfigError> {
        if !self.did.starts_with("webvh:") {
            return Ok(None);
        }
        let p = self.webvh_dir().join("did.jsonl");
        let text = std::fs::read_to_string(&p)
            .map_err(|e| ConfigError(format!("{}: {e}", p.display())))?;
        let log = vertep_webvh::DidLog::from_files(&text, None)
            .map_err(|e| ConfigError(e.to_string()))?;
        let dir = vertep_webvh::log_dir(log.did()).map_err(|e| ConfigError(e.to_string()))?;
        let base: Vec<String> = dir
            .iter()
            .skip(1)
            .map(|c| c.to_string_lossy().into_owned())
            .collect();
        Ok(Some((format!("/{}", base.join("/")), log)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_then_environment() {
        let mut c = MediatorConfig::parse(
            r#"
            listen = "127.0.0.1:9000"
            public_url = "https://m.example/tsp"
            did = "web:m.example"
            [limits]
            max_messages = 5
            retention_secs = 60
            "#,
        )
        .unwrap();
        c.apply_env(|k| (k == "VT_MEDIATOR_LISTEN").then(|| "0.0.0.0:1".to_owned()));
        assert_eq!(c.listen, "0.0.0.0:1");
        assert_eq!(c.options().limits.max_messages, 5);
        assert_eq!(c.options().limits.retention_ms, 60_000);
        assert!(c.validate().is_ok());
        assert!(MediatorConfig::parse("nonsense = 1").is_err());
    }
}
