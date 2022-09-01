use derive_more::{Display, Error, From};
use tonic::Status;

/// Newtype around [sqlx::Error]
#[derive(Debug, Display, Error, From)]
pub struct SqlError(sqlx::Error);

impl From<SqlError> for Status {
    fn from(error: SqlError) -> Self {
        Self::internal(error.to_string())
    }
}
