use std::collections::HashMap;

use glam::{EulerRot, Quat, Vec3};
use oce_core::Result;
use oce_data::Game;
use oce_nif::Nif;
use oce_plugin::{Plugin, Record, WalkEntry};

pub use oce_nif::MeshVertex;

const TAMRIEL: u32 = 0x0000_003C;
const TARGET_CELL: u32 = 0x0000_99A2;

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
        let mut cache: HashMap<String, Vec<oce_nif::MeshPart>> = HashMap::new();
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
                let mesh = match Nif::parse(&bytes) {
                    Ok(nif) => nif.meshes(),
                    Err(_) => {
                        failed_models += 1;
                        continue;
                    }
                };
                loaded_models += 1;
                cache.insert(model.clone(), mesh.clone());
                mesh
            };
            if mesh.is_empty() {
                continue;
            }
            let rotation = Quat::from_euler(
                EulerRot::XYZ,
                reference.rotation[0],
                reference.rotation[1],
                reference.rotation[2],
            );
            for part in mesh {
                push_part(
                    &mut objects,
                    part,
                    rotation,
                    Vec3::from_array(reference.position),
                    reference.scale,
                );
            }
        }

        let (textures, failed_textures) = load_textures(game, &objects)?;
        Ok(Self {
            objects,
            textures,
            loaded_models,
            failed_models,
            failed_textures,
        })
    }

    pub fn load_test_cell(game: &Game) -> Result<Self> {
        // NIF is Z-up; rotate around Z here so the render-space Y/Z swap
        // turns it into a yaw instead of a sideways tilt.
        struct TestModel {
            path: &'static str,
            origin: [f32; 3],
            rotation: [f32; 3],
            scale: f32,
            sink: f32,
        }
        const MODELS: &[TestModel] = &[
            TestModel {
                path: "meshes\\architecture\\farmhouse\\farmhouse01.nif",
                origin: [600.0, 600.0, 0.0],
                rotation: [0.0, 0.0, 0.6],
                scale: 1.0,
                sink: 0.0,
            },
            TestModel {
                path: "meshes\\architecture\\farmhouse\\farmhouse01walkway.nif",
                origin: [1400.0, 600.0, 0.0],
                rotation: [0.0, 0.0, 0.6],
                scale: 1.0,
                sink: 0.0,
            },
            TestModel {
                path: "meshes\\landscape\\rocks\\wetrocks\\rockl01wet.nif",
                origin: [1800.0, 1400.0, 0.0],
                rotation: [0.0, 0.0, 0.0],
                scale: 1.0,
                sink: 0.0,
            },
            TestModel {
                path: "meshes\\landscape\\trees\\treepineforest01.nif",
                origin: [1000.0, 1800.0, 0.0],
                rotation: [0.0, 0.0, 0.0],
                scale: 1.0,
                sink: 0.0,
            },
            TestModel {
                path: "meshes\\actors\\character\\character assets\\femalebodyastrid_0.nif",
                origin: [2000.0, 2000.0, 0.0],
                rotation: [0.0, 0.0, 0.0],
                scale: 1.0,
                sink: 0.0,
            },
            TestModel {
                path: "meshes\\architecture\\farmhouse\\farmhouse01.nif",
                origin: [2800.0, 3000.0, 0.0],
                rotation: [0.0, 0.0, 0.0],
                scale: 1.0,
                sink: 310.0,
            },
        ];

        let mut objects = Vec::new();
        let mut loaded_models = 0;
        let mut failed_models = 0;

        for model in MODELS {
            let bytes = match game.read_asset(model.path) {
                Ok(Some(bytes)) => bytes,
                _ => {
                    failed_models += 1;
                    continue;
                }
            };
            let nif = match Nif::parse(&bytes) {
                Ok(nif) => nif,
                Err(_) => {
                    failed_models += 1;
                    continue;
                }
            };
            loaded_models += 1;
            let rotation = Quat::from_euler(
                EulerRot::XYZ,
                model.rotation[0],
                model.rotation[1],
                model.rotation[2],
            );
            let parts = nif.meshes();
            // Put the model's lowest point on the flat test floor.
            let min_height = parts
                .iter()
                .flat_map(|part| part.vertices.iter())
                .map(|vertex| (rotation * (Vec3::from_array(vertex.position) * model.scale)).z)
                .fold(f32::INFINITY, f32::min);
            let mut origin = Vec3::from_array(model.origin);
            if min_height.is_finite() {
                origin.z -= min_height;
            }
            origin.z -= model.sink;
            for part in parts {
                push_part(&mut objects, part, rotation, origin, model.scale);
            }
        }

        let (textures, failed_textures) = load_textures(game, &objects)?;
        Ok(Self {
            objects,
            textures,
            loaded_models,
            failed_models,
            failed_textures,
        })
    }
}

fn push_part(
    objects: &mut Vec<SceneObject>,
    part: oce_nif::MeshPart,
    rotation: Quat,
    origin: Vec3,
    scale: f32,
) {
    let mut vertices = part.vertices;
    for vertex in &mut vertices {
        let position = rotation * (Vec3::from_array(vertex.position) * scale) + origin;
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

fn load_textures(
    game: &Game,
    objects: &[SceneObject],
) -> Result<(HashMap<String, TextureData>, usize)> {
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
    Ok((textures, failed_textures))
}

struct Reference {
    base: u32,
    position: [f32; 3],
    rotation: [f32; 3],
    scale: f32,
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
