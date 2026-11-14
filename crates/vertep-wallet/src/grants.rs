//! Grant credentials as wallet credentials (`did-identity` requirement `Grant Credentials From
//! A Wallet`, design D13).
//!
//! A grant credential is an ordinary credential (VCDM 2.0, Data Integrity `eddsa-jcs-2022`, of
//! type `vt:GrantCredential`, claims `target` and `events`), so it is issued into, held by and
//! presented from a wallet over any exchange protocol, and stays readable by
//! `vertep_federation::verify_grant_credential`. A presentation the target's verifier accepted
//! (signatures, holder binding, trust, validity and status) yields a [`Grant`] for the target's
//! inbound policy, which `vertep_federation::admit` enforces before any journal append.
//! [`PresentedGrants::refresh`] drops grants whose credential has since been revoked.

use serde_json::{Value, json};
use vertep_federation::vocab::GRANT_CREDENTIAL_TYPE;
use vertep_federation::{Grant, GrantSet};

use crate::credential::StatusRef;
use crate::error::WalletError;
use crate::exchange::{Offer, Verified};
use crate::plugin::StatusResolver;

/// The offer of a grant credential: the holder may send `events` to `target`.
pub fn grant_offer(
    target: &str,
    events: &[&str],
    valid_until: Option<i64>,
    status: Option<StatusRef>,
) -> Offer {
    Offer {
        format: crate::vocab::FORMAT_DI_JCS.to_owned(),
        types: vec![GRANT_CREDENTIAL_TYPE.to_owned()],
        claims: json!({"target": target, "events": events})
            .as_object()
            .cloned()
            .unwrap_or_default(),
        valid_until,
        status,
    }
}

/// The grant an accepted presentation of a grant credential carries, for the actor `target`.
///
/// # Errors
///
/// [`WalletError`] when the presentation is not of a grant credential, grants rights on
/// another actor, or names no events.
// @tg: implements:: [[openspec:did-identity#Grant Credentials From A Wallet]]
pub fn grant_from(verified: &Verified, target: &str) -> Result<Grant, WalletError> {
    if !verified.types.iter().any(|t| t == GRANT_CREDENTIAL_TYPE) {
        return Err(WalletError::Malformed("not a grant credential".into()));
    }
    let granted_target = verified
        .disclosed
        .get("target")
        .and_then(Value::as_str)
        .ok_or_else(|| WalletError::Malformed("grant without target".into()))?;
    if granted_target != target {
        return Err(WalletError::Malformed(format!(
            "the grant targets {granted_target}"
        )));
    }
    let events: Vec<String> = verified
        .disclosed
        .get("events")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default();
    if events.is_empty() {
        return Err(WalletError::Malformed("grant without events".into()));
    }
    let mut g = Grant::new(verified.holder.clone(), target, events);
    g.issuer = Some(verified.issuer.clone());
    g.id = Some(verified.credential.clone());
    Ok(g)
}

/// The grants an actor received as presented credentials.
#[derive(Clone, Debug, Default)]
pub struct PresentedGrants {
    entries: Vec<(Grant, Option<StatusRef>)>,
}

impl PresentedGrants {
    /// No grants.
    pub fn new() -> Self {
        Self::default()
    }

    /// Installs the grant of an accepted presentation for `target`.
    ///
    /// # Errors
    ///
    /// As [`grant_from`].
    pub fn install(&mut self, verified: &Verified, target: &str) -> Result<Grant, WalletError> {
        let g = grant_from(verified, target)?;
        self.entries.retain(|(e, _)| e.id != g.id);
        self.entries.push((g.clone(), verified.status.clone()));
        Ok(g)
    }

    /// Drops every grant whose credential is revoked, or whose status cannot be determined, at
    /// `now`; returns how many were dropped.
    pub fn refresh(&mut self, status: &dyn StatusResolver, now: i64) -> usize {
        let before = self.entries.len();
        self.entries.retain(|(g, s)| match (s, &g.issuer) {
            (Some(s), Some(issuer)) => matches!(status.is_set(s, issuer, now), Ok(false)),
            _ => true,
        });
        before - self.entries.len()
    }

    /// The grants, for an inbound policy.
    pub fn grant_set(&self) -> GrantSet {
        let mut set = GrantSet::new();
        for (g, _) in &self.entries {
            set.add(g.clone());
        }
        set
    }
}
