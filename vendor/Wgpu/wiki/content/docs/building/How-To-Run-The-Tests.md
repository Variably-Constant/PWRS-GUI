---
title: How To Run The Tests
weight: 2
---

Check a change before committing it: lint the crate as the module and as the bare Rust library, run the unit tests and the Pester suites in both hosts, run the Rust library's example, run the tests on another adapter and on another platform, and add tests for what the change does.

The output quoted on this page is from one run of each command on the machine [Where It Has Been Run](Where-It-Has-Been-Run.md) describes, of the code this page describes.

## Lint the crate both ways

```text
cargo clippy -p wgpu-pwsh --no-deps --all-targets -- -D warnings
cargo clippy -p wgpu-pwsh --no-deps --all-targets --no-default-features -- -D warnings
cargo doc -p wgpu-pwsh --no-deps
cargo doc -p wgpu-pwsh --no-deps --no-default-features
```

The first two lint the crate as the PowerShell module and as the bare Rust library, the example included; both exit 0 with no warnings. Run the `cargo doc` commands with `RUSTDOCFLAGS` set to `-D warnings`: they document both and exit 0. Code that compiles only on Unix is linted only on a Unix host, so lint a change to it there too.

## Run the unit tests and the Pester suites

```text
cargo pwrs test --release
```

`cargo pwrs` here is cargo-pwrs 0.8.0, installed as [Building From Source](Building-From-Source.md) sets it up. It runs the Rust unit tests and `tests/version_references.rs`, which checks that the crates.io README, the Rust library's page and the CHANGELOG name the crate's version, builds the module, and runs every `tests/*.Tests.ps1` in PowerShell 7 and then in Windows PowerShell 5.1, with `PWRS_MODULE` naming the module folder. Both hosts run the suites under Pester 6.2.0, which cargo-pwrs fetches into its toolchain the first time, or under the Pester `PWRS_PESTER_PATH` names when it is set; the suites run on Pester 5 and 6. Each host prints a line such as `pwrs pester host=7.6.7 pester=6.2.0 passed=<n> failed=<n>`, and a failing host ends the run. Then the surface check reports any cmdlet no test ran, any pipeline parameter no test fed, and any cmdlet that declares `-WhatIf` but never asked. The lines from the run:

```text
test result: ok. 179 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 15.08s
test result: ok. 4 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
pwrs: Pester in pwsh
[+] C:\Projects\Wgpu\tests\Compute.Tests.ps1 1.79s (38 tests)
[+] C:\Projects\Wgpu\tests\Render.Tests.ps1 298ms (24 tests)
[+] C:\Projects\Wgpu\tests\Shader.Tests.ps1 601ms (47 tests)
[+] C:\Projects\Wgpu\tests\Texture.Tests.ps1 5.85s (124 tests)
[+] C:\Projects\Wgpu\tests\Wgpu.Tests.ps1 18.16s (73 tests)
Tests Passed: 306, Failed: 0, Skipped: 0, Inconclusive: 0, NotRun: 0
pwrs pester host=7.6.7 pester=6.2.0 passed=306 failed=0
pwrs: Pester in Windows PowerShell
[+] C:\Projects\Wgpu\tests\Compute.Tests.ps1 1.79s (38 tests)
[+] C:\Projects\Wgpu\tests\Render.Tests.ps1 399ms (24 tests)
[+] C:\Projects\Wgpu\tests\Shader.Tests.ps1 646ms (47 tests)
[+] C:\Projects\Wgpu\tests\Texture.Tests.ps1 6.17s (124 tests)
[+] C:\Projects\Wgpu\tests\Wgpu.Tests.ps1 16.57s (73 tests)
Tests Passed: 306, Failed: 0, Skipped: 0, Inconclusive: 0, NotRun: 0
pwrs pester host=5.1.26100.9711 pester=6.2.0 passed=306 failed=0
pwrs: surface check: every cmdlet ran, every pipeline parameter took a record, every ShouldProcess cmdlet called should_process
```

The unit tests drive the Rust library: buffers, compute and render programs, a device opened with wgpu itself and taken by `Gpu::from_parts`, whose error raised outside any call the next call returns, errors raised outside any call taken in order by `take_errors` and cleared, several threads on one device, and devices opened while other threads list adapters and exit, which a watchdog ends if the process deadlocks. They also cover the texture code: the ASTC decoder, checked against Basis Universal's and texture2ddecoder's decoding of the same blocks, and the EAC decoder, against the specification and texture2ddecoder; DDS and KTX2 files written and read back; each kind of pixels the AVIF decoder gives made into an image sample for sample, and an AVIF the image crate's encoder writes read back; Basis Universal's encoding and transcoding; mip levels rendered on the GPU by each filter, Triangle and Lanczos against the image crate's resize of level 0, the Wrap and Mirror edges against its resize of the image tiled three by three, Kaiser and Mitchell against values from numpy's sinc and Bessel function and from Mitchell and Netravali's cubic, and Min and Max against a scan on the CPU; texels packed into every color layout on the CPU; BC encoding on the GPU, with BC7 encoded a few rows at a time checked against the whole level at once, and an opaque picture's alpha kept at 255 through BC7 at each of five efforts; and a compressed texture decoded for an image on the CPU and on the GPU. Resizes are checked against the image crate's resize with the same filter, Box against the exact area average, and crops, rotations, flips and padding texel for texel. Channel and color changes are checked against ImageMagick's formulas for brightness, contrast, gamma and negate, texconv's Reinhard curve and three.js's ACES Filmic, AgX and Neutral, Pillow's grayscale, and the sRGB transfer functions. Normal maps made from heights are checked against texconv's normals and occlusion for the same height map, against a reference in doubles for every kernel, edge and alpha, and for a slope of exactly 0 wherever the heights match across a texel; their mip levels against unit length, and their conversions against the normals written again as each format stores them. A comparison's PSNR and SSIM are checked against scikit-image's on the same texels, with the Gaussian window of Wang et al.'s SSIM, and a measurement's statistics against numpy's; integer, depth, stencil, multisampled, NV12 and P010 textures are read against the values they were written or cleared with, an NV12 texture's planes as Red and as Green and Blue at their own sizes, and the difference picture against its log scale. Cube maps are checked against a reference in doubles that `tools/cube_reference.py` computes apart from the shaders, for every projection both ways, the irradiance, the specular levels of both mappings and the BRDF table, and against Filament's cmgen for the harmonics, the Quadratic levels and the table; every layout gives its faces back texel for texel. One checks that a wait for a submission ends at an interrupt, as Ctrl+C does, that a wait whose submission thread has ended raises `WgpuSubmitThread`, and that a panic in a submit is raised as `WgpuPanic` with its message, as a panic in a task's work is. Another checks that the errors a device reports during a call are all reported, an internal or out-of-memory one before a validation one. The blend tests build the blend of each mode, tell which blends read the blend constant, and draw by the constant a draw sets. The depth and stencil tests build the state each test names, refuse a test its depth target cannot hold, draw reversed depth against a target started at 0, and keep a draw to where an earlier one wrote the stencil reference. The Pester suites run every cmdlet, as the surface check confirms, and many of the classes' methods, with their errors, on the default device; on a host with no adapter they skip the cases that need a GPU, saying so. One stops Wait-WgpuDevice partway through queued work, as Ctrl+C does, and checks that it returns while the GPU is still busy. Others check that the default device stays the default when another runspace that imported the module closes, a parallel run's included, and that the five cmdlets that expand wildcards in `-Path` say so in their help.

## Run the Rust library's example

```text
cargo run --example library --no-default-features
```

It builds the crate as a Rust project that depends on it would, and prints what [How To Use The Rust Library](How-To-Use-The-Rust-Library.md) shows.

## Run them on another adapter

To reproduce a failure that shows only on one adapter, such as WARP, the software adapter Windows has where it has no GPU, run the tests on that adapter. The unit tests open their devices on the adapter wgpu prefers, or, when `WGPU_PWSH_TEST_ADAPTER` is set, on the first adapter whose name contains it; `Basic Render` names WARP:

```powershell
$env:WGPU_PWSH_TEST_ADAPTER = 'Basic Render'
cargo test -p wgpu-pwsh --lib
```

```text
test result: ok. 179 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 24.54s
```

The Pester suites run on the module's default device, so open a device on that adapter as the default and run them in the same session, here from the repository's root:

```powershell
$env:PWRS_MODULE = Join-Path $PWD target\pwrs\Wgpu
Import-Module (Join-Path $env:PWRS_MODULE Wgpu.psd1)
$null = Open-WgpuDevice -Fallback -Backend Dx12 -Default
Invoke-Pester -Path tests
```

```text
Tests Passed: 306, Failed: 0, Skipped: 0, Inconclusive: 0, NotRun: 0
```

Pester runs the files in the order of their names, and the last, `Wgpu.Tests.ps1`, checks that `Open-WgpuDevice -Default` replaces the default device, so the tests after that one run on the adapter wgpu prefers.

## Run them on another platform

The same commands run on Linux, macOS and FreeBSD, and on Linux for a glibc as well:

```text
cargo pwrs test --release --target x86_64-unknown-linux-gnu.2.35
```

That builds the library through cargo-zigbuild against glibc 2.35, then tests it in the host's PowerShell 7. [Where It Has Been Run](Where-It-Has-Been-Run.md) lists the platforms these runs have covered.

## Add a test

Put a unit test in the `mod tests` at the end of the file in `src/` whose code it checks. A test that needs a device opens it with its module's helper, which picks the adapter as above and, on a host with no adapter, prints `skipped: this host has no GPU adapter` and returns `None`, and the test then returns at once.

Put a Pester test in the file of the part of the module it checks: `Compute.Tests.ps1` for compute, maps, measures, encoders and queries; `Render.Tests.ps1` for drawing; `Shader.Tests.ps1` for checking, describing, translating and loading shaders, which needs no GPU; `Texture.Tests.ps1` for textures, samplers, image files and their conversions; and `Wgpu.Tests.ps1` for the aliases, adapters, devices, buffers, tasks and events. A case that needs a GPU takes `-Skip:$noGpu`, which each file sets in its `BeforeDiscovery` block when `Get-WgpuAdapter` lists no adapter. A new cmdlet no test runs, or a pipeline parameter no test feeds, shows in the surface check.
