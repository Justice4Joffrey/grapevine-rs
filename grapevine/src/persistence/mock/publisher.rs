use std::collections::BTreeMap;

use async_stream::stream;
use tokio::sync::{Mutex, MutexGuard};

use crate::{
    persistence::{mock::error::MockPersistenceError, PublisherPersistence},
    publisher::MessageStream,
};

/// An in-memory persistence service. Don't use this for any real-life
/// purpose, as it has no functionality to clear up its memory usage.
#[derive(Debug, Default)]
pub struct MockPublisherPersistence {
    messages: Mutex<BTreeMap<i64, Vec<u8>>>,
}

impl MockPublisherPersistence {
    pub fn new() -> Self {
        Default::default()
    }

    pub async fn messages(&self) -> MutexGuard<'_, BTreeMap<i64, Vec<u8>>> {
        self.messages.lock().await
    }
}

#[async_trait::async_trait]
impl PublisherPersistence for MockPublisherPersistence {
    type Error = MockPersistenceError;

    async fn next_sequence(&self) -> Result<i64, Self::Error> {
        Ok(*self.messages.lock().await.keys().max().unwrap_or(&0))
    }

    async fn write_message(&self, sequence: i64, _: i64, buf: &[u8]) -> Result<(), Self::Error> {
        let mut lock = self.messages.lock().await;
        if !lock.contains_key(&sequence) {
            lock.insert(sequence, buf.to_vec());
            Ok(())
        } else {
            Err(MockPersistenceError::SequenceExists(sequence))
        }
    }

    async fn read_sequence(&self, sequence: i64) -> Result<Vec<u8>, Self::Error> {
        let lock = self.messages.lock().await;
        lock.get(&sequence)
            .map(Clone::clone)
            .ok_or(MockPersistenceError::SequenceExists(sequence))
    }

    async fn stream_from_sequence(
        &self,
        sequence: i64,
    ) -> Result<MessageStream<Vec<u8>>, Self::Error> {
        let rows = {
            let lock = self.messages.lock().await;
            lock.range(sequence..)
                .map(|(_, v)| v.clone())
                .collect::<Vec<_>>()
        };

        Ok(Box::pin(stream! {
            for row in rows {
                yield Ok(row);
            }
        }))
    }
}
