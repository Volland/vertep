//! Client-side networks over real sockets: [`HttpNetwork`], a blocking HTTP
//! [`RelayNetwork`] that posts TSP messages, a `did:web` fetcher, and [`WsLiveNetwork`], a
//! WebSocket [`LiveNetwork`] for live sessions with a mediator.
//!
//! The mediator uses [`HttpNetwork`] to forward middle hops and deliver to online recipients;
//! clients use both to talk to a mediator. HTTP status codes map onto relay errors: 404 and 410
//! are `Gone`, 5xx and transport failures `Unreachable` (worth retrying), other refusals
//! `Refused`. The WebSocket network speaks `ws://`; put TLS in front of it with a terminating
//! proxy.

use std::sync::Arc;
use std::time::Duration;

use futures::{SinkExt, StreamExt};
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::Message;
use vertep_federation::live::{FrameSink, LiveNetwork};
use vertep_federation::net::{NetError, RelayNetwork};
use vertep_federation::resolver::{FetchError, HttpFetcher};

/// The media type of TSP messages posted over HTTP.
pub const TSP_MEDIA_TYPE: &str = "application/octet-stream";

/// A blocking HTTP client for relay posts.
#[derive(Clone, Debug)]
pub struct HttpNetwork {
    agent: ureq::Agent,
}

impl Default for HttpNetwork {
    fn default() -> Self {
        Self::new(Duration::from_secs(10))
    }
}

impl HttpNetwork {
    /// A client whose requests time out after `timeout`.
    pub fn new(timeout: Duration) -> Self {
        HttpNetwork {
            agent: ureq::AgentBuilder::new().timeout(timeout).build(),
        }
    }
}

impl RelayNetwork for HttpNetwork {
    fn post(&self, endpoint: &str, body: &[u8]) -> Result<Vec<u8>, NetError> {
        match self
            .agent
            .post(endpoint)
            .set("content-type", TSP_MEDIA_TYPE)
            .send_bytes(body)
        {
            Ok(resp) => {
                let mut out = Vec::new();
                std::io::Read::read_to_end(&mut resp.into_reader(), &mut out)
                    .map_err(|e| NetError::Unreachable(format!("{endpoint}: {e}")))?;
                Ok(out)
            }
            Err(ureq::Error::Status(code, resp)) => {
                let text = resp.into_string().unwrap_or_default();
                Err(match code {
                    404 | 410 => NetError::Gone(format!("{endpoint}: {code}")),
                    500..=599 => NetError::Unreachable(format!("{endpoint}: {code} {text}")),
                    _ => NetError::Refused(format!("{endpoint}: {code} {text}")),
                })
            }
            Err(e) => Err(NetError::Unreachable(format!("{endpoint}: {e}"))),
        }
    }
}

impl HttpFetcher for HttpNetwork {
    fn get(&self, url: &str, timeout: Duration) -> Result<Vec<u8>, FetchError> {
        match self.agent.get(url).timeout(timeout).call() {
            Ok(resp) => {
                let mut out = Vec::new();
                std::io::Read::read_to_end(&mut resp.into_reader(), &mut out)
                    .map_err(|e| FetchError::Network(e.to_string()))?;
                Ok(out)
            }
            Err(ureq::Error::Status(404 | 410, _)) => Err(FetchError::NotFound),
            Err(ureq::Error::Status(code, _)) => Err(FetchError::Status(code)),
            Err(e) => Err(FetchError::Network(e.to_string())),
        }
    }
}

/// A WebSocket client for live sessions. Each connection runs on its own thread with a small
/// tokio runtime; frames from the relay are handed to the session's sink on that thread.
// @tg: implements:: [[openspec:relay-mailboxes#Live Delivery Over WebSocket]] {note: "the client end of the WebSocket"}
#[derive(Clone, Copy, Debug, Default)]
pub struct WsLiveNetwork;

struct WsUp {
    tx: mpsc::UnboundedSender<Option<Vec<u8>>>,
}

impl FrameSink for WsUp {
    fn send(&self, frame: &[u8]) -> Result<(), NetError> {
        self.tx
            .send(Some(frame.to_vec()))
            .map_err(|_| NetError::Unreachable("live session closed".into()))
    }

    fn closed(&self) {
        let _ = self.tx.send(None);
    }
}

impl LiveNetwork for WsLiveNetwork {
    fn connect(
        &self,
        uri: &str,
        to_client: Arc<dyn FrameSink>,
    ) -> Result<Arc<dyn FrameSink>, NetError> {
        let (tx, mut rx) = mpsc::unbounded_channel::<Option<Vec<u8>>>();
        let (ready_tx, ready_rx) = std::sync::mpsc::channel::<Result<(), NetError>>();
        let uri = uri.to_owned();
        std::thread::Builder::new()
            .name("vt-live".into())
            .spawn(move || {
                let rt = match tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                {
                    Ok(rt) => rt,
                    Err(e) => {
                        let _ = ready_tx.send(Err(NetError::Unreachable(e.to_string())));
                        return;
                    }
                };
                rt.block_on(async move {
                    let ws = match tokio_tungstenite::connect_async(uri.as_str()).await {
                        Ok((ws, _)) => ws,
                        Err(e) => {
                            let _ =
                                ready_tx.send(Err(NetError::Unreachable(format!("{uri}: {e}"))));
                            return;
                        }
                    };
                    let _ = ready_tx.send(Ok(()));
                    let (mut write, mut read) = ws.split();
                    loop {
                        tokio::select! {
                            out = rx.recv() => match out {
                                Some(Some(frame)) => {
                                    if write.send(Message::Binary(frame.into())).await.is_err() {
                                        break;
                                    }
                                }
                                _ => {
                                    let _ = write.close().await;
                                    break;
                                }
                            },
                            incoming = read.next() => match incoming {
                                Some(Ok(Message::Binary(b))) => {
                                    let _ = to_client.send(&b);
                                }
                                Some(Ok(Message::Text(t))) => {
                                    let _ = to_client.send(t.as_bytes());
                                }
                                Some(Ok(_)) => {}
                                _ => break,
                            },
                        }
                    }
                    to_client.closed();
                });
            })
            .map_err(|e| NetError::Unreachable(e.to_string()))?;
        ready_rx
            .recv_timeout(Duration::from_secs(10))
            .map_err(|_| NetError::Unreachable("live connect timed out".into()))??;
        Ok(Arc::new(WsUp { tx }))
    }
}
