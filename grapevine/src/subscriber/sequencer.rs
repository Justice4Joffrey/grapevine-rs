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
    /// How long to wait for a message before requesting a sync.
    sequence_timeout: Duration,
    /// How many messages can be missing before requesting a full resync from earliest missing message to last
    /// This option affects method of syncing but only after `sequence_timeout` elapsed
    batch_resync_threshold: i64,
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
        batch_resync_threshold: i64,
        want_sequence: i64,
        sequence_timeout: Duration,
    ) -> Self {
        Self {
            recovery_client,
            udp_framed: UdpFramed::new(socket, crate::codec::Decoder::<RawMessage>::new()),
            heap: BinaryHeap::new(),
            batch_resync_threshold,
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
                    // TODO: missing = (heap.max - heap.min) - len(heap)
                    let missing = next_msg.0.metadata.sequence - self.want_sequence;
                    if missing > self.batch_resync_threshold {
                        // restream from want_sequence
                        debug!("Requesting sync from {}", self.want_sequence);

                        // gRPC guarantee order of messages inside stream (https://grpc.io/docs/what-is-grpc/core-concepts/)
                        // correctness of messages from gRPC was not checked
                        // they should be sorted and start exactly from `want_sequence`

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


#[cfg(test)]
mod tests {
    use std::{convert::Infallible, net::SocketAddrV4, sync::Mutex};
    use bytes::{BytesMut, Bytes};
    use chrono::Utc;
    use futures::{pin_mut, TryStreamExt};
    use tokio::net::TcpListener;
    use tokio_stream::wrappers::TcpListenerStream;
    use tokio_util::codec::Encoder;
    use tonic::{Response, Status, transport::Server};
    use rand::seq::SliceRandom;
    use rand::thread_rng;

    use super::*;
    use crate::{StateSync, proto::{recovery::recovery_api_server::{RecoveryApiServer, RecoveryApi}, grapevine::{Metadata, Delta}}, publisher::MessageStream};

    #[derive(Default)]
    struct TestStateSync {}

    impl StateSync for TestStateSync {
        type ApplyError = Infallible;
        type Delta = i64;

        fn apply_delta(&mut self, delta: Self::Delta) -> Result<(), Self::ApplyError> {
            Ok(())
        }
    }

    struct TestRecoveryApi {
        count: i64,
        sent: Mutex<i64>,
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
            Ok(Response::new(RawMessage { metadata: Metadata { timestamp: 0, sequence: inner.sequence_id }, payload: None }))
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
                        yield Result::Ok(RawMessage { metadata: Metadata { timestamp: 0, sequence: id }, payload: None });
                    }
                }
            };

            Ok(Response::new(Box::pin(msg_stream)))
        }
    }

    fn get_id(s: SubscriberStream<i64>) -> Option<i64> {
        if let SubscriberStream::Message(SubscriberStreamMessage::Delta(id)) = s {
            Some(id)
        } else {
            None
        }
    }

    fn make_raw_msg(i: i64) -> anyhow::Result<RawMessage> {
        let mut msg = RawMessage::default();
        msg.metadata.sequence = i;
        msg.metadata.timestamp = Utc::now().timestamp_nanos();

        let mut buf = BytesMut::new();
        <i64>::encode(&msg.metadata.sequence, &mut buf)?;
        msg.payload = Some(Payload::Delta(Delta { body: buf.to_vec() }));

        Ok(msg)
    }

    fn make_msg(i: i64) -> anyhow::Result<Bytes> {
        let msg = make_raw_msg(i)?;

        let mut encoder = crate::codec::Encoder::<RawMessage>::default();
        let mut buffer = BytesMut::new();
        encoder.encode(msg, &mut buffer)?;
        Ok(buffer.freeze())
    }

    async fn make_recovery_server(total_len: i64) -> anyhow::Result<std::net::SocketAddr> {
        let bind_addr: SocketAddrV4 = "127.0.0.1:0".parse()?;
        let socket = TcpListener::bind(bind_addr).await?;
        let addr = socket.local_addr()?;
        let stream = TcpListenerStream::new(socket);
        let api = TestRecoveryApi { count: total_len, sent: Mutex::new(0) };

        tokio::spawn(async move {
            Server::builder()
                .add_service(RecoveryApiServer::new(api))
                .serve_with_incoming(stream).await?;
            Ok::<_, anyhow::Error>(())
        });

        Ok(addr)
    }

    async fn test_it(source: Vec<i64>, target: Vec<i64>, want_sequence: i64) -> anyhow::Result<()> {
        let subscriber = UdpSocket::bind("127.0.0.1:0").await?;
        let publisher = UdpSocket::bind("127.0.0.1:0").await?;
        publisher.connect(subscriber.local_addr()?).await?;

        let recovery_addr = make_recovery_server(target.len() as i64).await?;

        let recovery_client_channel = Channel::builder(recovery_addr.to_string().parse().unwrap()).connect_lazy();
        let recovery_client = RecoveryApiClient::new(recovery_client_channel);

        let subscriber = Arc::new(subscriber);
        let sequencer = MessageSequencer::<TestStateSync>::new(subscriber, recovery_client, i64::MAX, want_sequence, Duration::from_secs(60));
        let result_stream = sequencer.into_stream();
        pin_mut!(result_stream);

        let publisher_thread = tokio::spawn(async move {
            for i in source {
                tokio::time::sleep(Duration::from_millis(1)).await;
                publisher.send(make_msg(i)?.as_ref()).await?;
            }

            Ok::<_, anyhow::Error>(())
        });

        let out: Result<Vec<_>, _> = result_stream.take(target.len()).try_collect().await;
        let out = out.expect("stream error");
        let out = out.into_iter()
            .map(|v| get_id(v))
            .collect::<Option<Vec<_>>>()
            .expect("non deltas found");
        publisher_thread.await.unwrap().unwrap();

        assert_eq!(target, out);

        Ok(())
    }

    #[tokio::test]
    async fn ordered() -> anyhow::Result<()> {
        let count = 100;
        let source: Vec<_> = (0..count).collect();
        let target: Vec<_> = (0..count).collect();

        test_it(source, target, 0).await?;

        Ok(())
    }

    #[tokio::test]
    async fn unordered() -> anyhow::Result<()> {
        let count = 100;
        let target: Vec<_> = (0..count).collect();

        let mut source: Vec<_> = (0..count).collect();
        let mut rng = thread_rng();
        source.shuffle(&mut rng);

        test_it(source, target, 0).await?;

        Ok(())
    }

    #[tokio::test]
    async fn unordered_from() -> anyhow::Result<()> {
        let count = 100;
        let target: Vec<_> = (10..count).collect();

        let mut source: Vec<_> = (0..count).collect();
        let mut rng = thread_rng();
        source.shuffle(&mut rng);

        test_it(source, target, 10).await?;

        Ok(())
    }
}
