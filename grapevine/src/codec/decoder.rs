use std::marker::PhantomData;

use bytes::{Buf, BytesMut};
use prost::{DecodeError, Message};
use tokio_util::codec;
use tonic::{Code, Status};

use crate::constants::HEADER_SIZE;

#[derive(Debug, Copy, Clone, Eq, PartialEq)]
enum State {
    ReadHeader,
    ReadBody { len: usize },
}

pub struct Decoder<U> {
    _phantom: PhantomData<U>,
    state: State,
}

impl<U: Message + Default> Default for Decoder<U> {
    fn default() -> Self {
        Self::new()
    }
}

impl<U: Message + Default> Decoder<U> {
    pub fn new() -> Self {
        Self {
            _phantom: PhantomData::default(),
            state: State::ReadHeader,
        }
    }

    fn decode_payload(buf: &mut BytesMut) -> Result<Option<U>, Status> {
        let item = Message::decode(buf).map(Some).map_err(from_decode_error)?;
        Ok(item)
    }
}

fn from_decode_error(error: DecodeError) -> Status {
    // Map Protobuf parse errors to an INTERNAL status code, as per
    // https://github.com/grpc/grpc/blob/master/doc/statuscodes.md
    Status::new(Code::Internal, error.to_string())
}

impl<U: Message + Default> codec::Decoder for Decoder<U> {
    type Error = Status;
    type Item = U;

    fn decode(&mut self, src: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
        if let State::ReadHeader = self.state {
            if src.remaining() < HEADER_SIZE {
                return Ok(None);
            }
            // don't support compression, but keep the flag in place
            match src.get_u8() {
                0 => {}
                v => {
                    return Err(Status::new(
                        Code::Internal,
                        format!("Compression not supported: found {}", v),
                    ))
                }
            };
            let len = src.get_u16() as usize;
            src.reserve(len);
            self.state = State::ReadBody { len };
        }

        if let State::ReadBody { len } = self.state {
            if src.remaining() < len || src.len() < len {
                Ok(None)
            } else {
                Self::decode_payload(src).map(|msg| {
                    self.state = State::ReadHeader;
                    msg
                })
            }
        } else {
            Ok(None)
        }
    }
}
