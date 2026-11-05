//! BSA version 105 archive reader (Skyrim Special Edition).
//!
//! Version 105 stores 24-byte folder records, interleaves each folder name with
//! its file records, then stores a NUL-terminated file-name block before the
//! payload area. Compressed archives store each file as a 4-byte uncompressed
//! size followed by an LZ4 frame. Archives with the `EMBED_FILE_NAMES` flag
//! prefix each payload with a byte-length-prefixed path.
//!
//! Only metadata is read at open time. File payloads are read on demand.

use std::collections::HashMap;
use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use oce_core::{archive_flags, Error, Reader, Result};

pub const BSA_MAGIC: [u8; 4] = *b"BSA\0";
pub const SUPPORTED_VERSION: u32 = 105;
pub const HEADER_SIZE: usize = 36;
pub const FOLDER_RECORD_SIZE: usize = 24;
pub const FILE_RECORD_SIZE: usize = 16;
const MAX_METADATA_SIZE: usize = 256 * 1024 * 1024;
const MAX_DECOMPRESSED_FILE: usize = 2 * 1024 * 1024 * 1024;

#[derive(Clone, Debug)]
pub struct Header {
    pub version: u32,
    pub offset: u32,
    pub archive_flags: u32,
    pub folder_count: u32,
    pub file_count: u32,
    pub total_folder_name_length: u32,
    pub total_file_name_length: u32,
    pub file_flags: u32,
}

impl Header {
    pub fn compressed(&self) -> bool {
        self.archive_flags & archive_flags::COMPRESSED != 0
    }

    pub fn embedded_file_names(&self) -> bool {
        self.archive_flags & archive_flags::EMBED_FILE_NAMES != 0
    }
}

#[derive(Clone, Debug)]
pub struct Folder {
    pub name: String,
    pub hash: u64,
    pub first_file: usize,
    pub file_count: usize,
}

#[derive(Clone, Debug)]
pub struct FileEntry {
    pub folder: String,
    pub name: String,
    pub path: String,
    pub hash: u64,
    pub size_flags: u32,
    pub offset: u32,
}

impl FileEntry {
    /// Stored byte count, with the compression and embedded-name flag bits removed.
    pub fn stored_size(&self) -> u32 {
        self.size_flags & 0x3FFF_FFFF
    }

    pub fn is_compressed(&self, header: &Header) -> bool {
        header.compressed() && (self.size_flags & 0x4000_0000) == 0
    }
}

pub struct Archive {
    path: PathBuf,
    file_size: u64,
    header: Header,
    folders: Vec<Folder>,
    files: Vec<FileEntry>,
    index: HashMap<String, usize>,
}

impl Archive {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let file = File::open(path).map_err(|error| {
            Error::invalid(format!("cannot open archive {}: {error}", path.display()))
        })?;
        let file_size = file
            .metadata()
            .map_err(|error| {
                Error::invalid(format!("cannot stat archive {}: {error}", path.display()))
            })?
            .len();

        let mut header_bytes = [0u8; HEADER_SIZE];
        let mut file = file;
        file.read_exact(&mut header_bytes).map_err(|error| {
            Error::invalid(format!(
                "cannot read archive header {}: {error}",
                path.display()
            ))
        })?;
        let header = parse_header(&header_bytes, file_size)?;
        let metadata_size = metadata_size(&header)?;
        if metadata_size as u64 > file_size {
            return Err(Error::invalid(format!(
                "archive metadata is {metadata_size} bytes but file is {file_size} bytes"
            )));
        }

        file.seek(SeekFrom::Start(0))?;
        let mut metadata = vec![0u8; metadata_size];
        file.read_exact(&mut metadata)?;

        let parsed = parse_metadata(&header, &metadata)?;
        Ok(Self {
            path: path.to_path_buf(),
            file_size,
            header,
            folders: parsed.folders,
            files: parsed.files,
            index: parsed.index,
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn header(&self) -> &Header {
        &self.header
    }

    pub fn folders(&self) -> &[Folder] {
        &self.folders
    }

    pub fn files(&self) -> &[FileEntry] {
        &self.files
    }

    pub fn file_count(&self) -> usize {
        self.files.len()
    }

    pub fn find(&self, path: &str) -> Option<usize> {
        self.index.get(&normalize_path(path)).copied()
    }

    pub fn entry(&self, index: usize) -> Option<&FileEntry> {
        self.files.get(index)
    }

    pub fn read_file(&self, index: usize) -> Result<Vec<u8>> {
        let entry = self
            .files
            .get(index)
            .ok_or_else(|| Error::invalid(format!("file index {index} is out of range")))?;
        let stored_size = entry.stored_size() as usize;
        if stored_size == 0 {
            return Ok(Vec::new());
        }
        if entry.offset as u64 + stored_size as u64 > self.file_size {
            return Err(Error::invalid(format!(
                "{} points past end of archive (offset 0x{:X}, size {})",
                entry.path, entry.offset, stored_size
            )));
        }

        let mut file = File::open(&self.path)?;
        file.seek(SeekFrom::Start(entry.offset as u64))?;
        let mut raw = vec![0u8; stored_size];
        file.read_exact(&mut raw)?;

        let mut payload: &[u8] = &raw;
        if self.header.embedded_file_names() {
            let mut reader = Reader::new(payload);
            let embedded = reader.read_bzstring()?;
            let consumed = reader.position();
            let _ = embedded;
            payload = &payload[consumed..];
        }

        if entry.is_compressed(&self.header) {
            if payload.len() < 4 {
                return Err(Error::invalid(format!(
                    "compressed file {} has no size prefix",
                    entry.path
                )));
            }
            let expected = u32::from_le_bytes(payload[..4].try_into().unwrap()) as usize;
            if expected > MAX_DECOMPRESSED_FILE {
                return Err(Error::invalid(format!(
                    "{} declares an implausible uncompressed size of {expected}",
                    entry.path
                )));
            }
            let decoded = decompress_lz4_frame(&payload[4..], expected).map_err(|error| {
                Error::invalid(format!("cannot decompress {}: {error}", entry.path))
            })?;
            Ok(decoded)
        } else {
            Ok(payload.to_vec())
        }
    }

    pub fn read_path(&self, path: &str) -> Result<Option<Vec<u8>>> {
        match self.find(path) {
            Some(index) => self.read_file(index).map(Some),
            None => Ok(None),
        }
    }

    pub fn extract_to(&self, index: usize, output: impl AsRef<Path>) -> Result<usize> {
        let bytes = self.read_file(index)?;
        fs::write(output.as_ref(), &bytes).map_err(|error| {
            Error::invalid(format!(
                "cannot write {}: {error}",
                output.as_ref().display()
            ))
        })?;
        Ok(bytes.len())
    }
}

fn parse_header(bytes: &[u8], file_size: u64) -> Result<Header> {
    let mut reader = Reader::new(bytes);
    let magic = reader.read_array::<4>()?;
    if magic != BSA_MAGIC {
        return Err(Error::invalid(format!(
            "expected BSA\\0 signature, found {:?}",
            String::from_utf8_lossy(&magic)
        )));
    }
    let version = reader.read_u32()?;
    if version != SUPPORTED_VERSION {
        return Err(Error::unsupported(format!(
            "BSA version {version}; this reader implements version {SUPPORTED_VERSION}"
        )));
    }
    let offset = reader.read_u32()?;
    if offset as usize != HEADER_SIZE {
        return Err(Error::invalid(format!(
            "BSA header offset is {offset}, expected {HEADER_SIZE}"
        )));
    }
    let header = Header {
        version,
        offset,
        archive_flags: reader.read_u32()?,
        folder_count: reader.read_u32()?,
        file_count: reader.read_u32()?,
        total_folder_name_length: reader.read_u32()?,
        total_file_name_length: reader.read_u32()?,
        file_flags: reader.read_u32()?,
    };
    if header.folder_count as u64 > file_size / FOLDER_RECORD_SIZE as u64 {
        return Err(Error::invalid("folder count exceeds file size"));
    }
    if header.file_count as u64 > file_size / FILE_RECORD_SIZE as u64 {
        return Err(Error::invalid("file count exceeds file size"));
    }
    Ok(header)
}

fn metadata_size(header: &Header) -> Result<usize> {
    let folder_records = (header.folder_count as usize)
        .checked_mul(FOLDER_RECORD_SIZE)
        .ok_or_else(|| Error::invalid("folder record size overflow"))?;
    let file_records = (header.file_count as usize)
        .checked_mul(FILE_RECORD_SIZE)
        .ok_or_else(|| Error::invalid("file record size overflow"))?;
    let folder_names = (header.total_folder_name_length as usize)
        .checked_add(header.folder_count as usize)
        .ok_or_else(|| Error::invalid("folder name size overflow"))?;
    let size = HEADER_SIZE
        .checked_add(folder_records)
        .and_then(|value| value.checked_add(folder_names))
        .and_then(|value| value.checked_add(file_records))
        .and_then(|value| value.checked_add(header.total_file_name_length as usize))
        .ok_or_else(|| Error::invalid("metadata size overflow"))?;
    if size > MAX_METADATA_SIZE {
        return Err(Error::invalid(format!(
            "archive metadata is {size} bytes, above the {MAX_METADATA_SIZE} byte cap"
        )));
    }
    Ok(size)
}

struct ParsedMetadata {
    folders: Vec<Folder>,
    files: Vec<FileEntry>,
    index: HashMap<String, usize>,
}

fn parse_metadata(header: &Header, metadata: &[u8]) -> Result<ParsedMetadata> {
    let names_start = metadata
        .len()
        .checked_sub(header.total_file_name_length as usize)
        .ok_or_else(|| Error::invalid("file-name block is larger than the metadata"))?;

    let mut folders = Vec::with_capacity(header.folder_count as usize);
    let mut raw_files = Vec::with_capacity(header.file_count as usize);
    let mut cursor = HEADER_SIZE;

    for folder_index in 0..header.folder_count as usize {
        let record_offset = HEADER_SIZE + folder_index * FOLDER_RECORD_SIZE;
        let mut reader = Reader::new(&metadata[record_offset..record_offset + FOLDER_RECORD_SIZE]);
        let hash = reader.read_u64()?;
        let count = reader.read_u32()? as usize;
        let _padding = reader.read_u32()?;
        let name_offset = reader.read_u32()? as usize;
        let _padding2 = reader.read_u32()?;

        let name_position = name_offset
            .checked_sub(header.total_file_name_length as usize)
            .ok_or_else(|| {
                Error::invalid(format!(
                    "folder {folder_index} name offset 0x{name_offset:X} is below the file-name block"
                ))
            })?;
        if name_position >= metadata.len() {
            return Err(Error::invalid(format!(
                "folder {folder_index} name offset 0x{name_offset:X} is out of range"
            )));
        }
        // The byte-length prefix includes the trailing NUL.
        let prefix_len = metadata[name_position] as usize;
        let mut name_reader = Reader::new(&metadata[name_position..]);
        let name = name_reader.read_bzstring()?;

        let first_file = raw_files.len();
        let mut file_reader = Reader::new(
            metadata
                .get(name_position + 1 + prefix_len..names_start)
                .ok_or_else(|| Error::invalid("folder name extends into the file-name block"))?,
        );
        for _ in 0..count {
            let hash = file_reader.read_u64()?;
            let size = file_reader.read_u32()?;
            let offset = file_reader.read_u32()?;
            raw_files.push((hash, size, offset));
        }

        folders.push(Folder {
            name: String::from_utf8_lossy(name).into_owned(),
            hash,
            first_file,
            file_count: count,
        });
        cursor = files_end(&metadata[..names_start], name_position, prefix_len, count)?;
    }

    if cursor != names_start {
        return Err(Error::invalid(format!(
            "folder records end at 0x{cursor:X}, expected file-name block at 0x{names_start:X}"
        )));
    }
    if raw_files.len() != header.file_count as usize {
        return Err(Error::invalid(format!(
            "parsed {} file records, header declares {}",
            raw_files.len(),
            header.file_count
        )));
    }

    let mut names = Vec::with_capacity(header.file_count as usize);
    let mut name_cursor = names_start;
    for _ in 0..header.file_count {
        let end = metadata[name_cursor..]
            .iter()
            .position(|&byte| byte == 0)
            .map(|position| name_cursor + position)
            .ok_or_else(|| Error::invalid("unterminated file name in name block"))?;
        let name = String::from_utf8_lossy(&metadata[name_cursor..end]).into_owned();
        names.push(name);
        name_cursor = end + 1;
    }
    if name_cursor != metadata.len() {
        let trailing = &metadata[name_cursor..];
        // Some BSA archives pad the name block with zero bytes.
        if trailing.len() > 64 || trailing.iter().any(|&byte| byte != 0) {
            return Err(Error::invalid(format!(
                "file-name block ended at 0x{name_cursor:X}, metadata ends at 0x{:X}",
                metadata.len()
            )));
        }
    }

    let mut files = Vec::with_capacity(header.file_count as usize);
    let mut index = HashMap::with_capacity(header.file_count as usize);
    let mut file_index = 0usize;
    for folder in &folders {
        for _ in 0..folder.file_count {
            let (hash, size_flags, offset) = raw_files[file_index];
            let name = names[file_index].clone();
            let path = if folder.name.is_empty() {
                name.clone()
            } else {
                format!("{}\\{}", folder.name, name)
            };
            let normalized = normalize_path(&path);
            index.entry(normalized).or_insert(file_index);
            files.push(FileEntry {
                folder: folder.name.clone(),
                name,
                path,
                hash,
                size_flags,
                offset,
            });
            file_index += 1;
        }
    }

    if files.is_empty() {
        return Ok(ParsedMetadata {
            folders,
            files,
            index,
        });
    }
    let data_start = files.iter().map(|file| file.offset).min().unwrap() as usize;
    if names_start > data_start {
        return Err(Error::invalid(format!(
            "file-name block ends at 0x{names_start:X}, but data starts at 0x{data_start:X}"
        )));
    }

    Ok(ParsedMetadata {
        folders,
        files,
        index,
    })
}

fn files_end(
    metadata: &[u8],
    name_position: usize,
    prefix_len: usize,
    count: usize,
) -> Result<usize> {
    let start = name_position
        .checked_add(1)
        .and_then(|value| value.checked_add(prefix_len))
        .ok_or_else(|| Error::invalid("folder name size overflow"))?;
    let end = start
        .checked_add(
            count
                .checked_mul(FILE_RECORD_SIZE)
                .ok_or_else(|| Error::invalid("folder file-record size overflow"))?,
        )
        .ok_or_else(|| Error::invalid("folder end overflow"))?;
    if end > metadata.len() {
        return Err(Error::invalid(format!(
            "folder at 0x{name_position:X} extends past metadata"
        )));
    }
    Ok(end)
}

fn normalize_path(path: &str) -> String {
    path.replace('/', "\\").to_ascii_lowercase()
}

/// Decompresses an LZ4 frame with a known output size.
pub fn decompress_lz4_frame(source: &[u8], expected: usize) -> Result<Vec<u8>> {
    if source.len() < 7 {
        return Err(Error::invalid("LZ4 frame is too short"));
    }
    if source[..4] != [0x04, 0x22, 0x4D, 0x18] {
        return Err(Error::invalid("bad LZ4 frame magic"));
    }
    let flg = source[4];
    let _bd = source[5];
    if flg >> 6 != 1 {
        return Err(Error::invalid("unsupported LZ4 frame version"));
    }
    let content_size_present = flg & 0x08 != 0;
    let content_checksum = flg & 0x04 != 0;
    let block_checksum = flg & 0x10 != 0;
    let dict_id_present = flg & 0x01 != 0;

    let mut position = 6usize;
    if content_size_present {
        if source.len() < position + 8 {
            return Err(Error::invalid("truncated LZ4 content size"));
        }
        position += 8;
    }
    if dict_id_present {
        if source.len() < position + 4 {
            return Err(Error::invalid("truncated LZ4 dictionary id"));
        }
        position += 4;
    }
    if source.len() <= position {
        return Err(Error::invalid("truncated LZ4 frame header"));
    }
    position += 1; // header checksum

    let mut output = Vec::with_capacity(expected.min(64 * 1024 * 1024));
    loop {
        if source.len() < position + 4 {
            return Err(Error::invalid("truncated LZ4 block header"));
        }
        let word = u32::from_le_bytes(source[position..position + 4].try_into().unwrap());
        position += 4;
        if word == 0 {
            break;
        }
        let uncompressed = word & 0x8000_0000 != 0;
        let block_size = (word & 0x7FFF_FFFF) as usize;
        if source.len() < position + block_size {
            return Err(Error::invalid("truncated LZ4 block"));
        }
        let block = &source[position..position + block_size];
        position += block_size;
        if uncompressed {
            if output.len() + block_size > expected {
                return Err(Error::invalid("LZ4 uncompressed block overruns output"));
            }
            output.extend_from_slice(block);
        } else {
            decompress_lz4_block(block, &mut output, expected)?;
        }
        if block_checksum {
            if source.len() < position + 4 {
                return Err(Error::invalid("truncated LZ4 block checksum"));
            }
            position += 4;
        }
    }
    if content_checksum && source.len() < position + 4 {
        return Err(Error::invalid("truncated LZ4 content checksum"));
    }
    if output.len() != expected {
        return Err(Error::invalid(format!(
            "LZ4 output is {} bytes, expected {expected}",
            output.len()
        )));
    }
    Ok(output)
}

fn decompress_lz4_block(source: &[u8], output: &mut Vec<u8>, expected: usize) -> Result<()> {
    let mut position = 0usize;
    while position < source.len() {
        let token = source[position];
        position += 1;
        let mut literal_length = (token >> 4) as usize;
        if literal_length == 15 {
            loop {
                let byte = *source
                    .get(position)
                    .ok_or_else(|| Error::invalid("truncated LZ4 literal length"))?;
                position += 1;
                literal_length += byte as usize;
                if byte != 255 {
                    break;
                }
            }
        }
        if position + literal_length > source.len() {
            return Err(Error::invalid("LZ4 literal overruns input"));
        }
        if output.len() + literal_length > expected {
            return Err(Error::invalid("LZ4 literal overruns output"));
        }
        output.extend_from_slice(&source[position..position + literal_length]);
        position += literal_length;
        if position == source.len() {
            break;
        }

        if position + 2 > source.len() {
            return Err(Error::invalid("truncated LZ4 match offset"));
        }
        let offset = u16::from_le_bytes([source[position], source[position + 1]]) as usize;
        position += 2;
        if offset == 0 || offset > output.len() {
            return Err(Error::invalid("invalid LZ4 match offset"));
        }
        let mut match_length = (token & 0x0F) as usize + 4;
        if token & 0x0F == 15 {
            loop {
                let byte = *source
                    .get(position)
                    .ok_or_else(|| Error::invalid("truncated LZ4 match length"))?;
                position += 1;
                match_length += byte as usize;
                if byte != 255 {
                    break;
                }
            }
        }
        if output.len() + match_length > expected {
            return Err(Error::invalid("LZ4 match overruns output"));
        }
        let start = output.len() - offset;
        for index in 0..match_length {
            let byte = output[start + index];
            output.push(byte);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn lz4_uncompressed_block() {
        let mut frame = Vec::new();
        frame.extend_from_slice(&[0x04, 0x22, 0x4D, 0x18]);
        frame.push(0x60); // version 1, independent blocks
        frame.push(0x70); // 4 MiB max block
        frame.push(0x00); // header checksum (not verified)
        frame.extend_from_slice(&(0x8000_0005u32).to_le_bytes());
        frame.extend_from_slice(b"hello");
        frame.extend_from_slice(&0u32.to_le_bytes());
        assert_eq!(decompress_lz4_frame(&frame, 5).unwrap(), b"hello");
    }

    #[test]
    fn lz4_compressed_block() {
        let mut frame = Vec::new();
        frame.extend_from_slice(&[0x04, 0x22, 0x4D, 0x18]);
        frame.push(0x60);
        frame.push(0x70);
        frame.push(0x00);
        // One literal 'a', then a match of length 9 at offset 1.
        let block = [0x15, b'a', 0x01, 0x00];
        frame.extend_from_slice(&(block.len() as u32).to_le_bytes());
        frame.extend_from_slice(&block);
        frame.extend_from_slice(&0u32.to_le_bytes());
        assert_eq!(
            decompress_lz4_frame(&frame, 10).unwrap(),
            b"aaaaaaaaaa".to_vec()
        );
    }

    #[test]
    fn parses_tiny_archive() {
        let folder_name = b"test\0";
        let file_name = b"a.txt\0";
        let file_data = b"data";
        let folder_name_length = folder_name.len() as u32;
        let file_name_length = file_name.len() as u32;

        let metadata_len = HEADER_SIZE
            + FOLDER_RECORD_SIZE
            + 1
            + folder_name.len()
            + FILE_RECORD_SIZE
            + file_name.len();
        let data_offset = metadata_len as u32;

        let mut bytes = Vec::new();
        bytes.extend_from_slice(&BSA_MAGIC);
        bytes.extend_from_slice(&SUPPORTED_VERSION.to_le_bytes());
        bytes.extend_from_slice(&(HEADER_SIZE as u32).to_le_bytes());
        bytes.extend_from_slice(&0x3u32.to_le_bytes());
        bytes.extend_from_slice(&1u32.to_le_bytes());
        bytes.extend_from_slice(&1u32.to_le_bytes());
        bytes.extend_from_slice(&folder_name_length.to_le_bytes());
        bytes.extend_from_slice(&file_name_length.to_le_bytes());
        bytes.extend_from_slice(&0u32.to_le_bytes());

        bytes.extend_from_slice(&0u64.to_le_bytes());
        bytes.extend_from_slice(&1u32.to_le_bytes());
        bytes.extend_from_slice(&0u32.to_le_bytes());
        let name_offset = (HEADER_SIZE + FOLDER_RECORD_SIZE) as u32 + file_name_length;
        bytes.extend_from_slice(&name_offset.to_le_bytes());
        bytes.extend_from_slice(&0u32.to_le_bytes());

        bytes.push(folder_name.len() as u8);
        bytes.extend_from_slice(folder_name);
        bytes.extend_from_slice(&0u64.to_le_bytes());
        bytes.extend_from_slice(&(file_data.len() as u32).to_le_bytes());
        bytes.extend_from_slice(&data_offset.to_le_bytes());
        bytes.extend_from_slice(file_name);
        bytes.extend_from_slice(file_data);

        let path = std::env::temp_dir().join(format!(
            "oce-tiny-{}-{}.bsa",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        {
            let mut file = File::create(&path).unwrap();
            file.write_all(&bytes).unwrap();
        }

        let archive = Archive::open(&path).unwrap();
        assert_eq!(archive.header().folder_count, 1);
        assert_eq!(archive.file_count(), 1);
        assert_eq!(archive.files()[0].path, "test\\a.txt");
        assert_eq!(archive.read_file(0).unwrap(), b"data");
        assert_eq!(archive.find("TEST/A.TXT"), Some(0));
        fs::remove_file(path).unwrap();
    }

    fn game_data() -> Option<PathBuf> {
        let root = std::env::var_os("OCE_SKYRIM_SE").map(PathBuf::from)?;
        let data = if root.join("Data").is_dir() {
            root.join("Data")
        } else {
            root
        };
        data.join("Skyrim - Meshes0.bsa").exists().then_some(data)
    }

    #[test]
    fn real_skyrim_archives_if_present() {
        let Some(data) = game_data() else {
            eprintln!("skipping real BSA test: game data not found");
            return;
        };

        let meshes = Archive::open(data.join("Skyrim - Meshes0.bsa")).unwrap();
        assert!(meshes.header().compressed());
        assert!(meshes.file_count() > 19_000);
        let path = "meshes\\dungeons\\nordic\\levers\\pullchainanim\\norpullchainanim01.nif";
        let index = meshes.find(path).expect("known mesh is missing");
        let bytes = meshes.read_file(index).unwrap();
        assert!(bytes.starts_with(b"Gamebryo File"));

        let textures = Archive::open(data.join("Skyrim - Textures0.bsa")).unwrap();
        assert!(textures.header().embedded_file_names());
        let index = textures
            .find("textures\\actors\\dog\\dog.dds")
            .expect("known texture is missing");
        let bytes = textures.read_file(index).unwrap();
        assert!(bytes.starts_with(b"DDS "));

        let misc = Archive::open(data.join("Skyrim - Misc.bsa")).unwrap();
        assert!(!misc.header().compressed());
        assert!(misc.file_count() > 14_000);

        let marketplace = data.join("MarketplaceTextures.bsa");
        if marketplace.exists() {
            let padded = Archive::open(&marketplace).unwrap();
            assert!(padded.file_count() > 0);
        }

        eprintln!(
            "real BSA check: {} mesh files, {} texture files, {} misc files",
            meshes.file_count(),
            textures.file_count(),
            misc.file_count()
        );
    }
}
