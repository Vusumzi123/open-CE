//! TES4-family plugin reader for Skyrim Special Edition.
//!
//! The reader covers the container layer: the TES4 file header, GRUP nesting,
//! record headers, subrecords (including the `XXXX` extended-size form),
//! zlib-compressed record payloads, master lists and form identifiers.
//! Record schemas are intentionally out of scope; callers receive the raw
//! subrecord payloads.

use std::borrow::Cow;
use std::collections::HashMap;
use std::fs::{self, File};
use std::io::Read;
use std::path::{Path, PathBuf};

use flate2::read::ZlibDecoder;
use oce_core::{record_flags, strip_nul, Error, FormId, Reader, Result};

pub mod strings;

pub const PLUGIN_SIGNATURE: [u8; 4] = *b"TES4";
pub const GROUP_SIGNATURE: [u8; 4] = *b"GRUP";
pub const RECORD_HEADER_SIZE: usize = 24;
pub const GROUP_HEADER_SIZE: usize = 24;
pub const MAX_GROUP_DEPTH: usize = 128;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PluginKind {
    /// A full master file (`.esm` with the master flag).
    Master,
    /// A light master (`.esl`, or an `.esp` with the light flag).
    LightMaster,
    /// A regular plugin (`.esp`).
    Plugin,
}

impl PluginKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Master => "master",
            Self::LightMaster => "light-master",
            Self::Plugin => "plugin",
        }
    }
}

#[derive(Clone, Debug)]
pub struct PluginHeader {
    pub signature: [u8; 4],
    pub data_size: u32,
    pub flags: u32,
    pub form_id: u32,
    pub version_control_info: u32,
    pub form_version: u16,
    pub version_control_info2: u16,
    pub hed_version: Option<f32>,
    pub num_records: Option<u32>,
    pub next_object_id: Option<u32>,
    pub author: Option<String>,
    pub description: Option<String>,
    pub masters: Vec<String>,
    pub overrides: Vec<FormId>,
}

impl PluginHeader {
    pub fn is_master(&self) -> bool {
        self.flags & oce_core::plugin_flags::MASTER != 0
    }

    pub fn is_light(&self) -> bool {
        self.flags & oce_core::plugin_flags::LIGHT != 0
    }

    pub fn is_localized(&self) -> bool {
        self.flags & oce_core::plugin_flags::LOCALIZED != 0
    }

    /// The load-order index this plugin uses for its own records.
    pub fn own_mod_index(&self) -> u8 {
        self.masters.len().min(0xFF) as u8
    }
}

#[derive(Clone, Copy, Debug)]
pub struct RecordHeader {
    pub signature: [u8; 4],
    pub data_size: u32,
    pub flags: u32,
    pub form_id: FormId,
    pub version_control_info: u32,
    pub form_version: u16,
    pub version_control_info2: u16,
}

impl RecordHeader {
    pub fn is_compressed(&self) -> bool {
        self.flags & record_flags::COMPRESSED != 0
    }

    pub fn is_deleted(&self) -> bool {
        self.flags & record_flags::DELETED != 0
    }

    pub fn is_localized(&self) -> bool {
        self.flags & record_flags::LOCALIZED != 0
    }

    pub fn type_name(&self) -> String {
        String::from_utf8_lossy(&self.signature).into_owned()
    }
}

#[derive(Clone, Copy, Debug)]
pub struct GroupHeader {
    pub signature: [u8; 4],
    pub data_size: u32,
    pub label: [u8; 4],
    pub group_type: i32,
    pub stamp: u16,
    pub unknown: u16,
    pub version_control_info: u16,
    pub unknown2: u16,
}

impl GroupHeader {
    pub fn label_name(&self) -> String {
        String::from_utf8_lossy(&self.label).into_owned()
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Subrecord<'a> {
    pub signature: [u8; 4],
    pub data: &'a [u8],
}

impl<'a> Subrecord<'a> {
    pub fn type_name(&self) -> String {
        String::from_utf8_lossy(&self.signature).into_owned()
    }

    pub fn as_u32(&self) -> Option<u32> {
        (self.data.len() == 4).then(|| u32::from_le_bytes(self.data.try_into().unwrap()))
    }

    pub fn as_i32(&self) -> Option<i32> {
        (self.data.len() == 4).then(|| i32::from_le_bytes(self.data.try_into().unwrap()))
    }

    pub fn as_f32(&self) -> Option<f32> {
        (self.data.len() == 4).then(|| f32::from_le_bytes(self.data.try_into().unwrap()))
    }

    pub fn as_cstring(&self) -> Option<&'a [u8]> {
        if self.data.contains(&0) {
            Some(strip_nul(
                &self.data[..self.data.iter().position(|&b| b == 0).unwrap()],
            ))
        } else {
            Some(self.data)
        }
    }
}

/// A record view borrowing the plugin bytes.
#[derive(Clone, Copy, Debug)]
pub struct Record<'a> {
    pub header: RecordHeader,
    pub body: &'a [u8],
    pub offset: usize,
}

impl<'a> Record<'a> {
    /// Decodes the record body, decompressing it when the record flag is set.
    pub fn decode(&self) -> Result<DecodedRecord<'a>> {
        if !self.header.is_compressed() {
            return Ok(DecodedRecord {
                body: Cow::Borrowed(self.body),
            });
        }

        if self.body.len() < 4 {
            return Err(Error::invalid(format!(
                "compressed record {} at 0x{:X} has no size prefix",
                self.header.type_name(),
                self.offset
            )));
        }
        let expected = u32::from_le_bytes(self.body[..4].try_into().unwrap()) as usize;
        let mut decoder = ZlibDecoder::new(&self.body[4..]);
        let mut decoded = Vec::with_capacity(expected.min(16 * 1024 * 1024));
        decoder.read_to_end(&mut decoded).map_err(|error| {
            Error::invalid(format!(
                "zlib decompression failed for record {} at 0x{:X}: {error}",
                self.header.type_name(),
                self.offset
            ))
        })?;
        if decoded.len() != expected {
            return Err(Error::invalid(format!(
                "record {} at 0x{:X} decompressed to {} bytes, expected {}",
                self.header.type_name(),
                self.offset,
                decoded.len(),
                expected
            )));
        }
        Ok(DecodedRecord {
            body: Cow::Owned(decoded),
        })
    }
}

pub struct DecodedRecord<'a> {
    body: Cow<'a, [u8]>,
}

impl<'a> DecodedRecord<'a> {
    pub fn body(&self) -> &[u8] {
        &self.body
    }

    pub fn subrecords(&self) -> Result<Vec<Subrecord<'_>>> {
        parse_subrecords(&self.body)
    }
}

#[derive(Clone, Copy, Debug)]
pub enum WalkEntry<'a> {
    Group {
        header: GroupHeader,
        offset: usize,
        depth: usize,
    },
    Record {
        header: RecordHeader,
        body: &'a [u8],
        offset: usize,
        depth: usize,
    },
}

#[derive(Clone, Debug, Default)]
pub struct WalkStats {
    pub records: u64,
    pub groups: u64,
    pub compressed_records: u64,
    pub localized_records: u64,
    pub record_types: HashMap<[u8; 4], u64>,
}

pub struct Plugin {
    path: PathBuf,
    data: Vec<u8>,
    header: PluginHeader,
}

impl Plugin {
    /// Reads only the TES4 header record, without loading the rest of the file.
    pub fn read_header(path: impl AsRef<Path>) -> Result<PluginHeader> {
        const MAX_HEADER_DATA: usize = 16 * 1024 * 1024;
        let path = path.as_ref();
        let mut file = File::open(path)
            .map_err(|error| Error::invalid(format!("cannot read {}: {error}", path.display())))?;
        let mut prefix = [0u8; RECORD_HEADER_SIZE];
        file.read_exact(&mut prefix).map_err(|error| {
            Error::invalid(format!("cannot read header of {}: {error}", path.display()))
        })?;
        let data_size = u32::from_le_bytes(prefix[4..8].try_into().unwrap()) as usize;
        if data_size > MAX_HEADER_DATA {
            return Err(Error::invalid(format!(
                "{} declares a {data_size}-byte header record",
                path.display()
            )));
        }
        let mut bytes = vec![0u8; RECORD_HEADER_SIZE + data_size];
        bytes[..RECORD_HEADER_SIZE].copy_from_slice(&prefix);
        file.read_exact(&mut bytes[RECORD_HEADER_SIZE..])
            .map_err(|error| {
                Error::invalid(format!("cannot read header of {}: {error}", path.display()))
            })?;
        parse_header(&bytes)
    }

    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let data = fs::read(path)
            .map_err(|error| Error::invalid(format!("cannot read {}: {error}", path.display())))?;
        Self::from_bytes(path.to_path_buf(), data)
    }

    pub fn from_bytes(path: PathBuf, data: Vec<u8>) -> Result<Self> {
        let header = parse_header(&data)?;
        Ok(Self { path, data, header })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn data(&self) -> &[u8] {
        &self.data
    }

    pub fn header(&self) -> &PluginHeader {
        &self.header
    }

    pub fn kind(&self) -> PluginKind {
        let extension = self
            .path
            .extension()
            .and_then(|value| value.to_str())
            .unwrap_or_default()
            .to_ascii_lowercase();
        if self.header.is_light() || extension == "esl" {
            PluginKind::LightMaster
        } else if self.header.is_master() {
            PluginKind::Master
        } else {
            PluginKind::Plugin
        }
    }

    pub fn local_record_index(&self) -> u8 {
        self.header.own_mod_index()
    }

    fn body_start(&self) -> usize {
        RECORD_HEADER_SIZE + self.header.data_size as usize
    }

    /// Visits every top-level group and record in file order.
    pub fn walk<F>(&self, mut visitor: F) -> Result<WalkStats>
    where
        F: FnMut(WalkEntry<'_>) -> Result<()>,
    {
        let start = self.body_start();
        if start > self.data.len() {
            return Err(Error::invalid("plugin header extends past end of file"));
        }
        let mut stats = WalkStats::default();
        walk_level(&self.data[start..], start, 0, &mut visitor, &mut stats)?;
        Ok(stats)
    }

    pub fn find_record(&self, form_id: FormId) -> Result<Option<Record<'_>>> {
        let start = self.body_start();
        if start > self.data.len() {
            return Err(Error::invalid("plugin header extends past end of file"));
        }
        find_record_in(&self.data[start..], start, form_id)
    }

    /// Returns true when a form id belongs to this plugin, one of its masters,
    /// or (for light masters) the light-master index space.
    pub fn owns_form_id(&self, form_id: FormId) -> bool {
        if form_id.is_light() {
            return self.header.is_light();
        }
        form_id.mod_index() as usize <= self.header.masters.len()
    }

    /// Resolves a form id to a source name using only this plugin's master list.
    pub fn form_id_source(&self, form_id: FormId) -> FormIdSource<'_> {
        if let Some(light) = form_id.light_index() {
            return FormIdSource::Light(light);
        }
        let index = form_id.mod_index() as usize;
        match self.header.masters.get(index) {
            Some(name) => FormIdSource::Master(name),
            None if index == self.header.masters.len() => FormIdSource::Local,
            None => FormIdSource::Unknown(index as u8),
        }
    }
}

/// Convenience wrapper around [`Plugin::read_header`].
pub fn read_header(path: impl AsRef<Path>) -> Result<PluginHeader> {
    Plugin::read_header(path)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FormIdSource<'a> {
    Master(&'a str),
    Local,
    Light(u16),
    Unknown(u8),
}

fn parse_header(data: &[u8]) -> Result<PluginHeader> {
    let mut reader = Reader::new(data);
    let signature = reader.read_array::<4>()?;
    if signature != PLUGIN_SIGNATURE {
        return Err(Error::invalid(format!(
            "expected TES4 signature, found {:?}",
            String::from_utf8_lossy(&signature)
        )));
    }
    let data_size = reader.read_u32()?;
    let flags = reader.read_u32()?;
    let form_id = reader.read_u32()?;
    let version_control_info = reader.read_u32()?;
    let form_version = reader.read_u16()?;
    let version_control_info2 = reader.read_u16()?;

    let body = reader.read_bytes(data_size as usize)?;
    let subrecords = parse_subrecords(body)?;

    let mut header = PluginHeader {
        signature,
        data_size,
        flags,
        form_id,
        version_control_info,
        form_version,
        version_control_info2,
        hed_version: None,
        num_records: None,
        next_object_id: None,
        author: None,
        description: None,
        masters: Vec::new(),
        overrides: Vec::new(),
    };

    for subrecord in subrecords {
        match &subrecord.signature {
            b"HEDR" => {
                if subrecord.data.len() >= 12 {
                    header.hed_version =
                        Some(f32::from_le_bytes(subrecord.data[0..4].try_into().unwrap()));
                    header.num_records =
                        Some(u32::from_le_bytes(subrecord.data[4..8].try_into().unwrap()));
                    header.next_object_id = Some(u32::from_le_bytes(
                        subrecord.data[8..12].try_into().unwrap(),
                    ));
                }
            }
            b"MAST" => header
                .masters
                .push(String::from_utf8_lossy(strip_nul(subrecord.data)).into_owned()),
            b"CNAM" => {
                header.author =
                    Some(String::from_utf8_lossy(strip_nul(subrecord.data)).into_owned())
            }
            b"SNAM" => {
                header.description =
                    Some(String::from_utf8_lossy(strip_nul(subrecord.data)).into_owned())
            }
            b"ONAM" => {
                for chunk in subrecord.data.as_chunks::<4>().0 {
                    header.overrides.push(FormId(u32::from_le_bytes(*chunk)));
                }
            }
            _ => {}
        }
    }

    Ok(header)
}

/// Parses a subrecord list. Handles `XXXX` extended-size subrecords.
pub fn parse_subrecords(body: &[u8]) -> Result<Vec<Subrecord<'_>>> {
    let mut reader = Reader::new(body);
    let mut subrecords = Vec::new();

    while reader.remaining() > 0 {
        if reader.remaining() < 6 {
            return Err(Error::invalid(format!(
                "trailing {} byte(s) after subrecord list at 0x{:X}",
                reader.remaining(),
                reader.position()
            )));
        }
        let signature = reader.read_array::<4>()?;
        let size = reader.read_u16()? as usize;

        if signature == *b"XXXX" {
            if size != 4 {
                return Err(Error::invalid(format!(
                    "XXXX subrecord at 0x{:X} has size {size}, expected 4",
                    reader.position() - 6
                )));
            }
            let extended = reader.read_u32()? as usize;
            let actual_signature = reader.read_array::<4>()?;
            let declared = reader.read_u16()? as usize;
            if declared != 0 {
                return Err(Error::invalid(format!(
                    "extended subrecord {} declares size {declared}, expected 0",
                    String::from_utf8_lossy(&actual_signature)
                )));
            }
            let data = reader.read_bytes(extended)?;
            subrecords.push(Subrecord {
                signature: actual_signature,
                data,
            });
        } else {
            let data = reader.read_bytes(size)?;
            subrecords.push(Subrecord { signature, data });
        }
    }

    Ok(subrecords)
}

fn read_record_header(reader: &mut Reader<'_>) -> Result<RecordHeader> {
    let signature = reader.read_array::<4>()?;
    let data_size = reader.read_u32()?;
    let flags = reader.read_u32()?;
    let form_id = FormId(reader.read_u32()?);
    let version_control_info = reader.read_u32()?;
    let form_version = reader.read_u16()?;
    let version_control_info2 = reader.read_u16()?;
    Ok(RecordHeader {
        signature,
        data_size,
        flags,
        form_id,
        version_control_info,
        form_version,
        version_control_info2,
    })
}

fn read_group_header(reader: &mut Reader<'_>) -> Result<GroupHeader> {
    let signature = reader.read_array::<4>()?;
    let data_size = reader.read_u32()?;
    let label = reader.read_array::<4>()?;
    let group_type = reader.read_i32()?;
    let stamp = reader.read_u16()?;
    let unknown = reader.read_u16()?;
    let version_control_info = reader.read_u16()?;
    let unknown2 = reader.read_u16()?;
    Ok(GroupHeader {
        signature,
        data_size,
        label,
        group_type,
        stamp,
        unknown,
        version_control_info,
        unknown2,
    })
}

fn walk_level<'a, F>(
    body: &'a [u8],
    base_offset: usize,
    depth: usize,
    visitor: &mut F,
    stats: &mut WalkStats,
) -> Result<()>
where
    F: FnMut(WalkEntry<'a>) -> Result<()>,
{
    if depth > MAX_GROUP_DEPTH {
        return Err(Error::invalid(format!(
            "group nesting exceeds {MAX_GROUP_DEPTH} levels"
        )));
    }
    let mut reader = Reader::new(body);
    while reader.remaining() >= RECORD_HEADER_SIZE {
        let start = reader.position();
        let offset = base_offset + start;
        let signature: [u8; 4] = reader.data()[start..start + 4].try_into().unwrap();

        if signature == GROUP_SIGNATURE {
            let header = read_group_header(&mut reader)?;
            if header.data_size < GROUP_HEADER_SIZE as u32 {
                return Err(Error::invalid(format!(
                    "group at 0x{offset:X} has size {}",
                    header.data_size
                )));
            }
            let group_body = reader.read_bytes(header.data_size as usize - GROUP_HEADER_SIZE)?;
            stats.groups += 1;
            visitor(WalkEntry::Group {
                header,
                offset,
                depth,
            })?;
            walk_level(
                group_body,
                offset + GROUP_HEADER_SIZE,
                depth + 1,
                visitor,
                stats,
            )?;
        } else {
            let header = read_record_header(&mut reader)?;
            let record_body = reader.read_bytes(header.data_size as usize)?;
            stats.records += 1;
            *stats.record_types.entry(header.signature).or_default() += 1;
            if header.is_compressed() {
                stats.compressed_records += 1;
            }
            if header.is_localized() {
                stats.localized_records += 1;
            }
            visitor(WalkEntry::Record {
                header,
                body: record_body,
                offset,
                depth,
            })?;
        }
    }

    if reader.remaining() != 0 {
        return Err(Error::invalid(format!(
            "{} trailing byte(s) in container at 0x{:X}",
            reader.remaining(),
            base_offset + reader.position()
        )));
    }
    Ok(())
}

fn find_record_in<'a>(
    body: &'a [u8],
    base_offset: usize,
    form_id: FormId,
) -> Result<Option<Record<'a>>> {
    let mut reader = Reader::new(body);
    while reader.remaining() >= RECORD_HEADER_SIZE {
        let start = reader.position();
        let signature: [u8; 4] = reader.data()[start..start + 4].try_into().unwrap();
        if signature == GROUP_SIGNATURE {
            let header = read_group_header(&mut reader)?;
            if header.data_size < GROUP_HEADER_SIZE as u32 {
                return Err(Error::invalid(format!(
                    "group at 0x{:X} has size {}",
                    base_offset + start,
                    header.data_size
                )));
            }
            let group_body = reader.read_bytes(header.data_size as usize - GROUP_HEADER_SIZE)?;
            if let Some(record) =
                find_record_in(group_body, base_offset + start + GROUP_HEADER_SIZE, form_id)?
            {
                return Ok(Some(record));
            }
        } else {
            let header = read_record_header(&mut reader)?;
            let record_body = reader.read_bytes(header.data_size as usize)?;
            if header.form_id == form_id {
                return Ok(Some(Record {
                    header,
                    body: record_body,
                    offset: base_offset + start,
                }));
            }
        }
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(signature: &[u8; 4], body: &[u8], form_id: u32, flags: u32) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(signature);
        out.extend_from_slice(&(body.len() as u32).to_le_bytes());
        out.extend_from_slice(&flags.to_le_bytes());
        out.extend_from_slice(&form_id.to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
        out.extend_from_slice(&44u16.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(body);
        out
    }

    fn sub(signature: &[u8; 4], data: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(signature);
        out.extend_from_slice(&(data.len() as u16).to_le_bytes());
        out.extend_from_slice(data);
        out
    }

    fn minimal_plugin(flags: u32) -> Vec<u8> {
        let mut header_body = Vec::new();
        let mut hedr = Vec::new();
        hedr.extend_from_slice(&1.7f32.to_le_bytes());
        hedr.extend_from_slice(&1u32.to_le_bytes());
        hedr.extend_from_slice(&0x800u32.to_le_bytes());
        header_body.extend_from_slice(&sub(b"HEDR", &hedr));
        header_body.extend_from_slice(&sub(b"MAST", b"Skyrim.esm\0"));
        header_body.extend_from_slice(&sub(b"DATA", &0u64.to_le_bytes()));
        header_body.extend_from_slice(&sub(b"CNAM", b"test\0"));
        let mut data = record(b"TES4", &header_body, 0, flags);
        data.extend_from_slice(&record(b"GMST", b"", 0x800, 0));
        data
    }

    #[test]
    fn parses_header_and_walks() {
        let data = minimal_plugin(oce_core::plugin_flags::MASTER);
        let plugin = Plugin::from_bytes(PathBuf::from("test.esm"), data).unwrap();
        assert_eq!(plugin.header().masters, vec!["Skyrim.esm"]);
        assert_eq!(plugin.header().num_records, Some(1));
        assert_eq!(plugin.kind(), PluginKind::Master);
        let mut records = 0;
        let stats = plugin
            .walk(|entry| {
                if let WalkEntry::Record { header, .. } = entry {
                    assert_eq!(header.form_id, FormId(0x800));
                    records += 1;
                }
                Ok(())
            })
            .unwrap();
        assert_eq!(records, 1);
        assert_eq!(stats.records, 1);
        assert_eq!(stats.groups, 0);
    }

    #[test]
    fn extended_subrecord() {
        let mut body = Vec::new();
        // XXXX with size 4, then the real subrecord header with size 0.
        body.extend_from_slice(b"XXXX");
        body.extend_from_slice(&4u16.to_le_bytes());
        body.extend_from_slice(&3u32.to_le_bytes());
        body.extend_from_slice(b"DATA");
        body.extend_from_slice(&0u16.to_le_bytes());
        body.extend_from_slice(b"abc");
        let parsed = parse_subrecords(&body).unwrap();
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].signature, *b"DATA");
        assert_eq!(parsed[0].data, b"abc");
    }

    fn game_data() -> Option<PathBuf> {
        let root = std::env::var_os("OCE_SKYRIM_SE").map(PathBuf::from)?;
        let data = if root.join("Data").is_dir() {
            root.join("Data")
        } else {
            root
        };
        data.join("Skyrim.esm").exists().then_some(data)
    }

    #[test]
    fn real_skyrim_esm_if_present() {
        let Some(data) = game_data() else {
            eprintln!("skipping real plugin test: game data not found");
            return;
        };
        let plugin = Plugin::open(data.join("Skyrim.esm")).unwrap();
        let declared = plugin.header().num_records.expect("HEDR record count");
        let stats = plugin.walk(|_| Ok(())).unwrap();
        assert_eq!(stats.records + stats.groups, declared as u64);
        assert!(stats.records > 800_000);
        assert!(stats.groups > 50_000);
        assert!(stats.compressed_records > 40_000);

        let record = plugin
            .find_record(FormId(0x0000_0EB4))
            .unwrap()
            .expect("known NPC_ record is missing");
        assert!(record.header.is_compressed());
        let decoded = record.decode().unwrap();
        let subrecords = decoded.subrecords().unwrap();
        assert!(!subrecords.is_empty());
        eprintln!(
            "real plugin check: {} records, {} groups, {} compressed, {} subrecords in {:08X}",
            stats.records,
            stats.groups,
            stats.compressed_records,
            subrecords.len(),
            record.header.form_id.raw()
        );
    }

    #[test]
    fn real_light_master_if_present() {
        let Some(data) = game_data() else {
            return;
        };
        let path = data.join("ccbgssse002-exoticarrows.esl");
        if !path.exists() {
            eprintln!("skipping light-master test: file not found");
            return;
        }
        let plugin = Plugin::open(path).unwrap();
        assert_eq!(plugin.kind(), PluginKind::LightMaster);
        assert!(plugin.header().is_light());
        let own_index = plugin.header().masters.len() as u8;
        let mut found_local_form = false;
        plugin
            .walk(|entry| {
                if let WalkEntry::Record { header, .. } = entry {
                    // ESL files store their own forms with the "number of
                    // masters" index; the game remaps that to 0xFE at load time.
                    if header.form_id.mod_index() == own_index {
                        found_local_form = true;
                    }
                }
                Ok(())
            })
            .unwrap();
        assert!(found_local_form);
    }

    #[test]
    fn compressed_record_decodes() {
        use flate2::write::ZlibEncoder;
        use flate2::Compression;
        use std::io::Write;

        let payload = b"hello compressed world";
        let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(payload).unwrap();
        let compressed = encoder.finish().unwrap();
        let mut body = Vec::new();
        body.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        body.extend_from_slice(&compressed);
        let mut data = minimal_plugin(0);
        data.extend_from_slice(&record(b"NAVM", &body, 0x1234, record_flags::COMPRESSED));
        let plugin = Plugin::from_bytes(PathBuf::from("test.esp"), data).unwrap();
        let found = plugin.find_record(FormId(0x1234)).unwrap().unwrap();
        assert!(found.header.is_compressed());
        let decoded = found.decode().unwrap();
        assert_eq!(decoded.body(), payload);
    }
}
