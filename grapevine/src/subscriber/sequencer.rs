use std::{cmp::Reverse, collections::BinaryHeap, sync::Arc, time::Duration};

use futures::stream::StreamExt;
use pin_project::pin_project;
use tokio::{
    net::UdpSocket,
    time::{timeout, Instant},
};
use tokio_util::udp::UdpFramed;
use tonic::{transport::Channel, Request};
use tracing::{trace, warn};

use crate::proto::{
    grapevine::RawMessage,
    recovery::{recovery_api_client::RecoveryApiClient, SequenceRequest},
};

/// Takes a stream of potentially out of order messages and returns a stream
/// of in order messages.
#[pin_project]
pub struct MessageSequencer {
    /// Client to request missing messages from the publisher.
    recovery_client: RecoveryApiClient<Channel>,
    #[pin]
    udp_framed: UdpFramed<crate::codec::Decoder<RawMessage>, Arc<UdpSocket>>,
    /// Buffer of messages that are received with a higher sequence than
    /// `want_sequence`.
    heap: BinaryHeap<Reverse<RawMessage>>,
    /// How many messages can be missing before requesting a sync.
    max_missing_messages: i64,
    /// How long to wait for a message before requesting a sync.
    sequence_timeout: Duration,
    /// The sequence number of the next message we want.
    want_sequence: i64,
    /// The deadline at which we
    last_deadline: Instant,
    /// Whether we are currently disconnected. TODO: should be "connected"?
    disconnected: bool,
}

pub enum SubscriberStream<T> {
    InSync(T),
    Syncing(T),
    RequestSync,
    Disconnected,
}

impl MessageSequencer {
    pub fn new(
        udp_framed: UdpFramed<crate::codec::Decoder<RawMessage>, Arc<UdpSocket>>,
        recovery_client: RecoveryApiClient<Channel>,
        max_missing_messages: i64,
        want_sequence: i64,
        sequence_timeout: Duration,
    ) -> Self {
        Self {
            recovery_client,
            udp_framed,
            heap: BinaryHeap::new(),
            max_missing_messages,
            sequence_timeout,
            want_sequence,
            last_deadline: Instant::now(),
            disconnected: true,
        }
    }

    async fn next(
        &mut self,
    ) -> Result<SubscriberStream<RawMessage>, Box<dyn std::error::Error + Send + Sync>> {
        // First check whether the heap has the next message.
        while let Some(Reverse(msg)) = self.heap.peek() {
            if msg.metadata.sequence == self.want_sequence {
                // unwrap: we just peeked
                let msg = self.heap.pop().unwrap().0;
                self.want_sequence += 1;
                self.last_deadline = Instant::now() + self.sequence_timeout;
                return Ok(SubscriberStream::InSync(msg));
            } else if msg.metadata.sequence < self.want_sequence {
                // unwrap: we just peeked
                self.heap.pop().unwrap();
                continue;
            } else {
                break;
            }
        }

        if self.disconnected {
            self.last_deadline = Instant::now() + self.sequence_timeout;
        }

        // Listen for the next message.
        loop {
            let now = Instant::now();
            if now > self.last_deadline {
                // TODO: this could be a little smarter e.g. take a heap
                //  and want_sequence and tell you the diff of sync time and
                //  the number of missing messages
                // Time to request sync or individual messages
                if let Some(next_msg) = self.heap.peek() {
                    let missing = next_msg.0.metadata.sequence - self.want_sequence;
                    if missing > self.max_missing_messages {
                        // restream from want_sequence
                        return match self
                            .recovery_client
                            .stream_from(Request::new(SequenceRequest {
                                sequence_id: self.want_sequence,
                            }))
                            .await
                        {
                            Ok(response) => {
                                let mut stream = response.into_inner();
                                // TODO: err
                                let msg = stream.next().await.ok_or("Empty stream")??;
                                self.want_sequence += 1;
                                self.last_deadline = now + self.sequence_timeout;
                                while let Some(Ok(msg)) = stream.next().await {
                                    trace!("Pushing {} to heap from stream", msg.metadata.sequence);
                                    self.heap.push(Reverse(msg));
                                }
                                Ok(SubscriberStream::Syncing(msg))
                            }
                            Err(err) => Err(Box::new(err)),
                        };
                    } else if missing <= 1 {
                        let msg = self
                            .recovery_client
                            .get_message(SequenceRequest {
                                sequence_id: self.want_sequence,
                            })
                            .await?;
                        self.want_sequence += 1;
                        self.last_deadline = now + self.sequence_timeout;
                        return Ok(SubscriberStream::Syncing(msg.into_inner()));
                    } else {
                        // recover multiple
                        todo!()
                    };
                } else {
                    // the socket has stopped receiving messages
                    self.disconnected = true;
                    // TODO: should be an error?
                    return Ok(SubscriberStream::Disconnected);
                }
            }

            let duration = self.last_deadline.duration_since(now);
            match timeout(duration, self.udp_framed.next()).await {
                Ok(Some(Ok((msg, _)))) => {
                    self.disconnected = false;
                    if msg.metadata.sequence < self.want_sequence {
                        continue;
                    } else if msg.metadata.sequence == self.want_sequence {
                        self.want_sequence += 1;
                        self.last_deadline = now + self.sequence_timeout;
                        return Ok(SubscriberStream::InSync(msg));
                    } else {
                        self.heap.push(Reverse(msg));
                        continue;
                    }
                }
                Ok(Some(Err(err))) => return Err(Box::new(err)),
                Ok(None) => return Ok(SubscriberStream::Disconnected),
                Err(_) => {
                    warn!("Timeout waiting for message");
                }
            }
        }
    }
}
