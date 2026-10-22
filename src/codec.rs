//! Small, dependency-free binary codec for prototype checkpoint files.
//! The CRC32 checksum detects accidental damage; it is not authentication.

use core::fmt;

pub(crate) const MAX_CHECKPOINT_BYTES: usize = 64 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CodecError {
    Truncated,
    TooLarge,
    BadChecksum,
    TrailingBytes,
    NonCanonical,
    Invalid(&'static str),
}

impl fmt::Display for CodecError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{self:?}")
    }
}

pub(crate) struct Cursor<'a> {
    data: &'a [u8],
    offset: usize,
}

impl<'a> Cursor<'a> {
    pub(crate) fn new(data: &'a [u8]) -> Self {
        Self { data, offset: 0 }
    }

    pub(crate) fn take(&mut self, length: usize) -> Result<&'a [u8], CodecError> {
        let end = self.offset.checked_add(length).ok_or(CodecError::TooLarge)?;
        let bytes = self.data.get(self.offset..end).ok_or(CodecError::Truncated)?;
        self.offset = end;
        Ok(bytes)
    }

    pub(crate) fn fixed<const N: usize>(&mut self) -> Result<[u8; N], CodecError> {
        self.take(N)?.try_into().map_err(|_| CodecError::Truncated)
    }

    pub(crate) fn u8(&mut self) -> Result<u8, CodecError> {
        Ok(self.fixed::<1>()?[0])
    }

    pub(crate) fn u32(&mut self) -> Result<u32, CodecError> {
        Ok(u32::from_be_bytes(self.fixed::<4>()?))
    }

    pub(crate) fn u64(&mut self) -> Result<u64, CodecError> {
        Ok(u64::from_be_bytes(self.fixed::<8>()?))
    }

    pub(crate) fn u128(&mut self) -> Result<u128, CodecError> {
        Ok(u128::from_be_bytes(self.fixed::<16>()?))
    }

    pub(crate) fn end(&self) -> Result<(), CodecError> {
        if self.offset == self.data.len() {
            Ok(())
        } else {
            Err(CodecError::TrailingBytes)
        }
    }
}

pub(crate) fn crc32(data: &[u8]) -> u32 {
    let mut crc: u32 = !0;
    for &byte in data {
        crc ^= u32::from(byte);
        for _ in 0..8 {
            crc = (crc >> 1) ^ (0xedb8_8320 & (0u32.wrapping_sub(crc & 1)));
        }
    }
    !crc
}

pub(crate) fn with_crc(mut bytes: Vec<u8>) -> Result<Vec<u8>, CodecError> {
    if bytes.len().saturating_add(4) > MAX_CHECKPOINT_BYTES {
        return Err(CodecError::TooLarge);
    }
    let crc = crc32(&bytes);
    bytes.extend_from_slice(&crc.to_be_bytes());
    Ok(bytes)
}

pub(crate) fn verify_crc(data: &[u8]) -> Result<&[u8], CodecError> {
    if data.len() > MAX_CHECKPOINT_BYTES {
        return Err(CodecError::TooLarge);
    }
    let split_at = data.len().checked_sub(4).ok_or(CodecError::Truncated)?;
    let (payload, expected) = data.split_at(split_at);
    let expected: [u8; 4] = expected.try_into().map_err(|_| CodecError::Truncated)?;
    if crc32(payload) != u32::from_be_bytes(expected) {
        return Err(CodecError::BadChecksum);
    }
    Ok(payload)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crc32_reference_vector() {
        assert_eq!(crc32(b"123456789"), 0xcbf4_3926);
        let data = with_crc(b"trade-layer".to_vec()).unwrap();
        assert_eq!(verify_crc(&data), Ok(&b"trade-layer"[..]));
        let mut damaged = data;
        damaged[0] ^= 1;
        assert_eq!(verify_crc(&damaged), Err(CodecError::BadChecksum));
    }
}
