//! Shared primitives for the Open CE data layer.
//!
//! This crate deliberately has no dependencies so the format readers remain
//! usable in constrained builds. It carries the values and flags shared by the
//! plugin and archive readers.

use std::fmt;

/// A TES4-family form identifier.
///
/// The top byte is the load-order index, except for light masters where the
/// top byte is `0xFE` and the next twelve bits are the light index.
#[derive(Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct FormId(pub u32);

impl FormId {
    pub const fn new(raw: u32) -> Self {
        Self(raw)
    }

    pub const fn raw(self) -> u32 {
        self.0
    }

    pub const fn mod_index(self) -> u8 {
        (self.0 >> 24) as u8
    }

    pub const fn object_index(self) -> u32 {
        self.0 & 0x00FF_FFFF
    }

    pub const fn is_light(self) -> bool {
        self.mod_index() == 0xFE
    }

    /// Light-master index for a `0xFE` form id.
    pub const fn light_index(self) -> Option<u16> {
        if self.is_light() {
            Some(((self.0 >> 12) & 0x0FFF) as u16)
        } else {
            None
        }
    }
}

impl fmt::Display for FormId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:08X}", self.0)
    }
}

impl fmt::Debug for FormId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "FormId(0x{:08X})", self.0)
    }
}

/// Plugin header flags used by Skyrim Special Edition.
pub mod plugin_flags {
    /// The plugin is a master (ESM).
    pub const MASTER: u32 = 0x0000_0001;
    /// Record strings are stored in external `.STRINGS` tables.
    pub const LOCALIZED: u32 = 0x0000_0080;
    /// The plugin is a light master (ESL).
    pub const LIGHT: u32 = 0x0000_0200;
}

/// Record header flags used by Skyrim Special Edition.
pub mod record_flags {
    /// The record is deleted.
    pub const DELETED: u32 = 0x0000_0020;
    /// The record's string fields are string-table ids.
    pub const LOCALIZED: u32 = 0x0000_0080;
    /// The record payload is zlib-compressed and starts with its uncompressed size.
    pub const COMPRESSED: u32 = 0x0004_0000;
}

/// BSA archive flags. Only the bits we interpret are named.
pub mod archive_flags {
    pub const INCLUDE_DIRECTORY_NAMES: u32 = 0x0000_0001;
    pub const INCLUDE_FILE_NAMES: u32 = 0x0000_0002;
    pub const COMPRESSED: u32 = 0x0000_0004;
    pub const RETAIN_DIRECTORY_NAMES: u32 = 0x0000_0008;
    pub const RETAIN_FILE_NAMES: u32 = 0x0000_0010;
    pub const RETAIN_FILE_NAME_OFFSETS: u32 = 0x0000_0020;
    pub const XBOX360_ARCHIVE: u32 = 0x0000_0040;
    pub const RETAIN_STRINGS_DURING_STARTUP: u32 = 0x0000_0080;
    pub const EMBED_FILE_NAMES: u32 = 0x0000_0100;
    pub const XMEM_CODEC: u32 = 0x0000_0200;
}

/// BSA file flags (a bitmask of the content families in the archive).
pub mod archive_file_flags {
    pub const MESHES: u32 = 0x0000_0001;
    pub const TEXTURES: u32 = 0x0000_0002;
    pub const MENUS: u32 = 0x0000_0004;
    pub const SOUNDS: u32 = 0x0000_0008;
    pub const VOICES: u32 = 0x0000_0010;
    pub const SHADERS: u32 = 0x0000_0020;
    pub const TREES: u32 = 0x0000_0040;
    pub const FONTS: u32 = 0x0000_0080;
    pub const MISCELLANEOUS: u32 = 0x0000_0100;
}

#[derive(Debug)]
pub enum Error {
    Io(std::io::Error),
    InvalidData(String),
    Unsupported(String),
}

impl Error {
    pub fn invalid(message: impl Into<String>) -> Self {
        Self::InvalidData(message.into())
    }

    pub fn unsupported(message: impl Into<String>) -> Self {
        Self::Unsupported(message.into())
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(f, "I/O error: {error}"),
            Self::InvalidData(message) => write!(f, "invalid data: {message}"),
            Self::Unsupported(message) => write!(f, "unsupported: {message}"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            _ => None,
        }
    }
}

impl From<std::io::Error> for Error {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}

pub type Result<T> = std::result::Result<T, Error>;

/// Bounds-checked little-endian byte reader.
#[derive(Clone)]
pub struct Reader<'a> {
    data: &'a [u8],
    position: usize,
}

impl<'a> Reader<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        Self { data, position: 0 }
    }

    pub fn position(&self) -> usize {
        self.position
    }

    pub fn remaining(&self) -> usize {
        self.data.len().saturating_sub(self.position)
    }

    pub fn data(&self) -> &'a [u8] {
        self.data
    }

    pub fn is_empty(&self) -> bool {
        self.remaining() == 0
    }

    pub fn read_bytes(&mut self, length: usize) -> Result<&'a [u8]> {
        let end = self
            .position
            .checked_add(length)
            .ok_or_else(|| Error::invalid("read length overflow"))?;
        if end > self.data.len() {
            return Err(Error::invalid(format!(
                "unexpected end of data at 0x{:X}: need {length} bytes, have {}",
                self.position,
                self.remaining()
            )));
        }
        let bytes = &self.data[self.position..end];
        self.position = end;
        Ok(bytes)
    }

    pub fn skip(&mut self, length: usize) -> Result<()> {
        self.read_bytes(length).map(|_| ())
    }

    pub fn read_array<const N: usize>(&mut self) -> Result<[u8; N]> {
        let bytes = self.read_bytes(N)?;
        let mut array = [0u8; N];
        array.copy_from_slice(bytes);
        Ok(array)
    }

    pub fn read_u8(&mut self) -> Result<u8> {
        Ok(self.read_array::<1>()?[0])
    }

    pub fn read_u16(&mut self) -> Result<u16> {
        Ok(u16::from_le_bytes(self.read_array()?))
    }

    pub fn read_i32(&mut self) -> Result<i32> {
        Ok(i32::from_le_bytes(self.read_array()?))
    }

    pub fn read_u32(&mut self) -> Result<u32> {
        Ok(u32::from_le_bytes(self.read_array()?))
    }

    pub fn read_u64(&mut self) -> Result<u64> {
        Ok(u64::from_le_bytes(self.read_array()?))
    }

    pub fn read_f32(&mut self) -> Result<f32> {
        Ok(f32::from_le_bytes(self.read_array()?))
    }

    /// Reads a NUL-terminated string and consumes the terminator when present.
    pub fn read_cstring(&mut self) -> Result<&'a [u8]> {
        let start = self.position;
        while self.position < self.data.len() {
            if self.data[self.position] == 0 {
                let bytes = &self.data[start..self.position];
                self.position += 1;
                return Ok(bytes);
            }
            self.position += 1;
        }
        Err(Error::invalid(format!(
            "unterminated string at 0x{start:X}"
        )))
    }

    /// Reads a byte-length-prefixed string. The length includes the trailing NUL
    /// in BSA archives; a NUL is stripped when present.
    pub fn read_bzstring(&mut self) -> Result<&'a [u8]> {
        let length = self.read_u8()? as usize;
        if length == 0 {
            return Ok(&[]);
        }
        let bytes = self.read_bytes(length)?;
        Ok(bytes.strip_suffix(&[0]).unwrap_or(bytes))
    }
}

/// Strips a single trailing NUL from a byte slice.
pub fn strip_nul(bytes: &[u8]) -> &[u8] {
    bytes.strip_suffix(&[0]).unwrap_or(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn form_id_parts() {
        let id = FormId(0x0001_2345);
        assert_eq!(id.mod_index(), 0);
        assert_eq!(id.object_index(), 0x01_2345);
        assert!(!id.is_light());

        let light = FormId(0xFE01_2345);
        assert!(light.is_light());
        assert_eq!(light.light_index(), Some(0x0012));
        assert_eq!(light.object_index(), 0x01_2345);
    }

    #[test]
    fn reader_bounds() {
        let mut reader = Reader::new(&[1, 2, 3, 4]);
        assert_eq!(reader.read_u16().unwrap(), 0x0201);
        assert_eq!(reader.read_u16().unwrap(), 0x0403);
        assert!(reader.read_u8().is_err());
    }

    #[test]
    fn bzstring_strips_nul() {
        let mut reader = Reader::new(b"\x04seq\0");
        assert_eq!(reader.read_bzstring().unwrap(), b"seq");
        assert_eq!(reader.remaining(), 0);
    }
}
