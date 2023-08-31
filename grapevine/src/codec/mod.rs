//! The codec is pretty much a like for like copy of how Tonic implements
//! streaming, but without compression.
//!
//! Every message is sent with a future compatible compression flag (always
//! zero for now) and a body size (n). The next n bytes will then be
//! deserializeable in to a
//! [RawMessage](crate::proto::grapevine::RawMessage), which is an envelope
//! for a [Delta](crate::StateSync::Delta).

mod decoder;
mod encoder;

pub use decoder::Decoder;
pub use encoder::Encoder;
