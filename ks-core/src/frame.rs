use super::{MessageType, ProtocolError, HEADER_SIZE, MAGIC, MAX_FRAME_SIZE};

/// Borrowed frame view. The payload remains in the caller-owned transport buffer.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Frame<'a> {
    pub message_type: MessageType,
    pub payload: &'a [u8],
}

impl<'a> Frame<'a> {
    pub fn parse(input: &'a [u8]) -> Result<Self, ProtocolError> {
        if input.len() < HEADER_SIZE {
            return Err(ProtocolError::BufferTooSmall);
        }
        if u32::from_le_bytes(input[..4].try_into().unwrap()) != MAGIC {
            return Err(ProtocolError::InvalidMagic);
        }
        let ty = MessageType::from_u16(u16::from_le_bytes(input[4..6].try_into().unwrap()))?;
        let length = u32::from_le_bytes(input[6..10].try_into().unwrap()) as usize;
        if length > MAX_FRAME_SIZE - HEADER_SIZE {
            return Err(ProtocolError::TooLarge);
        }
        let end = HEADER_SIZE
            .checked_add(length)
            .ok_or(ProtocolError::InvalidLength)?;
        if input.len() != end {
            return Err(if input.len() < end {
                ProtocolError::BufferTooSmall
            } else {
                ProtocolError::InvalidLength
            });
        }
        Ok(Self {
            message_type: ty,
            payload: &input[HEADER_SIZE..end],
        })
    }
}

#[cfg(feature = "alloc")]
pub struct FrameDecoder {
    buffer: alloc::vec::Vec<u8>,
    max_frame_size: usize,
}

#[cfg(feature = "alloc")]
impl FrameDecoder {
    pub fn new() -> Self {
        Self::with_capacity(MAX_FRAME_SIZE)
    }

    pub fn with_capacity(max_frame_size: usize) -> Self {
        Self {
            buffer: alloc::vec::Vec::new(),
            max_frame_size: core::cmp::max(
                HEADER_SIZE,
                core::cmp::min(max_frame_size, MAX_FRAME_SIZE),
            ),
        }
    }

    pub fn push(&mut self, bytes: &[u8]) -> Result<(), ProtocolError> {
        if self
            .buffer
            .len()
            .checked_add(bytes.len())
            .ok_or(ProtocolError::TooLarge)?
            > self.max_frame_size
        {
            return Err(ProtocolError::TooLarge);
        }
        self.buffer.extend_from_slice(bytes);
        Ok(())
    }

    pub fn next(&mut self) -> Result<Option<Frame<'_>>, ProtocolError> {
        if self.buffer.len() < HEADER_SIZE {
            return Ok(None);
        }
        if u32::from_le_bytes(self.buffer[..4].try_into().unwrap()) != MAGIC {
            self.buffer.clear();
            return Err(ProtocolError::InvalidMagic);
        }
        let length = u32::from_le_bytes(self.buffer[6..10].try_into().unwrap()) as usize;
        let total = HEADER_SIZE
            .checked_add(length)
            .ok_or(ProtocolError::InvalidLength)?;
        if length > self.max_frame_size - HEADER_SIZE {
            self.buffer.clear();
            return Err(ProtocolError::TooLarge);
        }
        if self.buffer.len() < total {
            return Ok(None);
        }
        let ty = MessageType::from_u16(u16::from_le_bytes(self.buffer[4..6].try_into().unwrap()))?;
        Ok(Some(Frame {
            message_type: ty,
            payload: &self.buffer[HEADER_SIZE..total],
        }))
    }

    pub fn consume(&mut self) -> Result<(), ProtocolError> {
        if self.buffer.len() < HEADER_SIZE {
            return Err(ProtocolError::BufferTooSmall);
        }
        let length = u32::from_le_bytes(self.buffer[6..10].try_into().unwrap()) as usize;
        let total = HEADER_SIZE
            .checked_add(length)
            .ok_or(ProtocolError::InvalidLength)?;
        if self.buffer.len() < total {
            return Err(ProtocolError::BufferTooSmall);
        }
        self.buffer.drain(..total);
        Ok(())
    }

    pub fn buffered_len(&self) -> usize {
        self.buffer.len()
    }
}
