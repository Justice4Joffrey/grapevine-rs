use std::{cmp::Reverse, marker::PhantomData, sync::Arc, time::Duration};

use async_stream::stream;
use futures::{stream::StreamExt, Stream};
use pin_project::pin_project;
use prost::{Message, DecodeError};
use tokio::{
    net::UdpSocket,
    time::{timeout, Instant},
};
use tokio_util::udp::UdpFramed;
use tonic::{transport::Channel, Request};
use tracing::{debug, trace, warn};
use thiserror::Error;

use crate::{
    proto::{
        grapevine::{raw_message::Payload, RawMessage},
        recovery::{recovery_api_client::RecoveryApiClient, SequenceRequest},
    },
    StateSync,
};

use super::{min_max_heap::MinMaxHeap, message::{SubscriberStreamMessage, Origin, ReceivedMessage}};

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
    heap: MinMaxHeap,
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
pub enum SubscriberStream<T> {
    Message {
        /// according to sender (in nano-seconds)
        send_ts: i64,
        /// according to our process (in nano-seconds)
        recv_ts: i64,
        origin: Origin,
        msg: SubscriberStreamMessage<T>
    },
    Disconnected,
}

#[derive(Debug, Error)]
pub enum SubscriberStreamError {
    #[error("invalid sequence")]
    InvalidSequence,
    #[error("empty stream")]
    EmptyStream,
}

impl<S: StateSync> MessageSequencer<S> {
    pub fn new(
        socket: Arc<UdpSocket>,
        recovery_client: RecoveryApiClient<Channel>,
        want_sequence: i64,
        sequence_timeout: Duration,
    ) -> Self {
        Self {
            recovery_client,
            udp_framed: UdpFramed::new(socket, crate::codec::Decoder::<RawMessage>::new()),
            heap: MinMaxHeap::new(),
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
        message_from: &ReceivedMessage,
    ) -> Result<SubscriberStream<S::Delta>, Box<dyn std::error::Error + Send + Sync>> {
        let payload: Result<SubscriberStreamMessage<S::Delta>, DecodeError> = match message_from.raw.payload.as_ref().ok_or("Missing payload")? {
            Payload::Heartbeat(_) => Ok(SubscriberStreamMessage::Heartbeat),
            Payload::Started(_) => Ok(SubscriberStreamMessage::Started),
            Payload::Delta(delta) => {
                let delta = S::Delta::decode(delta.body.as_ref())?;
                Ok(SubscriberStreamMessage::Delta(delta))
            }
        };
        let payload = payload?;

        Ok(SubscriberStream::Message {
            send_ts: message_from.raw.metadata.timestamp,
            recv_ts: message_from.recv_ts,
            origin: message_from.origin.clone(),
            msg: payload,
        })
    }

    async fn next(
        &mut self,
    ) -> Result<SubscriberStream<S::Delta>, Box<dyn std::error::Error + Send + Sync>> {
        // First check whether the heap has the next message.
        while let Some(Reverse(msg)) = self.heap.peek() {
            if msg.raw.metadata.sequence == self.want_sequence {
                // unwrap: we just peeked
                let msg = self.heap.pop().unwrap().0;
                self.want_sequence += 1;
                self.last_deadline = Instant::now() + self.sequence_timeout;
                return Ok(Self::try_parse(&msg)?);
            } else if msg.raw.metadata.sequence < self.want_sequence {
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
                    let single_missing = self.want_sequence + 1 == next_msg.0.raw.metadata.sequence &&
                        !self.heap.has_gaps();

                    if single_missing {
                        debug!("Requesting message {}", self.want_sequence);
                        let msg = self
                            .recovery_client
                            .get_message(SequenceRequest {
                                sequence_id: self.want_sequence,
                            })
                            .await?;
                        let msg = ReceivedMessage::new(msg.into_inner(), Origin::Missing);

                        if msg.raw.metadata.sequence != self.want_sequence {
                            return Err(Box::new(SubscriberStreamError::InvalidSequence));
                        }

                        self.want_sequence += 1;
                        self.last_deadline = now + self.sequence_timeout;
                        return Ok(Self::try_parse(&msg)?);
                    } else {
                        // restream from want_sequence
                        debug!("Requesting sync from {}", self.want_sequence);

                        let response = self
                            .recovery_client
                            .stream_from(Request::new(SequenceRequest {
                                sequence_id: self.want_sequence,
                            }))
                            .await?;

                        let mut stream = response.into_inner();

                        // if don't clean heap then there will be duplicates
                        // and origin (udp/sync) will be mixed
                        // other solution is to implement Ord which compare tuples (seq_id, origin)
                        self.heap.clear();

                        while let Some(Ok(msg)) = stream.next().await {
                            trace!("Pushing {} to heap from stream", msg.metadata.sequence);
                            self.heap.push(Reverse(ReceivedMessage::new(msg, Origin::Sync)));
                        }

                        let msg = self.heap.pop()
                            .ok_or(Box::new(SubscriberStreamError::EmptyStream))?
                            .0;

                        if msg.raw.metadata.sequence != self.want_sequence {
                            return Err(Box::new(SubscriberStreamError::InvalidSequence));
                        }

                        self.want_sequence += 1;
                        self.last_deadline = now + self.sequence_timeout;

                        return Ok(Self::try_parse(&msg)?);
                    }
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
                    let msg = ReceivedMessage::new(msg, Origin::Udp);

                    self.connected = true;
                    if msg.raw.metadata.sequence < self.want_sequence {
                        continue;
                    } else if msg.raw.metadata.sequence == self.want_sequence {
                        self.want_sequence += 1;
                        self.last_deadline = now + self.sequence_timeout;
                        return Ok(Self::try_parse(&msg)?);
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


#[cfg(test)]
mod tests {
    use std::{convert::Infallible, net::SocketAddrV4, sync::Mutex};
    use futures::{pin_mut, TryStreamExt};
    use tokio::net::TcpListener;
    use tokio_stream::wrappers::TcpListenerStream;
    use tonic::{Response, Status, transport::{Server, Uri}};
    use rand::{thread_rng, seq::SliceRandom};

    use super::*;
    use crate::{StateSync, proto::recovery::recovery_api_server::{RecoveryApiServer, RecoveryApi}, publisher::MessageStream, subscriber::message::test_utils::{make_raw_msg, make_raw_msg_bytes}};

    #[derive(Default)]
    struct TestStateSync {}

    impl StateSync for TestStateSync {
        type ApplyError = Infallible;
        type Delta = i64;

        fn apply_delta(&mut self, _delta: Self::Delta) -> Result<(), Self::ApplyError> {
            Ok(())
        }
    }

    struct TestRecoveryApi {
        count: i64,
        sent: Arc<Mutex<i64>>,
    }

    #[tonic::async_trait]
    impl RecoveryApi for TestRecoveryApi {
        type StreamFromStream = MessageStream<RawMessage>;

        async fn get_message(
            &self,
            request: Request<SequenceRequest>,
        ) -> Result<Response<RawMessage>, Status> {
            let inner = request.into_inner();
            *self.sent.lock().unwrap() += 1;
            let msg = make_raw_msg(inner.sequence_id).map_err(|_| Status::internal("cannot make msg"))?;
            Ok(Response::new(msg))
        }

        async fn stream_from(
            &self,
            request: Request<SequenceRequest>,
        ) -> Result<Response<Self::StreamFromStream>, Status> {
            let msg_stream = {
                let sequence_id = request.into_inner().sequence_id;
                let ids: Vec<_> = (sequence_id..self.count).collect();
                *self.sent.lock().unwrap() += ids.len() as i64;

                stream! {
                    for id in ids {
                        let msg = make_raw_msg(id).map_err(|_| Status::internal("cannot make msg"))?;
                        yield Result::Ok(msg);
                    }
                }
            };

            Ok(Response::new(Box::pin(msg_stream)))
        }
    }

    fn get_id(s: SubscriberStream<i64>) -> Option<(i64, Origin)> {
        if let SubscriberStream::Message { msg: SubscriberStreamMessage::Delta(id), origin, .. } = s {
            Some((id, origin))
        } else {
            None
        }
    }

    fn get_ids(data: Vec<SubscriberStream<i64>>) -> anyhow::Result<Vec<(i64, Origin)>> {
        data.into_iter()
            .map(|v| get_id(v))
            .collect::<Option<Vec<_>>>()
            .ok_or(anyhow::anyhow!("non deltas found"))
    }

    async fn make_recovery_server(total_len: i64) -> anyhow::Result<(Uri, Arc<Mutex<i64>>)> {
        let bind_addr: SocketAddrV4 = "127.0.0.1:0".parse()?;
        let socket = TcpListener::bind(bind_addr).await?;
        let addr = socket.local_addr()?;
        let stream = TcpListenerStream::new(socket);
        let sent_counter = Arc::new(Mutex::new(0));
        let api = TestRecoveryApi { count: total_len, sent: sent_counter.clone() };

        tokio::spawn(async move {
            Server::builder()
                .add_service(RecoveryApiServer::new(api))
                .serve_with_incoming(stream).await?;
            Ok::<_, anyhow::Error>(())
        });

        let uri = format!("http://{}", addr.to_string()).parse::<Uri>()?;
        Ok((uri, sent_counter))
    }

    async fn test_it(
        source: Vec<i64>,
        target: Vec<(i64, Origin)>,
        want_sequence: i64,
        sequence_timeout: Duration,
    ) -> anyhow::Result<()> {
        let subscriber = UdpSocket::bind("127.0.0.1:0").await?;
        let publisher = UdpSocket::bind("127.0.0.1:0").await?;
        publisher.connect(subscriber.local_addr()?).await?;

        let (recovery_uri, sent_by_recovery) = make_recovery_server(target.len() as i64).await?;
        let recovery_client_channel = Channel::builder(recovery_uri).connect_lazy();
        let recovery_client = RecoveryApiClient::new(recovery_client_channel);

        let subscriber = Arc::new(subscriber);
        let sequencer = MessageSequencer::<TestStateSync>::new(
            subscriber,
            recovery_client,
            want_sequence,
            sequence_timeout,
        );
        let result_stream = sequencer.into_stream();
        pin_mut!(result_stream);

        let publisher_thread = tokio::spawn(async move {
            for i in source {
                tokio::time::sleep(Duration::from_millis(1)).await;
                publisher.send(make_raw_msg_bytes(i)?.as_ref()).await?;
            }

            Ok::<_, anyhow::Error>(())
        });

        let out: Result<Vec<_>, _> = result_stream.take(target.len()).try_collect().await;
        let out = out.map_err(|e| anyhow::anyhow!(e))?;
        let out = get_ids(out)?;
        publisher_thread.await??;

        assert_eq!(target, out);

        Ok(())
    }

    #[tokio::test]
    async fn ordered() -> anyhow::Result<()> {
        let count = 100;
        let source: Vec<_> = (0..count).collect();
        let target: Vec<_> = (0..count).map(|id| (id, Origin::Udp)).collect();

        test_it(source, target, 0, Duration::from_secs(60)).await?;

        Ok(())
    }

    #[tokio::test]
    async fn unordered() -> anyhow::Result<()> {
        let count = 100;
        let target: Vec<_> = (0..count).map(|id| (id, Origin::Udp)).collect();

        let mut source: Vec<_> = (0..count).collect();
        let mut rng = thread_rng();
        source.shuffle(&mut rng);

        test_it(source, target, 0, Duration::from_secs(60)).await?;

        Ok(())
    }

    #[tokio::test]
    async fn unordered_from() -> anyhow::Result<()> {
        let count = 100;
        let target: Vec<_> = (10..count).map(|id| (id, Origin::Udp)).collect();

        let mut source: Vec<_> = (0..count).collect();
        let mut rng = thread_rng();
        source.shuffle(&mut rng);

        test_it(source, target, 10, Duration::from_secs(60)).await?;

        Ok(())
    }

    #[tokio::test]
    async fn unordered_with_duplicates() -> anyhow::Result<()> {
        let count = 100;
        let target: Vec<_> = (0..count).map(|id| (id, Origin::Udp)).collect();

        let mut source: Vec<_> = (0..count).chain(0..count).collect();
        let mut rng = thread_rng();
        source.shuffle(&mut rng);

        test_it(source, target, 0, Duration::from_secs(60)).await?;

        Ok(())
    }

    #[tokio::test]
    async fn singe_recovery() -> anyhow::Result<()> {
        let count = 100;
        let mut target: Vec<_> = (0..count).map(|id| (id, Origin::Udp)).collect();

        let mut source: Vec<_> = (0..count).collect();
        let mut rng = thread_rng();
        source.shuffle(&mut rng);

        let missing_id = source.pop().unwrap();
        target[missing_id as usize].1 = Origin::Missing;

        let start_at = Instant::now();
        test_it(source, target, 0, Duration::from_millis(250)).await?;

        assert!(Instant::now() - start_at < Duration::from_millis(500));

        Ok(())
    }

    #[tokio::test]
    async fn batch_recovery() -> anyhow::Result<()> {
        let count = 100;
        let missing = vec![70, 72, 74, 76, 78];

        let target: Vec<_> = (0..70).map(|id| (id, Origin::Udp))
            .chain((70..100).map(|id| (id, Origin::Sync)))
            .collect();
        let source: Vec<_> = (0..count).filter(|v| !missing.contains(v)).collect();

        let start_at = Instant::now();
        test_it(source, target, 0, Duration::from_millis(250)).await?;

        assert!(Instant::now() - start_at < Duration::from_millis(500));

        Ok(())
    }

    /// tries to break MinMaxHeap::has_gaps() { max - min + 1 - len != 0 }
    #[tokio::test]
    async fn batch_recovery_with_duplicates() -> anyhow::Result<()> {
        let target: Vec<_> = (0..70).map(|id| (id, Origin::Udp))
            .chain((70..100).map(|id| (id, Origin::Sync)))
            .collect();

        let missing = vec![70, 72, 74, 76, 78];
        let duplicates = vec![71, 73, 75, 77, 79];
        let source: Vec<_> = (0..100).filter(|v| !missing.contains(v))
            .chain(duplicates).collect();
        assert_eq!(source.len(), target.len());

        let start_at = Instant::now();
        test_it(source, target, 0, Duration::from_millis(250)).await?;

        assert!(Instant::now() - start_at < Duration::from_millis(500));

        Ok(())
    }

    // TODO: udp timeout (last message/heartbeat was missed) -> empty sync stream / empty get_message -> stream error
    // TODO: SubscriberStreamMessage: Started, Heartbeat should be passed as is
    // TODO: tests for SubscriberStream & SubscriberStreamError
    // TODO: recovery_is_down
}
