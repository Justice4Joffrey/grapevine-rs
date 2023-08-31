/// / The publisher has checked in.
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct Heartbeat {
}
/// / The publisher has started publishing.
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct Started {
}
/// / An incremental update of custom state.
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct Delta {
    /// / The custom delta as bytes
    #[prost(bytes="vec", required, tag="1")]
    pub body: ::prost::alloc::vec::Vec<u8>,
}
/// / Metadata for the message payload.
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct Metadata {
    /// / The creation timestamp of this in nano-seconds.
    #[prost(int64, required, tag="1")]
    pub timestamp: i64,
    /// / A unique monotonically-increasing sequence ID for this message.
    #[prost(int64, required, tag="2")]
    pub sequence: i64,
}
/// / A self contained message which must be sent as a single frame (i.e. no snapshots, over UDP).
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct RawMessage {
    /// / The header of the message.
    #[prost(message, required, tag="1")]
    pub metadata: Metadata,
    /// / The payload. Signals what type of message this is and provides custom data.
    #[prost(oneof="raw_message::Payload", tags="2, 3, 4")]
    pub payload: ::core::option::Option<raw_message::Payload>,
}
/// Nested message and enum types in `RawMessage`.
pub mod raw_message {
    /// / The payload. Signals what type of message this is and provides custom data.
    #[derive(Clone, PartialEq, ::prost::Oneof)]
    pub enum Payload {
        #[prost(message, tag="2")]
        Heartbeat(super::Heartbeat),
        #[prost(message, tag="3")]
        Started(super::Started),
        #[prost(message, tag="4")]
        Delta(super::Delta),
    }
}
