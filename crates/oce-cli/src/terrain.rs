use std::collections::HashMap;

use glam::Vec3;
use oce_core::{Error, Result};
use oce_plugin::{Plugin, Record, WalkEntry};

const TAMRIEL: u32 = 0x0000_003C;
const CELL_SIZE: f32 = 4096.0;
const HEIGHT_STEP: f32 = 128.0;
const HEIGHT_SCALE: f32 = 8.0;

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct Vertex {
    pub position: [f32; 3],
    pub normal: [f32; 3],
    pub color: [f32; 3],
}

pub struct TerrainCell {
    pub x: i32,
    pub y: i32,
    pub heights: [f32; 1089],
}

pub struct Terrain {
    cells: HashMap<(i32, i32), TerrainCell>,
    min_x: f32,
    max_x: f32,
    min_z: f32,
    max_z: f32,
    min_height: f32,
    max_height: f32,
}

pub struct TerrainMesh {
    pub vertices: Vec<Vertex>,
    pub indices: Vec<u32>,
}

impl Terrain {
    pub fn load(plugin: &Plugin, center: (i32, i32), radius: i32) -> Result<Self> {
        let mut cells: HashMap<(i32, i32), TerrainCell> = HashMap::new();
        let mut world = None;
        let mut current: Option<(i32, i32)> = None;

        plugin.walk(|entry| {
            match entry {
                WalkEntry::Record {
                    header,
                    body,
                    offset,
                    ..
                } => {
                    if &header.signature == b"WRLD" {
                        world = Some(header.form_id.raw());
                        current = None;
                    } else if &header.signature == b"CELL" {
                        current = None;
                        if world == Some(TAMRIEL) {
                            let record = Record {
                                header,
                                body,
                                offset,
                            };
                            let decoded = record.decode()?;
                            if let Some((x, y)) = cell_grid(&decoded.subrecords()?) {
                                if (x - center.0).abs() <= radius && (y - center.1).abs() <= radius
                                {
                                    current = Some((x, y));
                                }
                            }
                        }
                    } else if &header.signature == b"LAND" {
                        if let Some((x, y)) = current {
                            let record = Record {
                                header,
                                body,
                                offset,
                            };
                            let decoded = record.decode()?;
                            if let Some(heights) = landscape_heights(&decoded.subrecords()?) {
                                cells.insert((x, y), TerrainCell { x, y, heights });
                            }
                        }
                    }
                }
                WalkEntry::Group { depth: 0, .. } => {
                    world = None;
                    current = None;
                }
                WalkEntry::Group { .. } => {}
            }
            Ok(())
        })?;

        if cells.is_empty() {
            return Err(Error::invalid(
                "no exterior LAND data found for the requested cells",
            ));
        }

        let min_cell_x = cells.keys().map(|(x, _)| *x).min().unwrap();
        let max_cell_x = cells.keys().map(|(x, _)| *x).max().unwrap();
        let min_cell_y = cells.keys().map(|(_, y)| *y).min().unwrap();
        let max_cell_y = cells.keys().map(|(_, y)| *y).max().unwrap();
        let min_height = cells
            .values()
            .flat_map(|cell| cell.heights)
            .fold(f32::INFINITY, f32::min);
        let max_height = cells
            .values()
            .flat_map(|cell| cell.heights)
            .fold(f32::NEG_INFINITY, f32::max);

        Ok(Self {
            cells,
            min_x: min_cell_x as f32 * CELL_SIZE,
            max_x: (max_cell_x + 1) as f32 * CELL_SIZE,
            min_z: min_cell_y as f32 * CELL_SIZE,
            max_z: (max_cell_y + 1) as f32 * CELL_SIZE,
            min_height,
            max_height,
        })
    }

    pub fn cell_count(&self) -> usize {
        self.cells.len()
    }

    pub fn height_range(&self) -> (f32, f32) {
        (
            self.min_height * HEIGHT_SCALE,
            self.max_height * HEIGHT_SCALE,
        )
    }

    pub fn bounds(&self) -> (f32, f32, f32, f32) {
        (self.min_x, self.max_x, self.min_z, self.max_z)
    }

    pub fn flat() -> Self {
        Self {
            cells: HashMap::from([(
                (0, 0),
                TerrainCell {
                    x: 0,
                    y: 0,
                    heights: [0.0; 1089],
                },
            )]),
            min_x: 0.0,
            max_x: CELL_SIZE,
            min_z: 0.0,
            max_z: CELL_SIZE,
            min_height: 0.0,
            max_height: 0.0,
        }
    }

    pub fn height_at(&self, x: f32, z: f32) -> Option<f32> {
        let cell_x = (x / CELL_SIZE).floor() as i32;
        let cell_y = (z / CELL_SIZE).floor() as i32;
        let cell = self.cells.get(&(cell_x, cell_y))?;
        let local_x = ((x - cell_x as f32 * CELL_SIZE) / HEIGHT_STEP).clamp(0.0, 32.0);
        let local_z = ((z - cell_y as f32 * CELL_SIZE) / HEIGHT_STEP).clamp(0.0, 32.0);
        let ix = local_x.floor().min(31.0) as usize;
        let iz = local_z.floor().min(31.0) as usize;
        let fx = local_x - ix as f32;
        let fz = local_z - iz as f32;
        let h00 = cell.heights[iz * 33 + ix];
        let h10 = cell.heights[iz * 33 + ix + 1];
        let h01 = cell.heights[(iz + 1) * 33 + ix];
        let h11 = cell.heights[(iz + 1) * 33 + ix + 1];
        let top = h00 + (h10 - h00) * fx;
        let bottom = h01 + (h11 - h01) * fx;
        Some((top + (bottom - top) * fz) * HEIGHT_SCALE)
    }

    pub fn mesh(&self) -> TerrainMesh {
        let mut vertices = Vec::with_capacity(self.cells.len() * 1089);
        let mut indices = Vec::with_capacity(self.cells.len() * 32 * 32 * 6);
        let mut cells: Vec<&TerrainCell> = self.cells.values().collect();
        cells.sort_by_key(|cell| (cell.y, cell.x));

        for cell in cells {
            let base = vertices.len() as u32;
            let world_x = cell.x as f32 * CELL_SIZE;
            let world_z = cell.y as f32 * CELL_SIZE;
            for iz in 0..33 {
                for ix in 0..33 {
                    let height = cell.heights[iz * 33 + ix] * HEIGHT_SCALE;
                    let hx0 = cell.heights[iz * 33 + ix.saturating_sub(1)];
                    let hx1 = cell.heights[iz * 33 + (ix + 1).min(32)];
                    let hz0 = cell.heights[iz.saturating_sub(1) * 33 + ix];
                    let hz1 = cell.heights[(iz + 1).min(32) * 33 + ix];
                    let normal = Vec3::new(
                        -(hx1 - hx0) * HEIGHT_SCALE / (2.0 * HEIGHT_STEP),
                        1.0,
                        -(hz1 - hz0) * HEIGHT_SCALE / (2.0 * HEIGHT_STEP),
                    )
                    .normalize();
                    let t = ((height - self.min_height * HEIGHT_SCALE)
                        / ((self.max_height - self.min_height) * HEIGHT_SCALE).max(1.0))
                    .clamp(0.0, 1.0);
                    let slope = (1.0 - normal.y).clamp(0.0, 1.0);
                    let rock = smoothstep(0.18, 0.55, slope);
                    let mut color =
                        Vec3::new(0.20, 0.31, 0.14).lerp(Vec3::new(0.38, 0.35, 0.30), rock);
                    if t > 0.78 && slope < 0.35 {
                        color = color.lerp(Vec3::new(0.76, 0.79, 0.82), (t - 0.78) / 0.22);
                    }
                    vertices.push(Vertex {
                        position: [
                            world_x + ix as f32 * HEIGHT_STEP,
                            height,
                            world_z + iz as f32 * HEIGHT_STEP,
                        ],
                        normal: normal.to_array(),
                        color: color.to_array(),
                    });
                }
            }
            for iz in 0..32 {
                for ix in 0..32 {
                    let i = base + (iz * 33 + ix) as u32;
                    let right = i + 1;
                    let down = i + 33;
                    let down_right = down + 1;
                    indices.extend_from_slice(&[i, down, right, right, down, down_right]);
                }
            }
        }

        TerrainMesh { vertices, indices }
    }
}

fn cell_grid(subrecords: &[oce_plugin::Subrecord<'_>]) -> Option<(i32, i32)> {
    let xclc = subrecords.iter().find(|sub| &sub.signature == b"XCLC")?;
    (xclc.data.len() >= 8).then(|| {
        (
            i32::from_le_bytes(xclc.data[0..4].try_into().unwrap()),
            i32::from_le_bytes(xclc.data[4..8].try_into().unwrap()),
        )
    })
}

fn landscape_heights(subrecords: &[oce_plugin::Subrecord<'_>]) -> Option<[f32; 1089]> {
    let vhgt = subrecords.iter().find(|sub| &sub.signature == b"VHGT")?;
    if vhgt.data.len() < 4 + 1089 {
        return None;
    }
    let offset = f32::from_le_bytes(vhgt.data[0..4].try_into().unwrap());
    let mut heights = [0.0; 1089];
    let mut row_base = offset;
    for z in 0..33 {
        let row = z * 33;
        row_base += vhgt.data[4 + row] as i8 as f32;
        heights[row] = row_base;

        let mut east = row_base;
        for x in 1..33 {
            east += vhgt.data[4 + row + x] as i8 as f32;
            heights[row + x] = east;
        }
    }
    Some(heights)
}

fn smoothstep(edge0: f32, edge1: f32, x: f32) -> f32 {
    let t = ((x - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;

    #[test]
    fn real_landscape_builds_a_mesh() {
        let Ok(root) = std::env::var("OCE_SKYRIM_SE") else {
            return;
        };
        let path = Path::new(&root).join("Data/Skyrim.esm");
        if !path.is_file() {
            return;
        }
        let plugin = Plugin::open(path).unwrap();
        let terrain = Terrain::load(&plugin, (0, 0), 1).unwrap();
        assert!(terrain.cell_count() >= 9);
        let mesh = terrain.mesh();
        assert!(mesh.vertices.len() > 9 * 1000);
        assert_eq!(mesh.indices.len(), terrain.cell_count() * 32 * 32 * 6);
        let (min, max) = terrain.height_range();
        assert!(max > min);
        let height = terrain.height_at(2048.0, 2048.0).unwrap();
        assert!(height.is_finite());
    }
}
