use std::{
    net::{Ipv4Addr, SocketAddrV4},
    str::FromStr,
    sync::Arc,
};

use ::grapevine::*;
use examples::order_book::Level1OrderBook;
use grapevine::{
    proto::recovery::recovery_api_client::RecoveryApiClient, sqlite::SqliteSubscriberPersistence,
    transport::new_multicast_socket,
};
use sqlx::{
    sqlite::{SqliteConnectOptions, SqliteJournalMode, SqliteSynchronous},
    SqlitePool,
};
use tokio_stream::StreamExt;
use tonic::transport::Channel;
use tracing::info;

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt::init();

    // TODO: this is aligned to publisher. don't do that
    let recovery_addr = "http://127.0.0.1:7755".parse().unwrap();
    // the pre-configured multicast address
    let mc_addr = SocketAddrV4::new(Ipv4Addr::new(224, 0, 0, 123), 1234);

    let socket = new_multicast_socket(&mc_addr).unwrap();

    let database_url =
        std::env::var("DATABASE_URL").unwrap_or_else(|_| "sqlite::memory:".to_string());

    // TODO: probably don't need a pool
    let pool = SqlitePool::connect_with(
        SqliteConnectOptions::from_str(database_url.as_str())
            .unwrap()
            .journal_mode(SqliteJournalMode::Wal)
            .synchronous(SqliteSynchronous::Off),
    )
    .await
    .unwrap();

    sqlx::migrate!("../grapevine/migrations")
        .run(&pool)
        .await
        .unwrap();

    let persistence = SqliteSubscriberPersistence::new(pool);

    let channel = Channel::builder(recovery_addr).connect_lazy();
    // TODO: create a subscriber
    // let subscriber = SubscriberStream::<Level1OrderBook, _>::new(
    //     Default::default(),
    //     std::time::Duration::from_secs(5),
    //     std::time::Duration::from_millis(20),
    //     5,
    //     Arc::new(socket),
    //     persistence,
    //     RecoveryApiClient::new(channel),
    // );
    //
    // let mut stream = subscriber.stream();
    //
    // while let Some(Some(Ok((state, _, sequence)))) = stream.next().await {
    //     if sequence % 10000 == 0 {
    //         let lock = state.read().await;
    //         info!("Sequence {} Order book: {}", sequence, lock.state());
    //     }
    // }
}
