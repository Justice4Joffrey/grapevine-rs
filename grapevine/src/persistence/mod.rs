//! # Functionality to store and retrieve RawMessages
//!
//! Persistence logs are a key concept in Grapevine's architecture.
//!
//! ## Master logs
//!
//! Every [Publisher](crate::Publisher) must record each message it produces
//! so that there is a 'source of truth' to compare downstream sequences to.
//!
//! ## Replica logs
//!
//! Whether a replica log (a subscriber's copy of the master log) is required
//! depends on the subscriber strategy employed.
//!
//! ### Exactly Once Processing
//!
//! For [Subscribers][crate::SubscriberStream] wishing to process each and
//! every message a publisher creates exactly once, storing a replica log is
//! essential. This allows the subscriber to recover any state it previously
//! had built, and know from exactly which sequence it needs to attempt to
//! recover.
//!
//! As an example, an trade-reporting service responsible for
//! tracking all trades from an exchange platform may require processing
//! every trade exactly once.
//!
//! ### Eventual consistency
//!
//! There are many situations where processing every message once is
//! overkill. If 'eventual consistency' (ignoring missed messages in favour
//! of minimizing time to synchronize state) is preferable, no replica log
//! need be created. This is especially common when dealing with large
//! amounts of data. In this case, current deltas can be layered upon
//! snapshots (WIP), allowing subscribers to synchronize with publishers in
//! as little time possible.
//!
//! For example, an order book viewing tool likely wants to show the most
//! accurate up-to-date view of current orders on an exchange possible.
//! Consuming and persisting any orders generated whilst the tool wasn't
//! listening is not necessary.

#[cfg(feature = "sqlite")]
pub mod sqlite;

#[cfg(feature = "mocks")]
pub mod mock;

mod publisher_persistence;
mod subscriber_persistence;

pub use publisher_persistence::PublisherPersistence;
pub use subscriber_persistence::SubscriberPersistence;
