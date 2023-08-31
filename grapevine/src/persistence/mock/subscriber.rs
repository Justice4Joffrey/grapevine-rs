use std::collections::BTreeMap;

use tokio::sync::Mutex;

use crate::{mock::MockPersistenceError, SubscriberPersistence};

#[derive(Debug, Default)]
pub struct MockSubscriberPersistence {
    messages: Mutex<BTreeMap<i64, Vec<u8>>>,
}

#[async_trait::async_trait]
impl SubscriberPersistence for MockSubscriberPersistence {
    type Error = MockPersistenceError;

    async fn read_last_sequence(&self) -> Result<Option<i64>, Self::Error> {
        Ok(self.messages.lock().await.keys().max().map(|v| *v))
    }

    async fn write_message(
        &self,
        sequence: i64,
        _: i64,
        _: i64,
        _: bool,
        buf: &[u8],
    ) -> Result<(), Self::Error> {
        self.messages.lock().await.insert(sequence, buf.to_vec());
        Ok(())
    }
}
