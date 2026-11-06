use std::fmt;

const MAGIC: &[u8; 4] = b"DDS ";
const HEADER_SIZE: usize = 124;
const PIXEL_FORMAT_OFFSET: usize = 76;
const PIXEL_FORMAT_SIZE: usize = 32;
const DX10_OFFSET: usize = 128;
const DX10_SIZE: usize = 20;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DdsFormat {
    Bc1,
    Bc1Srgb,
    Bc2,
    Bc2Srgb,
    Bc3,
    Bc3Srgb,
    Bc4,
    Bc4Snorm,
    Bc5,
    Bc5Snorm,
    Bc7,
    Bc7Srgb,
    Other(u32),
}

impl DdsFormat {
    pub fn is_bc5(self) -> bool {
        matches!(self, Self::Bc5 | Self::Bc5Snorm)
    }

    pub fn is_bc7(self) -> bool {
        matches!(self, Self::Bc7 | Self::Bc7Srgb)
    }

    pub fn block_bytes(self) -> Option<u32> {
        match self {
            Self::Bc1 | Self::Bc1Srgb | Self::Bc4 | Self::Bc4Snorm => Some(8),
            Self::Bc2
            | Self::Bc2Srgb
            | Self::Bc3
            | Self::Bc3Srgb
            | Self::Bc5
            | Self::Bc5Snorm
            | Self::Bc7
            | Self::Bc7Srgb => Some(16),
            Self::Other(_) => None,
        }
    }
}

impl fmt::Display for DdsFormat {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Bc1 => write!(f, "BC1"),
            Self::Bc1Srgb => write!(f, "BC1 sRGB"),
            Self::Bc2 => write!(f, "BC2"),
            Self::Bc2Srgb => write!(f, "BC2 sRGB"),
            Self::Bc3 => write!(f, "BC3"),
            Self::Bc3Srgb => write!(f, "BC3 sRGB"),
            Self::Bc4 => write!(f, "BC4"),
            Self::Bc4Snorm => write!(f, "BC4 SNORM"),
            Self::Bc5 => write!(f, "BC5"),
            Self::Bc5Snorm => write!(f, "BC5 SNORM"),
            Self::Bc7 => write!(f, "BC7"),
            Self::Bc7Srgb => write!(f, "BC7 sRGB"),
            Self::Other(value) => write!(f, "DXGI {value}"),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DdsInfo {
    pub width: u32,
    pub height: u32,
    pub mip_count: u32,
    pub format: DdsFormat,
    pub data_offset: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CompressedTexture {
    pub width: u32,
    pub height: u32,
    pub format: DdsFormat,
    pub data: Vec<u8>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DdsError {
    TooShort { length: usize, needed: usize },
    BadMagic,
    BadHeaderSize(u32),
    BadPixelFormatSize(u32),
    BadDimensions { width: u32, height: u32 },
    Cubemap,
    Volume,
    Array { size: u32 },
    NotTexture2d(u32),
    Truncated { expected: usize, actual: usize },
}

impl fmt::Display for DdsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooShort { length, needed } => {
                write!(f, "DDS is {length} bytes; need at least {needed}")
            }
            Self::BadMagic => write!(f, "not a DDS file"),
            Self::BadHeaderSize(size) => write!(f, "unexpected DDS header size {size}"),
            Self::BadPixelFormatSize(size) => {
                write!(f, "unexpected DDS pixel format size {size}")
            }
            Self::BadDimensions { width, height } => {
                write!(f, "invalid DDS dimensions {width}x{height}")
            }
            Self::Cubemap => write!(f, "DDS cubemaps are not supported"),
            Self::Volume => write!(f, "DDS volume textures are not supported"),
            Self::Array { size } => write!(f, "DDS array size {size} is not supported"),
            Self::NotTexture2d(dimension) => {
                write!(f, "DDS resource dimension {dimension} is not 2D")
            }
            Self::Truncated { expected, actual } => {
                write!(f, "DDS payload is {actual} bytes; need {expected}")
            }
        }
    }
}

impl std::error::Error for DdsError {}

pub fn parse(bytes: &[u8]) -> Result<DdsInfo, DdsError> {
    if bytes.len() < DX10_OFFSET {
        return Err(DdsError::TooShort {
            length: bytes.len(),
            needed: DX10_OFFSET,
        });
    }
    if &bytes[0..4] != MAGIC {
        return Err(DdsError::BadMagic);
    }
    let header_size = read_u32(bytes, 4);
    if header_size != HEADER_SIZE as u32 {
        return Err(DdsError::BadHeaderSize(header_size));
    }
    let pixel_format_size = read_u32(bytes, PIXEL_FORMAT_OFFSET);
    if pixel_format_size != PIXEL_FORMAT_SIZE as u32 {
        return Err(DdsError::BadPixelFormatSize(pixel_format_size));
    }

    let width = read_u32(bytes, 16);
    let height = read_u32(bytes, 12);
    if width == 0 || height == 0 || width > 16384 || height > 16384 {
        return Err(DdsError::BadDimensions { width, height });
    }

    let mip_count = read_u32(bytes, 28).max(1);
    let caps2 = read_u32(bytes, 112);
    if caps2 & 0x200 != 0 {
        return Err(DdsError::Cubemap);
    }
    if caps2 & 0x0020_0000 != 0 {
        return Err(DdsError::Volume);
    }

    let fourcc = &bytes[84..88];
    let mut data_offset = DX10_OFFSET;
    let format = if fourcc == b"DX10" {
        if bytes.len() < DX10_OFFSET + DX10_SIZE {
            return Err(DdsError::TooShort {
                length: bytes.len(),
                needed: DX10_OFFSET + DX10_SIZE,
            });
        }
        data_offset = DX10_OFFSET + DX10_SIZE;
        let dxgi_format = read_u32(bytes, DX10_OFFSET);
        let resource_dimension = read_u32(bytes, DX10_OFFSET + 4);
        let misc_flag = read_u32(bytes, DX10_OFFSET + 8);
        let array_size = read_u32(bytes, DX10_OFFSET + 12);
        if resource_dimension != 3 {
            return Err(DdsError::NotTexture2d(resource_dimension));
        }
        if misc_flag & 0x4 != 0 {
            return Err(DdsError::Cubemap);
        }
        if array_size > 1 {
            return Err(DdsError::Array { size: array_size });
        }
        format_from_dxgi(dxgi_format)
    } else if fourcc == b"DXT1" {
        DdsFormat::Bc1
    } else if fourcc == b"DXT3" {
        DdsFormat::Bc2
    } else if fourcc == b"DXT5" {
        DdsFormat::Bc3
    } else if fourcc == b"ATI2" || fourcc == b"BC5U" {
        DdsFormat::Bc5
    } else if fourcc == b"BC4U" {
        DdsFormat::Bc4
    } else {
        DdsFormat::Other(0)
    };

    Ok(DdsInfo {
        width,
        height,
        mip_count,
        format,
        data_offset,
    })
}

pub fn compressed(bytes: &[u8]) -> Result<Option<CompressedTexture>, DdsError> {
    let info = parse(bytes)?;
    if !info.format.is_bc5() && !info.format.is_bc7() {
        return Ok(None);
    }
    let block_bytes = info.format.block_bytes().unwrap() as usize;
    let blocks_x = info.width.div_ceil(4) as usize;
    let blocks_y = info.height.div_ceil(4) as usize;
    let length = blocks_x * blocks_y * block_bytes;
    let expected = info.data_offset + length;
    if bytes.len() < expected {
        return Err(DdsError::Truncated {
            expected,
            actual: bytes.len(),
        });
    }
    Ok(Some(CompressedTexture {
        width: info.width,
        height: info.height,
        format: info.format,
        data: bytes[info.data_offset..expected].to_vec(),
    }))
}

fn format_from_dxgi(value: u32) -> DdsFormat {
    match value {
        70 | 71 => DdsFormat::Bc1,
        72 => DdsFormat::Bc1Srgb,
        73 | 74 => DdsFormat::Bc2,
        75 => DdsFormat::Bc2Srgb,
        76 | 77 => DdsFormat::Bc3,
        78 => DdsFormat::Bc3Srgb,
        79 | 80 => DdsFormat::Bc4,
        81 => DdsFormat::Bc4Snorm,
        82 | 83 => DdsFormat::Bc5,
        84 => DdsFormat::Bc5Snorm,
        97 | 98 => DdsFormat::Bc7,
        99 => DdsFormat::Bc7Srgb,
        other => DdsFormat::Other(other),
    }
}

fn read_u32(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn header(fourcc: [u8; 4], dxgi: Option<u32>, width: u32, height: u32) -> Vec<u8> {
        let mut bytes = vec![0u8; 148];
        bytes[0..4].copy_from_slice(MAGIC);
        write_u32(&mut bytes, 4, 124);
        write_u32(&mut bytes, 8, 0x1007);
        write_u32(&mut bytes, 12, height);
        write_u32(&mut bytes, 16, width);
        write_u32(&mut bytes, 28, 1);
        write_u32(&mut bytes, 76, 32);
        write_u32(&mut bytes, 80, 0x4);
        bytes[84..88].copy_from_slice(&fourcc);
        write_u32(&mut bytes, 108, 0x1000);
        if let Some(dxgi) = dxgi {
            write_u32(&mut bytes, DX10_OFFSET, dxgi);
            write_u32(&mut bytes, DX10_OFFSET + 4, 3);
            write_u32(&mut bytes, DX10_OFFSET + 12, 1);
        }
        bytes
    }

    fn write_u32(bytes: &mut [u8], offset: usize, value: u32) {
        bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
    }

    #[test]
    fn parses_bc5_dx10() {
        let bytes = header(*b"DX10", Some(83), 8, 8);
        let info = parse(&bytes).unwrap();
        assert_eq!(info.format, DdsFormat::Bc5);
        assert_eq!(info.width, 8);
        assert_eq!(info.height, 8);
        assert_eq!(info.data_offset, 148);
    }

    #[test]
    fn parses_bc7_dx10_srgb() {
        let bytes = header(*b"DX10", Some(99), 5, 7);
        let info = parse(&bytes).unwrap();
        assert_eq!(info.format, DdsFormat::Bc7Srgb);
        assert_eq!(info.width, 5);
        assert_eq!(info.height, 7);
    }

    #[test]
    fn parses_legacy_dxt5() {
        let bytes = header(*b"DXT5", None, 4, 4);
        let info = parse(&bytes).unwrap();
        assert_eq!(info.format, DdsFormat::Bc3);
        assert_eq!(info.data_offset, 128);
    }

    #[test]
    fn extracts_top_mip_of_bc5() {
        let mut bytes = header(*b"DX10", Some(83), 4, 4);
        bytes.extend(vec![0xCD; 16]);
        let texture = compressed(&bytes).unwrap().unwrap();
        assert_eq!(texture.format, DdsFormat::Bc5);
        assert_eq!(texture.data.len(), 16);
    }

    #[test]
    fn extracts_top_mip_of_bc7() {
        let mut bytes = header(*b"DX10", Some(98), 5, 7);
        bytes.extend(vec![0xAB; 4 * 4 * 16]);
        let texture = compressed(&bytes).unwrap().unwrap();
        assert_eq!(texture.format, DdsFormat::Bc7);
        assert_eq!(texture.data.len(), 2 * 2 * 16);
    }

    #[test]
    fn leaves_other_formats_to_the_image_crate() {
        let bytes = header(*b"DXT1", None, 4, 4);
        assert!(compressed(&bytes).unwrap().is_none());
    }

    #[test]
    fn rejects_short_payload() {
        let mut bytes = header(*b"DX10", Some(98), 4, 4);
        bytes.truncate(160);
        assert!(matches!(
            compressed(&bytes),
            Err(DdsError::Truncated { .. })
        ));
    }

    #[test]
    fn rejects_bad_magic() {
        let mut bytes = header(*b"DX10", Some(98), 4, 4);
        bytes[0] = b'X';
        assert_eq!(parse(&bytes), Err(DdsError::BadMagic));
    }

    #[test]
    fn rejects_cubemap() {
        let mut bytes = header(*b"DX10", Some(98), 4, 4);
        write_u32(&mut bytes, 112, 0x200);
        assert_eq!(parse(&bytes), Err(DdsError::Cubemap));
    }

    #[test]
    fn rejects_array() {
        let mut bytes = header(*b"DX10", Some(98), 4, 4);
        write_u32(&mut bytes, DX10_OFFSET + 12, 2);
        assert_eq!(parse(&bytes), Err(DdsError::Array { size: 2 }));
    }
}
