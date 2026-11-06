use std::error::Error;
use std::sync::Arc;
use std::time::Instant;

use bytemuck::{Pod, Zeroable};
use glam::{Mat4, Vec3};
use wgpu::util::DeviceExt;
use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::{DeviceEvent, ElementState, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::window::{CursorGrabMode, Window, WindowId};

use crate::collision::{CharacterController, CollisionWorld, ControllerConfig};
use crate::debug::MovementSample;
use crate::scene::{Scene, TextureData};
use crate::terrain::{Terrain, Vertex};

#[repr(C)]
#[derive(Copy, Clone, Pod, Zeroable)]
struct CameraUniform {
    view_proj: [[f32; 4]; 4],
    eye: [f32; 4],
    light: [f32; 4],
}

struct Camera {
    yaw: f32,
    pitch: f32,
}

impl Camera {
    fn new(scene: &Scene, player: &CharacterController) -> Self {
        let target = scene
            .bounds()
            .map(|(min, max)| (min + max) * 0.5)
            .unwrap_or(player.position + Vec3::Z * 1024.0);
        let yaw = (target.x - player.position.x).atan2(target.z - player.position.z);
        Self { yaw, pitch: 0.0 }
    }

    fn forward(&self) -> Vec3 {
        let cos_pitch = self.pitch.cos();
        Vec3::new(
            self.yaw.sin() * cos_pitch,
            self.pitch.sin(),
            self.yaw.cos() * cos_pitch,
        )
    }

    fn view_projection(&self, eye: Vec3, aspect: f32) -> Mat4 {
        let view = Mat4::look_to_rh(eye, self.forward(), Vec3::Y);
        let projection = Mat4::perspective_rh(70f32.to_radians(), aspect.max(0.01), 1.0, 24000.0);
        projection * view
    }
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct ObjectVertex {
    position: [f32; 3],
    normal: [f32; 3],
    uv: [f32; 2],
    color: [f32; 3],
}

struct ObjectGpu {
    vertex_buffer: wgpu::Buffer,
    index_buffer: wgpu::Buffer,
    index_count: u32,
    texture_index: usize,
}

struct State {
    window: Arc<Window>,
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    pipeline: wgpu::RenderPipeline,
    vertex_buffer: wgpu::Buffer,
    index_buffer: wgpu::Buffer,
    index_count: u32,
    camera_buffer: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    depth_view: wgpu::TextureView,
    camera: Camera,
    player: CharacterController,
    terrain: Terrain,
    collision: CollisionWorld,
    object_pipeline: wgpu::RenderPipeline,
    objects: Vec<ObjectGpu>,
    texture_bind_groups: Vec<wgpu::BindGroup>,
    keys: [bool; 256],
    mouse_delta: (f32, f32),
    mouse_grabbed: bool,
    last_cursor: Option<winit::dpi::PhysicalPosition<f64>>,
    last_frame: Instant,
    debug: bool,
    frame: u64,
}

impl State {
    fn new(
        window: Arc<Window>,
        terrain: Terrain,
        scene: Scene,
        controller: ControllerConfig,
        collision: CollisionWorld,
        mouse_grabbed: bool,
        debug: bool,
    ) -> Result<Self, Box<dyn Error>> {
        let objects = scene.objects.len();
        let textures = scene.textures.len();
        let collision_triangles = collision.triangle_count();
        let size = window.inner_size();
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::VULKAN,
            ..Default::default()
        });
        let surface = instance.create_surface(window.clone())?;
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            compatible_surface: Some(&surface),
            force_fallback_adapter: false,
        }))
        .ok_or("no Vulkan adapter")?;
        let info = adapter.get_info();
        println!("gpu:       {:?} {}", info.backend, info.name);
        if debug {
            println!(
                "debug: runtime gpu=\"{}\" backend={:?} objects={} textures={} collision_triangles={}",
                info.name, info.backend, objects, textures, collision_triangles
            );
        }
        let supports_bc = adapter
            .features()
            .contains(wgpu::Features::TEXTURE_COMPRESSION_BC);
        if !supports_bc {
            eprintln!("warning: GPU does not support BC compressed textures");
        }
        let required_features = if supports_bc {
            wgpu::Features::TEXTURE_COMPRESSION_BC
        } else {
            wgpu::Features::empty()
        };
        let (device, queue) = pollster::block_on(adapter.request_device(
            &wgpu::DeviceDescriptor {
                label: Some("oce"),
                required_features,
                required_limits: wgpu::Limits::default(),
            },
            None,
        ))?;
        let capabilities = surface.get_capabilities(&adapter);
        let format = capabilities
            .formats
            .iter()
            .copied()
            .find(|format| format.is_srgb())
            .unwrap_or(capabilities.formats[0]);
        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            width: size.width.max(1),
            height: size.height.max(1),
            present_mode: wgpu::PresentMode::Fifo,
            alpha_mode: capabilities.alpha_modes[0],
            view_formats: vec![],
            desired_maximum_frame_latency: 2,
        };
        surface.configure(&device, &config);

        let mesh = terrain.mesh();
        let vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("terrain vertices"),
            contents: bytemuck::cast_slice(&mesh.vertices),
            usage: wgpu::BufferUsages::VERTEX,
        });
        let index_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("terrain indices"),
            contents: bytemuck::cast_slice(&mesh.indices),
            usage: wgpu::BufferUsages::INDEX,
        });

        let camera_uniform = CameraUniform {
            view_proj: Mat4::IDENTITY.to_cols_array_2d(),
            eye: [0.0; 4],
            light: [0.4, 0.8, 0.3, 0.0],
        };
        let camera_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("camera"),
            contents: bytemuck::bytes_of(&camera_uniform),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("camera layout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("camera bind group"),
            layout: &bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: camera_buffer.as_entire_binding(),
            }],
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("terrain shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("terrain.wgsl").into()),
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("terrain layout"),
            bind_group_layouts: &[&bind_group_layout],
            push_constant_ranges: &[],
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("terrain pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: "vs_main",
                buffers: &[vertex_layout()],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: "fs_main",
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState::REPLACE),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                strip_index_format: None,
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: Some(wgpu::Face::Back),
                polygon_mode: wgpu::PolygonMode::Fill,
                unclipped_depth: false,
                conservative: false,
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: true,
                depth_compare: wgpu::CompareFunction::Less,
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
        });
        let object_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("object texture layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let object_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("object shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("object.wgsl").into()),
        });
        let object_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("object layout"),
                bind_group_layouts: &[&bind_group_layout, &object_layout],
                push_constant_ranges: &[],
            });
        let object_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("object pipeline"),
            layout: Some(&object_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &object_shader,
                entry_point: "vs_main",
                buffers: &[object_vertex_layout()],
            },
            fragment: Some(wgpu::FragmentState {
                module: &object_shader,
                entry_point: "fs_main",
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState::REPLACE),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                strip_index_format: None,
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: None,
                polygon_mode: wgpu::PolygonMode::Fill,
                unclipped_depth: false,
                conservative: false,
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: true,
                depth_compare: wgpu::CompareFunction::Less,
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("object sampler"),
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let white = TextureData {
            width: 1,
            height: 1,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            data: vec![255, 255, 255, 255],
        };
        let mut texture_indices: std::collections::HashMap<String, usize> =
            std::collections::HashMap::new();
        let white_view = create_texture_view(&device, &queue, &white);
        let mut texture_bind_groups = vec![create_texture_bind_group(
            &device,
            &object_layout,
            &white_view,
            &sampler,
        )];
        for path in scene.textures.keys() {
            if let Some(texture) = scene.textures.get(path) {
                if texture.format.is_compressed() && !supports_bc {
                    continue;
                }
                let view = create_texture_view(&device, &queue, texture);
                let bind_group =
                    create_texture_bind_group(&device, &object_layout, &view, &sampler);
                texture_indices.insert(path.clone(), texture_bind_groups.len());
                texture_bind_groups.push(bind_group);
            }
        }
        let player = CharacterController::new(&terrain, controller);
        let camera = Camera::new(&scene, &player);
        let mut objects = Vec::new();
        for object in scene.objects {
            if object.vertices.is_empty() || object.indices.is_empty() {
                continue;
            }
            let vertices: Vec<ObjectVertex> = object
                .vertices
                .iter()
                .map(|vertex| ObjectVertex {
                    position: vertex.position,
                    normal: vertex.normal,
                    uv: vertex.uv,
                    color: vertex.color,
                })
                .collect();
            let vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("object vertices"),
                contents: bytemuck::cast_slice(&vertices),
                usage: wgpu::BufferUsages::VERTEX,
            });
            let index_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("object indices"),
                contents: bytemuck::cast_slice(&object.indices),
                usage: wgpu::BufferUsages::INDEX,
            });
            let texture_index = object
                .texture
                .as_ref()
                .and_then(|path| texture_indices.get(path).copied())
                .unwrap_or(0);
            objects.push(ObjectGpu {
                vertex_buffer,
                index_buffer,
                index_count: object.indices.len() as u32,
                texture_index,
            });
        }
        let depth_view = depth_view(&device, &config);

        Ok(Self {
            window,
            surface,
            device,
            queue,
            config,
            pipeline,
            vertex_buffer,
            index_buffer,
            index_count: mesh.indices.len() as u32,
            camera_buffer,
            bind_group,
            depth_view,
            camera,
            player,
            terrain,
            collision,
            object_pipeline,
            objects,
            texture_bind_groups,
            keys: [false; 256],
            mouse_delta: (0.0, 0.0),
            mouse_grabbed,
            last_cursor: None,
            last_frame: Instant::now(),
            debug,
            frame: 0,
        })
    }

    fn resize(&mut self, width: u32, height: u32) {
        if width == 0 || height == 0 {
            return;
        }
        self.config.width = width;
        self.config.height = height;
        self.surface.configure(&self.device, &self.config);
        self.depth_view = depth_view(&self.device, &self.config);
    }

    fn update(&mut self) {
        let now = Instant::now();
        let elapsed = now.duration_since(self.last_frame).as_secs_f32();
        let dt = elapsed.min(0.1);
        self.last_frame = now;

        let mouse = self.mouse_delta;
        self.mouse_delta = (0.0, 0.0);
        let sensitivity = 0.0022;
        self.camera.yaw -= mouse.0 * sensitivity;
        self.camera.pitch = (self.camera.pitch - mouse.1 * sensitivity).clamp(-1.45, 1.45);

        let forward = Vec3::new(self.camera.yaw.sin(), 0.0, self.camera.yaw.cos());
        let right = forward.cross(Vec3::Y);
        let mut direction = Vec3::ZERO;
        if self.key(KeyCode::KeyW) {
            direction += forward;
        }
        if self.key(KeyCode::KeyS) {
            direction -= forward;
        }
        if self.key(KeyCode::KeyD) {
            direction += right;
        }
        if self.key(KeyCode::KeyA) {
            direction -= right;
        }
        let running = self.key(KeyCode::ShiftLeft) || self.key(KeyCode::ShiftRight);
        let jump = self.key(KeyCode::Space);
        self.player
            .update(dt, direction, running, jump, &self.terrain, &self.collision);

        if self.debug {
            self.frame += 1;
            println!(
                "{}",
                MovementSample {
                    frame: self.frame,
                    dt: elapsed,
                    position: self.player.position,
                    eye: self.player.eye(),
                    velocity_y: self.player.velocity_y,
                    on_ground: self.player.on_ground,
                    yaw: self.camera.yaw,
                    pitch: self.camera.pitch,
                    direction,
                    running,
                    jump,
                }
                .line()
            );
        }
    }

    fn key(&self, code: KeyCode) -> bool {
        self.keys[code as usize]
    }

    fn set_key(&mut self, code: KeyCode, pressed: bool) {
        self.keys[code as usize] = pressed;
    }

    fn render(&mut self) -> Result<(), Box<dyn Error>> {
        let eye = self.player.eye();
        let camera = CameraUniform {
            view_proj: self
                .camera
                .view_projection(eye, self.config.width as f32 / self.config.height as f32)
                .to_cols_array_2d(),
            eye: [eye.x, eye.y, eye.z, 1.0],
            light: [0.4, 0.8, 0.3, 0.0],
        };
        self.queue
            .write_buffer(&self.camera_buffer, 0, bytemuck::bytes_of(&camera));

        let output = match self.surface.get_current_texture() {
            Ok(output) => output,
            Err(wgpu::SurfaceError::Lost | wgpu::SurfaceError::Outdated) => {
                self.surface.configure(&self.device, &self.config);
                return Ok(());
            }
            Err(error) => return Err(error.into()),
        };
        let view = output
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("oce") });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("terrain pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: 0.52,
                            g: 0.66,
                            b: 0.82,
                            a: 1.0,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &self.depth_view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &self.bind_group, &[]);
            pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
            pass.set_index_buffer(self.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
            pass.draw_indexed(0..self.index_count, 0, 0..1);

            pass.set_pipeline(&self.object_pipeline);
            for object in &self.objects {
                pass.set_bind_group(0, &self.bind_group, &[]);
                pass.set_bind_group(1, &self.texture_bind_groups[object.texture_index], &[]);
                pass.set_vertex_buffer(0, object.vertex_buffer.slice(..));
                pass.set_index_buffer(object.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
                pass.draw_indexed(0..object.index_count, 0, 0..1);
            }
        }
        self.queue.submit(Some(encoder.finish()));
        output.present();
        Ok(())
    }
}

fn vertex_layout() -> wgpu::VertexBufferLayout<'static> {
    wgpu::VertexBufferLayout {
        array_stride: std::mem::size_of::<Vertex>() as wgpu::BufferAddress,
        step_mode: wgpu::VertexStepMode::Vertex,
        attributes: &[
            wgpu::VertexAttribute {
                offset: 0,
                shader_location: 0,
                format: wgpu::VertexFormat::Float32x3,
            },
            wgpu::VertexAttribute {
                offset: 12,
                shader_location: 1,
                format: wgpu::VertexFormat::Float32x3,
            },
            wgpu::VertexAttribute {
                offset: 24,
                shader_location: 2,
                format: wgpu::VertexFormat::Float32x3,
            },
        ],
    }
}

fn object_vertex_layout() -> wgpu::VertexBufferLayout<'static> {
    wgpu::VertexBufferLayout {
        array_stride: std::mem::size_of::<ObjectVertex>() as wgpu::BufferAddress,
        step_mode: wgpu::VertexStepMode::Vertex,
        attributes: &[
            wgpu::VertexAttribute {
                offset: 0,
                shader_location: 0,
                format: wgpu::VertexFormat::Float32x3,
            },
            wgpu::VertexAttribute {
                offset: 12,
                shader_location: 1,
                format: wgpu::VertexFormat::Float32x3,
            },
            wgpu::VertexAttribute {
                offset: 24,
                shader_location: 2,
                format: wgpu::VertexFormat::Float32x2,
            },
            wgpu::VertexAttribute {
                offset: 32,
                shader_location: 3,
                format: wgpu::VertexFormat::Float32x3,
            },
        ],
    }
}

fn create_texture_view(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    data: &TextureData,
) -> wgpu::TextureView {
    let width = data.width.max(1);
    let height = data.height.max(1);
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("object texture"),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: data.format,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    let (block_width, block_height) = data.format.block_dimensions();
    let bytes_per_block = data.format.block_copy_size(None).unwrap();
    let bytes_per_row = width.div_ceil(block_width) * bytes_per_block;
    let rows_per_image = height.div_ceil(block_height);
    queue.write_texture(
        wgpu::ImageCopyTexture {
            texture: &texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        &data.data,
        wgpu::ImageDataLayout {
            offset: 0,
            bytes_per_row: Some(bytes_per_row),
            rows_per_image: Some(rows_per_image),
        },
        wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
    );
    texture.create_view(&wgpu::TextureViewDescriptor::default())
}

fn create_texture_bind_group(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    view: &wgpu::TextureView,
    sampler: &wgpu::Sampler,
) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("object texture bind group"),
        layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(view),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::Sampler(sampler),
            },
        ],
    })
}

fn depth_view(device: &wgpu::Device, config: &wgpu::SurfaceConfiguration) -> wgpu::TextureView {
    device
        .create_texture(&wgpu::TextureDescriptor {
            label: Some("depth"),
            size: wgpu::Extent3d {
                width: config.width.max(1),
                height: config.height.max(1),
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Depth32Float,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        })
        .create_view(&wgpu::TextureViewDescriptor::default())
}

struct App {
    terrain: Option<Terrain>,
    scene: Option<Scene>,
    controller: Option<ControllerConfig>,
    collision: Option<CollisionWorld>,
    state: Option<State>,
    frames: Option<usize>,
    rendered: usize,
    debug: bool,
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.state.is_some() {
            return;
        }
        let attributes = Window::default_attributes()
            .with_title("Open CE - Bleakwind Basin")
            .with_inner_size(LogicalSize::new(1280, 720));
        let window = match event_loop.create_window(attributes) {
            Ok(window) => Arc::new(window),
            Err(error) => {
                eprintln!("oce: cannot create window: {error}");
                event_loop.exit();
                return;
            }
        };
        let grabbed = window
            .set_cursor_grab(CursorGrabMode::Locked)
            .or_else(|_| window.set_cursor_grab(CursorGrabMode::Confined))
            .is_ok();
        if !grabbed {
            eprintln!("oce: cursor grab unavailable; mouse look may need a click");
        }
        window.set_cursor_visible(false);
        window.focus_window();
        let terrain = self.terrain.take().expect("terrain");
        let scene = self.scene.take().expect("scene");
        let controller = self.controller.take().expect("controller");
        let collision = self.collision.take().expect("collision");
        match State::new(
            window, terrain, scene, controller, collision, grabbed, self.debug,
        ) {
            Ok(state) => self.state = Some(state),
            Err(error) => {
                eprintln!("oce: cannot initialize Vulkan: {error}");
                event_loop.exit();
            }
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        let Some(state) = self.state.as_mut() else {
            return;
        };
        match event {
            WindowEvent::CloseRequested
            | WindowEvent::KeyboardInput {
                event:
                    winit::event::KeyEvent {
                        physical_key: PhysicalKey::Code(KeyCode::Escape),
                        state: ElementState::Pressed,
                        ..
                    },
                ..
            } => event_loop.exit(),
            WindowEvent::Resized(size) => state.resize(size.width, size.height),
            WindowEvent::CursorMoved { position, .. } if !state.mouse_grabbed => {
                if let Some(last) = state.last_cursor {
                    state.mouse_delta.0 += (position.x - last.x) as f32;
                    state.mouse_delta.1 += (position.y - last.y) as f32;
                }
                state.last_cursor = Some(position);
            }
            WindowEvent::KeyboardInput { event, .. } => {
                if let PhysicalKey::Code(code) = event.physical_key {
                    state.set_key(code, event.state == ElementState::Pressed);
                }
            }
            WindowEvent::RedrawRequested => {
                state.update();
                if let Err(error) = state.render() {
                    eprintln!("oce: render error: {error}");
                    event_loop.exit();
                    return;
                }
                self.rendered += 1;
                if self.frames.is_some_and(|limit| self.rendered >= limit) {
                    event_loop.exit();
                }
            }
            _ => {}
        }
    }

    fn device_event(
        &mut self,
        _event_loop: &ActiveEventLoop,
        _device_id: winit::event::DeviceId,
        event: DeviceEvent,
    ) {
        if let DeviceEvent::MouseMotion { delta } = event {
            if let Some(state) = self.state.as_mut() {
                state.mouse_delta.0 += delta.0 as f32;
                state.mouse_delta.1 += delta.1 as f32;
            }
        }
    }

    fn about_to_wait(&mut self, _event_loop: &ActiveEventLoop) {
        if let Some(state) = self.state.as_ref() {
            state.window.request_redraw();
        }
    }
}

pub fn run(
    terrain: Terrain,
    scene: Scene,
    controller: ControllerConfig,
    collision: CollisionWorld,
    debug: bool,
    frames: Option<usize>,
) -> Result<(), Box<dyn Error>> {
    let event_loop = EventLoop::new()?;
    event_loop.set_control_flow(ControlFlow::Poll);
    let mut app = App {
        terrain: Some(terrain),
        scene: Some(scene),
        controller: Some(controller),
        collision: Some(collision),
        state: None,
        frames,
        rendered: 0,
        debug,
    };
    event_loop.run_app(&mut app)?;
    Ok(())
}
