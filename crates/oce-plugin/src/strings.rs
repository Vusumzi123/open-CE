//! Reader for Skyrim `.STRINGS`, `.DLSTRINGS` and `.ILSTRINGS` tables.
//!
//! The file is a small index: a count, then `(string id, absolute offset)`
//! pairs, followed by NUL-terminated UTF-8 strings.

use std::collections::BTreeMap;

use oce_core::{strip_nul, Error, Reader, Result};

#[derive(Clone, Debug, Default)]
pub struct StringsFile {
    entries: BTreeMap<u32, String>,
}

impl StringsFile {
    pub fn open(path: impl AsRef<std::path::Path>) -> Result<Self> {
        let data = std::fs::read(path.as_ref()).map_err(|error| {
            Error::invalid(format!(
                "cannot read strings file {}: {error}",
                path.as_ref().display()
            ))
        })?;
        Self::parse(&data)
    }

    pub fn parse(data: &[u8]) -> Result<Self> {
        let mut reader = Reader::new(data);
        let count = reader.read_u32()? as usize;
        if count > data.len() / 8 {
            return Err(Error::invalid(format!(
                "strings file declares {count} entries for {} bytes",
                data.len()
            )));
        }
        let mut index = Vec::with_capacity(count);
        for _ in 0..count {
            index.push((reader.read_u32()?, reader.read_u32()? as usize));
        }

        let mut entries = BTreeMap::new();
        for (id, offset) in index {
            if offset >= data.len() {
                return Err(Error::invalid(format!(
                    "string {id} points past end of file at 0x{offset:X}"
                )));
            }
            let rest = &data[offset..];
            let end = rest
                .iter()
                .position(|&byte| byte == 0)
                .unwrap_or(rest.len());
            entries.insert(
                id,
                String::from_utf8_lossy(strip_nul(&rest[..end])).into_owned(),
            );
        }
        Ok(Self { entries })
    }

    pub fn get(&self, id: u32) -> Option<&str> {
        self.entries.get(&id).map(String::as_str)
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn entries(&self) -> impl Iterator<Item = (u32, &str)> {
        self.entries.iter().map(|(&id, value)| (id, value.as_str()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_index_and_strings() {
        let mut data = Vec::new();
        data.extend_from_slice(&2u32.to_le_bytes());
        data.extend_from_slice(&1u32.to_le_bytes());
        data.extend_from_slice(&20u32.to_le_bytes());
        data.extend_from_slice(&2u32.to_le_bytes());
        data.extend_from_slice(&26u32.to_le_bytes());
        data.extend_from_slice(b"first\0");
        data.extend_from_slice(b"second\0");
        let table = StringsFile::parse(&data).unwrap();
        assert_eq!(table.get(1), Some("first"));
        assert_eq!(table.get(2), Some("second"));
        assert_eq!(table.len(), 2);
    }
}
