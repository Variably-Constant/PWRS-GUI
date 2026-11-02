# The GPU commands, in every mode the platform has. Buffers, textures,
# samplers and pipelines live in the egui renderer whichever renderer draws
# widget windows; a surface's cases run in windows of both renderers. See
# Common.ps1 for the deadlines.

BeforeDiscovery {
    $modes = if ($IsWindows -or $PSVersionTable.PSEdition -eq 'Desktop') { 'Helper', 'InProcess' } else { 'Helper' }
    $cases = foreach ($m in $modes) { @{ Mode = $m } }
    $surfaceCases = foreach ($m in $modes) { foreach ($r in 'Egui', 'Html') { @{ Mode = $m; Renderer = $r } } }
}

BeforeAll {
    . (Join-Path $PSScriptRoot 'Common.ps1')
    $script:Watchdog = Start-Watchdog

    # The error id a command's terminating error has, without the command's
    # name after it.
    function Get-ErrorId([scriptblock]$Command) {
        try { & $Command; 'no error' } catch { ($_.FullyQualifiedErrorId -split ',')[0] }
    }
}

AfterAll {
    Stop-Watchdog $script:Watchdog
}

Describe 'GPU buffers in <Mode> mode' -ForEach $cases {
    BeforeAll {
        Reset-GuiSession $Mode
    }

    It 'makes a buffer from a typed array, of the same type, and reads it back' {
        $b = New-GuiGpuBuffer ([float[]](1.5, 2.5, 3.5))
        $b.GetType().FullName | Should -Be 'PwrsGui.GpuBuffer'
        $b.ElementType | Should -Be 'Float32'
        $b.Length | Should -Be 3
        $b.Size | Should -Be 12
        $b.Label | Should -Be '3 x Float32'
        $b.Usage | Should -Be @('CopySrc', 'CopyDst', 'Index', 'Vertex', 'Uniform', 'Storage', 'Indirect', 'QueryResolve')
        $b.Adapter | Should -Be (Get-GuiGpuAdapter | Where-Object InUse | Select-Object -First 1).Name
        $values = Read-GuiGpuBuffer $b
        $values.GetType().FullName | Should -Be 'System.Single[]'
        $values | Should -Be @(1.5, 2.5, 3.5)
    }

    It 'converts an untyped array to -Type as PowerShell casts numbers, and to Float32 without it' {
        (Read-GuiGpuBuffer (New-GuiGpuBuffer @(1.5, 2.5, -3) -Type Int32)) | Should -Be @(2, 2, -3)
        (New-GuiGpuBuffer @(1, 2)).ElementType | Should -Be 'Float32'
        (Read-GuiGpuBuffer (New-GuiGpuBuffer ([int[]](7, -1)) -Type UInt8)) | Should -Be @(7, 255)
    }

    It 'refuses a number outside the type, text, and no elements, with the Wgpu module''s ids and its own' {
        Get-ErrorId { New-GuiGpuBuffer @(300) -Type UInt8 -ErrorAction Stop } | Should -Be 'WgpuOutOfRange'
        Get-ErrorId { New-GuiGpuBuffer 'seven' -ErrorAction Stop } | Should -Be 'WgpuNotNumeric'
        Get-ErrorId { New-GuiGpuBuffer -Length 0 -ErrorAction Stop } | Should -Be 'GuiGpuEmpty'
    }

    It 'makes a zeroed buffer of a length and type under its label, its size rounded up to 4 bytes' {
        $b = New-GuiGpuBuffer -Length 5 -Type UInt16 -Label zeros
        $b.Label | Should -Be 'zeros'
        $b.Length | Should -Be 5
        $b.Size | Should -Be 12
        $values = Read-GuiGpuBuffer $b
        $values.GetType().FullName | Should -Be 'System.UInt16[]'
        $values | Should -Be @(0, 0, 0, 0, 0)
    }

    It 'joins numbers piped in into one buffer' {
        $b = 1..4 | New-GuiGpuBuffer -Type Int32
        $b.Length | Should -Be 4
        (Read-GuiGpuBuffer $b) | Should -Be @(1, 2, 3, 4)
    }

    It 'reads a range, the bytes as another type, element by element, and as Memory' {
        $b = New-GuiGpuBuffer ([int[]](1..10))
        (Read-GuiGpuBuffer $b -Offset 2 -Count 3) | Should -Be @(3, 4, 5)
        (Read-GuiGpuBuffer $b -Type UInt32).GetType().FullName | Should -Be 'System.UInt32[]'
        (Read-GuiGpuBuffer $b -Enumerate | Measure-Object -Sum).Sum | Should -Be 55
        $memory = Read-GuiGpuBuffer $b -AsMemory
        if ($PSVersionTable.PSEdition -eq 'Desktop') {
            $memory.GetType().FullName | Should -Be 'System.Int32[]'
        } else {
            $memory.GetType().Name | Should -Be 'Memory`1'
            $memory.ToArray() | Should -Be (1..10)
        }
    }

    It 'writes values from an element on, converted to the buffer''s type' {
        $b = New-GuiGpuBuffer -Length 5 -Type Int32
        Write-GuiGpuBuffer $b @(7.5, 8) -Offset 2
        (Read-GuiGpuBuffer $b) | Should -Be @(0, 0, 8, 8, 0)
    }

    It 'refuses a read or a write past the end' {
        $b = New-GuiGpuBuffer -Length 5 -Type Int32
        Get-ErrorId { Read-GuiGpuBuffer $b -Offset 4 -Count 3 -ErrorAction Stop } | Should -Be 'GuiGpuRange'
        Get-ErrorId { Write-GuiGpuBuffer $b @(1, 2) -Offset 4 -ErrorAction Stop } | Should -Be 'GuiGpuRange'
    }

    It 'takes the usages it is given, listed once each in order' {
        $b = New-GuiGpuBuffer -Length 4 -Usage Storage, CopySrc, CopyDst, Storage
        $b.Usage | Should -Be @('CopySrc', 'CopyDst', 'Storage')
        Write-GuiGpuBuffer $b ([float[]](1, 2, 3, 4))
        (Read-GuiGpuBuffer $b) | Should -Be @(1, 2, 3, 4)
    }

    It 'removes a buffer, which is then refused, and removes it again without complaint' {
        $b = New-GuiGpuBuffer ([float[]](1, 2))
        Remove-GuiGpuBuffer $b -Confirm:$false
        Get-ErrorId { Read-GuiGpuBuffer $b -ErrorAction Stop } | Should -Be 'GuiGpuRemoved'
        { Remove-GuiGpuBuffer $b -Confirm:$false -ErrorAction Stop } | Should -Not -Throw
    }
}

Describe 'GPU compute in <Mode> mode' -ForEach $cases {
    BeforeAll {
        Reset-GuiSession $Mode
        $script:Double = @'
@group(0) @binding(0) var<storage, read_write> v: array<f32>;
@compute @workgroup_size(64) fn main(@builtin(global_invocation_id) i: vec3<u32>) {
    if (i.x < arrayLength(&v)) { v[i.x] = v[i.x] * 2.0; }
}
'@
        $script:Add = @'
@group(0) @binding(0) var<storage, read_write> a: array<f32>;
@group(0) @binding(1) var<storage, read> b: array<f32>;
@compute @workgroup_size(64) fn main(@builtin(global_invocation_id) i: vec3<u32>) {
    if (i.x < arrayLength(&a)) { a[i.x] = a[i.x] + b[i.x]; }
}
'@
    }

    It 'runs a shader over a typed array, one invocation per element, and writes the array back' {
        $out = Invoke-GuiGpuCompute $script:Double ([float[]](1, 2, 3, 4))
        $out.GetType().FullName | Should -Be 'System.Single[]'
        $out | Should -Be @(2, 4, 6, 8)
    }

    It 'converts an untyped array to the type the shader declares as PowerShell casts numbers' {
        $increment = @'
@group(0) @binding(0) var<storage, read_write> v: array<i32>;
@compute @workgroup_size(1) fn main(@builtin(global_invocation_id) i: vec3<u32>) { v[i.x] = v[i.x] + 1; }
'@
        $out = Invoke-GuiGpuCompute $increment @(1.5, 2.5, -4) -Workgroups 3
        $out.GetType().FullName | Should -Be 'System.Int32[]'
        $out | Should -Be @(3, 3, -3)
        Get-ErrorId { Invoke-GuiGpuCompute $increment @(3e10) -Workgroups 1 -ErrorAction Stop } | Should -Be 'WgpuOutOfRange'
    }

    It 'binds a GPU buffer by name, which stays on the GPU, beside an array, in any case' {
        $a = New-GuiGpuBuffer ([float[]](1, 2, 3))
        $written = @(Invoke-GuiGpuCompute $script:Add @{ A = $a; B = [float[]](10, 20, 30) } -Invocations 3)
        $written.Count | Should -Be 0
        (Read-GuiGpuBuffer $a) | Should -Be @(11, 22, 33)
        Invoke-GuiGpuCompute $script:Add @{ '0:0' = $a; '0:1' = [float[]](1, 1, 1) } -Workgroups 1
        (Read-GuiGpuBuffer $a) | Should -Be @(12, 23, 34)
    }

    It 'binds resources in declared order, and with -PassThru writes the outputs by name and the workgroups' {
        $result = Invoke-GuiGpuCompute $script:Add ([float[]](1, 2), [float[]](5, 5)) -PassThru
        $result.GetType().FullName | Should -Be 'PwrsGui.ComputeResult'
        $result.Outputs['a'] | Should -Be @(6, 7)
        $result.Outputs.ContainsKey('b') | Should -BeFalse
        $result.Workgroups | Should -Be @(1, 1, 1)
        $result.CpuTimeMs | Should -BeGreaterThan 0
    }

    It 'runs GLSL, whose entry point is main' {
        $glsl = @'
#version 450
layout(local_size_x = 4) in;
layout(set = 0, binding = 0) buffer Values { float v[]; };
void main() { v[gl_GlobalInvocationID.x] += 1.0; }
'@
        (Invoke-GuiGpuCompute $glsl ([float[]](1, 2, 3, 4))) | Should -Be @(2, 3, 4, 5)
        Get-ErrorId { Invoke-GuiGpuCompute $glsl ([float[]](1)) -Entry other -ErrorAction Stop } | Should -Be 'GuiGpuEntry'
    }

    It 'refuses a shader that does not validate, a count of resources unlike its bindings, and a key naming none' {
        Get-ErrorId { Invoke-GuiGpuCompute 'fn broken(' ([float[]](1)) -ErrorAction Stop } | Should -Be 'WgpuShaderInvalid'
        $a = New-GuiGpuBuffer ([float[]](1))
        Get-ErrorId { Invoke-GuiGpuCompute $script:Double ($a, $a) -ErrorAction Stop } | Should -Be 'GuiGpuBindingCount'
        Get-ErrorId { Invoke-GuiGpuCompute $script:Double @{ missing = $a } -ErrorAction Stop } | Should -Be 'GuiGpuBinding'
        Get-ErrorId { Invoke-GuiGpuCompute $script:Double $a -Workgroups 1 -Invocations 1 -ErrorAction Stop } | Should -Be 'GuiGpuDispatchSize'
    }
}

Describe 'GPU textures in <Mode> mode' -ForEach $cases {
    BeforeAll {
        Reset-GuiSession $Mode
        $script:Sample = @'
@group(0) @binding(0) var picture: texture_2d<f32>;
@group(0) @binding(1) var look: sampler;
@group(0) @binding(2) var<storage, read_write> seen: array<f32>;
@compute @workgroup_size(1)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    var at = array<f32, 4>(0.375, 0.625, 1.375, 1.625);
    seen[id.x] = textureSampleLevel(picture, look, vec2<f32>(at[id.x], 0.5), 0.0).r;
}
'@
        # A PNG of 2 by 2 texels: red and green over blue and white.
        $script:QuartersPng = [Convert]::FromBase64String('iVBORw0KGgoAAAANSUhEUgAAAAIAAAACCAYAAABytg0kAAAAEklEQVR42mP4z8DwHwyBNBgAAEnICfcD2WTxAAAAAElFTkSuQmCC')
    }

    It 'makes a zeroed texture of a size and format under its label, and reads its texels back as bytes' {
        $t = New-GuiGpuTexture 4 2
        $t.GetType().FullName | Should -Be 'PwrsGui.GpuTexture'
        ($t.Width, $t.Height, $t.Format, $t.Label) | Should -Be @(4, 2, 'Rgba8Unorm', '4 x 2 Rgba8Unorm')
        $t.Adapter | Should -Be (Get-GuiGpuAdapter | Where-Object InUse | Select-Object -First 1).Name
        $texels = Read-GuiGpuTexture $t
        $texels.GetType().FullName | Should -Be 'System.Byte[]'
        $texels.Length | Should -Be 32
        @($texels | Where-Object { $_ -ne 0 }).Count | Should -Be 0
        (New-GuiGpuTexture 3 -Label row).Height | Should -Be 1
    }

    It 'writes and reads each family of formats as its own number type, and as bytes' {
        $halves = New-GuiGpuTexture 2 1 -Format Rg16Float -Data ([float[]](0.5, 1, 2, -4))
        $read = Read-GuiGpuTexture $halves
        $read.GetType().FullName | Should -Be 'System.Single[]'
        $read | Should -Be @(0.5, 1, 2, -4)
        (Read-GuiGpuTexture $halves -AsBytes) | Should -Be @(0, 56, 0, 60, 0, 64, 0, 196)
        $floats = New-GuiGpuTexture 2 2 -Format R32Float
        Write-GuiGpuTexture $floats (1, 2, 3, 4)
        (Read-GuiGpuTexture $floats) | Should -Be @(1, 2, 3, 4)
        $counts = New-GuiGpuTexture 2 1 -Format R32Uint -Data ([uint32[]](7, 4000000000))
        (Read-GuiGpuTexture $counts).GetType().FullName | Should -Be 'System.UInt32[]'
        (Read-GuiGpuTexture $counts) | Should -Be @(7, 4000000000)
        (Read-GuiGpuTexture (New-GuiGpuTexture 1 1 -Format R32Sint -Data (-5))) | Should -Be @(-5)
        (Read-GuiGpuTexture (New-GuiGpuTexture 1 1 -Format Bgra8Unorm -Data ([byte[]](1, 2, 3, 4)))) | Should -Be @(1, 2, 3, 4) -Because 'texels read back in the order they were written'
    }

    It 'refuses a side of 0, texels short of every one, a value outside the format''s type, and a removed texture' {
        Get-ErrorId { New-GuiGpuTexture 0 4 -ErrorAction Stop } | Should -Be 'GuiGpuTextureSize'
        Get-ErrorId { New-GuiGpuTexture 2 2 -Data ([byte[]](1, 2, 3)) -ErrorAction Stop } | Should -Be 'GuiGpuTextureSize'
        Get-ErrorId { New-GuiGpuTexture 1 1 -Format R32Uint -Data (-1) -ErrorAction Stop } | Should -Be 'WgpuOutOfRange'
        $t = New-GuiGpuTexture 1 1 -Data ([byte[]](1, 2, 3, 4))
        Get-ErrorId { Write-GuiGpuTexture $t ([byte[]](9)) -ErrorAction Stop } | Should -Be 'GuiGpuTextureSize'
        (Read-GuiGpuTexture $t) | Should -Be @(1, 2, 3, 4) -Because 'a refused write leaves the texels'
        Remove-GuiGpuTexture $t -Confirm:$false
        Get-ErrorId { Read-GuiGpuTexture $t -ErrorAction Stop } | Should -Be 'GuiGpuRemoved'
        Get-ErrorId { Write-GuiGpuTexture $t ([byte[]](1, 2, 3, 4)) -ErrorAction Stop } | Should -Be 'GuiGpuRemoved'
        { Remove-GuiGpuTexture $t -Confirm:$false -ErrorAction Stop } | Should -Not -Throw
    }

    It 'imports an image file as Rgba8Unorm, or Rgba8UnormSrgb with -Srgb, under its path, going on past a file it cannot read' {
        $folder = Join-Path ([IO.Path]::GetTempPath()) "pwrs-gui-test-images-$PID-$Mode"
        $null = New-Item -ItemType Directory -Path $folder -Force
        $png = Join-Path $folder 'quarters.png'
        $broken = Join-Path $folder 'broken.png'
        [IO.File]::WriteAllBytes($png, $script:QuartersPng)
        $t = Import-GuiGpuImage -LiteralPath $png
        ($t.Width, $t.Height, $t.Format, $t.Label) | Should -Be @(2, 2, 'Rgba8Unorm', $png)
        (Read-GuiGpuTexture $t) | Should -Be @(255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 255, 255)
        $lit = Import-GuiGpuImage $png -Srgb -Label lit
        ($lit.Format, $lit.Label) | Should -Be @('Rgba8UnormSrgb', 'lit')
        (Read-GuiGpuTexture $lit) | Should -Be (Read-GuiGpuTexture $t) -Because 'sRGB changes what a shader samples, not the bytes'
        Set-Content -LiteralPath $broken 'not an image'
        $loaded = @(Import-GuiGpuImage (Join-Path $folder '*.png') -ErrorAction SilentlyContinue -ErrorVariable failed)
        $loaded.Count | Should -Be 1 -Because 'the image after the broken file still loads'
        ($failed[0].FullyQualifiedErrorId -split ',')[0] | Should -Be 'GuiGpuImage'
        @(Get-ChildItem -LiteralPath $folder -Filter quarters.png | Import-GuiGpuImage).Count | Should -Be 1 -Because 'the files Get-ChildItem pipes bind to -LiteralPath'
        @([pscustomobject]@{ Path = $png } | Import-GuiGpuImage).Count | Should -Be 1 -Because 'an object with a Path binds to -Path'
        Get-ErrorId { Import-GuiGpuImage $png -Format R32Float -ErrorAction Stop } | Should -Be 'GuiGpuImageFormat'
        Get-ErrorId { Import-GuiGpuImage $png -Format Rgba8Unorm -Srgb -ErrorAction Stop } | Should -Be 'GuiGpuImageFormat'
        Get-ErrorId { Import-GuiGpuImage (Join-Path $folder 'missing.png') -ErrorAction Stop } | Should -Be 'GuiGpuImage'
        Remove-Item -LiteralPath $broken, $png
        Remove-Item -LiteralPath $folder
    }

    It 'takes buffers and textures from the pipeline' {
        $b = New-GuiGpuBuffer ([int[]](4, 5, 6))
        ($b | Read-GuiGpuBuffer) | Should -Be @(4, 5, 6)
        $b | Remove-GuiGpuBuffer -Confirm:$false
        Get-ErrorId { Read-GuiGpuBuffer $b -ErrorAction Stop } | Should -Be 'GuiGpuRemoved'
        $t = New-GuiGpuTexture 1 1 -Data ([byte[]](1, 2, 3, 4))
        ($t | Read-GuiGpuTexture) | Should -Be @(1, 2, 3, 4)
        $t | Remove-GuiGpuTexture -Confirm:$false
        Get-ErrorId { Read-GuiGpuTexture $t -ErrorAction Stop } | Should -Be 'GuiGpuRemoved'
    }

    It 'samples a texture by each filter and address mode in a compute run, the texture and sampler bound by name' {
        $picture = New-GuiGpuTexture 2 1 -Format R8Unorm -Data ([byte[]](0, 255))
        $nearest = New-GuiGpuSampler
        $nearest.GetType().FullName | Should -Be 'PwrsGui.GpuSampler'
        ($nearest.Filter, $nearest.AddressMode) | Should -Be @('Nearest', 'ClampToEdge')
        $nearest.Label | Should -BeLike 'sampler *'
        $seen = { param($look) Invoke-GuiGpuCompute $script:Sample @{ picture = $picture; look = $look; seen = [float[]]::new(4) } -Invocations 4 }
        (& $seen $nearest) | Should -Be @(0, 1, 1, 1)
        $linear = & $seen (New-GuiGpuSampler -Filter Linear)
        $off = @(0..3 | Where-Object { [math]::Abs($linear[$_] - @(0.25, 0.75, 1, 1)[$_]) -gt 2 / 255 })
        $off.Count | Should -Be 0 -Because "a linear sample weighs the two texels by a quarter and three quarters, within the device's filtering precision; it read $($linear -join ', ')"
        (& $seen (New-GuiGpuSampler -AddressMode Repeat)) | Should -Be @(0, 1, 0, 1)
        (& $seen (New-GuiGpuSampler -AddressMode MirrorRepeat -Label mirror)) | Should -Be @(0, 1, 1, 0)
    }
}

Describe 'GPU render pipelines in <Mode> mode' -ForEach $cases {
    BeforeAll {
        Reset-GuiSession $Mode
        $script:Fill = @'
@vertex fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    var corners = array<vec2<f32>, 3>(vec2<f32>(-1.0, -1.0), vec2<f32>(3.0, -1.0), vec2<f32>(-1.0, 3.0));
    return vec4<f32>(corners[i], 0.0, 1.0);
}
@fragment fn fs() -> @location(0) vec4<f32> { return vec4<f32>(1.0, 0.0, 0.0, 1.0); }
'@
        $script:Tinted = @'
struct Tint { color: vec4<f32> }
@group(0) @binding(0) var<uniform> tint: Tint;
@vertex fn vs(@location(0) position: vec2<f32>) -> @builtin(position) vec4<f32> { return vec4<f32>(position, 0.0, 1.0); }
@fragment fn fs() -> @location(0) vec4<f32> { return tint.color; }
'@
    }

    It 'builds a pipeline that says what its shader reads, with the depth and samples it was given' {
        $p = New-GuiGpuRenderPipeline $script:Tinted
        $p.GetType().FullName | Should -Be 'PwrsGui.GpuRenderPipeline'
        $p.VertexInputs | Should -Be @('position')
        $p.Bindings | Should -Be @('tint')
        $p.SampleCount | Should -Be 1
        $p.DepthFormat | Should -BeNullOrEmpty
        $p.VertexEntryPoint | Should -BeNullOrEmpty
        $p.Label | Should -BeLike 'render pipeline *'
        $p.Adapter | Should -Be (Get-GuiGpuAdapter | Where-Object InUse | Select-Object -First 1).Name
        $deep = New-GuiGpuRenderPipeline $script:Fill -Depth -SampleCount 4 -VertexEntry vs -Label deep -Topology TriangleStrip -CullMode Back
        $deep.DepthFormat | Should -Be 'Depth32Float'
        $deep.SampleCount | Should -Be 4
        $deep.VertexEntryPoint | Should -Be 'vs'
        $deep.Label | Should -Be 'deep'
        @($deep.VertexInputs).Count | Should -Be 0
        (New-GuiGpuRenderPipeline $script:Fill -DepthFormat Stencil8 -StencilPass Replace -BackStencilFail Zero -StencilWriteMask 1).DepthFormat | Should -Be 'Stencil8'
        (New-GuiGpuRenderPipeline $script:Fill -Depth -DepthFormat Depth24Plus -DepthCompare GreaterEqual -DepthWrite:$false -DepthBias 2 -DepthBiasSlopeScale 1.5).DepthFormat | Should -Be 'Depth24Plus' -Because 'a format given wins over -Depth'
    }

    It 'refuses GLSL, WGSL that does not validate, and a custom blend''s part without -Blend Custom' {
        Get-ErrorId { New-GuiGpuRenderPipeline "#version 450`nvoid main() {}" -ErrorAction Stop } | Should -Be 'GuiGpuShaderLanguage'
        Get-ErrorId { New-GuiGpuRenderPipeline 'fn broken(' -ErrorAction Stop } | Should -Be 'WgpuShaderInvalid'
        Get-ErrorId { New-GuiGpuRenderPipeline $script:Fill -Blend Alpha -BlendSource One -ErrorAction Stop } | Should -Be 'WgpuBlend'
    }

    It 'refuses a depth test without a format that holds depth, a stencil test without one that holds stencil, and a depth bias off triangles' {
        Get-ErrorId { New-GuiGpuRenderPipeline $script:Fill -DepthCompare Greater -ErrorAction Stop } | Should -Be 'WgpuDepthTest'
        Get-ErrorId { New-GuiGpuRenderPipeline $script:Fill -DepthFormat Stencil8 -DepthWrite:$false -ErrorAction Stop } | Should -Be 'WgpuDepthTest'
        Get-ErrorId { New-GuiGpuRenderPipeline $script:Fill -Depth -StencilPass Replace -ErrorAction Stop } | Should -Be 'WgpuStencil'
        Get-ErrorId { New-GuiGpuRenderPipeline $script:Fill -BackStencilCompare Never -ErrorAction Stop } | Should -Be 'WgpuStencil'
        Get-ErrorId { New-GuiGpuRenderPipeline $script:Fill -Topology PointList -Depth -DepthBiasClamp 1 -ErrorAction Stop } | Should -Be 'WgpuDepthBias'
    }

    It 'takes its shader text from the pipeline' {
        ($script:Tinted | New-GuiGpuRenderPipeline).PSObject.TypeNames[0] | Should -Be 'PwrsGui.GpuRenderPipeline'
    }

    It 'describes a GpuSurface by its name, refusing a side of 0 points' {
        $s = New-GuiGpuSurface View -Width 320 -Height 200
        $s.Kind | Should -Be 'GpuSurface'
        $s.Name | Should -Be 'View'
        $s.Properties.Width | Should -Be 320
        $s.Properties.Height | Should -Be 200
        Get-ErrorId { New-GuiGpuSurface View -Width 0 -ErrorAction Stop } | Should -Be 'GuiBadSize'
    }

    It 'refuses a draw it cannot read before it looks for the window, and a surface outside a handler without -Window' {
        $key = { Set-GuiGpuSurface View -Window 1 -Draw @{ Shader = $script:Fill; VertexCount = 3 }, @{ Shader = $script:Fill; Tint = 1 } -ErrorAction Stop } | Should -Throw -ErrorId 'GuiGpuDrawKey*' -PassThru
        $key.Exception.Message | Should -BeLike 'draw 1: a draw takes Shader, *; it was given Tint'
        Get-ErrorId { Set-GuiGpuSurface View -Window 1 -Draw @{ Shader = $script:Fill; Topology = 'Triangles' } -ErrorAction Stop } | Should -Be 'GuiGpuDrawValue'
        Get-ErrorId { Set-GuiGpuSurface View -Window 1 -Draw 'not a hashtable' -ErrorAction Stop } | Should -Be 'GuiGpuDrawKey'
        Get-ErrorId { Set-GuiGpuSurface View -VertexCount 3 -Window 1 -ErrorAction Stop } | Should -Be 'GuiGpuNoShader'
        Get-ErrorId { Set-GuiGpuSurface View $script:Fill -ClearColor 1, 2 -Window 1 -ErrorAction Stop } | Should -Be 'GuiGpuClearColor'
        $far = { Set-GuiGpuSurface View $script:Fill -Depth -ClearDepth 2 -Window 1 -ErrorAction Stop } | Should -Throw -ErrorId 'GuiGpuClearDepth*' -PassThru
        $far.Exception.Message | Should -Be '-ClearDepth is a depth from 0 to 1, and 2 is not'
        Get-ErrorId { Set-GuiGpuSurface View -Window 1 -Draw @{ Shader = $script:Fill; DepthFormat = 'Depth8' } -ErrorAction Stop } | Should -Be 'GuiGpuDrawValue'
        Get-ErrorId { Set-GuiGpuSurface View -Window 1 -Draw @{ Shader = $script:Fill; StencilCompare = 'Sometimes' } -ErrorAction Stop } | Should -Be 'GuiGpuDrawValue'
        Get-ErrorId { Set-GuiGpuSurface View -Window 1 -Draw @{ Shader = $script:Fill; StencilReference = -1 } -ErrorAction Stop } | Should -Be 'GuiGpuDrawValue'
        Get-ErrorId { Set-GuiGpuSurface View -StencilReference 1 -Window 1 -ErrorAction Stop } | Should -Be 'GuiGpuNoShader' -Because 'a reference describes a draw, which takes a shader'
        Get-ErrorId { Set-GuiGpuSurface View $script:Fill -VertexCount 3 -ErrorAction Stop } | Should -Be 'GuiNoWindow'
        Get-ErrorId { Save-GuiGpuSurface View 'frame.png' -ErrorAction Stop } | Should -Be 'GuiNoWindow'
    }
}

# Each case opens a window, so these belong on an idle desktop or a guest's
# display. They run in windows of both renderers: an HTML window draws on
# its page's WebGPU, or on WebGL 2 where the engine offers no WebGPU, which
# runs no compute pass.
Describe 'A GPU surface in <Mode> mode drawn with <Renderer>' -ForEach $surfaceCases {
    BeforeAll {
        Reset-GuiSession $Mode $Renderer
        $script:Fill = @'
@vertex fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    var corners = array<vec2<f32>, 3>(vec2<f32>(-1.0, -1.0), vec2<f32>(3.0, -1.0), vec2<f32>(-1.0, 3.0));
    return vec4<f32>(corners[i], 0.0, 1.0);
}
@fragment fn fs() -> @location(0) vec4<f32> { return vec4<f32>(1.0, 0.0, 0.0, 1.0); }
'@
        $script:Tinted = @'
struct Tint { color: vec4<f32> }
@group(0) @binding(0) var<uniform> tint: Tint;
@vertex fn vs(@location(0) position: vec2<f32>) -> @builtin(position) vec4<f32> { return vec4<f32>(position, 0.0, 1.0); }
@fragment fn fs() -> @location(0) vec4<f32> { return tint.color; }
'@
        $script:Framed = @'
struct Frame { time: f32, delta: f32, resolution: vec2<f32>, pointer: vec2<f32>, view: mat4x4<f32>, projection: mat4x4<f32>, tint: vec4<f32> }
@group(1) @binding(0) var<uniform> frame: Frame;
@vertex fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    var corners = array<vec2<f32>, 3>(vec2<f32>(-1.0, -1.0), vec2<f32>(3.0, -1.0), vec2<f32>(-1.0, 3.0));
    return vec4<f32>(corners[i], 0.0, 1.0);
}
@fragment fn fs(@builtin(position) at: vec4<f32>) -> @location(0) vec4<f32> {
    if (at.x < frame.resolution.x / 2.0) { return frame.tint; }
    return vec4<f32>(0.0, 0.0, 1.0, 1.0);
}
'@
        $script:Viewed = @'
struct Frame { time: f32, delta: f32, resolution: vec2<f32>, pointer: vec2<f32>, view: mat4x4<f32>, projection: mat4x4<f32> }
@group(1) @binding(0) var<uniform> frame: Frame;
@vertex fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    var corners = array<vec2<f32>, 3>(vec2<f32>(-1.0, -1.0), vec2<f32>(3.0, -1.0), vec2<f32>(-1.0, 3.0));
    return vec4<f32>(corners[i], 0.0, 1.0);
}
@fragment fn fs() -> @location(0) vec4<f32> {
    let far = select(0.0, 1.0, frame.view[3][2] < -4.0);
    let narrow = select(0.0, 1.0, frame.projection[1][1] > 2.0);
    return vec4<f32>(0.0, far, narrow, 1.0);
}
'@
        # Red in the dark theme and green in the light, from the frame
        # uniform's byte 24, and in blue a quarter of its byte 28, the
        # display's pixels per point.
        $script:Themed = @'
struct Frame { time: f32, delta: f32, resolution: vec2<f32>, pointer: vec2<f32>, dark: f32, scale: f32, view: mat4x4<f32>, projection: mat4x4<f32> }
@group(1) @binding(0) var<uniform> frame: Frame;
@vertex fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    var corners = array<vec2<f32>, 3>(vec2<f32>(-1.0, -1.0), vec2<f32>(3.0, -1.0), vec2<f32>(-1.0, 3.0));
    return vec4<f32>(corners[i], 0.0, 1.0);
}
@fragment fn fs() -> @location(0) vec4<f32> { return vec4<f32>(frame.dark, 1.0 - frame.dark, frame.scale / 4.0, 1.0); }
'@
        $script:GreenPng = [Convert]::FromBase64String('iVBORw0KGgoAAAANSUhEUgAAAAgAAAAECAIAAAA8r+mnAAAAEElEQVR4nGNg+M+AHVFPAgA6fh/hYeiK3wAAAABJRU5ErkJggg==')
        $script:Textured = @'
@group(0) @binding(0) var picture: texture_2d<f32>;
@group(0) @binding(1) var look: sampler;
struct Out { @builtin(position) at: vec4<f32>, @location(0) uv: vec2<f32> }
@vertex fn vs(@builtin(vertex_index) i: u32) -> Out {
    var corners = array<vec2<f32>, 3>(vec2<f32>(-1.0, -1.0), vec2<f32>(3.0, -1.0), vec2<f32>(-1.0, 3.0));
    let c = corners[i];
    return Out(vec4<f32>(c, 0.0, 1.0), vec2<f32>((c.x + 1.0) / 2.0, (1.0 - c.y) / 2.0));
}
@fragment fn fs(in: Out) -> @location(0) vec4<f32> { return textureSample(picture, look, in.uv); }
'@
        # The tint compute passes wrote while their count is 1, blue once it
        # passes 4, and red otherwise. It reads the frame uniform, so its
        # surface draws every frame in either renderer.
        $script:Passed = @'
struct Frame { time: f32, delta: f32, resolution: vec2<f32>, pointer: vec2<f32>, view: mat4x4<f32>, projection: mat4x4<f32> }
@group(1) @binding(0) var<uniform> frame: Frame;
@group(0) @binding(0) var<storage, read> tint: array<f32, 4>;
@group(0) @binding(1) var<storage, read> count: array<u32>;
@vertex fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    var corners = array<vec2<f32>, 3>(vec2<f32>(-1.0, -1.0), vec2<f32>(3.0, -1.0), vec2<f32>(-1.0, 3.0));
    return vec4<f32>(corners[i], 0.0, 1.0);
}
@fragment fn fs() -> @location(0) vec4<f32> {
    let n = count[0];
    if (n > 4u) { return vec4<f32>(0.0, 0.0, 1.0, 1.0); }
    if (n == 1u && frame.time >= 0.0) { return vec4<f32>(tint[0], tint[1], tint[2], tint[3]); }
    return vec4<f32>(1.0, 0.0, 0.0, 1.0);
}
'@
        $script:Blue = @'
@group(0) @binding(0) var<storage, read_write> tint: array<f32, 4>;
@compute @workgroup_size(1) fn main() { tint = array<f32, 4>(0.0, 0.0, 1.0, 1.0); }
'@
        # A triangle over the whole surface at the depth its constant depth
        # names, red at red 1 and blue at red 0.
        $script:Deep = @'
override depth: f32 = 0.5;
override red: f32 = 1.0;
@vertex fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    var corners = array<vec2<f32>, 3>(vec2<f32>(-1.0, -1.0), vec2<f32>(3.0, -1.0), vec2<f32>(-1.0, 3.0));
    return vec4<f32>(corners[i], depth, 1.0);
}
@fragment fn fs() -> @location(0) vec4<f32> { return vec4<f32>(red, 0.0, 1.0 - red, 1.0); }
'@
        # A green triangle over the surface's left half.
        $script:LeftHalf = @'
@vertex fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    var corners = array<vec2<f32>, 3>(vec2<f32>(-1.0, -1.0), vec2<f32>(0.0, -1.0), vec2<f32>(-1.0, 3.0));
    return vec4<f32>(corners[i], 0.0, 1.0);
}
@fragment fn fs() -> @location(0) vec4<f32> { return vec4<f32>(0.0, 1.0, 0.0, 1.0); }
'@
        # Red triangles at the positions in its vertex input.
        $script:Placed = @'
@vertex fn vs(@location(0) position: vec2<f32>) -> @builtin(position) vec4<f32> { return vec4<f32>(position, 0.0, 1.0); }
@fragment fn fs() -> @location(0) vec4<f32> { return vec4<f32>(1.0, 0.0, 0.0, 1.0); }
'@

        # The red, green and blue of the texel halfway down a snapshot, $Across
        # of the way along it.
        function Texel($shot, [double]$Across = 0.5) {
            if (-not $shot) { return @() }
            $at = ([int][math]::Floor($shot.Height / 2) * $shot.Width + [int][math]::Floor(($shot.Width - 1) * $Across)) * 4
            @($shot.Pixels[$at..($at + 2)])
        }
        function Near($rgb, $want) { $rgb.Count -eq 3 -and @(0..2 | Where-Object { [math]::Abs([int]$rgb[$_] - $want[$_]) -gt 8 }).Count -eq 0 }
        function Shot($Window, [string]$Name) { Get-GuiSnapshot -Window $Window.Id -Name $Name -Pixels -ErrorAction Ignore }
        # Whether one snapshot of the surface View shows $Left a quarter of
        # the way across and $Right three quarters of the way.
        function Halves($Window, $Left, $Right) { $shot = Shot $Window View; $shot -and (Near (Texel $shot 0.25) $Left) -and (Near (Texel $shot 0.75) $Right) }

        # The next event of the widget named $Name for which $Wanted is true,
        # passing over any other, as a move the real pointer makes over the
        # window; nothing when none comes within 20 seconds.
        function Next($Window, [string]$Name, [scriptblock]$Wanted) {
            while ($null -ne ($e = Wait-GuiEvent -Window $Window.Id -Name $Name -Timeout 20)) {
                if ($e | Where-Object $Wanted) { return $e }
            }
        }
    }

    It 'shows its window''s background until it has draws, then draws them each frame, and saves the draws given before as a PNG' {
        $w = Show-GuiWindow -NoWait -Title 'PWRS-GUI test surface' -Renderer $Renderer { GpuSurface View -Width 64 -Height 64; Image -Bytes $script:GreenPng -Width 64 -Height 64 -Name Pic }
        $null = Wait-GuiWindow 'PWRS-GUI test surface'
        # The window's top left pixel lies in its margin, in its background.
        $background = { $whole = Get-GuiSnapshot -Window $w.Id -Pixels; @($whole.Pixels[0..2]) }
        Wait-Until { Near (Texel (Shot $w View)) (& $background) } 'the surface showing its window''s background'
        Set-GuiGpuSurface View $script:Fill -VertexCount 3 -Window $w.Id
        Wait-Until { Near (Texel (Shot $w View)) @(255, 0, 0) } 'the surface drawing red'
        $whole = New-GuiGpuBuffer ([float[]](-1, -1, 3, -1, -1, 3))
        $blue = New-GuiGpuBuffer ([float[]](0, 0, 1, 1))
        $file = Join-Path ([IO.Path]::GetTempPath()) "pwrs-gui-test-surface-$PID-$Mode-$Renderer.png"
        Set-GuiGpuSurface View $script:Tinted -Vertex $whole -Binding $blue -VertexCount 3 -Window $w.Id
        Save-GuiGpuSurface View $file -Window $w.Id
        $bytes = [IO.File]::ReadAllBytes($file)
        [BitConverter]::ToString($bytes, 0, 8) | Should -Be '89-50-4E-47-0D-0A-1A-0A'
        $width = (([int]$bytes[16] * 256 + $bytes[17]) * 256 + $bytes[18]) * 256 + $bytes[19]
        $height = (([int]$bytes[20] * 256 + $bytes[21]) * 256 + $bytes[22]) * 256 + $bytes[23]
        $width | Should -BeGreaterThan 0
        $height | Should -Be $width -Because 'the surface is square'
        Update-GuiWidget -Window $w.Id -Name Pic -Property @{ Path = $file }
        Wait-Until { Near (Texel (Shot $w Pic)) @(0, 0, 255) } 'the saved PNG, of the blue draw given just before the save, drawn as an Image'
        Close-GuiWindow -Window $w.Id
        (Wait-GuiEvent -Window $w.Id -Timeout 20).Kind | Should -Be 'Close'
        Remove-Item -LiteralPath $file
    }

    It 'takes the window its surface and camera commands act on from the pipeline' {
        $w = Show-GuiWindow -NoWait -Title 'PWRS-GUI test surface piped' -Renderer $Renderer { GpuSurface View -Width 64 -Height 64 }
        $null = Wait-GuiWindow 'PWRS-GUI test surface piped'
        $w | Set-GuiGpuSurface View $script:Fill -VertexCount 3
        Wait-Until { Near (Texel (Shot $w View)) @(255, 0, 0) } 'the piped draws drawing red'
        $file = Join-Path ([IO.Path]::GetTempPath()) "pwrs-gui-test-surface-piped-$PID-$Mode-$Renderer.png"
        $w | Save-GuiGpuSurface View $file
        [BitConverter]::ToString([IO.File]::ReadAllBytes($file), 0, 8) | Should -Be '89-50-4E-47-0D-0A-1A-0A'
        Remove-Item -LiteralPath $file
        $w | Set-GuiGpuCamera View -Distance 2
        ($w | Get-GuiGpuCamera View).Distance | Should -Be 2
        $w | Remove-GuiGpuCamera View -Confirm:$false
        $w | Get-GuiGpuCamera View | Should -BeNullOrEmpty
        Close-GuiWindow -Window $w.Id
        (Wait-GuiEvent -Window $w.Id -Timeout 20).Kind | Should -Be 'Close'
    }

    It 'replaces all its draws at once, still using a buffer removed while it draws with it until they change' {
        $w = Show-GuiWindow -NoWait -Title 'PWRS-GUI test surface draws' -Renderer $Renderer { GpuSurface View -Width 64 -Height 64 }
        $null = Wait-GuiWindow 'PWRS-GUI test surface draws'
        $left = New-GuiGpuBuffer ([float[]](-1, -1, 0, -1, -1, 3))
        $green = New-GuiGpuBuffer ([float[]](0, 1, 0, 1))
        Set-GuiGpuSurface View -Window $w.Id -Draw @{ Shader = $script:Fill; VertexCount = 3 }, @{ Shader = $script:Tinted; Vertex = @{ position = $left }; Binding = @{ tint = $green }; Index = [uint16[]](0, 1, 2) }
        Wait-Until { Near (Texel (Shot $w View) 0.25) @(0, 255, 0) } 'the left half drawn green over the red'
        Near (Texel (Shot $w View) 0.75) @(255, 0, 0) | Should -BeTrue -Because 'the right half stays red'
        Remove-GuiGpuBuffer $left -Confirm:$false
        $left = $null
        [GC]::Collect()
        [GC]::WaitForPendingFinalizers()
        Near (Texel (Shot $w View) 0.25) @(0, 255, 0) | Should -BeTrue -Because 'the surface still uses a buffer removed while it draws with it'
        Set-GuiGpuSurface View -ClearColor 0, 0, 1 -Window $w.Id
        Wait-Until { Near (Texel (Shot $w View) 0.25) @(0, 0, 255) } 'the draws replaced by the clear color alone'
        Near (Texel (Shot $w View) 0.75) @(0, 0, 255) | Should -BeTrue
        Close-GuiWindow -Window $w.Id
        (Wait-GuiEvent -Window $w.Id -Timeout 20).Kind | Should -Be 'Close'
    }

    It 'draws arrays given for a vertex input and a binding as the shader declares them, counting the vertices from the first input' {
        $w = Show-GuiWindow -NoWait -Title 'PWRS-GUI test surface arrays' -Renderer $Renderer { GpuSurface View -Width 64 -Height 64 }
        $null = Wait-GuiWindow 'PWRS-GUI test surface arrays'
        Set-GuiGpuSurface View $script:Tinted -Vertex @{ position = [double[]](-1, -1, 0, -1, -1, 3) } -Binding (0, 1, 0, 1) -ClearColor 1, 0, 0 -Window $w.Id
        Wait-Until { Near (Texel (Shot $w View) 0.25) @(0, 255, 0) } 'the left half drawn green from arrays'
        Near (Texel (Shot $w View) 0.75) @(255, 0, 0) | Should -BeTrue -Because 'six values of a vec2<f32> input count three vertices, and the right half stays the clear color'
        Close-GuiWindow -Window $w.Id
        (Wait-GuiEvent -Window $w.Id -Timeout 20).Kind | Should -Be 'Close'
    }

    It 'gives a draw declaring the frame uniform the surface''s header and the session''s fields after it, refusing a uniform no draw reads and a binding in its place' {
        $w = Show-GuiWindow -NoWait -Title 'PWRS-GUI test surface frame' -Renderer $Renderer { GpuSurface View -Width 64 -Height 64 }
        $null = Wait-GuiWindow 'PWRS-GUI test surface frame'
        Set-GuiGpuSurface View $script:Framed -VertexCount 3 -Uniform 0, 1, 0, 1 -Window $w.Id
        Wait-Until { Near (Texel (Shot $w View) 0.25) @(0, 255, 0) } 'the left half drawn in the green the uniform gives'
        Near (Texel (Shot $w View) 0.75) @(0, 0, 255) | Should -BeTrue -Because 'the shader splits the surface at half the resolution its header gives'
        Get-ErrorId { Set-GuiGpuSurface View $script:Fill -VertexCount 3 -Uniform 0, 1, 0, 1 -Window $w.Id -ErrorAction Stop } | Should -Be 'GuiGpuUniform'
        Get-ErrorId { Set-GuiGpuSurface View $script:Framed -VertexCount 3 -Binding @{ frame = (New-GuiGpuBuffer -Length 44) } -Window $w.Id -ErrorAction Stop } | Should -Be 'GuiGpuBinding'
        Close-GuiWindow -Window $w.Id
        (Wait-GuiEvent -Window $w.Id -Timeout 20).Kind | Should -Be 'Close'
    }

    It 'gives a draw the window''s <Theme> theme at byte 24 of the frame uniform and the display''s pixels per point at byte 28' -ForEach @(
        @{ Theme = 'Dark'; Dark = 255 }
        @{ Theme = 'Light'; Dark = 0 }
    ) {
        Set-GuiOption -Theme $Theme
        try {
            $w = Show-GuiWindow -NoWait -Title "PWRS-GUI test surface theme $Theme" -Renderer $Renderer { GpuSurface View -Width 64 -Height 64 }
            $null = Wait-GuiWindow "PWRS-GUI test surface theme $Theme"
            Set-GuiGpuSurface View $script:Themed -VertexCount 3 -Window $w.Id
            # The surface is 64 points across, so its snapshot's width in
            # pixels over 64 is the pixels per point the shader reads.
            Wait-Until { $shot = Shot $w View; $shot -and (Near (Texel $shot) @($Dark, (255 - $Dark), [int][math]::Round(255 * $shot.Width / 64 / 4))) } "the surface drawing the $Theme theme's color and a quarter of its pixels per point"
            Close-GuiWindow -Window $w.Id
            (Wait-GuiEvent -Window $w.Id -Timeout 20).Kind | Should -Be 'Close'
        } finally {
            Set-GuiOption -Theme System
        }
    }

    It 'fills the frame uniform''s view and projection from a camera set, read back, switched and removed by its own commands' {
        $w = Show-GuiWindow -NoWait -Title 'PWRS-GUI test surface camera' -Renderer $Renderer { GpuSurface View -Width 64 -Height 64 }
        $null = Wait-GuiWindow 'PWRS-GUI test surface camera'
        Set-GuiGpuSurface View $script:Viewed -VertexCount 3 -Window $w.Id
        Wait-Until { Near (Texel (Shot $w View)) @(0, 0, 0) } 'identity view and projection without a camera'
        Get-GuiGpuCamera View -Window $w.Id | Should -BeNullOrEmpty
        $camera = Set-GuiGpuCamera View -Window $w.Id -PassThru
        $camera.GetType().FullName | Should -Be 'PwrsGui.GpuCamera'
        $camera.Eye | Should -Be @(0, 0, 5)
        $camera.FieldOfView | Should -Be 50
        $camera.Controls | Should -Be 'Default'
        $camera.Bindings['Left'] | Should -Be 'Orbit'
        $camera.View.Count | Should -Be 16
        Wait-Until { Near (Texel (Shot $w View)) @(0, 255, 255) } 'the default camera, 5 units back with a 50 degree field of view'
        Set-GuiGpuCamera View -Distance 2 -Window $w.Id
        Wait-Until { Near (Texel (Shot $w View)) @(0, 0, 255) } 'the camera moved in to 2 units'
        Set-GuiGpuCamera View -Controls Blender -Window $w.Id
        $switched = Get-GuiGpuCamera View -Window $w.Id
        ($switched.Controls, $switched.Distance) | Should -Be @('Blender', 2) -Because 'switching the controls leaves the pose as it is'
        $switched.Bindings['Shift+Middle'] | Should -Be 'Pan'
        Set-GuiGpuCamera View -Bindings @{ 'shift+left' = 'pan'; Wheel = 'Zoom' } -Window $w.Id
        $custom = Get-GuiGpuCamera View -Window $w.Id
        $custom.Controls | Should -Be 'Custom'
        $custom.Bindings['shift+LEFT'] | Should -Be 'Pan' -Because 'the bindings are looked up without regard to case'
        $custom.Bindings.Count | Should -Be 2
        Get-ErrorId { Set-GuiGpuCamera View -Target 1, 2 -Window $w.Id -ErrorAction Stop } | Should -Be 'GuiGpuCamera'
        Get-ErrorId { Set-GuiGpuCamera View -Distance 0 -Window $w.Id -ErrorAction Stop } | Should -Be 'GuiGpuCamera'
        Get-ErrorId { Set-GuiGpuCamera View -Controls Custom -Window $w.Id -ErrorAction Stop } | Should -Be 'GuiGpuCamera'
        Get-ErrorId { Set-GuiGpuCamera View -Bindings @{ 'Meta+Left' = 'Pan' } -Window $w.Id -ErrorAction Stop } | Should -Be 'GuiGpuCamera'
        (Get-GuiGpuCamera View -Window $w.Id).Distance | Should -Be 2 -Because 'a refused change leaves the camera'
        Remove-GuiGpuCamera View -Window $w.Id
        Get-GuiGpuCamera View -Window $w.Id | Should -BeNullOrEmpty
        Wait-Until { Near (Texel (Shot $w View)) @(0, 0, 0) } 'identity view and projection again once the camera is removed'
        Close-GuiWindow -Window $w.Id
        (Wait-GuiEvent -Window $w.Id -Timeout 20).Kind | Should -Be 'Close'
    }

    It 'reports presses, a release wherever it is, moves and wheel turns, with where in pixels, fractions and scale, and the keys pressed once it has the focus' {
        $w = Show-GuiWindow -NoWait -Title 'PWRS-GUI test surface events' -Renderer $Renderer { GpuSurface View -Width 64 -Height 64 -OnPointerDown {} -OnPointerUp {} -OnPointerMove {} -OnWheel {} -OnKey {}; GpuSurface Quiet -Width 16 -Height 16 }
        $null = Wait-GuiWindow 'PWRS-GUI test surface events'
        Wait-Until { Shot $w View } 'the surface drawn, so a test event has a place to put the pointer on'
        Invoke-GuiTestEvent -Window $w.Id -Name View -Event PointerDown -Value @{ U = 0.25; V = 0.5; Button = 'Right'; Shift = $true }
        $down = Next $w View { $_.Kind -eq 'PointerDown' }
        ($down.Value.U, $down.Value.V, $down.Value.Button, $down.Value.Shift, $down.Value.Ctrl) | Should -Be @(0.25, 0.5, 'Right', $true, $false)
        $down.Value.PixelsPerPoint | Should -BeGreaterThan 0
        [math]::Abs($down.Value.X - 16 * $down.Value.PixelsPerPoint) | Should -BeLessThan 1 -Because 'a quarter of 64 points across, in physical pixels'
        Invoke-GuiTestEvent -Window $w.Id -Name View -Event PointerMove -Value @{ U = 0.75; V = 0.25 }
        $move = Next $w View { $_.Kind -eq 'PointerMove' -and $_.Value.U -eq 0.75 }
        $move.Value.V | Should -Be 0.25
        $move.Value.Button | Should -BeNullOrEmpty
        Invoke-GuiTestEvent -Window $w.Id -Name View -Event PointerUp -Value @{ U = 1.5; V = 0.5; Button = 'Right' }
        $up = Next $w View { $_.Kind -eq 'PointerUp' }
        $up.Value.U | Should -Be 1.5 -Because 'a release is reported wherever the pointer is'
        Invoke-GuiTestEvent -Window $w.Id -Name View -Event Wheel -Value @{ U = 0.5; V = 0.5; Notches = -2 }
        $wheel = Next $w View { $_.Kind -eq 'Wheel' }
        $wheel.Value.Notches | Should -Be -2
        $wheel.Value.Button | Should -BeNullOrEmpty
        Invoke-GuiTestEvent -Window $w.Id -Name View -Event Key -Value 'ctrl+k'
        (Next $w View { $_.Kind -eq 'Key' }).Value | Should -Be 'Ctrl+K'
        Get-ErrorId { Invoke-GuiTestEvent -Window $w.Id -Name Quiet -Event PointerDown -Value @{ U = 0; V = 0 } -ErrorAction Stop } | Should -Not -Be 'no error' -Because 'a surface without -OnPointerDown raises none'
        Close-GuiWindow -Window $w.Id
        (Next $w View { $_.Kind -eq 'Close' }).Kind | Should -Be 'Close'
    }

    It 'says what HTML pages draw surfaces with once one of them has drawn one, and nothing while no HTML renderer runs' {
        Reset-GuiSession $Mode $Renderer
        $w = Show-GuiWindow -NoWait -Title 'PWRS-GUI test surface graphics' -Renderer $Renderer { GpuSurface View -Width 32 -Height 32 }
        $null = Wait-GuiWindow 'PWRS-GUI test surface graphics'
        (Get-GuiOption).WebGpu | Should -BeNullOrEmpty -Because 'no page has drawn a surface yet'
        Set-GuiGpuSurface View -ClearColor 0, 0, 1 -Window $w.Id
        $graphics = (Get-GuiOption).WebGpu
        if ($Renderer -eq 'Egui') {
            $graphics | Should -BeNullOrEmpty -Because 'egui draws its surfaces on its own device, and no HTML renderer runs'
        } elseif ($IsWindows -or $PSVersionTable.PSEdition -eq 'Desktop') {
            $graphics | Should -Be 'WebGPU' -Because 'WebView2 on the project''s Windows hosts offers WebGPU'
        } else {
            $graphics | Should -BeIn 'WebGPU', 'WebGL2'
        }
        Close-GuiWindow -Window $w.Id
        (Wait-GuiEvent -Window $w.Id -Timeout 20).Kind | Should -Be 'Close'
    }

    It 'draws in a later frame what a write or a compute run puts in a buffer it draws with' {
        $w = Show-GuiWindow -NoWait -Title 'PWRS-GUI test surface writes' -Renderer $Renderer { GpuSurface View -Width 64 -Height 64 }
        $null = Wait-GuiWindow 'PWRS-GUI test surface writes'
        $whole = New-GuiGpuBuffer ([float[]](-1, -1, 3, -1, -1, 3))
        $tint = New-GuiGpuBuffer ([float[]](0, 1, 0, 1))
        Set-GuiGpuSurface View $script:Tinted -Vertex $whole -Binding $tint -VertexCount 3 -Window $w.Id
        Wait-Until { Near (Texel (Shot $w View)) @(0, 255, 0) } 'the surface drawing the green in the buffer'
        Write-GuiGpuBuffer $tint ([float[]](1, 0, 0, 1))
        Wait-Until { Near (Texel (Shot $w View)) @(255, 0, 0) } 'the red written into the buffer drawn'
        Invoke-GuiGpuCompute $script:Blue -Binding $tint -Workgroups 1
        Wait-Until { Near (Texel (Shot $w View)) @(0, 0, 255) } 'the blue a compute run put in the buffer drawn'
        Close-GuiWindow -Window $w.Id
        (Wait-GuiEvent -Window $w.Id -Timeout 20).Kind | Should -Be 'Close'
    }

    It 'samples a texture of the table through a sampler, both bound by name, and draws a write to the texture' {
        $w = Show-GuiWindow -NoWait -Title 'PWRS-GUI test surface texture' -Renderer $Renderer { GpuSurface View -Width 64 -Height 64 }
        $null = Wait-GuiWindow 'PWRS-GUI test surface texture'
        $picture = New-GuiGpuTexture 2 1 -Data ([byte[]](255, 0, 0, 255, 0, 0, 255, 255))
        Set-GuiGpuSurface View $script:Textured -VertexCount 3 -Binding @{ picture = $picture; look = (New-GuiGpuSampler) } -Window $w.Id
        Wait-Until { Near (Texel (Shot $w View) 0.25) @(255, 0, 0) } 'the left half drawn from the red texel'
        Near (Texel (Shot $w View) 0.75) @(0, 0, 255) | Should -BeTrue -Because 'the right half samples the blue texel'
        Write-GuiGpuTexture $picture ([byte[]](255, 0, 0, 255, 0, 255, 0, 255))
        Wait-Until { Near (Texel (Shot $w View) 0.75) @(0, 255, 0) } 'the texel written green drawn in a later frame'
        Close-GuiWindow -Window $w.Id
        (Wait-GuiEvent -Window $w.Id -Timeout 20).Kind | Should -Be 'Close'
    }

    It 'runs its compute passes before its draws, once or before every frame, the draws reading what they wrote, or refuses them on a page without WebGPU' {
        $w = Show-GuiWindow -NoWait -Title 'PWRS-GUI test surface passes' -Renderer $Renderer { GpuSurface View -Width 64 -Height 64 }
        $null = Wait-GuiWindow 'PWRS-GUI test surface passes'
        $tint = New-GuiGpuBuffer -Length 4
        $count = New-GuiGpuBuffer -Length 1 -Type UInt32
        $paint = @'
@group(0) @binding(0) var<storage, read_write> tint: array<f32, 4>;
@compute @workgroup_size(1) fn main() { tint = array<f32, 4>(0.0, 1.0, 0.0, 1.0); }
'@
        $tick = @'
@group(0) @binding(0) var<storage, read_write> count: array<u32>;
@compute @workgroup_size(1) fn main() { count[0] = count[0] + 1u; }
'@
        Get-ErrorId { Set-GuiGpuSurface View -Compute @{ Shader = $tick; Often = 1 } -Window $w.Id -ErrorAction Stop } | Should -Be 'GuiGpuPassKey'
        Get-ErrorId { Set-GuiGpuSurface View -Compute @{ Shader = $tick; Binding = $count; Every = 'Sometimes' } -Window $w.Id -ErrorAction Stop } | Should -Be 'GuiGpuPassValue'
        Get-ErrorId { Set-GuiGpuSurface View -Compute @{ Binding = $count } -Window $w.Id -ErrorAction Stop } | Should -Be 'GuiGpuNoShader'
        $draw = { param($Passes) Set-GuiGpuSurface View $script:Passed -Binding @{ tint = $tint; count = $count } -VertexCount 3 -Compute $Passes -Window $w.Id -ErrorAction Stop }
        $once = @{ Shader = $paint; Binding = $tint; Workgroups = 1; Every = 'Once' }, @{ Shader = $tick; Binding = $count; Workgroups = 1; Every = 'once' }
        # A surface drawn first settles what the page draws with.
        Set-GuiGpuSurface View -ClearColor 0, 0, 0 -Window $w.Id
        if ($Renderer -eq 'Html' -and (Get-GuiOption).WebGpu -ne 'WebGPU') {
            $refused = { & $draw $once } | Should -Throw -ErrorId 'GuiGpuNoWebGpu*' -PassThru
            $refused.Exception.Message | Should -BeLike '*has no WebGPU, and the surface has a compute pass*-Renderer Egui*'
        } else {
            & $draw $once
            Wait-Until { Near (Texel (Shot $w View)) @(0, 255, 0) } 'the draw reading the tint and the count the passes wrote'
            1..4 | ForEach-Object { $null = Shot $w View }
            Near (Texel (Shot $w View)) @(0, 255, 0) | Should -BeTrue -Because 'passes given Once run before the first frame alone, so the count stays 1 as frames are drawn'
            if ($Renderer -eq 'Egui') {
                (Read-GuiGpuBuffer $count)[0] | Should -Be 1 -Because 'an egui window''s passes write the buffer itself'
            } else {
                (Read-GuiGpuBuffer $count)[0] | Should -Be 0 -Because 'an HTML window''s passes write the page''s copy, and the buffer still has what the session wrote'
            }
            & $draw @{ Shader = $tick; Binding = $count; Workgroups = 1 }
            Wait-Until { Near (Texel (Shot $w View)) @(0, 0, 255) } 'a pass given no Every running before each frame, the count passing 4'
        }
        Close-GuiWindow -Window $w.Id
        (Wait-GuiEvent -Window $w.Id -Timeout 20).Kind | Should -Be 'Close'
    }

    It 'tests depth in each depth format, reversed by -DepthCompare Greater against -ClearDepth 0' {
        $w = Show-GuiWindow -NoWait -Title 'PWRS-GUI test surface depth' -Renderer $Renderer { GpuSurface View -Width 64 -Height 64 }
        $null = Wait-GuiWindow 'PWRS-GUI test surface depth'
        $formats = @('Depth16Unorm', 'Depth24Plus', 'Depth24PlusStencil8', 'Depth32Float')
        # Depth32FloatStencil8 needs a feature some adapters lack.
        $held = try { $null = New-GuiGpuRenderPipeline $script:Deep -DepthFormat Depth32FloatStencil8 -ErrorAction Stop; $true } catch { $false }
        if ($held) { $formats += 'Depth32FloatStencil8' }
        foreach ($format in $formats) {
            $near = New-GuiGpuRenderPipeline $script:Deep -DepthFormat $format -DepthCompare Greater -Constant @{ depth = 0.2; red = 1 }
            $far = New-GuiGpuRenderPipeline $script:Deep -DepthFormat $format -DepthCompare Greater -Constant @{ depth = 0.8; red = 0 }
            $near.DepthFormat | Should -Be $format
            $both = @{ Shader = $near; VertexCount = 3 }, @{ Shader = $far; VertexCount = 3 }
            Set-GuiGpuSurface View -Draw $both -ClearColor 0, 1, 0 -ClearDepth 0 -Window $w.Id
            Wait-Until { Near (Texel (Shot $w View)) @(0, 0, 255) } "$format drawing the far triangle over the near one, each greater than the depth before it"
            Set-GuiGpuSurface View -Draw $both -ClearColor 0, 1, 0 -Window $w.Id
            Wait-Until { Near (Texel (Shot $w View)) @(0, 255, 0) } "$format drawing neither triangle, as no depth is greater than the 1 depth starts at"
        }
        Set-GuiGpuSurface View -Window $w.Id -Draw @{ Shader = $script:Deep; Constant = @{ depth = 0.2; red = 1 }; VertexCount = 3; Depth = $true }, @{ Shader = $script:Deep; Constant = @{ depth = 0.8; red = 0 }; VertexCount = 3; Depth = $true }
        Wait-Until { Near (Texel (Shot $w View)) @(255, 0, 0) } 'Less, when not given, keeping the near triangle'
        Close-GuiWindow -Window $w.Id
        (Wait-GuiEvent -Window $w.Id -Timeout 20).Kind | Should -Be 'Close'
    }

    It 'keeps a draw to where an earlier one wrote its StencilReference, a draw given none testing 0, on the stencil -ClearStencil starts' {
        $w = Show-GuiWindow -NoWait -Title 'PWRS-GUI test surface stencil' -Renderer $Renderer { GpuSurface View -Width 64 -Height 64 }
        $null = Wait-GuiWindow 'PWRS-GUI test surface stencil'
        $writer = @{ Shader = $script:LeftHalf; VertexCount = 3; DepthFormat = 'Stencil8'; StencilPass = 'Replace'; StencilReference = 1 }
        $masked = @{ Shader = $script:Fill; VertexCount = 3; DepthFormat = 'stencil8'; StencilCompare = 'Equal' }
        Set-GuiGpuSurface View -Draw $writer, ($masked + @{ StencilReference = 1 }) -ClearColor 0, 0, 1 -Window $w.Id
        Wait-Until { Halves $w @(255, 0, 0) @(0, 0, 255) } 'the red draw kept to the left half, where the green one wrote 1'
        Set-GuiGpuSurface View -Draw $writer, $masked -ClearColor 0, 0, 1 -Window $w.Id
        Wait-Until { Halves $w @(0, 255, 0) @(255, 0, 0) } 'a draw given no reference testing 0, not the 1 the draw before it gave'
        Set-GuiGpuSurface View -Draw $writer, ($masked + @{ StencilReference = 1 }) -ClearColor 0, 0, 1 -ClearStencil 1 -Window $w.Id
        Wait-Until { Halves $w @(255, 0, 0) @(255, 0, 0) } 'the stencil starting at 1 everywhere'
        Close-GuiWindow -Window $w.Id
        (Wait-GuiEvent -Window $w.Id -Timeout 20).Kind | Should -Be 'Close'
    }

    It 'tests a triangle facing away with the -BackStencil parameters and one facing the viewer with the front ones' {
        $w = Show-GuiWindow -NoWait -Title 'PWRS-GUI test surface faces' -Renderer $Renderer { GpuSurface View -Width 64 -Height 64 }
        $null = Wait-GuiWindow 'PWRS-GUI test surface faces'
        # The left triangle's corners turn counterclockwise, so it faces the
        # viewer; the right one's turn clockwise.
        $faces = New-GuiGpuBuffer ([float[]](-1, -1, 0, -1, -1, 3, 0, -1, 0, 3, 2, -1))
        $back = New-GuiGpuRenderPipeline $script:Placed -DepthFormat Stencil8 -StencilCompare Never -BackStencilCompare Always
        Set-GuiGpuSurface View $back -Vertex $faces -ClearColor 0, 0, 1 -Window $w.Id
        Wait-Until { Halves $w @(0, 0, 255) @(255, 0, 0) } 'the triangle facing away drawn, and the one facing the viewer not'
        $front = New-GuiGpuRenderPipeline $script:Placed -DepthFormat Stencil8 -StencilCompare Always -BackStencilCompare Never
        Set-GuiGpuSurface View $front -Vertex $faces -ClearColor 0, 0, 1 -Window $w.Id
        Wait-Until { Halves $w @(255, 0, 0) @(0, 0, 255) } 'the triangle facing the viewer drawn, and the one facing away not'
        Set-GuiGpuSurface View $script:Placed -Vertex $faces -DepthFormat Stencil8 -StencilCompare Never -ClearColor 0, 0, 1 -Window $w.Id
        Wait-Until { Halves $w @(0, 0, 255) @(0, 0, 255) } 'the back face taking the front one''s compare when given none'
        Close-GuiWindow -Window $w.Id
        (Wait-GuiEvent -Window $w.Id -Timeout 20).Kind | Should -Be 'Close'
    }

    It 'tests depth without writing it under -DepthWrite:$false, and moves a triangle toward the viewer by -DepthBias as far as -DepthBiasClamp lets it' {
        $w = Show-GuiWindow -NoWait -Title 'PWRS-GUI test surface depth write' -Renderer $Renderer { GpuSurface View -Width 64 -Height 64 }
        $null = Wait-GuiWindow 'PWRS-GUI test surface depth write'
        $near = @{ Shader = $script:Deep; Constant = @{ depth = 0.2; red = 1 }; VertexCount = 3; Depth = $true }
        $far = @{ Shader = $script:Deep; Constant = @{ depth = 0.8; red = 0 }; VertexCount = 3; Depth = $true }
        Set-GuiGpuSurface View -Draw $near, $far -Window $w.Id
        Wait-Until { Near (Texel (Shot $w View)) @(255, 0, 0) } 'the near triangle writing its depth, so the far one is not drawn'
        Set-GuiGpuSurface View -Draw ($near + @{ DepthWrite = $false }), $far -Window $w.Id
        Wait-Until { Near (Texel (Shot $w View)) @(0, 0, 255) } 'the near triangle leaving the depth at 1, so the far one draws over it'
        # The red triangle lies a hundred-thousandth behind the blue one; a
        # bias of 1000 of Depth32Float's smallest steps at that depth brings it
        # 6e-5 nearer, and a clamp of a millionth keeps it behind.
        $base = @{ Shader = $script:Deep; Constant = @{ depth = 0.5; red = 0 }; VertexCount = 3; Depth = $true }
        $behind = @{ Shader = $script:Deep; Constant = @{ depth = 0.50001; red = 1 }; VertexCount = 3; Depth = $true }
        Set-GuiGpuSurface View -Draw $base, $behind -Window $w.Id
        Wait-Until { Near (Texel (Shot $w View)) @(0, 0, 255) } 'the red triangle hidden behind the blue one'
        Set-GuiGpuSurface View -Draw $base, ($behind + @{ DepthBias = -1000 }) -Window $w.Id
        Wait-Until { Near (Texel (Shot $w View)) @(255, 0, 0) } 'the biased red triangle in front'
        $clamped = { Set-GuiGpuSurface View -Draw $base, ($behind + @{ DepthBias = -1000; DepthBiasClamp = -0.000001 }) -Window $w.Id -ErrorAction Stop }
        $refused = if ($Renderer -eq 'Html' -and (Get-GuiOption).WebGpu -eq 'WebGL2') { Get-ErrorId $clamped } else { & $clamped; 'no error' }
        if ($refused -eq 'no error') {
            Wait-Until { Near (Texel (Shot $w View)) @(0, 0, 255) } 'the clamped bias leaving the red triangle behind'
        } else {
            $refused | Should -Be 'GuiGpuNoWebGpu' -Because 'a WebGL 2 page clamps a bias only through EXT_polygon_offset_clamp, and refuses one where the engine has none'
        }
        Close-GuiWindow -Window $w.Id
        (Wait-GuiEvent -Window $w.Id -Timeout 20).Kind | Should -Be 'Close'
    }

    It 'refuses a depth or stencil test, a clear or a reference that the draws'' depth attachment cannot take, and depth formats that differ' {
        $w = Show-GuiWindow -NoWait -Title 'PWRS-GUI test surface depth refusals' -Renderer $Renderer { GpuSurface View -Width 32 -Height 32 }
        $null = Wait-GuiWindow 'PWRS-GUI test surface depth refusals'
        $refused = { param($Attempt) try { & $Attempt; 'no error' } catch { '{0}: {1}' -f ($_.FullyQualifiedErrorId -split ',')[0], $_.Exception.Message } }
        & $refused { Set-GuiGpuSurface View $script:Fill -VertexCount 3 -DepthCompare Greater -Window $w.Id -ErrorAction Stop } | Should -Be 'WgpuDepthTest: -DepthCompare, -DepthWrite and the depth bias test depth, and there is no depth target: give -Depth or a -DepthFormat with depth'
        & $refused { Set-GuiGpuSurface View $script:Fill -VertexCount 3 -Depth -StencilCompare Equal -Window $w.Id -ErrorAction Stop } | Should -BeLike 'WgpuStencil: *the depth target, Depth32Float, has no stencil aspect*'
        & $refused { Set-GuiGpuSurface View $script:Fill -VertexCount 3 -Topology LineList -Depth -DepthBias 1 -Window $w.Id -ErrorAction Stop } | Should -Be 'WgpuDepthBias: a depth bias moves triangles, and the topology is LineList'
        & $refused { Set-GuiGpuSurface View $script:Fill -VertexCount 3 -Depth -StencilReference 1 -Window $w.Id -ErrorAction Stop } | Should -BeLike 'WgpuStencilReference: *and this program has no stencil test'
        & $refused { Set-GuiGpuSurface View $script:Fill -VertexCount 3 -ClearDepth 0 -Window $w.Id -ErrorAction Stop } | Should -Be "GuiGpuClearDepth: -ClearDepth sets the depth the depth target starts at, and the draws' pipelines have no depth target"
        & $refused { Set-GuiGpuSurface View $script:Fill -VertexCount 3 -DepthFormat Stencil8 -ClearDepth 0 -Window $w.Id -ErrorAction Stop } | Should -BeLike 'GuiGpuClearDepth: *Stencil8, has no depth aspect'
        & $refused { Set-GuiGpuSurface View $script:Fill -VertexCount 3 -Depth -ClearStencil 1 -Window $w.Id -ErrorAction Stop } | Should -BeLike 'GuiGpuClearStencil: *Depth32Float, has no stencil aspect'
        & $refused { Set-GuiGpuSurface View -Draw @{ Shader = $script:Fill; VertexCount = 3; Depth = $true }, @{ Shader = $script:Fill; VertexCount = 3; DepthFormat = 'Depth24Plus' } -Window $w.Id -ErrorAction Stop } | Should -BeLike "GuiGpuSurfaceTargets: *draw 1's pipeline draws with 1 samples and Depth24Plus depth, and draw 0's with 1 and Depth32Float depth*"
        Close-GuiWindow -Window $w.Id
        (Wait-GuiEvent -Window $w.Id -Timeout 20).Kind | Should -Be 'Close'
    }

    It 'refuses a custom blend''s factor or operation beside another blend, as New-GuiGpuRenderPipeline does, and draws one beside -Blend Custom' {
        $w = Show-GuiWindow -NoWait -Title 'PWRS-GUI test surface blend refusals' -Renderer $Renderer { GpuSurface View -Width 32 -Height 32 }
        $null = Wait-GuiWindow 'PWRS-GUI test surface blend refusals'
        $refused = { param($Attempt) try { & $Attempt; 'no error' } catch { '{0}: {1}' -f ($_.FullyQualifiedErrorId -split ',')[0], $_.Exception.Message } }
        $words = '-BlendSource, -BlendDestination, -BlendOperation and their alpha forms name a Custom blend, and the blend is {0}: give -Blend Custom'
        & $refused { Set-GuiGpuSurface View $script:Fill -VertexCount 3 -Blend Alpha -BlendSource One -Window $w.Id -ErrorAction Stop } | Should -Be ('WgpuBlend: ' + ($words -f 'Alpha'))
        & $refused { Set-GuiGpuSurface View -Draw @{ Shader = $script:Fill; VertexCount = 3; AlphaBlendOperation = 'Max' } -Window $w.Id -ErrorAction Stop } | Should -BeLike ('WgpuBlend: *' + ($words -f 'Replace'))
        Set-GuiGpuSurface View $script:Fill -VertexCount 3 -Blend Custom -BlendSource One -BlendDestination Zero -ClearColor 0, 0, 1 -Window $w.Id
        Wait-Until { Near (Texel (Shot $w View)) @(255, 0, 0) } 'the Custom blend of One and Zero drawing red over the blue'
        Close-GuiWindow -Window $w.Id
        (Wait-GuiEvent -Window $w.Id -Timeout 20).Kind | Should -Be 'Close'
    }

    It 'refuses a widget that is not a surface, pipeline parameters beside a pipeline object, a hashtable for a vertex input, and draws the renderer cannot record' {
        $w = Show-GuiWindow -NoWait -Title 'PWRS-GUI test surface refusals' -Renderer $Renderer { GpuSurface View -Width 32 -Height 32; Text 'not a surface' -Name Note }
        $null = Wait-GuiWindow 'PWRS-GUI test surface refusals'
        $pipeline = New-GuiGpuRenderPipeline $script:Fill
        $left = New-GuiGpuBuffer ([float[]](-1, -1, 0, -1, -1, 3))
        $green = New-GuiGpuBuffer ([float[]](0, 1, 0, 1))
        Get-ErrorId { Set-GuiGpuSurface Note $pipeline -VertexCount 3 -Window $w.Id -ErrorAction Stop } | Should -Be 'GuiNotASurface'
        Get-ErrorId { Set-GuiGpuSurface Missing $pipeline -VertexCount 3 -Window $w.Id -ErrorAction Stop } | Should -Be 'GuiNoWidget'
        Get-ErrorId { Set-GuiGpuSurface View $pipeline -Depth -VertexCount 3 -Window $w.Id -ErrorAction Stop } | Should -Be 'GuiGpuPipelineOptions'
        Get-ErrorId { Set-GuiGpuSurface View $script:Tinted -Vertex @{ position = @{ x = 0; y = 0 } } -Binding $green -VertexCount 3 -Window $w.Id -ErrorAction Stop } | Should -Be 'GuiGpuVertex'
        Get-ErrorId { Set-GuiGpuSurface View $script:Tinted -Vertex $left -Binding $green -Index (New-GuiGpuBuffer ([float[]](0, 1, 2))) -Window $w.Id -ErrorAction Stop } | Should -Be 'GuiGpuIndexType'
        Get-ErrorId { Set-GuiGpuSurface View $pipeline -Window $w.Id -ErrorAction Stop } | Should -Be 'GuiGpuVertexCount'
        Get-ErrorId { Set-GuiGpuSurface View $pipeline -VertexCount 3 -BlendConstant 1, 1, 1, 1 -Window $w.Id -ErrorAction Stop } | Should -Be 'WgpuBlendConstant'
        Get-ErrorId { Save-GuiGpuSurface View (Join-Path ([IO.Path]::GetTempPath()) 'pwrs-gui-test-frame.bmp') -Window $w.Id -ErrorAction Stop } | Should -Be 'GuiGpuSaveFormat'
        Close-GuiWindow -Window $w.Id
        (Wait-GuiEvent -Window $w.Id -Timeout 20).Kind | Should -Be 'Close'
    }
}

# A reset ends the renderer's eframe run and starts another on a new device.
# The first case opens no window, so it runs anywhere; the second opens one
# and belongs on an idle desktop or a guest's display.
Describe 'A reset GPU device in <Mode> mode' -ForEach $cases {
    BeforeAll {
        Reset-GuiSession $Mode
        $script:Double = @'
@group(0) @binding(0) var<storage, read_write> v: array<f32>;
@compute @workgroup_size(64) fn main(@builtin(global_invocation_id) i: vec3<u32>) {
    if (i.x < arrayLength(&v)) { v[i.x] = v[i.x] * 2.0; }
}
'@

        # Resets the device and returns the PwrsGui.DeviceLost event the reset
        # raised in this runspace, taken off its event queue, where an event
        # nothing subscribes to waits.
        function Invoke-Reset {
            Get-Event | Where-Object SourceIdentifier -EQ PwrsGui.DeviceLost | Remove-Event
            Reset-GuiGpuDevice -Confirm:$false
            $lost = Wait-Event -SourceIdentifier PwrsGui.DeviceLost -Timeout 20
            if ($lost) { Remove-Event -EventIdentifier $lost.EventIdentifier }
            $lost
        }
    }

    It 'goes on with a new device, telling the runspace, with what the session wrote and not what a shader did' {
        $kept = New-GuiGpuBuffer ([float[]](1, 2, 3))
        $computed = New-GuiGpuBuffer ([float[]](5, 6))
        Invoke-GuiGpuCompute $script:Double -Binding $computed
        (Read-GuiGpuBuffer $computed) | Should -Be @(10, 12)
        $lost = Invoke-Reset
        $lost | Should -Not -BeNullOrEmpty
        $lost.MessageData.GetType().FullName | Should -Be 'PwrsGui.GpuDeviceEvent'
        $lost.MessageData.Reason | Should -Not -BeNullOrEmpty
        (Read-GuiGpuBuffer $kept) | Should -Be @(1, 2, 3)
        (Read-GuiGpuBuffer $computed) | Should -Be @(5, 6)
        (Invoke-GuiGpuCompute $script:Double ([float[]](4))) | Should -Be @(8)
    }

    It 'opens its windows again on the new device' {
        1..2 | Out-GuiGridView -Title 'PWRS-GUI test reset'
        $w = Wait-GuiWindow 'PWRS-GUI test reset'
        Invoke-Reset | Should -Not -BeNullOrEmpty
        @(Get-GuiWindow -Title 'PWRS-GUI test reset').Id | Should -Contain $w.Id
        $shot = Get-GuiSnapshot -Window $w.Id -Pixels
        $shot.Width | Should -BeGreaterThan 0
        $shot.Pixels.Length | Should -Be ($shot.Width * $shot.Height * 4)
    }
}
