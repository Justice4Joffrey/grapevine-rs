//! Subscribers _may_ need to keep a replica log of messages received in
//! order to track previously consumed messages.

use std::{error::Error, fmt::Debug};

use tonic::Status;

/// This is the interface for persisting received messages to disk.
/// All messages must be persisted before being processed to facilitate
/// diagnosis.
#[async_trait::async_trait]
pub trait SubscriberPersistence: Send + Sync + 'static {
    /// An error emerging from an operation
    type Error: Debug + Into<Status> + Error;

    /// Recover a previously written message. None signifies nothing ever
    /// received.
    async fn read_last_sequence(&self) -> Result<Option<i64>, Self::Error>;

    /// Write a message buffer to the persistence layer.
    async fn write_message(
        &self,
        sequence: i64,
        timestamp: i64,
        recv_timestamp: i64,
        is_replay: bool,
        buf: &[u8],
    ) -> Result<(), Self::Error>;
}
