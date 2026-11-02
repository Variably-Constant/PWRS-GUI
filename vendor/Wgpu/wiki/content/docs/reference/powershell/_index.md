---
title: Cmdlets and types
weight: 100
sidebar:
  open: true
---

The module's 43 cmdlets, with each one's syntax and parameters on the page of its group, and every class and enumeration they write and take on [Types](Types.md). Each cmdlet also has an alias with `Gpu` in place of `Wgpu` in its noun: `Invoke-GpuCompute` is `Invoke-WgpuCompute`. The module runs in PowerShell 7.4 or later and in Windows PowerShell 5.1, and installs from the PowerShell Gallery:

```powershell
Install-Module Wgpu -Scope CurrentUser
```

The examples are on the task pages each group page names.

## How the cmdlets read and write

| Concern | Behavior |
|---|---|
| Devices | A cmdlet that does work on the GPU takes `-Device`; without it, it uses the default device, which the first such cmdlet opens on the adapter wgpu prefers. Everything a cmdlet makes belongs to the device it was made on. |
| Numbers | An array of any numeric type goes to the GPU as its own element type, or as the one `-ElementType` names; numbers with no type of their own are Float32. Numbers piped in become one array. |
| Buffers and textures | A `Wgpu.Buffer` or a `Wgpu.Texture` keeps its data on the GPU between cmdlets; nothing is copied back until a cmdlet or a method reads it. |
| Output | An array comes back as one object on the pipeline; `Read-WgpuBuffer -Enumerate` writes its elements one by one. |
| Files | A file a cmdlet reads is named by `-Path`, whose wildcards name every file they match, or by `-LiteralPath`, which names one file as it is written; `Import-WgpuImage` and the shader cmdlets bind the `PSPath` of what `Get-ChildItem` pipes, or the `FullName` of a .NET `FileInfo`, to `-LiteralPath`. A file a cmdlet writes, by `-OutFile` or `Export-WgpuImage`'s `-Path`, is named as it is written. A relative path resolves against PowerShell's location. |
| Changes | `Clear-WgpuBuffer`, `Remove-WgpuBuffer` and `Close-WgpuDevice` take `-WhatIf` and `-Confirm`. |
| Errors | A cmdlet writes a non-terminating error record whose `FullyQualifiedErrorId` begins with the module's id and goes on with its next input; a method throws. [Errors](Errors.md) shows both, and the [Error Reference](Error-Reference.md) lists every id. |
| Waiting | A cmdlet returns once the GPU has done its work, and Ctrl+C stops the wait; the methods' `Async` forms return a task at once, as [Tasks and threads](Tasks.md) shows. |

{{< cards >}}
  {{< card link="device-cmdlets/" title="Adapters and devices" subtitle="Get-WgpuAdapter, Open-WgpuDevice, Get-WgpuDevice, Wait-WgpuDevice and Close-WgpuDevice." >}}
  {{< card link="buffer-cmdlets/" title="Buffers" subtitle="New-WgpuBuffer, Write-WgpuBuffer, Read-WgpuBuffer, Copy-WgpuBuffer, Clear-WgpuBuffer and Remove-WgpuBuffer." >}}
  {{< card link="compute-cmdlets/" title="Compute" subtitle="Invoke-WgpuCompute, Invoke-WgpuMap, Measure-WgpuArray, New-WgpuComputePipeline and New-WgpuBindGroup." >}}
  {{< card link="shader-cmdlets/" title="Shaders" subtitle="New-WgpuShader, Test-WgpuShader, Get-WgpuShaderInfo and ConvertTo-WgpuShader." >}}
  {{< card link="texture-cmdlets/" title="Textures and images" subtitle="New-WgpuTexture, Write-WgpuTexture, Read-WgpuTexture, Import-WgpuImage, Export-WgpuImage, Resize-WgpuTexture, Convert-WgpuTexture, ConvertTo-WgpuNormalMap, ConvertTo-WgpuCubeMap, ConvertFrom-WgpuCubeMap, ConvertTo-WgpuIrradianceMap, ConvertTo-WgpuSpecularMap, New-WgpuBrdfTable, Join-WgpuTexture, Compare-WgpuTexture, Measure-WgpuTexture, Get-WgpuTextureFormat and New-WgpuSampler." >}}
  {{< card link="render-cmdlets/" title="Rendering" subtitle="Invoke-WgpuRender and New-WgpuRenderPipeline." >}}
  {{< card link="command-cmdlets/" title="Recording commands" subtitle="New-WgpuCommandEncoder, Submit-WgpuCommand and New-WgpuQuerySet." >}}
  {{< card link="types/" title="Types" subtitle="Every class and enumeration the cmdlets write and take, with their properties and methods." >}}
{{< /cards >}}

## Every cmdlet

| Cmdlet | Does |
|---|---|
| [Get-WgpuAdapter](Device-Cmdlets.md#get-wgpuadapter) | Lists the GPU adapters wgpu finds, with every feature and limit each supports. |
| [Open-WgpuDevice](Device-Cmdlets.md#open-wgpudevice) | Opens a GPU device with every feature and limit its adapter supports. |
| [Get-WgpuDevice](Device-Cmdlets.md#get-wgpudevice) | Gets the device cmdlets use when given no -Device, opening one on the adapter wgpu prefers if none is open. |
| [Wait-WgpuDevice](Device-Cmdlets.md#wait-wgpudevice) | Waits until a device has finished all the work submitted to it. |
| [Close-WgpuDevice](Device-Cmdlets.md#close-wgpudevice) | Destroys a device: every object made on it stops working. |
| [New-WgpuBuffer](Buffer-Cmdlets.md#new-wgpubuffer) | Creates a GPU buffer from an array of any numeric type, or of a length. |
| [Write-WgpuBuffer](Buffer-Cmdlets.md#write-wgpubuffer) | Overwrites elements of a buffer with new values. |
| [Read-WgpuBuffer](Buffer-Cmdlets.md#read-wgpubuffer) | Copies a buffer back from the GPU and writes its elements as one array. |
| [Copy-WgpuBuffer](Buffer-Cmdlets.md#copy-wgpubuffer) | Copies elements from one buffer to another on the GPU. |
| [Clear-WgpuBuffer](Buffer-Cmdlets.md#clear-wgpubuffer) | Sets every element of a buffer to zero, or to -Value. |
| [Remove-WgpuBuffer](Buffer-Cmdlets.md#remove-wgpubuffer) | Frees a buffer's GPU memory now, rather than when the object is collected. |
| [Invoke-WgpuCompute](Compute-Cmdlets.md#invoke-wgpucompute) | Runs a compute shader once over arrays or buffers and writes the arrays it may have written. |
| [Invoke-WgpuMap](Compute-Cmdlets.md#invoke-wgpumap) | Runs a WGSL expression over arrays on the GPU, one invocation per element, and writes the results as one array. |
| [Measure-WgpuArray](Compute-Cmdlets.md#measure-wgpuarray) | Sums, averages, and finds the minimum and maximum of an array or a buffer on the GPU, like Measure-Object over numbers. |
| [New-WgpuComputePipeline](Compute-Cmdlets.md#new-wgpucomputepipeline) | Builds a compute pipeline from a shader for one device. |
| [New-WgpuBindGroup](Compute-Cmdlets.md#new-wgpubindgroup) | Binds resources to one group of a compute pipeline's bindings. |
| [New-WgpuShader](Shader-Cmdlets.md#new-wgpushader) | Parses and validates a shader, independent of any device. |
| [Test-WgpuShader](Shader-Cmdlets.md#test-wgpushader) | Validates a shader and writes whether it is valid, or with -Diagnostics every problem found, with its line and column. |
| [Get-WgpuShaderInfo](Shader-Cmdlets.md#get-wgpushaderinfo) | Writes what a shader declares: its entry points with their stages and workgroup sizes, its resource bindings with their kinds and types, and its override constants. |
| [ConvertTo-WgpuShader](Shader-Cmdlets.md#convertto-wgpushader) | Translates a WGSL, GLSL or SPIR-V shader into WGSL, SPIR-V, GLSL, HLSL, MSL or a Graphviz graph of its intermediate form, with no GPU. |
| [New-WgpuTexture](Texture-Cmdlets.md#new-wgputexture) | Creates a texture: 1D, 2D (with array layers) or 3D, of any format. |
| [Write-WgpuTexture](Texture-Cmdlets.md#write-wgputexture) | Writes texels into a texture, rows packed, converted to the format's number type. |
| [Read-WgpuTexture](Texture-Cmdlets.md#read-wgputexture) | Copies a texture's texels back from the GPU and writes them as one array, rows packed, as the format's number type. |
| [Import-WgpuImage](Texture-Cmdlets.md#import-wgpuimage) | Loads an image file into a texture: PNG, JPEG, GIF, WebP, AVIF, JPEG XL, TIFF, Photoshop's PSD and PSB, BMP, TGA, EXR, HDR, QOI, ICO, PNM or Farbfeld, and a DDS or KTX2 file whole, with every mip level, array layer and cube face; and draws an SVG file or a PDF's first page. |
| [Export-WgpuImage](Texture-Cmdlets.md#export-wgpuimage) | Saves a texture: as a DDS or KTX2 file whole, with every mip level, array layer and cube face, or as any other image file, from mip level 0 of one layer, in the format its extension names (or -ImageFormat), converted to what that format stores, a two-channel texture as red and green with blue 0 and alpha 1. An .exr file is compressed with ZIP, in half floats for a texture of 16-bit float texels and in full floats for any other. |
| [Resize-WgpuTexture](Texture-Cmdlets.md#resize-wgputexture) | Changes a texture's size into a new texture: resizes it to a width and height, a scale, a size limit or powers of two, with a filter; crops it to a region, a size or margins; and pads it onto a larger canvas. It crops, then resizes, then pads, whatever order the parameters come in. |
| [Convert-WgpuTexture](Texture-Cmdlets.md#convert-wgputexture) | Flips or rotates a texture, changes its channels and colors, or filters it, into a new texture: -Flip mirrors it and -Rotate turns it 90, 180 or 270 degrees clockwise, turning first; then -AssignColorSpace, -ColorKey, -Swizzle, -ReconstructZ, -Normalize, -FlipGreen, -Unpremultiply, -AlphaThreshold, -Exposure, -ToneMap, -Brightness and -Contrast, -Gamma, -Grayscale, -Invert, -Premultiply and -ColorSpace change each texel, in that order, whatever order they are given in; then -Blur blurs the texels, -Sharpen sharpens them and -Edges finds their edges, in that order; and last -FixAlphaBorder colors the texels with alpha 0 from the nearest texels with alpha above 0. The filters work on level 0 of each layer, and of each depth slice of a 3D texture, on its own. |
| [ConvertTo-WgpuNormalMap](Texture-Cmdlets.md#convertto-wgpunormalmap) | Makes a normal map from a height map: each texel's normal from the slopes of the heights around it, by a kernel, at a strength. |
| [ConvertTo-WgpuCubeMap](Texture-Cmdlets.md#convertto-wgpucubemap) | Makes a cube map: from six textures, its faces, or from one image that holds every face, in a layout of square cells or a projection of every direction. |
| [ConvertFrom-WgpuCubeMap](Texture-Cmdlets.md#convertfrom-wgpucubemap) | Writes a cube map into one image: its faces in a layout of square cells, or a projection of every direction. |
| [ConvertTo-WgpuIrradianceMap](Texture-Cmdlets.md#convertto-wgpuirradiancemap) | Makes the irradiance cube of an environment: each texel the light a surface facing its direction receives from every direction, weighed by the cosine of the angle between them and divided by pi, the color a white diffuse surface reflects, which a shader multiplies by the surface's albedo. |
| [ConvertTo-WgpuSpecularMap](Texture-Cmdlets.md#convertto-wgpuspecularmap) | Makes the prefiltered specular cube of an environment, which split-sum image-based lighting reads beside the BRDF table: each mip level the environment as the GGX lobe of one roughness reflects it, the view taken along each texel's direction. |
| [New-WgpuBrdfTable](Texture-Cmdlets.md#new-wgpubrdftable) | Makes the BRDF table split-sum image-based lighting reads beside the specular map: for the cosine of the view's angle across and perceptual roughness down, the part of the specular light F0 scales, in red, and the part it adds, in green, with Smith's height-correlated GGX visibility, as Khronos's glTF IBL sampler and Filament compute them. |
| [Join-WgpuTexture](Texture-Cmdlets.md#join-wgputexture) | Packs channels of several textures, or numbers, into one new texture: -Red, -Green, -Blue and -Alpha each take a texture, whose red channel they read; a texture and a channel letter as a pair, as ($mask, 'a'); or a number. An omitted color channel is 0, and an omitted alpha 1. |
| [Compare-WgpuTexture](Texture-Cmdlets.md#compare-wgputexture) | Compares a texture with a reference, such as a block-compressed copy with its original: for each mip level both have, each layer and each channel both store, a row with the mean squared error (MSE), the peak signal-to-noise ratio (PSNR) and the structural similarity (SSIM) of the texture's values against the reference's. With -Difference it writes a picture of the error instead. |
| [Measure-WgpuTexture](Texture-Cmdlets.md#measure-wgputexture) | Measures a texture's values: for each mip level, layer and channel, a row with the minimum, maximum, mean, population standard deviation and a histogram of the values. |
| [Get-WgpuTextureFormat](Texture-Cmdlets.md#get-wgputextureformat) | Lists every texture format wgpu has, with its block size, channels and number type, the features it needs, and what the adapter can do with it. |
| [New-WgpuSampler](Texture-Cmdlets.md#new-wgpusampler) | Creates a sampler: how a shader filters and wraps when it samples a texture. |
| [Invoke-WgpuRender](Render-Cmdlets.md#invoke-wgpurender) | Draws offscreen with a vertex and a fragment shader into a texture, or into an image file. |
| [New-WgpuRenderPipeline](Render-Cmdlets.md#new-wgpurenderpipeline) | Builds a render pipeline from a shader with a vertex and a fragment entry point, for one device and one target format. |
| [New-WgpuCommandEncoder](Command-Cmdlets.md#new-wgpucommandencoder) | Opens a command encoder to record passes, copies and queries into. |
| [Submit-WgpuCommand](Command-Cmdlets.md#submit-wgpucommand) | Submits finished command buffers to a device's queue and waits for the GPU to run them. |
| [New-WgpuQuerySet](Command-Cmdlets.md#new-wgpuqueryset) | Creates a set of queries the GPU writes results into: timestamps, occlusion counts or pipeline statistics. |
