struct Camera {
    view_proj: mat4x4<f32>,
    eye: vec4<f32>,
    light: vec4<f32>,
};

@group(0) @binding(0) var<uniform> camera: Camera;
@group(1) @binding(0) var diffuse: texture_2d<f32>;
@group(1) @binding(1) var diffuse_sampler: sampler;

struct VertexIn {
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
    @location(3) color: vec3<f32>,
};

struct VertexOut {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) world_position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
    @location(3) color: vec3<f32>,
};

@vertex
fn vs_main(in: VertexIn) -> VertexOut {
    var out: VertexOut;
    out.clip_position = camera.view_proj * vec4<f32>(in.position, 1.0);
    out.world_position = in.position;
    out.normal = in.normal;
    out.uv = in.uv;
    out.color = in.color;
    return out;
}

@fragment
fn fs_main(in: VertexOut) -> @location(0) vec4<f32> {
    let albedo = textureSample(diffuse, diffuse_sampler, in.uv).rgb * in.color;
    let normal = normalize(in.normal);
    let light = normalize(camera.light.xyz);
    let diffuse_light = max(dot(normal, light), 0.0);
    let lit = albedo * (0.30 + 0.70 * diffuse_light);
    let distance = length(in.world_position - camera.eye.xyz);
    let fog = smoothstep(3500.0, 16000.0, distance);
    return vec4<f32>(mix(lit, vec3<f32>(0.52, 0.66, 0.82), fog * 0.78), 1.0);
}
