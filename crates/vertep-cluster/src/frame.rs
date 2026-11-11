//! The cluster wire format: one self-delimiting frame per message, and the receiver's reply.
//!
//! On a stream every frame is a 4-byte big-endian length followed by that many bytes of
//! payload. A message payload holds, in order: the format version, a tag, the message
//! identifier, the sender and target IRIs, the sender's `(shard, epoch)` fence when it is a
//! virtual actor, the forwarding hop count, the event name and the event data. Strings and
//! byte strings are a 4-byte big-endian length and the bytes. The format depends on nothing
//! platform-specific.

use core::fmt;

use crate::fencing::Fence;
use crate::meta::NodeId;
use crate::placement::ShardId;
use crate::routing::Redirect;

/// The version byte of the format.
pub const FORMAT_VERSION: u8 = 1;

/// The default maximum frame payload size: 1 MiB.
pub const DEFAULT_MAX_FRAME: usize = 1 << 20;

const TAG_MESSAGE: u8 = 1;
const TAG_ACK: u8 = 2;
const TAG_FORWARDED: u8 = 3;
const TAG_ERROR: u8 = 4;

/// A message between actors on different nodes.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
// @tg: implements:: [[openspec:cluster-transport#Message Framing]]
pub struct Frame {
    /// The message identifier, the deduplication key.
    pub id: String,
    /// The sending actor's IRI.
    pub sender: String,
    /// The target actor's IRI.
    pub target: String,
    /// The sender's shard and lease epoch, when the sender is a virtual actor.
    pub fence: Option<Fence>,
    /// How many times the message was forwarded between nodes.
    pub hops: u8,
    /// The event name.
    pub event: String,
    /// The event data, as encoded by the runtime.
    pub data: Vec<u8>,
}

/// Why a receiver did not accept a frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RejectKind {
    /// The frame exceeds the receiver's maximum size.
    TooLarge,
    /// The frame's fence is below the highest epoch the receiver has seen for the shard; it
    /// went to the dead-letter graph.
    Fenced,
    /// The message reached the forwarding hop limit.
    HopLimit,
    /// The frame could not be decoded.
    Malformed,
    /// The receiver cannot take the message now; retry later.
    Unavailable,
}

impl RejectKind {
    fn code(self) -> u8 {
        match self {
            RejectKind::TooLarge => 1,
            RejectKind::Fenced => 2,
            RejectKind::HopLimit => 3,
            RejectKind::Malformed => 4,
            RejectKind::Unavailable => 5,
        }
    }

    fn from_code(c: u8) -> Result<Self, FrameError> {
        Ok(match c {
            1 => RejectKind::TooLarge,
            2 => RejectKind::Fenced,
            3 => RejectKind::HopLimit,
            4 => RejectKind::Malformed,
            5 => RejectKind::Unavailable,
            other => return Err(FrameError::Malformed(format!("reject code {other}"))),
        })
    }
}

/// The receiver's answer to one frame, on the stream it arrived on.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Reply {
    /// The message is stored in the target's mailbox or journal (or was a duplicate of one
    /// that is).
    Ack {
        /// The message identifier.
        id: String,
    },
    /// The receiver does not own the target's shard: it forwarded the message and tells the
    /// sender the owner it knows.
    Forwarded {
        /// The message identifier.
        id: String,
        /// The owner and epoch to route the shard to from now on.
        redirect: Redirect,
    },
    /// The message was not accepted.
    Rejected {
        /// The message identifier, empty when the frame could not be read.
        id: String,
        /// Why.
        kind: RejectKind,
        /// A description.
        message: String,
    },
}

impl Reply {
    /// The message identifier the reply is for.
    pub fn id(&self) -> &str {
        match self {
            Reply::Ack { id } | Reply::Forwarded { id, .. } | Reply::Rejected { id, .. } => id,
        }
    }
}

/// Why bytes are not a frame.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FrameError {
    /// The payload exceeds the maximum size.
    TooLarge {
        /// The payload size.
        size: usize,
        /// The maximum.
        max: usize,
    },
    /// Another format version.
    Version(u8),
    /// The payload is malformed.
    Malformed(String),
}

impl fmt::Display for FrameError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FrameError::TooLarge { size, max } => {
                write!(f, "frame of {size} bytes exceeds the maximum of {max}")
            }
            FrameError::Version(v) => write!(f, "unknown frame format version {v}"),
            FrameError::Malformed(m) => write!(f, "malformed frame: {m}"),
        }
    }
}

impl std::error::Error for FrameError {}

struct Writer(Vec<u8>);

impl Writer {
    fn new(tag: u8) -> Self {
        Writer(vec![FORMAT_VERSION, tag])
    }
    fn u8(&mut self, v: u8) {
        self.0.push(v);
    }
    fn u32(&mut self, v: u32) {
        self.0.extend_from_slice(&v.to_be_bytes());
    }
    fn u64(&mut self, v: u64) {
        self.0.extend_from_slice(&v.to_be_bytes());
    }
    fn bytes(&mut self, b: &[u8]) {
        // a frame never reaches 4 GiB: the maximum frame size is checked before sending
        self.u32(u32::try_from(b.len()).unwrap_or(u32::MAX));
        self.0.extend_from_slice(b);
    }
    fn str(&mut self, s: &str) {
        self.bytes(s.as_bytes());
    }
}

struct Reader<'a>(&'a [u8]);

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], FrameError> {
        if self.0.len() < n {
            return Err(FrameError::Malformed("truncated".into()));
        }
        let (a, b) = self.0.split_at(n);
        self.0 = b;
        Ok(a)
    }
    fn u8(&mut self) -> Result<u8, FrameError> {
        Ok(self.take(1)?[0])
    }
    fn u32(&mut self) -> Result<u32, FrameError> {
        let b = self.take(4)?;
        Ok(u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
    }
    fn u64(&mut self) -> Result<u64, FrameError> {
        let b = self.take(8)?;
        let mut a = [0u8; 8];
        a.copy_from_slice(b);
        Ok(u64::from_be_bytes(a))
    }
    fn bytes(&mut self) -> Result<&'a [u8], FrameError> {
        let n = self.u32()? as usize;
        self.take(n)
    }
    fn str(&mut self) -> Result<String, FrameError> {
        String::from_utf8(self.bytes()?.to_vec())
            .map_err(|_| FrameError::Malformed("a string is not UTF-8".into()))
    }
    fn header(&mut self) -> Result<u8, FrameError> {
        let v = self.u8()?;
        if v != FORMAT_VERSION {
            return Err(FrameError::Version(v));
        }
        self.u8()
    }
    fn end(&self) -> Result<(), FrameError> {
        if self.0.is_empty() {
            Ok(())
        } else {
            Err(FrameError::Malformed("trailing bytes".into()))
        }
    }
}

impl Frame {
    /// The frame's payload, without the length prefix.
    pub fn encode(&self) -> Vec<u8> {
        let mut w = Writer::new(TAG_MESSAGE);
        w.str(&self.id);
        w.str(&self.sender);
        w.str(&self.target);
        match self.fence {
            Some(f) => {
                w.u8(1);
                w.u32(f.shard.get());
                w.u64(f.epoch);
            }
            None => w.u8(0),
        }
        w.u8(self.hops);
        w.str(&self.event);
        w.bytes(&self.data);
        w.0
    }

    /// The frame in `payload`.
    ///
    /// # Errors
    ///
    /// [`FrameError`] when the payload is not a message frame of this format.
    pub fn decode(payload: &[u8]) -> Result<Frame, FrameError> {
        let mut r = Reader(payload);
        if r.header()? != TAG_MESSAGE {
            return Err(FrameError::Malformed("not a message frame".into()));
        }
        let id = r.str()?;
        let sender = r.str()?;
        let target = r.str()?;
        let fence = match r.u8()? {
            0 => None,
            1 => Some(Fence {
                shard: ShardId::new(r.u32()?),
                epoch: r.u64()?,
            }),
            other => return Err(FrameError::Malformed(format!("fence flag {other}"))),
        };
        let hops = r.u8()?;
        let event = r.str()?;
        let data = r.bytes()?.to_vec();
        r.end()?;
        Ok(Frame {
            id,
            sender,
            target,
            fence,
            hops,
            event,
            data,
        })
    }
}

impl Reply {
    /// The reply's payload, without the length prefix.
    pub fn encode(&self) -> Vec<u8> {
        match self {
            Reply::Ack { id } => {
                let mut w = Writer::new(TAG_ACK);
                w.str(id);
                w.0
            }
            Reply::Forwarded { id, redirect } => {
                let mut w = Writer::new(TAG_FORWARDED);
                w.str(id);
                w.u32(redirect.shard.get());
                w.str(redirect.owner.as_str());
                w.u64(redirect.epoch);
                w.0
            }
            Reply::Rejected { id, kind, message } => {
                let mut w = Writer::new(TAG_ERROR);
                w.str(id);
                w.u8(kind.code());
                w.str(message);
                w.0
            }
        }
    }

    /// The reply in `payload`.
    ///
    /// # Errors
    ///
    /// [`FrameError`] when the payload is not a reply of this format.
    pub fn decode(payload: &[u8]) -> Result<Reply, FrameError> {
        let mut r = Reader(payload);
        let tag = r.header()?;
        let id = r.str()?;
        let out = match tag {
            TAG_ACK => Reply::Ack { id },
            TAG_FORWARDED => {
                let shard = ShardId::new(r.u32()?);
                let owner =
                    NodeId::new(r.str()?).map_err(|e| FrameError::Malformed(e.to_string()))?;
                let epoch = r.u64()?;
                Reply::Forwarded {
                    id,
                    redirect: Redirect {
                        shard,
                        owner,
                        epoch,
                    },
                }
            }
            TAG_ERROR => {
                let kind = RejectKind::from_code(r.u8()?)?;
                let message = r.str()?;
                Reply::Rejected { id, kind, message }
            }
            other => return Err(FrameError::Malformed(format!("reply tag {other}"))),
        };
        r.end()?;
        Ok(out)
    }
}

/// `payload` with its 4-byte big-endian length prefix.
///
/// # Errors
///
/// [`FrameError::TooLarge`] when the payload exceeds `max`.
pub fn delimit(payload: &[u8], max: usize) -> Result<Vec<u8>, FrameError> {
    if payload.len() > max || u32::try_from(payload.len()).is_err() {
        return Err(FrameError::TooLarge {
            size: payload.len(),
            max,
        });
    }
    let mut out = Vec::with_capacity(payload.len() + 4);
    out.extend_from_slice(&(payload.len() as u32).to_be_bytes());
    out.extend_from_slice(payload);
    Ok(out)
}
