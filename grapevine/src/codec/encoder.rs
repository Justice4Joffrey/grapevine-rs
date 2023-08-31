use std::marker::PhantomData;

use bytes::{BufMut, BytesMut};
use prost::Message;
use tokio_util::codec;
use tonic::Status;

use crate::constants::{HEADER_SIZE, MAX_PAYLOAD_SIZE};

#[derive(Debug, Clone, Default)]
pub struct Encoder<U> {
    _phantom: PhantomData<U>,
}

impl<U: Message> Encoder<U> {
    fn encode_message(item: U, buf: &mut BytesMut) -> Result<(), Status> {
        item.encode(buf)
            .map_err(|e| Status::internal(e.to_string()))
    }
}

impl<U: Message> codec::Encoder<U> for Encoder<U> {
    type Error = Status;

    fn encode(&mut self, item: U, dst: &mut BytesMut) -> Result<(), Self::Error> {
        dst.reserve(HEADER_SIZE);
        unsafe {
            dst.advance_mut(HEADER_SIZE);
        }
        Self::encode_message(item, dst)
            .map_err(|err| Status::internal(format!("Error encoding: {}", err)))?;
        let len = dst.len() - HEADER_SIZE;
        if len > MAX_PAYLOAD_SIZE {
            return Err(Status::internal(format!("Message too large: {}", len)));
        }
        // now that we know length, we can write the header
        {
            let mut buf = &mut dst[..HEADER_SIZE];
            // forward-compatible compression flag
            buf.put_u8(0);
            buf.put_u16(len as u16);
        }
        Ok(())
    }
}
