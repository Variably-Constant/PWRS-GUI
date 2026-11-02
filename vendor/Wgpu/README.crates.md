<div align="center">

# Wgpu

**GPU compute, offscreen rendering and shader tools for PowerShell, with wgpu compiled in.**

[![crates.io](https://img.shields.io/crates/v/wgpu-pwsh?style=flat-square)](https://crates.io/crates/wgpu-pwsh)
[![docs.rs](https://img.shields.io/docsrs/wgpu-pwsh?style=flat-square)](https://docs.rs/wgpu-pwsh)
[![PowerShell Gallery](https://img.shields.io/powershellgallery/v/Wgpu?style=flat-square)](https://www.powershellgallery.com/packages/Wgpu)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg?style=flat-square)](https://github.com/Variably-Constant/Wgpu/blob/main/LICENSE)

**[Wiki](https://variably-constant.github.io/Wgpu/)** | [The Rust library](https://variably-constant.github.io/Wgpu/docs/how-to/how-to-use-the-rust-library/) | [API reference](https://docs.rs/wgpu-pwsh) | [Repository](https://github.com/Variably-Constant/Wgpu) | [Changelog](https://github.com/Variably-Constant/Wgpu/blob/main/CHANGELOG.md)

This crate is the source of the Wgpu PowerShell module and the Rust library its cmdlets run on. Without its default `module` feature it is the library: a GPU device, and the buffers, textures and compute and render programs made on it, for Rust code on any thread. The module installs from the PowerShell Gallery with `Install-Module Wgpu`.

</div>

## Features

- [A device](https://variably-constant.github.io/Wgpu/docs/how-to/how-to-use-the-rust-library/#what-each-part-does) opened with every feature and limit its adapter supports, experimental features aside, on the adapter chosen by name, low power or the software one. Clones share it, and every method takes `&self` from any thread.
- [Compute programs](https://variably-constant.github.io/Wgpu/docs/how-to/how-to-use-the-rust-library/#what-each-part-does) from WGSL, with override constants, or from GLSL, with specialization constants, over buffers, textures, views and samplers bound by name or by group and binding.
- [Dispatches](https://variably-constant.github.io/Wgpu/docs/how-to/how-to-use-the-rust-library/#dispatch-immediate-data-and-timing) by workgroups, by invocations, by counts the GPU reads from a buffer, or one invocation per element of the first buffer bound, with immediate data and, on a device with timestamp queries, the GPU's time in the pass.
- [Render programs](https://variably-constant.github.io/Wgpu/docs/how-to/how-to-use-the-rust-library/#what-each-part-does) that draw into textures, with vertex buffers by input name, an index buffer, topology, culling, blending, depth, stencil and multisampling.
- [What a shader declares](https://variably-constant.github.io/Wgpu/docs/how-to/how-to-use-the-rust-library/#arrays-in-the-types-a-shader-declares) for each binding and vertex input, read from its text with no device, and arrays converted to those types as the cmdlets convert them.
- [A device another crate opened](https://variably-constant.github.io/Wgpu/docs/how-to/how-to-use-the-rust-library/#a-device-another-crate-opened), such as the one a window's renderer draws with, and [draws and dispatches recorded into a pass the caller began](https://variably-constant.github.io/Wgpu/docs/how-to/how-to-use-the-rust-library/#draws-and-dispatches-in-a-pass-of-your-own).
- [Errors](https://variably-constant.github.io/Wgpu/docs/how-to/how-to-use-the-rust-library/#errors) that carry the id the cmdlets raise for the same failure.
- `wgpu` itself, re-exported, for anything the library does not wrap.

## Quick start

A project leaves out the default `module` feature, which builds the PowerShell module, and the defaults `avif`, `jxl`, `svg` and `pdf`, which read .avif, JPEG XL, SVG and PDF files for the module's `Import-WgpuImage`, and keeps the sixth default, `static-dxc`, which links DXC, the DirectX shader compiler, into a Windows x64 build:

```toml
[dependencies]
wgpu-pwsh = { version = "0.1.0", default-features = false, features = ["static-dxc"] }
bytemuck = "1"

[patch.crates-io]
wgpu-core = { git = "https://github.com/Variably-Constant/Wgpu" }
```

The patch takes the wgpu-core the module builds with: 30.0.1 with the changes [`vendor/wgpu-core/CHANGES.md`](https://github.com/Variably-Constant/Wgpu/blob/main/vendor/wgpu-core/CHANGES.md) describes. Cargo applies patches only from the project it builds, so a project names it as the module's own `Cargo.toml` does.

[`examples/library.rs`](https://github.com/Variably-Constant/Wgpu/blob/main/examples/library.rs) opens a device, doubles four numbers with a compute program, draws into a texture with a render program, and shows an error:

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

## Documentation

- [How To Use The Rust Library](https://variably-constant.github.io/Wgpu/docs/how-to/how-to-use-the-rust-library/) goes through each part of the library with examples, and the [API reference](https://docs.rs/wgpu-pwsh) on docs.rs has every type and method.
- The [wiki](https://variably-constant.github.io/Wgpu/) has the PowerShell module's tutorials, how-to guides, explanations and reference.
- [Building From Source](https://variably-constant.github.io/Wgpu/docs/building/building-from-source/) lists what a build needs on each platform.

## Platforms

Windows x64 and Arm64, Linux x64 and Arm64, macOS on Apple silicon and on Intel, and FreeBSD x64, each with the crate's unit tests and the module's Pester tests passing; [Where It Has Been Run](https://variably-constant.github.io/Wgpu/docs/reference/where-it-has-been-run/) records the runs. The crate needs Rust 1.99 or later. Its build compiles Basis Universal's C++ with the cc crate and, on x64, the AVIF decoder's assembly with nasm.

## License

MIT, in [LICENSE](https://github.com/Variably-Constant/Wgpu/blob/main/LICENSE). The crate carries Basis Universal, Apache-2.0, with the zstd, tinyexr, qoi, tinydds and Android ASTC decoder code it carries, under the BSD-3-Clause, MIT and Apache-2.0 licenses; block_compression's shaders, MIT; and tone-mapping curves ported from three.js, MIT. Their license texts are in `vendor/` and `licenses/`.
