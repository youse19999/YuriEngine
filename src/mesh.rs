use bytemuck::{Pod, Zeroable};
use wgpu::util::DeviceExt;

use crate::fbx::{MeshData, Vertex};

pub const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;

pub fn create_depth_texture(
    device: &wgpu::Device,
    width: u32,
    height: u32,
    label: &'static str,
) -> (wgpu::Texture, wgpu::TextureView) {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d {
            width: width.max(1),
            height: height.max(1),
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: DEPTH_FORMAT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    (texture, view)
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Uniforms {
    view_projection: [[f32; 4]; 4],
}

pub struct MeshPipeline {
    pipeline: wgpu::RenderPipeline,
    vertex_buffer: wgpu::Buffer,
    vertex_count: u32,
    uniform_buffer: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    _textures: Vec<wgpu::Texture>,
    texture_bind_groups: Vec<wgpu::BindGroup>,
    draw_ranges: Vec<crate::fbx::DrawRange>,
    skin_buffer: wgpu::Buffer,
    skin_bind_group: wgpu::BindGroup,
}

impl MeshPipeline {
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        format: wgpu::TextureFormat,
        mesh: &MeshData,
    ) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("FBX mesh shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("../shaders/mesh.wgsl").into()),
        });
        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("FBX camera layout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let texture_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("FBX material texture layout"),
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
        let skin_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("FBX skin palette layout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Storage { read_only: true },
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("FBX mesh pipeline layout"),
            bind_group_layouts: &[
                Some(&bind_group_layout),
                Some(&texture_bind_group_layout),
                Some(&skin_layout),
            ],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("FBX mesh pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<Vertex>() as wgpu::BufferAddress,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &wgpu::vertex_attr_array![
                        0 => Float32x3,
                        1 => Float32x3,
                        2 => Float32x2,
                        3 => Float32x4,
                        4 => Uint32x4,
                        5 => Float32x4
                    ],
                }],
            },
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: DEPTH_FORMAT,
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::Less),
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });
        let vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Imported FBX mesh vertices"),
            contents: bytemuck::cast_slice(&mesh.vertices),
            usage: wgpu::BufferUsages::VERTEX,
        });
        let uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("FBX camera uniform"),
            size: std::mem::size_of::<Uniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("FBX camera bind group"),
            layout: &bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform_buffer.as_entire_binding(),
            }],
        });
        let identity = crate::fbx_animation::skin_palette(&mesh.animation, None);
        let initial_palette = if identity.is_empty() {
            vec![identity_matrix()]
        } else {
            identity
        };
        let skin_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("FBX skin matrices"),
            contents: bytemuck::cast_slice(&initial_palette),
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        });
        let skin_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("FBX skin palette bind group"),
            layout: &skin_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: skin_buffer.as_entire_binding(),
            }],
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("FBX material sampler"),
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            ..Default::default()
        });
        let (fallback_texture, fallback_bind_group) = create_material_texture(
            device,
            queue,
            &texture_bind_group_layout,
            &sampler,
            "FBX white fallback",
            1,
            1,
            &[255, 255, 255, 255],
        );
        let mut textures = vec![fallback_texture];
        let mut texture_bind_groups = vec![fallback_bind_group];
        for image in &mesh.textures {
            let (texture, texture_bind_group) = create_material_texture(
                device,
                queue,
                &texture_bind_group_layout,
                &sampler,
                &image.source,
                image.width,
                image.height,
                &image.rgba,
            );
            textures.push(texture);
            texture_bind_groups.push(texture_bind_group);
        }
        Self {
            pipeline,
            vertex_buffer,
            vertex_count: mesh.vertices.len() as u32,
            uniform_buffer,
            bind_group,
            _textures: textures,
            texture_bind_groups,
            draw_ranges: mesh.draw_ranges.clone(),
            skin_buffer,
            skin_bind_group,
        }
    }

    pub fn draw<'a>(
        &'a self,
        queue: &wgpu::Queue,
        pass: &mut wgpu::RenderPass<'a>,
        bounds: ([f32; 3], [f32; 3]),
        width: u32,
        height: u32,
        skin_palette: Option<&[[[f32; 4]; 4]]>,
        world_transform: [[f32; 4]; 4],
    ) {
        if let Some(palette) = skin_palette.filter(|palette| !palette.is_empty()) {
            queue.write_buffer(&self.skin_buffer, 0, bytemuck::cast_slice(palette));
        }
        let matrix = crate::scene::multiply_matrices(
            camera_matrix(bounds, width.max(1) as f32 / height.max(1) as f32),
            world_transform,
        );
        queue.write_buffer(
            &self.uniform_buffer,
            0,
            bytemuck::bytes_of(&Uniforms {
                view_projection: matrix,
            }),
        );
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.bind_group, &[]);
        pass.set_bind_group(2, &self.skin_bind_group, &[]);
        pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
        if self.draw_ranges.is_empty() {
            pass.set_bind_group(1, &self.texture_bind_groups[0], &[]);
            pass.draw(0..self.vertex_count, 0..1);
            return;
        }
        for range in &self.draw_ranges {
            let texture_group_index = range.texture_index.map_or(0, |index| index + 1);
            let texture_bind_group = self
                .texture_bind_groups
                .get(texture_group_index)
                .unwrap_or(&self.texture_bind_groups[0]);
            pass.set_bind_group(1, texture_bind_group, &[]);
            pass.draw(
                range.first_vertex..range.first_vertex + range.vertex_count,
                0..1,
            );
        }
    }
}

fn identity_matrix() -> [[f32; 4]; 4] {
    [
        [1.0, 0.0, 0.0, 0.0],
        [0.0, 1.0, 0.0, 0.0],
        [0.0, 0.0, 1.0, 0.0],
        [0.0, 0.0, 0.0, 1.0],
    ]
}

fn create_material_texture(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    layout: &wgpu::BindGroupLayout,
    sampler: &wgpu::Sampler,
    label: &str,
    width: u32,
    height: u32,
    rgba: &[u8],
) -> (wgpu::Texture, wgpu::BindGroup) {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8UnormSrgb,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    queue.write_texture(
        texture.as_image_copy(),
        rgba,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(width.saturating_mul(4)),
            rows_per_image: Some(height),
        },
        wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
    );
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some(label),
        layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&view),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::Sampler(sampler),
            },
        ],
    });
    (texture, bind_group)
}

fn camera_matrix(bounds: ([f32; 3], [f32; 3]), aspect: f32) -> [[f32; 4]; 4] {
    let (min, max) = bounds;
    let center = [
        (min[0] + max[0]) * 0.5,
        (min[1] + max[1]) * 0.5,
        (min[2] + max[2]) * 0.5,
    ];
    let extent = [
        (max[0] - min[0]).abs(),
        (max[1] - min[1]).abs(),
        (max[2] - min[2]).abs(),
    ];
    let radius = (extent[0] * extent[0] + extent[1] * extent[1] + extent[2] * extent[2])
        .sqrt()
        .max(1e-3)
        * 0.5;
    // Use a tighter fit so imported characters occupy more of the viewport.
    let scale = (0.9 * radius).max(1e-4);
    [
        [1.0 / (scale * aspect.max(0.01)), 0.0, 0.0, 0.0],
        [0.0, 1.0 / scale, 0.0, 0.0],
        [0.0, 0.0, -0.1 / scale, 0.0],
        [
            -center[0] / (scale * aspect.max(0.01)),
            -center[1] / scale,
            0.5 + center[2] * 0.1 / scale,
            1.0,
        ],
    ]
}
