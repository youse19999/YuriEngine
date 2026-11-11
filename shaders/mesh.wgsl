struct Camera {
    view_projection: mat4x4<f32>,
};

@group(0) @binding(0)
var<uniform> camera: Camera;

@group(1) @binding(0)
var base_color_texture: texture_2d<f32>;
@group(1) @binding(1)
var base_color_sampler: sampler;

@group(2) @binding(0)
var<storage, read> skin_matrices: array<mat4x4<f32>>;

struct VertexInput {
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) base_color: vec4<f32>,
    @location(3) joints: vec4<u32>,
    @location(4) weights: vec4<f32>,
    @location(5) uv: vec2<f32>,
};

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) normal: vec3<f32>,
    @location(1) base_color: vec4<f32>,
    @location(2) uv: vec2<f32>,
};

@vertex
fn vs_main(input: VertexInput) -> VertexOutput {
    var output: VertexOutput;
    var position = vec4<f32>(input.position, 1.0);
    var normal = input.normal;
    let total_weight = dot(input.weights, vec4<f32>(1.0));
    if total_weight > 0.0001 {
        let skinned_position = skin_matrices[input.joints.x] * vec4<f32>(input.position, 1.0) * input.weights.x
            + skin_matrices[input.joints.y] * vec4<f32>(input.position, 1.0) * input.weights.y
            + skin_matrices[input.joints.z] * vec4<f32>(input.position, 1.0) * input.weights.z
            + skin_matrices[input.joints.w] * vec4<f32>(input.position, 1.0) * input.weights.w;
        let skinned_normal = (skin_matrices[input.joints.x] * vec4<f32>(input.normal, 0.0)).xyz * input.weights.x
            + (skin_matrices[input.joints.y] * vec4<f32>(input.normal, 0.0)).xyz * input.weights.y
            + (skin_matrices[input.joints.z] * vec4<f32>(input.normal, 0.0)).xyz * input.weights.z
            + (skin_matrices[input.joints.w] * vec4<f32>(input.normal, 0.0)).xyz * input.weights.w;
        position = skinned_position / total_weight;
        normal = normalize(skinned_normal / total_weight);
    }
    output.position = camera.view_projection * position;
    output.normal = normal;
    output.base_color = input.base_color;
    output.uv = input.uv;
    return output;
}

@fragment
fn fs_main(input: VertexOutput) -> @location(0) vec4<f32> {
    let texel = textureSample(base_color_texture, base_color_sampler, input.uv);
    let color = texel.rgb * input.base_color.rgb;
    let alpha = input.base_color.a;
    if alpha < 0.01 {
        discard;
    }
    let light = normalize(vec3<f32>(0.4, -0.6, 1.0));
    let diffuse = max(dot(normalize(input.normal), light), 0.0);
    let shade = 0.22 + diffuse * 0.78;
    return vec4<f32>(color * shade, alpha);
}
