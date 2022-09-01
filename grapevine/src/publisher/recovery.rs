//! The recovery API allows subscribers to request
//! [individual messages](PublisherRecovery::get_message) by sequence number
//! or [stream messages](PublisherRecovery::stream_from) _from_ a sequence
//! number.
//!
//! In the case where packets are dropped, or a subscriber is restarted, the
//! methods above can be used to synchronize a subscriber to the publisher's
//! state.
//!
//! Every publisher should have a [RecoveryApiServer] running. This _should_
//! be seen as the last-resort, however. The set of processes allowed to
//! talk directly to a publisher should be limited. Instead, having
//! designated subscribers who are public [RecoveryApiServer]s is a more
//! scalable approach.

use std::{
    net::SocketAddr,
    sync::{mpsc::Receiver, Arc},
};

use futures::stream::StreamExt;
use prost::Message;
use tokio::select;
use tonic::{transport::Server, Request, Response, Status};
use tracing::{info, warn};

use crate::{
    destruct_future,
    persistence::PublisherPersistence,
    proto::{
        grapevine::RawMessage,
        recovery::{
            recovery_api_server::{RecoveryApi, RecoveryApiServer},
            SequenceRequest,
        },
    },
    publisher::MessageStream,
};

/// Represents the recovery state of a [Publisher]. Processes connecting to
/// this will be able to retrieve deltas from the [Publisher]'s master
/// record.
#[derive(Debug)]
pub(super) struct PublisherRecovery<P> {
    /// The internal record of this recovery server.
    persistence: Arc<P>,
}

impl<P: PublisherPersistence> PublisherRecovery<P> {
    /// Create a new instance.
    pub fn new(persistence: Arc<P>) -> Self {
        Self { persistence }
    }

    /// Bind a [RecoveryApiServer] to the socket provided and listen until
    /// a destructor message is sent.
    pub async fn run(self, address: SocketAddr, destructor: Receiver<()>) {
        info!("Recovery server running on {}", address);
        select! {
            res = Server::builder()
                .add_service(RecoveryApiServer::new(self))
                .serve(address) => {
                warn!("Recovery server exited: {:?}", res);
            },
            _ = destruct_future(destructor) => {
                warn!("Recovery server received destructor signal");
            }
        }
    }

    /// Decode bytes in to a [RawMessage].
    fn decode(bytes: &[u8]) -> Result<RawMessage, Status> {
        Message::decode(bytes).map_err(|e| Status::internal(e.to_string()))
    }

    /// Decode a [Result] of owned bytes in to a Result of [RawMessage]
    /// (essentially `flat_map`).
    fn decode_result(bytes: Result<Vec<u8>, Status>) -> Result<RawMessage, Status> {
        bytes.map(|b| Self::decode(b.as_slice()))?
    }
}

#[tonic::async_trait]
impl<P: PublisherPersistence> RecoveryApi for PublisherRecovery<P> {
    type StreamFromStream = MessageStream<RawMessage>;

    async fn get_message(
        &self,
        request: Request<SequenceRequest>,
    ) -> Result<Response<RawMessage>, Status> {
        let inner = request.into_inner();
        match self.persistence.read_sequence(inner.sequence_id).await {
            Ok(bytes) => {
                let message = Self::decode(bytes.as_slice())?;
                Ok(Response::new(message))
            }
            Err(e) => Err(e.into()),
        }
    }

    async fn stream_from(
        &self,
        request: Request<SequenceRequest>,
    ) -> Result<Response<Self::StreamFromStream>, Status> {
        let inner = request.into_inner();
        match self
            .persistence
            .stream_from_sequence(inner.sequence_id)
            .await
        {
            Ok(bytes_stream) => {
                let msg_stream = bytes_stream.map(Self::decode_result);
                Ok(Response::new(Box::pin(msg_stream)))
            }
            Err(e) => Err(e.into()),
        }
    }
}
