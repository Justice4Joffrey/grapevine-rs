use std::{
    error::Error,
    fmt::Display,
    io,
    net::{Ipv4Addr, SocketAddrV4},
};

use socket2::{Domain, Protocol, Socket, Type};
use thiserror::Error;
use tokio::net::UdpSocket;

/// Convenience result type.
pub type MulticastSocketResult<T> = Result<T, MulticastSocketError>;
/// Error arising from binding to a multicast address.
#[derive(Debug, Error)]
pub enum MulticastSocketError {
    #[error("not a multicast address: {0}")]
    NotMulticast(Ipv4Addr),
    #[error("io error")]
    Io(#[from] io::Error),
}

/// Connect a local socket to a multicast address with IP_MULTICAST_LOOP and
/// SO_REUSEADDR enabled. This can be used to listen to a multicast address.
pub fn new_multicast_socket(mc_addr: &SocketAddrV4) -> MulticastSocketResult<UdpSocket> {
    if !mc_addr.ip().is_multicast() {
        return Err(MulticastSocketError::NotMulticast(mc_addr.ip().to_owned()));
    }
    let addr = SocketAddrV4::new(Ipv4Addr::new(0, 0, 0, 0), 0);
    let socket = Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::UDP))?;
    socket.set_nonblocking(true)?;
    socket.set_reuse_address(true)?;
    socket.set_multicast_loop_v4(true)?;
    socket.join_multicast_v4(mc_addr.ip(), addr.ip())?;
    socket.bind(&socket2::SockAddr::from(*mc_addr))?;
    let std_udp_socket: std::net::UdpSocket = socket.into();
    Ok(UdpSocket::from_std(std_udp_socket)?)
}

/// Create a new socket on a random port which can publish to a multicast
/// address.
pub async fn new_multicast_publisher(mc_addr: &SocketAddrV4) -> MulticastSocketResult<UdpSocket> {
    let socket = new_multicast_socket(mc_addr)?;
    socket.connect(mc_addr).await?;
    Ok(socket)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_publish_subscribe() {
        let mc_addr = SocketAddrV4::new(Ipv4Addr::new(239, 255, 255, 250), 4534);
        let publisher = new_multicast_publisher(&mc_addr).await.unwrap();
        let subscriber = new_multicast_socket(&mc_addr).unwrap();
        let message = vec![1, 2, 3, 4, 5];
        publisher.send(message.as_slice()).await.unwrap();
        let mut buf = [0u8; 5];
        subscriber.recv(&mut buf).await.unwrap();
        let res = buf.to_vec();
        assert_eq!(res, message);
    }

    #[tokio::test]
    async fn test_not_multicast_socket() {
        let mc_addr = SocketAddrV4::new(Ipv4Addr::new(8, 8, 8, 8), 80);
        let res = new_multicast_publisher(&mc_addr).await;
        assert!(res.is_err());
    }
}
