use std::mem::size_of;

/// Maximum Transmission Unit.
const MTU: usize = 1500;
/// Header size in contiguous bytes.
pub const HEADER_SIZE: usize =
    // compression flag
    size_of::<u8>() +
        // data length
        size_of::<u16>();

/// Random internet googling suggests sailing close to the wind is not a good
/// idea. Allow some wiggle room (headers are not included in this capacity)
pub const MAX_PAYLOAD_SIZE: usize = MTU - HEADER_SIZE - 100;
