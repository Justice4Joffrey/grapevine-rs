use futures_util::stream::StreamExt;
use grapevine::{
    proto::grapevine::{raw_message::Payload, Heartbeat, Metadata, RawMessage},
    sqlite::SqlitePublisherPersistence,
    PublisherPersistence,
};
use prost::Message;
use sqlx::SqlitePool;

fn heartbeat(sequence: i64) -> RawMessage {
    RawMessage {
        metadata: Metadata {
            sequence,
            ..Default::default()
        },
        payload: Some(Payload::Heartbeat(Heartbeat {})),
    }
}

async fn test_db_pool() -> SqlitePool {
    let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
    sqlx::migrate!("../grapevine/migrations")
        .run(&pool)
        .await
        .unwrap();
    pool
}

#[tokio::test]
async fn initialize_empty() {
    let pool = test_db_pool().await;
    let persistence = SqlitePublisherPersistence::new(pool);
    assert_eq!(persistence.next_sequence().await.unwrap(), 0);
}

#[tokio::test]
async fn initialize_exists() {
    let pool = test_db_pool().await;
    let persistence = SqlitePublisherPersistence::new(pool);
    let msg = heartbeat(123);
    persistence
        .write_message(
            msg.metadata.sequence,
            msg.metadata.timestamp,
            Message::encode_to_vec(&msg).as_slice(),
        )
        .await
        .unwrap();
    assert_eq!(
        persistence.next_sequence().await.unwrap(),
        msg.metadata.sequence + 1
    );
}

#[tokio::test]
async fn write_read_message() {
    let pool = test_db_pool().await;
    let msg = heartbeat(1234);
    let persistence = SqlitePublisherPersistence::new(pool);
    let bytes = Message::encode_to_vec(&msg);
    persistence
        .write_message(
            msg.metadata.sequence,
            msg.metadata.timestamp,
            bytes.as_slice(),
        )
        .await
        .unwrap();
    assert_eq!(
        persistence
            .read_sequence(msg.metadata.sequence)
            .await
            .unwrap(),
        bytes
    );
}

#[tokio::test]
async fn stream_from_sequence() {
    let pool = test_db_pool().await;
    let data: Vec<_> = (0_i64..20).map(|sequence| heartbeat(sequence)).collect();
    let persistence = SqlitePublisherPersistence::new(pool);
    for msg in data.iter() {
        persistence
            .write_message(
                msg.metadata.sequence,
                msg.metadata.timestamp,
                Message::encode_to_vec(msg).as_slice(),
            )
            .await
            .unwrap();
    }
    let offset = 10;
    let mut stream = persistence.stream_from_sequence(offset).await.unwrap();
    let mut records = 0;
    while let Some(Ok(msg)) = stream.next().await {
        let msg: RawMessage = Message::decode(msg.as_slice()).unwrap();
        assert_eq!(data[msg.metadata.sequence as usize], msg);
        records += 1;
    }
    assert_eq!(records, 10);
}
