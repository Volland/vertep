//! The wire form of shipping messages, in the codec of the cluster frames ([`crate::frame`]):
//! the format version, a tag, then fields as big-endian integers and length-prefixed strings.
//! An entry travels as the N-Quads of its one named graph, the wire form durable-object custody
//! uses for entries too, with its position and digest in front.

use vertep_store::oxrdf::{Dataset, NamedNode};
use vertep_store::{Entry, Position};

use super::{PullBatch, PullError, PullRequest, ShippedEntry};
use crate::fencing::Fence;
use crate::frame::{FrameError, Reader, Writer};
use crate::meta::NodeId;
use crate::placement::ShardId;

const TAG_PULL: u8 = 0x10;
const TAG_BATCH: u8 = 0x11;
const TAG_REFUSED: u8 = 0x12;

/// An entry as N-Quads text (one named graph).
pub fn entry_to_nquads(entry: &Entry) -> String {
    let mut w = oxttl::NQuadsSerializer::new().for_writer(Vec::new());
    for q in entry.quads() {
        // writing to a Vec cannot fail
        let _ = w.serialize_quad(&q);
    }
    String::from_utf8(w.finish()).unwrap_or_default()
}

/// The entry encoded by N-Quads text.
///
/// # Errors
///
/// [`PullError::Malformed`] for a syntax error or a dataset that is not one entry.
pub fn entry_from_nquads(text: &str) -> Result<Entry, PullError> {
    let mut ds = Dataset::new();
    for q in oxttl::NQuadsParser::new().for_slice(text.as_bytes()) {
        ds.insert(&q.map_err(|e| PullError::Malformed(format!("n-quads: {e}")))?);
    }
    Entry::from_dataset(&ds).map_err(|e| PullError::Malformed(e.to_string()))
}

fn malformed(e: FrameError) -> PullError {
    PullError::Malformed(e.to_string())
}

fn iri(s: String) -> Result<NamedNode, PullError> {
    NamedNode::new(s).map_err(|e| PullError::Malformed(e.to_string()))
}

impl PullRequest {
    /// The request's bytes.
    pub fn encode(&self) -> Vec<u8> {
        let mut w = Writer::new(TAG_PULL);
        w.u32(self.shard.get());
        w.str(self.actor.as_str());
        w.u64(self.after.get());
        w.u32(self.window);
        w.0
    }

    /// The request in `bytes`.
    ///
    /// # Errors
    ///
    /// [`PullError::Malformed`].
    pub fn decode(bytes: &[u8]) -> Result<Self, PullError> {
        let mut r = Reader(bytes);
        if r.header().map_err(malformed)? != TAG_PULL {
            return Err(PullError::Malformed("not a pull request".into()));
        }
        let shard = ShardId::new(r.u32().map_err(malformed)?);
        let actor = iri(r.str().map_err(malformed)?)?;
        let after = Position::new(r.u64().map_err(malformed)?);
        let window = r.u32().map_err(malformed)?;
        r.end().map_err(malformed)?;
        Ok(PullRequest {
            shard,
            actor,
            after,
            window,
        })
    }
}

fn digest(w: &mut Writer, d: &[u8; 32]) {
    w.0.extend_from_slice(d);
}

fn read_digest(r: &mut Reader<'_>) -> Result<[u8; 32], PullError> {
    let b = r.take(32).map_err(malformed)?;
    let mut d = [0u8; 32];
    d.copy_from_slice(b);
    Ok(d)
}

/// The leader's answer to a pull, as bytes for [`crate::Reply::Data`].
pub fn encode_reply(reply: &Result<PullBatch, PullError>) -> Vec<u8> {
    match reply {
        Ok(b) => {
            let mut w = Writer::new(TAG_BATCH);
            w.str(b.leader.as_str());
            w.u32(b.fence.shard.get());
            w.u64(b.fence.epoch);
            w.str(b.actor.as_str());
            w.u64(b.after.get());
            match &b.prev {
                Some(d) => {
                    w.u8(1);
                    digest(&mut w, d);
                }
                None => w.u8(0),
            }
            w.u64(b.head.get());
            w.u32(u32::try_from(b.entries.len()).unwrap_or(u32::MAX));
            for e in &b.entries {
                w.u64(e.position.get());
                digest(&mut w, &e.digest);
                w.str(&entry_to_nquads(&e.entry));
            }
            w.0
        }
        Err(e) => {
            let mut w = Writer::new(TAG_REFUSED);
            match e {
                PullError::NotLeader { shard } => {
                    w.u8(1);
                    w.u32(shard.get());
                }
                PullError::Behind { head } => {
                    w.u8(2);
                    w.u64(head.get());
                }
                PullError::TooLarge {
                    position,
                    size,
                    window,
                } => {
                    w.u8(3);
                    w.u64(position.get());
                    w.u64(*size as u64);
                    w.u64(*window as u64);
                }
                PullError::Store(m) => {
                    w.u8(4);
                    w.str(m);
                }
                PullError::Unreachable(m) => {
                    w.u8(5);
                    w.str(m);
                }
                PullError::Malformed(m) => {
                    w.u8(6);
                    w.str(m);
                }
            }
            w.0
        }
    }
}

/// The leader's answer in `bytes`: the batch, or the leader's refusal as `Ok(Err(..))`.
///
/// # Errors
///
/// [`PullError::Malformed`] when the bytes are not an answer.
pub fn decode_reply(bytes: &[u8]) -> Result<Result<PullBatch, PullError>, PullError> {
    let mut r = Reader(bytes);
    let out = match r.header().map_err(malformed)? {
        TAG_BATCH => {
            let leader = NodeId::new(r.str().map_err(malformed)?)
                .map_err(|e| PullError::Malformed(e.to_string()))?;
            let fence = Fence {
                shard: ShardId::new(r.u32().map_err(malformed)?),
                epoch: r.u64().map_err(malformed)?,
            };
            let actor = iri(r.str().map_err(malformed)?)?;
            let after = Position::new(r.u64().map_err(malformed)?);
            let prev = match r.u8().map_err(malformed)? {
                0 => None,
                1 => Some(read_digest(&mut r)?),
                other => return Err(PullError::Malformed(format!("prev flag {other}"))),
            };
            let head = Position::new(r.u64().map_err(malformed)?);
            let n = r.u32().map_err(malformed)? as usize;
            let mut entries = Vec::with_capacity(n.min(super::MAX_BATCH_ENTRIES));
            for _ in 0..n {
                let position = Position::new(r.u64().map_err(malformed)?);
                let digest = read_digest(&mut r)?;
                let entry = entry_from_nquads(&r.str().map_err(malformed)?)?;
                entries.push(ShippedEntry {
                    position,
                    digest,
                    entry,
                });
            }
            Ok(PullBatch {
                leader,
                fence,
                actor,
                after,
                prev,
                head,
                entries,
            })
        }
        TAG_REFUSED => Err(match r.u8().map_err(malformed)? {
            1 => PullError::NotLeader {
                shard: ShardId::new(r.u32().map_err(malformed)?),
            },
            2 => PullError::Behind {
                head: Position::new(r.u64().map_err(malformed)?),
            },
            3 => PullError::TooLarge {
                position: Position::new(r.u64().map_err(malformed)?),
                size: r.u64().map_err(malformed)? as usize,
                window: r.u64().map_err(malformed)? as usize,
            },
            4 => PullError::Store(r.str().map_err(malformed)?),
            5 => PullError::Unreachable(r.str().map_err(malformed)?),
            6 => PullError::Malformed(r.str().map_err(malformed)?),
            other => return Err(PullError::Malformed(format!("refusal code {other}"))),
        }),
        other => return Err(PullError::Malformed(format!("answer tag {other}"))),
    };
    r.end().map_err(malformed)?;
    Ok(out)
}
