//! A [Publisher] is responsible for providing the part of your application
//! generating state changes (the 'source of truth') an API to propagate
//! these changes to the rest of your system.
//!
//! All publishers must have:
//!
//! - A `heartbeat` process. This signals to subscribers that the process is
//! still alive.
//! - A `persistence` layer. This stores each individual message in a
//! `master_log` (as opposed to a subscriber's `replica_log`).
//!
//! Currently not-optional (but will be eventually):
//!
//! - A local recovery server. This facilitates replay of lost messages
//! (both transient packet loss and messages missed during downtime).
//!
//! Example usage:
//!
//! ``` no_run
//! # use std::net::{Ipv4Addr, SocketAddrV4};
//! # use std::str::FromStr;
//! # use grapevine::Publisher;
//! # use grapevine::transport::new_multicast_publisher;
//! # use grapevine::mock::MockPublisherPersistence;
//! # use grapevine::proto::grapevine::RawMessage;
//! # use grapevine::proto::tree_state::{leaf_delta::Delta, DeleteLeaf, LeafDelta, TreeState};
//! # use grapevine::StateSync;
//! #
//! # async fn inner() {
//! # let recovery_addr = SocketAddrV4::new(Ipv4Addr::new(0, 0, 0, 0), 7755);
//! # let mc_addr = SocketAddrV4::new(Ipv4Addr::new(224, 0, 0, 123), 1234);
//! # let socket = new_multicast_publisher(&mc_addr).await.unwrap();
//! #
//! let persistence = MockPublisherPersistence::new();
//! let publisher = Publisher::<TreeState, _>::new(socket, persistence);
//! // start the publisher
//! let mut publisher_context = publisher.run(recovery_addr.into()).await.unwrap();
//!
//! // produce some data!
//! // `apply_delta` will:
//! // - store the message to the master log.
//! // - send the message over multicast.
//! let sequence = publisher_context
//!     .apply_delta(LeafDelta {
//!         delta: Some(Delta::DeleteLeaf(DeleteLeaf { id: 1 })),
//!     })
//!     .await
//!     .unwrap();
//! # }
//! ```

mod context;
mod heartbeat;
mod recovery;
mod state_wrapper;

use std::{
    error::Error,
    marker::PhantomData,
    net::SocketAddr,
    pin::Pin,
    sync::{mpsc, Arc},
    time::Duration,
};

use bytes::BytesMut;
use chrono::Utc;
pub use context::PublisherContext;
use tokio::{net::UdpSocket, sync::RwLock};
use tokio_stream::Stream;
use tokio_util::codec::Encoder;
use tonic::Status;

use crate::{
    constants::HEADER_SIZE,
    persistence::PublisherPersistence,
    proto::grapevine::{raw_message::Payload, Metadata, RawMessage, Started},
    publisher::{
        heartbeat::HeartBeatLoop, recovery::PublisherRecovery, state_wrapper::PublisherStateWrapper,
    },
    StateSync,
};

pub type MessageStream<T> = Pin<Box<dyn Stream<Item = Result<T, Status>> + Send>>;

pub struct Publisher<S, P> {
    socket: Arc<UdpSocket>,
    encoder: crate::codec::Encoder<RawMessage>,
    persistence: Arc<P>,
    phantom: PhantomData<S>,
}

impl<S, P: PublisherPersistence> Publisher<S, P> {
    /// Serializes a [RawMessage] in to the supplied buffer, writes it to
    /// the persistence and sends it over the socket.
    pub async fn publish(
        persistence: Arc<P>,
        encoder: &mut crate::codec::Encoder<RawMessage>,
        socket: Arc<UdpSocket>,
        message: RawMessage,
        mut buffer: BytesMut,
    ) -> Result<usize, Box<dyn Error>> {
        let sequence = message.metadata.sequence;
        let timestamp = message.metadata.timestamp;

        encoder.encode(message, &mut buffer)?;
        let buffer = buffer.freeze();
        persistence
            .write_message(sequence, timestamp, &buffer[HEADER_SIZE..])
            .await?;
        let bytes = socket.send(buffer.as_ref()).await?;

        Ok(bytes)
    }
}

impl<S: StateSync, P: PublisherPersistence> Publisher<S, P> {
    /// Create a new instance.
    pub fn new(socket: UdpSocket, persistence: P) -> Self {
        Self {
            socket: Arc::new(socket),
            encoder: Default::default(),
            persistence: Arc::new(persistence),
            phantom: Default::default(),
        }
    }

    /// Run this Publisher. This entails starting the recovery server,
    /// sending a [Payload::Started], starting the heartbeat loop and
    /// moving the publisher in to a [PublisherContext], which represents a
    /// [Publisher] running in the background.
    pub async fn run(
        mut self,
        recovery_address: SocketAddr,
    ) -> Result<PublisherContext<S, P>, Box<dyn Error>> {
        // Initialize the state
        let sequence = self.persistence.next_sequence().await?;
        let state = Arc::new(RwLock::new(PublisherStateWrapper::with_sequence(sequence)));

        // Start the recovery server
        let recovery = PublisherRecovery::new(Arc::clone(&self.persistence));
        let (rec_tx, rec_rx) = mpsc::channel();
        let _recovery_handle =
            tokio::spawn(async move { recovery.run(recovery_address, rec_rx).await });

        // Send a [Payload::Started] message
        {
            let mut guard = state.write().await;
            let sequence = guard.stamp();
            let message = RawMessage {
                metadata: Metadata {
                    timestamp: Utc::now().timestamp_nanos(),
                    sequence,
                },
                payload: Some(Payload::Started(Started {})),
            };
            let mut buffer = BytesMut::new();
            Self::publish(
                Arc::clone(&self.persistence),
                &mut self.encoder,
                Arc::clone(&self.socket),
                message,
                buffer,
            )
            .await?;
        }

        // Start sending heartbeats
        let (hb_tx, hb_rx) = mpsc::channel();
        let mut heartbeat_loop = HeartBeatLoop::new(
            hb_rx,
            Arc::clone(&self.socket),
            Duration::from_secs(3),
            Arc::clone(&state),
            Arc::clone(&self.persistence),
        );
        let _heartbeat_handle = tokio::spawn(async move { heartbeat_loop.run().await });

        // TODO: structured concurrency
        // let bg_future = (recovery_handle, heartbeat_handle).race();

        // Move in to a [PublisherContext]
        Ok(PublisherContext::new(
            state,
            self.socket,
            hb_tx,
            rec_tx,
            Arc::clone(&self.persistence),
        ))
    }
}
