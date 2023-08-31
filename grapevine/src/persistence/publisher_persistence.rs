//! It is essential for [Publishers](crate::Publisher) to create a master
//! record of every message sent in order to facilitate recovery and
//! guarantee unique sequence numbers between restarts.

use std::error::Error;

use async_trait::async_trait;
use tonic::Status;

use crate::publisher::MessageStream;

/// This is the interface for persisting messages to disk.
/// All messages must be persisted before being sent to facilitate recovery.
#[async_trait]
pub trait PublisherPersistence: Send + Sync + 'static {
    /// An error occurring from an operation.
    type Error: Into<Status> + Error;

    /// Get the next sequence number from the persistence store (or 0).
    /// This might be retrieved by checking the last message recorded being
    /// sent.
    async fn next_sequence(&self) -> Result<i64, Self::Error>;

    /// Write a message buffer to the persistence layer.
    async fn write_message(&self, sequence: i64, ts: i64, buf: &[u8]) -> Result<(), Self::Error>;

    /// Recover a previously written message.
    async fn read_sequence(&self, sequence: i64) -> Result<Vec<u8>, Self::Error>;

    /// Recover everything stored from this sequence (inclusive). It's
    /// expected that consumers will open a live stream and buffer it to
    /// process after this stream has ended.
    async fn stream_from_sequence(
        &self,
        sequence: i64,
    ) -> Result<MessageStream<Vec<u8>>, Self::Error>;
}
