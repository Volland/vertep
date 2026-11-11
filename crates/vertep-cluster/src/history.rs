//! Histories recorded by fault injection, their checker, and their export for a differential
//! oracle.
//!
//! A fault-injection run records every lease grant, every journal append with the decision
//! the fenced store made, every outbound frame with the decision the receiver's epoch filter
//! made, and every message acknowledgement. [`History::check`] then verifies the
//! single-activation guarantee on the record:
//!
//! 1. every actor journal is linear (versions 1, 2, 3, ... with no gap);
//! 2. no journal version holds two entries, and no `(shard, epoch)` had two writers;
//! 3. every acknowledged message was processed exactly once (one journal entry);
//! 4. no fenced side effect was accepted: a frame is admitted only if the append that
//!    produced it was accepted, and each receiver admits a shard's frames in non-decreasing
//!    epoch order.
//!
//! [`History::to_jsonl`] writes the record as JSON lines for an external oracle (the Lean
//! lease and fencing model), one object per event, in order:
//!
//! ```text
//! {"op":"grant","node":"n1","shard":3,"epoch":5}
//! {"op":"append","node":"n1","shard":3,"epoch":5,"actor":"urn:a","version":7,"message":"m9","decision":"accept"}
//! {"op":"frame","receiver":"r0","shard":3,"epoch":5,"message":"m9","decision":"admit"}
//! {"op":"ack","message":"m9"}
//! ```
//!
//! `decision` is `accept`, `fenced` or `conflict` for appends (`version` is the version the
//! append expected to write) and `admit` or `fenced` for frames. An oracle replays the
//! grants, appends and frames through its model and must reach the same decision for each.

use core::fmt;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;

use crate::meta::NodeId;
use crate::placement::ShardId;

/// The decision a fenced journal store made on an append.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum AppendDecision {
    /// Stored.
    Accept,
    /// Refused: a higher epoch was accepted for the shard.
    Fenced,
    /// Refused: the journal was not at the expected version.
    Conflict,
}

/// The decision a receiver's epoch filter made on a frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FrameDecision {
    /// Admitted and delivered.
    Admit,
    /// Dropped into the dead-letter graph.
    Fenced,
}

/// One recorded event.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HistoryEvent {
    /// The lease authority granted `shard` to `node` at `epoch`.
    Grant {
        /// The node.
        node: NodeId,
        /// The shard.
        shard: ShardId,
        /// The epoch.
        epoch: u64,
    },
    /// `node` appended to `actor`'s journal under `epoch`, expecting to write `version`.
    Append {
        /// The writer.
        node: NodeId,
        /// The shard.
        shard: ShardId,
        /// The writer's lease epoch.
        epoch: u64,
        /// The actor.
        actor: String,
        /// The version the append expected to write.
        version: u64,
        /// The message the entry processes.
        message: String,
        /// The store's decision.
        decision: AppendDecision,
    },
    /// A frame produced by the append of `message` under `(shard, epoch)` reached `receiver`.
    Frame {
        /// The receiving node.
        receiver: NodeId,
        /// The producing shard.
        shard: ShardId,
        /// The producing epoch.
        epoch: u64,
        /// The message whose processing produced the frame.
        message: String,
        /// The receiver's decision.
        decision: FrameDecision,
    },
    /// `message` was acknowledged to its sender.
    Ack {
        /// The message.
        message: String,
    },
}

/// A recorded run.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct History {
    /// The events, in the order they happened.
    pub events: Vec<HistoryEvent>,
}

/// A violation of the single-activation guarantee found in a history.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Violation {
    /// An actor's journal skips or repeats a version.
    NotLinear {
        /// The actor.
        actor: String,
        /// The version expected next.
        expected: u64,
        /// The version found.
        found: u64,
    },
    /// Two writers were accepted under one `(shard, epoch)`.
    TwoWriters {
        /// The shard.
        shard: ShardId,
        /// The epoch.
        epoch: u64,
        /// The writers.
        nodes: Vec<NodeId>,
    },
    /// An append was accepted under a lower epoch than one already accepted for the shard.
    EpochRegressed {
        /// The shard.
        shard: ShardId,
        /// The epoch accepted.
        epoch: u64,
        /// The higher epoch accepted before.
        after: u64,
    },
    /// An acknowledged message was processed a number of times other than once.
    NotOnce {
        /// The message.
        message: String,
        /// How many entries process it.
        times: usize,
    },
    /// A frame was admitted although its append was not accepted, or after a higher epoch.
    FencedEffectAccepted {
        /// The message.
        message: String,
        /// The receiver.
        receiver: NodeId,
    },
}

impl fmt::Display for Violation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Violation::NotLinear {
                actor,
                expected,
                found,
            } => write!(f, "{actor}: expected version {expected}, found {found}"),
            Violation::TwoWriters {
                shard,
                epoch,
                nodes,
            } => {
                write!(f, "{shard} epoch {epoch} had writers {nodes:?}")
            }
            Violation::EpochRegressed {
                shard,
                epoch,
                after,
            } => {
                write!(f, "{shard}: epoch {epoch} accepted after {after}")
            }
            Violation::NotOnce { message, times } => {
                write!(f, "acknowledged message {message} processed {times} times")
            }
            Violation::FencedEffectAccepted { message, receiver } => {
                write!(f, "{receiver} admitted a fenced effect of {message}")
            }
        }
    }
}

impl std::error::Error for Violation {}

/// What a successful check counted.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CheckReport {
    /// Appends accepted.
    pub accepted: usize,
    /// Appends fenced.
    pub fenced: usize,
    /// Appends that lost compare-and-set.
    pub conflicts: usize,
    /// Frames admitted.
    pub admitted: usize,
    /// Frames dropped as fenced.
    pub dropped: usize,
    /// Messages acknowledged.
    pub acked: usize,
}

impl History {
    /// An empty history.
    pub fn new() -> Self {
        Self::default()
    }

    /// Records `e`.
    pub fn record(&mut self, e: HistoryEvent) {
        self.events.push(e);
    }

    /// Checks the single-activation guarantee on the record.
    ///
    /// # Errors
    ///
    /// The first [`Violation`] found.
    // @tg: implements:: [[openspec:shard-placement#Single Activation Guarantee]]
    pub fn check(&self) -> Result<CheckReport, Violation> {
        let mut r = CheckReport::default();
        let mut next: BTreeMap<&str, u64> = BTreeMap::new();
        let mut writers: BTreeMap<(ShardId, u64), BTreeSet<&NodeId>> = BTreeMap::new();
        let mut highest: BTreeMap<ShardId, u64> = BTreeMap::new();
        let mut processed: BTreeMap<&str, usize> = BTreeMap::new();
        let mut accepted_msgs: BTreeSet<(&str, ShardId, u64)> = BTreeSet::new();
        let mut admitted: BTreeMap<(&NodeId, ShardId), u64> = BTreeMap::new();
        let mut acked: BTreeSet<&str> = BTreeSet::new();
        for e in &self.events {
            match e {
                HistoryEvent::Grant { .. } => {}
                HistoryEvent::Append {
                    node,
                    shard,
                    epoch,
                    actor,
                    version,
                    message,
                    decision,
                } => match decision {
                    AppendDecision::Accept => {
                        r.accepted += 1;
                        let n = next.entry(actor).or_insert(1);
                        if *version != *n {
                            return Err(Violation::NotLinear {
                                actor: actor.clone(),
                                expected: *n,
                                found: *version,
                            });
                        }
                        *n += 1;
                        let w = writers.entry((*shard, *epoch)).or_default();
                        w.insert(node);
                        if w.len() > 1 {
                            return Err(Violation::TwoWriters {
                                shard: *shard,
                                epoch: *epoch,
                                nodes: w.iter().map(|n| (*n).clone()).collect(),
                            });
                        }
                        let h = highest.entry(*shard).or_insert(0);
                        if *epoch < *h {
                            return Err(Violation::EpochRegressed {
                                shard: *shard,
                                epoch: *epoch,
                                after: *h,
                            });
                        }
                        *h = *epoch;
                        *processed.entry(message).or_default() += 1;
                        accepted_msgs.insert((message, *shard, *epoch));
                    }
                    AppendDecision::Fenced => r.fenced += 1,
                    AppendDecision::Conflict => r.conflicts += 1,
                },
                HistoryEvent::Frame {
                    receiver,
                    shard,
                    epoch,
                    message,
                    decision,
                } => match decision {
                    FrameDecision::Admit => {
                        r.admitted += 1;
                        let seen = admitted.entry((receiver, *shard)).or_insert(0);
                        if !accepted_msgs.contains(&(message.as_str(), *shard, *epoch))
                            || *epoch < *seen
                        {
                            return Err(Violation::FencedEffectAccepted {
                                message: message.clone(),
                                receiver: receiver.clone(),
                            });
                        }
                        *seen = *epoch;
                    }
                    FrameDecision::Fenced => r.dropped += 1,
                },
                HistoryEvent::Ack { message } => {
                    acked.insert(message);
                }
            }
        }
        for m in &acked {
            let times = processed.get(m).copied().unwrap_or(0);
            if times != 1 {
                return Err(Violation::NotOnce {
                    message: (*m).to_owned(),
                    times,
                });
            }
        }
        r.acked = acked.len();
        Ok(r)
    }

    /// The history as JSON lines, in the format of the module documentation.
    pub fn to_jsonl(&self) -> String {
        let mut out = String::new();
        for e in &self.events {
            match e {
                HistoryEvent::Grant { node, shard, epoch } => {
                    let _ = writeln!(
                        out,
                        r#"{{"op":"grant","node":{},"shard":{},"epoch":{epoch}}}"#,
                        json(node.as_str()),
                        shard.get()
                    );
                }
                HistoryEvent::Append {
                    node,
                    shard,
                    epoch,
                    actor,
                    version,
                    message,
                    decision,
                } => {
                    let d = match decision {
                        AppendDecision::Accept => "accept",
                        AppendDecision::Fenced => "fenced",
                        AppendDecision::Conflict => "conflict",
                    };
                    let _ = writeln!(
                        out,
                        r#"{{"op":"append","node":{},"shard":{},"epoch":{epoch},"actor":{},"version":{version},"message":{},"decision":"{d}"}}"#,
                        json(node.as_str()),
                        shard.get(),
                        json(actor),
                        json(message)
                    );
                }
                HistoryEvent::Frame {
                    receiver,
                    shard,
                    epoch,
                    message,
                    decision,
                } => {
                    let d = match decision {
                        FrameDecision::Admit => "admit",
                        FrameDecision::Fenced => "fenced",
                    };
                    let _ = writeln!(
                        out,
                        r#"{{"op":"frame","receiver":{},"shard":{},"epoch":{epoch},"message":{},"decision":"{d}"}}"#,
                        json(receiver.as_str()),
                        shard.get(),
                        json(message)
                    );
                }
                HistoryEvent::Ack { message } => {
                    let _ = writeln!(out, r#"{{"op":"ack","message":{}}}"#, json(message));
                }
            }
        }
        out
    }
}

/// A JSON string literal.
fn json(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn n(s: &str) -> NodeId {
        NodeId::new(s).unwrap()
    }

    fn append(node: &str, epoch: u64, version: u64, msg: &str, d: AppendDecision) -> HistoryEvent {
        HistoryEvent::Append {
            node: n(node),
            shard: ShardId::new(1),
            epoch,
            actor: "urn:a".into(),
            version,
            message: msg.into(),
            decision: d,
        }
    }

    #[test]
    fn a_clean_history_passes() {
        let mut h = History::new();
        h.record(append("n1", 1, 1, "m1", AppendDecision::Accept));
        h.record(append("n2", 2, 2, "m2", AppendDecision::Accept));
        h.record(append("n1", 1, 3, "m3", AppendDecision::Fenced));
        h.record(HistoryEvent::Ack {
            message: "m1".into(),
        });
        let r = h.check().unwrap();
        assert_eq!((r.accepted, r.fenced, r.acked), (2, 1, 1));
        assert_eq!(h.to_jsonl().lines().count(), 4);
        assert!(h.to_jsonl().contains(r#""decision":"fenced""#));
    }

    #[test]
    fn violations_are_found() {
        let mut h = History::new();
        h.record(append("n1", 1, 1, "m1", AppendDecision::Accept));
        h.record(append("n2", 1, 2, "m2", AppendDecision::Accept));
        assert!(matches!(h.check(), Err(Violation::TwoWriters { .. })));

        let mut h = History::new();
        h.record(append("n1", 1, 1, "m1", AppendDecision::Accept));
        h.record(append("n1", 1, 1, "m2", AppendDecision::Accept));
        assert!(matches!(h.check(), Err(Violation::NotLinear { .. })));

        let mut h = History::new();
        h.record(append("n1", 1, 1, "m1", AppendDecision::Accept));
        h.record(append("n1", 1, 2, "m1", AppendDecision::Accept));
        h.record(HistoryEvent::Ack {
            message: "m1".into(),
        });
        assert!(matches!(
            h.check(),
            Err(Violation::NotOnce { times: 2, .. })
        ));

        let mut h = History::new();
        h.record(HistoryEvent::Frame {
            receiver: n("r"),
            shard: ShardId::new(1),
            epoch: 1,
            message: "m9".into(),
            decision: FrameDecision::Admit,
        });
        assert!(matches!(
            h.check(),
            Err(Violation::FencedEffectAccepted { .. })
        ));
    }
}
