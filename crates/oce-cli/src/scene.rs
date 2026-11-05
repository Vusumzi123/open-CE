use std::collections::HashMap;
use std::error::Error;

use glam::{EulerRot, Quat, Vec3};
use half::f16;
use oce_core::Result;
use oce_data::Game;
use oce_plugin::{Plugin, Record, WalkEntry};

const TAMRIEL: u32 = 0x0000_003C;
const TARGET_CELL: u32 = 0x0000_99A2;

#[derive(Clone)]
pub struct MeshVertex {
    pub position: [f32; 3],
    pub normal: [f32; 3],
    pub uv: [f32; 2],
    pub color: [f32; 3],
}

pub struct SceneObject {
    pub vertices: Vec<MeshVertex>,
    pub indices: Vec<u32>,
    pub texture: Option<String>,
}

pub struct Scene {
    pub objects: Vec<SceneObject>,
    pub textures: HashMap<String, TextureData>,
    pub loaded_models: usize,
    pub failed_models: usize,
    pub failed_textures: usize,
}

pub struct TextureData {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

impl Scene {
    pub fn bounds(&self) -> Option<(Vec3, Vec3)> {
        let mut min = Vec3::splat(f32::INFINITY);
        let mut max = Vec3::splat(f32::NEG_INFINITY);
        let mut seen = false;
        for object in &self.objects {
            for vertex in &object.vertices {
                let position = Vec3::from_array(vertex.position);
                min = min.min(position);
                max = max.max(position);
                seen = true;
            }
        }
        seen.then_some((min, max))
    }

    pub fn load(game: &Game, plugin: &Plugin) -> Result<Self> {
        let refs = cell_refs(plugin)?;
        let wanted: std::collections::HashSet<u32> =
            refs.iter().map(|reference| reference.base).collect();
        let base_models = base_models(plugin, &wanted)?;
        let mut cache: HashMap<String, NifMesh> = HashMap::new();
        let mut objects = Vec::new();
        let mut loaded_models = 0;
        let mut failed_models = 0;

        for reference in refs {
            let Some(model) = base_models.get(&reference.base) else {
                continue;
            };
            if !keep_model(model) {
                continue;
            }
            let mesh = if let Some(mesh) = cache.get(model) {
                mesh.clone()
            } else {
                let path = format!("meshes\\{model}");
                let bytes = match game.read_asset(&path) {
                    Ok(Some(bytes)) => bytes,
                    _ => {
                        failed_models += 1;
                        continue;
                    }
                };
                let mesh = match parse_nif(&bytes) {
                    Ok(mesh) => mesh,
                    Err(_) => {
                        failed_models += 1;
                        continue;
                    }
                };
                loaded_models += 1;
                cache.insert(model.clone(), mesh.clone());
                mesh
            };
            if mesh.meshes.is_empty() {
                continue;
            }
            let rotation = Quat::from_euler(
                EulerRot::XYZ,
                reference.rotation[0],
                reference.rotation[1],
                reference.rotation[2],
            );
            let scale = reference.scale;
            for part in mesh.meshes {
                let mut vertices = part.vertices;
                for vertex in &mut vertices {
                    let position = rotation * (Vec3::from_array(vertex.position) * scale)
                        + Vec3::from_array(reference.position);
                    vertex.position = Vec3::new(position.x, position.z, position.y).to_array();
                    let normal = rotation * Vec3::from_array(vertex.normal);
                    vertex.normal = Vec3::new(normal.x, normal.z, normal.y).to_array();
                }
                objects.push(SceneObject {
                    vertices,
                    indices: part.indices,
                    texture: part.texture,
                });
            }
        }

        let mut textures = HashMap::new();
        let mut failed_textures = 0;
        for path in objects.iter().filter_map(|object| object.texture.as_ref()) {
            if textures.contains_key(path) {
                continue;
            }
            let Some(bytes) = game.read_asset(path)? else {
                failed_textures += 1;
                continue;
            };
            match image::load_from_memory_with_format(&bytes, image::ImageFormat::Dds) {
                Ok(image) => {
                    let image = image.to_rgba8();
                    textures.insert(
                        path.clone(),
                        TextureData {
                            width: image.width(),
                            height: image.height(),
                            rgba: image.into_raw(),
                        },
                    );
                }
                Err(_) => failed_textures += 1,
            }
        }

        Ok(Self {
            objects,
            textures,
            loaded_models,
            failed_models,
            failed_textures,
        })
    }
}

#[derive(Clone)]
struct NifMesh {
    meshes: Vec<MeshPart>,
}

#[derive(Clone)]
struct MeshPart {
    vertices: Vec<MeshVertex>,
    indices: Vec<u32>,
    texture: Option<String>,
}

struct Reference {
    base: u32,
    position: [f32; 3],
    rotation: [f32; 3],
    scale: f32,
}

#[derive(Clone, Copy)]
struct Transform {
    translation: Vec3,
    rotation: glam::Mat3,
    scale: f32,
}

impl Transform {
    const IDENTITY: Self = Self {
        translation: Vec3::ZERO,
        rotation: glam::Mat3::IDENTITY,
        scale: 1.0,
    };

    fn apply(&self, point: Vec3) -> Vec3 {
        self.translation + self.rotation * (point * self.scale)
    }

    fn then(&self, child: &Transform) -> Self {
        Self {
            translation: self.apply(child.translation),
            rotation: self.rotation * child.rotation,
            scale: self.scale * child.scale,
        }
    }
}

fn base_models(
    plugin: &Plugin,
    wanted: &std::collections::HashSet<u32>,
) -> Result<HashMap<u32, String>> {
    let mut models = HashMap::new();
    plugin.walk(|entry| {
        if let WalkEntry::Record {
            header,
            body,
            offset,
            ..
        } = entry
        {
            if wanted.contains(&header.form_id.raw())
                && matches!(
                    &header.signature,
                    b"STAT" | b"TREE" | b"FLOR" | b"CONT" | b"MSTT" | b"ACTI" | b"DOOR"
                )
            {
                let record = Record {
                    header,
                    body,
                    offset,
                };
                let decoded = record.decode()?;
                for subrecord in decoded.subrecords()? {
                    if &subrecord.signature == b"MODL" {
                        if let Some(model) = subrecord.as_cstring() {
                            if let Ok(model) = std::str::from_utf8(model) {
                                models.insert(header.form_id.raw(), model.to_string());
                            }
                        }
                        break;
                    }
                }
            }
        }
        Ok(())
    })?;
    Ok(models)
}

fn cell_refs(plugin: &Plugin) -> Result<Vec<Reference>> {
    let mut refs = Vec::new();
    let mut world = None;
    let mut in_cell = false;
    plugin.walk(|entry| {
        if let WalkEntry::Record {
            header,
            body,
            offset,
            ..
        } = entry
        {
            if &header.signature == b"WRLD" {
                world = Some(header.form_id.raw());
                in_cell = false;
            } else if &header.signature == b"CELL" {
                in_cell = world == Some(TAMRIEL) && header.form_id.raw() == TARGET_CELL;
            } else if &header.signature == b"REFR" && in_cell {
                let record = Record {
                    header,
                    body,
                    offset,
                };
                let decoded = record.decode()?;
                let mut base = None;
                let mut position = [0.0; 3];
                let mut rotation = [0.0; 3];
                let mut scale = 1.0;
                for subrecord in decoded.subrecords()? {
                    match &subrecord.signature {
                        b"NAME" if subrecord.data.len() >= 4 => {
                            base =
                                Some(u32::from_le_bytes(subrecord.data[..4].try_into().unwrap()));
                        }
                        b"DATA" if subrecord.data.len() >= 24 => {
                            position[0] =
                                f32::from_le_bytes(subrecord.data[0..4].try_into().unwrap());
                            position[1] =
                                f32::from_le_bytes(subrecord.data[4..8].try_into().unwrap());
                            position[2] =
                                f32::from_le_bytes(subrecord.data[8..12].try_into().unwrap());
                            rotation[0] =
                                f32::from_le_bytes(subrecord.data[12..16].try_into().unwrap());
                            rotation[1] =
                                f32::from_le_bytes(subrecord.data[16..20].try_into().unwrap());
                            rotation[2] =
                                f32::from_le_bytes(subrecord.data[20..24].try_into().unwrap());
                        }
                        b"XSCL" if subrecord.data.len() >= 4 => {
                            scale = f32::from_le_bytes(subrecord.data[..4].try_into().unwrap());
                        }
                        _ => {}
                    }
                }
                if let Some(base) = base {
                    refs.push(Reference {
                        base,
                        position,
                        rotation,
                        scale,
                    });
                }
            }
        }
        Ok(())
    })?;
    Ok(refs)
}

fn keep_model(model: &str) -> bool {
    let lower = model.to_ascii_lowercase();
    ![
        "marker",
        "sky\\",
        "water",
        "effects",
        "critters",
        "idle",
        "trigger",
        "sound",
        "light",
        "furniture",
        "actors",
        "magic",
        "trap",
        "portcullis",
        "door",
        "tree",
        "plants",
        "grass",
    ]
    .iter()
    .any(|part| lower.contains(part))
}

fn parse_nif(data: &[u8]) -> std::result::Result<NifMesh, Box<dyn Error>> {
    let mut cursor = Cursor::new(data);
    let line_end = data
        .iter()
        .position(|byte| *byte == b'\n')
        .ok_or("NIF header has no newline")?;
    cursor.pos = line_end + 1;
    let _version = cursor.u32()?;
    let _endian = cursor.u8()?;
    let user_version = cursor.u32()?;
    let block_count = cursor.u32()? as usize;
    let _user_version_2 = cursor.u32()?;
    if user_version >= 10 {
        cursor.short_string()?;
        cursor.short_string()?;
        cursor.short_string()?;
    }
    let block_type_count = cursor.u16()? as usize;
    let mut block_types = Vec::with_capacity(block_type_count);
    for _ in 0..block_type_count {
        block_types.push(cursor.string()?);
    }
    let mut block_type_indices = Vec::with_capacity(block_count);
    for _ in 0..block_count {
        block_type_indices.push(cursor.u16()? as usize);
    }
    let mut block_sizes = Vec::with_capacity(block_count);
    for _ in 0..block_count {
        block_sizes.push(cursor.u32()? as usize);
    }
    let string_count = cursor.u32()? as usize;
    let _max_string_length = cursor.u32()?;
    for _ in 0..string_count {
        let _ = cursor.string()?;
    }
    let _unknown = cursor.u32()?;

    let mut offsets = Vec::with_capacity(block_count);
    let mut data_start = cursor.pos;
    for size in &block_sizes {
        offsets.push(data_start);
        data_start += *size;
    }

    let mut texture_sets = vec![None; block_count];
    for (index, offset) in offsets.iter().enumerate() {
        if block_types[block_type_indices[index]] == "BSShaderTextureSet" {
            if let Ok(paths) = parse_texture_set(data, *offset) {
                texture_sets[index] = Some(paths);
            }
        }
    }

    let mut transforms = vec![Transform::IDENTITY; block_count];
    let mut visited = vec![false; block_count];
    walk_node(
        data,
        0,
        &offsets,
        &block_types,
        &block_type_indices,
        &Transform::IDENTITY,
        &mut transforms,
        &mut visited,
    );

    let mut meshes = Vec::new();
    for (index, offset) in offsets.iter().enumerate() {
        let block_type = block_types[block_type_indices[index]].as_str();
        if block_type == "BSTriShape" {
            if let Ok(part) = parse_shape(
                data,
                *offset,
                &texture_sets,
                &block_type_indices,
                &transforms[index],
            ) {
                meshes.push(part);
            }
        }
    }

    Ok(NifMesh { meshes })
}

#[allow(clippy::too_many_arguments)]
fn walk_node(
    data: &[u8],
    index: usize,
    offsets: &[usize],
    block_types: &[String],
    block_type_indices: &[usize],
    parent: &Transform,
    transforms: &mut [Transform],
    visited: &mut [bool],
) {
    if index >= offsets.len() || visited[index] {
        return;
    }
    visited[index] = true;
    let block_type = block_types[block_type_indices[index]].as_str();
    if !matches!(block_type, "NiNode" | "BSFadeNode" | "BSLeafAnimNode") {
        transforms[index] = *parent;
        return;
    }
    let Ok((local, children)) = parse_node(data, offsets[index]) else {
        transforms[index] = *parent;
        return;
    };
    let world = parent.then(&local);
    transforms[index] = world;
    for child in children {
        if child >= 0 {
            walk_node(
                data,
                child as usize,
                offsets,
                block_types,
                block_type_indices,
                &world,
                transforms,
                visited,
            );
        }
    }
}

fn parse_node(
    data: &[u8],
    offset: usize,
) -> std::result::Result<(Transform, Vec<i32>), Box<dyn Error>> {
    let mut cursor = Cursor::new(data);
    cursor.pos = offset;
    let _name = cursor.u32()?;
    let extra_count = cursor.u32()? as usize;
    for _ in 0..extra_count {
        let _ = cursor.u32()?;
    }
    let _controller = cursor.i32()?;
    let _flags = cursor.u32()?;
    let translation = cursor.vec3()?;
    let rotation = [
        cursor.f32()?,
        cursor.f32()?,
        cursor.f32()?,
        cursor.f32()?,
        cursor.f32()?,
        cursor.f32()?,
        cursor.f32()?,
        cursor.f32()?,
        cursor.f32()?,
    ];
    let scale = cursor.f32()?;
    let _collision = cursor.i32()?;
    let child_count = cursor.u32()? as usize;
    let mut children = Vec::with_capacity(child_count);
    for _ in 0..child_count {
        children.push(cursor.i32()?);
    }
    let rotation = glam::Mat3::from_cols(
        Vec3::new(rotation[0], rotation[3], rotation[6]),
        Vec3::new(rotation[1], rotation[4], rotation[7]),
        Vec3::new(rotation[2], rotation[5], rotation[8]),
    );
    Ok((
        Transform {
            translation,
            rotation,
            scale,
        },
        children,
    ))
}

fn parse_shape(
    data: &[u8],
    offset: usize,
    texture_sets: &[Option<Vec<String>>],
    block_type_indices: &[usize],
    parent: &Transform,
) -> std::result::Result<MeshPart, Box<dyn Error>> {
    let mut cursor = Cursor::new(data);
    cursor.pos = offset;
    let _name = cursor.u32()?;
    let extra_count = cursor.u32()? as usize;
    for _ in 0..extra_count {
        let _ = cursor.u32()?;
    }
    let _controller = cursor.i32()?;
    let _flags = cursor.u32()?;
    let translation = cursor.vec3()?;
    let rotation = [
        cursor.f32()?,
        cursor.f32()?,
        cursor.f32()?,
        cursor.f32()?,
        cursor.f32()?,
        cursor.f32()?,
        cursor.f32()?,
        cursor.f32()?,
        cursor.f32()?,
    ];
    let scale = cursor.f32()?;
    let _collision = cursor.i32()?;
    for _ in 0..4 {
        let _ = cursor.f32()?;
    }
    let _skin = cursor.i32()?;
    let shader = cursor.i32()?;
    let _alpha = cursor.i32()?;
    let _vertex_desc = cursor.u64()?;
    let triangles = cursor.u16()? as usize;
    let vertices = cursor.u16()? as usize;
    let data_size = cursor.u32()? as usize;
    if vertices == 0 || triangles == 0 || data_size < triangles * 6 {
        return Err("empty BSTriShape".into());
    }
    let vertex_bytes = data_size - triangles * 6;
    if vertex_bytes / vertices * vertices != vertex_bytes {
        return Err("BSTriShape stride is not integral".into());
    }
    let stride = vertex_bytes / vertices;
    if !(28..=96).contains(&stride) {
        return Err("unsupported BSTriShape stride".into());
    }
    let texture = texture_for(shader, texture_sets, block_type_indices);
    let vertex_start = cursor.pos;
    let mut out = Vec::with_capacity(vertices);
    let rotation = glam::Mat3::from_cols(
        Vec3::new(rotation[0], rotation[3], rotation[6]),
        Vec3::new(rotation[1], rotation[4], rotation[7]),
        Vec3::new(rotation[2], rotation[5], rotation[8]),
    );
    for index in 0..vertices {
        let base = vertex_start + index * stride;
        if base + 28 > data.len() {
            return Err("BSTriShape vertex data outside file".into());
        }
        let position = Vec3::new(
            read_f32(data, base)?,
            read_f32(data, base + 4)?,
            read_f32(data, base + 8)?,
        );
        let uv = [
            f16::from_bits(read_u16(data, base + 16)?).to_f32(),
            f16::from_bits(read_u16(data, base + 18)?).to_f32(),
        ];
        let normal = decode_normal(data, base + 20)?;
        let color = if stride >= 32 {
            [
                data[base + 28] as f32 / 255.0,
                data[base + 29] as f32 / 255.0,
                data[base + 30] as f32 / 255.0,
            ]
        } else {
            [1.0, 1.0, 1.0]
        };
        let local = Transform {
            translation,
            rotation,
            scale,
        };
        out.push(MeshVertex {
            position: parent.apply(local.apply(position)).to_array(),
            normal: (parent.rotation * rotation * normal).to_array(),
            uv,
            color,
        });
    }
    let index_start = vertex_start + vertices * stride;
    if index_start + triangles * 6 > data.len() {
        return Err("BSTriShape index data outside file".into());
    }
    let mut indices = Vec::with_capacity(triangles * 3);
    for index in 0..triangles * 3 {
        indices.push(read_u16(data, index_start + index * 2)? as u32);
    }
    Ok(MeshPart {
        vertices: out,
        indices,
        texture,
    })
}

fn texture_for(
    shader: i32,
    texture_sets: &[Option<Vec<String>>],
    block_type_indices: &[usize],
) -> Option<String> {
    if shader >= 0 {
        let shader = shader as usize;
        if shader + 1 < texture_sets.len()
            && block_type_indices.get(shader + 1).is_some()
            && texture_sets[shader + 1].is_some()
        {
            if let Some(paths) = &texture_sets[shader + 1] {
                return paths.first().filter(|path| !path.is_empty()).cloned();
            }
        }
    }
    texture_sets
        .iter()
        .flatten()
        .flat_map(|paths| paths.iter())
        .find(|path| !path.is_empty())
        .cloned()
}

fn parse_texture_set(
    data: &[u8],
    offset: usize,
) -> std::result::Result<Vec<String>, Box<dyn Error>> {
    let mut cursor = Cursor::new(data);
    cursor.pos = offset;
    let count = cursor.u32()? as usize;
    let mut paths = Vec::with_capacity(count.min(16));
    for _ in 0..count {
        paths.push(cursor.string()?);
    }
    Ok(paths)
}

fn read_u16(data: &[u8], offset: usize) -> std::result::Result<u16, Box<dyn Error>> {
    let bytes = data.get(offset..offset + 2).ok_or("read past end")?;
    Ok(u16::from_le_bytes(bytes.try_into().unwrap()))
}

fn read_f32(data: &[u8], offset: usize) -> std::result::Result<f32, Box<dyn Error>> {
    let bytes = data.get(offset..offset + 4).ok_or("read past end")?;
    Ok(f32::from_le_bytes(bytes.try_into().unwrap()))
}

fn decode_normal(data: &[u8], offset: usize) -> std::result::Result<Vec3, Box<dyn Error>> {
    let bytes = data.get(offset..offset + 4).ok_or("read past end")?;
    let normal = Vec3::new(
        bytes[0] as f32 / 255.0 * 2.0 - 1.0,
        bytes[1] as f32 / 255.0 * 2.0 - 1.0,
        bytes[2] as f32 / 255.0 * 2.0 - 1.0,
    );
    Ok(normal.normalize_or_zero())
}

struct Cursor<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Cursor<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0 }
    }
    fn bytes(&mut self, count: usize) -> std::result::Result<&'a [u8], Box<dyn Error>> {
        let bytes = self
            .data
            .get(self.pos..self.pos + count)
            .ok_or("read past end")?;
        self.pos += count;
        Ok(bytes)
    }
    fn u8(&mut self) -> std::result::Result<u8, Box<dyn Error>> {
        Ok(self.bytes(1)?[0])
    }
    fn u16(&mut self) -> std::result::Result<u16, Box<dyn Error>> {
        Ok(u16::from_le_bytes(self.bytes(2)?.try_into().unwrap()))
    }
    fn i32(&mut self) -> std::result::Result<i32, Box<dyn Error>> {
        Ok(i32::from_le_bytes(self.bytes(4)?.try_into().unwrap()))
    }
    fn u32(&mut self) -> std::result::Result<u32, Box<dyn Error>> {
        Ok(u32::from_le_bytes(self.bytes(4)?.try_into().unwrap()))
    }
    fn u64(&mut self) -> std::result::Result<u64, Box<dyn Error>> {
        Ok(u64::from_le_bytes(self.bytes(8)?.try_into().unwrap()))
    }
    fn f32(&mut self) -> std::result::Result<f32, Box<dyn Error>> {
        Ok(f32::from_le_bytes(self.bytes(4)?.try_into().unwrap()))
    }
    fn vec3(&mut self) -> std::result::Result<Vec3, Box<dyn Error>> {
        Ok(Vec3::new(self.f32()?, self.f32()?, self.f32()?))
    }
    fn string(&mut self) -> std::result::Result<String, Box<dyn Error>> {
        let length = self.u32()? as usize;
        let bytes = self.bytes(length)?;
        Ok(String::from_utf8_lossy(bytes).into_owned())
    }
    fn short_string(&mut self) -> std::result::Result<String, Box<dyn Error>> {
        let length = self.u8()? as usize;
        let bytes = self.bytes(length)?;
        Ok(String::from_utf8_lossy(bytes).into_owned())
    }
}
