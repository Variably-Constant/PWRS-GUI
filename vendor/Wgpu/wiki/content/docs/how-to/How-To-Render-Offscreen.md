---
title: How To Render Offscreen
weight: 12
---

Draw charts of a year's rainfall into image files for a report, with no window and no screen: a bar chart drawn as one bar for each instance of a square, and a pie chart of the seasons drawn as fans of triangles, its round edge smoothed by multisampling. [Rendering](Rendering.md) has each part on its own.

## The data

Twelve months of rainfall, in millimeters:

```powershell
PS> $rain = [float[]](78, 62, 55, 43, 38, 29, 21, 25, 37, 58, 71, 84)
PS> $rain | Measure-Object -Sum -Maximum | Select-Object Sum, Maximum
   Sum Maximum
   --- -------
601.00   84.00
```

## A bar for each month

A draw runs the vertex shader once for each vertex, which places it, and the fragment shader for each pixel a triangle covers, which colors it. This vertex shader draws a square, its four corners from 0 to 1, once for each month: `instance_index` counts the instances, and picks the month's rainfall from a storage buffer to set the bar's height and color. A uniform gives the number of bars and the rainfall at the top of the chart:

```powershell
PS> Get-Content ./bars.wgsl
struct Chart { count: f32, top: f32 }
@group(0) @binding(0) var<storage, read> values: array<f32>;
@group(0) @binding(1) var<uniform> chart: Chart;
struct Out { @builtin(position) position: vec4<f32>, @location(0) color: vec3<f32> }
@vertex
fn vs(@location(0) corner: vec2<f32>, @builtin(instance_index) month: u32) -> Out {
    let width = 1.8 / chart.count;
    let share = values[month] / chart.top;
    var out: Out;
    out.position = vec4<f32>(-0.9 + width * (f32(month) + 0.15 + 0.7 * corner.x), -0.9 + 1.8 * share * corner.y, 0.0, 1.0);
    out.color = mix(vec3<f32>(0.6, 0.78, 0.95), vec3<f32>(0.1, 0.3, 0.65), share);
    return out;
}
@fragment
fn fs(input: Out) -> @location(0) vec4<f32> { return vec4<f32>(input.color, 1.0); }
```

## Draw it into a file

The square's four corners go in as a vertex input, and six indices make it of two triangles. `-InstanceCount` draws it once for each month, `-Binding` gives the shaders their buffers as `Invoke-WgpuCompute` does, the uniform from a hashtable of its members, and `-OutFile` saves the drawing:

```powershell
PS> $square = @{ corner = [float[]](0, 0, 1, 0, 0, 1, 1, 1) }
PS> $twoTriangles = [uint32[]](0, 1, 2, 2, 1, 3)
PS> $chart = @{ values = $rain; chart = @{ count = $rain.Length; top = 100 } }
PS> Invoke-WgpuRender (Get-Content ./bars.wgsl -Raw) 480 240 -Vertex $square -Index $twoTriangles -InstanceCount $rain.Length -Binding $chart -ClearColor 1, 1, 1 -OutFile ./rain.png
```

![Twelve bars of a year's rainfall on white, highest and darkest blue in December and January, lowest and palest in July](/images/rain.png)

Positions are in clip space, x and y from -1 to 1 with y up, so the bars start at y = -0.9 and the target's top row is y = 1.

## A pie of the seasons

The seasons' shares go in as where each slice starts, from 0 to 1, with 1 at the end. Winter is December to February:

```powershell
PS> $seasons = @(($rain[11] + $rain[0] + $rain[1]), ($rain[2..4] | Measure-Object -Sum).Sum, ($rain[5..7] | Measure-Object -Sum).Sum, ($rain[8..10] | Measure-Object -Sum).Sum)
PS> $starts = [float[]]::new(5)
PS> for ($s = 0; $s -lt 4; $s++) { $starts[$s + 1] = $starts[$s] + $seasons[$s] / ($seasons | Measure-Object -Sum).Sum }
PS> ($starts | ForEach-Object { [math]::Round($_, 3) }) -join ', '
0, 0.373, 0.599, 0.724, 1
```

This shader needs no vertex input. Each instance is a slice, a fan of 32 triangles from the center to its arc, and `vertex_index` says which corner of which triangle a vertex is:

```powershell
PS> Get-Content ./pie.wgsl
@group(0) @binding(0) var<storage, read> starts: array<f32>;
struct Out { @builtin(position) position: vec4<f32>, @location(0) color: vec3<f32> }
@vertex
fn vs(@builtin(vertex_index) v: u32, @builtin(instance_index) slice: u32) -> Out {
    let colors = array<vec3<f32>, 4>(vec3<f32>(0.2, 0.4, 0.75), vec3<f32>(0.3, 0.65, 0.35), vec3<f32>(0.95, 0.7, 0.2), vec3<f32>(0.8, 0.35, 0.2));
    let step = v / 3u + select(0u, 1u, v % 3u == 2u);
    let turn = mix(starts[slice], starts[slice + 1u], f32(step) / 32.0) * 6.2831853;
    var out: Out;
    out.position = select(vec4<f32>(0.9 * sin(turn), 0.9 * cos(turn), 0.0, 1.0), vec4<f32>(0.0, 0.0, 0.0, 1.0), v % 3u == 0u);
    out.color = colors[slice];
    return out;
}
@fragment
fn fs(input: Out) -> @location(0) vec4<f32> { return vec4<f32>(input.color, 1.0); }
```

With one sample a pixel, a pixel on the rim is either in a slice or out of it, and the rim steps. `-SampleCount 4` takes four samples a pixel and blends them, so a rim pixel takes the share of it the slice covers:

```powershell
PS> $pie = Get-Content ./pie.wgsl -Raw
PS> Invoke-WgpuRender $pie 160 160 -VertexCount 96 -InstanceCount 4 -Binding @{ starts = $starts } -ClearColor 1, 1, 1 -OutFile ./seasons.png
PS> Invoke-WgpuRender $pie 160 160 -VertexCount 96 -InstanceCount 4 -Binding @{ starts = $starts } -ClearColor 1, 1, 1 -SampleCount 4 -OutFile ./seasons-smooth.png
```

![The pie of the seasons with one sample a pixel: winter blue, spring green, summer yellow and autumn red, clockwise from the top, its rim stepped](/images/seasons.png) ![The same pie with four samples a pixel, its rim smooth](/images/seasons-smooth.png)

## One pipeline for many charts

`Invoke-WgpuRender` given shader text builds a render pipeline for the draw. `New-WgpuRenderPipeline` builds it once, and each draw through it reuses it, which counts when a report holds many charts:

```powershell
PS> $barPipeline = New-WgpuRenderPipeline (Get-Content ./bars.wgsl -Raw)
PS> $years = [ordered]@{ 2024 = [float[]](65, 70, 49, 40, 35, 31, 18, 22, 40, 61, 66, 90); 2025 = $rain }
PS> foreach ($year in $years.GetEnumerator()) {
>>     $binding = @{ values = $year.Value; chart = @{ count = 12; top = 100 } }
>>     Invoke-WgpuRender $barPipeline 480 240 -Vertex $square -Index $twoTriangles -InstanceCount 12 -Binding $binding -ClearColor 1, 1, 1 -OutFile "./rain-$($year.Key).png"
>> }
PS> Get-ChildItem ./rain-*.png | Select-Object -ExpandProperty Name
rain-2024.png
rain-2025.png
```

A drawing of several layers, such as gridlines under the bars, is one render pass of several draws: [How To Draw In A Render Pass Of Your Own](How-To-Draw-In-A-Render-Pass-Of-Your-Own.md). Depth and stencil tests, blending and culling are on [Rendering](Rendering.md).
