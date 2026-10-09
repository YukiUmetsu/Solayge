//! Length-bounded framing over a QUIC stream.
//!
//! Every frame is written as a 4-byte big-endian length followed by the JSON
//! envelope. The reader checks the declared length against
//! [`solayge_protocol::MAX_FRAME_BYTES`] *before* allocating, so a malicious or
//! buggy peer cannot make us reserve an unbounded buffer. The same bound is
//! enforced on write, so an oversized frame fails locally instead of on the
//! wire.

use iroh::endpoint::{RecvStream, SendStream};
use solayge_protocol::MAX_FRAME_BYTES;

use crate::error::NetworkError;

/// Bytes used for the big-endian frame length prefix.
pub const LENGTH_PREFIX_BYTES: usize = 4;

/// Write one length-prefixed frame and finish the send half.
pub async fn write_frame(send: &mut SendStream, frame: &[u8]) -> Result<(), NetworkError> {
    if frame.len() > MAX_FRAME_BYTES {
        return Err(NetworkError::FrameTooLarge {
            size: frame.len(),
            max: MAX_FRAME_BYTES,
        });
    }
    let length = (frame.len() as u32).to_be_bytes();
    send.write_all(&length)
        .await
        .map_err(|e| NetworkError::connectivity(format!("write frame length failed: {e}")))?;
    send.write_all(frame)
        .await
        .map_err(|e| NetworkError::connectivity(format!("write frame body failed: {e}")))?;
    send.finish()
        .map_err(|e| NetworkError::connectivity(format!("finish frame failed: {e}")))?;
    Ok(())
}

/// Read one length-prefixed frame, enforcing the frame bound before allocating.
pub async fn read_frame(recv: &mut RecvStream) -> Result<Vec<u8>, NetworkError> {
    let mut length = [0u8; LENGTH_PREFIX_BYTES];
    recv.read_exact(&mut length)
        .await
        .map_err(|e| NetworkError::connectivity(format!("read frame length failed: {e}")))?;
    let size = u32::from_be_bytes(length) as usize;
    if size > MAX_FRAME_BYTES {
        return Err(NetworkError::FrameTooLarge {
            size,
            max: MAX_FRAME_BYTES,
        });
    }
    let mut buffer = vec![0u8; size];
    recv.read_exact(&mut buffer)
        .await
        .map_err(|e| NetworkError::connectivity(format!("read frame body failed: {e}")))?;
    Ok(buffer)
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_length_prefix_is_stable() {
        assert_eq!(super::LENGTH_PREFIX_BYTES, 4);
        assert_eq!(u32::from_be_bytes((7u32).to_be_bytes()), 7);
    }
}
