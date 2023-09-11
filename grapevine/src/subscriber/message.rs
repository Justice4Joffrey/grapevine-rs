use chrono::Utc;

use crate::proto::grapevine::RawMessage;

#[derive(Debug)]
pub enum SubscriberStreamMessage<T> {
    Delta(T),
    Started,
    Heartbeat,
}

#[derive(Debug, PartialEq, Eq, Clone)]
pub enum Origin {
    /// Batch sync
    Sync,
    /// Packet from stream
    Udp,
    /// Single missing udp packet recovered over gRPC
    Missing,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReceivedMessage {
    /// according to our process (in nano-seconds)
    pub recv_ts: i64,
    pub origin: Origin,
    pub raw: RawMessage,
}

impl ReceivedMessage {
    pub fn new(raw: RawMessage, origin: Origin) -> Self {
        ReceivedMessage {
            recv_ts: Utc::now().timestamp_nanos(),
            origin,
            raw,
        }
    }
}

impl PartialOrd for ReceivedMessage {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for ReceivedMessage {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.raw.cmp(&other.raw)
    }
}


#[cfg(test)]
pub mod test_utils {
    use bytes::{BytesMut, Bytes};
    use chrono::Utc;
    use tokio_util::codec::Encoder;
    use prost::Message;

    use crate::proto::grapevine::{RawMessage, Delta, raw_message::Payload};

    pub fn make_raw_msg(i: i64) -> anyhow::Result<RawMessage> {
        let mut msg = RawMessage::default();
        msg.metadata.sequence = i;
        msg.metadata.timestamp = Utc::now().timestamp_nanos();

        let mut buf = BytesMut::new();
        <i64>::encode(&msg.metadata.sequence, &mut buf)?;
        msg.payload = Some(Payload::Delta(Delta { body: buf.to_vec() }));

        Ok(msg)
    }

    pub fn make_raw_msg_bytes(i: i64) -> anyhow::Result<Bytes> {
        let msg = make_raw_msg(i)?;

        let mut encoder = crate::codec::Encoder::<RawMessage>::default();
        let mut buffer = BytesMut::new();
        encoder.encode(msg, &mut buffer)?;
        Ok(buffer.freeze())
    }
}
