---
title: How To Light A Scene With An Environment
weight: 13
---

Light a scene the way glTF viewers and game engines light one from an HDR panorama: make the panorama into the three maps split-sum image-based lighting reads, and draw a row of spheres from smooth to rough with a shader that reads them. [Make cube maps and lighting maps](Images.md#make-cube-maps-and-lighting-maps) has each map on its own, and [How To Render Offscreen](How-To-Render-Offscreen.md) has drawing into image files.

## Get an environment

An environment is an HDR picture of the light that arrives from every direction, most often a panorama in a Radiance .hdr or OpenEXR file. [Poly Haven](https://polyhaven.com/hdris) has nearly a thousand photographed skies, interiors and studios under CC0, free for any use, at sizes from 1k up to their full size, 16k or more for nearly all of them. This page lights its spheres with two panoramas the wiki draws: [sky.hdr](/images/sky.hdr), a clear sky with the sun 30 degrees up over green ground, and [studio.exr](/images/studio.exr), a dark studio with two softboxes beside the camera and a cyan strip light behind the subject. Tone mapped for a screen, they look like this:

![A clear sky over green ground, with the sun on the right](/images/sky.png)

![A dark studio: two white softboxes and a thin cyan strip light against a gray backdrop](/images/studio.png)

The sun is over a thousand times brighter than the sky around it, which an HDR file keeps and an 8-bit picture cannot:

```powershell
PS> $sky = Import-WgpuImage ./sky.hdr
PS> $sky | Measure-WgpuTexture | Where-Object Channel -EQ Red | Select-Object Minimum, Maximum, Average
Minimum Maximum Average
------- ------- -------
   0.08 1800.00    0.42
```

## Make the three maps

The irradiance map is the diffuse light a surface facing each direction receives, the specular map the environment reflected by a surface of each roughness, one roughness to a mip level, and the BRDF table how much of that reflection reaches the eye at each angle. The two cube maps come from the panorama, converted to a cube of 128-texel faces first; the table depends on no environment, so one table serves every scene:

{{< tabs >}}
{{< tab name="Cmdlets" >}}
```powershell
PS> $irradiance = $sky | ConvertTo-WgpuIrradianceMap
PS> $specular = $sky | ConvertTo-WgpuSpecularMap
PS> $brdf = New-WgpuBrdfTable
```
{{< /tab >}}
{{< tab name="Methods" >}}
```powershell
PS> $irradiance = $sky.ToIrradianceMap($null, $false, $null)
PS> $specular = $sky.ToSpecularMap($null, $null, $null, $null)
PS> $brdf = [Wgpu.Texture]::NewBrdfTable($null, $null, $null, $null)
```
{{< /tab >}}
{{< /tabs >}}

```powershell
PS> $irradiance, $specular, $brdf | Select-Object Width, DepthOrArrayLayers, MipLevelCount, Format
Width DepthOrArrayLayers MipLevelCount Format
----- ------------------ ------------- ------
   32                  6             1 Rgba16Float
  128                  6             7 Rgba16Float
  128                  1             1 Rg16Float
```

## Write the shader

One triangle covers the picture, and each pixel finds the sphere in its cell of 128 pixels and the direction the sphere's surface faces there, seen from straight ahead. Roughness runs from 0 to 1 across, with red plastic above and gold below. The light is the split sum: the irradiance map in the surface's direction times its albedo, for diffuse light, which a metal has none of; and the specular map at the level of the surface's roughness, in the direction a ray from the eye reflects, times F0 scaled and offset by the BRDF table at the cosine of the view's angle and the roughness. The map's levels step through roughness linearly, as `ConvertTo-WgpuSpecularMap` writes them by default, so the level is the roughness times the last level. Pixels between the spheres show the environment as a camera looking toward -Z sees it, read from the specular map's level 0:

```powershell
PS> Get-Content ./spheres.wgsl
@group(0) @binding(0) var irradiance: texture_cube<f32>;
@group(0) @binding(1) var specular: texture_cube<f32>;
@group(0) @binding(2) var brdf: texture_2d<f32>;
@group(0) @binding(3) var smp: sampler;
@vertex
fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    var corners = array<vec2<f32>, 3>(vec2<f32>(-1.0, -1.0), vec2<f32>(3.0, -1.0), vec2<f32>(-1.0, 3.0));
    return vec4<f32>(corners[i], 0.0, 1.0);
}
@fragment
fn fs(@builtin(position) p: vec4<f32>) -> @location(0) vec4<f32> {
    // The environment, as a camera looking toward -Z sees it.
    let screen = (p.xy / vec2<f32>(640.0, 256.0) - 0.5) * vec2<f32>(2.5, -1.0);
    let background = textureSampleLevel(specular, smp, normalize(vec3<f32>(screen, -1.0)), 0.0).rgb;
    // A sphere in each cell of 128 pixels, seen from straight ahead.
    let cell = floor(p.xy / 128.0);
    let local = (p.xy - (cell + 0.5) * 128.0) / 54.0;
    let n = vec3<f32>(local.x, -local.y, sqrt(max(1.0 - dot(local, local), 0.0))) / max(length(local), 1.0);
    // Roughness 0 to 1 across; red plastic above, gold below.
    let roughness = cell.x / 4.0;
    let metal = cell.y == 1.0;
    let albedo = select(vec3<f32>(0.7, 0.08, 0.06), vec3<f32>(1.0, 0.78, 0.34), metal);
    let f0 = select(vec3<f32>(0.04), albedo, metal);
    // The split sum: the specular map's level for this roughness, times F0
    // scaled and offset by the BRDF table at (NdotV, roughness).
    let ab = textureSampleLevel(brdf, smp, vec2<f32>(max(n.z, 0.001), roughness), 0.0).rg;
    let levels = f32(textureNumLevels(specular));
    let prefiltered = textureSampleLevel(specular, smp, reflect(vec3<f32>(0.0, 0.0, -1.0), n), roughness * (levels - 1.0)).rgb;
    let fresnel = f0 * ab.x + ab.y;
    let diffuse = select(textureSampleLevel(irradiance, smp, n, 0.0).rgb * albedo * (1.0 - fresnel), vec3<f32>(0.0), metal);
    let coverage = clamp((1.0 - length(local)) * 54.0 + 0.5, 0.0, 1.0);
    return vec4<f32>(mix(background, diffuse + prefiltered * fresnel, coverage), 1.0);
}
PS> $spheres = Get-Content ./spheres.wgsl -Raw
```

A map whose levels step through roughness as cmgen writes them, `-RoughnessMapping Quadratic`, is read at (levels - 1) times (1 - (1 - roughness)^2) instead.

## Draw the spheres

The maps bind whole, the cube maps to the `texture_cube` bindings and the table to the `texture_2d` one, and one sampler blends between texels and between mip levels. The lighting is HDR, so the picture renders in Rgba16Float, and AgX tone maps it into 8-bit sRGB for the file:

```powershell
PS> $lighting = @{ irradiance = $irradiance; specular = $specular; brdf = $brdf; smp = (New-WgpuSampler -Filter Linear) }
PS> Invoke-WgpuRender $spheres 640 256 -VertexCount 3 -Binding $lighting -Format Rgba16Float | Convert-WgpuTexture -ToneMap AgX -ColorSpace Srgb -Format Rgba8Unorm | Export-WgpuImage -Path ./spheres-sky.png
```

![Ten spheres in front of the sky: red plastic above and gold below, smooth on the left and rough on the right; the smooth ones mirror the horizon and a small bright sun, and the rough ones show a soft glow from the sun's side](/images/spheres-sky.png)

The smooth spheres mirror the horizon and a small spot of sun, and the reflection blurs into the sky's colors as roughness grows. The plastic keeps its red, its reflection a few percent of the light seen head on and more toward its edge, where the BRDF table's offset grows; the gold reflects everything in its own color.

The roughest gold sphere shows the shape of the specular map's last level, 2 texels a face, which is the level the default chain ends at, as Khronos's sampler ends it. `-MipLevels 5` ends the chain at 8 texels a face, so roughness 1 reads a level fine enough to blend smoothly; the shader finds the number of levels itself:

```powershell
PS> $lighting.specular = $sky | ConvertTo-WgpuSpecularMap -MipLevels 5
PS> Invoke-WgpuRender $spheres 640 256 -VertexCount 3 -Binding $lighting -Format Rgba16Float | Convert-WgpuTexture -ToneMap AgX -ColorSpace Srgb -Format Rgba8Unorm | Export-WgpuImage -Path ./spheres-sky-8.png
```

![The same spheres with the specular map ending at 8 texels a face: the roughest gold sphere blends smoothly from light to dark](/images/spheres-sky-8.png)

## Light them in a studio

The studio from its OpenEXR file gives the same spheres sharp reflections of the two softboxes, and a cyan line along their right edges from the strip behind them, which the camera does not see:

```powershell
PS> $studio = Import-WgpuImage ./studio.exr
PS> $lighting.irradiance = $studio | ConvertTo-WgpuIrradianceMap
PS> $lighting.specular = $studio | ConvertTo-WgpuSpecularMap
PS> Invoke-WgpuRender $spheres 640 256 -VertexCount 3 -Binding $lighting -Format Rgba16Float | Convert-WgpuTexture -ToneMap AgX -ColorSpace Srgb -Format Rgba8Unorm | Export-WgpuImage -Path ./spheres-studio.png
```

![Ten spheres in the studio: the smooth ones mirror two bright softboxes and a thin cyan line on their right edges, and the rough ones glow softly from the upper left](/images/spheres-studio.png)

## Save the maps for a renderer

A renderer loads the maps from files. A KTX2 or DDS file holds a cube map whole, every face and mip level, and glTF viewers read KTX2. The BRDF table saves the same way, or as an image in red and green, as many renderers ship it: an .exr file keeps its half floats, and a PNG 16 bits a channel:

```powershell
PS> Export-WgpuImage $irradiance ./sky-irradiance.ktx2
PS> Export-WgpuImage $specular ./sky-specular.ktx2
PS> Export-WgpuImage $brdf ./brdf.ktx2
PS> Export-WgpuImage $brdf ./brdf.png
PS> Get-ChildItem ./sky-irradiance.ktx2, ./sky-specular.ktx2, ./brdf.ktx2, ./brdf.png | Select-Object Name, Length
Name                 Length
----                 ------
sky-irradiance.ktx2   49376
sky-specular.ktx2   1048880
brdf.ktx2             65728
brdf.png              53990

PS> Import-WgpuImage ./sky-specular.ktx2 | Select-Object Width, DepthOrArrayLayers, MipLevelCount, IsCube, Format
Width              : 128
DepthOrArrayLayers : 6
MipLevelCount      : 7
IsCube             : True
Format             : Rgba16Float
```

`ConvertTo-WgpuIrradianceMap -Harmonics` gives the map nine numbers a color channel, which a shader can sum in place of reading the irradiance cube, as [Make cube maps and lighting maps](Images.md#make-cube-maps-and-lighting-maps) shows.
