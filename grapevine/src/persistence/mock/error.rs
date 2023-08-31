use std::error::Error;
use thiserror::Error;

use tonic::Status;

#[derive(Debug, Error)]
pub enum MockPersistenceError {
    #[error("sequence {0} already exists")]
    SequenceExists(i64),
    #[error("sequence {0} not found")]
    SequenceNotFound(i64),
    #[error("connection error")]
    ConnectionError,
}

impl From<MockPersistenceError> for Status {
    fn from(e: MockPersistenceError) -> Self {
        Status::internal(format!("mock persistence error {e}"))
    }
}