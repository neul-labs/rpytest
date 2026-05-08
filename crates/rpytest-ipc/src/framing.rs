//! MessagePack encoding and length-prefixed framing.

use serde::{de::DeserializeOwned, Serialize};
use thiserror::Error;

/// Errors that can occur during framing operations.
#[derive(Debug, Error)]
pub enum FramingError {
    #[error("Serialization error: {0}")]
    Serialize(#[from] rmp_serde::encode::Error),

    #[error("Deserialization error: {0}")]
    Deserialize(#[from] rmp_serde::decode::Error),

    #[error("Message too large: {0} bytes (max: {1})")]
    MessageTooLarge(usize, usize),

    #[error("Invalid frame: {0}")]
    InvalidFrame(String),
}

/// Maximum message size (16 MB).
pub const MAX_MESSAGE_SIZE: usize = 16 * 1024 * 1024;

/// Encode a message to MessagePack bytes with length prefix.
///
/// Format: [4-byte little-endian length][MessagePack payload]
pub fn encode<T: Serialize>(msg: &T) -> Result<Vec<u8>, FramingError> {
    let payload = rmp_serde::to_vec_named(msg)?;

    if payload.len() > MAX_MESSAGE_SIZE {
        return Err(FramingError::MessageTooLarge(
            payload.len(),
            MAX_MESSAGE_SIZE,
        ));
    }

    let len = payload.len() as u32;
    let mut frame = Vec::with_capacity(4 + payload.len());
    frame.extend_from_slice(&len.to_le_bytes());
    frame.extend(payload);

    Ok(frame)
}

/// Encode a message to MessagePack bytes without length prefix.
pub fn encode_payload<T: Serialize>(msg: &T) -> Result<Vec<u8>, FramingError> {
    let payload = rmp_serde::to_vec_named(msg)?;

    if payload.len() > MAX_MESSAGE_SIZE {
        return Err(FramingError::MessageTooLarge(
            payload.len(),
            MAX_MESSAGE_SIZE,
        ));
    }

    Ok(payload)
}

/// Decode a MessagePack payload (without length prefix).
pub fn decode<T: DeserializeOwned>(data: &[u8]) -> Result<T, FramingError> {
    Ok(rmp_serde::from_slice(data)?)
}

/// Parse a length-prefixed frame and return (length, payload_start_offset).
pub fn parse_frame_header(data: &[u8]) -> Result<(usize, usize), FramingError> {
    if data.len() < 4 {
        return Err(FramingError::InvalidFrame(
            "Frame too short for header".to_string(),
        ));
    }

    let len = u32::from_le_bytes([data[0], data[1], data[2], data[3]]) as usize;

    if len > MAX_MESSAGE_SIZE {
        return Err(FramingError::MessageTooLarge(len, MAX_MESSAGE_SIZE));
    }

    Ok((len, 4))
}

#[cfg(test)]
mod tests {
    use super::*;
    use rpytest_core::protocol::{Request, Response, PROTOCOL_VERSION};

    #[test]
    fn encode_decode_roundtrip() {
        let request = Request::InitContext {
            protocol_version: PROTOCOL_VERSION,
            repo_path: "/path/to/repo".to_string(),
            python_path: Some("/usr/bin/python3".to_string()),
            execution_mode: None,
        };

        let encoded = encode(&request).unwrap();

        // Check length prefix
        let (len, offset) = parse_frame_header(&encoded).unwrap();
        assert_eq!(offset, 4);
        assert_eq!(len, encoded.len() - 4);

        // Decode payload
        let decoded: Request = decode(&encoded[offset..]).unwrap();
        assert_eq!(request, decoded);
    }

    #[test]
    fn encode_decode_response() {
        let response = Response::ContextReady {
            protocol_version: PROTOCOL_VERSION,
            context_id: "ctx-123".to_string(),
            inventory_hash: "abc123".to_string(),
        };

        let payload = encode_payload(&response).unwrap();
        let decoded: Response = decode(&payload).unwrap();
        assert_eq!(response, decoded);
    }

    #[test]
    fn rejects_oversized_message() {
        let large_data = vec![0u8; MAX_MESSAGE_SIZE + 1];

        // This should fail because the payload is too large
        let result = encode_payload(&large_data);
        assert!(matches!(result, Err(FramingError::MessageTooLarge(_, _))));
    }

    #[test]
    fn parse_short_header_fails() {
        let short_data = vec![0u8, 1u8, 2u8]; // Only 3 bytes
        let result = parse_frame_header(&short_data);
        assert!(matches!(result, Err(FramingError::InvalidFrame(_))));
    }
}
