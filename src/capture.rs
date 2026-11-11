use std::collections::HashMap;
use std::error::Error;
use std::fs::File;
use std::path::Path;
use std::sync::mpsc;

use crate::{
    editor::build_sample_ui,
    fbx, fbx_animation,
    mesh::MeshPipeline,
    scene::{Component, Scene},
};
use imgui::Context;
use imgui_wgpu::{Renderer as ImGuiRenderer, RendererConfig};

const WIDTH: u32 = 800;
const HEIGHT: u32 = 600;
const BYTES_PER_PIXEL: u32 = 4;
const COPY_BYTES_PER_ROW_ALIGNMENT: u32 = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;

pub async fn render_to_png(path: &Path, model_path: &Path) -> Result<(), Box<dyn Error>> {
    let instance = wgpu::Instance::default();
    let adapter = instance
        .request_adapter(&wgpu::RequestAdapterOptions {
            force_fallback_adapter: true,
            ..Default::default()
        })
        .await?;
    let info = adapter.get_info();
    eprintln!("Offscreen adapter: {} ({:?})", info.name, info.backend);
    let (device, queue) = adapter
        .request_device(&wgpu::DeviceDescriptor {
            label: Some("YuriEngine offscreen capture device"),
            ..Default::default()
        })
        .await?;

    let target = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("YuriEngine offscreen frame"),
        size: wgpu::Extent3d {
            width: WIDTH,
            height: HEIGHT,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8UnormSrgb,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let view = target.create_view(&wgpu::TextureViewDescriptor::default());
    let (_depth_texture, depth_view) =
        crate::mesh::create_depth_texture(&device, WIDTH, HEIGHT, "YuriEngine capture depth");
    let model = fbx::load(model_path)?;
    eprintln!(
        "Capture loaded {} meshes / {} triangles / {} material bindings / {} nodes / {} bones / {} skinned meshes / animation clips [{}]",
        model.source_meshes,
        model.triangles,
        model.materials.len(),
        model.hierarchy.nodes.len(),
        model.animation.bones.len(),
        model.animation.skinned_meshes.len(),
        model.animation.clips.iter().map(|clip| format!("{}: {:.3}s, {} samples", clip.name, clip.end_seconds - clip.start_seconds, clip.frames.len())).collect::<Vec<_>>().join(", ")
    );
    let mut scene = Scene::new();
    let model_id = scene.add_model(model_path.to_path_buf(), model);
    let root_id = scene.create_game_object("Scene Root");
    let object_id = scene
        .spawn_model("Main Model", model_id)
        .ok_or("failed to create game object for model")?;
    if !scene.set_parent(object_id, Some(root_id)) {
        return Err("failed to parent model game object to scene root".into());
    }
    let model_asset = scene
        .model(model_id)
        .ok_or("loaded model asset disappeared")?;
    let mesh_pipeline = MeshPipeline::new(
        &device,
        &queue,
        wgpu::TextureFormat::Rgba8UnormSrgb,
        &model_asset.mesh,
    );
    let mesh_pipelines = HashMap::from([(object_id, mesh_pipeline)]);

    let mut context = Context::create();
    context.set_ini_filename(None);
    context.io_mut().display_size = [WIDTH as f32, HEIGHT as f32];
    context.io_mut().delta_time = 1.0 / 60.0;
    let mut imgui = ImGuiRenderer::new(
        &mut context,
        &device,
        &queue,
        RendererConfig {
            texture_format: wgpu::TextureFormat::Rgba8UnormSrgb,
            ..Default::default()
        },
    );
    let ui = context.frame();
    build_sample_ui(ui);
    let draw_data = context.render();

    let unpadded_bytes_per_row = WIDTH * BYTES_PER_PIXEL;
    let padded_bytes_per_row = unpadded_bytes_per_row.div_ceil(COPY_BYTES_PER_ROW_ALIGNMENT)
        * COPY_BYTES_PER_ROW_ALIGNMENT;
    let readback = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("YuriEngine frame readback"),
        size: u64::from(padded_bytes_per_row) * u64::from(HEIGHT),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });

    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("YuriEngine offscreen frame encoder"),
    });
    {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("YuriEngine offscreen FBX pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color {
                        r: 1.0,
                        g: 0.0,
                        b: 0.55,
                        a: 1.0,
                    }),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: &depth_view,
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Clear(1.0),
                    store: wgpu::StoreOp::Store,
                }),
                stencil_ops: None,
            }),
            ..Default::default()
        });
        for object in scene.objects() {
            for component in object.components() {
                let Component::MeshRenderer(renderer) = component else {
                    continue;
                };
                let Some(model) = scene.model(renderer.model) else {
                    continue;
                };
                let Some(mesh) = mesh_pipelines.get(&object.id) else {
                    continue;
                };
                let Some(world_transform) = scene.world_matrix(object.id) else {
                    continue;
                };
                let frame = object
                    .animator()
                    .and_then(|animator| animator.sample(&model.mesh.animation));
                let palette = fbx_animation::skin_palette(&model.mesh.animation, frame);
                let bounds = (model.mesh.bounds_min, model.mesh.bounds_max);
                mesh.draw(
                    &queue,
                    &mut pass,
                    bounds,
                    WIDTH,
                    HEIGHT,
                    Some(&palette),
                    world_transform,
                );
            }
        }
    }
    {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("YuriEngine offscreen ImGui pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                },
            })],
            ..Default::default()
        });
        imgui.render(draw_data, &queue, &device, &mut pass)?;
    }
    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture: &target,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &readback,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(padded_bytes_per_row),
                rows_per_image: Some(HEIGHT),
            },
        },
        wgpu::Extent3d {
            width: WIDTH,
            height: HEIGHT,
            depth_or_array_layers: 1,
        },
    );
    queue.submit([encoder.finish()]);

    let read_slice = readback.slice(..);
    let (sender, receiver) = mpsc::channel();
    read_slice.map_async(wgpu::MapMode::Read, move |result| {
        let _ = sender.send(result);
    });
    device.poll(wgpu::PollType::wait_indefinitely())?;
    receiver.recv()??;

    let mapped = read_slice.get_mapped_range();
    let mut pixels = Vec::with_capacity((WIDTH * HEIGHT * BYTES_PER_PIXEL) as usize);
    for row in mapped
        .chunks_exact(padded_bytes_per_row as usize)
        .take(HEIGHT as usize)
    {
        pixels.extend_from_slice(&row[..unpadded_bytes_per_row as usize]);
    }
    drop(mapped);
    readback.unmap();

    let file = File::create(path)?;
    let mut encoder = png::Encoder::new(file, WIDTH, HEIGHT);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.write_header()?.write_image_data(&pixels)?;
    eprintln!("Saved rendered frame to {}", path.display());
    Ok(())
}
