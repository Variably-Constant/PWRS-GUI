<div align="center">

# Wgpu

**GPU compute, offscreen rendering and shader tools for PowerShell, with wgpu compiled in.**

[![PowerShell Gallery](https://img.shields.io/powershellgallery/v/Wgpu?style=flat-square)](https://www.powershellgallery.com/packages/Wgpu)
[![crates.io](https://img.shields.io/crates/v/wgpu-pwsh?style=flat-square)](https://crates.io/crates/wgpu-pwsh)
[![Wiki](https://img.shields.io/github/actions/workflow/status/Variably-Constant/Wgpu/wiki-deploy.yml?branch=main&label=wiki&style=flat-square)](https://variably-constant.github.io/Wgpu/)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg?style=flat-square)](https://github.com/Variably-Constant/Wgpu/blob/main/LICENSE)
[![PowerShell](https://img.shields.io/badge/PowerShell-7.4%2B%20%7C%205.1-5391FE.svg?style=flat-square)](https://learn.microsoft.com/powershell/)

**[Wiki](https://variably-constant.github.io/Wgpu/)** | [Getting started](https://variably-constant.github.io/Wgpu/docs/tutorials/getting-started/) | [Reference](https://variably-constant.github.io/Wgpu/docs/reference/) | [Changelog](https://github.com/Variably-Constant/Wgpu/blob/main/CHANGELOG.md)

Buffers, textures, shaders and pipelines are objects a script holds, and compute shaders run over arrays or over buffers that stay on the GPU, through Direct3D 12, Vulkan, Metal or OpenGL. [wgpu](https://wgpu.rs/) 30 is compiled into the module, which runs in PowerShell 7.4 or later and Windows PowerShell 5.1 and is built with [PoWerRuSt](https://github.com/Variably-Constant/PWRS).

</div>

---

<details>
<summary><b>Table of contents</b></summary>

- [Features](#features)
- [Quick start](#quick-start)
- [Why](#why)
- [Cmdlets](#cmdlets)
- [Tested platforms](#tested-platforms)
- [Building from source](#building-from-source)
- [Repository layout](#repository-layout)
- [Wiki](#wiki)
- [Use of AI tools](#use-of-ai-tools)
- [License](#license)
- [Contributing](#contributing)

</details>

---

## Features

- [Compute shaders](https://variably-constant.github.io/Wgpu/docs/how-to/how-to-run-a-compute-shader/) in WGSL, GLSL or SPIR-V over arrays of any numeric type, or over buffers that stay on the GPU.
- [An expression over arrays, and sums, averages and extremes](https://variably-constant.github.io/Wgpu/docs/how-to/how-to-map-and-reduce-arrays/), with no shader to write.
- [Offscreen rendering](https://variably-constant.github.io/Wgpu/docs/how-to/how-to-render-offscreen/) into textures and image files, with depth and stencil tests, blending, culling and multisampling.
- [Textures](https://variably-constant.github.io/Wgpu/docs/how-to/how-to-work-with-textures-and-images/) in every format wgpu has, loaded from and saved to sixteen file formats, DDS and KTX2 whole with every mip level, layer and cube face.
- [Images turned as their files say and converted to sRGB from their color profiles](https://variably-constant.github.io/Wgpu/docs/reference/images/#follow-a-files-orientation-and-color-profile) as littleCMS converts them, CMYK and floating point D2B tables included, and PQ and HLG AVIF and JPEG XL files loaded as linear light.
- [SVG files and PDF pages drawn](https://variably-constant.github.io/Wgpu/docs/reference/images/#draw-an-svg-file-or-a-pdf-page) at the size or resolution asked for, never resampled: SVG text in the machine's fonts, and protected PDFs opened with a SecureString.
- [The planes of NV12 and P010 textures](https://variably-constant.github.io/Wgpu/docs/reference/images/#read-and-write-the-planes-of-nv12-and-p010) read, written and viewed one at a time, and compared and measured with the luma as red and the chroma as green and blue.
- [Images made into GPU-ready textures](https://variably-constant.github.io/Wgpu/docs/reference/images/#compress-a-texture): mip levels made on the GPU by Box, Triangle, Kaiser, Mitchell, Lanczos, Min or Max, with wrapped or mirrored edges for tiling textures, BC compression on the GPU, and the Basis Universal formats in KTX2 files.
- [Textures resized, cropped, padded, flipped and rotated](https://variably-constant.github.io/Wgpu/docs/reference/images/#change-a-textures-size-and-orientation) on the GPU, with five filters, compressed ones encoded again in their format.
- [Channels and colors changed](https://variably-constant.github.io/Wgpu/docs/reference/images/#change-a-textures-channels-and-colors) on the GPU: swizzles, premultiplied alpha, grayscale, ImageMagick's brightness, contrast, gamma and negate, exposure, four tone-mapping curves and color spaces, and channels of several textures packed into one.
- [Textures blurred, sharpened and outlined](https://variably-constant.github.io/Wgpu/docs/reference/images/#blur-sharpen-and-find-edges) on the GPU: a Gaussian blur, an unsharp mask, the Sobel, Scharr and Prewitt gradients, the Laplacian, and Canny's edges, checked against scipy and scikit-image.
- [Normal maps made from height maps](https://variably-constant.github.io/Wgpu/docs/reference/images/#make-normal-maps) on the GPU by texconv's kernel, Sobel's or NVIDIA's, kept unit length in every mip level, and laid out in two channels for BC5 or Basis Universal.
- [Cube maps and image-based lighting](https://variably-constant.github.io/Wgpu/docs/how-to/how-to-light-a-scene-with-an-environment/): panoramas, crosses, strips, and angular and octahedral maps made into cube maps and back on the GPU, and the irradiance map, the prefiltered specular map and the BRDF table that split-sum lighting reads, checked against Filament's cmgen.
- [Textures compared and measured](https://variably-constant.github.io/Wgpu/docs/reference/images/#compare-and-measure-textures): MSE, PSNR and SSIM against a reference for each level, layer and channel, a picture of the error on a fixed log scale, and each channel's minimum, maximum, mean, standard deviation and histogram.
- [Shader validation, reflection and translation](https://variably-constant.github.io/Wgpu/docs/how-to/how-to-check-and-translate-shaders/) from WGSL, GLSL and SPIR-V into WGSL, SPIR-V, GLSL, HLSL and MSL, with no GPU.
- [Every adapter](https://variably-constant.github.io/Wgpu/docs/how-to/how-to-choose-an-adapter/), opened with every limit it has and every feature but wgpu's experimental ones, which `-Experimental` adds.
- [Commands recorded by hand](https://variably-constant.github.io/Wgpu/docs/how-to/how-to-record-commands-by-hand/): passes, copies, timestamps, occlusion and pipeline-statistics queries.
- [Methods that return tasks](https://variably-constant.github.io/Wgpu/docs/how-to/how-to-wait-without-blocking/), with cancellation, and Ctrl+C that ends every cmdlet's wait.
- [Every failure](https://variably-constant.github.io/Wgpu/docs/reference/error-reference/) an error record or exception with an id of its own.
- A `Gpu` alias for every cmdlet, as `Invoke-GpuCompute` for `Invoke-WgpuCompute`.
- [The Rust library](https://variably-constant.github.io/Wgpu/docs/how-to/how-to-use-the-rust-library/) the cmdlets run on, for Rust programs, from crates.io.
- wgpu and its shader compilers compiled into the module, so no Vulkan SDK, DXC or Visual C++ Redistributable needs installing.

## Quick start

```powershell
Install-Module Wgpu -Scope CurrentUser
```

```powershell
PS> Invoke-WgpuMap 'x * x' ([float[]](1..5))
1
4
9
16
25

PS> Get-WgpuDevice | Select-Object Adapter, Backend, DeviceType
Adapter                 Backend  DeviceType
-------                 -------  ----------
NVIDIA GeForce RTX 5070  Vulkan DiscreteGpu

PS> Test-WgpuShader 'fn half(x: f32) -> f32 { let two: f32 = 2u; return x / two; }' -Diagnostics | Select-Object Line, Column, Message
Line Column Message
---- ------ -------
   1     30 the type of `two` is expected to be `f32`, but got `u32`
```

The first command squares five numbers on the GPU, the second shows the device it ran on, and the third checks a shader with no GPU at all. The wiki's [tutorials](https://variably-constant.github.io/Wgpu/docs/tutorials/) go on from here, a chapter at a time, to buffers, compute shaders, textures and drawing.

## Why

wgpu, the Rust implementation of WebGPU at the core of Firefox's WebGPU, is one validated API over Direct3D 12, Vulkan, Metal and OpenGL. This module compiles it into a native library that both hosts load, through [PWRS](https://github.com/Variably-Constant/PWRS), and puts it behind cmdlets and objects, so a script runs its own shaders on the GPU over the arrays it already holds. The wiki's [Why Wgpu](https://variably-constant.github.io/Wgpu/docs/explanation/why-wgpu/) says what that adds and what it does not do, and [How Data Moves](https://variably-constant.github.io/Wgpu/docs/explanation/how-data-moves/) measures what crossing to the GPU costs.

## Cmdlets

<details>
<summary><b>43 cmdlets, by what they act on</b></summary>

| Acts on | Cmdlets |
|---|---|
| Adapters and devices | [`Get-WgpuAdapter`](https://variably-constant.github.io/Wgpu/docs/reference/powershell/device-cmdlets/#get-wgpuadapter), [`Open-WgpuDevice`](https://variably-constant.github.io/Wgpu/docs/reference/powershell/device-cmdlets/#open-wgpudevice), [`Get-WgpuDevice`](https://variably-constant.github.io/Wgpu/docs/reference/powershell/device-cmdlets/#get-wgpudevice), [`Close-WgpuDevice`](https://variably-constant.github.io/Wgpu/docs/reference/powershell/device-cmdlets/#close-wgpudevice), [`Wait-WgpuDevice`](https://variably-constant.github.io/Wgpu/docs/reference/powershell/device-cmdlets/#wait-wgpudevice) |
| Buffers | [`New-WgpuBuffer`](https://variably-constant.github.io/Wgpu/docs/reference/powershell/buffer-cmdlets/#new-wgpubuffer), [`Read-WgpuBuffer`](https://variably-constant.github.io/Wgpu/docs/reference/powershell/buffer-cmdlets/#read-wgpubuffer), [`Write-WgpuBuffer`](https://variably-constant.github.io/Wgpu/docs/reference/powershell/buffer-cmdlets/#write-wgpubuffer), [`Copy-WgpuBuffer`](https://variably-constant.github.io/Wgpu/docs/reference/powershell/buffer-cmdlets/#copy-wgpubuffer), [`Clear-WgpuBuffer`](https://variably-constant.github.io/Wgpu/docs/reference/powershell/buffer-cmdlets/#clear-wgpubuffer), [`Remove-WgpuBuffer`](https://variably-constant.github.io/Wgpu/docs/reference/powershell/buffer-cmdlets/#remove-wgpubuffer) |
| Compute | [`Invoke-WgpuCompute`](https://variably-constant.github.io/Wgpu/docs/reference/powershell/compute-cmdlets/#invoke-wgpucompute), [`Invoke-WgpuMap`](https://variably-constant.github.io/Wgpu/docs/reference/powershell/compute-cmdlets/#invoke-wgpumap), [`Measure-WgpuArray`](https://variably-constant.github.io/Wgpu/docs/reference/powershell/compute-cmdlets/#measure-wgpuarray), [`New-WgpuComputePipeline`](https://variably-constant.github.io/Wgpu/docs/reference/powershell/compute-cmdlets/#new-wgpucomputepipeline), [`New-WgpuBindGroup`](https://variably-constant.github.io/Wgpu/docs/reference/powershell/compute-cmdlets/#new-wgpubindgroup) |
| Commands | [`New-WgpuCommandEncoder`](https://variably-constant.github.io/Wgpu/docs/reference/powershell/command-cmdlets/#new-wgpucommandencoder), [`Submit-WgpuCommand`](https://variably-constant.github.io/Wgpu/docs/reference/powershell/command-cmdlets/#submit-wgpucommand), [`New-WgpuQuerySet`](https://variably-constant.github.io/Wgpu/docs/reference/powershell/command-cmdlets/#new-wgpuqueryset) |
| Shaders | [`New-WgpuShader`](https://variably-constant.github.io/Wgpu/docs/reference/powershell/shader-cmdlets/#new-wgpushader), [`Test-WgpuShader`](https://variably-constant.github.io/Wgpu/docs/reference/powershell/shader-cmdlets/#test-wgpushader), [`Get-WgpuShaderInfo`](https://variably-constant.github.io/Wgpu/docs/reference/powershell/shader-cmdlets/#get-wgpushaderinfo), [`ConvertTo-WgpuShader`](https://variably-constant.github.io/Wgpu/docs/reference/powershell/shader-cmdlets/#convertto-wgpushader) |
| Textures and images | [`New-WgpuTexture`](https://variably-constant.github.io/Wgpu/docs/reference/powershell/texture-cmdlets/#new-wgputexture), [`Read-WgpuTexture`](https://variably-constant.github.io/Wgpu/docs/reference/powershell/texture-cmdlets/#read-wgputexture), [`Write-WgpuTexture`](https://variably-constant.github.io/Wgpu/docs/reference/powershell/texture-cmdlets/#write-wgputexture), [`Import-WgpuImage`](https://variably-constant.github.io/Wgpu/docs/reference/powershell/texture-cmdlets/#import-wgpuimage), [`Export-WgpuImage`](https://variably-constant.github.io/Wgpu/docs/reference/powershell/texture-cmdlets/#export-wgpuimage), [`Resize-WgpuTexture`](https://variably-constant.github.io/Wgpu/docs/reference/powershell/texture-cmdlets/#resize-wgputexture), [`Convert-WgpuTexture`](https://variably-constant.github.io/Wgpu/docs/reference/powershell/texture-cmdlets/#convert-wgputexture), [`ConvertTo-WgpuNormalMap`](https://variably-constant.github.io/Wgpu/docs/reference/powershell/texture-cmdlets/#convertto-wgpunormalmap), [`ConvertTo-WgpuCubeMap`](https://variably-constant.github.io/Wgpu/docs/reference/powershell/texture-cmdlets/#convertto-wgpucubemap), [`ConvertFrom-WgpuCubeMap`](https://variably-constant.github.io/Wgpu/docs/reference/powershell/texture-cmdlets/#convertfrom-wgpucubemap), [`ConvertTo-WgpuIrradianceMap`](https://variably-constant.github.io/Wgpu/docs/reference/powershell/texture-cmdlets/#convertto-wgpuirradiancemap), [`ConvertTo-WgpuSpecularMap`](https://variably-constant.github.io/Wgpu/docs/reference/powershell/texture-cmdlets/#convertto-wgpuspecularmap), [`New-WgpuBrdfTable`](https://variably-constant.github.io/Wgpu/docs/reference/powershell/texture-cmdlets/#new-wgpubrdftable), [`Join-WgpuTexture`](https://variably-constant.github.io/Wgpu/docs/reference/powershell/texture-cmdlets/#join-wgputexture), [`Compare-WgpuTexture`](https://variably-constant.github.io/Wgpu/docs/reference/powershell/texture-cmdlets/#compare-wgputexture), [`Measure-WgpuTexture`](https://variably-constant.github.io/Wgpu/docs/reference/powershell/texture-cmdlets/#measure-wgputexture), [`New-WgpuSampler`](https://variably-constant.github.io/Wgpu/docs/reference/powershell/texture-cmdlets/#new-wgpusampler), [`Get-WgpuTextureFormat`](https://variably-constant.github.io/Wgpu/docs/reference/powershell/texture-cmdlets/#get-wgputextureformat) |
| Rendering | [`New-WgpuRenderPipeline`](https://variably-constant.github.io/Wgpu/docs/reference/powershell/render-cmdlets/#new-wgpurenderpipeline), [`Invoke-WgpuRender`](https://variably-constant.github.io/Wgpu/docs/reference/powershell/render-cmdlets/#invoke-wgpurender) |

</details>

Each cmdlet's entry gives its syntax, every parameter with its type, what it takes from the pipeline and writes, and the errors it raises, and the [reference's task pages](https://variably-constant.github.io/Wgpu/docs/reference/) show them at work, with their output. The objects the cmdlets write are [32 classes](https://variably-constant.github.io/Wgpu/docs/reference/powershell/types/#classes), with methods for scripts that hold them, and the values they take are [42 enumerations](https://variably-constant.github.io/Wgpu/docs/reference/powershell/types/#enumerations).

## Tested platforms

Each platform's library was built from this repository with Rust 1.99.0, PoWerRuSt 0.8.0 and cargo-pwrs 0.8.0, linted clean with warnings denied, and tested:

| Platform | Machine and adapter | PowerShell | Unit tests | Pester tests |
|---|---|---|---|---|
| Windows x64 | Windows 11 on an NVIDIA GeForce RTX 5070, and on Microsoft's software renderer, WARP, in 7.6.7 and 5.1 | 7.6.7, 7.5.12, 7.4.21, 5.1 | 179 passed | 306 passed |
| Windows Arm64 | GitHub's runner, on WARP | 7.6.8, 5.1 | 179 passed | 306 passed |
| Linux x64 | Ubuntu virtual machine with no GPU, on a software adapter | 7.6.6 | 179 passed | 306 passed |
| Linux Arm64 | GitHub's runner, on Mesa's llvmpipe | 7.6.6 | 179 passed | 306 passed |
| macOS on Apple silicon | GitHub's runner, on Apple's paravirtual GPU through Metal | 7.6.6 | 179 passed | 300 passed, 6 skipped |
| macOS on Intel | GitHub's runner, on Apple's paravirtual GPU through Metal | 7.6.4 | 179 passed | 300 passed, 6 skipped |
| FreeBSD x64 | FreeBSD 15.0 virtual machine with no GPU, on a software adapter | 7.5.5 | 179 passed | 306 passed |

The six skipped on macOS test features that adapter lacks. Every example in the wiki ran on Windows x64 in PowerShell 7.6.7 with the output its page shows, and in Windows PowerShell 5.1 with no error its page does not show. The wiki's [Where It Has Been Run](https://variably-constant.github.io/Wgpu/docs/reference/where-it-has-been-run/) has each run's versions in full, and what has not been run.

## Building from source

The module builds from this repository with Rust 1.99.0, which `rust-toolchain.toml` names and rustup installs, [PoWerRuSt](https://crates.io/crates/PoWerRuSt) 0.8.0 from crates.io, and cargo-pwrs 0.8.0, the tool of the same release. On Windows x64 it needs the Visual Studio 2022 Build Tools with their ATL component, which the statically linked DXC uses, and every x64 build needs [nasm](https://www.nasm.us/) on `PATH` for the AVIF decoder's assembly:

```text
cargo install cargo-pwrs --version 0.8.0 --locked
cargo pwrs build --release
cargo clippy -p wgpu-pwsh --no-deps --all-targets -- -D warnings
cargo pwrs test --release
```

The module folder is `target\pwrs\Wgpu`. The wiki's [Building From Source](https://variably-constant.github.io/Wgpu/docs/building/building-from-source/) lists what each platform's build needs and what the folder holds, [How To Run The Tests](https://variably-constant.github.io/Wgpu/docs/building/how-to-run-the-tests/) what each test layer covers, and [How To Update The Docs](https://variably-constant.github.io/Wgpu/docs/building/how-to-update-the-docs/) how the wiki's reference and example output are generated.

## Repository layout

| Path | Role |
|---|---|
| `src/lib.rs` | the module's exports, and the Rust library's |
| `src/device.rs`, `src/names.rs` | instances, adapters and devices; features, limits and backends |
| `src/buffer.rs`, `src/element.rs` | buffers, and the conversions between PowerShell's arrays and their elements |
| `src/shader.rs` | parsing, validating, reflecting and translating shaders with naga, with no device |
| `src/compute.rs`, `src/pipeline.rs`, `src/binding.rs` | compute runs, expressions and reductions; pipelines; resources bound to a shader |
| `src/encoder.rs`, `src/render.rs` | command encoders, passes, command buffers and query sets; render pipelines and offscreen drawing |
| `src/texture.rs` | textures, views, samplers and the format list |
| `src/image_io.rs`, `src/texfile.rs`, `src/convert.rs`, `src/texel.rs` | image, DDS and KTX2 files; mip levels and format conversions on the GPU; BC encoding; texels packed on the CPU |
| `src/transform.rs`, `src/color.rs`, `src/alpha.rs`, `src/normal.rs` | a texture's size and orientation; its channels and colors, and channels of several textures packed into one; alpha for cutouts; normal maps |
| `src/filter.rs`, `src/filter_reference.rs`, `tools/filter_reference.py` | blurs, unsharp masks and edges; the numbers their tests check against, which the Python script computes with scipy and scikit-image |
| `src/compare.rs` | textures compared with a reference, and measured |
| `src/cube.rs`, `src/cube_reference.rs`, `tools/cube_reference.py` | cube maps and the lighting maps; the numbers their tests check against, which the Python script computes in doubles |
| `src/astc.rs`, `src/basis.rs`, `build.rs`, `native/` | the ASTC decoder; Basis Universal's encoder and transcoder, built by `build.rs`, with the C file that catches what it prints |
| `vendor/` | Basis Universal 2.50 as its release ships it, block_compression 0.11.0's shaders with the changes `vendor/block_compression/CHANGES.md` describes, and wgpu-core 30.0.1 with the changes `vendor/wgpu-core/CHANGES.md` describes |
| `src/tasks.rs`, `src/error.rs` | methods that return tasks; the error records the module raises |
| `src/gpu.rs` | the Rust library, with its unit tests |
| `src/faults.rs` | the report the unit tests write on Windows of a fault that ends their process |
| `tests/*.Tests.ps1` | the Pester suites `cargo pwrs test` runs in both hosts |
| `tests/version_references.rs` | the check that the crates.io README, the Rust library's page and the CHANGELOG name the crate's version |
| `examples/library.rs` | the Rust library used with no PowerShell |
| `tools/` | the generator of the wiki's reference and the error lists it reads, the runner that writes every example's output, the wiki's images and shader slots, and its link, highlighting and wording checks |
| `wiki/` | the Hugo site the wiki is published as, on the Hextra theme |
| `licenses/` | license files the notices quote for crates whose packages carry none, and for code compiled in that its own files do not cover, and where the source of the MPL-2.0 crate the library links is |
| `rust-toolchain.toml`, `.cargo/config.toml` | the Rust release every build uses; the baseline CPUs and the static C runtime |
| `Cargo.toml`, `Cargo.lock` | the crate, `wgpu-pwsh`, and the exact versions of its dependencies every build compiles |
| `README.crates.md` | the README crates.io shows for the crate |
| `CHANGELOG.md` | what changed in each version |
| `LICENSE` | MIT |

## Wiki

The documentation is published at [variably-constant.github.io/Wgpu](https://variably-constant.github.io/Wgpu/), built from [`wiki/content`](https://github.com/Variably-Constant/Wgpu/tree/main/wiki/content) by the Diataxis framework: a tutorial in seven chapters, twenty-one how-to guides, four explanations, a reference of nine pages by task with every cmdlet's syntax and parameters, every class and enumeration and every error id, the record of where it has been run, and three pages on building, testing and the docs themselves. Every example on them was run, with the output it printed written into the page by `tools/Invoke-Documentation.ps1`.

## Use of AI tools

The author used Claude (Anthropic) via the Claude Code CLI for code development assistance, documentation drafting and test scripting during the preparation of this repository. All design decisions and the final content were determined by the author. The Rust implementation, the unit tests and the Pester suites were verified through zero-warning `cargo clippy` passes, `cargo pwrs test` in PowerShell 7 and Windows PowerShell 5.1, and the wiki's examples, run against the built module on Windows 11 x64.

## License

MIT, in [LICENSE](https://github.com/Variably-Constant/Wgpu/blob/main/LICENSE). The module compiles in wgpu and naga, MIT or Apache-2.0, and the image crate, MIT or Apache-2.0, with their dependencies; Basis Universal, Apache-2.0, with the zstd, tinyexr, qoi, tinydds and Android ASTC decoder code it carries, under the BSD-3-Clause, MIT and Apache-2.0 licenses; and block_compression's shaders, MIT. cargo-pwrs writes the notices the module carries into its folder: `THIRD-PARTY-NOTICES.txt` at the root, for the assemblies PWRS compiles and the four .NET assemblies it ships for Windows PowerShell, and one beside each platform's native library, with the license of each crate compiled into it.

## Contributing

Issues and pull requests go to [github.com/Variably-Constant/Wgpu](https://github.com/Variably-Constant/Wgpu). A change keeps clippy clean with warnings denied, with and without the module feature, and `cargo pwrs test --release` passing in both hosts, and an example added to the wiki shows the output `tools/Invoke-Documentation.ps1` wrote for it.
