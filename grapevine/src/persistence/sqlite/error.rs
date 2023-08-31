use thiserror::Error;
use tonic::Status;

/// Newtype around [sqlx::Error]
#[derive(Debug, Error)]
#[error("sqlx error")]
pub struct SqlError(#[from] sqlx::Error);

impl From<SqlError> for Status {
    fn from(error: SqlError) -> Self {
        Self::internal(error.to_string())
    }
}
