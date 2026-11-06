use glam::{Vec2, Vec3};

use crate::scene::Scene;
use crate::settings::GameSettings;
use crate::terrain::Terrain;

const TERRAIN_PROBE: f32 = 64.0;

pub struct ControllerConfig {
    pub radius: f32,
    pub height: f32,
    pub eye_height: f32,
    pub step_height: f32,
    pub max_slope_cos: f32,
    pub gravity: f32,
    pub walk_speed: f32,
    pub run_speed: f32,
    pub jump_height: f32,
}

impl Default for ControllerConfig {
    fn default() -> Self {
        Self {
            radius: 32.0,
            height: 128.0,
            eye_height: 120.0,
            step_height: 32.0,
            max_slope_cos: 0.5,
            gravity: 686.0,
            walk_speed: 85.0,
            run_speed: 370.0,
            jump_height: 64.0,
        }
    }
}

impl ControllerConfig {
    pub fn from_settings(settings: &GameSettings) -> Self {
        Self {
            walk_speed: settings.f32("fMoveCharWalkBase", 85.0),
            run_speed: settings.f32("fMoveCharRunBase", 370.0),
            jump_height: settings.f32("fJumpHeightMin", 64.0),
            ..Self::default()
        }
    }
}

pub struct CharacterController {
    pub position: Vec3,
    pub velocity_y: f32,
    pub on_ground: bool,
    config: ControllerConfig,
    jump_speed: f32,
    jump_held: bool,
}

impl CharacterController {
    pub fn new(terrain: &Terrain, config: ControllerConfig) -> Self {
        let (min_x, max_x, min_z, max_z) = terrain.bounds();
        let x = (min_x + max_x) * 0.5;
        let z = (min_z + max_z) * 0.5;
        let ground = terrain.height_at(x, z).unwrap_or(0.0);
        let jump_speed = (2.0 * config.gravity * config.jump_height).sqrt();
        Self {
            position: Vec3::new(x, ground, z),
            velocity_y: 0.0,
            on_ground: true,
            config,
            jump_speed,
            jump_held: false,
        }
    }

    pub fn eye(&self) -> Vec3 {
        self.position + Vec3::Y * self.config.eye_height
    }

    pub fn update(
        &mut self,
        dt: f32,
        direction: Vec3,
        running: bool,
        jump: bool,
        terrain: &Terrain,
        world: &CollisionWorld,
    ) {
        let dt = dt.clamp(0.0, 0.1);
        if dt == 0.0 {
            return;
        }
        let mut wish = Vec3::new(direction.x, 0.0, direction.z);
        if wish.length_squared() > 1e-6 {
            let speed = if running {
                self.config.run_speed
            } else {
                self.config.walk_speed
            };
            wish = wish.normalize() * speed;
        }

        if jump && !self.jump_held && self.on_ground {
            self.velocity_y = self.jump_speed;
            self.on_ground = false;
        }
        self.jump_held = jump;
        self.velocity_y -= self.config.gravity * dt;

        let start = self.position;
        let horizontal = wish * dt;
        let displacement = horizontal + Vec3::Y * (self.velocity_y * dt);
        let steps = (displacement.length() / (self.config.radius * 0.25))
            .ceil()
            .clamp(1.0, 8.0) as usize;
        let step = displacement / steps as f32;
        let max_correction = self.config.radius.max(displacement.length());
        let mut correction = Vec3::ZERO;

        for _ in 0..steps {
            let previous = self.position;
            self.position += step;
            self.resolve_terrain(previous, terrain);
            let desired = self.position;
            let remaining = (max_correction - correction.length()).max(0.0);
            let (resolved, blocked) = world.resolve_capsule(
                start,
                desired,
                self.config.radius,
                self.config.height,
                self.config.max_slope_cos,
                remaining,
            );
            let falling = !self.on_ground && self.velocity_y <= 0.0;
            if blocked && !falling {
                self.position = previous;
                break;
            }
            self.position = resolved;
            correction += self.position - desired;
        }

        let (min_x, max_x, min_z, max_z) = terrain.bounds();
        self.position.x = self
            .position
            .x
            .clamp(min_x + self.config.radius, max_x - self.config.radius);
        self.position.z = self
            .position
            .z
            .clamp(min_z + self.config.radius, max_z - self.config.radius);

        self.on_ground = false;
        if self.velocity_y <= 0.0 {
            let moved = Vec2::new(self.position.x - start.x, self.position.z - start.z).length();
            let slope = self.config.max_slope_cos.max(0.1);
            let fall = (-self.velocity_y * dt).max(0.0);
            let climb = moved / slope;
            let tan = (1.0 - slope * slope).max(0.0).sqrt() / slope;
            let rise = 0.05 + fall + climb;
            let drop = (0.6 + moved * tan).min(self.config.step_height * 2.0);
            let previous_ground = terrain.height_at(start.x, start.z);
            let terrain_continuous =
                previous_ground.is_some_and(|ground| (start.y - ground).abs() <= 0.6);
            let terrain_drop = if terrain_continuous { drop } else { 0.6 };
            let terrain_support =
                terrain
                    .height_at(self.position.x, self.position.z)
                    .filter(|ground| {
                        *ground <= self.position.y + rise
                            && *ground >= self.position.y - terrain_drop
                    });
            let static_support = world.support_height(SupportProbe {
                previous: start,
                point: Vec3::new(self.position.x, self.position.y, self.position.z),
                max_rise: rise,
                max_drop: drop,
                min_normal_y: self.config.max_slope_cos,
            });
            let support = match (terrain_support, static_support) {
                (Some(terrain), Some(static_support)) => Some(terrain.max(static_support)),
                (Some(terrain), None) => Some(terrain),
                (None, Some(static_support)) => Some(static_support),
                (None, None) => None,
            };
            if let Some(support) = support {
                self.position.y = support;
                self.velocity_y = 0.0;
                self.on_ground = true;
            }
        }
    }

    fn resolve_terrain(&mut self, previous: Vec3, terrain: &Terrain) {
        let Some(ground) = terrain.height_at(self.position.x, self.position.z) else {
            return;
        };
        let previous_ground = terrain.height_at(previous.x, previous.z).unwrap_or(ground);

        let rising = ground > previous_ground + 0.01;
        if rising && ground > previous.y + self.config.step_height {
            self.position.x = previous.x;
            self.position.z = previous.z;
            return;
        }
        if rising && self.velocity_y <= 0.0 {
            let normal = terrain_normal(terrain, self.position.x, self.position.z);
            if normal.y < self.config.max_slope_cos {
                self.position.x = previous.x;
                self.position.z = previous.z;
                return;
            }
        }

        if self.position.y < ground {
            if self.velocity_y > 0.0 {
                self.position.x = previous.x;
                self.position.z = previous.z;
                return;
            }
            self.position.y = ground;
            if self.velocity_y < 0.0 {
                self.velocity_y = 0.0;
            }
        }
    }
}

fn terrain_normal(terrain: &Terrain, x: f32, z: f32) -> Vec3 {
    let center = terrain.height_at(x, z).unwrap_or(0.0);
    let left = terrain.height_at(x - TERRAIN_PROBE, z).unwrap_or(center);
    let right = terrain.height_at(x + TERRAIN_PROBE, z).unwrap_or(center);
    let back = terrain.height_at(x, z - TERRAIN_PROBE).unwrap_or(center);
    let front = terrain.height_at(x, z + TERRAIN_PROBE).unwrap_or(center);
    Vec3::new(-(right - left), TERRAIN_PROBE * 2.0, -(front - back)).normalize()
}

struct SupportProbe {
    previous: Vec3,
    point: Vec3,
    max_rise: f32,
    max_drop: f32,
    min_normal_y: f32,
}

pub struct CollisionWorld {
    triangles: Vec<Triangle>,
    objects: usize,
}

struct Triangle {
    a: Vec3,
    b: Vec3,
    c: Vec3,
    normal: Vec3,
    min: Vec3,
    max: Vec3,
}

impl Triangle {
    fn new(a: Vec3, b: Vec3, c: Vec3) -> Option<Self> {
        let cross = (b - a).cross(c - a);
        let length = cross.length();
        if length < 1e-4 {
            return None;
        }
        Some(Self {
            a,
            b,
            c,
            normal: cross / length,
            min: a.min(b).min(c),
            max: a.max(b).max(c),
        })
    }

    fn overlaps(&self, center: Vec3, radius: f32) -> bool {
        center.x + radius >= self.min.x
            && center.x - radius <= self.max.x
            && center.y + radius >= self.min.y
            && center.y - radius <= self.max.y
            && center.z + radius >= self.min.z
            && center.z - radius <= self.max.z
    }
}

impl CollisionWorld {
    pub fn build(scene: &Scene) -> Self {
        let mut triangles = Vec::new();
        let mut objects = 0;
        for object in &scene.objects {
            let mut added = false;
            for index in object.indices.as_chunks::<3>().0 {
                let Some(a) = object
                    .vertices
                    .get(index[0] as usize)
                    .map(|vertex| Vec3::from_array(vertex.position))
                else {
                    continue;
                };
                let Some(b) = object
                    .vertices
                    .get(index[1] as usize)
                    .map(|vertex| Vec3::from_array(vertex.position))
                else {
                    continue;
                };
                let Some(c) = object
                    .vertices
                    .get(index[2] as usize)
                    .map(|vertex| Vec3::from_array(vertex.position))
                else {
                    continue;
                };
                if let Some(triangle) = Triangle::new(a, b, c) {
                    triangles.push(triangle);
                    added = true;
                }
            }
            if added {
                objects += 1;
            }
        }
        Self { triangles, objects }
    }

    pub fn triangle_count(&self) -> usize {
        self.triangles.len()
    }

    pub fn object_count(&self) -> usize {
        self.objects
    }

    fn support_height(&self, probe: SupportProbe) -> Option<f32> {
        let SupportProbe {
            previous,
            point,
            max_rise,
            max_drop,
            min_normal_y,
        } = probe;
        let top = point.y + max_rise.max(0.0);
        let bottom = point.y - max_drop.max(0.0);
        let mut best = None;
        for triangle in &self.triangles {
            if triangle.normal.y.abs() < min_normal_y {
                continue;
            }
            let Some(height) = triangle_height_at(triangle, point.x, point.z) else {
                continue;
            };
            if height > top || height < bottom {
                continue;
            }
            if height < point.y - 0.6 {
                let Some(previous_height) = triangle_height_at(triangle, previous.x, previous.z)
                else {
                    continue;
                };
                if (previous.y - previous_height).abs() > 0.6 {
                    continue;
                }
            }
            if height > point.y + 0.05 {
                let Some(previous_height) = triangle_height_at(triangle, previous.x, previous.z)
                else {
                    continue;
                };
                if previous_height > previous.y + 0.05 {
                    continue;
                }
            }
            best = Some(best.map_or(height, |best: f32| best.max(height)));
        }
        best
    }

    pub fn resolve_capsule(
        &self,
        previous: Vec3,
        position: Vec3,
        radius: f32,
        height: f32,
        max_slope_cos: f32,
        max_displacement: f32,
    ) -> (Vec3, bool) {
        if max_displacement <= 0.0 {
            return (previous, true);
        }
        let bottom = radius;
        let top = (height - radius).max(bottom);
        let samples = (((top - bottom) / (radius * 0.5)).ceil() as usize + 1).max(1);
        let start = position;
        let mut corrected = position;
        let mut blocked = false;

        for _ in 0..4 {
            let mut deepest: Option<(f32, Vec3)> = None;
            for sample in 0..samples {
                let t = if samples == 1 {
                    0.0
                } else {
                    sample as f32 / (samples - 1) as f32
                };
                let y_offset = bottom + (top - bottom) * t;
                let center = corrected + Vec3::Y * y_offset;
                let previous_center = previous + Vec3::Y * y_offset;
                for triangle in &self.triangles {
                    if !triangle.overlaps(center, radius) {
                        continue;
                    }
                    let closest =
                        closest_point_on_triangle(center, triangle.a, triangle.b, triangle.c);
                    let offset = center - closest;
                    let distance_sq = offset.length_squared();
                    if distance_sq >= radius * radius {
                        continue;
                    }
                    let distance = distance_sq.sqrt();
                    let previous_offset = previous_center - closest;
                    let direction = if distance > 1e-4 {
                        if previous_offset.length_squared() > 1e-8
                            && offset.dot(previous_offset) < 0.0
                        {
                            -offset / distance
                        } else {
                            offset / distance
                        }
                    } else if previous_offset.length_squared() > 1e-8 {
                        previous_offset.normalize()
                    } else {
                        continue;
                    };
                    if direction.y >= max_slope_cos {
                        match triangle_height_at(triangle, previous.x, previous.z) {
                            Some(height) if height <= previous.y + 0.05 => continue,
                            Some(_) => {
                                blocked = true;
                                continue;
                            }
                            None => {}
                        }
                    }
                    let penetration = radius - distance;
                    if deepest.is_none_or(|(best, _)| penetration > best) {
                        // A steep surface must not push upward: that cancels gravity
                        // and lets the capsule hover. Push horizontally instead.
                        let push = if direction.y > 0.05 && direction.y < max_slope_cos {
                            let horizontal = Vec3::new(direction.x, 0.0, direction.z);
                            if horizontal.length_squared() > 1e-8 {
                                horizontal.normalize()
                            } else {
                                direction
                            }
                        } else {
                            direction
                        };
                        deepest = Some((penetration, push * penetration));
                    }
                }
            }
            let Some((_, push)) = deepest else {
                break;
            };
            corrected += push;
            let correction = corrected - start;
            if correction.length() > max_displacement {
                corrected = start + correction.normalize() * max_displacement;
            }
        }

        (corrected, blocked)
    }
}

fn triangle_height_at(triangle: &Triangle, x: f32, z: f32) -> Option<f32> {
    let a = Vec2::new(triangle.a.x, triangle.a.z);
    let b = Vec2::new(triangle.b.x, triangle.b.z);
    let c = Vec2::new(triangle.c.x, triangle.c.z);
    let point = Vec2::new(x, z);
    let v0 = b - a;
    let v1 = c - a;
    let v2 = point - a;
    let denom = v0.x * v1.y - v1.x * v0.y;
    if denom.abs() < 1e-6 {
        return None;
    }
    let u = (v2.x * v1.y - v1.x * v2.y) / denom;
    let v = (v0.x * v2.y - v2.x * v0.y) / denom;
    if u < -1e-4 || v < -1e-4 || u + v > 1.0 + 1e-4 {
        return None;
    }
    Some(triangle.a.y + u * (triangle.b.y - triangle.a.y) + v * (triangle.c.y - triangle.a.y))
}

fn closest_point_on_triangle(point: Vec3, a: Vec3, b: Vec3, c: Vec3) -> Vec3 {
    let ab = b - a;
    let ac = c - a;
    let ap = point - a;
    let d1 = ab.dot(ap);
    let d2 = ac.dot(ap);
    if d1 <= 0.0 && d2 <= 0.0 {
        return a;
    }

    let bp = point - b;
    let d3 = ab.dot(bp);
    let d4 = ac.dot(bp);
    if d3 >= 0.0 && d4 <= d3 {
        return b;
    }

    let vc = d1 * d4 - d3 * d2;
    if vc <= 0.0 && d1 >= 0.0 && d3 <= 0.0 {
        let v = d1 / (d1 - d3);
        return a + ab * v;
    }

    let cp = point - c;
    let d5 = ab.dot(cp);
    let d6 = ac.dot(cp);
    if d6 >= 0.0 && d5 <= d6 {
        return c;
    }

    let vb = d5 * d2 - d1 * d6;
    if vb <= 0.0 && d2 >= 0.0 && d6 <= 0.0 {
        let w = d2 / (d2 - d6);
        return a + ac * w;
    }

    let va = d3 * d6 - d5 * d4;
    if va <= 0.0 && (d4 - d3) >= 0.0 && (d5 - d6) >= 0.0 {
        let w = (d4 - d3) / ((d4 - d3) + (d5 - d6));
        return b + (c - b) * w;
    }

    let denom = 1.0 / (va + vb + vc);
    let v = vb * denom;
    let w = vc * denom;
    a + ab * v + ac * w
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;
    use crate::scene::{MeshVertex, SceneObject};

    fn scene_with_triangle() -> Scene {
        Scene {
            objects: vec![SceneObject {
                vertices: vec![
                    MeshVertex {
                        position: [-100.0, 0.0, -100.0],
                        normal: [0.0, 1.0, 0.0],
                        uv: [0.0, 0.0],
                        color: [1.0, 1.0, 1.0],
                    },
                    MeshVertex {
                        position: [100.0, 0.0, -100.0],
                        normal: [0.0, 1.0, 0.0],
                        uv: [1.0, 0.0],
                        color: [1.0, 1.0, 1.0],
                    },
                    MeshVertex {
                        position: [0.0, 0.0, 100.0],
                        normal: [0.0, 1.0, 0.0],
                        uv: [0.5, 1.0],
                        color: [1.0, 1.0, 1.0],
                    },
                ],
                indices: vec![0, 1, 2],
                texture: None,
            }],
            textures: HashMap::new(),
            loaded_models: 0,
            failed_models: 0,
            failed_textures: 0,
        }
    }

    #[test]
    fn closest_point_inside_triangle_lies_on_plane() {
        let point = Vec3::new(0.0, 10.0, 0.0);
        let closest = closest_point_on_triangle(
            point,
            Vec3::new(-10.0, 0.0, -10.0),
            Vec3::new(10.0, 0.0, -10.0),
            Vec3::new(0.0, 0.0, 10.0),
        );
        assert!(closest.y.abs() < 1e-4);
    }
    #[test]
    fn capsule_leaves_walkable_floor_to_support_query() {
        let world = CollisionWorld::build(&scene_with_triangle());
        let position = Vec3::ZERO;
        let (corrected, blocked) =
            world.resolve_capsule(position, position, 32.0, 128.0, 0.5, 16.0);
        assert_eq!(corrected, position);
        assert!(!blocked);
        assert_eq!(
            world.support_height(SupportProbe {
                previous: Vec3::ZERO,
                point: Vec3::ZERO,
                max_rise: 0.05,
                max_drop: 0.6,
                min_normal_y: 0.5,
            }),
            Some(0.0)
        );
    }

    fn flat_terrain() -> Terrain {
        Terrain::flat()
    }

    #[test]
    fn support_query_ignores_surface_above_origin() {
        let world = CollisionWorld {
            triangles: vec![Triangle::new(
                Vec3::new(-100.0, 0.2, -100.0),
                Vec3::new(100.0, 0.2, -100.0),
                Vec3::new(0.0, 0.2, 100.0),
            )
            .unwrap()],
            objects: 1,
        };
        assert_eq!(
            world.support_height(SupportProbe {
                previous: Vec3::ZERO,
                point: Vec3::ZERO,
                max_rise: 0.0,
                max_drop: 0.5,
                min_normal_y: 0.5,
            }),
            None
        );
        assert_eq!(
            world.support_height(SupportProbe {
                previous: Vec3::ZERO,
                point: Vec3::new(0.0, 0.25, 0.0),
                max_rise: 0.0,
                max_drop: 0.5,
                min_normal_y: 0.5,
            }),
            Some(0.2)
        );
    }

    #[test]
    fn support_query_finds_floor_below() {
        let world = CollisionWorld {
            triangles: vec![Triangle::new(
                Vec3::new(-100.0, -0.2, -100.0),
                Vec3::new(100.0, -0.2, -100.0),
                Vec3::new(0.0, -0.2, 100.0),
            )
            .unwrap()],
            objects: 1,
        };
        assert_eq!(
            world.support_height(SupportProbe {
                previous: Vec3::ZERO,
                point: Vec3::ZERO,
                max_rise: 0.05,
                max_drop: 0.6,
                min_normal_y: 0.5,
            }),
            Some(-0.2)
        );
    }

    #[test]
    fn support_query_rejects_surface_above_previous_feet() {
        let world = CollisionWorld {
            triangles: vec![Triangle::new(
                Vec3::new(-100.0, 5.0, -100.0),
                Vec3::new(0.0, 5.0, 100.0),
                Vec3::new(100.0, 5.0, -100.0),
            )
            .unwrap()],
            objects: 1,
        };
        assert_eq!(
            world.support_height(SupportProbe {
                previous: Vec3::ZERO,
                point: Vec3::ZERO,
                max_rise: 20.0,
                max_drop: 0.6,
                min_normal_y: 0.5,
            }),
            None
        );
    }

    #[test]
    fn capsule_correction_is_capped_inside_a_thin_triangle() {
        let world = CollisionWorld {
            triangles: vec![Triangle::new(
                Vec3::new(0.0, 0.0, -100.0),
                Vec3::new(0.0, 400.0, 100.0),
                Vec3::new(0.0, 0.0, 100.0),
            )
            .unwrap()],
            objects: 1,
        };
        let position = Vec3::ZERO;
        let previous = Vec3::new(10.0, 0.0, 0.0);
        let (corrected, blocked) =
            world.resolve_capsule(previous, position, 32.0, 128.0, 0.5, 16.0);
        assert!(!blocked);
        let moved = (corrected - position).length();
        assert!(moved <= 16.5, "moved {moved}");
    }

    fn real_terrain() -> Option<Terrain> {
        use std::path::Path;

        use oce_plugin::Plugin;

        let root = std::env::var("OCE_SKYRIM_SE").ok()?;
        let path = Path::new(&root).join("Data/Skyrim.esm");
        path.is_file()
            .then(|| Terrain::load(&Plugin::open(path).unwrap(), (0, 0), 1).unwrap())
    }

    #[test]
    fn controller_stands_and_jumps_on_real_terrain() {
        let Some(terrain) = real_terrain() else {
            return;
        };
        let world = CollisionWorld {
            triangles: Vec::new(),
            objects: 0,
        };
        let mut controller = CharacterController::new(&terrain, ControllerConfig::default());
        let start = controller.position;
        for _ in 0..120 {
            controller.update(1.0 / 60.0, Vec3::ZERO, false, false, &terrain, &world);
        }
        assert!(controller.on_ground);
        assert!((controller.position.y - start.y).abs() < 1.0);

        controller.update(1.0 / 60.0, Vec3::ZERO, false, true, &terrain, &world);
        assert!(controller.velocity_y > 0.0);
        for _ in 0..180 {
            controller.update(1.0 / 60.0, Vec3::ZERO, false, false, &terrain, &world);
        }
        assert!(controller.on_ground);
    }

    #[test]
    fn controller_stops_at_a_static_wall() {
        let terrain = flat_terrain();
        let mut controller = CharacterController::new(&terrain, ControllerConfig::default());
        let start = controller.position;
        let wall_x = start.x + 128.0;
        let wall = Triangle::new(
            Vec3::new(wall_x, start.y - 100.0, start.z - 200.0),
            Vec3::new(wall_x, start.y - 100.0, start.z + 200.0),
            Vec3::new(wall_x, start.y + 400.0, start.z),
        )
        .unwrap();
        let world = CollisionWorld {
            triangles: vec![wall],
            objects: 1,
        };
        for _ in 0..120 {
            controller.update(1.0 / 60.0, Vec3::X, false, false, &terrain, &world);
        }
        assert!(
            controller.position.x > start.x + 1.0,
            "controller did not move: x={} start_x={}",
            controller.position.x,
            start.x
        );
        assert!(
            controller.position.x < wall_x - 30.0,
            "controller crossed wall: x={} wall_x={}",
            controller.position.x,
            wall_x
        );
    }

    #[test]
    fn controller_is_blocked_by_a_low_ledge() {
        let terrain = flat_terrain();
        let mut controller = CharacterController::new(&terrain, ControllerConfig::default());
        let start = controller.position;
        let step_x = start.x + 128.0;
        let local_ground = terrain.height_at(step_x, start.z).unwrap_or(start.y);
        let top = local_ground + 20.0;
        let z0 = start.z - 200.0;
        let z1 = start.z + 200.0;
        let back = Vec3::new(step_x, top, z0);
        let right = Vec3::new(step_x + 300.0, top, z0);
        let right_front = Vec3::new(step_x + 300.0, top, z1);
        let front = Vec3::new(step_x, top, z1);
        let face = Triangle::new(
            Vec3::new(step_x, local_ground - 100.0, z0),
            Vec3::new(step_x, local_ground - 100.0, z1),
            Vec3::new(step_x, top, z0),
        )
        .unwrap();
        let top_a = Triangle::new(back, right, right_front).unwrap();
        let top_b = Triangle::new(back, right_front, front).unwrap();
        let world = CollisionWorld {
            triangles: vec![face, top_a, top_b],
            objects: 1,
        };
        for _ in 0..180 {
            controller.update(1.0 / 60.0, Vec3::X, false, false, &terrain, &world);
        }
        assert!(
            controller.position.x < step_x - 1.0,
            "controller crossed ledge: x={} step_x={}",
            controller.position.x,
            step_x
        );
        assert!(
            controller.position.y < top - 1.0,
            "controller climbed ledge: y={} top={}",
            controller.position.y,
            top
        );
    }

    #[test]
    fn controller_walks_up_a_walkable_static_ramp() {
        let terrain = flat_terrain();
        for dt in [1.0 / 30.0, 1.0 / 60.0, 1.0 / 240.0] {
            let mut controller = CharacterController::new(&terrain, ControllerConfig::default());
            let start = controller.position;
            let world = CollisionWorld {
                triangles: vec![Triangle::new(
                    Vec3::new(start.x, start.y, start.z - 200.0),
                    Vec3::new(start.x, start.y, start.z + 200.0),
                    Vec3::new(start.x + 400.0, start.y + 400.0, start.z),
                )
                .unwrap()],
                objects: 1,
            };
            let mut flips = 0;
            for _ in 0..60 {
                controller.update(dt, Vec3::X, false, false, &terrain, &world);
                if !controller.on_ground {
                    flips += 1;
                }
            }
            assert!(
                controller.position.x > start.x + 10.0,
                "did not climb at dt {dt}: x={}",
                controller.position.x
            );
            assert!(
                controller.position.y > 10.0,
                "y={} at dt {dt}",
                controller.position.y
            );
            assert_eq!(flips, 0, "ground flips at dt {dt}: {flips}");
        }
    }

    #[test]
    fn controller_walks_down_a_walkable_static_ramp() {
        let terrain = flat_terrain();
        for (dt, steps) in [(1.0 / 30.0, 2), (1.0 / 60.0, 4), (1.0 / 240.0, 16)] {
            let mut controller = CharacterController::new(&terrain, ControllerConfig::default());
            let start = controller.position;
            controller.position.y = 10.0;
            let world = CollisionWorld {
                triangles: vec![Triangle::new(
                    Vec3::new(start.x, 10.0, start.z - 200.0),
                    Vec3::new(start.x, 10.0, start.z + 200.0),
                    Vec3::new(start.x + 400.0, -390.0, start.z),
                )
                .unwrap()],
                objects: 1,
            };
            let mut flips = 0;
            for _ in 0..steps {
                controller.update(dt, Vec3::X, false, false, &terrain, &world);
                if !controller.on_ground {
                    flips += 1;
                }
            }
            assert!(
                controller.position.x > start.x + 5.0,
                "did not move at dt {dt}: x={}",
                controller.position.x
            );
            assert!(
                controller.position.y < 9.0 && controller.position.y > 2.0,
                "y={} at dt {dt}",
                controller.position.y
            );
            assert_eq!(flips, 0, "ground flips at dt {dt}: {flips}");
        }
    }

    #[test]
    fn controller_does_not_climb_an_overhead_surface_without_movement() {
        let terrain = flat_terrain();
        let mut controller = CharacterController::new(&terrain, ControllerConfig::default());
        let start = controller.position;
        let world = CollisionWorld {
            triangles: vec![Triangle::new(
                Vec3::new(start.x - 200.0, 10.0, start.z - 200.0),
                Vec3::new(start.x - 200.0, 10.0, start.z + 200.0),
                Vec3::new(start.x + 200.0, 10.0, start.z),
            )
            .unwrap()],
            objects: 1,
        };
        for _ in 0..60 {
            controller.update(1.0 / 60.0, Vec3::ZERO, false, false, &terrain, &world);
        }
        assert!(controller.on_ground);
        assert!(controller.position.y < 1.0, "y={}", controller.position.y);
    }

    #[test]
    fn controller_blocks_under_a_low_overhead() {
        let terrain = flat_terrain();
        for dt in [1.0 / 30.0, 1.0 / 60.0] {
            let mut controller = CharacterController::new(&terrain, ControllerConfig::default());
            let start = controller.position;
            let world = CollisionWorld {
                triangles: vec![Triangle::new(
                    Vec3::new(start.x - 200.0, 10.0, start.z - 200.0),
                    Vec3::new(start.x - 200.0, 10.0, start.z + 200.0),
                    Vec3::new(start.x + 200.0, 10.0, start.z),
                )
                .unwrap()],
                objects: 1,
            };
            for _ in 0..60 {
                controller.update(dt, Vec3::X, false, false, &terrain, &world);
            }
            assert!(
                controller.position.x < start.x + 0.1,
                "walked under overhead at dt {dt}: x={}",
                controller.position.x
            );
            assert!(controller.position.y < 1.0, "lifted at dt {dt}");
        }
    }

    #[test]
    fn controller_does_not_snap_down_a_ledge_at_run_speed() {
        let terrain = flat_terrain();
        for dt in [1.0_f32 / 30.0, 1.0 / 60.0, 1.0 / 240.0] {
            let mut controller = CharacterController::new(&terrain, ControllerConfig::default());
            let start = controller.position;
            controller.position.y = 10.0;
            let world = CollisionWorld {
                triangles: vec![Triangle::new(
                    Vec3::new(start.x - 20.0, 10.0, start.z - 200.0),
                    Vec3::new(start.x - 20.0, 10.0, start.z + 200.0),
                    Vec3::new(start.x + 20.0, 10.0, start.z),
                )
                .unwrap()],
                objects: 1,
            };
            let mut max_drop = 0.0_f32;
            let steps = (0.5 / dt).ceil() as usize;
            for _ in 0..steps {
                let before = controller.position.y;
                controller.update(dt, Vec3::X, true, false, &terrain, &world);
                max_drop = max_drop.max(before - controller.position.y);
            }
            assert!(max_drop < 8.0, "snapped down by {max_drop} at dt {dt}");
        }
    }

    #[test]
    fn controller_runs_up_a_walkable_static_ramp_at_low_frame_rates() {
        let terrain = flat_terrain();
        for dt in [1.0_f32 / 20.0, 0.1] {
            let mut controller = CharacterController::new(&terrain, ControllerConfig::default());
            let start = controller.position;
            let world = CollisionWorld {
                triangles: vec![Triangle::new(
                    Vec3::new(start.x, start.y, start.z - 200.0),
                    Vec3::new(start.x, start.y, start.z + 200.0),
                    Vec3::new(start.x + 400.0, start.y + 400.0, start.z),
                )
                .unwrap()],
                objects: 1,
            };
            let steps = (0.3 / dt).ceil() as usize;
            for _ in 0..steps {
                controller.update(dt, Vec3::X, true, false, &terrain, &world);
            }
            assert!(
                controller.position.x > start.x + 50.0,
                "did not climb at dt {dt}: x={}",
                controller.position.x
            );
            assert!(
                controller.position.y > 50.0,
                "y={} at dt {dt}",
                controller.position.y
            );
            assert!(controller.on_ground, "not grounded at dt {dt}");
        }
    }

    #[test]
    fn controller_runs_down_walkable_static_ramps() {
        let terrain = flat_terrain();
        for slope in [1.0, 1.732] {
            for (dt, steps) in [(1.0 / 30.0, 2), (1.0 / 60.0, 4)] {
                let mut controller =
                    CharacterController::new(&terrain, ControllerConfig::default());
                let start = controller.position;
                controller.position.y = 100.0;
                let world = CollisionWorld {
                    triangles: vec![Triangle::new(
                        Vec3::new(start.x, 100.0, start.z - 200.0),
                        Vec3::new(start.x, 100.0, start.z + 200.0),
                        Vec3::new(start.x + 100.0, 100.0 - 100.0 * slope, start.z),
                    )
                    .unwrap()],
                    objects: 1,
                };
                for _ in 0..steps {
                    controller.update(dt, Vec3::X, true, false, &terrain, &world);
                }
                assert!(
                    controller.position.y < 99.0 && controller.position.y > 50.0,
                    "y={} slope={slope} dt={dt}",
                    controller.position.y
                );
                assert!(controller.on_ground, "not grounded slope={slope} dt={dt}");
            }
        }
    }

    #[test]
    fn controller_stops_at_wall_at_low_frame_rate() {
        let terrain = flat_terrain();
        let mut controller = CharacterController::new(&terrain, ControllerConfig::default());
        let start = controller.position;
        let wall_x = start.x + 100.0;
        let wall = Triangle::new(
            Vec3::new(wall_x, start.y - 100.0, start.z - 200.0),
            Vec3::new(wall_x, start.y - 100.0, start.z + 200.0),
            Vec3::new(wall_x, start.y + 400.0, start.z),
        )
        .unwrap();
        let world = CollisionWorld {
            triangles: vec![wall],
            objects: 1,
        };
        for _ in 0..10 {
            controller.update(0.1, Vec3::X, true, false, &terrain, &world);
        }
        assert!(
            controller.position.x > start.x + 1.0,
            "did not move: x={}",
            controller.position.x
        );
        assert!(
            controller.position.x < wall_x - 30.0,
            "crossed wall: x={} wall_x={}",
            controller.position.x,
            wall_x
        );
    }

    #[test]
    fn controller_slides_along_a_wall_instead_of_sticking() {
        let terrain = flat_terrain();
        let mut controller = CharacterController::new(&terrain, ControllerConfig::default());
        let start = controller.position;
        let wall_x = start.x + 40.0;
        let wall = Triangle::new(
            Vec3::new(wall_x, start.y - 100.0, start.z - 200.0),
            Vec3::new(wall_x, start.y - 100.0, start.z + 200.0),
            Vec3::new(wall_x, start.y + 400.0, start.z),
        )
        .unwrap();
        let world = CollisionWorld {
            triangles: vec![wall],
            objects: 1,
        };
        for _ in 0..120 {
            controller.update(
                1.0 / 60.0,
                Vec3::new(1.0, 0.0, 1.0).normalize(),
                false,
                false,
                &terrain,
                &world,
            );
        }
        assert!(
            controller.position.z > start.z + 10.0,
            "did not slide along wall: z={} start_z={}",
            controller.position.z,
            start.z
        );
        assert!(
            controller.position.x < wall_x - 30.0,
            "crossed wall: x={} wall_x={}",
            controller.position.x,
            wall_x
        );
    }

    #[test]
    fn controller_slides_around_a_low_rock_without_sticking() {
        let terrain = flat_terrain();
        let mut controller = CharacterController::new(&terrain, ControllerConfig::default());
        let start = controller.position;
        let rock_x = start.x + 40.0;
        let top = start.y + 20.0;
        let z0 = start.z - 300.0;
        let z1 = start.z + 300.0;
        let face = Triangle::new(
            Vec3::new(rock_x, start.y - 100.0, z0),
            Vec3::new(rock_x, start.y - 100.0, z1),
            Vec3::new(rock_x, top, z0),
        )
        .unwrap();
        let top_a = Triangle::new(
            Vec3::new(rock_x, top, z0),
            Vec3::new(rock_x + 300.0, top, z0),
            Vec3::new(rock_x + 300.0, top, z1),
        )
        .unwrap();
        let top_b = Triangle::new(
            Vec3::new(rock_x, top, z0),
            Vec3::new(rock_x + 300.0, top, z1),
            Vec3::new(rock_x, top, z1),
        )
        .unwrap();
        let world = CollisionWorld {
            triangles: vec![face, top_a, top_b],
            objects: 1,
        };
        for _ in 0..120 {
            controller.update(
                1.0 / 60.0,
                Vec3::new(1.0, 0.0, 4.0).normalize(),
                false,
                false,
                &terrain,
                &world,
            );
        }
        assert!(
            controller.position.z > start.z + 50.0,
            "stuck on low rock: z={} start_z={}",
            controller.position.z,
            start.z
        );
        assert!(
            controller.position.x < rock_x - 28.0,
            "crossed low rock: x={} rock_x={}",
            controller.position.x,
            rock_x
        );
        assert!(controller.on_ground);
    }

    #[test]
    fn controller_can_back_away_from_a_wall() {
        let terrain = flat_terrain();
        let mut controller = CharacterController::new(&terrain, ControllerConfig::default());
        let start = controller.position;
        let wall_x = start.x + 40.0;
        let wall = Triangle::new(
            Vec3::new(wall_x, start.y - 100.0, start.z - 200.0),
            Vec3::new(wall_x, start.y - 100.0, start.z + 200.0),
            Vec3::new(wall_x, start.y + 400.0, start.z),
        )
        .unwrap();
        let world = CollisionWorld {
            triangles: vec![wall],
            objects: 1,
        };
        for _ in 0..120 {
            controller.update(1.0 / 60.0, Vec3::X, false, false, &terrain, &world);
        }
        for _ in 0..120 {
            controller.update(1.0 / 60.0, -Vec3::X, false, false, &terrain, &world);
        }
        assert!(
            controller.position.x < start.x - 10.0,
            "stuck after backing away: x={} start_x={}",
            controller.position.x,
            start.x
        );
    }

    #[test]
    fn controller_does_not_ground_on_an_angled_wall() {
        let terrain = flat_terrain();
        let mut controller = CharacterController::new(&terrain, ControllerConfig::default());
        controller.position = Vec3::new(2048.0, 100.0, 2048.0);
        let world = CollisionWorld {
            triangles: vec![Triangle::new(
                Vec3::new(2048.0, 100.0, 1948.0),
                Vec3::new(2048.0, 100.0, 2148.0),
                Vec3::new(2148.0, 375.0, 2048.0),
            )
            .unwrap()],
            objects: 1,
        };
        controller.update(1.0 / 60.0, Vec3::ZERO, false, false, &terrain, &world);
        assert!(!controller.on_ground);
    }

    #[test]
    fn controller_stays_grounded_on_static_mesh_at_varied_frame_rates() {
        let terrain = flat_terrain();
        for dt in [1.0 / 30.0, 1.0 / 60.0, 1.0 / 120.0, 1.0 / 240.0] {
            let mut controller = CharacterController::new(&terrain, ControllerConfig::default());
            let start = controller.position;
            let top = start.y + 50.0;
            let back = Vec3::new(start.x - 200.0, top, start.z - 200.0);
            let right = Vec3::new(start.x + 200.0, top, start.z - 200.0);
            let right_front = Vec3::new(start.x + 200.0, top, start.z + 200.0);
            let front = Vec3::new(start.x - 200.0, top, start.z + 200.0);
            let world = CollisionWorld {
                triangles: vec![
                    Triangle::new(back, right, right_front).unwrap(),
                    Triangle::new(back, right_front, front).unwrap(),
                ],
                objects: 1,
            };
            controller.position.y = top;
            for _ in 0..120 {
                controller.update(dt, Vec3::ZERO, false, false, &terrain, &world);
            }
            assert!(controller.on_ground, "not grounded at dt {dt}");
            assert!(
                (controller.position.y - top).abs() < 1.0,
                "y drift at dt {dt}: {}",
                controller.position.y
            );
            controller.update(dt, Vec3::ZERO, false, true, &terrain, &world);
            assert!(controller.velocity_y > 0.0, "jump failed at dt {dt}");
        }
    }
    #[test]
    fn controller_slides_off_a_steep_static_ramp() {
        let terrain = flat_terrain();
        for jump in [false, true] {
            let mut controller = CharacterController::new(&terrain, ControllerConfig::default());
            let start = controller.position;
            let world = CollisionWorld {
                triangles: vec![Triangle::new(
                    Vec3::new(start.x, start.y, start.z - 200.0),
                    Vec3::new(start.x, start.y, start.z + 200.0),
                    Vec3::new(start.x + 400.0, start.y + 800.0, start.z),
                )
                .unwrap()],
                objects: 1,
            };
            for frame in 0..240 {
                controller.update(
                    1.0 / 60.0,
                    Vec3::X,
                    true,
                    jump && frame == 0,
                    &terrain,
                    &world,
                );
            }
            assert!(
                controller.on_ground,
                "stuck off ground jump={jump}: pos={:?} vel={}",
                controller.position, controller.velocity_y
            );
            assert!(
                controller.position.y.abs() < 1.0,
                "stuck above ground jump={jump}: y={}",
                controller.position.y
            );
        }
    }
    #[test]
    fn controller_escapes_when_started_under_a_walkable_overhead() {
        let terrain = flat_terrain();
        let x = 2048.0;
        let z = 2048.0;
        let a = Vec3::new(x - 200.0, 160.0, z - 200.0);
        let b = Vec3::new(x + 200.0, 160.0, z - 200.0);
        let c = Vec3::new(x, 160.0, z + 200.0);
        for reverse in [false, true] {
            let (a, b) = if reverse { (b, a) } else { (a, b) };
            let world = CollisionWorld {
                triangles: vec![Triangle::new(a, b, c).unwrap()],
                objects: 1,
            };
            let mut controller = CharacterController::new(&terrain, ControllerConfig::default());
            controller.position = Vec3::new(x, 75.0, z);
            for _ in 0..240 {
                controller.update(1.0 / 60.0, Vec3::ZERO, false, false, &terrain, &world);
            }
            assert!(
                controller.on_ground,
                "stuck under overhead reverse={reverse}: pos={:?} vel={}",
                controller.position, controller.velocity_y
            );
            assert!(
                controller.position.y.abs() < 1.0,
                "did not reach the floor reverse={reverse}: pos={:?}",
                controller.position
            );
            let settled = controller.position;
            for _ in 0..60 {
                controller.update(1.0 / 60.0, Vec3::X, false, false, &terrain, &world);
            }
            assert!(
                controller.position.x > settled.x + 10.0,
                "cannot move after escape reverse={reverse}: settled={settled:?} moved={:?}",
                controller.position
            );
        }
    }
}
