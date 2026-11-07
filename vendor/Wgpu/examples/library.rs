//! The Rust library without PowerShell: a device, a compute program run
//! over a buffer, a render program drawing into a texture, and an error
//! the library returns. Run it with
//!
//!     cargo run --example library --no-default-features

use wgpu_pwsh::{Binding, Draw, Gpu, GpuOptions, PsError, RenderOptions, wgpu};

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
    let info = gpu.adapter_info();
    println!("{} through {:?}", info.name, info.backend);

    let usage = wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC;
    let values = gpu.create_buffer(bytemuck::cast_slice(&[1.0f32, 2.0, 3.0, 4.0]), usage, "values")?;
    let double = gpu.compute_program(DOUBLE, None, &[])?;
    println!("{} runs {:?} invocations a workgroup and binds {:?}", double.entry_point(), double.workgroup_size(), double.bindings());
    double.run(&[("values", Binding::Buffer(&values))], [1, 1, 1])?;
    let doubled: Vec<f32> = bytemuck::pod_collect_to_vec(&gpu.read_buffer(&values, 0, 16)?);
    println!("doubled: {doubled:?}");

    let target = gpu.create_texture(4, 4, wgpu::TextureFormat::Rgba8Unorm, None, "target")?;
    let cover = gpu.render_program(COVER, &RenderOptions::default())?;
    cover.draw(&target, &Draw { vertex_count: 3, ..Default::default() })?;
    let texels = gpu.read_texture(&target)?;
    println!("first pixel: {:?}", &texels[..4]);

    if let Err(e) = double.run(&[], [1, 1, 1]) {
        println!("refused as {} ({:?}): {}", e.error_id, e.category, e.message);
    }
    Ok(())
}
