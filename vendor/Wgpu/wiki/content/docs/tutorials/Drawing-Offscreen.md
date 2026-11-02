---
title: Drawing Offscreen
weight: 6
---

A draw runs two shaders: a vertex shader once for each corner of a shape, which says where the corner goes, and a fragment shader once for each pixel the shape covers, which says what color the pixel is. Both are entry points of one WGSL shader. This chapter draws a triangle into an image file, feeds the shaders corners from arrays, lets depth decide which shape is in front, and blends one color over another. Every draw here is offscreen: it lands in a texture or a file, never on a screen.

## A triangle

This shader places three corners by their index and gives each a color, which the GPU blends across the triangle for the fragment shader:

```powershell
PS> Get-Content ./triangle.wgsl
struct Out { @builtin(position) position: vec4<f32>, @location(0) color: vec3<f32> }
@vertex
fn vs(@builtin(vertex_index) i: u32) -> Out {
    var corners = array<vec2<f32>, 3>(vec2<f32>(0.0, 0.8), vec2<f32>(-0.8, -0.8), vec2<f32>(0.8, -0.8));
    var colors = array<vec3<f32>, 3>(vec3<f32>(1.0, 0.0, 0.0), vec3<f32>(0.0, 1.0, 0.0), vec3<f32>(0.0, 0.0, 1.0));
    var out: Out;
    out.position = vec4<f32>(corners[i], 0.0, 1.0);
    out.color = colors[i];
    return out;
}
@fragment
fn fs(input: Out) -> @location(0) vec4<f32> { return vec4<f32>(input.color, 1.0); }
```

The corners are in normalized device coordinates: x and y run from -1 to 1 across the target whatever its size, with y up. `Invoke-WgpuRender` draws three vertices into a 256 by 256 target and saves it as a PNG:

```powershell
PS> Invoke-WgpuRender (Get-Content ./triangle.wgsl -Raw) 256 256 -VertexCount 3 -OutFile ./triangle.png
PS> Get-Item ./triangle.png | Select-Object Name, Length
Name         Length
----         ------
triangle.png  26561
```

![The triangle Invoke-WgpuRender drew: red at the top, green at the bottom left, blue at the bottom right, blended between them, on black](/images/triangle.png)

## Corners from arrays

A vertex shader can read its corners from arrays instead: each `@location` input reads an array of its own, given in `-Vertex` by the input's name, with as many numbers for each corner as the input has. This one reads a position and a color for each corner, and here covers the whole 2 by 2 target in green:

```powershell
PS> Get-Content ./colored.wgsl
struct Out { @builtin(position) position: vec4<f32>, @location(0) color: vec3<f32> }
@vertex
fn vs(@location(0) position: vec3<f32>, @location(1) color: vec3<f32>) -> Out {
    var out: Out;
    out.position = vec4<f32>(position, 1.0);
    out.color = color;
    return out;
}
@fragment
fn fs(input: Out) -> @location(0) vec4<f32> { return vec4<f32>(input.color, 1.0); }
PS> $colored = Get-Content ./colored.wgsl -Raw
PS> $green = @{ position = [float[]](-1, -1, 0, 3, -1, 0, -1, 3, 0); color = [float[]](0, 1, 0, 0, 1, 0, 0, 1, 0) }
PS> $texture = Invoke-WgpuRender $colored 2 2 -Vertex $green
PS> (Read-WgpuTexture $texture)[0..3] -join ','
0,255,0,255
```

With no `-OutFile`, the drawing comes back as a texture, which reads like any other. The number of vertices came from the arrays.

## Nearer wins

Two triangles over the whole target: a red one at depth 0.2, drawn first, and a blue one at 0.8, drawn after. Without a depth test, the last one drawn covers the first:

```powershell
PS> $pair = @{
>>     position = [float[]](-1, -1, 0.2, 3, -1, 0.2, -1, 3, 0.2, -1, -1, 0.8, 3, -1, 0.8, -1, 3, 0.8)
>>     color = [float[]](1, 0, 0, 1, 0, 0, 1, 0, 0, 0, 0, 1, 0, 0, 1, 0, 0, 1)
>> }
PS> (Read-WgpuTexture (Invoke-WgpuRender $colored 4 4 -Vertex $pair))[0..3] -join ','
0,0,255,255
```

`-Depth` gives the draw a depth target, and a fragment is kept only when it is nearer than what is already there, so the red triangle stays in front:

```powershell
PS> (Read-WgpuTexture (Invoke-WgpuRender $colored 4 4 -Vertex $pair -Depth))[0..3] -join ','
255,0,0,255
```

## See through

`-Blend Alpha` mixes a fragment over what the target holds by the fragment's alpha. Half-transparent blue over a white target comes out light blue:

```powershell
PS> Get-Content ./glass.wgsl
@vertex
fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    var corners = array<vec2<f32>, 3>(vec2<f32>(-1.0, -1.0), vec2<f32>(3.0, -1.0), vec2<f32>(-1.0, 3.0));
    return vec4<f32>(corners[i], 0.0, 1.0);
}
@fragment
fn fs() -> @location(0) vec4<f32> { return vec4<f32>(0.0, 0.0, 1.0, 0.5); }
PS> $glass = Get-Content ./glass.wgsl -Raw
PS> (Read-WgpuTexture (Invoke-WgpuRender $glass 2 2 -VertexCount 3 -ClearColor 1, 1, 1, 1 -Blend Alpha))[0..3] -join ','
128,128,255,255
```

`-ClearColor` filled the target with white before the draw; half of each channel came from the blue and half from the white.

## Where next

[Commands And Tasks](Commands-And-Tasks.md) records several steps into one submission, and runs work while PowerShell goes on with other things.
