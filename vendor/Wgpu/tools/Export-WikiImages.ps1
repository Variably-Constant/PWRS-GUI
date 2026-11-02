# Draws the images the wiki shows, with the shaders and the commands the
# pages that show them run, into wiki/static/images:
#
#   triangle.png   Drawing Offscreen
#   squares.png    Rendering
#   scene.png, scene.hdr, sprite.png, badge.png, occlusion.png,
#   roughness.png, metalness.png, height.png
#                  the pictures the image how-tos load
#   edges.png, card.png
#                  How To Use An Existing Image
#   thumb.png, adjusted.png, gray.png, negative.png, soft.png, sharp.png,
#   outline.png, badge-cut.png, scene-agx.png, scene-darker.png
#                  How To Edit And Adjust Images
#   scene-bc7-error.png, wall-normal.png
#                  How To Prepare Textures For A Renderer
#   packed.png     How To Pack Channels Into One Texture
#   two-views.png  How To Draw In A Render Pass Of Your Own
#   tiles.png, tiles-cool.png, floor-level0.png, floor-mipmapped.png,
#   floor-anisotropic.png
#                  How To Work With Textures And Images
#   rain.png, seasons.png, seasons-smooth.png
#                  How To Render Offscreen
#   life.png       How To Record Commands By Hand
#   sky.hdr, sky.exr, sky.png, studio.hdr, studio.exr, studio.png
#                  the panoramas the cube map pages load
#   sky-cross.png, sky-px.png, sky-nx.png, sky-py.png, sky-ny.png,
#   sky-pz.png, sky-nz.png, turned.jpg, landscape-p3.jpg
#                  Textures and images
#   spheres-sky.png, spheres-studio.png, spheres-sky-8.png
#                  How To Light A Scene With An Environment
#
#   pwsh -NoProfile -File tools/Export-WikiImages.ps1 -Module target/pwrs/Wgpu
[CmdletBinding()]
param(
    # The built module: its folder, or the Wgpu.psd1 inside it.
    [Parameter(Mandatory)] [string] $Module
)

$ErrorActionPreference = 'Stop'
$item = Get-Item -LiteralPath $Module
$manifest = if ($item.PSIsContainer) { Join-Path $item.FullName 'Wgpu.psd1' } else { $item.FullName }
Import-Module $manifest
$images = [System.IO.Path]::GetFullPath((Join-Path $PSScriptRoot '../wiki/static/images'))
if (-not (Test-Path -LiteralPath $images)) { New-Item -ItemType Directory -Path $images | Out-Null }

$triangle = @'
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
'@
Invoke-WgpuRender $triangle 256 256 -VertexCount 3 -OutFile (Join-Path $images 'triangle.png')

$squares = @'
struct Out { @builtin(position) position: vec4<f32>, @location(0) color: vec3<f32> }
@vertex
fn vs(@location(0) corner: vec2<f32>, @builtin(instance_index) n: u32) -> Out {
    let offset = vec2<f32>(f32(n % 2u) - 0.5, f32(n / 2u) - 0.5);
    var out: Out;
    out.position = vec4<f32>(corner * 0.4 + offset, 0.0, 1.0);
    out.color = vec3<f32>(f32(n) / 3.0, 0.6, 1.0 - f32(n) / 3.0);
    return out;
}
@fragment
fn fs(input: Out) -> @location(0) vec4<f32> { return vec4<f32>(input.color, 1.0); }
'@
$corners = @{ corner = [float[]](-1, -1, 1, -1, -1, 1, 1, 1) }
Invoke-WgpuRender $squares 128 128 -Vertex $corners -Index ([uint32[]](0, 1, 2, 2, 1, 3)) -InstanceCount 4 -ClearColor 1, 1, 1 -OutFile (Join-Path $images 'squares.png')

# The pictures the image how-tos load. Each fragment shader colors every
# pixel of the target, which one triangle covers.
$cover = @'
@vertex
fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    var corners = array<vec2<f32>, 3>(vec2<f32>(-1.0, -1.0), vec2<f32>(3.0, -1.0), vec2<f32>(-1.0, 3.0));
    return vec4<f32>(corners[i], 0.0, 1.0);
}
'@

# A landscape: sky, sun and two hills, edges smoothed over a pixel. With
# sun_power above 1 the sun is brighter than white, for the copy in
# scene.hdr, a Radiance file whose shared-exponent texels keep it in a few
# kilobytes.
$landscape = $cover + @'
override sun_power: f32 = 1.0;
@fragment
fn fs(@builtin(position) p: vec4<f32>) -> @location(0) vec4<f32> {
    let uv = p.xy / vec2<f32>(256.0, 160.0);
    var color = mix(vec3<f32>(0.18, 0.42, 0.82), vec3<f32>(0.86, 0.93, 1.0), clamp(uv.y / 0.6, 0.0, 1.0));
    color = mix(color, vec3<f32>(1.0, 0.94, 0.62) * sun_power, clamp(17.0 - distance(p.xy, vec2<f32>(188.0, 44.0)), 0.0, 1.0));
    color = mix(color, vec3<f32>(0.33, 0.56, 0.37), clamp((uv.y - 0.56 - 0.05 * sin(uv.x * 9.0)) * 160.0, 0.0, 1.0));
    color = mix(color, vec3<f32>(0.19, 0.4, 0.17), clamp((uv.y - 0.74 - 0.06 * sin(uv.x * 5.0 + 1.5)) * 160.0, 0.0, 1.0));
    return vec4<f32>(color, 1.0);
}
'@
Invoke-WgpuRender $landscape 256 160 -VertexCount 3 -OutFile (Join-Path $images 'scene.png')
Invoke-WgpuRender $landscape 256 160 -VertexCount 3 -Format Rgba16Float -Constant @{ sun_power = 24 } -OutFile (Join-Path $images 'scene.hdr')

# A sprite: an orange disc with a darker ring, whose alpha fades to 0 over
# its last 8 pixels. Every texel with alpha 0 is black.
$sprite = $cover + @'
@fragment
fn fs(@builtin(position) p: vec4<f32>) -> @location(0) vec4<f32> {
    let d = distance(p.xy, vec2<f32>(64.0, 64.0));
    let alpha = clamp((52.0 - d) / 8.0, 0.0, 1.0);
    let ring = clamp(1.0 - abs(d - 36.0) / 4.0, 0.0, 1.0);
    let color = mix(vec3<f32>(1.0, 0.62, 0.15), vec3<f32>(0.6, 0.22, 0.05), ring);
    return vec4<f32>(color * ceil(alpha), alpha);
}
'@
Invoke-WgpuRender $sprite 128 128 -VertexCount 3 -OutFile (Join-Path $images 'sprite.png')

# A badge on a magenta key color, with hard edges as a sprite sheet has.
$badge = $cover + @'
@fragment
fn fs(@builtin(position) p: vec4<f32>) -> @location(0) vec4<f32> {
    let d = distance(p.xy, vec2<f32>(32.0, 32.0));
    var color = vec3<f32>(1.0, 0.0, 1.0);
    if (d < 26.0) { color = vec3<f32>(0.15, 0.55, 0.9); }
    if (d < 14.0) { color = vec3<f32>(1.0, 1.0, 1.0); }
    return vec4<f32>(color, 1.0);
}
'@
Invoke-WgpuRender $badge 64 64 -VertexCount 3 -OutFile (Join-Path $images 'badge.png')

# Three grayscale masks of a material: occlusion darkening toward the
# corners, roughness in bands, and metal in a checkerboard.
$masks = $cover + @'
override mask: u32 = 0u;
@fragment
fn fs(@builtin(position) p: vec4<f32>) -> @location(0) vec4<f32> {
    let cell = vec2<u32>(p.xy) / 16u;
    var value = 1.0 - 0.7 * smoothstep(0.3, 1.2, distance(p.xy, vec2<f32>(32.0, 32.0)) / 32.0);
    if (mask == 1u) { value = select(0.2, 0.8, cell.y % 2u == 1u); }
    if (mask == 2u) { value = select(0.0, 1.0, (cell.x + cell.y) % 2u == 0u); }
    return vec4<f32>(value, value, value, 1.0);
}
'@
Invoke-WgpuRender $masks 64 64 -VertexCount 3 -Format R8Unorm -Constant @{ mask = 0 } -OutFile (Join-Path $images 'occlusion.png')
Invoke-WgpuRender $masks 64 64 -VertexCount 3 -Format R8Unorm -Constant @{ mask = 1 } -OutFile (Join-Path $images 'roughness.png')
Invoke-WgpuRender $masks 64 64 -VertexCount 3 -Format R8Unorm -Constant @{ mask = 2 } -OutFile (Join-Path $images 'metalness.png')

# A height map of a brick wall that tiles: bricks of 64 by 32 pixels, each
# row shifted by half a brick, their faces beveled down to the mortar over
# 5 pixels, and each brick a little higher or lower than its neighbors.
$bricks = $cover + @'
@fragment
fn fs(@builtin(position) p: vec4<f32>) -> @location(0) vec4<f32> {
    let row = u32(p.y) / 32u;
    let x = p.x + select(0.0, 32.0, row % 2u == 1u);
    let column = u32(x) / 64u % 2u;
    let local = vec2<f32>(x % 64.0, p.y % 32.0);
    let edge = min(min(local.x, 64.0 - local.x), min(local.y, 32.0 - local.y));
    let face = 0.75 + 0.08 * f32((row * 3u + column * 5u) % 4u) / 3.0;
    let height = mix(0.2, face, smoothstep(1.0, 6.0, edge));
    return vec4<f32>(height, height, height, 1.0);
}
'@
Invoke-WgpuRender $bricks 128 128 -VertexCount 3 -Format R8Unorm -OutFile (Join-Path $images 'height.png')

# How To Use An Existing Image: the landscape's edges, found by a compute
# shader, and the landscape sampled onto a turned card.
$scene = Import-WgpuImage (Join-Path $images 'scene.png')
$edgeFinder = @'
@group(0) @binding(0) var source: texture_2d<f32>;
@group(0) @binding(1) var edges: texture_storage_2d<rgba8unorm, write>;
fn brightness(x: i32, y: i32) -> f32 {
    let size = vec2<i32>(textureDimensions(source));
    let c = textureLoad(source, clamp(vec2<i32>(x, y), vec2<i32>(0), size - 1), 0).rgb;
    return dot(c, vec3<f32>(0.2126, 0.7152, 0.0722));
}
@compute @workgroup_size(8, 8)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    let size = textureDimensions(source);
    if (id.x >= size.x || id.y >= size.y) { return; }
    let x = i32(id.x);
    let y = i32(id.y);
    let across = brightness(x + 1, y - 1) + 2.0 * brightness(x + 1, y) + brightness(x + 1, y + 1)
        - brightness(x - 1, y - 1) - 2.0 * brightness(x - 1, y) - brightness(x - 1, y + 1);
    let down = brightness(x - 1, y + 1) + 2.0 * brightness(x, y + 1) + brightness(x + 1, y + 1)
        - brightness(x - 1, y - 1) - 2.0 * brightness(x, y - 1) - brightness(x + 1, y - 1);
    let edge = min(length(vec2<f32>(across, down)), 1.0);
    textureStore(edges, vec2<i32>(id.xy), vec4<f32>(edge, edge, edge, 1.0));
}
'@
$edges = New-WgpuTexture $scene.Width $scene.Height -Usage StorageBinding, CopySrc
Invoke-WgpuCompute $edgeFinder @{ source = $scene; edges = $edges } -Invocations $scene.Width, $scene.Height
Export-WgpuImage $edges (Join-Path $images 'edges.png')
$card = @'
struct Out { @builtin(position) position: vec4<f32>, @location(0) uv: vec2<f32> }
@group(0) @binding(0) var picture: texture_2d<f32>;
@group(0) @binding(1) var picture_sampler: sampler;
@vertex
fn vs(@location(0) corner: vec2<f32>) -> Out {
    let turn = radians(12.0);
    let p = corner * vec2<f32>(100.0, 62.5);
    let turned = vec2<f32>(p.x * cos(turn) - p.y * sin(turn), p.x * sin(turn) + p.y * cos(turn));
    var out: Out;
    out.position = vec4<f32>(turned / vec2<f32>(160.0, 120.0), 0.0, 1.0);
    out.uv = vec2<f32>(corner.x + 1.0, 1.0 - corner.y) / 2.0;
    return out;
}
@fragment
fn fs(input: Out) -> @location(0) vec4<f32> { return textureSample(picture, picture_sampler, input.uv); }
'@
$cardCorners = @{ corner = [float[]](-1, -1, 1, -1, -1, 1, 1, 1) }
$look = @{ picture = $scene; picture_sampler = (New-WgpuSampler -Filter Linear) }
Invoke-WgpuRender $card 320 240 -Vertex $cardCorners -Index ([uint32[]](0, 1, 2, 2, 1, 3)) -Binding $look -SampleCount 4 -ClearColor 0.9, 0.9, 0.9 -OutFile (Join-Path $images 'card.png')

# How To Edit And Adjust Images: a framed thumbnail, three color changes,
# the landscape blurred, sharpened and outlined, the badge cut out of its
# key color, and the landscape in scene.hdr tone mapped.
$thumb = $scene | Resize-WgpuTexture -Crop 128, 0, 128, 96 -Width 80 -Pad 96, 96 -PadColor 1, 1, 1, 1
Export-WgpuImage $thumb (Join-Path $images 'thumb.png')
$small = $scene | Resize-WgpuTexture -Width 128
$small | Convert-WgpuTexture -Brightness 10 -Contrast -20 -Gamma 1.2 | Export-WgpuImage -Path (Join-Path $images 'adjusted.png')
$small | Convert-WgpuTexture -Grayscale | Export-WgpuImage -Path (Join-Path $images 'gray.png')
$small | Convert-WgpuTexture -Invert | Export-WgpuImage -Path (Join-Path $images 'negative.png')
$small | Convert-WgpuTexture -Blur 2 | Export-WgpuImage -Path (Join-Path $images 'soft.png')
$small | Convert-WgpuTexture -Sharpen 1 | Export-WgpuImage -Path (Join-Path $images 'sharp.png')
$small | Convert-WgpuTexture -Edges Canny | Export-WgpuImage -Path (Join-Path $images 'outline.png')
$badgeTexture = Import-WgpuImage (Join-Path $images 'badge.png')
Export-WgpuImage ($badgeTexture | Convert-WgpuTexture -ColorKey '#FF00FF') (Join-Path $images 'badge-cut.png')
$hdr = Import-WgpuImage (Join-Path $images 'scene.hdr')
$hdr | Convert-WgpuTexture -ToneMap AgX -ColorSpace Srgb | Export-WgpuImage -Path (Join-Path $images 'scene-agx.png')
$hdr | Convert-WgpuTexture -Exposure -2 -ToneMap AgX -ColorSpace Srgb | Export-WgpuImage -Path (Join-Path $images 'scene-darker.png')

# Textures and images: the small landscape as JPEG files that record more
# than their pixels, in a segment after the start of the image.
# turned.jpg is recorded as turned a quarter clockwise, its EXIF
# orientation of 6 in an APP1 segment. landscape-p3.jpg keeps the
# landscape's values under a Display P3 profile, in an APP2 segment: a
# version 2 display profile of Display P3's colorants, adapted to D50, and
# sRGB's curve.
function ConvertTo-BigEndian([long] $value, [int] $count) {
    [byte[]]@(for ($i = $count - 1; $i -ge 0; $i--) { ($value -shr (8 * $i)) -band 255 })
}
function Add-JpegSegment([string] $path, [byte] $marker, [byte[]] $body) {
    $jpeg = [IO.File]::ReadAllBytes($path)
    $segment = [byte[]]((0xFF, $marker) + (ConvertTo-BigEndian ($body.Length + 2) 2) + $body)
    [IO.File]::WriteAllBytes($path, [byte[]]($jpeg[0..1] + $segment + $jpeg[2..($jpeg.Length - 1)]))
}
$turnedPath = Join-Path $images 'turned.jpg'
Export-WgpuImage $small $turnedPath
$orientation = [byte[]](0x49, 0x49, 42, 0, 8, 0, 0, 0, 1, 0, 0x12, 0x01, 3, 0, 1, 0, 0, 0, 6, 0, 0, 0, 0, 0, 0, 0)
Add-JpegSegment $turnedPath 0xE1 ([byte[]]([Text.Encoding]::ASCII.GetBytes("Exif`0`0") + $orientation))
$s15 = { param([double] $v) ConvertTo-BigEndian ([long][math]::Round($v * 65536)) 4 }
$xyz = { param([double[]] $v) [byte[]]([Text.Encoding]::ASCII.GetBytes('XYZ ') + [byte[]]::new(4) + ($v | ForEach-Object { & $s15 $_ })) }
$name = [Text.Encoding]::ASCII.GetBytes("Display P3`0")
$srgbCurve = [byte[]]([Text.Encoding]::ASCII.GetBytes('para') + [byte[]]::new(4) + (ConvertTo-BigEndian 3 2) + [byte[]]::new(2) + ((2.4, (1 / 1.055), (0.055 / 1.055), (1 / 12.92), 0.04045) | ForEach-Object { & $s15 $_ }))
$tags = [ordered]@{
    desc = [byte[]]([Text.Encoding]::ASCII.GetBytes('desc') + [byte[]]::new(4) + (ConvertTo-BigEndian $name.Length 4) + $name + [byte[]]::new(78))
    wtpt = & $xyz 0.9642, 1.0, 0.8249
    rXYZ = & $xyz 0.5151215, 0.2411957, -0.0010529
    gXYZ = & $xyz 0.2919769, 0.6922455, 0.0418854
    bXYZ = & $xyz 0.1571045, 0.0665588, 0.7840729
    rTRC = $srgbCurve
    gTRC = $srgbCurve
    bTRC = $srgbCurve
}
$table = [byte[]](ConvertTo-BigEndian $tags.Count 4)
$body = [byte[]]@()
foreach ($tag in $tags.Keys) {
    $data = [byte[]]$tags[$tag]
    $table += [byte[]]([Text.Encoding]::ASCII.GetBytes($tag) + (ConvertTo-BigEndian (132 + 12 * $tags.Count + $body.Length) 4) + (ConvertTo-BigEndian $data.Length 4))
    $body += $data + [byte[]]::new((4 - $data.Length % 4) % 4)
}
# The header: size, CMM, version 2.1, class, space and PCS, date,
# signature, platform to intent, D50 and the rest.
$header = [byte[]]((ConvertTo-BigEndian (128 + $table.Length + $body.Length) 4) + [byte[]](0, 0, 0, 0, 2, 0x10, 0, 0) + [Text.Encoding]::ASCII.GetBytes('mntrRGB XYZ ') + [byte[]]::new(12) + [Text.Encoding]::ASCII.GetBytes('acsp') + [byte[]]::new(28) + ((0.9642, 1.0, 0.8249) | ForEach-Object { & $s15 $_ }) + [byte[]]::new(48))
$p3Path = Join-Path $images 'landscape-p3.jpg'
Export-WgpuImage $small $p3Path
Add-JpegSegment $p3Path 0xE2 ([byte[]]([Text.Encoding]::ASCII.GetBytes("ICC_PROFILE`0") + [byte[]](1, 1) + $header + $table + $body))

# How To Prepare Textures For A Renderer: the error of the landscape's
# BC7 copy against the picture loaded the same way without -Format, and
# the normal map of the brick wall.
$scenePath = Join-Path $images 'scene.png'
$bc7Scene = Import-WgpuImage $scenePath -PowerOfTwo Nearest -MipLevels 0 -MipFilter Kaiser -Format Bc7RgbaUnormSrgb
$plainScene = Import-WgpuImage $scenePath -PowerOfTwo Nearest -MipLevels 0 -MipFilter Kaiser -Srgb
$bc7Scene | Compare-WgpuTexture $plainScene -Difference | Export-WgpuImage -Path (Join-Path $images 'scene-bc7-error.png')
Import-WgpuImage (Join-Path $images 'height.png') | ConvertTo-WgpuNormalMap -Strength 4 -HeightEdge Wrap | Export-WgpuImage -Path (Join-Path $images 'wall-normal.png')

# How To Pack Channels Into One Texture: the three masks in one texture.
$packedMasks =Join-WgpuTexture -Red (Import-WgpuImage (Join-Path $images 'occlusion.png')) -Green (Import-WgpuImage (Join-Path $images 'roughness.png')) -Blue (Import-WgpuImage (Join-Path $images 'metalness.png'))
Export-WgpuImage $packedMasks (Join-Path $images 'packed.png')

# How To Draw In A Render Pass Of Your Own: one scene drawn twice in one
# pass, through two viewports, the second inside a scissor rectangle.
$tilesShader = @'
struct View { center: vec2<f32>, zoom: f32 }
struct Out { @builtin(position) position: vec4<f32>, @location(0) color: vec3<f32> }
@group(0) @binding(0) var<uniform> view: View;
@vertex
fn vs(@location(0) corner: vec2<f32>, @builtin(instance_index) n: u32) -> Out {
    let cell = vec2<f32>(f32(n % 4u), f32(n / 4u)) - vec2<f32>(1.5);
    let turn = f32(n) * 0.2;
    let c = corner * 0.22;
    let turned = vec2<f32>(c.x * cos(turn) - c.y * sin(turn), c.x * sin(turn) + c.y * cos(turn));
    var out: Out;
    out.position = vec4<f32>((cell * 0.42 + turned - view.center) * view.zoom, f32(n) / 16.0, 1.0);
    let t = f32(n) / 15.0;
    out.color = vec3<f32>(t, 0.4 + 0.5 * sin(t * 3.0), 1.0 - t);
    return out;
}
@fragment
fn fs(input: Out) -> @location(0) vec4<f32> { return vec4<f32>(input.color, 1.0); }
'@
$tilesPipeline = New-WgpuRenderPipeline $tilesShader -Depth -SampleCount 4
$tileCorners = New-WgpuBuffer ([float[]](-1, -1, 1, -1, -1, 1, 1, 1))
$tileIndices = New-WgpuBuffer ([uint32[]](0, 1, 2, 2, 1, 3))
$whole = $tilesPipeline.BindGroup(0, @{ view = (New-WgpuBuffer ([float[]](0, 0, 1, 0))) })
$close = $tilesPipeline.BindGroup(0, @{ view = (New-WgpuBuffer ([float[]](-0.21, -0.21, 2.5, 0))) })
$samples = New-WgpuTexture 256 128 -SampleCount 4 -Usage RenderAttachment
$depthTarget = New-WgpuTexture 256 128 -Format Depth32Float -SampleCount 4 -Usage RenderAttachment
$frame = New-WgpuTexture 256 128 -Usage RenderAttachment, CopySrc
$encoder = New-WgpuCommandEncoder -Label frame
$pass = $encoder.BeginRenderPass($samples, @(1, 1, 1, 1), $depthTarget, $frame)
$pass.SetPipeline($tilesPipeline)
$pass.SetVertexBuffer(0, $tileCorners)
$pass.SetIndexBuffer($tileIndices)
$pass.SetViewport(0, 0, 128, 128)
$pass.SetBindGroup(0, $whole)
$pass.DrawIndexed(6, 16)
$pass.SetViewport(128, 0, 128, 128)
$pass.SetScissorRect(136, 8, 112, 112)
$pass.SetBindGroup(0, $close)
$pass.DrawIndexed(6, 16)
$pass.End()
Submit-WgpuCommand $encoder.Finish()
Export-WgpuImage $frame (Join-Path $images 'two-views.png')

# How To Work With Textures And Images: two kinds of floor tile written by
# a compute shader into the layers of one texture, given mip levels, and
# an endless floor of them drawn through three samplers.
$tileWriter = @'
@group(0) @binding(0) var tiles: texture_storage_2d_array<rgba8unorm, write>;
@compute @workgroup_size(8, 8)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    let size = textureDimensions(tiles);
    if (id.x >= size.x || id.y >= size.y) { return; }
    var shade = select(0.3, 0.8, (id.x / 16u + id.y / 16u) % 2u == 0u);
    if (id.x % 16u == 0u || id.y % 16u == 0u) { shade = 0.1; }
    let tint = select(vec3<f32>(1.0, 0.85, 0.7), vec3<f32>(0.7, 0.85, 1.0), id.z == 1u);
    textureStore(tiles, vec2<i32>(id.xy), id.z, vec4<f32>(shade * tint, 1.0));
}
'@
$tiles = New-WgpuTexture 128 128 -Layers 2 -MipLevels 8
Invoke-WgpuCompute $tileWriter @{ tiles = $tiles.CreateView($null, 'D2Array', 0, 1) } -Invocations 128, 128, 2
Export-WgpuImage $tiles (Join-Path $images 'tiles.png')
Export-WgpuImage $tiles (Join-Path $images 'tiles-cool.png') -Layer 1
$tiles.GenerateMips()
$floor = @'
@group(0) @binding(0) var tiles: texture_2d_array<f32>;
@group(0) @binding(1) var tile_sampler: sampler;
struct Out { @builtin(position) position: vec4<f32>, @location(0) screen: vec2<f32> }
@vertex
fn vs(@builtin(vertex_index) i: u32) -> Out {
    let p = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u)) * 2.0 - 1.0;
    return Out(vec4<f32>(p, 0.0, 1.0), p);
}
@fragment
fn fs(input: Out) -> @location(0) vec4<f32> {
    let below = max(0.5 - input.screen.y, 0.001);
    let uv = vec2<f32>(input.screen.x, 1.0) / below;
    let layer = select(0, 1, abs(uv.x) < 0.5);
    let ground = textureSample(tiles, tile_sampler, uv, layer);
    return select(ground, vec4<f32>(0.6, 0.75, 0.95, 1.0), input.screen.y > 0.5);
}
'@
$level0Only = New-WgpuSampler -Filter Linear -AddressMode Repeat -LodMaxClamp 0
Invoke-WgpuRender $floor 256 128 -VertexCount 3 -Binding @{ tiles = $tiles; tile_sampler = $level0Only } -OutFile (Join-Path $images 'floor-level0.png')
$mipmapped = New-WgpuSampler -Filter Linear -AddressMode Repeat
Invoke-WgpuRender $floor 256 128 -VertexCount 3 -Binding @{ tiles = $tiles; tile_sampler = $mipmapped } -OutFile (Join-Path $images 'floor-mipmapped.png')
$anisotropic = New-WgpuSampler -Filter Linear -AddressMode Repeat -Anisotropy 16
Invoke-WgpuRender $floor 256 128 -VertexCount 3 -Binding @{ tiles = $tiles; tile_sampler = $anisotropic } -OutFile (Join-Path $images 'floor-anisotropic.png')

# How To Render Offscreen: a year's rainfall as a bar for each month, and
# the seasons as a pie, with one sample a pixel and with four.
$rain = [float[]](78, 62, 55, 43, 38, 29, 21, 25, 37, 58, 71, 84)
$bars = @'
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
'@
$square = @{ corner = [float[]](0, 0, 1, 0, 0, 1, 1, 1) }
$twoTriangles = [uint32[]](0, 1, 2, 2, 1, 3)
$chart = @{ values = $rain; chart = @{ count = $rain.Length; top = 100 } }
Invoke-WgpuRender $bars 480 240 -Vertex $square -Index $twoTriangles -InstanceCount $rain.Length -Binding $chart -ClearColor 1, 1, 1 -OutFile (Join-Path $images 'rain.png')
$seasons = @(($rain[11] + $rain[0] + $rain[1]), ($rain[2..4] | Measure-Object -Sum).Sum, ($rain[5..7] | Measure-Object -Sum).Sum, ($rain[8..10] | Measure-Object -Sum).Sum)
$starts = [float[]]::new(5)
for ($s = 0; $s -lt 4; $s++) { $starts[$s + 1] = $starts[$s] + $seasons[$s] / ($seasons | Measure-Object -Sum).Sum }
$pie = @'
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
'@
Invoke-WgpuRender $pie 160 160 -VertexCount 96 -InstanceCount 4 -Binding @{ starts = $starts } -ClearColor 1, 1, 1 -OutFile (Join-Path $images 'seasons.png')
Invoke-WgpuRender $pie 160 160 -VertexCount 96 -InstanceCount 4 -Binding @{ starts = $starts } -ClearColor 1, 1, 1 -SampleCount 4 -OutFile (Join-Path $images 'seasons-smooth.png')

# How To Record Commands By Hand: 64 generations of the Game of Life from
# an R-pentomino in one submission, every 16th kept and drawn side by side.
$lifeShader = @'
struct Rule { born: u32, survives: u32 }
var<immediate> rule: Rule;
@group(0) @binding(0) var<storage, read> current: array<u32>;
@group(0) @binding(1) var<storage, read_write> next: array<u32>;
const side = 64u;
@compute @workgroup_size(8, 8)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    if (id.x >= side || id.y >= side) { return; }
    var neighbors = 0u;
    for (var dy = 0u; dy < 3u; dy++) {
        for (var dx = 0u; dx < 3u; dx++) {
            if (dx == 1u && dy == 1u) { continue; }
            neighbors += current[((id.y + dy + side - 1u) % side) * side + (id.x + dx + side - 1u) % side];
        }
    }
    let alive = current[id.y * side + id.x] == 1u;
    next[id.y * side + id.x] = (select(rule.born, rule.survives, alive) >> neighbors) & 1u;
}
'@
$life = New-WgpuComputePipeline $lifeShader
$side = 64
$cells = [uint32[]]::new($side * $side)
foreach ($cell in @(32, 31), @(33, 31), @(31, 32), @(32, 32), @(32, 33)) { $cells[$cell[1] * $side + $cell[0]] = 1 }
$a = New-WgpuBuffer $cells
$b = New-WgpuBuffer -Length $cells.Length -ElementType UInt32
$aToB = New-WgpuBindGroup $life @{ current = $a; next = $b }
$bToA = New-WgpuBindGroup $life @{ current = $b; next = $a }
$history = New-WgpuBuffer -Length ($cells.Length * 5) -ElementType UInt32
$counts = New-WgpuBuffer ([uint32[]](8, 8, 1))
$lifeEncoder = New-WgpuCommandEncoder -Label life
$lifeEncoder.CopyBufferToBuffer($a, 0, $history, 0)
for ($snapshot = 1; $snapshot -le 4; $snapshot++) {
    $lifePass = $lifeEncoder.BeginComputePass()
    $lifePass.SetPipeline($life)
    $lifePass.SetImmediates(0, [uint32[]](8, 12))
    for ($generation = 0; $generation -lt 16; $generation++) {
        $lifePass.SetBindGroup(0, $(if ($generation % 2 -eq 0) { $aToB } else { $bToA }))
        $lifePass.DispatchIndirect($counts, 0)
    }
    $lifePass.End()
    $lifeEncoder.CopyBufferToBuffer($a, 0, $history, $snapshot * $cells.Length * 4)
}
Submit-WgpuCommand $lifeEncoder.Finish()
$all = [uint32[]](Read-WgpuBuffer $history)
$pixels = [byte[]]::new($all.Length * 4)
for ($i = 0; $i -lt $all.Length; $i++) {
    $grid = [math]::Floor($i / 4096); $cell = $i % 4096
    $at = ([math]::Floor($cell / $side) * 5 * $side + $grid * $side + $cell % $side) * 4
    $shade = if ($all[$i]) { 30 } else { 240 }
    $pixels[$at] = $shade; $pixels[$at + 1] = $shade; $pixels[$at + 2] = $shade; $pixels[$at + 3] = 255
}
New-WgpuTexture (5 * $side) $side -Data $pixels | Resize-WgpuTexture -Scale 2 -Filter Nearest | Export-WgpuImage -Path (Join-Path $images 'life.png')

# How To Light A Scene With An Environment and Textures and images: two
# panoramas, 512 by 256, each texel the light from the direction it faces,
# center +X, left quarter -Z and top +Y, as the cube map cmdlets read them.
$panoramaDirection = @'
fn facing(p: vec2<f32>) -> vec3<f32> {
    let phi = (p.x / 512.0 - 0.5) * 6.2831853;
    let theta = p.y / 256.0 * 3.1415927;
    return vec3<f32>(sin(theta) * cos(phi), cos(theta), sin(theta) * sin(phi));
}
'@

# A clear sky by Preetham, Shirley and Smits's model at turbidity 2.2: the
# zenith's luminance and chromaticity from the sun's height, spread over
# the sky by Perez's function, and converted from Yxy to linear Rec. 709.
# The sun is 3 degrees across, 30 degrees up toward +Z and -X, and lights
# a dark green ground that fades into the horizon's haze.
$skyShader = $cover + $panoramaDirection + @'
const turbidity = 2.2;
const sun = vec3<f32>(-0.6123724, 0.5, 0.6123724);
fn perez(cos_theta: f32, gamma: f32) -> vec3<f32> {
    let t = turbidity;
    let a = vec3<f32>(0.1787 * t - 1.463, -0.0193 * t - 0.2592, -0.0167 * t - 0.2608);
    let b = vec3<f32>(-0.3554 * t + 0.4275, -0.0665 * t + 0.0008, -0.095 * t + 0.0092);
    let c = vec3<f32>(-0.0227 * t + 5.3251, -0.0004 * t + 0.2125, -0.0079 * t + 0.2102);
    let d = vec3<f32>(0.1206 * t - 2.5771, -0.0641 * t - 0.8989, -0.0441 * t - 1.6537);
    let e = vec3<f32>(-0.067 * t + 0.3703, -0.0033 * t + 0.0452, -0.0109 * t + 0.0529);
    return (1.0 + a * exp(b / max(cos_theta, 0.01))) * (1.0 + c * exp(d * gamma) + e * cos(gamma) * cos(gamma));
}
fn sky(d: vec3<f32>) -> vec3<f32> {
    let t = turbidity;
    let theta_s = acos(sun.y);
    let chi = (4.0 / 9.0 - t / 120.0) * (3.1415927 - 2.0 * theta_s);
    let zenith_y = (4.0453 * t - 4.971) * tan(chi) - 0.2155 * t + 2.4192;
    let s = vec3<f32>(theta_s * theta_s * theta_s, theta_s * theta_s, theta_s);
    let zenith_x = t * t * dot(vec3<f32>(0.00166, -0.00375, 0.00209), s) + t * (dot(vec3<f32>(-0.02903, 0.06377, -0.03202), s) + 0.00394) + dot(vec3<f32>(0.11693, -0.21196, 0.06052), s) + 0.25886;
    let zenith_xy = t * t * dot(vec3<f32>(0.00275, -0.0061, 0.00317), s) + t * (dot(vec3<f32>(-0.04214, 0.0897, -0.04153), s) + 0.00516) + dot(vec3<f32>(0.15346, -0.26756, 0.0667), s) + 0.26688;
    let gamma = acos(clamp(dot(d, sun), -1.0, 1.0));
    let yxy = vec3<f32>(zenith_y, zenith_x, zenith_xy) * perez(d.y, gamma) / perez(1.0, theta_s);
    let big_y = yxy.x * 0.05;
    let xyz = vec3<f32>(yxy.y / yxy.z * big_y, big_y, (1.0 - yxy.y - yxy.z) / yxy.z * big_y);
    return max(mat3x3<f32>(3.2406, -0.9689, 0.0557, -1.5372, 1.8758, -0.204, -0.4986, 0.0415, 1.057) * xyz, vec3<f32>(0.0));
}
@fragment
fn fs(@builtin(position) p: vec4<f32>) -> @location(0) vec4<f32> {
    let d = facing(p.xy);
    let horizon = sky(normalize(vec3<f32>(d.x, 0.0, d.z)));
    let ground = vec3<f32>(0.1, 0.1, 0.06) * (3.9 * sun.y + 1.1) / 3.1415927;
    var color = select(sky(d), mix(horizon, ground, smoothstep(0.0, 0.12, -d.y)), d.y < 0.0);
    let disc = 1.0 - smoothstep(0.99962, 0.99968, dot(d, sun));
    color = mix(color, vec3<f32>(1.0, 0.92, 0.8) * 1800.0, 1.0 - disc);
    return vec4<f32>(color, 1.0);
}
'@

# A studio: a dark backdrop, a warm key softbox and a cool fill beside a
# camera looking toward -Z, and a cyan strip behind the subject, out of the
# camera's view, that lights its edges.
$studioShader = $cover + $panoramaDirection + @'
fn panel(d: vec3<f32>, center: vec3<f32>, half_size: vec2<f32>) -> f32 {
    let right = normalize(cross(vec3<f32>(0.0, 1.0, 0.0), center));
    let up = cross(center, right);
    let along = dot(d, center);
    let angle = vec2<f32>(atan2(dot(d, right), along), atan2(dot(d, up), along));
    let inside = (1.0 - smoothstep(half_size.x - 0.02, half_size.x, abs(angle.x))) * (1.0 - smoothstep(half_size.y - 0.02, half_size.y, abs(angle.y)));
    return select(0.0, inside * (1.0 - 0.25 * dot(angle / half_size, angle / half_size)), along > 0.0);
}
fn toward(azimuth: f32, elevation: f32) -> vec3<f32> {
    let a = radians(azimuth);
    let e = radians(elevation);
    return vec3<f32>(cos(e) * cos(a), sin(e), cos(e) * sin(a));
}
@fragment
fn fs(@builtin(position) p: vec4<f32>) -> @location(0) vec4<f32> {
    let d = facing(p.xy);
    var color = mix(vec3<f32>(0.03, 0.03, 0.032), vec3<f32>(0.07, 0.07, 0.075), smoothstep(-0.2, 0.6, d.y));
    color = mix(color, vec3<f32>(0.012, 0.012, 0.012), smoothstep(-0.05, -0.4, d.y));
    color += vec3<f32>(1.0, 0.95, 0.88) * 12.0 * panel(d, toward(130.0, 30.0), vec2<f32>(radians(20.0), radians(13.0)));
    color += vec3<f32>(0.88, 0.94, 1.0) * 4.0 * panel(d, toward(50.0, 18.0), vec2<f32>(radians(11.0), radians(20.0)));
    color += vec3<f32>(0.2, 0.7, 1.0) * 10.0 * panel(d, toward(-30.0, 12.0), vec2<f32>(radians(3.0), radians(24.0)));
    return vec4<f32>(color, 1.0);
}
'@
foreach ($panorama in @(@('sky', $skyShader), @('studio', $studioShader))) {
    foreach ($extension in 'hdr', 'exr') {
        Invoke-WgpuRender $panorama[1] 512 256 -VertexCount 3 -Format Rgba16Float -OutFile (Join-Path $images "$($panorama[0]).$extension")
    }
    Import-WgpuImage (Join-Path $images "$($panorama[0]).hdr") | Convert-WgpuTexture -ToneMap AgX -ColorSpace Srgb -Format Rgba8Unorm | Export-WgpuImage -Path (Join-Path $images "$($panorama[0]).png")
}

# Textures and images: the sky as a cube of 64-texel faces, laid out as a
# cross and cut into its six faces, tone mapped as each panorama is.
$skyCube = Import-WgpuImage (Join-Path $images 'sky.hdr') | ConvertTo-WgpuCubeMap -FaceSize 64 | Convert-WgpuTexture -ToneMap AgX -ColorSpace Srgb -Format Rgba8Unorm
$skyCube | ConvertFrom-WgpuCubeMap | Export-WgpuImage -Path (Join-Path $images 'sky-cross.png')
$faceNames = 'px', 'nx', 'py', 'ny', 'pz', 'nz'
foreach ($face in 0..5) { Export-WgpuImage $skyCube (Join-Path $images "sky-$($faceNames[$face]).png") -Layer $face }

# How To Light A Scene With An Environment: spheres from smooth to rough,
# red plastic above and gold below, lit by each panorama through split-sum
# image-based lighting, and lit by the sky again from a specular map whose
# levels end at 8 texels a face.
$spheres = @'
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
'@
$brdf = New-WgpuBrdfTable
$smooth = New-WgpuSampler -Filter Linear
foreach ($name in 'sky', 'studio') {
    # The page lights the spheres from sky.hdr and from studio.exr.
    $environment = Import-WgpuImage (Join-Path $images $(if ($name -eq 'sky') { 'sky.hdr' } else { 'studio.exr' }))
    $lighting = @{ irradiance = ($environment | ConvertTo-WgpuIrradianceMap); specular = ($environment | ConvertTo-WgpuSpecularMap); brdf = $brdf; smp = $smooth }
    Invoke-WgpuRender $spheres 640 256 -VertexCount 3 -Binding $lighting -Format Rgba16Float | Convert-WgpuTexture -ToneMap AgX -ColorSpace Srgb -Format Rgba8Unorm | Export-WgpuImage -Path (Join-Path $images "spheres-$name.png")
    if ($name -eq 'sky') {
        $lighting.specular = $environment | ConvertTo-WgpuSpecularMap -MipLevels 5
        Invoke-WgpuRender $spheres 640 256 -VertexCount 3 -Binding $lighting -Format Rgba16Float | Convert-WgpuTexture -ToneMap AgX -ColorSpace Srgb -Format Rgba8Unorm | Export-WgpuImage -Path (Join-Path $images 'spheres-sky-8.png')
    }
}

Get-ChildItem -LiteralPath $images -Filter *.png | Select-Object Name, Length
