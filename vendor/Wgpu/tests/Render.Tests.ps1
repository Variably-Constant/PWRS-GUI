# Pester 5 syntax; runs in pwsh and Windows PowerShell. PWRS_MODULE points
# at the built module folder. GPU cases are skipped on a host with no
# adapter, and say so.
BeforeDiscovery {
    $module = $env:PWRS_MODULE
    if (-not $module) { throw 'PWRS_MODULE is not set' }
    Import-Module (Join-Path $module 'Wgpu.psd1') -ErrorAction Stop
    $script:noGpu = @(Get-WgpuAdapter).Count -eq 0
}

BeforeAll {
    Import-Module (Join-Path $env:PWRS_MODULE 'Wgpu.psd1') -ErrorAction Stop
    $script:fullscreen = @'
@vertex
fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    var corners = array<vec2<f32>, 3>(vec2<f32>(-1.0, -1.0), vec2<f32>(3.0, -1.0), vec2<f32>(-1.0, 3.0));
    return vec4<f32>(corners[i], 0.0, 1.0);
}
@fragment
fn fs() -> @location(0) vec4<f32> { return vec4<f32>(1.0, 0.0, 0.0, 1.0); }
'@
    $script:colored = @'
struct Out { @builtin(position) position: vec4<f32>, @location(0) color: vec3<f32> }
@vertex
fn vs(@location(0) position: vec2<f32>, @location(1) color: vec3<f32>) -> Out {
    var out: Out;
    out.position = vec4<f32>(position, 0.0, 1.0);
    out.color = color;
    return out;
}
@fragment
fn fs(input: Out) -> @location(0) vec4<f32> { return vec4<f32>(input.color, 1.0); }
'@
    # A green triangle over the left half of a target two texels wide.
    $script:leftHalf = @'
@vertex
fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    var corners = array<vec2<f32>, 3>(vec2<f32>(-1.0, -1.0), vec2<f32>(0.0, -1.0), vec2<f32>(-1.0, 3.0));
    return vec4<f32>(corners[i], 0.0, 1.0);
}
@fragment
fn fs() -> @location(0) vec4<f32> { return vec4<f32>(0.0, 1.0, 0.0, 1.0); }
'@
    # A full-screen triangle at the depth `depth` names, red at red 1 and
    # blue at red 0; triangles from vertex 3 on repeat the first.
    $script:deep = @'
override depth: f32 = 0.5;
override red: f32 = 1.0;
@vertex
fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    var corners = array<vec2<f32>, 3>(vec2<f32>(-1.0, -1.0), vec2<f32>(3.0, -1.0), vec2<f32>(-1.0, 3.0));
    return vec4<f32>(corners[i % 3u], depth, 1.0);
}
@fragment
fn fs() -> @location(0) vec4<f32> { return vec4<f32>(red, 0.0, 1.0 - red, 1.0); }
'@
    $script:everyPixel = { param($bytes, $rgba) for ($i = 0; $i -lt $bytes.Length; $i += 4) { if (($bytes[$i..($i + 3)] -join ',') -ne $rgba) { return $false } }; $true }
    # Every pixel within one step of each channel of $rgba: a GPU may blend
    # an 8-bit target at its own precision.
    $script:nearEveryPixel = {
        param($bytes, $rgba)
        $want = $rgba -split ','
        for ($i = 0; $i -lt $bytes.Length; $i++) { if ([Math]::Abs([int]$bytes[$i] - [int]$want[$i % 4]) -gt 1) { return $false } }
        $true
    }
}

Describe 'Invoke-WgpuRender' {
    It 'draws a full-screen triangle from vertex_index alone' -Skip:$noGpu {
        $texture = Invoke-WgpuRender $fullscreen 8 8 -VertexCount 3
        $texture.Format | Should -Be 'Rgba8Unorm'
        & $everyPixel (Read-WgpuTexture $texture) '255,0,0,255' | Should -BeTrue
    }

    It 'feeds vertex inputs by name and counts the vertices from them' -Skip:$noGpu {
        $vertices = @{ position = [float[]](-1, -1, 3, -1, -1, 3); color = [float[]](0, 1, 0, 0, 1, 0, 0, 1, 0) }
        $texture = Invoke-WgpuRender $colored 8 8 -Vertex $vertices
        & $everyPixel (Read-WgpuTexture $texture) '0,255,0,255' | Should -BeTrue
    }

    It 'draws by indices' -Skip:$noGpu {
        $vertices = @{ position = [float[]](-1, -1, 1, -1, -1, 1, 1, 1); color = [float[]](0, 0, 1, 0, 0, 1, 0, 0, 1, 0, 0, 1) }
        $texture = Invoke-WgpuRender $colored 8 8 -Vertex $vertices -Index ([uint32[]](0, 1, 2, 2, 1, 3))
        & $everyPixel (Read-WgpuTexture $texture) '0,0,255,255' | Should -BeTrue
    }

    It 'draws a shader read from SPIR-V the way up its WGSL draws' -Skip:$noGpu {
        $corner = @'
@vertex
fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    var corners = array<vec2<f32>, 3>(vec2<f32>(-1.0, -1.0), vec2<f32>(0.0, -1.0), vec2<f32>(-1.0, 0.0));
    return vec4<f32>(corners[i], 0.0, 1.0);
}
@fragment
fn fs() -> @location(0) vec4<f32> { return vec4<f32>(1.0, 1.0, 1.0, 1.0); }
'@
        $spirv = New-WgpuShader -SpirV (ConvertTo-WgpuShader -Source $corner -To SpirV)
        $fromSpirV = Read-WgpuTexture (Invoke-WgpuRender $spirv 8 8 -VertexCount 3)
        $fromWgsl = Read-WgpuTexture (Invoke-WgpuRender $corner 8 8 -VertexCount 3)
        $fromSpirV[224..227] -join ',' | Should -Be '255,255,255,255'
        $fromSpirV[0..3] -join ',' | Should -Be '0,0,0,255'
        ($fromSpirV -join ',') | Should -Be ($fromWgsl -join ',')
    }

    It 'clears to the color given and leaves undrawn pixels that color' -Skip:$noGpu {
        $vertices = @{ position = [float[]](-1, -1, 0, -1, -1, 0); color = [float[]](1, 1, 1, 1, 1, 1, 1, 1, 1) }
        $texels = Read-WgpuTexture (Invoke-WgpuRender $colored 8 8 -Vertex $vertices -ClearColor 0, 0, 1, 1)
        $last = $texels.Length - 4
        $texels[$last..($last + 3)] -join ',' | Should -Be '0,0,255,255'
    }

    It 'discards the counterclockwise triangle with -CullMode Front and keeps it with Back' -Skip:$noGpu {
        & $everyPixel (Read-WgpuTexture (Invoke-WgpuRender $fullscreen 4 4 -VertexCount 3 -CullMode Front)) '0,0,0,255' | Should -BeTrue
        & $everyPixel (Read-WgpuTexture (Invoke-WgpuRender $fullscreen 4 4 -VertexCount 3 -CullMode Back)) '255,0,0,255' | Should -BeTrue
    }

    It 'draws with multisampling and depth' -Skip:$noGpu {
        $texture = Invoke-WgpuRender $fullscreen 8 8 -VertexCount 3 -SampleCount 4 -Depth
        & $everyPixel (Read-WgpuTexture $texture) '255,0,0,255' | Should -BeTrue
    }

    It 'saves the drawing as an image file' -Skip:$noGpu {
        $file = Join-Path ([IO.Path]::GetTempPath()) "wgpu-render-$PID.png"
        try {
            Invoke-WgpuRender $fullscreen 16 16 -VertexCount 3 -OutFile $file | Should -BeNullOrEmpty
            (Get-Item $file).Length | Should -BeGreaterThan 0
            & $everyPixel (Read-WgpuTexture (Import-WgpuImage $file)) '255,0,0,255' | Should -BeTrue
        } finally {
            Remove-Item $file -ErrorAction Ignore
        }
    }

    It 'asks for vertices a shader reads' -Skip:$noGpu {
        { Invoke-WgpuRender $colored 8 8 } | Should -Throw '*give them with -Vertex*'
    }

    It 'blends with the factors and operations a Custom blend names, alpha taking the color''s unless given' -Skip:$noGpu {
        $custom = @{ Blend = 'Custom'; BlendSource = 'One'; BlendDestination = 'One'; BlendOperation = 'ReverseSubtract' }
        & $everyPixel (Read-WgpuTexture (Invoke-WgpuRender $fullscreen 4 4 -VertexCount 3 -ClearColor 1, 1, 1, 1 @custom)) '0,255,255,0' | Should -BeTrue
        & $everyPixel (Read-WgpuTexture (Invoke-WgpuRender $fullscreen 4 4 -VertexCount 3 -ClearColor 1, 1, 1, 1 @custom -AlphaBlendOperation Max)) '0,255,255,255' | Should -BeTrue
    }

    It 'refuses a blend factor or operation beside a blend that is not Custom' -Skip:$noGpu {
        { Invoke-WgpuRender $fullscreen 4 4 -VertexCount 3 -Blend Alpha -BlendSource Constant } | Should -Throw '*the blend is Alpha: give -Blend Custom'
        { New-WgpuRenderPipeline $fullscreen -AlphaBlendOperation Max } | Should -Throw '*the blend is Replace: give -Blend Custom'
    }

    It 'mixes the fragment over the target by -BlendConstant' -Skip:$noGpu {
        $mixed = Invoke-WgpuRender $fullscreen 4 4 -VertexCount 3 -ClearColor 0, 0, 1, 1 -Blend Constant -BlendConstant 0.25, 0.25, 0.25, 0.25
        & $nearEveryPixel (Read-WgpuTexture $mixed) '64,0,191,255' | Should -BeTrue
    }

    It 'refuses a blend that reads a constant without -BlendConstant, and -BlendConstant beside a blend that reads none' -Skip:$noGpu {
        { Invoke-WgpuRender $fullscreen 4 4 -VertexCount 3 -Blend Constant } | Should -Throw '*the blend reads a constant: give -BlendConstant'
        { Invoke-WgpuRender $fullscreen 4 4 -VertexCount 3 -Blend Custom -AlphaBlendDestination OneMinusConstant } | Should -Throw '*the blend reads a constant: give -BlendConstant'
        { Invoke-WgpuRender $fullscreen 4 4 -VertexCount 3 -BlendConstant 1, 1, 1, 1 } | Should -Throw '*-BlendConstant sets the constant a blend reads, and this blend reads none'
        { Invoke-WgpuRender $fullscreen 4 4 -VertexCount 3 -Blend Constant -BlendConstant 0.5, 0.5, 0.5 } | Should -Throw '*a blend constant is four numbers, red, green, blue and alpha; 3 were given'
    }

    It 'draws each pixel once in one draw with a stencil test, where an additive blend draws two triangles twice' -Skip:$noGpu {
        $twice = @{ VertexCount = 6; Blend = 'Additive'; Constant = @{ red = 0.25 } }
        & $nearEveryPixel (Read-WgpuTexture (Invoke-WgpuRender $deep 4 4 @twice)) '128,0,255,255' | Should -BeTrue
        $once = Invoke-WgpuRender $deep 4 4 @twice -DepthFormat Stencil8 -StencilCompare Equal -StencilPass IncrementClamp
        & $nearEveryPixel (Read-WgpuTexture $once) '64,0,191,255' | Should -BeTrue
    }

    It 'tests depth Greater against -ClearDepth 0, and draws nothing against the 1 depth starts at' -Skip:$noGpu {
        & $everyPixel (Read-WgpuTexture (Invoke-WgpuRender $deep 4 4 -VertexCount 3 -Depth -DepthCompare Greater -ClearDepth 0)) '255,0,0,255' | Should -BeTrue
        & $everyPixel (Read-WgpuTexture (Invoke-WgpuRender $deep 4 4 -VertexCount 3 -Depth -DepthCompare Greater)) '0,0,0,255' | Should -BeTrue
    }

    It 'refuses a test or a start the depth target cannot hold, a reference with no stencil test, and a bias off triangles' -Skip:$noGpu {
        { Invoke-WgpuRender $fullscreen 4 4 -VertexCount 3 -Depth -StencilCompare Equal } | Should -Throw '*the depth target, Depth32Float, has no stencil aspect*'
        { Invoke-WgpuRender $fullscreen 4 4 -VertexCount 3 -DepthCompare Greater } | Should -Throw '*and there is no depth target: give -Depth or a -DepthFormat with depth'
        { Invoke-WgpuRender $fullscreen 4 4 -VertexCount 3 -DepthFormat Stencil8 -DepthWrite:$false } | Should -Throw '*the depth target, Stencil8, has no depth aspect*'
        { Invoke-WgpuRender $fullscreen 4 4 -VertexCount 3 -DepthFormat Rgba8Unorm } | Should -Throw '*-DepthFormat names a depth or stencil format*Rgba8Unorm is not one'
        { Invoke-WgpuRender $fullscreen 4 4 -VertexCount 3 -Depth -StencilReference 1 } | Should -Throw '*and this pipeline has no stencil test'
        { Invoke-WgpuRender $fullscreen 4 4 -VertexCount 3 -Depth -ClearStencil 1 } | Should -Throw '*-ClearStencil sets the value the stencil starts at, and the depth target, Depth32Float, has no stencil aspect'
        { Invoke-WgpuRender $fullscreen 4 4 -VertexCount 3 -Depth -ClearDepth 2 } | Should -Throw '*-ClearDepth is a depth from 0 to 1, and 2 is not'
        { Invoke-WgpuRender $fullscreen 4 4 -VertexCount 3 -Topology LineList -Depth -DepthBias 1 } | Should -Throw '*a depth bias moves triangles, and the topology is LineList'
    }
}

Describe 'Render pipelines and passes' {
    It 'reflects the vertex inputs of a pipeline' -Skip:$noGpu {
        $pipeline = New-WgpuRenderPipeline $colored
        $pipeline.VertexInputs -join ',' | Should -Be 'position,color'
        $pipeline.TargetFormats | Should -Be @('Rgba8Unorm')
    }

    It 'builds a pipeline from the shader piped to it, as text or as a shader object' -Skip:$noGpu {
        ($colored | New-WgpuRenderPipeline).VertexInputs -join ',' | Should -Be 'position,color'
        (New-WgpuShader -Source $colored | New-WgpuRenderPipeline).VertexInputs -join ',' | Should -Be 'position,color'
    }

    It 'draws through a pipeline built with multisampling and depth as it was built' -Skip:$noGpu {
        $pipeline = New-WgpuRenderPipeline $fullscreen -SampleCount 4 -Depth
        $texture = Invoke-WgpuRender $pipeline 8 8 -VertexCount 3
        & $everyPixel (Read-WgpuTexture $texture) '255,0,0,255' | Should -BeTrue
    }

    It 'records a render pass by hand' -Skip:$noGpu {
        $pipeline = New-WgpuRenderPipeline $fullscreen
        $target = New-WgpuTexture 4 4
        $encoder = New-WgpuCommandEncoder
        $pass = $encoder.BeginRenderPass($target, @(0, 0, 0, 1), $null, $null, $null, $null)
        $pass.SetPipeline($pipeline)
        $pass.Draw(3, $null, $null, $null)
        $pass.End()
        $encoder.Finish() | Submit-WgpuCommand
        & $everyPixel (Read-WgpuTexture $target) '255,0,0,255' | Should -BeTrue
    }

    It 'mixes the fragment over the target by the blend constant a pass sets' -Skip:$noGpu {
        $pipeline = New-WgpuRenderPipeline $fullscreen -Blend Constant
        $target = New-WgpuTexture 4 4
        $encoder = New-WgpuCommandEncoder
        $pass = $encoder.BeginRenderPass($target, @(0, 0, 1, 1), $null, $null, $null, $null)
        $pass.SetPipeline($pipeline)
        $pass.SetBlendConstant(0.25, 0.25, 0.25, 0.25)
        $pass.Draw(3, $null, $null, $null)
        $pass.End()
        $encoder.Finish() | Submit-WgpuCommand
        & $nearEveryPixel (Read-WgpuTexture $target) '64,0,191,255' | Should -BeTrue
    }

    It 'keeps a second draw to where a first wrote the stencil reference, in one pass and in a pass that loads the stencil' -Skip:$noGpu {
        $writer = New-WgpuRenderPipeline $leftHalf -DepthFormat Stencil8 -StencilPass Replace
        $masked = New-WgpuRenderPipeline $fullscreen -DepthFormat Stencil8 -StencilCompare Equal
        $writer.DepthFormat | Should -Be 'Stencil8'
        $stencil = New-WgpuTexture 2 1 -Format Stencil8
        $target = New-WgpuTexture 2 1
        $encoder = New-WgpuCommandEncoder
        $pass = $encoder.BeginRenderPass(@{ Color = $target; ClearColor = @(0, 0, 1, 1); Depth = $stencil })
        $pass.SetStencilReference(1)
        $pass.SetPipeline($writer)
        $pass.Draw(3, $null, $null, $null)
        $pass.SetPipeline($masked)
        $pass.Draw(3, $null, $null, $null)
        $pass.End()
        $encoder.Finish() | Submit-WgpuCommand
        (Read-WgpuTexture $target) -join ',' | Should -Be '255,0,0,255,0,0,255,255'
        (Read-WgpuTexture $stencil -Aspect StencilOnly) -join ',' | Should -Be '1,0'
        foreach ($case in @(@{ Stencil = 'Load'; Left = '255,0,0,255' }, @{ Stencil = 0; Left = '0,255,0,255' })) {
            $encoder = New-WgpuCommandEncoder
            $first = $encoder.BeginRenderPass(@{ Color = $target; Depth = $stencil; ClearStencil = 0 })
            $first.SetStencilReference(1)
            $first.SetPipeline($writer)
            $first.Draw(3, $null, $null, $null)
            $first.End()
            $second = $encoder.BeginRenderPass(@{ Color = $target; ClearColor = 'Load'; Depth = $stencil; ClearStencil = $case.Stencil })
            $second.SetStencilReference(1)
            $second.SetPipeline($masked)
            $second.Draw(3, $null, $null, $null)
            $second.End()
            $encoder.Finish() | Submit-WgpuCommand
            (Read-WgpuTexture $target) -join ',' | Should -Be "$($case.Left),0,0,0,255"
        }
    }

    It 'tests depth without writing it with -DepthWrite:$false' -Skip:$noGpu {
        $depth = New-WgpuTexture 4 4 -Format Depth32Float
        $far = New-WgpuRenderPipeline $deep -Depth -Constant @{ depth = 0.8; red = 0 }
        foreach ($case in @(@{ Write = $true; Color = '255,0,0,255' }, @{ Write = $false; Color = '0,0,255,255' })) {
            $near = New-WgpuRenderPipeline $deep -Depth -DepthWrite:($case.Write) -Constant @{ depth = 0.2; red = 1 }
            $target = New-WgpuTexture 4 4
            $encoder = New-WgpuCommandEncoder
            $pass = $encoder.BeginRenderPass(@{ Color = $target; Depth = $depth })
            $pass.SetPipeline($near)
            $pass.Draw(3, $null, $null, $null)
            $pass.SetPipeline($far)
            $pass.Draw(3, $null, $null, $null)
            $pass.End()
            $encoder.Finish() | Submit-WgpuCommand
            & $everyPixel (Read-WgpuTexture $target) $case.Color | Should -BeTrue
        }
    }

    It 'refuses a pass hashtable with an unknown key or no Color, a clear its depth target cannot take, and arguments beside it' -Skip:$noGpu {
        $target = New-WgpuTexture 2 1
        $encoder = New-WgpuCommandEncoder
        { $encoder.BeginRenderPass(@{ Target = $target }) } | Should -Throw '*OcclusionQuerySet and Label, and Target is none of them*'
        { $encoder.BeginRenderPass(@{ ClearColor = @(0, 0, 0, 1) }) } | Should -Throw '*names no Color, the texture or view the pass draws into*'
        { $encoder.BeginRenderPass(@{ Color = $target; ClearStencil = 0 }) } | Should -Throw '*ClearStencil sets what the depth target''s stencil starts at, and there is no depth target*'
        $depth = New-WgpuTexture 2 1 -Format Depth32Float
        { $encoder.BeginRenderPass(@{ Color = $target; Depth = $depth; ClearStencil = 'Load' }) } | Should -Throw '*the depth target, Depth32Float, has no stencil aspect*'
        { $encoder.BeginRenderPass(@{ Color = $target; Depth = $depth; ClearDepth = 3 }) } | Should -Throw '*ClearDepth is a depth from 0 to 1 or ''Load'', and 3 is not*'
        { $encoder.BeginRenderPass(@{ Color = $target }, @(0, 0, 0, 1), $null, $null, $null, $null) } | Should -Throw '*so BeginRenderPass takes it alone*'
    }
}
