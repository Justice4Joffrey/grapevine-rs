use std::sync::mpsc::{Receiver, TryRecvError};

use tokio::task::{spawn_blocking, JoinError};
use tracing::{error, warn};

/// Check whether a message has been sent on the destructor channel
///
/// We use a regular mpsc channel here as we never make blocking calls, and
/// the main use case of this is to implement [Drop].
pub fn should_destruct(rx: &mut Receiver<()>) -> bool {
    match rx.try_recv() {
        Err(TryRecvError::Empty) => false,
        Ok(()) => {
            warn!("Received destruct signal");
            true
        }
        Err(e) => {
            error!("Failed to receive destruct signal: {}", e);
            true
        }
    }
}

/// Turn a sync [Receiver] into an async [Receiver]. We need to call this
/// from [Drop], so we either need to call async from sync or vice-versa.
pub async fn destruct_future(rx: Receiver<()>) -> Result<(), JoinError> {
    spawn_blocking(move || {
        rx.recv().expect("Failed to receive destruct signal");
    })
    .await
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc;

    use super::*;

    #[test]
    fn test_should_destruct() {
        let (_tx, mut rx) = mpsc::channel();
        assert!(!should_destruct(&mut rx));
    }

    #[test]
    fn test_should_destruct_signal() {
        let (tx, mut rx) = mpsc::channel();
        tx.send(()).unwrap();
        assert!(should_destruct(&mut rx));
    }

    #[test]
    fn test_should_destruct_error() {
        // tx is dropped here
        let (_, mut rx) = mpsc::channel();
        assert!(should_destruct(&mut rx));
    }
}
