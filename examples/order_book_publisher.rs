use std::{
    net::{Ipv4Addr, SocketAddrV4},
    str::FromStr,
    time::Duration,
};

use ::grapevine::*;
use examples::order_book::Level1OrderBook;
use grapevine::{sqlite::SqlitePublisherPersistence, transport::new_multicast_publisher};
use sqlx::{
    sqlite::{SqliteConnectOptions, SqliteJournalMode, SqliteSynchronous},
    SqlitePool,
};
use tokio::time::sleep;
use tracing::info;

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt::init();

    let recovery_addr = SocketAddrV4::new(Ipv4Addr::new(0, 0, 0, 0), 7755);
    // the pre-configured multicast address
    let mc_addr = SocketAddrV4::new(Ipv4Addr::new(224, 0, 0, 123), 1234);

    let socket = new_multicast_publisher(&mc_addr).await.unwrap();

    let database_url =
        std::env::var("DATABASE_URL").unwrap_or_else(|_| "sqlite::memory:".to_string());

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

    let persistence = SqlitePublisherPersistence::new(pool);

    let publisher = Publisher::<Level1OrderBook, _>::new(socket, persistence);

    let mut publisher = publisher.run(recovery_addr.into()).await.unwrap();

    loop {
        let delta = {
            let lock = publisher.read().await;
            lock.state().new_random_delta()
        };

        let sequence = publisher.apply_delta(delta).await.unwrap();

        if sequence % 10000 == 0 {
            let lock = publisher.read().await;
            info!("Sequence {} Order book: {}", sequence, lock.state());
        }

        // Minimum sleep is actually considerably longer
        sleep(Duration::from_nanos(1)).await;
    }
}
