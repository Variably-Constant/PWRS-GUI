//! The Rust library recording into passes the caller begins, as a
//! renderer does inside its own frame: a dispatch into a compute pass and
//! a draw into a multisampled render pass with depth, whose attachments
//! are made from the program's accessors, both submitted by the caller.
//! Run it with
//!
//!     cargo run --example record --no-default-features

use wgpu_pwsh::{Binding, Draw, Gpu, GpuOptions, PsError, RenderOptions, RunSpec, wgpu};

const DOUBLE: &str = "
@group(0) @binding(0) var<storage, read_write> values: array<f32>;
@compute @workgroup_size(64)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    if (id.x < arrayLength(&values)) { values[id.x] = values[id.x] * 2.0; }
}";

const COVER: &str = "
@vertex
fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    var corners = array<vec2<f32>, 3>(vec2<f32>(-1.0, -1.0), vec2<f32>(3.0, -1.0), vec2<f32>(-1.0, 3.0));
    return vec4<f32>(corners[i], 0.0, 1.0);
}
@fragment
fn fs() -> @location(0) vec4<f32> { return vec4<f32>(0.2, 0.4, 0.8, 1.0); }";

fn main() -> Result<(), PsError> {
    let gpu = Gpu::open(&GpuOptions::default())?;
    let device = gpu.device();

    let usage = wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC;
    let values = gpu.create_buffer(bytemuck::cast_slice(&[1.0f32, 2.0, 3.0, 4.0]), usage, "values")?;
    let double = gpu.compute_program(DOUBLE, None, &[])?;
    let bindings = [("values", Binding::Buffer(&values))];

    let cover = gpu.render_program(COVER, &RenderOptions { sample_count: 4, depth: true, ..Default::default() })?;
    println!("draws into {:?} at {} samples, depth {:?}", cover.format(), cover.sample_count(), cover.depth_format());
    let attachment = |format: wgpu::TextureFormat| {
        device.create_texture(&wgpu::TextureDescriptor {
            label: None,
            size: wgpu::Extent3d { width: 4, height: 4, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: cover.sample_count(),
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        })
    };
    let target = gpu.create_texture(4, 4, cover.format(), None, "target")?;
    let resolved = target.create_view(&wgpu::TextureViewDescriptor::default());
    let color = attachment(cover.format()).create_view(&wgpu::TextureViewDescriptor::default());
    let depth = cover.depth_format().map(|format| attachment(format).create_view(&wgpu::TextureViewDescriptor::default()));

    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("frame") });
    {
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor::default());
        let workgroups = double.record(&mut pass, &RunSpec { bindings: &bindings, ..Default::default() })?;
        println!("recorded {workgroups:?} workgroups");
    }
    {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &color,
                depth_slice: None,
                resolve_target: Some(&resolved),
                ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::BLACK), store: wgpu::StoreOp::Store },
            })],
            depth_stencil_attachment: depth.as_ref().map(|view| wgpu::RenderPassDepthStencilAttachment {
                view,
                depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Clear(1.0), store: wgpu::StoreOp::Store }),
                stencil_ops: None,
            }),
            ..Default::default()
        });
        if let Err(e) = cover.record(&mut pass, &Draw { vertex_count: 3, ..Default::default() }) {
            println!("refused as {}: {}", e.error_id, e.message);
        }
        cover.record(&mut pass, &Draw { vertex_count: 3, clear: None, ..Default::default() })?;
    }
    gpu.queue().submit([encoder.finish()]);
    for error in gpu.take_errors() {
        println!("the device reported: {error}");
    }

    let doubled: Vec<f32> = bytemuck::pod_collect_to_vec(&gpu.read_buffer(&values, 0, 16)?);
    println!("doubled: {doubled:?}");
    println!("first pixel: {:?}", &gpu.read_texture(&target)?[..4]);
    Ok(())
}
