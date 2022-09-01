//! # Subscriber
//!
//! ## Subscriber Strategies
//!
//! ### Exactly Once
//!
//! The current implementation of a [Subscriber](SubscriberStream) listens
//! to every single message in order and plays it through the internal
//! [StateSync] type.
//!
//! Example usage:
//!
//! ``` no_run
//! # use std::net::{Ipv4Addr, SocketAddrV4};
//! # use std::str::FromStr;
//! # use std::sync::Arc;
//! # use tonic::transport::Channel;
//! # use grapevine::proto::recovery::recovery_api_client::RecoveryApiClient;
//! # use grapevine::sqlite::SqliteSubscriberPersistence;
//! # use grapevine::{StateSync, SubscriberStream};
//! # use grapevine::mock::MockSubscriberPersistence;
//! # use grapevine::proto::tree_state::TreeState;
//! # use grapevine::transport::new_multicast_socket;
//! # use tokio_stream::StreamExt;
//! #
//! # async fn inner() {
//! #
//! # let recovery_addr = "http://127.0.0.1:7755".parse().unwrap();
//! # let mc_addr = SocketAddrV4::new(Ipv4Addr::new(224, 0, 0, 123), 1234);
//! # let socket = new_multicast_socket(&mc_addr).unwrap();
//! #
//! # let persistence = MockSubscriberPersistence::default();
//! # let channel = Channel::builder(recovery_addr).connect_lazy();
//! #
//! // create a subscriber for the [TreeState] example
//! let subscriber = SubscriberStream::<TreeState, _>::new(
//!     Default::default(),
//!     // ...
//! #    std::time::Duration::from_secs(5),
//! #    std::time::Duration::from_millis(20),
//! #    5,
//! #    Arc::new(socket),
//! #    persistence,
//! #    RecoveryApiClient::new(channel),
//! );
//!
//! // move it in to its stream
//! let mut stream = subscriber.stream();
//!
//! // await messages!
//! // the stream will:
//! // - synchronize its state on startup.
//! // - connect to the multicast stream.
//! // - ensure to play _all_ messages in sequence order.
//! // - apply deltas to the internal state as they arrive.
//! while let Some(Some(Ok((state, action, sequence)))) = stream.next().await {
//!     // do something!
//! }
//! # }
//! ```
//!
//! On startup, it will read the most recent sequence in the replica log and
//! request playback of all messages since.
//!
//! Messages received through the multicast socket will either be:
//!
//! - `applied`: sequence number matches the sequence [SubscriberStream] is
//! looking for.
//! - `stored`: sequence number is greater than the sequence desired.
//! - `discarded`: sequence number is less than the sequence desired.
//!
//! ### Eventual consistency
//!
//! Processing every message can be wasteful in many circumstances. In
//! scenarios where eventual consistency is the best approach, Snapshots
//! (WIP) can be used instead.

use std::{
    cmp::{Ordering, Reverse},
    collections::BinaryHeap,
    error::Error,
    fmt,
    pin::Pin,
    sync::Arc,
};

use async_stream::stream;
use bytes::BytesMut;
use chrono::{DateTime, Duration, TimeZone, Utc};
use prost::{DecodeError, Message};
use tokio::{net::UdpSocket, sync::RwLock, time::timeout};
use tokio_stream::{Stream, StreamExt};
use tokio_util::udp::UdpFramed;
use tracing::*;

use crate::{
    proto::{
        grapevine,
        grapevine::{raw_message::Payload, RawMessage},
        recovery::{recovery_api_client::RecoveryApiClient, SequenceRequest},
    },
    subscriber::sync_status::SyncStatus,
    StateSync, SubscriberPersistence,
};

mod sync_status;

/// The container for all state needed to recover and connect to a
/// publisher stream.
pub struct SubscriberStream<S, P> {
    /// Internal state.
    state: Arc<RwLock<SubscriberStateWrapper<S>>>,
    /// The default timeout for reading from the socket.
    read_timeout: std::time::Duration,
    /// How long to wait after receiving an out of sequence message before
    /// requesting it from the recovery client over TCP.
    recover_timeout: std::time::Duration,
    /// Maximum heap size before a restream is requested.
    heap_size_to_sync: usize,
    /// Storage for messages of higher sequence than desired.
    heap: BinaryHeap<Reverse<RawMessage>>,
    /// Internal message framing layer.
    udp_framed: Pin<Box<UdpFramed<crate::codec::Decoder<RawMessage>, Arc<UdpSocket>>>>,
    /// A buffer specifically for deserialized messages.
    persistence_buffer: BytesMut,
    /// A type of [crate::SubscriberPersistence].
    persistence: P,
    /// A client to the recovery API to recover missing packets.
    recovery_client: RecoveryApiClient<tonic::transport::Channel>,
}

impl<S: StateSync, P: SubscriberPersistence> SubscriberStream<S, P> {
    /// Create a new instance.
    pub fn new(
        state: Arc<RwLock<SubscriberStateWrapper<S>>>,
        read_timeout: std::time::Duration,
        recover_timeout: std::time::Duration,
        heap_size_to_sync: usize,
        socket: Arc<UdpSocket>,
        persistence: P,
        recovery_client: RecoveryApiClient<tonic::transport::Channel>,
    ) -> Self {
        Self {
            state,
            read_timeout,
            recover_timeout,
            heap_size_to_sync,
            heap: Default::default(),
            udp_framed: Box::pin(UdpFramed::new(
                socket,
                crate::codec::Decoder::<RawMessage>::new(),
            )),
            persistence_buffer: Default::default(),
            persistence,
            recovery_client,
        }
    }

    /// Consumes in to an async stream.
    ///
    /// The underlying functionality is built from two event processing
    /// components.
    ///
    /// The first os a standard udp_framed read with a timeout of
    /// `read_timeout`. The producer of the state we're subscribed to is
    /// expected to send multiple messages within `read_timeout`, therefore
    /// nothing being received is evidence that the producer has stopped.
    ///
    /// For messages which are received within `read_timeout`, the message's
    /// sequence number is compared to the sequence we want (this is the
    /// previous message's sequence + 1). If it's greater, the message is
    /// added to the message heap. If it's less, the message is ignored. If
    /// it's equal, we call [StateSync::apply_delta] with the message's
    /// payload and yield a cloned reference to `state`.
    ///
    /// The second event processing component is a message heap. Messages
    /// with a sequence ID greater than `want_sequence` are pushed to this to
    /// be processed later (out of order messages are common over UDP). Once
    /// we've received a message greater than the `want_sequence`, we will
    /// wait for `recover_timeout` before requesting the missing message from
    /// the recovery client over TCP.
    ///
    /// When requesting a missing message, we will check to see the number
    /// of missing/buffered messages exceeds the configured threshold. Where
    /// it's not feasible to recover each packet individually, request a full
    /// playback of every packet since `want_sequence` from the recovery
    /// client.
    ///
    /// TODO: it looks like the things you can do with an async stream are
    ///  pretty limited. We will probably have to implement Stream directly
    ///  and handle all of the async state transitions manually to be able to
    ///  yield a RwLockReadGuard rather than the entire RwLock (without doing
    ///  something weird like Box::leaking it. It will also be easier to
    ///  write the transitions down as smaller bits of logic rather than this
    ///  enormous loop!
    pub fn stream(
        self,
    ) -> Pin<
        Box<
            impl Stream<
                Item = Option<
                    Result<
                        (Arc<RwLock<SubscriberStateWrapper<S>>>, ApplyAction<S>, i64),
                        IrrecoverableError,
                    >,
                >,
            >,
        >,
    > {
        let s = stream! {
            let read_timeout = self.read_timeout;
            let recover_timeout = Duration::from_std(self.recover_timeout).unwrap();
            let mut udp_framed = self.udp_framed;
            let mut heap = self.heap;
            let heap_size_to_sync = self.heap_size_to_sync;
            let state = self.state.clone();
            let mut persistence_buffer = self.persistence_buffer;
            let persistence = self.persistence;
            let mut recovery_client = self.recovery_client;
            // TODO: not ok to unwrap the outer error
            let mut want_sequence = persistence.read_last_sequence().await.unwrap().unwrap_or(0);
            loop {
                debug!("Looking for sequence {} stored: {}", want_sequence, heap.len());
                let heap_action = if heap.is_empty() {
                    SyncStrategyAction::ListenDefault
                } else {
                    Self::process_heap(
                        &mut heap,
                        &mut want_sequence,
                        Utc::now(),
                        recover_timeout,
                        heap_size_to_sync,
                        Arc::clone(&state),
                        &mut persistence_buffer,
                        &persistence
                    ).await
                };

                let next_read_timeout = match heap_action {
                  SyncStrategyAction::ListenDefault => read_timeout,
                  SyncStrategyAction::Listen(next_read_timeout) => next_read_timeout,
                  SyncStrategyAction::Recover => {
                        // recover these packets from the recovery client
                        warn!("Recovering from missing packet. Want: {} ", want_sequence);
                        let request = tonic::Request::new(SequenceRequest {sequence_id: want_sequence});
                        match recovery_client.get_message(request).await {
                            Ok(response) => {
                                // todo: it's slow to stick it on the heap
                                //  but that's easy for now
                                heap.push(Reverse(response.into_inner()));
                                continue;
                            }
                            Err(e) => {
                                error!("Error recovering sequence {} {}", want_sequence, e);
                                std::time::Duration::ZERO
                            }
                        }
                    }
                 SyncStrategyAction::Restream => {
                        // restart the stream from this sequence
                        warn!("Restreaming packets want: {} ", want_sequence);
                        let request = tonic::Request::new(SequenceRequest {sequence_id: want_sequence});
                        match recovery_client.stream_from(request).await {
                            Ok(response) => {
                                // todo: it's slow and mem intense to stick
                                //  it on the heap but that's easy for now
                                let mut stream = response.into_inner();
                                // todo: handle atomic error
                                while let Some(Ok(msg)) = stream.next().await {
                                    trace!("Pushing {}", msg.metadata.sequence);
                                    heap.push(Reverse(msg))
                                }
                                continue;
                            }
                            Err(e) => {
                                // not clear what to try after a restream
                                // fails
                                error!("Error recovering sequence stream from {} {}", want_sequence, e);
                                yield Some(Err(IrrecoverableError::GrpcError));
                                yield None;
                                return;
                            }
                        }
                    }
                };
                // process any messages in the heap first.
                match Self::handle_recv(
                    Self::try_recv(
                        &mut udp_framed,
                        next_read_timeout
                    ).await,
                    &mut heap,
                    Arc::clone(&state),
                    &mut want_sequence,
                    &mut persistence_buffer,
                    &persistence
                ).await {
                    Ok(action) => {
                        // TODO: you shouldn't return ApplyAction here unless the caller can
                        //  expect all variants to be returned.
                        match action {
                            // storing a message doesn't lead to a state change. We should change
                            // state only once the timeout to await for the next sequence (when in
                            // sync) has elapsed.
                            ApplyAction::Store(_sequence) => {}
                            // discarding a message doesn't lead to a state change
                            ApplyAction::Discard(_sequence) => {}
                            // TODO: actually do something about this (just loops again)
                            ApplyAction::Recover => {}
                            ApplyAction::InSequenceNoOp => {
                                yield Some(Ok((Arc::clone(&state), ApplyAction::InSequenceNoOp, want_sequence)));
                            }
                            ApplyAction::Delta(d) => {
                                yield Some(Ok((Arc::clone(&state), ApplyAction::Delta(d), want_sequence)));
                            }
                            ApplyAction::Reset(s) => {
                                yield Some(Ok((Arc::clone(&state), ApplyAction::Reset(s), want_sequence)));
                            }
                            ApplyAction::Timeout => {
                                yield Some(Ok((Arc::clone(&state), ApplyAction::Timeout, want_sequence)));
                            }
                        }
                   }
                   Err(e) => {
                        error!("Fatal: {}", e);
                        // TODO: do I yield None or irrecoverable? or both?
                        yield Some(Err(e));
                        yield None;
                   }
                }
            }
        };
        Box::pin(s)
    }

    // TODO: should return the entire state in the case we reset (and replace our
    // copy with default)
    async fn handle_recv(
        result: Result<RawMessage, RecvError>,
        heap: &mut BinaryHeap<Reverse<RawMessage>>,
        state: Arc<RwLock<SubscriberStateWrapper<S>>>,
        want_sequence: &mut i64,
        persistence_buffer: &mut BytesMut,
        persistence: &P,
    ) -> Result<ApplyAction<S>, IrrecoverableError> {
        match result {
            Ok(msg) => {
                let sequence = msg.metadata.sequence;
                match sequence.cmp(want_sequence) {
                    Ordering::Equal => {
                        Self::handle_in_sequence_message(
                            msg,
                            state,
                            true,
                            want_sequence,
                            persistence_buffer,
                            persistence,
                        )
                        .await
                    }
                    Ordering::Greater => {
                        debug!("Storing message {}", sequence);
                        heap.push(Reverse(msg));
                        Ok(ApplyAction::Store(sequence))
                    }
                    Ordering::Less => {
                        // TODO: I think you could potentially be
                        //  vulnerable to a huge stream of low-sequence
                        //  messages. Have a think about this - it's
                        //  probably good to have some sense bounds on
                        //  this e.g. max number / max time.
                        debug!("Ignoring out of sequence message {}", sequence);
                        Ok(ApplyAction::Discard(sequence))
                    }
                }
            }
            Err(e) => {
                match e {
                    RecvError::Timeout => {
                        if heap.is_empty() {
                            // TODO: set missed heartbeat, yield state
                            error!("Timeout waiting for heartbeat");
                            {
                                let mut lock = state.write().await;
                                // TODO: yield state
                                lock.sync_status = SyncStatus::MissedHeartbeat;
                            }
                            // TODO: apply action isn't really relevant for
                            //  'we didn't hear so just keep trying after
                            //  yielding'
                            Ok(ApplyAction::Timeout)
                        } else {
                            debug!("Out of order message timed out");
                            // TODO: it's neither relevant for 'go and sync'
                            Ok(ApplyAction::Recover)
                        }
                    }
                    RecvError::SocketDisconnect => Err(IrrecoverableError::SocketDisconnect),
                    RecvError::IoError => Err(IrrecoverableError::IoError),
                }
            }
        }
    }

    /// Handle an in sequence message. If streaming, we can consider the new
    /// state to be in sync with the remote version.
    async fn handle_in_sequence_message(
        msg: RawMessage,
        state: Arc<RwLock<SubscriberStateWrapper<S>>>,
        // todo: should be is_replay?
        streaming: bool,
        current_sequence: &mut i64,
        persistence_buffer: &mut BytesMut,
        persistence: &P,
    ) -> Result<ApplyAction<S>, IrrecoverableError> {
        *current_sequence += 1;
        // TODO: don't reserialize?
        persistence_buffer.clear();
        Message::encode(&msg, persistence_buffer).unwrap();
        // TODO: from persistenceerror
        persistence
            .write_message(
                msg.metadata.sequence,
                msg.metadata.timestamp,
                Utc::now().timestamp_nanos(),
                streaming,
                persistence_buffer,
            )
            .await
            .unwrap();
        match msg.payload.expect("Empty payload") {
            Payload::Started(_) => {
                let mut lock = state.write().await;
                if streaming {
                    lock.sync_status = SyncStatus::Sync;
                }
                let state = S::default();
                // TODO: switch with state
                // std::mem::replace(state, lock.state);
                Ok(ApplyAction::Reset(state))
            }
            Payload::Heartbeat(_) => {
                if streaming {
                    let mut lock = state.write().await;
                    lock.sync_status = SyncStatus::Sync;
                }
                Ok(ApplyAction::InSequenceNoOp)
            }
            Payload::Delta(grapevine::Delta { body }) => {
                debug!("Applying delta");
                let mut lock = state.write().await;
                let delta = Message::decode(body.as_slice())?;
                lock.state.apply_delta(delta).unwrap();
                if streaming {
                    lock.sync_status = SyncStatus::Sync;
                }
                // TODO: downstream might want to know what's
                //  changed, but don't necessarily want to pay to
                //  deserialize twice / clone
                Ok(ApplyAction::Delta(Message::decode(body.as_slice())?))
            }
        }
    }

    async fn process_heap(
        heap: &mut BinaryHeap<Reverse<RawMessage>>,
        want_sequence: &mut i64,
        now: DateTime<Utc>,
        out_sequence_tolerance: Duration,
        heap_size_to_sync: usize,
        state: Arc<RwLock<SubscriberStateWrapper<S>>>,
        persistence_buffer: &mut BytesMut,
        persistence: &P,
    ) -> SyncStrategyAction {
        while let Some(next) = heap.peek() {
            let sequence = next.0.metadata.sequence;
            match sequence.cmp(want_sequence) {
                Ordering::Equal => {
                    // it's the sequence we want
                    let msg = heap.pop().unwrap();
                    debug!("Processing buffered message {}", sequence);
                    // TODO: handle error
                    Self::handle_in_sequence_message(
                        msg.0,
                        Arc::clone(&state),
                        false,
                        want_sequence,
                        persistence_buffer,
                        persistence,
                    )
                    .await
                    .unwrap();
                }
                Ordering::Less => {
                    // we've seen this sequence
                    debug!(
                        "Discarding buffered sequence {} (want {})",
                        sequence, *want_sequence
                    );
                    heap.pop().unwrap();
                }
                Ordering::Greater => {
                    // we want messages before this one
                    let send_time = Utc.timestamp_nanos(next.0.metadata.timestamp);
                    let diff = send_time + out_sequence_tolerance - now;
                    // TODO: you should only do this when the state == Synced
                    //  otherwise you might forever stay behind syncing
                    //  individual packets.
                    //  This is king of an argument for keeping the sync
                    //  status outside the state, so we don't need to
                    //  wait to unlock the mutex
                    return if diff > Duration::zero() {
                        // could be transient reordering. don't give up on the
                        // UDP stream just yet.
                        debug!("Will wait {} for out of order seq {}", diff, *want_sequence);
                        SyncStrategyAction::Listen(diff.to_std().unwrap())
                    } else {
                        let effective_len =
                            heap.len().max((sequence - *want_sequence).max(0) as usize);
                        if effective_len > heap_size_to_sync {
                            // we've exceeded the threshold number of missing
                            // messages to recover individually.
                            warn!(
                                "Heap size {} - syncing from {}",
                                effective_len, *want_sequence
                            );
                            // TODO: apply sync_status == Syncing
                            SyncStrategyAction::Restream
                        } else {
                            // we're missing a small number of messages
                            let range = *want_sequence..sequence;
                            warn!("Heap size {} - requesting {:?}", effective_len, range);
                            // TODO: apply sync_status == Syncing
                            SyncStrategyAction::Recover
                        }
                    };
                }
            }
        }
        trace!("Cleared heap");
        SyncStrategyAction::ListenDefault
    }

    /// Wait for `read_timeout` for the next frame or return a [RecvError].
    async fn try_recv(
        udp_framed: &mut UdpFramed<crate::codec::Decoder<RawMessage>, Arc<UdpSocket>>,
        read_timeout: std::time::Duration,
    ) -> Result<RawMessage, RecvError> {
        let res = timeout(read_timeout, udp_framed.next())
            .await
            .map_err(|_| RecvError::Timeout)?;
        let (msg, _) = res.ok_or(RecvError::SocketDisconnect)?.map_err(|e| {
            error!("Error decoding message: {}", e);
            RecvError::IoError
        })?;
        Ok(msg)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApplyAction<S: StateSync> {
    /// Stick this on the heap to process later. Sequence is higher than
    /// desired.
    Store(i64),
    /// Ignore this message. Sequence is lower than desired.
    Discard(i64),
    /// In sequence no-operation. The message is a heartbeat.
    InSequenceNoOp,
    /// An in-sequence delta has been applied. A copy is returned.
    Delta(S::Delta),
    /// The internal state has been cleared. Old state is provided.
    Reset(S),
    /// There are no messages coming from the publisher
    Timeout,
    /// Given up waiting for out of sequence packet
    Recover,
}

/// The set of errors we can get reading a framed message
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
enum RecvError {
    /// No messages were received within the specified time.
    Timeout,
    /// Internal error from [UdpFramed].
    IoError,
    /// The underlying socket returned [None], indicating it was
    /// disconnected.
    SocketDisconnect,
}

/// Errors which we have no resolution for.
/// // TODO: we could provide some more context
#[derive(Debug, Copy, Clone)]
pub enum IrrecoverableError {
    DeserializationError,
    SocketDisconnect,
    IoError,
    GrpcError,
    PersistenceError,
}

impl fmt::Display for IrrecoverableError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DeserializationError => f.write_str("Deserialization error"),
            Self::SocketDisconnect => f.write_str("Socket disconnect"),
            Self::IoError => f.write_str("Io error"),
            Self::GrpcError => f.write_str("Grpc error"),
            Self::PersistenceError => f.write_str("Persistence error"),
        }
    }
}

impl Error for IrrecoverableError {}

impl From<DecodeError> for IrrecoverableError {
    fn from(_: DecodeError) -> Self {
        Self::DeserializationError
    }
}

/// When processing our buffered messages, we can communicate to the
/// receiving logic what we want to do, based on information we get from
/// the buffered heap.
#[derive(Debug, Clone, PartialEq, Eq)]
enum SyncStrategyAction {
    /// There was no evidence from the heap that we should do anything in
    /// particular. It was either empty or contained old messages.
    ListenDefault,
    /// The next sequence in the stream is larger than the one we're
    /// expecting, but it was received recently and we're configured to
    /// not give up on a message for a configured interval. This returns
    /// the amount of that interval remaining which we should wait for.
    Listen(std::time::Duration),
    /// We are missing a small number of messages which can be recovered
    /// individually.
    Recover,
    /// We're missing a sufficiently large number of packets that requesting
    /// them individually is not a good idea.
    Restream,
}

/// Wraps the state and provides a status field to communicate the Subscriber
/// sync status
#[derive(Debug, Default, PartialEq, Eq, Clone)]
pub struct SubscriberStateWrapper<S> {
    state: S,
    sync_status: SyncStatus,
}

impl<S> SubscriberStateWrapper<S> {
    pub fn new(state: S) -> Self {
        Self {
            state,
            sync_status: SyncStatus::Syncing,
        }
    }

    pub fn sync_status(&self) -> SyncStatus {
        self.sync_status
    }

    pub fn state(&self) -> &S {
        &self.state
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{mock::MockSubscriberPersistence, proto::tree_state::TreeState};

    #[tokio::test]
    async fn process_heap_seen_sequences() {
        let mut heap: BinaryHeap<Reverse<RawMessage>> = Default::default();
        (0..4).for_each(|i| {
            heap.push(Reverse(RawMessage {
                metadata: grapevine::Metadata {
                    sequence: i,
                    ..Default::default()
                },
                payload: None,
            }));
        });
        let mut want_sequence: i64 = 5;
        let state = Arc::new(RwLock::new(SubscriberStateWrapper::new(
            TreeState::default(),
        )));
        let persistence = MockSubscriberPersistence::default();
        let mut persistence_buffer = BytesMut::new();

        assert_eq!(
            SubscriberStream::process_heap(
                &mut heap,
                &mut want_sequence,
                Utc::now(),
                Duration::zero(),
                0,
                state.clone(),
                &mut persistence_buffer,
                &persistence
            )
            .await,
            SyncStrategyAction::ListenDefault
        );
        assert_eq!(heap.len(), 0);
        assert_eq!(want_sequence, 5);
        let state = state.read().await;
        assert_eq!(*state, SubscriberStateWrapper::new(TreeState::default()));
    }
}
