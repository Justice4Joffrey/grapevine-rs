use async_stream::stream;
use sqlx::{Row, SqlitePool};

use super::error::SqlError;
use crate::{persistence::PublisherPersistence, publisher::MessageStream};

/// A persistence type backed by sqlite.
pub struct SqlitePublisherPersistence {
    /// Connection pool
    pool: SqlitePool,
}

impl SqlitePublisherPersistence {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }
}

#[async_trait::async_trait]
impl PublisherPersistence for SqlitePublisherPersistence {
    type Error = SqlError;

    async fn next_sequence(&self) -> Result<i64, Self::Error> {
        let mut conn = self.pool.acquire().await?;
        let maybe_row = sqlx::query(
            r#"
                SELECT id
                FROM master_log
                ORDER BY id DESC
                LIMIT 1
            "#,
        )
        .fetch_optional(&mut conn)
        .await?;
        Ok(maybe_row.map(|r| r.get::<i64, _>("id") + 1).unwrap_or(0))
    }

    async fn write_message(
        &self,
        sequence: i64,
        timestamp: i64,
        buf: &[u8],
    ) -> Result<(), Self::Error> {
        let mut conn = self.pool.acquire().await?;
        sqlx::query(
            r#"
                INSERT INTO master_log (id, ts, raw)
                VALUES (?1, ?2, ?3)
            "#,
        )
        .bind(sequence)
        .bind(timestamp)
        .bind(buf)
        .execute(&mut conn)
        .await?;
        Ok(())
    }

    async fn read_sequence(&self, sequence: i64) -> Result<Vec<u8>, Self::Error> {
        let mut conn = self.pool.acquire().await?;
        let row = sqlx::query(
            r#"
                SELECT raw
                FROM master_log
                WHERE id = ?1
            "#,
        )
        .bind(sequence)
        .fetch_one(&mut conn)
        .await?;
        Ok(row.get::<Vec<u8>, _>("raw"))
    }

    async fn stream_from_sequence(
        &self,
        sequence: i64,
    ) -> Result<MessageStream<Vec<u8>>, Self::Error> {
        let mut conn = self.pool.acquire().await?;
        let rows = sqlx::query(
            r#"
                SELECT id, raw
                FROM master_log
                WHERE id >= ?1
                ORDER BY id
            "#,
        )
        .bind(sequence)
        .fetch_all(&mut conn)
        .await?;
        // TODO: figure out how to keep a &mut conn alive so that we can
        //  use .fetch() above and stream directly from the OS to the gRPC
        //  call (we're currently loading all in to memory).
        // TODO: below is a hack to keep the same semantics as the trait
        //  requires
        Ok(Box::pin(stream! {
            for row in rows {
                yield Ok(row.get::<Vec<u8>, _>("raw"));
            }
        }))
    }
}
