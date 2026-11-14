//! Role projection: a protocol restricted to what one role sends and receives.
//!
//! Every transition becomes a step of the role's automaton: a send when the role is the sender,
//! a receive when it is a receiver, and a silent step otherwise. The interface view lists, for
//! each state, the messages the role accepts and the roles that may send them.

use std::collections::{BTreeMap, BTreeSet};

use crate::document::{Protocol, Transition};

/// What a step does for the projected role.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    /// The role sends `message` to `receivers`.
    Send {
        /// The message type.
        message: String,
        /// The receiving roles.
        receivers: BTreeSet<String>,
    },
    /// The role receives `message` from `sender`.
    Receive {
        /// The message type.
        message: String,
        /// The sending role.
        sender: String,
    },
    /// The role takes no part: the state changes without it seeing a message.
    Silent,
}

/// One step of a projection.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Step {
    /// The state it leaves.
    pub from: String,
    /// The state it enters.
    pub to: String,
    /// What the role does.
    pub action: Action,
}

/// A protocol as one role runs it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Projection {
    /// The role.
    pub role: String,
    /// The start state.
    pub initial: String,
    /// The final states.
    pub finals: BTreeSet<String>,
    /// The steps, sorted by source state, message and target.
    pub steps: Vec<Step>,
}

/// What a role accepts in each state.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Interface {
    /// The role.
    pub role: String,
    /// For each state, the accepted messages and who may send them, as `(message, sender)`.
    pub accepts: BTreeMap<String, Vec<(String, String)>>,
}

fn step_of(role: &str, t: &Transition) -> Step {
    let action = if t.sender == role {
        Action::Send {
            message: t.message.clone(),
            receivers: t.receivers.clone(),
        }
    } else if t.receivers.contains(role) {
        Action::Receive {
            message: t.message.clone(),
            sender: t.sender.clone(),
        }
    } else {
        Action::Silent
    };
    Step {
        from: t.from.clone(),
        to: t.to.clone(),
        action,
    }
}

impl Protocol {
    /// The projection onto `role`; `None` if the protocol has no such role.
    ///
    /// The result depends only on the protocol and the role, so it is the same however often
    /// and in whatever order roles are projected.
    // @tg: implements:: [[openspec:agent-protocols#Role Projection]]
    #[must_use]
    pub fn project(&self, role: &str) -> Option<Projection> {
        if !self.has_role(role) {
            return None;
        }
        let mut steps: Vec<Step> = self
            .transitions()
            .iter()
            .map(|t| step_of(role, t))
            .collect();
        steps.sort_by(|a, b| {
            (&a.from, &a.to, format!("{:?}", a.action)).cmp(&(
                &b.from,
                &b.to,
                format!("{:?}", b.action),
            ))
        });
        Some(Projection {
            role: role.to_owned(),
            initial: self.initial().to_owned(),
            finals: self.finals().clone(),
            steps,
        })
    }

    /// The interface view of `role`: what it accepts in each state; `None` for an unknown role.
    // @tg: implements:: [[openspec:agent-protocols#Role Projection]]
    #[must_use]
    pub fn interface(&self, role: &str) -> Option<Interface> {
        let projection = self.project(role)?;
        let mut accepts: BTreeMap<String, Vec<(String, String)>> = BTreeMap::new();
        for s in projection.steps {
            if let Action::Receive { message, sender } = s.action {
                accepts.entry(s.from).or_default().push((message, sender));
            }
        }
        Some(Interface {
            role: role.to_owned(),
            accepts,
        })
    }
}
