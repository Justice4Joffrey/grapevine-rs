use sqlx::{Row, SqlitePool};

use super::error::SqlError;
use crate::persistence::SubscriberPersistence;

/// A persistence type backed by sqlite.
pub struct SqliteSubscriberPersistence {
    /// Connection pool
    pool: SqlitePool,
}

impl SqliteSubscriberPersistence {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }
}

#[async_trait::async_trait]
impl SubscriberPersistence for SqliteSubscriberPersistence {
    type Error = SqlError;

    async fn read_last_sequence(&self) -> Result<Option<i64>, Self::Error> {
        let mut conn = self.pool.acquire().await?;
        let maybe_row = sqlx::query(
            r#"
                SELECT id
                FROM replica_log
                ORDER BY id DESC
                LIMIT 1
            "#,
        )
        .fetch_optional(&mut conn)
        .await?;
        Ok(maybe_row.map(|r| r.get::<i64, _>("id")))
    }

    async fn write_message(
        &self,
        sequence: i64,
        timestamp: i64,
        recv_timestamp: i64,
        is_replay: bool,
        buf: &[u8],
    ) -> Result<(), Self::Error> {
        let mut conn = self.pool.acquire().await?;
        sqlx::query(
            r#"
                INSERT INTO replica_log (id, ts, recv_ts, replay, raw)
                VALUES (?1, ?2, ?3, ?4, ?5)
            "#,
        )
        .bind(sequence)
        .bind(timestamp)
        .bind(recv_timestamp)
        .bind(is_replay)
        .bind(buf)
        .execute(&mut conn)
        .await?;
        Ok(())
    }
}
