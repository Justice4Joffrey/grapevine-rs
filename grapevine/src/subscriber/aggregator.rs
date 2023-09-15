use std::sync::Arc;
use futures::StreamExt;
use tokio::sync::{mpsc, RwLockWriteGuard, watch};
use tokio::sync::mpsc::error::TryRecvError;
use tracing::{debug, error};

use crate::{MessageSequencer, StateSync};
use crate::subscriber::message::SubscriberStreamMessage;
use crate::subscriber::sequencer::{SubscriberStream};

fn get_delta<T>(data: SubscriberStream<T>) -> Option<T> {
    if let SubscriberStream::Message { msg: SubscriberStreamMessage::Delta(delta), .. } = data {
        Some(delta)
    } else {
        None
    }
}

// aggregator and it's caller share same memory
type State<S> = Arc<tokio::sync::RwLock<S>>;

pub struct Aggregator<S: StateSync> {
    state_rx: watch::Receiver<State<S>>,
}

impl<S: StateSync> Aggregator<S> {
    pub fn new(
        sequencer: MessageSequencer<S>,
    ) -> Self {
        let state = State::default();
        let (state_tx, state_rx) = watch::channel(state.clone());
        let (sequencer_tx, mut sequencer_rx) = mpsc::unbounded_channel();

        // convert `Stream` to `unbounded_channel` because `Stream` doesn't have `try_recv`
        tokio::spawn(async move {
            let sequencer = sequencer.into_stream();
            futures::pin_mut!(sequencer);
            while let Some(result) = sequencer.next().await {
                match result {
                    Ok(data) => {
                        // todo: handle non-deltas
                        if let Some(delta) = get_delta(data) {
                            if sequencer_tx.send(delta).is_err() {
                                debug!("sequencer_rx was closed");
                                break;
                            }
                        }
                    },
                    Err(err) => {
                        error!("sequencer error {:?}", err);
                        break;
                    }
                }
            }
        });

        // snapshot producer
        tokio::spawn(async move {
            // drain channel with busy loop then wait next messages with `.await`
            let mut empty = true;
            loop {
                let received = if empty {
                    empty = false;
                    sequencer_rx.recv().await.ok_or(TryRecvError::Disconnected)
                } else {
                    sequencer_rx.try_recv()
                };

                match received {
                    Ok(delta) => {
                        let mut state: RwLockWriteGuard<S> = state.write().await;
                        if let Err(err) = state.apply_delta(delta) {
                            error!("delta apply error: {:?}", err);
                            break;
                        }
                    },
                    Err(TryRecvError::Empty) => {
                        if state_tx.send(state.clone()).is_err() {
                            debug!("state_rx was closed");
                            break;
                        }
                        empty = true;
                    },
                    Err(TryRecvError::Disconnected) => {
                        break;
                    },
                }
            }
        });

        Self {
            state_rx,
        }
    }

    /// subscribes to recent changes of state
    pub fn subscribe(&self) -> watch::Receiver<State<S>> {
        self.state_rx.clone()
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;
    use async_stream::stream;
    use tokio::net::UdpSocket;
    use tokio_stream::wrappers::WatchStream;
    use futures::Stream;
    use tokio::time::timeout;
    use tokio_stream::StreamExt;
    use crate::subscriber::message::test_utils::make_raw_msg_bytes;
    use crate::subscriber::sequencer::tests::{TestStateSync, make_default_sequencer};

    use super::*;

    fn prefix_sum(arr: Vec<i64>) -> Vec<i64> {
        let mut ps = Vec::with_capacity(arr.len());
        for v in arr {
            if let Some(last) = ps.last() {
                ps.push(last + v);
            } else {
                ps.push(v);
            }
        }
        ps
    }

    async fn publish_chunks(sizes: Vec<i64>, publisher: UdpSocket) {
        let wait_between_chunks = Duration::from_millis(10);

        tokio::spawn(async move {
            let stream = stream! {
                let mut seq_id = 0;
                for size in sizes {
                    for _ in 0..size {
                        yield seq_id;
                        seq_id += 1;
                    }
                    tokio::time::sleep(wait_between_chunks).await;
                }
            };
            publish_to_socket(stream, publisher).await?;

            Ok::<_, anyhow::Error>(())
        });
    }

    async fn publish_to_socket(
        stream: impl Stream<Item = i64>,
        publisher: UdpSocket,
    ) -> anyhow::Result<()> {
        let mut stream = Box::pin(stream);
        while let Some(i) = stream.next().await {
            publisher.send(make_raw_msg_bytes(i)?.as_ref()).await?;
        }

        Ok(())
    }

    async fn collect_result(
        aggregator: Aggregator<TestStateSync>,
        expected_len: usize
    ) -> anyhow::Result<Vec<i64>> {
        let result_stream = WatchStream::from_changes(aggregator.subscribe())
            .take(expected_len)
            .then(|v| async move { v.read().await.sum })
            .collect::<Vec<_>>();

        let result = timeout(Duration::from_secs(5), result_stream).await?;

        Ok(result)
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn chunks_are_valid() -> anyhow::Result<()> {
        // too big chunks can be divided (especially in multi-threaded runtime) which is decreasing test reproducibility
        let sizes = vec![3, 5, 10, 3, 2, 2, 1, 1, 1];

        let (sequencer, publisher) = make_default_sequencer().await?;
        let aggregator = Aggregator::<TestStateSync>::new(sequencer);

        publish_chunks(sizes.clone(), publisher).await;

        let result = collect_result(aggregator, sizes.len()).await?;
        assert_eq!(result, prefix_sum(sizes));

        Ok(())
    }

    // busy loop will be infinite in single-threaded runtime so non-blocking implementation should pass test
    #[tokio::test]
    async fn is_not_busy_loop() -> anyhow::Result<()> {
        let sizes = vec![1000, 500, 200, 100, 100, 100, 50, 25, 10, 5, 1, 1, 1];

        let (sequencer, publisher) = make_default_sequencer().await?;
        let aggregator = Aggregator::<TestStateSync>::new(sequencer);

        publish_chunks(sizes.clone(), publisher).await;

        let result = collect_result(aggregator, sizes.len()).await?;
        assert_eq!(result, prefix_sum(sizes));

        Ok(())
    }
}
