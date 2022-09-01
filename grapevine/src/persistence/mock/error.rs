use std::error::Error;

use derive_more::Display;
use tonic::Status;

#[derive(Debug, Display)]
pub enum MockPersistenceError {
    SequenceExists(i64),
    SequenceNotFound(i64),
    ConnectionError,
}

impl Error for MockPersistenceError {}

impl From<MockPersistenceError> for Status {
    fn from(error: MockPersistenceError) -> Self {
        Self::internal(error.to_string())
    }
}
