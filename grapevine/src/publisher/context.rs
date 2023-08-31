//! [PublisherContext] is the public interface to a collection of publisher
//! tasks running in the background. The main API for application code is
//! the [apply_delta](PublisherContext::apply_delta) method, which will
//! update the internal state, record a message to the persistence log and
//! publish it to any subscribers.
//!
//! Upon dropping a [PublisherContext], a destructor signal will be sent to
//! the spawned processes causing them to shut down.

use std::{
    error::Error,
    sync::{mpsc::Sender, Arc},
};

use bytes::BytesMut;
use chrono::Utc;
use prost::Message;
use tokio::{
    net::UdpSocket,
    sync::{RwLock, RwLockReadGuard},
};
use tracing::error;

use crate::{
    constants::MAX_PAYLOAD_SIZE,
    persistence::PublisherPersistence,
    proto::grapevine::{raw_message::Payload, Delta, Metadata, RawMessage},
    publisher::{state_wrapper::PublisherStateWrapper, Publisher},
    StateSync,
};

/// Represents a publisher which has activated its background tasks.
/// This is effectively the public interface for a running [Publisher]
pub struct PublisherContext<S, P> {
    state: Arc<RwLock<PublisherStateWrapper<S>>>,
    socket: Arc<UdpSocket>,
    heartbeat_destructor: Sender<()>,
    recovery_destructor: Sender<()>,
    buffer: BytesMut,
    encoder: crate::codec::Encoder<RawMessage>,
    persistence: Arc<P>,
}

impl<S: StateSync, P: PublisherPersistence> PublisherContext<S, P> {
    pub(super) fn new(
        state: Arc<RwLock<PublisherStateWrapper<S>>>,
        socket: Arc<UdpSocket>,
        heartbeat_destructor: Sender<()>,
        recovery_destructor: Sender<()>,
        persistence: Arc<P>,
    ) -> Self {
        Self {
            state,
            socket,
            heartbeat_destructor,
            recovery_destructor,
            buffer: BytesMut::with_capacity(65536),
            encoder: Default::default(),
            persistence,
        }
    }

    /// Applies a `Delta` to the internal `State`, wraps it in a
    /// [RawMessage], stores and publishes it.
    pub async fn apply_delta(&mut self, delta: S::Delta) -> Result<i64, Box<dyn Error>> {
        let message = {
            let mut guard = self.state.write().await;
            let sequence = guard.stamp();
            let body = delta.encode_to_vec();
            guard.state_mut().apply_delta(delta)?;
            RawMessage {
                metadata: Metadata {
                    timestamp: Utc::now().timestamp_nanos(),
                    sequence,
                },
                payload: Some(Payload::Delta(Delta { body })),
            }
        };
        let sequence = message.metadata.sequence;
        Publisher::<S, P>::publish(
            Arc::clone(&self.persistence),
            &mut self.encoder,
            Arc::clone(&self.socket),
            message,
            self.buffer.split(),
        )
        .await?;
        Ok(sequence)
    }

    /// Get a read-only reference to the current state.
    pub async fn read(&self) -> RwLockReadGuard<'_, PublisherStateWrapper<S>> {
        self.state.read().await
    }
}

impl<S, P> Drop for PublisherContext<S, P> {
    fn drop(&mut self) {
        if let Err(e) = self.heartbeat_destructor.send(()) {
            error!("Failed to send the heartbeat destructor message: {e}");
        }
        if let Err(e) = self.recovery_destructor.send(()) {
            error!("Failed to send the recovery destructor message: {e}");
        }
    }
}

#[cfg(test)]
mod tests {
    use std::net::{Ipv4Addr, SocketAddrV4};

    use tokio_stream::StreamExt;
    use tokio_util::udp::UdpFramed;

    use super::*;
    use crate::{
        persistence::mock::MockPublisherPersistence,
        proto::{
            grapevine,
            tree_state::{leaf_delta::Delta, *},
        },
        transport::{new_multicast_publisher, new_multicast_socket},
    };

    #[tokio::test]
    async fn apply_delta() {
        let (tx1, _rx1) = std::sync::mpsc::channel();
        let (tx2, _rx2) = std::sync::mpsc::channel();
        let mc_addr = SocketAddrV4::new(Ipv4Addr::new(224, 0, 0, 124), 4535);
        let publisher = new_multicast_publisher(&mc_addr).await.unwrap();
        let mut subscriber = UdpFramed::new(
            new_multicast_socket(&mc_addr).unwrap(),
            crate::codec::Decoder::<RawMessage>::new(),
        );
        let persistence = Arc::new(MockPublisherPersistence::new());
        let mut context = PublisherContext::<TreeState, _>::new(
            Default::default(),
            Arc::new(publisher),
            tx1,
            tx2,
            persistence,
        );

        let delta1 = LeafDelta {
            delta: Some(Delta::NewLeaf(NewLeaf {
                leaf: Leaf { id: 1, a: 2, b: 3 },
            })),
        };
        let body1 = delta1.encode_to_vec();
        context.apply_delta(delta1).await.unwrap();
        let delta2 = LeafDelta {
            delta: Some(Delta::NewLeaf(NewLeaf {
                leaf: Leaf { id: 2, a: 3, b: 4 },
            })),
        };
        let body2 = delta2.encode_to_vec();
        context.apply_delta(delta2).await.unwrap();
        let guard = context.state.read().await;
        assert_eq!(guard.state().leaves.len(), 2);
        assert_eq!(guard.sequence(), 2);
        let (message, _) = subscriber.next().await.unwrap().unwrap();
        assert_eq!(
            message,
            RawMessage {
                metadata: Metadata {
                    timestamp: message.metadata.timestamp,
                    sequence: 0
                },
                payload: Some(Payload::Delta(grapevine::Delta { body: body1 }))
            }
        );
        let (message, _) = subscriber.next().await.unwrap().unwrap();
        assert_eq!(
            message,
            RawMessage {
                metadata: Metadata {
                    timestamp: message.metadata.timestamp,
                    sequence: 1
                },
                payload: Some(Payload::Delta(grapevine::Delta { body: body2 }))
            }
        );
    }
}
