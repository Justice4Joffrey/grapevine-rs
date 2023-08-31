//! Utilities to create multicast-ready sockets for both publishing and
//! subscribing to data.

mod multicast;

pub use multicast::{
    new_multicast_publisher, new_multicast_socket, MulticastSocketError, MulticastSocketResult,
};
