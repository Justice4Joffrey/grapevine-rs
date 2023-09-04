use std::{cmp::Reverse, collections::BinaryHeap, marker::PhantomData, sync::Arc, time::Duration};

use async_stream::stream;
use futures::{stream::StreamExt, Stream};
use pin_project::pin_project;
use prost::Message;
use tokio::{
    net::UdpSocket,
    time::{timeout, Instant},
};
use tokio_util::udp::UdpFramed;
use tonic::{transport::Channel, Request};
use tracing::{debug, trace, warn};

use crate::{
    proto::{
        grapevine::{raw_message::Payload, RawMessage},
        recovery::{recovery_api_client::RecoveryApiClient, SequenceRequest},
    },
    StateSync,
};

/// Takes a stream of potentially out of order messages and returns a stream
/// of in order messages.
///
/// Depending on the configuration parameters passed, the sequencer will use
/// its TCP client to attempt to recover missing messages.
#[pin_project]
pub struct MessageSequencer<S> {
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
    /// Whether we are currently connected.
    connected: bool,
    /// Bind a sequencer to a specific message type.
    phantom: PhantomData<S>,
}

#[derive(Debug)]
pub enum SubscriberStreamMessage<T> {
    Delta(T),
    Started,
    Heartbeat,
}

#[derive(Debug)]
pub enum SubscriberStream<T> {
    Message(SubscriberStreamMessage<T>),
    RequestSync,
    Disconnected,
}

impl<S: StateSync> MessageSequencer<S> {
    pub fn new(
        socket: Arc<UdpSocket>,
        recovery_client: RecoveryApiClient<Channel>,
        max_missing_messages: i64,
        want_sequence: i64,
        sequence_timeout: Duration,
    ) -> Self {
        Self {
            recovery_client,
            udp_framed: UdpFramed::new(socket, crate::codec::Decoder::<RawMessage>::new()),
            heap: BinaryHeap::new(),
            max_missing_messages,
            sequence_timeout,
            want_sequence,
            last_deadline: Instant::now(),
            connected: false,
            phantom: PhantomData,
        }
    }

    pub fn into_stream(
        mut self,
    ) -> impl Stream<Item = Result<SubscriberStream<S::Delta>, Box<dyn std::error::Error + Send + Sync>>>
    {
        let stream = stream! {
            loop {
                yield self.next().await;
            }
        };
        stream
    }

    fn try_parse(
        message: &RawMessage,
    ) -> Result<SubscriberStreamMessage<S::Delta>, Box<dyn std::error::Error + Send + Sync>> {
        match message.payload.as_ref().ok_or("Missing payload")? {
            Payload::Heartbeat(_) => Ok(SubscriberStreamMessage::Heartbeat),
            Payload::Started(_) => Ok(SubscriberStreamMessage::Started),
            Payload::Delta(delta) => {
                let delta = S::Delta::decode(delta.body.as_ref())?;
                Ok(SubscriberStreamMessage::Delta(delta))
            }
        }
    }

    async fn next(
        &mut self,
    ) -> Result<SubscriberStream<S::Delta>, Box<dyn std::error::Error + Send + Sync>> {
        // First check whether the heap has the next message.
        while let Some(Reverse(msg)) = self.heap.peek() {
            if msg.metadata.sequence == self.want_sequence {
                // unwrap: we just peeked
                let msg = self.heap.pop().unwrap().0;
                self.want_sequence += 1;
                self.last_deadline = Instant::now() + self.sequence_timeout;
                return Ok(SubscriberStream::Message(Self::try_parse(&msg)?));
            } else if msg.metadata.sequence < self.want_sequence {
                // unwrap: we just peeked
                self.heap.pop().unwrap();
                continue;
            } else {
                break;
            }
        }

        if !self.connected {
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
                        debug!("Requesting sync from {}", self.want_sequence);
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
                                Ok(SubscriberStream::Message(Self::try_parse(&msg)?))
                            }
                            Err(err) => Err(Box::new(err)),
                        };
                    } else if missing <= 1 {
                        debug!("Requesting message {}", self.want_sequence);
                        let msg = self
                            .recovery_client
                            .get_message(SequenceRequest {
                                sequence_id: self.want_sequence,
                            })
                            .await?;
                        self.want_sequence += 1;
                        self.last_deadline = now + self.sequence_timeout;
                        return Ok(SubscriberStream::Message(Self::try_parse(
                            &msg.into_inner(),
                        )?));
                    } else {
                        // recover multiple
                        // TODO: log messages
                        debug!("Requesting messages");
                        todo!()
                    };
                } else {
                    // the socket has stopped receiving messages
                    self.connected = false;
                    // TODO: should be an error?
                    return Ok(SubscriberStream::Disconnected);
                }
            }

            let duration = self.last_deadline.duration_since(now);
            match timeout(duration, self.udp_framed.next()).await {
                Ok(Some(Ok((msg, _)))) => {
                    self.connected = true;
                    if msg.metadata.sequence < self.want_sequence {
                        continue;
                    } else if msg.metadata.sequence == self.want_sequence {
                        self.want_sequence += 1;
                        self.last_deadline = now + self.sequence_timeout;
                        return Ok(SubscriberStream::Message(Self::try_parse(&msg)?));
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
