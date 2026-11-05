struct Camera {
    view_proj: mat4x4<f32>,
    eye: vec4<f32>,
    light: vec4<f32>,
};

@group(0) @binding(0) var<uniform> camera: Camera;

struct VertexIn {
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) color: vec3<f32>,
};

struct VertexOut {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) world_position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) color: vec3<f32>,
};

@vertex
fn vs_main(in: VertexIn) -> VertexOut {
    var out: VertexOut;
    out.clip_position = camera.view_proj * vec4<f32>(in.position, 1.0);
    out.world_position = in.position;
    out.normal = in.normal;
    out.color = in.color;
    return out;
}

@fragment
fn fs_main(in: VertexOut) -> @location(0) vec4<f32> {
    let normal = normalize(in.normal);
    let light = normalize(camera.light.xyz);
    let diffuse = max(dot(normal, light), 0.0);
    let lit = in.color * (0.34 + 0.66 * diffuse);
    let distance = length(in.world_position - camera.eye.xyz);
    let grid_fade = 1.0 - smoothstep(1800.0, 7000.0, distance);
    let gx = abs(fract(in.world_position.x / 256.0) - 0.5);
    let gz = abs(fract(in.world_position.z / 256.0) - 0.5);
    let line = smoothstep(0.47, 0.5, max(gx, gz));
    let grid = mix(lit, vec3<f32>(0.07, 0.09, 0.07), line * 0.22 * grid_fade);
    let fog = smoothstep(3500.0, 16000.0, distance);
    let sky = vec3<f32>(0.52, 0.66, 0.82);
    return vec4<f32>(mix(grid, sky, fog * 0.78), 1.0);
}
