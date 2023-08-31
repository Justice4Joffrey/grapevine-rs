//! Communication should be connectionless and one-way where possible.
//!
//! For this reason, every publisher should have a `HeartBeatLoop` which
//! is responsible for sending periodic messages to subscribers, allowing
//! them to know when a publisher has stopped.
//!
//! Just like deltas, heartbeat messages must also be given a unique
//! sequenced and be written to log files.

use std::{
    sync::{mpsc::Receiver, Arc},
    time::Duration,
};

use bytes::BytesMut;
use chrono::Utc;
use tokio::{net::UdpSocket, sync::RwLock, time::sleep};
use tracing::{debug, error};

use crate::{
    constants::MAX_PAYLOAD_SIZE,
    persistence::PublisherPersistence,
    proto::grapevine::{raw_message::Payload, Heartbeat, Metadata, RawMessage},
    publisher::{state_wrapper::PublisherStateWrapper, Publisher},
    should_destruct,
};

/// The state of the heartbeat task
pub(super) struct HeartBeatLoop<S, P> {
    receiver: Receiver<()>,
    socket: Arc<UdpSocket>,
    interval: Duration,
    state: Arc<RwLock<PublisherStateWrapper<S>>>,
    buffer: BytesMut,
    encoder: crate::codec::Encoder<RawMessage>,
    persistence: Arc<P>,
}

impl<S, P: PublisherPersistence> HeartBeatLoop<S, P> {
    /// Create a new instance.
    pub fn new(
        receiver: Receiver<()>,
        socket: Arc<UdpSocket>,
        interval: Duration,
        state: Arc<RwLock<PublisherStateWrapper<S>>>,
        persistence: Arc<P>,
    ) -> Self {
        Self {
            receiver,
            socket,
            interval,
            state,
            buffer: BytesMut::with_capacity(MAX_PAYLOAD_SIZE),
            encoder: Default::default(),
            persistence,
        }
    }

    /// A task which will send heartbeats periodically until a destructor
    /// signal is sent.
    pub async fn run(&mut self) {
        loop {
            if should_destruct(&mut self.receiver) {
                break;
            }
            self.heartbeat().await;
            sleep(self.interval).await;
        }
    }

    /// Publish a heartbeat message.
    async fn heartbeat(&mut self) {
        let (sequence, timestamp) = {
            let mut lock = self.state.write().await;
            (lock.stamp(), Utc::now().timestamp_nanos())
        };
        debug!("Sending heartbeat seq {}", sequence);

        let message = RawMessage {
            metadata: Metadata {
                timestamp,
                sequence,
            },
            payload: Some(Payload::Heartbeat(Heartbeat {})),
        };

        // It's probably not critical to exit on a failed heartbeat.
        // Subscribers can just request this message via replay, with a
        // smaller time penalty than crashing.
        if let Err(e) = Publisher::<S, P>::publish(
            Arc::clone(&self.persistence),
            &mut self.encoder,
            Arc::clone(&self.socket),
            message,
            self.buffer.split(),
        )
        .await
        {
            error!("Error sending heartbeat: {}", e);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{
        net::{Ipv4Addr, SocketAddrV4},
        sync::{mpsc, Arc},
        time::Duration,
    };

    use tokio_stream::StreamExt;
    use tokio_util::udp::UdpFramed;

    use crate::{
        persistence::mock::MockPublisherPersistence,
        proto::{
            grapevine,
            grapevine::{raw_message::Payload, Metadata, RawMessage},
        },
        publisher::heartbeat::HeartBeatLoop,
        transport::{new_multicast_publisher, new_multicast_socket},
    };

    #[tokio::test]
    async fn test_heartbeat() {
        let (_tx, rx) = mpsc::channel();
        let mc_addr = SocketAddrV4::new(Ipv4Addr::new(224, 0, 0, 124), 4536);
        let publisher = new_multicast_publisher(&mc_addr).await.unwrap();
        let mut subscriber = UdpFramed::new(
            new_multicast_socket(&mc_addr).unwrap(),
            crate::codec::Decoder::<RawMessage>::new(),
        );
        let persistence = Arc::new(MockPublisherPersistence::new());
        let mut heartbeat_loop = HeartBeatLoop::<(), _>::new(
            rx,
            Arc::new(publisher),
            Duration::from_millis(100),
            Default::default(),
            persistence,
        );
        heartbeat_loop.heartbeat().await;
        heartbeat_loop.heartbeat().await;
        let (message, _) = subscriber.next().await.unwrap().unwrap();
        assert_eq!(
            message,
            RawMessage {
                metadata: Metadata {
                    timestamp: message.metadata.timestamp,
                    sequence: 0
                },
                payload: Some(Payload::Heartbeat(grapevine::Heartbeat {}))
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
                payload: Some(Payload::Heartbeat(grapevine::Heartbeat {}))
            }
        );
    }
}
