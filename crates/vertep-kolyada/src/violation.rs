//! Proof that a message was refused, and proof of equivocation.
//!
//! A [`Violation`] keeps the signed message with the reason it was refused, so any participant
//! can run [`Instance::check`] on it and reach the same verdict. An [`Equivocation`] is two
//! different validly signed messages from one role with one sequence number.

use serde::{Deserialize, Serialize};

use crate::envelope::{Message, Verifier};
use crate::instance::{Instance, Reason, Verdict};

/// A refused message with the reason, as recorded by the refuser.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Violation {
    /// The reason, as [`Reason::as_str`] gives it.
    pub reason: String,
    /// The signed message that was refused.
    pub message: Message,
}

impl Violation {
    /// Records a refusal.
    #[must_use]
    pub fn new(reason: &Reason, message: Message) -> Self {
        Self {
            reason: reason.as_str().to_owned(),
            message,
        }
    }

    /// Re-runs the check on the verifier's own state of the instance. True when the message is
    /// refused for the recorded reason there too.
    // @tg: implements:: [[openspec:protocol-conformance#Violation Records]]
    #[must_use]
    pub fn verify(&self, instance: &Instance, verifier: &dyn Verifier) -> bool {
        matches!(instance.check(&self.message, verifier), Verdict::Refused(r) if r.as_str() == self.reason)
    }
}

/// Two different signed messages from one role with one sequence number.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Equivocation {
    /// One message.
    pub a: Message,
    /// The other.
    pub b: Message,
}

impl Equivocation {
    /// True when both signatures verify, the messages are from the same sender, role, protocol,
    /// instance and sequence number, and they differ.
    // @tg: implements:: [[openspec:protocol-conformance#Violation Records]]
    #[must_use]
    pub fn verify(&self, verifier: &dyn Verifier) -> bool {
        let (x, y) = (&self.a.body, &self.b.body);
        self.a.verify(verifier).is_ok()
            && self.b.verify(verifier).is_ok()
            && x.sender == y.sender
            && x.role == y.role
            && x.protocol == y.protocol
            && x.instance == y.instance
            && x.seq == y.seq
            && self.a.hash() != self.b.hash()
    }
}
