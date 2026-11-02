# Changelog

What changed in each version of the Wgpu module. The repository ships as
a single root commit that is rewritten on every release, so this file is
the record of what came before it; the commit log is not.

## Unreleased

### Added

- `Import-WgpuImage` turns an image as its file's orientation says: an
  EXIF orientation in a JPEG, PNG, WebP or TIFF file, a PNG's in the text
  chunk ImageMagick writes it in included, or an AVIF file's rotation and
  mirror.
- `Import-WgpuImage` converts an image's colors to sRGB from the color
  profile its file carries, an ICC profile of RGB, gray or CMYK color or
  an AVIF file's CICP values, as littleCMS 2.19 converts with its
  optimizations off, perceptually: by the profile's own table for the
  intent, floating point D2B tables included, or by its matrix and
  curves, through moxcms where moxcms converts as littleCMS does. A CMYK
  JPEG or TIFF file with a profile converts from its inks as stored.
- A PQ or HLG AVIF file loads as linear light in sRGB's primaries, with
  BT.2408's HDR reference white of 203 nits at 1, in Rgba16Float.
- A file whose orientation or color profile the module cannot read or
  use loads as stored, with a warning that says why.

## 0.1.0

### Added

- The crate `wgpu-pwsh`, a PowerShell module named `Wgpu` built on
  PoWerRuSt 0.8.0 and wgpu 30.0.1, in Rust's 2024 edition with Rust
  1.99.0 and its LLVM 23, the newest stable release, pinned in
  `rust-toolchain.toml`. wgpu is compiled in with every backend and
  shader front end it has natively: Direct3D 12 with DXC linked in
  statically (Windows x64) or Microsoft's `dxcompiler.dll` shipped
  beside the library (Windows Arm64), Vulkan, OpenGL and OpenGL ES, Metal
  and MoltenVK (macOS), wgpu's noop backend, and WGSL, GLSL and SPIR-V
  input.
- Every library is built for its architecture's baseline CPU, with the C
  runtime linked in statically on Windows, so the module imports on any
  machine of its architecture without the Visual C++ Redistributable.
- Devices: `Get-WgpuAdapter` lists every adapter with its features,
  limits and downlevel flags; `Open-WgpuDevice` opens one with every
  feature and limit it supports, or the features named, with validation
  layers, API traces, memory hints and wgpu's experimental features on
  request; `Get-WgpuDevice`, `Wait-WgpuDevice` and `Close-WgpuDevice`,
  which takes `-WhatIf` and `-Confirm`. A
  device reports its status, object counts, allocator report and
  timestamp period, raises `Wgpu.DeviceLost` and `Wgpu.UncapturedError`
  engine events whose `MessageData` is a `Wgpu.DeviceEvent` naming the
  device, and drives RenderDoc and Xcode captures. A destroyed device
  finishes the work it was given, then raises `Wgpu.DeviceLost`.
- Adapters are looked for through Vulkan and Direct3D 12 on Windows, and
  through every backend elsewhere, unless `-Backend` names others. On
  Windows, NVIDIA's driver deadlocks the process when a thread that used
  OpenGL exits while another thread opens or closes a device, and wgpu
  uses OpenGL on every thread that lists GL adapters, so GL runs there
  only when asked for. Instances, and the default device, live until no
  runspace of the process has the module, so closing a device ends no
  thread, a runspace that removes the module or closes leaves the default
  device to the others, and the two libraries NVIDIA's Vulkan driver
  loads while holding its lock stay loaded. Each instance lists its
  adapters once, when it is made, because NVIDIA's Vulkan driver faults
  when threads query one instance's adapters at once; an adapter attached
  later appears once every runspace has removed the module and it is
  imported again.
- Methods that return tasks: `SubmitAsync`, `WaitAsync`, `RunAsync` and
  `DownloadAsync` on devices; `ReadAsync`, `ReadRangeAsync`,
  `ReadMemoryAsync`, `ReadIntoAsync`, `WriteAsync`, `FillAsync`,
  `ClearAsync` and `CopyToAsync` on buffers; `ReadAsync` and
  `ReadIntoAsync` on buffer slices;
  `DispatchAsync` on compute pipelines; `ReadAsync` and
  `ReadNanosecondsAsync` on query sets; `ReadAsync`, `SaveAsync`,
  `GenerateMipsAsync` and `CompressAsync` on textures. Each waits for the
  GPU on a thread of its own and takes an
  optional `CancellationToken` that ends the wait. A panic on that thread,
  or on a device's submission thread, is raised as `WgpuPanic` with its
  message. Ctrl+C ends every
  cmdlet's wait for the GPU. Each device submits its work on a thread of
  its own, in order, so these waits end promptly even where a driver's
  submit waits for earlier work, as Mesa's software Vulkan's does; an
  OpenGL device submits on the calling thread. Devices on WARP take
  turns: each is a queue of the one Direct3D 12 device the process has
  for WARP, and WARP crashes the process when two of its queues run work
  at once, so a submission there waits until WARP has finished the newest
  submission another device made. wgpu-core 30.0.1 is built with wgpu's
  fix for an assertion a wait could trip while another thread polled the
  same device, from `vendor/wgpu-core`.
- Buffers of eleven element types, from Int8 to Float64 and Float16:
  `New-WgpuBuffer`, `Read-WgpuBuffer` (arrays, element ranges, other
  types, `Memory<T>`, and with `-Destination` an array the caller holds),
  `Write-WgpuBuffer`, `Copy-WgpuBuffer`, `Clear-WgpuBuffer` and
  `Remove-WgpuBuffer`, the last two with `-WhatIf` and `-Confirm`; slices
  bind a range of a buffer. A .NET array of the element type is read in
  place, with no copy, and a read back copies the bytes once on the CPU,
  from the staging buffer the GPU fills straight into the array or
  `Memory<T>` it returns, or into the array `-Destination` or the
  `ReadInto` method of a buffer or a slice gives it. A read of 1 MiB or
  more has the GPU place its bytes where the destination's elements
  begin within a page of memory, since some PowerShell 7.6 processes copy
  many times slower between unequal offsets. Numbers with no .NET array
  type of their own are converted as PowerShell converts them: into an
  integer type rounded half to even, exactly up to 64 bits, and refused
  outside its range as `WgpuOutOfRange`; `-Value` of `Clear-WgpuBuffer`
  and the `Fill` and `FillAsync` methods take any number the same way.
- Shaders with no GPU: `New-WgpuShader`, `Test-WgpuShader` with
  diagnostics by line and column, `Get-WgpuShaderInfo` with entry points,
  workgroup sizes, bindings, struct layouts and override constants, and
  `ConvertTo-WgpuShader` to WGSL, SPIR-V, GLSL, HLSL, MSL and Graphviz.
  The last three take shader text or a `Wgpu.Shader` from
  `New-WgpuShader`, by position or from the pipeline. SPIR-V is written
  and read in Vulkan's clip space, so SPIR-V from Vulkan's tools, and
  SPIR-V written here, draw the right way up. SPIR-V written here carries
  the names of the shader's bindings, structs and members, as glslc's
  does, and a buffer naga's SPIR-V writer wraps in a struct of one unnamed
  member binds and reflects as that member, so the resources that bind a
  shader bind its SPIR-V too.
- Files read by name or by wildcard: `Import-WgpuImage`, `New-WgpuShader`,
  `Test-WgpuShader`, `Get-WgpuShaderInfo` and `ConvertTo-WgpuShader` take
  `-Path`, whose wildcards name every file they match, each read in turn,
  and which the help marks as taking wildcards, and `-LiteralPath`, which
  names one file as it is written, brackets included, and binds the
  `PSPath` of what `Get-ChildItem` pipes or the `FullName` of a .NET
  `FileInfo`. A file that fails is written as an
  error and the files after it are still read. `Wgpu.ShaderDiagnostic`
  and `Wgpu.ShaderInfo` name their shader in `Label`, its file's path for
  a file, and each result of `ConvertTo-WgpuShader` carries it as a
  `Label` note; `-OutFile` holds one shader and refuses a second.
- Compute: `Invoke-WgpuCompute` binds arrays, buffers, textures and
  samplers by position, by name, by group and binding, and struct
  uniforms from hashtables of their members, and dispatches by
  workgroups, by invocations or indirectly, with GPU timing;
  `New-WgpuComputePipeline`, `New-WgpuBindGroup`, `New-WgpuCommandEncoder`,
  `Submit-WgpuCommand` and `New-WgpuQuerySet` for recording work by hand;
  `Invoke-WgpuMap` runs WGSL expressions over arrays and
  `Measure-WgpuArray` sums, averages and finds their extremes, adding
  32-bit integers in 64 bits and every workgroup's sum in 128 bits; the
  sum it reports is a double, exact up to 2^53.
- Textures and images: `New-WgpuTexture`, `Read-WgpuTexture`,
  `Write-WgpuTexture`, `New-WgpuSampler`, `Get-WgpuTextureFormat` (every
  format wgpu has, with what the adapter can do with each),
  `Import-WgpuImage` and `Export-WgpuImage` for PNG, JPEG, WebP, AVIF,
  TIFF, GIF, BMP, TGA, OpenEXR, Radiance HDR, QOI, ICO, PNM and Farbfeld,
  AVIF read through rav1d, the Rust port of the dav1d decoder, at 8 bits,
  or at 16 for a file of 10 or 12, with its alpha, a file whose last box
  runs to its end, as Photoshop writes them, included. Without `-Usage`, a
  texture takes every usage its format allows on the device: what the
  adapter reports when the device has adapter-specific format features,
  and what WebGPU guarantees otherwise. A depth or stencil format is never
  a storage binding, as
  WebGPU defines them, although adapters report storage for Depth16Unorm
  and Depth32Float: Direct3D 12 from the color formats their views use,
  where WARP loses the device creating such a texture, and NVIDIA's
  Vulkan driver as well.
- DDS and KTX2 files whole: `Import-WgpuImage`, `Export-WgpuImage` and
  the `Save` method read and write every mip level, array layer and cube
  face, in the texture's own format, block-compressed ones included.
  KTX2 is read with Zstandard and zlib supercompression and in every
  Basis Universal format, which loads transcoded to the best format the
  device holds, and written with Zstandard on request. A block-compressed
  texture the device cannot hold is decoded on the CPU with a warning:
  BC by bcdec_rs, ETC2 by texture2ddecoder, and EAC and ASTC, HDR
  included, by decoders of the module's own. Saved as an image file, a
  block-compressed texture is decoded by the same decoders, or on the GPU
  with `-Decoder Gpu` and the decoder argument of `Save` and `SaveAsync`.
  `-Cube` on `New-WgpuTexture`
  and `Export-WgpuImage`, and `IsCube` on a texture, mark a cube map,
  which DDS and KTX2 files keep.
- Textures made ready for the GPU: `-Format`, `-MipLevels`, `-MipFilter`
  and `-MipEdge` on `Import-WgpuImage` and `Export-WgpuImage`, and the
  `GenerateMips` and `Compress` methods. Mip levels are rendered on the
  GPU in floating point, in linear light for an sRGB format, and rounded
  once, in every color format a shader reads as floating point: by Box,
  the default, each from the level above; by Triangle, Kaiser or
  Mitchell (NVTT's), or Lanczos 3, each from level 0, where `-MipEdge`
  Clamp leaves out the texels past an edge, Wrap takes the other side
  and Mirror the image mirrored, as a sampler's address mode also names
  them; or by Min or Max, the smallest or largest of the texels under
  each. A filter or an edge that changes nothing for any file is
  refused, and a warning says when the file had the levels or Basis
  Universal made them. BC1 to BC7 and BC6H are encoded on the GPU by
  block_compression's shaders, and signed BC4 and BC5 by the module's
  own; each format's encoder is built the first time it is used. The
  shaders look for each block's line of colors starting from the channel
  that varies most in it, not from block_compression's vector of ones,
  which Apple's GPUs never left for a gradient such as red to blue,
  encoding it as one flat color. The BC7 and BC6H shaders call each of
  their encoders from one place, so Direct3D 12's software adapter, WARP,
  compiles them; block_compression's BC7 shader calls its partition
  encoder from five places and its BC6H shader its mode test from twenty,
  and WARP crashes, stops responding or writes wrong blocks on them. BC7
  is encoded in four passes, since WARP on Arm64 compiles no shader the
  size of the whole encoder; the blocks are those of one pass. An opaque
  picture's BC7 copy decodes with alpha 255 in every texel: in an opaque
  encode, mode 6 keeps the p-bit with which its alpha endpoints decode as
  255, where block_compression chose it by the colors alone and left up
  to three quarters of a smooth gradient with alpha 254. The 34
  Basis Universal formats are encoded by Basis Universal 2.50, compiled
  in, with `-Quality` and `-Effort`, and what it prints becomes warnings
  and error reasons. A setting that does nothing for its format is
  refused. A texture's `Notes` say what making it involved.
- Size and orientation: `Resize-WgpuTexture` resizes a texture to a
  width or height, a scale, a size limit or powers of two, crops it to a
  region, a size or margins and pads it onto a larger canvas;
  `Convert-WgpuTexture` flips it and turns it by 90, 180 or 270 degrees.
  `Import-WgpuImage` and `Export-WgpuImage` take the same parameters, and
  the `Resize`, `Crop`, `Pad`, `Flip` and `Rotate` methods, each with an
  Async form, do the same. The filters are Nearest, Box, Bilinear,
  Bicubic (Catmull-Rom) and Lanczos 3, as the image crate computes them;
  Box shrinks and Bicubic enlarges by default. Each runs on the GPU for
  every layer and depth slice and returns a new texture in the texture's
  format with its mip levels rendered again: BC is encoded again on the
  GPU and ETC2, unsigned EAC and ASTC through Basis Universal, and a
  format the module has no encoder for comes back in the format its
  texels decode into, with a warning. A cube map is not flipped or
  rotated, and a 1D texture keeps its height of 1.
- Channels and color: `Convert-WgpuTexture` also swizzles a texture's
  channels by four letters, premultiplies and unpremultiplies alpha,
  makes it grayscale with Rec. 709's weights in linear light, Rec. 601's
  on the stored values as Pillow does, or weights given, inverts it and
  changes its brightness, contrast and gamma as ImageMagick's `-negate`,
  `-brightness-contrast` and `-gamma` do, multiplies it by stops of
  exposure, tone maps it with Reinhard (texconv's, from the brightest
  texel), ACES Filmic, AgX or Khronos PBR Neutral (three.js's), and
  converts or relabels its color space, all in one pass in a fixed order
  after any size change. `Join-WgpuTexture` and `[Wgpu.Texture]::Join`
  pack channels of several textures, or numbers, into one, resizing
  smaller textures to the largest. `Import-WgpuImage` and
  `Export-WgpuImage` take every switch, and the `Swizzle`, `Premultiply`,
  `Unpremultiply`, `ConvertColorSpace`, `AssignColorSpace`, `Grayscale`,
  `Invert`, `Adjust` and `ToneMap` methods, each with an Async form, do
  the same.
- Filters: `-Blur` blurs a texture by a Gaussian of a standard deviation
  in texels, or of one across and one down, reaching 4 standard
  deviations as scipy's `gaussian_filter` does, on linear values weighed
  by alpha; `-Sharpen` adds back that many times each texel's difference
  from a blur of it, an unsharp mask, with `-SharpenRadius` and
  `-SharpenThreshold`; and `-Edges` gives the size of each channel's
  Sobel, Scharr or Prewitt gradient or its Laplacian, or Canny's edges
  of the luminance as scikit-image finds them, with `-EdgeThreshold` and
  `-EdgeSigma`. `-FilterEdge` reads past the edge as Clamp, Wrap or
  Mirror. They run in compute passes after the changes to each texel and
  before the border fix, on each layer and depth slice alone, and the
  module's tests check them against scipy and scikit-image.
  `Convert-WgpuTexture` takes them all, `Import-WgpuImage` and
  `Export-WgpuImage` the blur and the unsharp mask, and the `Blur`,
  `Sharpen` and `FindEdges` methods, each with an Async form, do the same.
- Alpha for cutouts: `-ColorKey`, a color as red, green and blue from 0
  to 1 at the texture's own precision or as `'#RRGGBB'`, with
  `-ColorKeyTolerance`, gives alpha 0 to the texels of that color and
  keeps every other alpha; `-AlphaThreshold` makes alpha 0 below it and
  1 from it; `-FixAlphaBorder` gives each texel with alpha 0 the color
  of the nearest texel with alpha above 0, found exactly in two compute
  passes, and `-BorderReach` limits it to that many texels.
  `Convert-WgpuTexture`, `Import-WgpuImage` and `Export-WgpuImage` take
  all three, and the `ColorKey`, `AlphaThreshold` and `FixAlphaBorder`
  methods, each with an Async form, do the same. The key reads the
  colors as the texture holds them, the threshold cuts after a swizzle
  and unpremultiplying and before premultiplying, and the border fix
  runs last; with `-Premultiply` it is refused.
- `-KeepCoverage` scales the alpha of each mip level a call renders so
  the share of its texels at or above an alpha test's reference stays
  level 0's, counted on the GPU in steps of 1/4096: each layer against its
  own level 0, a 3D texture's every depth slice together. Import and
  Export take it with `-MipLevels`, `GenerateMips` as its fourth
  argument, and `Convert-WgpuTexture` and `Resize-WgpuTexture` take it
  with `-MipFilter` and `-MipEdge` for the levels they render again.
- Normal maps: `ConvertTo-WgpuNormalMap` makes one from a height map on
  the GPU, reading the heights from one channel or the luminance and
  taking the slopes by texconv's Prewitt kernel, central differences,
  Sobel's kernel or NVIDIA Texture Tools' 5 by 5, at a strength as
  texconv's `-nmapamp` sets it. Past the edges the slope at each edge
  carries on, or the map wraps, clamps or mirrors, and the Notes say when
  a map looks like it tiles. Green points up, as glTF reads it, or down
  with `-FlipGreen`; alpha holds 1, the height or texconv's occlusion
  term; a format that holds no negative values stores n / 2 + 1 / 2 and
  any other n; `-TwoChannel` keeps X and Y alone for a two-channel format
  or a Basis Universal LDR one; and each mip level's normals are scaled
  back to length 1, or filtered in slope space. `Convert-WgpuTexture`
  rebuilds Z, normalizes and flips green; `-NormalMips` keeps normals
  unit length in the levels `Resize-WgpuTexture`, `Convert-WgpuTexture`,
  `Import-WgpuImage`, `Export-WgpuImage` and `GenerateMips` render, and
  marks the texels as normals, which a `-Format` that stores them the
  other way takes written again; and Import and Export make heights into
  normals with `-HeightToNormal`.
  The `ToNormalMap`, `ReconstructZ`, `Normalize` and `FlipGreen` methods,
  each with an Async form, do the same. A test checks the normals against
  texconv's to within 1e-5.
- Export-WgpuImage refuses a `-Layer` past the texture's last layer with
  `WgpuLayer`, with or without a change of size.
- Compare and measure: `Compare-WgpuTexture` compares a texture with a
  reference of the same dimension, size and layers, its formats and mip
  levels free to differ, and writes a `Wgpu.TextureComparison` for each
  mip level both have, each layer and each channel both store: the mean
  squared error, the PSNR, and Wang et al.'s SSIM as scikit-image
  computes it with a Gaussian window of sigma 1.5, run on the GPU, `$null`
  for a level under 11 texels on a side. `-Difference` writes instead an
  Rgba8Unorm picture of the error on a fixed log scale, black at 2^-12 or
  less and full at 1 or more, and `-Channel` shows one channel as gray.
  `Measure-WgpuTexture` writes a `Wgpu.TextureMeasurement` for each mip
  level, layer and channel: minimum, maximum, mean, population standard
  deviation, and a histogram of `-Bins` bins over `-Range`, values outside
  it counted in the end bins. Values are read as stored, from 0 to 1, an
  sRGB texture's encoded, a block-compressed one's decoded on the CPU,
  snorm ones from -1 to 1, an integer format's through the norm format
  with its bits, depth from 0 to 1 and stencil over 255, and a
  multisampled texture's as a resolve would store them. PSNR's peak is 1,
  2 for values from -1 to 1, and for a float texture the reference
  channel's largest value. The `Compare`, `Difference` and `Measure`
  methods, each with an Async form, do the same, and the new enumeration
  `Wgpu.TextureChannel` names the channels.
- NV12 and P010: `-Aspect Plane0` reads and writes the luma, one value a
  texel at the texture's size, and `Plane1` the chroma, two values a
  texel at half its width and height, as bytes for NV12 and 16-bit values
  for P010; a view of a plane binds it as R8Unorm or Rg8Unorm, R16Unorm or
  Rg16Unorm, and a planar texture takes the usages both planes' formats
  allow, as wgpu-core checks them. `Compare-WgpuTexture` and
  `Measure-WgpuTexture` read the luma as Red and the chroma as Green and
  Blue at the chroma plane's size; a planar texture compares only with
  another, and the difference picture shows each chroma texel's error over
  the 2 by 2 texels that share it. The other commands that read a whole
  texture, such as `Export-WgpuImage`, refuse a planar one and name the
  planes `Read-WgpuTexture` reads.
  Direct3D 12 and Vulkan allow these formats one mip level and two
  dimensions, and wgpu-core 30.0.1 lets more through, which loses the
  device on Direct3D 12; the wgpu-core in `vendor/wgpu-core` refuses
  them. It also clears a new planar texture by copies into each plane,
  with wgpu's fix from its trunk: 30.0.1 left the chroma uncleared, and on
  NVIDIA's Vulkan driver the luma too, so a new texture read back what its
  memory held before.
- Cube maps and lighting: `ConvertTo-WgpuCubeMap` makes a cube map from
  six textures, its faces as they are, or from one image: a layout of
  square cells (a horizontal or vertical cross, a cross with -Z turned,
  a tee, strips and Godot's 2x3 and 3x2 grids), cut as it is, or a
  panorama, an angular map or an octahedral map, resampled on the GPU,
  each texel the mean of a grid of samples weighed by the solid angle
  each covers. The image's shape tells most of them apart and `-Layout`
  names the rest; `-Yaw` turns a projection and `-FaceSize` sizes its
  faces. `ConvertFrom-WgpuCubeMap` writes a cube into any of them, and a
  panorama with `-Panorama`; `Import-WgpuImage -Cube` and
  `Export-WgpuImage -Layout` and `-Panorama` do the same while loading and
  saving. `ConvertTo-WgpuIrradianceMap` sums the irradiance over the
  environment's exact texel solid angles, and `-Harmonics` gives its nine
  spherical harmonics a channel as cmgen's `--sh-irradiance` writes them;
  `ConvertTo-WgpuSpecularMap` prefilters the GGX lobe into mip levels from
  Hammersley samples read at the environment's mip levels, roughness
  stepping linearly as Khronos's glTF IBL sampler writes the levels or
  quadratically as cmgen does; and `New-WgpuBrdfTable` makes the
  split-sum table with Smith's height-correlated visibility. Tests check
  them against a double-precision reference computed apart from the
  shaders, and the harmonics, the quadratic levels and the table against
  Filament's cmgen 1.77.1. The `ToCubeMap`, `CubeFromFaces`, `ToLayout`,
  `ToPanorama`, `ToIrradianceMap`, `ToSpecularMap` and `NewBrdfTable`
  methods, each with an Async form, do the same, with the new
  `Wgpu.CubeLayout`, `Wgpu.RoughnessMapping` and `Wgpu.Harmonics`.
- A texture given whole to a shader binds through the view its binding
  reads: the binding's dimension over every layer and mip level, and of a
  format holding depth and stencil, the depth or the stencil by the
  binding's sample type. A binding that cannot read the texture whole is
  refused with `WgpuWrongResource`, naming `CreateView`. A cube map's
  `CreateView()` with no dimension is a cube view.
- `Convert-WgpuTexture` and `Resize-WgpuTexture` take `-Format`, as the
  other cmdlets that make a texture do, and `Convert-WgpuTexture -Format`
  alone converts each level. `-ColorSpace` switches a `-Format` that has
  an sRGB form to the form it names, on Import and Export as on these
  two, so a tone-mapped HDR texture becomes 8-bit sRGB with its values
  encoded once; an image that a transform reads loads whole, before a
  `-Format` converts it.
- .exr files are written with ZIP compression, in half floats for a
  texture of 16-bit float texels and full floats for any other. Every
  unsigned norm and float format saves as an image, the packed ones
  rendered into half floats first, and a two-channel texture saves as
  red and green with blue 0 and alpha 1, as DirectXTex expands one; a
  two-channel `-Format` loads red and green from a color image, and gray
  and alpha from a gray one.
- `Get-WgpuShaderInfo` writes a storage texture's type as WGSL does, as
  `texture_storage_2d<rgba8unorm, write>`.
- Rendering offscreen: `New-WgpuRenderPipeline` and `Invoke-WgpuRender`,
  with vertex inputs fed by name, indexed draws, depth, multisampling and
  blending, into a texture or an image file, and render passes recorded
  by hand. A blend is Replace, Alpha, Additive, Premultiplied, Constant,
  which mixes by `-BlendConstant` or a pass's `SetBlendConstant`, or
  Custom, whose factors and operations for color and alpha
  `-BlendSource`, `-BlendDestination`, `-BlendOperation` and their alpha
  forms name. A depth target is Depth32Float or any depth or stencil
  format `-DepthFormat` names, tested by `-DepthCompare`, written unless
  `-DepthWrite:$false`, with a depth bias, and started at `-ClearDepth`;
  a stencil test compares `-StencilReference` or a pass's
  `SetStencilReference` by `-StencilCompare` and changes the stencil by
  `-StencilFail`, `-StencilDepthFail` and `-StencilPass`, for each face
  with the `-BackStencil` forms and through `-StencilReadMask` and
  `-StencilWriteMask`. `BeginRenderPass` also takes a hashtable naming a
  pass's attachments and clears, whose clear keys take `'Load'` to keep
  what a texture holds.
- `-Constant`, `-Define` and `-Variable` take any dictionary, an
  `[ordered]` table as well as a hashtable.
- Every cmdlet has an alias with Gpu in its noun as well (`Invoke-GpuCompute`).
- A Rust library beside the module: the crate builds as an `rlib` too,
  and the module is built only with the default `module` feature, so a crate
  that depends on it with `default-features = false` links no module
  exports. DXC is linked only with the default `static-dxc` feature,
  which such a crate names to keep it, and rav1d and avif-parse, which
  read .avif files for `Import-WgpuImage`, only with the default `avif`
  feature, so such a crate links neither; a build without it refuses an
  .avif file. `Gpu` opens a device on the adapter asked for, or takes one another
  crate opened through `Gpu::from_parts`, with the module's error and
  device-lost callbacks installed, and `take_errors` returns and clears
  the errors raised outside its calls, as a renderer sharing the device
  raises them; it creates,
  writes and reads buffers and textures, which are wgpu's own types;
  `ComputeProgram` and `RenderProgram` compile WGSL once and run with
  resources bound by name, from any thread. `ComputeProgram::binding_info`
  gives each binding's element type and whether the shader writes it, as
  `compute_binding_info` and `compute_binding_info_glsl` do from a
  shader's text with no device,
  and `ElementType::convert` converts an array between element types as
  Invoke-WgpuCompute does. `ComputeProgram::run_with` takes a `RunSpec`
  of bindings, a dispatch by workgroups, invocations, counts in a buffer
  or the first buffer's length, immediate data and a request to time the
  pass, and `Gpu::compute_program_glsl` compiles GLSL compute shaders.
  `RenderProgram::record` and `ComputeProgram::record` bind by name and
  draw or dispatch into a pass the caller began, submitting nothing, for
  a program that records its own frame; `format`, `sample_count` and
  `depth_format` give the attachments a render program's pass needs, and
  `examples/record.rs` records one of each into one encoder.
  `RenderOptions` takes a `depth_format`, a `DepthTest` and a
  `StencilTest`, as the cmdlets' depth and stencil parameters do, and a
  `Draw` its `stencil_reference` and the `clear_depth` and
  `clear_stencil` its own depth target starts at.
  `RenderProgram::binding_info` gives each binding's group and number as
  the compute program's does, and `vertex_input_info` each vertex input's
  name, location, element type and component count, as
  `render_binding_info` and `render_vertex_input_info` do from a shader's
  text with no device.
  For a PowerShell module built on the library, `host_bytes` reads a
  PowerShell array as the cmdlets do, and `to_clr_array` and
  `to_clr_memory` write the arrays they return. `wgpu` is re-exported, and so
  is `PsError`, the error the methods return, whose `error_id` is the one
  the cmdlets raise for the same failure. `examples/library.rs` uses the
  library with no PowerShell. The crate is on crates.io as `wgpu-pwsh`,
  whose page shows `README.crates.md`, and `tests/version_references.rs`
  checks that it, the Rust library's page and this file name the crate's
  version.
- The notices in the module folder quote the license of every crate the
  native library links, of DXC, which the Windows x64 library carries,
  and of the code the module's own build compiles: Basis Universal, with
  the zstd, tinyexr, qoi, tinydds and Android ASTC decoder it carries,
  block_compression's shaders, and the tone-mapping curves ported from
  three.js. The last of the module's own texts says where the source of
  avif-parse is, the one crate under the Mozilla Public License 2.0,
  which that license requires a binary carrying it to say. `licenses/`
  holds the texts the packages
  leave out: the license files of the linked crates whose packages ship
  none, with SPDX's text of MIT beside that of the objc2 crates the macOS
  library links, which names their licenses without the text; DXC's; and
  the copyrights of the code Basis Universal compiles in that its own
  license files do not list.
