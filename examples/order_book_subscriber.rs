use std::{
    net::{Ipv4Addr, SocketAddrV4},
    str::FromStr,
    sync::Arc,
};

use ::grapevine::*;
use examples::order_book::Level1OrderBook;
use futures::pin_mut;
use grapevine::{
    proto::recovery::recovery_api_client::RecoveryApiClient, sqlite::SqliteSubscriberPersistence,
    transport::new_multicast_socket,
};
use sqlx::{
    sqlite::{SqliteConnectOptions, SqliteJournalMode, SqliteSynchronous},
    SqlitePool,
};
use tokio_stream::StreamExt;
use tokio_util::udp::UdpFramed;
use tonic::transport::Channel;
use tracing::info;

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt::init();

    // TODO: this is aligned to publisher. don't do that
    let recovery_addr = SocketAddrV4::new(Ipv4Addr::new(127, 0, 0, 1), 7755).to_string();
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

    let channel = Channel::builder(recovery_addr.parse().unwrap()).connect_lazy();

    let mut sequencer = MessageSequencer::<Level1OrderBook>::new(
        Arc::new(socket),
        RecoveryApiClient::new(channel),
        3,
        0,
        std::time::Duration::from_secs(5),
    );
    let stream = sequencer.into_stream();
    pin_mut!(stream);

    while let Some(message) = stream.next().await {
        let message = message.unwrap();
        info!("Message: {:?}", message);
    }
}
