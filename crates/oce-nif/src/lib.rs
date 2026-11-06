use std::collections::{BTreeMap, HashMap};

use glam::{Mat3, Vec3};
use half::f16;
use oce_core::{Error, Result};

pub const SSE_VERSION: u32 = 0x1402_0007;

const NODE_TYPES: &[&str] = &[
    "NiNode",
    "BSFadeNode",
    "BSLeafAnimNode",
    "BSTreeNode",
    "NiSwitchNode",
    "BSMultiBoundNode",
    "NiLODNode",
    "NiBillboardNode",
    "NiBSParticleNode",
    "NiParticleSystem",
    "BSFaceGenNiNode",
    "BSFurnitureMarkerNode",
    "BSOrderedNode",
    "BSValueNode",
];

const NON_NODE_TYPES: &[&str] = &[
    "BSXFlags",
    "BSMultiBound",
    "BSMultiBoundOBB",
    "BSMultiBoundAABB",
    "BSMultiBoundSphere",
    "BSShaderTextureSet",
    "BSLightingShaderProperty",
    "BSShaderPPLightingProperty",
    "BSShaderNoLightingProperty",
    "BSShaderProperty",
    "NiAlphaProperty",
    "NiMaterialProperty",
    "NiTexturingProperty",
    "NiSourceTexture",
    "NiVertexColorProperty",
    "NiZBufferProperty",
    "NiSpecularProperty",
    "NiStencilProperty",
    "NiWireframeProperty",
    "NiShadeProperty",
    "NiDitherProperty",
    "NiFogProperty",
    "NiRendererProperty",
    "NiStringExtraData",
    "NiIntegerExtraData",
    "NiFloatExtraData",
    "NiBooleanExtraData",
    "NiBinaryExtraData",
    "NiTextKeyExtraData",
    "NiVectorExtraData",
    "NiColorExtraData",
    "NiExtraData",
    "BSTriShape",
    "BSSubIndexTriShape",
    "BSDynamicTriShape",
    "BSMeshLODTriShape",
    "BSLODTriShape",
    "NiTriShape",
    "NiTriShapeData",
    "NiTriStrips",
    "NiTriStripsData",
    "NiSkinInstance",
    "NiSkinData",
    "NiSkinPartition",
    "NiSkinBone",
    "NiSkinDataBoneData",
    "bhkCapsuleShape",
    "bhkListShape",
    "bhkRigidBody",
    "bhkCollisionObject",
    "bhkBoxShape",
    "bhkSphereShape",
    "bhkConvexVerticesShape",
    "bhkMoppBvTreeShape",
    "bhkTransformShape",
    "bhkNiTriStripsShape",
    "bhkPackedNiTriStripsData",
    "BSBehaviorGraphExtraData",
    "BSDismemberSkinInstance",
    "BSEffectShaderProperty",
    "BSEffectShaderPropertyFloatController",
    "BSInvMarker",
    "BSLightingShaderPropertyColorController",
    "BSLightingShaderPropertyFloatController",
    "BSPSysLODModifier",
    "BSPSysScaleModifier",
    "BSPSysSimpleColorModifier",
    "BSPSysSubTexModifier",
    "NiBlendBoolInterpolator",
    "NiBlendFloatInterpolator",
    "NiBoolData",
    "NiBoolInterpolator",
    "NiBoolTimelineInterpolator",
    "NiControllerManager",
    "NiControllerSequence",
    "NiDefaultAVObjectPalette",
    "NiFloatData",
    "NiFloatInterpolator",
    "NiMultiTargetTransformController",
    "NiPSysAgeDeathModifier",
    "NiPSysBombModifier",
    "NiPSysBoundUpdateModifier",
    "NiPSysBoxEmitter",
    "NiPSysCylinderEmitter",
    "NiPSysData",
    "NiPSysDragModifier",
    "NiPSysEmitterCtlr",
    "NiPSysGravityModifier",
    "NiPSysModifierActiveCtlr",
    "NiPSysPositionModifier",
    "NiPSysRotationModifier",
    "NiPSysSpawnModifier",
    "NiPSysUpdateCtlr",
    "NiPoint3Interpolator",
    "NiPosData",
    "NiTransformController",
    "NiTransformData",
    "NiTransformInterpolator",
    "NiVisController",
    "bhkCompressedMeshShape",
    "bhkCompressedMeshShapeData",
    "bhkConvexTransformShape",
    "bhkCylinderShape",
    "bhkHingeConstraint",
    "bhkRigidBodyT",
    "bhkSPCollisionObject",
    "bhkSimpleShapePhantom",
];

#[derive(Clone, Debug)]
pub struct Block {
    pub type_name: String,
    pub offset: usize,
    pub size: usize,
}

#[derive(Clone, Debug)]
pub struct Node {
    pub translation: Vec3,
    pub rotation: Mat3,
    pub scale: f32,
    pub children: Vec<i32>,
}

#[derive(Clone, Debug)]
pub struct MeshVertex {
    pub position: [f32; 3],
    pub normal: [f32; 3],
    pub uv: [f32; 2],
    pub color: [f32; 3],
}

#[derive(Clone, Debug)]
pub struct MeshPart {
    pub vertices: Vec<MeshVertex>,
    pub indices: Vec<u32>,
    pub texture: Option<String>,
    pub alpha_flags: u16,
}

#[derive(Clone, Debug)]
pub struct SkinPartitionSummary {
    pub vertices: usize,
    pub triangles: usize,
}

#[derive(Clone, Debug)]
pub struct SkinSummary {
    pub shape: usize,
    pub bones: usize,
    pub dismember: usize,
    pub has_vertex_weights: bool,
    pub resolved_bones: usize,
    pub descriptor: u64,
    pub vertex_size: usize,
    pub vertices: usize,
    pub triangles: usize,
    pub weights_per_vertex: usize,
    pub partitions: Vec<SkinPartitionSummary>,
    pub bind_deviation: f32,
}

#[derive(Clone, Copy, Debug)]
struct Transform {
    translation: Vec3,
    rotation: Mat3,
    scale: f32,
}

impl Transform {
    const IDENTITY: Self = Self {
        translation: Vec3::ZERO,
        rotation: Mat3::IDENTITY,
        scale: 1.0,
    };

    fn apply(self, point: Vec3) -> Vec3 {
        self.translation + self.rotation * (point * self.scale)
    }

    fn then(self, child: Self) -> Self {
        Self {
            translation: self.apply(child.translation),
            rotation: self.rotation * child.rotation,
            scale: self.scale * child.scale,
        }
    }
}

#[derive(Clone, Debug)]
struct SkinInstance {
    data: usize,
    partition: usize,
    bones: Vec<usize>,
    dismember: usize,
}

#[derive(Clone, Debug)]
struct SkinData {
    has_vertex_weights: bool,
    bones: Vec<Transform>,
}

#[derive(Clone, Copy, Debug)]
struct VertexLayout {
    size: usize,
    uv: usize,
    normal: usize,
    tangent: usize,
    color: usize,
    skin: usize,
    flags: u16,
}

#[derive(Clone, Copy, Debug)]
struct SkinVertex {
    position: Vec3,
    normal: Option<Vec3>,
    uv: [f32; 2],
    color: [f32; 3],
}

#[derive(Clone, Debug)]
struct SkinPartition {
    vertices: usize,
    triangles: usize,
    bones: Vec<u16>,
    vertex_map: Option<Vec<u16>>,
    weights_per_vertex: usize,
    weights: Vec<f32>,
    bone_indices: Option<Vec<u8>>,
    faces: Vec<[u16; 3]>,
}

#[derive(Clone, Debug)]
struct SkinPartitionData {
    descriptor: u64,
    vertex_size: usize,
    vertices: Vec<SkinVertex>,
    partitions: Vec<SkinPartition>,
}

struct SkinnedShape {
    part: MeshPart,
    summary: SkinSummary,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BlockRole {
    Node,
    NonNode,
    Unknown,
}

pub fn classify(type_name: &str) -> BlockRole {
    if NODE_TYPES.contains(&type_name) {
        BlockRole::Node
    } else if NON_NODE_TYPES.contains(&type_name) {
        BlockRole::NonNode
    } else {
        BlockRole::Unknown
    }
}

pub struct Nif {
    data: Vec<u8>,
    version: u32,
    user_version: u32,
    strings: Vec<String>,
    blocks: Vec<Block>,
}

impl Nif {
    pub fn parse(data: &[u8]) -> Result<Self> {
        let mut cursor = Cursor::new(data);
        let line_end = data
            .iter()
            .position(|byte| *byte == b'\n')
            .ok_or_else(|| Error::invalid("NIF header has no newline"))?;
        cursor.pos = line_end + 1;

        let version = cursor.u32()?;
        let endian = cursor.u8()?;
        if endian != 1 {
            return Err(Error::unsupported(format!(
                "NIF endian marker {endian} is not little-endian"
            )));
        }
        let user_version = cursor.u32()?;
        let block_count = cursor.u32()? as usize;
        if block_count > 1_000_000 {
            return Err(Error::invalid("NIF block count is unreasonable"));
        }
        if user_version >= 10 {
            let _bs_version = cursor.u32()?;
            for _ in 0..3 {
                let _ = cursor.short_string()?;
            }
        }

        let block_type_count = cursor.u16()? as usize;
        if block_type_count > 4096 {
            return Err(Error::invalid("NIF block type count is unreasonable"));
        }
        let mut block_types = Vec::with_capacity(block_type_count);
        for _ in 0..block_type_count {
            block_types.push(cursor.string()?.trim_end_matches('\0').to_string());
        }

        let mut block_type_indices = Vec::with_capacity(block_count);
        for _ in 0..block_count {
            let index = cursor.u16()? as usize;
            if index >= block_type_count {
                return Err(Error::invalid("NIF block type index is out of range"));
            }
            block_type_indices.push(index);
        }

        let mut block_sizes = Vec::with_capacity(block_count);
        for _ in 0..block_count {
            block_sizes.push(cursor.u32()? as usize);
        }

        let string_count = cursor.u32()? as usize;
        if string_count > 1_000_000 {
            return Err(Error::invalid("NIF string count is unreasonable"));
        }
        let _max_string_length = cursor.u32()?;
        let mut strings = Vec::with_capacity(string_count);
        for _ in 0..string_count {
            strings.push(cursor.string()?.trim_end_matches('\0').to_string());
        }
        let _groups = cursor.u32()?;

        let mut offset = cursor.pos;
        let mut blocks = Vec::with_capacity(block_count);
        for (index, size) in block_sizes.into_iter().enumerate() {
            let end = offset
                .checked_add(size)
                .ok_or_else(|| Error::invalid("NIF block range overflows"))?;
            if end > data.len() {
                return Err(Error::invalid("NIF block extends past the file"));
            }
            blocks.push(Block {
                type_name: block_types[block_type_indices[index]].clone(),
                offset,
                size,
            });
            offset = end;
        }

        Ok(Self {
            data: data.to_vec(),
            version,
            user_version,
            strings,
            blocks,
        })
    }

    pub fn version(&self) -> u32 {
        self.version
    }

    pub fn user_version(&self) -> u32 {
        self.user_version
    }

    pub fn strings(&self) -> &[String] {
        &self.strings
    }

    pub fn blocks(&self) -> &[Block] {
        &self.blocks
    }

    pub fn block_type(&self, index: usize) -> Option<&str> {
        self.blocks.get(index).map(|block| block.type_name.as_str())
    }

    pub fn histogram(&self) -> BTreeMap<String, usize> {
        let mut counts = BTreeMap::new();
        for block in &self.blocks {
            *counts.entry(block.type_name.clone()).or_insert(0) += 1;
        }
        counts
    }

    pub fn node(&self, index: usize) -> Result<Option<Node>> {
        let Some(block) = self.blocks.get(index) else {
            return Ok(None);
        };
        if !NODE_TYPES.contains(&block.type_name.as_str()) {
            return Ok(None);
        }

        let mut cursor = Cursor::at(&self.data, block.offset);
        let _name = cursor.u32()?;
        let extra_count = cursor.u32()? as usize;
        if extra_count > 4096 {
            return Err(Error::invalid("NIF node extra-data count is unreasonable"));
        }
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
        if child_count > 65536 {
            return Err(Error::invalid("NIF node child count is unreasonable"));
        }
        let mut children = Vec::with_capacity(child_count);
        for _ in 0..child_count {
            children.push(cursor.i32()?);
        }

        Ok(Some(Node {
            translation,
            rotation: Mat3::from_cols(
                Vec3::new(rotation[0], rotation[3], rotation[6]),
                Vec3::new(rotation[1], rotation[4], rotation[7]),
                Vec3::new(rotation[2], rotation[5], rotation[8]),
            ),
            scale,
            children,
        }))
    }

    pub fn texture_for_shader(&self, shader: i32) -> Option<String> {
        let shader = usize::try_from(shader).ok()?;
        let block = self.blocks.get(shader)?;
        if block.type_name != "BSLightingShaderProperty" {
            return None;
        }

        let ref_offset = block.offset.checked_add(40)?;
        let bytes = self.data.get(ref_offset..ref_offset + 4)?;
        let texture_set = i32::from_le_bytes(bytes.try_into().ok()?);
        let texture_set = usize::try_from(texture_set).ok()?;
        self.texture_set(texture_set)
            .and_then(|paths| paths.into_iter().find(|path| !path.is_empty()))
    }

    pub fn texture_for_shape(&self, shape: usize) -> Option<String> {
        self.shape_shader(shape)
            .and_then(|shader| self.texture_for_shader(shader))
    }

    pub fn alpha_for_shape(&self, shape: usize) -> u16 {
        self.shape_alpha(shape)
            .map(|alpha| self.alpha_flags(alpha))
            .unwrap_or(0)
    }

    fn texture_set(&self, index: usize) -> Option<Vec<String>> {
        let block = self.blocks.get(index)?;
        if block.type_name != "BSShaderTextureSet" {
            return None;
        }
        let mut cursor = Cursor::at(&self.data, block.offset);
        let count = cursor.u32().ok()? as usize;
        if count > 4096 {
            return None;
        }
        let mut paths = Vec::with_capacity(count);
        for _ in 0..count {
            paths.push(cursor.string().ok()?);
        }
        Some(paths)
    }

    fn alpha_flags(&self, alpha: i32) -> u16 {
        let Ok(index) = usize::try_from(alpha) else {
            return 0;
        };
        let Some(block) = self.blocks.get(index) else {
            return 0;
        };
        if block.type_name != "NiAlphaProperty" {
            return 0;
        }
        let Some(offset) = block.offset.checked_add(12) else {
            return 0;
        };
        let Some(bytes) = self.data.get(offset..offset + 2) else {
            return 0;
        };
        u16::from_le_bytes(bytes.try_into().unwrap())
    }

    fn shape_shader(&self, shape: usize) -> Option<i32> {
        let block = self.blocks.get(shape)?;
        if block.type_name != "BSTriShape" {
            return None;
        }
        let mut cursor = Cursor::at(&self.data, block.offset);
        skip_av_object(&mut cursor).ok()?;
        skip_av_transform(&mut cursor).ok()?;
        for _ in 0..4 {
            let _ = cursor.f32().ok()?;
        }
        let _skin = cursor.i32().ok()?;
        cursor.i32().ok()
    }

    fn shape_alpha(&self, shape: usize) -> Option<i32> {
        let block = self.blocks.get(shape)?;
        if block.type_name != "BSTriShape" {
            return None;
        }
        let mut cursor = Cursor::at(&self.data, block.offset);
        skip_av_object(&mut cursor).ok()?;
        skip_av_transform(&mut cursor).ok()?;
        for _ in 0..4 {
            let _ = cursor.f32().ok()?;
        }
        let _skin = cursor.i32().ok()?;
        let _shader = cursor.i32().ok()?;
        cursor.i32().ok()
    }

    pub fn meshes(&self) -> Vec<MeshPart> {
        let transforms = self.transforms();
        let mut parts = Vec::new();
        for (index, block) in self.blocks.iter().enumerate() {
            if block.type_name != "BSTriShape" {
                continue;
            }
            if let Ok(Some(shape)) = self.parse_skinned(index, transforms[index], &transforms) {
                parts.push(shape.part);
            } else if let Ok(Some(part)) = self.parse_shape(index, transforms[index]) {
                parts.push(part);
            }
        }
        parts
    }

    pub fn skin_summaries(&self) -> Result<Vec<SkinSummary>> {
        let transforms = self.transforms();
        let mut summaries = Vec::new();
        for (index, block) in self.blocks.iter().enumerate() {
            if block.type_name != "BSTriShape" {
                continue;
            }
            if let Some(shape) = self.parse_skinned(index, transforms[index], &transforms)? {
                summaries.push(shape.summary);
            }
        }
        Ok(summaries)
    }

    fn parse_skinned(
        &self,
        index: usize,
        parent: Transform,
        transforms: &[Transform],
    ) -> Result<Option<SkinnedShape>> {
        let block = &self.blocks[index];
        let mut cursor = Cursor::at(&self.data, block.offset);
        skip_av_object(&mut cursor)?;
        let translation = cursor.vec3()?;
        let rotation = read_matrix(&mut cursor)?;
        let scale = cursor.f32()?;
        let _collision = cursor.i32()?;
        for _ in 0..4 {
            let _ = cursor.f32()?;
        }
        let skin_ref = cursor.i32()?;
        let _shader = cursor.i32()?;
        let alpha = cursor.i32()?;
        let _vertex_desc = cursor.u64()?;
        let _triangles = cursor.u16()?;
        let _vertices = cursor.u16()?;
        let _data_size = cursor.u32()?;

        if skin_ref < 0 {
            return Ok(None);
        }

        let local = Transform {
            translation,
            rotation,
            scale,
        };
        let shape_world = parent.then(local);
        let instance = self.parse_skin_instance(skin_ref as usize)?;
        let data = self.parse_skin_data(instance.data)?;
        let partitions = self.parse_skin_partition(instance.partition)?;
        let layout = VertexLayout::from_descriptor(partitions.descriptor)?;

        if partitions.vertex_size != layout.size {
            return Err(Error::invalid(
                "NiSkinPartition vertex size does not match its descriptor",
            ));
        }
        if data.bones.len() != instance.bones.len() {
            return Err(Error::invalid(
                "NiSkinData bone count does not match NiSkinInstance",
            ));
        }

        let mut bone_world = Vec::with_capacity(instance.bones.len());
        for &bone in &instance.bones {
            let Some(block) = self.blocks.get(bone) else {
                return Err(Error::invalid("skin bone reference is out of range"));
            };
            if !NODE_TYPES.contains(&block.type_name.as_str()) {
                return Err(Error::invalid("skin bone reference is not a node"));
            }
            bone_world.push(transforms[bone]);
        }

        let mut vertices = Vec::new();
        let mut indices = Vec::new();
        let mut triangle_count = 0usize;
        let mut bind_deviation = 0.0f32;

        for partition in &partitions.partitions {
            if partition.triangles == 0 && partition.faces.is_empty() {
                continue;
            }
            let Some(partition_bone_indices) = partition.bone_indices.as_ref() else {
                return Err(Error::unsupported(
                    "skinned partition has no per-vertex bone indices",
                ));
            };
            if partition.weights.len() != partition.vertices * partition.weights_per_vertex {
                return Err(Error::invalid(
                    "skinned partition weight count does not match its vertices",
                ));
            }
            if partition_bone_indices.len() != partition.vertices * partition.weights_per_vertex {
                return Err(Error::invalid(
                    "skinned partition bone index count does not match its vertices",
                ));
            }

            let mut local_by_global = HashMap::with_capacity(partition.vertices);
            for local in 0..partition.vertices {
                let global = partition
                    .vertex_map
                    .as_ref()
                    .and_then(|map| map.get(local))
                    .copied()
                    .unwrap_or(local as u16);
                if local_by_global.insert(global, local as u16).is_some() {
                    return Err(Error::invalid(
                        "skinned partition vertex map contains duplicates",
                    ));
                }
            }

            let base = vertices.len() as u32;
            for local in 0..partition.vertices {
                let global = partition
                    .vertex_map
                    .as_ref()
                    .and_then(|map| map.get(local))
                    .copied()
                    .unwrap_or(local as u16) as usize;
                let Some(vertex) = partitions.vertices.get(global) else {
                    return Err(Error::invalid(
                        "skinned partition vertex map points past the vertex block",
                    ));
                };

                let mut position = Vec3::ZERO;
                let mut normal = Vec3::ZERO;
                let mut weight_sum = 0.0f32;
                let mut has_normal = false;
                for influence in 0..partition.weights_per_vertex {
                    let weight =
                        partition.weights[local * partition.weights_per_vertex + influence];
                    if !weight.is_finite() || weight < 0.0 {
                        return Err(Error::invalid("skinned vertex weight is invalid"));
                    }
                    if weight == 0.0 {
                        continue;
                    }
                    let palette_index = partition_bone_indices
                        [local * partition.weights_per_vertex + influence]
                        as usize;
                    let Some(&bone_index) = partition.bones.get(palette_index) else {
                        return Err(Error::invalid(
                            "skinned partition bone index is out of range",
                        ));
                    };
                    let bone_index = bone_index as usize;
                    let Some(&bone_transform) = data.bones.get(bone_index) else {
                        return Err(Error::invalid("skinned partition bone is out of range"));
                    };
                    let matrix = bone_world[bone_index].then(bone_transform);
                    position += weight * matrix.apply(vertex.position);
                    if let Some(vertex_normal) = vertex.normal {
                        normal += weight * (matrix.rotation * vertex_normal);
                        has_normal = true;
                    }
                    weight_sum += weight;
                }

                if weight_sum > 0.0 {
                    position /= weight_sum;
                    normal /= weight_sum;
                } else {
                    position = shape_world.apply(vertex.position);
                }
                let bind_position = shape_world.apply(vertex.position);
                bind_deviation = bind_deviation.max(position.distance(bind_position));

                vertices.push(MeshVertex {
                    position: position.to_array(),
                    normal: if has_normal {
                        normal.normalize_or_zero().to_array()
                    } else {
                        [0.0, 0.0, 0.0]
                    },
                    uv: vertex.uv,
                    color: vertex.color,
                });
            }

            for face in &partition.faces {
                let mut mapped = [0u32; 3];
                for (slot, &global) in face.iter().enumerate() {
                    let Some(&local) = local_by_global.get(&global) else {
                        return Err(Error::invalid(
                            "skinned partition triangle vertex is not in its vertex map",
                        ));
                    };
                    mapped[slot] = base + local as u32;
                }
                indices.extend_from_slice(&mapped);
            }
            triangle_count += partition.faces.len();
        }

        if vertices
            .iter()
            .any(|vertex| Vec3::from_array(vertex.normal).length_squared() == 0.0)
        {
            compute_normals(&mut vertices, &indices);
        }

        let summary = SkinSummary {
            shape: index,
            bones: instance.bones.len(),
            dismember: instance.dismember,
            has_vertex_weights: data.has_vertex_weights,
            resolved_bones: bone_world.len(),
            descriptor: partitions.descriptor,
            vertex_size: partitions.vertex_size,
            vertices: partitions.vertices.len(),
            triangles: triangle_count,
            weights_per_vertex: partitions
                .partitions
                .first()
                .map(|partition| partition.weights_per_vertex)
                .unwrap_or(0),
            partitions: partitions
                .partitions
                .iter()
                .map(|partition| SkinPartitionSummary {
                    vertices: partition.vertices,
                    triangles: partition.triangles,
                })
                .collect(),
            bind_deviation,
        };

        Ok(Some(SkinnedShape {
            part: MeshPart {
                vertices,
                indices,
                texture: self.texture_for_shape(index),
                alpha_flags: self.alpha_flags(alpha),
            },
            summary,
        }))
    }

    fn parse_skin_instance(&self, index: usize) -> Result<SkinInstance> {
        let block = self
            .blocks
            .get(index)
            .ok_or_else(|| Error::invalid("NiSkinInstance reference is out of range"))?;
        let is_dismember = block.type_name == "BSDismemberSkinInstance";
        if block.type_name != "NiSkinInstance" && !is_dismember {
            return Err(Error::invalid(
                "skin instance reference is not a NiSkinInstance",
            ));
        }

        let mut cursor = Cursor::at(&self.data, block.offset);
        let data = block_ref(cursor.i32()?)?;
        let partition = block_ref(cursor.i32()?)?;
        let _skeleton_root = cursor.i32()?;
        let bone_count = cursor.u32()? as usize;
        if bone_count > 4096 {
            return Err(Error::invalid("skin bone count is unreasonable"));
        }
        let mut bones = Vec::with_capacity(bone_count);
        for _ in 0..bone_count {
            bones.push(block_ref(cursor.i32()?)?);
        }

        let mut dismember = 0;
        if is_dismember {
            let partition_count = cursor.u32()? as usize;
            if partition_count > 4096 {
                return Err(Error::invalid(
                    "skin dismember partition count is unreasonable",
                ));
            }
            for _ in 0..partition_count {
                let _flags = cursor.u16()?;
                let _body_part = cursor.u16()?;
            }
            dismember = partition_count;
        }
        ensure_block_end(&self.data, block, &cursor, "NiSkinInstance")?;
        Ok(SkinInstance {
            data,
            partition,
            bones,
            dismember,
        })
    }

    fn parse_skin_data(&self, index: usize) -> Result<SkinData> {
        let block = self
            .blocks
            .get(index)
            .ok_or_else(|| Error::invalid("NiSkinData reference is out of range"))?;
        if block.type_name != "NiSkinData" {
            return Err(Error::invalid("skin data reference is not NiSkinData"));
        }

        let mut cursor = Cursor::at(&self.data, block.offset);
        let _skin_transform = read_nif_transform(&mut cursor)?;
        let bone_count = cursor.u32()? as usize;
        if bone_count > 4096 {
            return Err(Error::invalid("skin data bone count is unreasonable"));
        }
        let has_vertex_weights = cursor.u8()? != 0;
        let mut bones = Vec::with_capacity(bone_count);
        for _ in 0..bone_count {
            let transform = read_nif_transform(&mut cursor)?;
            let _bound_center = cursor.vec3()?;
            let _bound_radius = cursor.f32()?;
            let vertex_count = cursor.u16()? as usize;
            if vertex_count > 10_000_000 {
                return Err(Error::invalid("skin bone vertex count is unreasonable"));
            }
            if has_vertex_weights {
                for _ in 0..vertex_count {
                    let _index = cursor.u16()?;
                    let weight = cursor.f32()?;
                    if !weight.is_finite() {
                        return Err(Error::invalid("skin bone vertex weight is invalid"));
                    }
                }
            }
            bones.push(transform);
        }

        ensure_block_end(&self.data, block, &cursor, "NiSkinData")?;
        Ok(SkinData {
            has_vertex_weights,
            bones,
        })
    }

    fn parse_skin_partition(&self, index: usize) -> Result<SkinPartitionData> {
        let block = self
            .blocks
            .get(index)
            .ok_or_else(|| Error::invalid("NiSkinPartition reference is out of range"))?;
        if block.type_name != "NiSkinPartition" {
            return Err(Error::invalid(
                "skin partition reference is not NiSkinPartition",
            ));
        }

        let mut cursor = Cursor::at(&self.data, block.offset);
        let partition_count = cursor.u32()? as usize;
        if partition_count > 4096 {
            return Err(Error::invalid(
                "NiSkinPartition partition count is unreasonable",
            ));
        }
        let data_size = cursor.u32()? as usize;
        let vertex_size = cursor.u32()? as usize;
        let descriptor = cursor.u64()?;
        let layout = VertexLayout::from_descriptor(descriptor)?;
        if vertex_size != layout.size {
            return Err(Error::invalid(
                "NiSkinPartition vertex size does not match its descriptor",
            ));
        }
        if vertex_size == 0 || !data_size.is_multiple_of(vertex_size) {
            return Err(Error::invalid(
                "NiSkinPartition vertex data size is not integral",
            ));
        }
        let vertex_count = data_size / vertex_size;
        if vertex_count > 10_000_000 {
            return Err(Error::invalid(
                "NiSkinPartition vertex count is unreasonable",
            ));
        }
        let vertex_bytes = cursor.bytes(data_size)?;
        let mut vertices = Vec::with_capacity(vertex_count);
        for vertex in 0..vertex_count {
            let start = vertex * vertex_size;
            vertices.push(decode_skin_vertex(
                &vertex_bytes[start..start + vertex_size],
                &layout,
            )?);
        }

        let mut partitions = Vec::with_capacity(partition_count);
        for _ in 0..partition_count {
            let vertex_count = cursor.u16()? as usize;
            let triangle_count = cursor.u16()? as usize;
            let bone_count = cursor.u16()? as usize;
            let strip_count = cursor.u16()? as usize;
            let weights_per_vertex = cursor.u16()? as usize;
            if vertex_count > 10_000_000
                || triangle_count > 10_000_000
                || bone_count > 4096
                || strip_count > 4096
                || weights_per_vertex > 16
            {
                return Err(Error::invalid(
                    "NiSkinPartition partition counts are unreasonable",
                ));
            }

            let mut bones = Vec::with_capacity(bone_count);
            for _ in 0..bone_count {
                bones.push(cursor.u16()?);
            }

            let has_vertex_map = cursor.u8()? != 0;
            let mut vertex_map = None;
            if has_vertex_map {
                let mut map = Vec::with_capacity(vertex_count);
                for _ in 0..vertex_count {
                    map.push(cursor.u16()?);
                }
                vertex_map = Some(map);
            }

            let has_vertex_weights = cursor.u8()? != 0;
            let mut weights = Vec::new();
            if has_vertex_weights {
                weights.reserve(vertex_count * weights_per_vertex);
                for _ in 0..vertex_count * weights_per_vertex {
                    weights.push(cursor.f32()?);
                }
            }

            let mut strip_lengths = Vec::with_capacity(strip_count);
            for _ in 0..strip_count {
                strip_lengths.push(cursor.u16()? as usize);
            }

            let has_faces = cursor.u8()? != 0;
            let mut faces = Vec::new();
            if has_faces && strip_count == 0 {
                faces.reserve(triangle_count);
                for _ in 0..triangle_count {
                    faces.push([cursor.u16()?, cursor.u16()?, cursor.u16()?]);
                }
            } else if has_faces {
                let strip_vertices: usize = strip_lengths.iter().sum();
                if strip_vertices > 10_000_000 {
                    return Err(Error::invalid(
                        "NiSkinPartition strip vertex count is unreasonable",
                    ));
                }
                for _ in 0..strip_vertices {
                    let _ = cursor.u16()?;
                }
                return Err(Error::unsupported(
                    "skinned triangle strips are not supported",
                ));
            }

            let has_bone_indices = cursor.u8()? != 0;
            let mut bone_indices = None;
            if has_bone_indices {
                let mut indices = Vec::with_capacity(vertex_count * weights_per_vertex);
                for _ in 0..vertex_count * weights_per_vertex {
                    indices.push(cursor.u8()?);
                }
                bone_indices = Some(indices);
            }

            let _lod_level = cursor.u8()?;
            let _global_vb = cursor.u8()?;
            let _partition_desc = cursor.u64()?;
            for _ in 0..triangle_count {
                let _ = cursor.u16()?;
                let _ = cursor.u16()?;
                let _ = cursor.u16()?;
            }

            if let Some(map) = &vertex_map {
                if map.iter().any(|&vertex| vertex as usize >= vertices.len()) {
                    return Err(Error::invalid("NiSkinPartition vertex map is out of range"));
                }
            }
            if faces
                .iter()
                .flatten()
                .any(|&vertex| vertex as usize >= vertices.len())
            {
                return Err(Error::invalid(
                    "NiSkinPartition triangle index is out of range",
                ));
            }

            partitions.push(SkinPartition {
                vertices: vertex_count,
                triangles: triangle_count,
                bones,
                vertex_map,
                weights_per_vertex,
                weights,
                bone_indices,
                faces,
            });
        }

        ensure_block_end(&self.data, block, &cursor, "NiSkinPartition")?;
        Ok(SkinPartitionData {
            descriptor,
            vertex_size,
            vertices,
            partitions,
        })
    }

    fn transforms(&self) -> Vec<Transform> {
        let mut transforms = vec![Transform::IDENTITY; self.blocks.len()];
        let mut visited = vec![false; self.blocks.len()];
        self.walk_node(0, Transform::IDENTITY, &mut transforms, &mut visited);
        transforms
    }

    fn walk_node(
        &self,
        index: usize,
        parent: Transform,
        transforms: &mut [Transform],
        visited: &mut [bool],
    ) {
        if index >= self.blocks.len() || visited[index] {
            return;
        }
        visited[index] = true;
        if !NODE_TYPES.contains(&self.blocks[index].type_name.as_str()) {
            transforms[index] = parent;
            return;
        }
        let Ok(Some(node)) = self.node(index) else {
            transforms[index] = parent;
            return;
        };
        let world = parent.then(Transform {
            translation: node.translation,
            rotation: node.rotation,
            scale: node.scale,
        });
        transforms[index] = world;
        for child in node.children {
            if child >= 0 {
                self.walk_node(child as usize, world, transforms, visited);
            }
        }
    }

    fn parse_shape(&self, index: usize, parent: Transform) -> Result<Option<MeshPart>> {
        let block = &self.blocks[index];
        let mut cursor = Cursor::at(&self.data, block.offset);
        skip_av_object(&mut cursor)?;
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
        let alpha = cursor.i32()?;
        let _vertex_desc = cursor.u64()?;
        let triangles = cursor.u16()? as usize;
        let vertices = cursor.u16()? as usize;
        let data_size = cursor.u32()? as usize;

        if vertices == 0 || triangles == 0 || data_size < triangles * 6 {
            return Ok(None);
        }
        let vertex_bytes = data_size - triangles * 6;
        let stride = vertex_bytes / vertices;
        if stride * vertices != vertex_bytes {
            return Err(Error::invalid("BSTriShape vertex stride is not integral"));
        }
        if !(28..=96).contains(&stride) {
            return Err(Error::unsupported(
                "BSTriShape vertex stride is unsupported",
            ));
        }
        let vertex_start = cursor.pos;
        let end = vertex_start
            .checked_add(data_size)
            .ok_or_else(|| Error::invalid("BSTriShape data range overflows"))?;
        if end > self.data.len() {
            return Err(Error::invalid("BSTriShape data extends past the file"));
        }

        let local = Transform {
            translation,
            rotation: Mat3::from_cols(
                Vec3::new(rotation[0], rotation[3], rotation[6]),
                Vec3::new(rotation[1], rotation[4], rotation[7]),
                Vec3::new(rotation[2], rotation[5], rotation[8]),
            ),
            scale,
        };
        let world = parent.then(local);
        let mut out = Vec::with_capacity(vertices);
        for vertex in 0..vertices {
            let base = vertex_start + vertex * stride;
            let position = Vec3::new(
                read_f32(&self.data, base)?,
                read_f32(&self.data, base + 4)?,
                read_f32(&self.data, base + 8)?,
            );
            let uv = [
                f16::from_bits(read_u16(&self.data, base + 16)?).to_f32(),
                f16::from_bits(read_u16(&self.data, base + 18)?).to_f32(),
            ];
            let normal = decode_normal(&self.data, base + 20)?;
            let color = if stride >= 32 {
                [
                    self.data[base + 28] as f32 / 255.0,
                    self.data[base + 29] as f32 / 255.0,
                    self.data[base + 30] as f32 / 255.0,
                ]
            } else {
                [1.0, 1.0, 1.0]
            };
            out.push(MeshVertex {
                position: world.apply(position).to_array(),
                normal: (world.rotation * normal).to_array(),
                uv,
                color,
            });
        }

        let index_start = vertex_start + vertices * stride;
        let mut indices = Vec::with_capacity(triangles * 3);
        for index in 0..triangles * 3 {
            indices.push(read_u16(&self.data, index_start + index * 2)? as u32);
        }

        Ok(Some(MeshPart {
            vertices: out,
            indices,
            texture: self.texture_for_shader(shader),
            alpha_flags: self.alpha_flags(alpha),
        }))
    }
}

fn skip_av_object(cursor: &mut Cursor<'_>) -> Result<()> {
    let _name = cursor.u32()?;
    let extra_count = cursor.u32()? as usize;
    if extra_count > 4096 {
        return Err(Error::invalid("NIF extra-data count is unreasonable"));
    }
    for _ in 0..extra_count {
        let _ = cursor.u32()?;
    }
    let _controller = cursor.i32()?;
    let _flags = cursor.u32()?;
    Ok(())
}

fn skip_av_transform(cursor: &mut Cursor<'_>) -> Result<()> {
    let _translation = cursor.vec3()?;
    for _ in 0..9 {
        let _ = cursor.f32()?;
    }
    let _scale = cursor.f32()?;
    let _collision = cursor.i32()?;
    Ok(())
}

fn read_u16(data: &[u8], offset: usize) -> Result<u16> {
    let bytes = data
        .get(offset..offset + 2)
        .ok_or_else(|| Error::invalid("NIF read past end"))?;
    Ok(u16::from_le_bytes(bytes.try_into().unwrap()))
}

fn read_f32(data: &[u8], offset: usize) -> Result<f32> {
    let bytes = data
        .get(offset..offset + 4)
        .ok_or_else(|| Error::invalid("NIF read past end"))?;
    Ok(f32::from_le_bytes(bytes.try_into().unwrap()))
}

fn decode_normal(data: &[u8], offset: usize) -> Result<Vec3> {
    let bytes = data
        .get(offset..offset + 4)
        .ok_or_else(|| Error::invalid("NIF read past end"))?;
    let normal = Vec3::new(
        bytes[0] as f32 / 255.0 * 2.0 - 1.0,
        bytes[1] as f32 / 255.0 * 2.0 - 1.0,
        bytes[2] as f32 / 255.0 * 2.0 - 1.0,
    );
    Ok(normal.normalize_or_zero())
}

fn read_matrix(cursor: &mut Cursor<'_>) -> Result<Mat3> {
    let mut rotation = [0.0f32; 9];
    for value in &mut rotation {
        *value = cursor.f32()?;
    }
    Ok(Mat3::from_cols(
        Vec3::new(rotation[0], rotation[3], rotation[6]),
        Vec3::new(rotation[1], rotation[4], rotation[7]),
        Vec3::new(rotation[2], rotation[5], rotation[8]),
    ))
}

fn read_nif_transform(cursor: &mut Cursor<'_>) -> Result<Transform> {
    let rotation = read_matrix(cursor)?;
    let translation = cursor.vec3()?;
    let scale = cursor.f32()?;
    Ok(Transform {
        translation,
        rotation,
        scale,
    })
}

fn block_ref(reference: i32) -> Result<usize> {
    usize::try_from(reference).map_err(|_| Error::invalid("NIF block reference is negative"))
}

fn ensure_block_end(data: &[u8], block: &Block, cursor: &Cursor<'_>, label: &str) -> Result<()> {
    let end = block
        .offset
        .checked_add(block.size)
        .ok_or_else(|| Error::invalid(format!("{label} block range overflows")))?;
    if end > data.len() {
        return Err(Error::invalid(format!(
            "{label} block extends past the file"
        )));
    }
    if cursor.pos != end {
        return Err(Error::invalid(format!(
            "{label} block was not fully consumed"
        )));
    }
    Ok(())
}

impl VertexLayout {
    fn from_descriptor(descriptor: u64) -> Result<Self> {
        let size = ((descriptor & 0xf) as usize) * 4;
        if size == 0 || size > 256 {
            return Err(Error::unsupported("NIF vertex size is unsupported"));
        }
        let layout = Self {
            size,
            uv: (((descriptor >> 8) & 0xf) as usize) * 4,
            normal: (((descriptor >> 16) & 0xf) as usize) * 4,
            tangent: (((descriptor >> 20) & 0xf) as usize) * 4,
            color: (((descriptor >> 24) & 0xf) as usize) * 4,
            skin: (((descriptor >> 28) & 0xf) as usize) * 4,
            flags: ((descriptor >> 44) & 0xfff) as u16,
        };
        if layout.flags & 0x1 == 0 {
            return Err(Error::unsupported(
                "NIF skinned vertex has no position attribute",
            ));
        }
        if size < 16 {
            return Err(Error::invalid("NIF skinned vertex is too small"));
        }
        for (offset, length, enabled) in [
            (layout.uv, 4, layout.flags & 0x2 != 0),
            (layout.normal, 4, layout.flags & 0x8 != 0),
            (layout.tangent, 4, layout.flags & 0x18 == 0x18),
            (layout.color, 4, layout.flags & 0x20 != 0),
            (layout.skin, 12, layout.flags & 0x40 != 0),
        ] {
            if enabled && offset + length > size {
                return Err(Error::invalid(
                    "NIF skinned vertex attribute is out of range",
                ));
            }
        }
        Ok(layout)
    }
}

fn decode_skin_vertex(data: &[u8], layout: &VertexLayout) -> Result<SkinVertex> {
    let position = Vec3::new(read_f32(data, 0)?, read_f32(data, 4)?, read_f32(data, 8)?);
    let uv = if layout.flags & 0x2 != 0 {
        [
            f16::from_bits(read_u16(data, layout.uv)?).to_f32(),
            f16::from_bits(read_u16(data, layout.uv + 2)?).to_f32(),
        ]
    } else {
        [0.0, 0.0]
    };
    let normal = if layout.flags & 0x8 != 0 {
        Some(decode_normal(data, layout.normal)?)
    } else {
        None
    };
    let color = if layout.flags & 0x20 != 0 {
        [
            read_u8(data, layout.color)? as f32 / 255.0,
            read_u8(data, layout.color + 1)? as f32 / 255.0,
            read_u8(data, layout.color + 2)? as f32 / 255.0,
        ]
    } else {
        [1.0, 1.0, 1.0]
    };

    Ok(SkinVertex {
        position,
        normal,
        uv,
        color,
    })
}

fn compute_normals(vertices: &mut [MeshVertex], indices: &[u32]) {
    let mut normals = vec![Vec3::ZERO; vertices.len()];
    for triangle in indices.as_chunks::<3>().0 {
        let [a, b, c] = [triangle[0], triangle[1], triangle[2]];
        let (Some(a), Some(b), Some(c)) = (
            vertices.get(a as usize),
            vertices.get(b as usize),
            vertices.get(c as usize),
        ) else {
            continue;
        };
        let edge1 = Vec3::from_array(b.position) - Vec3::from_array(a.position);
        let edge2 = Vec3::from_array(c.position) - Vec3::from_array(a.position);
        let normal = edge1.cross(edge2);
        if normal.length_squared() > 0.0 {
            normals[triangle[0] as usize] += normal;
            normals[triangle[1] as usize] += normal;
            normals[triangle[2] as usize] += normal;
        }
    }
    for (vertex, normal) in vertices.iter_mut().zip(normals) {
        if Vec3::from_array(vertex.normal).length_squared() == 0.0 {
            vertex.normal = if normal.length_squared() > 0.0 {
                normal.normalize().to_array()
            } else {
                [0.0, 0.0, 1.0]
            };
        }
    }
}

fn read_u8(data: &[u8], offset: usize) -> Result<u8> {
    data.get(offset)
        .copied()
        .ok_or_else(|| Error::invalid("NIF read past end"))
}

struct Cursor<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Cursor<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0 }
    }

    fn at(data: &'a [u8], pos: usize) -> Self {
        Self { data, pos }
    }

    fn bytes(&mut self, count: usize) -> Result<&'a [u8]> {
        let end = self
            .pos
            .checked_add(count)
            .ok_or_else(|| Error::invalid("NIF offset overflows"))?;
        let bytes = self
            .data
            .get(self.pos..end)
            .ok_or_else(|| Error::invalid("NIF read past end"))?;
        self.pos = end;
        Ok(bytes)
    }

    fn u8(&mut self) -> Result<u8> {
        Ok(self.bytes(1)?[0])
    }

    fn u16(&mut self) -> Result<u16> {
        Ok(u16::from_le_bytes(self.bytes(2)?.try_into().unwrap()))
    }

    fn u32(&mut self) -> Result<u32> {
        Ok(u32::from_le_bytes(self.bytes(4)?.try_into().unwrap()))
    }

    fn i32(&mut self) -> Result<i32> {
        Ok(i32::from_le_bytes(self.bytes(4)?.try_into().unwrap()))
    }

    fn u64(&mut self) -> Result<u64> {
        Ok(u64::from_le_bytes(self.bytes(8)?.try_into().unwrap()))
    }

    fn f32(&mut self) -> Result<f32> {
        Ok(f32::from_le_bytes(self.bytes(4)?.try_into().unwrap()))
    }

    fn vec3(&mut self) -> Result<Vec3> {
        Ok(Vec3::new(self.f32()?, self.f32()?, self.f32()?))
    }

    fn string(&mut self) -> Result<String> {
        let length = self.u32()? as usize;
        let bytes = self.bytes(length)?;
        Ok(String::from_utf8_lossy(bytes).into_owned())
    }

    fn short_string(&mut self) -> Result<String> {
        let length = self.u8()? as usize;
        let bytes = self.bytes(length)?;
        Ok(String::from_utf8_lossy(bytes).into_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn push_u16(out: &mut Vec<u8>, value: u16) {
        out.extend_from_slice(&value.to_le_bytes());
    }

    fn push_u32(out: &mut Vec<u8>, value: u32) {
        out.extend_from_slice(&value.to_le_bytes());
    }

    fn push_i32(out: &mut Vec<u8>, value: i32) {
        out.extend_from_slice(&value.to_le_bytes());
    }

    fn push_f32(out: &mut Vec<u8>, value: f32) {
        out.extend_from_slice(&value.to_le_bytes());
    }

    fn push_string(out: &mut Vec<u8>, value: &str) {
        push_u32(out, value.len() as u32);
        out.extend_from_slice(value.as_bytes());
    }

    fn push_short_string(out: &mut Vec<u8>, value: &str) {
        out.push(value.len() as u8);
        out.extend_from_slice(value.as_bytes());
    }

    fn push_header(out: &mut Vec<u8>, types: &[&str], sizes: &[u32]) {
        out.extend_from_slice(b"Gamebryo File Format, Version 20.2.0.7\n");
        push_u32(out, SSE_VERSION);
        out.push(1);
        push_u32(out, 12);
        push_u32(out, sizes.len() as u32);
        push_u32(out, 100);
        push_short_string(out, "");
        push_short_string(out, "");
        push_short_string(out, "");
        push_u16(out, types.len() as u16);
        for name in types {
            push_string(out, name);
        }
        for index in 0..sizes.len() {
            push_u16(out, index as u16);
        }
        for size in sizes {
            push_u32(out, *size);
        }
        push_u32(out, 0);
        push_u32(out, 0);
        push_u32(out, 0);
    }

    fn node_block(children: &[i32], translation: [f32; 3]) -> Vec<u8> {
        let mut out = Vec::new();
        push_u32(&mut out, 0xffff_ffff);
        push_u32(&mut out, 0);
        push_i32(&mut out, -1);
        push_u32(&mut out, 0);
        for value in translation {
            push_f32(&mut out, value);
        }
        for index in 0..9 {
            push_f32(&mut out, if index % 4 == 0 { 1.0 } else { 0.0 });
        }
        push_f32(&mut out, 1.0);
        push_i32(&mut out, -1);
        push_u32(&mut out, children.len() as u32);
        for child in children {
            push_i32(&mut out, *child);
        }
        out
    }

    fn finish(mut out: Vec<u8>, blocks: Vec<Vec<u8>>) -> Vec<u8> {
        let data_start = out.len();
        let mut offset = data_start;
        for block in &blocks {
            out.extend_from_slice(block);
            offset += block.len();
        }
        let _ = offset;
        out
    }

    #[test]
    fn parses_header_and_histogram() {
        let mut out = Vec::new();
        push_header(&mut out, &["NiNode"], &[76]);
        let block = node_block(&[], [0.0, 0.0, 0.0]);
        let bytes = finish(out, vec![block]);
        let nif = Nif::parse(&bytes).unwrap();
        assert_eq!(nif.version(), SSE_VERSION);
        assert_eq!(nif.blocks().len(), 1);
        assert_eq!(nif.block_type(0), Some("NiNode"));
        assert_eq!(nif.histogram().get("NiNode"), Some(&1));
    }

    #[test]
    fn reads_node_transform_and_children() {
        let mut out = Vec::new();
        push_header(&mut out, &["NiNode", "NiNode"], &[76, 76]);
        let bytes = finish(
            out,
            vec![
                node_block(&[1], [1.0, 2.0, 3.0]),
                node_block(&[], [0.0, 0.0, 0.0]),
            ],
        );
        let nif = Nif::parse(&bytes).unwrap();
        let node = nif.node(0).unwrap().unwrap();
        assert_eq!(node.translation, Vec3::new(1.0, 2.0, 3.0));
        assert_eq!(node.children, vec![1]);
        assert_eq!(node.scale, 1.0);
    }

    #[test]
    fn resolves_shader_texture_set_by_reference() {
        let mut shader = vec![0u8; 44];
        shader[40..44].copy_from_slice(&1u32.to_le_bytes());
        let mut texture_set = Vec::new();
        push_u32(&mut texture_set, 1);
        push_string(&mut texture_set, "textures\\test.dds");

        let mut out = Vec::new();
        push_header(
            &mut out,
            &["BSLightingShaderProperty", "BSShaderTextureSet"],
            &[shader.len() as u32, texture_set.len() as u32],
        );
        let bytes = finish(out, vec![shader, texture_set]);
        let nif = Nif::parse(&bytes).unwrap();
        assert_eq!(
            nif.texture_for_shader(0).as_deref(),
            Some("textures\\test.dds")
        );
    }

    fn shape_block(shader: i32, alpha: i32) -> Vec<u8> {
        let mut out = Vec::new();
        push_u32(&mut out, 0xffff_ffff);
        push_u32(&mut out, 0);
        push_i32(&mut out, -1);
        push_u32(&mut out, 0);
        for _ in 0..3 {
            push_f32(&mut out, 0.0);
        }
        for index in 0..9 {
            push_f32(&mut out, if index % 4 == 0 { 1.0 } else { 0.0 });
        }
        push_f32(&mut out, 1.0);
        push_i32(&mut out, -1);
        for _ in 0..4 {
            push_f32(&mut out, 0.0);
        }
        push_i32(&mut out, -1);
        push_i32(&mut out, shader);
        push_i32(&mut out, alpha);
        out.extend_from_slice(&0u64.to_le_bytes());
        push_u16(&mut out, 1);
        push_u16(&mut out, 3);
        let data_size = 3 * 32 + 6;
        push_u32(&mut out, data_size as u32);

        for vertex in 0..3 {
            let mut data = [0u8; 32];
            data[0..4].copy_from_slice(&(vertex as f32).to_le_bytes());
            data[4..8].copy_from_slice(&(if vertex == 2 { 1.0f32 } else { 0.0f32 }).to_le_bytes());
            data[16..18].copy_from_slice(&f16::from_f32(0.5).to_bits().to_le_bytes());
            data[18..20].copy_from_slice(&f16::from_f32(0.5).to_bits().to_le_bytes());
            data[20] = 128;
            data[21] = 128;
            data[22] = 255;
            data[28] = 255;
            data[29] = 255;
            data[30] = 255;
            out.extend_from_slice(&data);
        }
        push_u16(&mut out, 0);
        push_u16(&mut out, 1);
        push_u16(&mut out, 2);
        out
    }

    fn push_transform(out: &mut Vec<u8>, translation: [f32; 3]) {
        for index in 0..9 {
            push_f32(out, if index % 4 == 0 { 1.0 } else { 0.0 });
        }
        for value in translation {
            push_f32(out, value);
        }
        push_f32(out, 1.0);
    }

    fn skinned_shape_block(skin: i32, descriptor: u64) -> Vec<u8> {
        let mut out = Vec::new();
        push_u32(&mut out, 0xffff_ffff);
        push_u32(&mut out, 0);
        push_i32(&mut out, -1);
        push_u32(&mut out, 0);
        for _ in 0..3 {
            push_f32(&mut out, 0.0);
        }
        for index in 0..9 {
            push_f32(&mut out, if index % 4 == 0 { 1.0 } else { 0.0 });
        }
        push_f32(&mut out, 1.0);
        push_i32(&mut out, -1);
        for _ in 0..4 {
            push_f32(&mut out, 0.0);
        }
        push_i32(&mut out, skin);
        push_i32(&mut out, -1);
        push_i32(&mut out, -1);
        out.extend_from_slice(&descriptor.to_le_bytes());
        push_u16(&mut out, 0);
        push_u16(&mut out, 0);
        push_u32(&mut out, 0);
        out
    }

    fn skin_instance_block(data: i32, partition: i32, dismember: bool) -> Vec<u8> {
        let mut out = Vec::new();
        push_i32(&mut out, data);
        push_i32(&mut out, partition);
        push_i32(&mut out, 0);
        push_u32(&mut out, 1);
        push_i32(&mut out, 5);
        if dismember {
            push_u32(&mut out, 3);
            for body_part in 0..3 {
                push_u16(&mut out, 257);
                push_u16(&mut out, body_part);
            }
        }
        out
    }

    fn skin_data_block() -> Vec<u8> {
        let mut out = Vec::new();
        push_transform(&mut out, [0.0, 0.0, 0.0]);
        push_u32(&mut out, 1);
        out.push(1);
        push_transform(&mut out, [0.0, 0.0, 0.0]);
        for _ in 0..3 {
            push_f32(&mut out, 0.0);
        }
        push_f32(&mut out, 1.0);
        push_u16(&mut out, 3);
        for vertex in 0..3 {
            push_u16(&mut out, vertex);
            push_f32(&mut out, 1.0);
        }
        out
    }

    fn skin_partition_block(descriptor: u64) -> Vec<u8> {
        let mut out = Vec::new();
        push_u32(&mut out, 1);
        push_u32(&mut out, 3 * 32);
        push_u32(&mut out, 32);
        out.extend_from_slice(&descriptor.to_le_bytes());
        for vertex in 0..3 {
            let mut data = [0u8; 32];
            data[0..4].copy_from_slice(&(if vertex == 1 { 1.0f32 } else { 0.0f32 }).to_le_bytes());
            data[4..8].copy_from_slice(&(if vertex == 2 { 1.0f32 } else { 0.0f32 }).to_le_bytes());
            data[16..18].copy_from_slice(&f16::from_f32(0.5).to_bits().to_le_bytes());
            data[18..20].copy_from_slice(&f16::from_f32(0.5).to_bits().to_le_bytes());
            data[20..22].copy_from_slice(&f16::from_f32(1.0).to_bits().to_le_bytes());
            out.extend_from_slice(&data);
        }
        push_u16(&mut out, 3);
        push_u16(&mut out, 1);
        push_u16(&mut out, 1);
        push_u16(&mut out, 0);
        push_u16(&mut out, 2);
        push_u16(&mut out, 0);
        out.push(1);
        for vertex in [2u16, 0, 1] {
            push_u16(&mut out, vertex);
        }
        out.push(1);
        for _ in 0..3 {
            push_f32(&mut out, 2.0);
            push_f32(&mut out, 0.0);
        }
        out.push(1);
        for vertex in 0..3 {
            push_u16(&mut out, vertex);
        }
        out.push(1);
        out.extend([0u8; 6]);
        out.push(0);
        out.push(0);
        out.extend_from_slice(&descriptor.to_le_bytes());
        for vertex in 0..3 {
            push_u16(&mut out, vertex);
        }
        out
    }

    fn skinned_nif(bone_translation: [f32; 3], dismember: bool) -> Vec<u8> {
        let descriptor = 0x0004_3000_5000_0408;
        let shape = skinned_shape_block(2, descriptor);
        let instance = skin_instance_block(3, 4, dismember);
        let data = skin_data_block();
        let partition = skin_partition_block(descriptor);
        let root = node_block(&[1, 5], [0.0, 0.0, 0.0]);
        let bone = node_block(&[], bone_translation);
        let types = if dismember {
            [
                "NiNode",
                "BSTriShape",
                "BSDismemberSkinInstance",
                "NiSkinData",
                "NiSkinPartition",
                "NiNode",
            ]
        } else {
            [
                "NiNode",
                "BSTriShape",
                "NiSkinInstance",
                "NiSkinData",
                "NiSkinPartition",
                "NiNode",
            ]
        };
        let sizes = [
            root.len() as u32,
            shape.len() as u32,
            instance.len() as u32,
            data.len() as u32,
            partition.len() as u32,
            bone.len() as u32,
        ];
        let mut out = Vec::new();
        push_header(&mut out, &types, &sizes);
        finish(out, vec![root, shape, instance, data, partition, bone])
    }

    #[test]
    fn reads_skinned_shape_with_partition() {
        let bytes = skinned_nif([0.0, 0.0, 0.0], false);
        let nif = Nif::parse(&bytes).unwrap();
        let summaries = nif.skin_summaries().unwrap();
        assert_eq!(summaries.len(), 1);
        assert_eq!(summaries[0].bones, 1);
        assert_eq!(summaries[0].dismember, 0);
        assert_eq!(summaries[0].resolved_bones, 1);
        assert_eq!(summaries[0].vertices, 3);
        assert_eq!(summaries[0].triangles, 1);
        assert_eq!(summaries[0].weights_per_vertex, 2);
        assert!(summaries[0].bind_deviation < 0.001);

        let parts = nif.meshes();
        assert_eq!(parts.len(), 1);
        assert_eq!(parts[0].vertices.len(), 3);
        assert_eq!(parts[0].indices, vec![1, 2, 0]);
        assert_eq!(parts[0].vertices[2].position, [1.0, 0.0, 0.0]);
        assert!(parts[0]
            .vertices
            .iter()
            .all(|vertex| Vec3::from_array(vertex.normal).length_squared() > 0.0));
    }

    #[test]
    fn dismember_skin_reports_partitions() {
        let bytes = skinned_nif([0.0, 0.0, 0.0], true);
        let nif = Nif::parse(&bytes).unwrap();
        let summaries = nif.skin_summaries().unwrap();
        assert_eq!(summaries[0].dismember, 3);
    }

    #[test]
    fn skinned_vertex_follows_bone_transform() {
        let bytes = skinned_nif([10.0, 0.0, 0.0], false);
        let nif = Nif::parse(&bytes).unwrap();
        let parts = nif.meshes();
        assert_eq!(parts.len(), 1);
        assert!(
            (parts[0].vertices[0].position[0] - 10.0).abs() < 0.001,
            "position={:?}",
            parts[0].vertices[0].position
        );
        let summaries = nif.skin_summaries().unwrap();
        assert!((summaries[0].bind_deviation - 10.0).abs() < 0.001);
    }

    #[test]
    fn reads_tri_shape_geometry() {
        let mut shader = vec![0u8; 44];
        shader[40..44].copy_from_slice(&2u32.to_le_bytes());
        let mut texture_set = Vec::new();
        push_u32(&mut texture_set, 1);
        push_string(&mut texture_set, "textures\\tri.dds");
        let shape = shape_block(1, -1);

        let mut out = Vec::new();
        push_header(
            &mut out,
            &[
                "BSTriShape",
                "BSLightingShaderProperty",
                "BSShaderTextureSet",
            ],
            &[
                shape.len() as u32,
                shader.len() as u32,
                texture_set.len() as u32,
            ],
        );
        let bytes = finish(out, vec![shape, shader, texture_set]);
        let nif = Nif::parse(&bytes).unwrap();
        let parts = nif.meshes();
        assert_eq!(parts.len(), 1);
        assert_eq!(parts[0].vertices.len(), 3);
        assert_eq!(parts[0].indices, vec![0, 1, 2]);
        assert_eq!(parts[0].texture.as_deref(), Some("textures\\tri.dds"));
        assert_eq!(parts[0].alpha_flags, 0);
    }

    #[test]
    fn reads_alpha_flags() {
        let mut alpha = vec![0u8; 15];
        alpha[12..14].copy_from_slice(&0x12ecu16.to_le_bytes());
        let mut out = Vec::new();
        push_header(&mut out, &["NiAlphaProperty"], &[alpha.len() as u32]);
        let bytes = finish(out, vec![alpha]);
        let nif = Nif::parse(&bytes).unwrap();
        assert_eq!(nif.alpha_flags(0), 0x12ec);
    }
}
