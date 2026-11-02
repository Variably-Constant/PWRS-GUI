---
title: How To Use The Rust Library
weight: 21
---

The cmdlets run on a Rust library, and a Rust program can use it without PowerShell: a device, buffers and textures, and WGSL compute and render programs bound by name, from any thread. Source: `src/gpu.rs`, `src/lib.rs`, `examples/library.rs` and `examples/record.rs`.

## Depend on it

A Rust project takes the crate from crates.io without its default `module` feature, which is the PowerShell module, and without its defaults `avif`, `jxl`, `svg` and `pdf`, which read .avif, JPEG XL, SVG and PDF files for the module's `Import-WgpuImage`, and with its sixth default feature, `static-dxc`:

```toml
[dependencies]
wgpu-pwsh = { version = "0.1.0", default-features = false, features = ["static-dxc"] }
bytemuck = "1"

[patch.crates-io]
wgpu-core = { git = "https://github.com/Variably-Constant/Wgpu" }
```

The patch takes the wgpu-core the module builds with: 30.0.1 with wgpu's fixes for an assertion a wait can trip while another thread polls the same device and for NV12 and P010 textures it left uncleared, and with NV12 and P010 textures of more than one mip level or a dimension other than 2D refused, as Direct3D 12 and Vulkan refuse them. `vendor/wgpu-core/CHANGES.md` describes each change. Cargo applies patches only from the project it builds, so a project names it as the module's own `Cargo.toml` does.

The crate needs Rust 1.99 or later. It builds wgpu with the backends and features the module has. `static-dxc` links DXC, the DirectX shader compiler, into a Windows x64 build, which then needs what [Building From Source](Building-From-Source.md) lists for that. A project that leaves it out builds smaller, and wgpu's Direct3D 12 backend then loads a `dxcompiler.dll` from the PATH, or else compiles with FXC, which has no shader model 6 features and cannot compile the BC6H and BC7 encoders. `avif` links rav1d, the AV1 decoder, and avif-parse, for the module's image import alone, so a project without it links neither. Its types are at the crate root: `Gpu`, `GpuOptions`, `ComputeProgram`, `RenderProgram`, `RenderOptions`, `Draw`, `Binding` and `BindingInfo`, the `RunSpec`, `Dispatch` and `RunReport` of a compute run, the `ElementType` of the module's buffers and the `host_bytes`, `to_clr_array` and `to_clr_memory` that convert PowerShell arrays as the cmdlets do, the `PrimitiveTopology`, `CullMode`, `BlendMode`, `BlendFactor`, `BlendOperation` and `CustomBlend` the cmdlets take, the `PsError` its methods return, and `wgpu` itself, for anything the library does not wrap. bytemuck above turns numbers into the bytes buffers hold and back.

## A device, a computation and a drawing

`examples/library.rs` in the repository opens a device, doubles four numbers with a compute program, draws into a texture with a render program, and shows an error:

```rust
use wgpu_pwsh::{Binding, Draw, Gpu, GpuOptions, PsError, RenderOptions, wgpu};

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
```

`DOUBLE` is a compute shader that doubles each value of its storage buffer `values`, and `COVER` a vertex and fragment shader whose one triangle covers the target in one color. `cargo run --example library --no-default-features` in the repository printed:

```text
NVIDIA GeForce RTX 5070 through Vulkan
main runs [64, 1, 1] invocations a workgroup and binds ["values"]
doubled: [2.0, 4.0, 6.0, 8.0]
first pixel: [51, 102, 204, 255]
refused as WgpuBindingMissing (InvalidArgument): no resource was given for values (0:0)
```

## What each part does

`Gpu::open` opens a device with every feature and limit its adapter has, as Open-WgpuDevice does; `GpuOptions` chooses the adapter by name, low power or the software one, and the backends to look through. Clones of a `Gpu` share one device, and every method takes `&self` from any thread.

A `ComputeProgram` is a compiled entry point, run as often as needed: `run` binds each resource to the binding of that name, or of that "group:binding", and dispatches the workgroups given, waiting for the GPU. `compute_program` compiles WGSL, and `compute_program_glsl` a GLSL compute shader, whose specialization constants take values by `constant_id`. A `RenderProgram` draws into textures of the format its `RenderOptions` names, with the topology, culling, blending, depth and samples given there; a `Draw` gives it vertex buffers by input name, an index buffer, bindings, the clear color and the blend constant. Both expose their wgpu pipeline for passes recorded with wgpu directly, and both record into a pass the caller began, as [Draws and dispatches in a pass of your own](#draws-and-dispatches-in-a-pass-of-your-own) shows.

## Dispatch, immediate data and timing

`run_with` takes a `RunSpec`, which gives a run what Invoke-WgpuCompute's parameters give it: the bindings; a `Dispatch` of workgroups, of invocations divided by the workgroup size, of counts the GPU reads from a buffer with INDIRECT usage, or, by default, one invocation per element of the first buffer bound; immediate data; and whether to time the pass. The `RunReport` it returns gives the workgroups dispatched, the GPU's nanoseconds in the pass on a device with the TIMESTAMP_QUERY feature, and the milliseconds the call took:

```rust
use wgpu_pwsh::{Binding, Dispatch, RunSpec};

let bindings = [("values", Binding::Buffer(&values))];
let report = double.run_with(&RunSpec { bindings: &bindings, dispatch: Dispatch::Invocations([4, 1, 1]), timed: true, ..Default::default() })?;
println!("{:?} workgroups, {:?} ns on the GPU", report.workgroups, report.gpu_time_ns);
```

A shader reads immediate data through a `var<immediate>` of a struct type, which every backend compiles; wgpu's Direct3D 12 backend refuses one of another type, as naga's HLSL writer takes only a struct.

## Arrays in the types a shader declares

`binding_info` gives what a compute program's shader declares for each binding it uses: the name, the group and binding, the `ElementType` of the numbers in a buffer, and whether the shader may write it. `compute_binding_info` and `compute_binding_info_glsl` give the same from the shader's text with no device, validating it as Test-WgpuShader does. `ElementType::convert` turns an array into another element type as Invoke-WgpuCompute does: a float truncates toward zero and saturates into an integer type, and an integer wraps into a narrower one. Together they prepare arrays as Invoke-WgpuCompute prepares them for each binding:

```rust
use wgpu_pwsh::ElementType;

let values = [1.5f64, -2.5, 300.0];
for binding in program.binding_info() {
    let Some(ty) = binding.element_type else { continue };
    let bytes = ElementType::Float64.convert(bytemuck::cast_slice(&values), ty);
    println!("{} ({}:{}) takes {} bytes of {ty:?}; the shader writes it: {}", binding.name, binding.group, binding.binding, bytes.len(), binding.writable);
}
```

A render program's `binding_info` gives the same for the bindings its vertex and fragment shaders use between them, and `vertex_input_info` each `@location` input of its vertex shader: the name, the location, the `ElementType` of each number and how many numbers one vertex's value holds. An array converts to that type as `Invoke-WgpuRender` converts it, and its length divided by the components counts the vertices. `render_binding_info` and `render_vertex_input_info` give both from the shader's text with no device:

```rust
for input in wgpu_pwsh::render_vertex_input_info(wgsl, None)? {
    let vertices = values.len() / input.components as usize;
    println!("{} at location {}: {vertices} vertices of {} {:?}", input.name, input.location, input.components, input.element_type);
}
```

A PowerShell module built with PWRS on the library reads its arguments as the cmdlets do. `host_bytes` takes a PowerShell object and the element type wanted, and returns the type and bytes: a typed .NET array is read in place and converted by `ElementType::convert`, untyped numbers go through PowerShell's casts, and a string, null or an integer out of range is refused with the error id the cmdlets raise. `to_clr_array` and `to_clr_memory` turn bytes back into the .NET arrays and `Memory<T>` the cmdlets return. All three need a PowerShell host.

## A device another crate opened

A program that already has a wgpu device, such as the one a window's renderer draws with, gives it to the library with `Gpu::from_parts`, with the adapter it was opened on, its queue and a label:

```rust
let gpu = Gpu::from_parts(adapter, device, queue, "renderer".to_string())?;
```

The `Gpu` installs the error and device-lost callbacks a device `Gpu::open` opens has, in place of any the device had: an error wgpu raises outside any call is returned by the next call, and a lost device's reason by `lost_reason`. Its features and limits are those the device was opened with. A program whose renderer draws on the device between the library's calls takes those errors itself, oldest first, and they are not returned again:

```rust
for error in gpu.take_errors() {
    eprintln!("the renderer's device reported: {error}");
}
```

## Draws and dispatches in a pass of your own

A program that records its own frame, such as a window drawing with a GUI toolkit's encoder, records the library's draws and dispatches into passes it begins itself and submits them with the rest of the frame. `RenderProgram::record` binds a `Draw`'s resources and vertex buffers by name, as `draw` does, and draws into the pass; the program's `format`, `sample_count` and `depth_format` give the attachments the pass needs. `ComputeProgram::record` binds a `RunSpec`'s resources, sets its immediate data, dispatches into a compute pass and returns the workgroups it dispatched. `examples/record.rs` in the repository records one of each into one encoder, the render pass on a multisampled color attachment with depth, resolved into `target`:

```rust
let cover = gpu.render_program(COVER, &RenderOptions { sample_count: 4, depth: true, ..Default::default() })?;
println!("draws into {:?} at {} samples, depth {:?}", cover.format(), cover.sample_count(), cover.depth_format());

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
```

`device` is `gpu.device()`, `double` and `bindings` double four values as in the first section, `resolved` is a view of `target`, and `color` and `depth` are views of textures the example makes at the program's sample count, in its format and its depth format. `cargo run --example record --no-default-features` in the repository printed:

```text
draws into Rgba8Unorm at 4 samples, depth Some(Depth32Float)
recorded [1, 1, 1] workgroups
refused as WgpuRecordClear: record draws into a pass the caller began, whose load decides what the target holds first: give the draw a clear of None
doubled: [2.0, 4.0, 6.0, 8.0]
first pixel: [51, 102, 204, 255]
```

Nothing runs until the program submits its encoder. The pass's load decides what the target holds first, so `record` refuses a `Draw` with a `clear`, a `clear_depth` or a `clear_stencil`, and timing needs queries around the whole pass, so it refuses a timed `RunSpec`; it refuses what `draw` and `run_with` refuse too, before it records anything. A program whose `RenderOptions` give a `depth_format` with a stencil aspect and a `StencilTest` tests stencil, against a `Draw`'s `stencil_reference` when it gives one and the pass's otherwise, and a `DepthTest` sets how it tests depth, as the cmdlets' depth and stencil parameters do. A fault in the recorded commands themselves is found when the encoder is submitted, and `take_errors` returns it.

## Errors

The methods return `PsError`, which displays as `[id] message`. Its `error_id` is the one the module's cmdlets raise for the same failure, and its `category` the PowerShell category they give it, so the [Error Reference](Error-Reference.md) describes both.
