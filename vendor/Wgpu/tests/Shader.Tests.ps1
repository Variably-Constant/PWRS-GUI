# Pester 5 syntax; runs in pwsh and Windows PowerShell. PWRS_MODULE points
# at the built module folder. Nothing here needs a GPU.
BeforeAll {
    $module = $env:PWRS_MODULE
    if (-not $module) { throw 'PWRS_MODULE is not set' }
    Import-Module (Join-Path $module 'Wgpu.psd1') -Force -ErrorAction Stop
    $script:double = @'
@group(0) @binding(0) var<storage, read_write> v: array<f32>;
@compute @workgroup_size(64)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    if (id.x < arrayLength(&v)) { v[id.x] = v[id.x] * 2.0; }
}
'@
    $script:params = @'
struct Params { scale: f32, offset: vec2<f32>, count: u32 }
override gain: f32 = 1.5;
@group(0) @binding(0) var<storage, read_write> data: array<f32>;
@group(0) @binding(1) var<uniform> p: Params;
@group(1) @binding(3) var<storage, read> extra: array<i32>;
@compute @workgroup_size(8, 4, 2)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    data[id.x] = data[id.x] * p.scale * gain + f32(extra[0]) + p.offset.x + f32(p.count);
}
'@
    $script:glsl = @'
#version 450
layout(local_size_x = 32) in;
layout(set = 0, binding = 0) buffer Data { float values[]; };
void main() { values[gl_GlobalInvocationID.x] += 1.0; }
'@
}

Describe 'Test-WgpuShader' {
    It 'returns $true for a valid shader' {
        Test-WgpuShader -Source $double | Should -BeTrue
    }

    It 'returns $false for an invalid one' {
        Test-WgpuShader -Source 'fn main( {' | Should -BeFalse
    }

    It 'writes the line and column of each problem with -Diagnostics' {
        $found = @(Test-WgpuShader -Source "fn main() {`n    let x: f32 = 1u;`n}" -Diagnostics)
        $found.Count | Should -Be 1
        $found[0].Line | Should -Be 2
        $found[0].Column | Should -BeGreaterThan 0
        $found[0].SourceLine | Should -Match 'let x'
        $found[0].Message | Should -Not -BeNullOrEmpty
    }

    It 'writes nothing with -Diagnostics for a valid shader' {
        @(Test-WgpuShader -Source $double -Diagnostics).Count | Should -Be 0
    }

    It 'checks GLSL' {
        Test-WgpuShader -Source $glsl | Should -BeTrue
    }

    It 'takes a shader object by position and from the pipeline, valid as it was made' {
        $shader = New-WgpuShader -Source $double
        Test-WgpuShader $shader | Should -BeTrue
        $shader | Test-WgpuShader | Should -BeTrue
        Test-WgpuShader -Shader $shader | Should -BeTrue
        @(Test-WgpuShader $shader -Diagnostics).Count | Should -Be 0
    }

    It 'checks the shader text piped to it' {
        $double | Test-WgpuShader | Should -BeTrue
        'fn main( {' | Test-WgpuShader | Should -BeFalse
    }

    It 'runs under its alias Test-GpuShader' {
        (Get-Alias Test-GpuShader).Definition | Should -Be 'Test-WgpuShader'
    }
}

Describe 'Get-WgpuShaderInfo' {
    It 'reflects the entry point, its workgroup size and its bindings' {
        $info = Get-WgpuShaderInfo -Source $double
        $info.Language | Should -Be 'Wgsl'
        $info.EntryPoints[0].Name | Should -Be 'main'
        $info.EntryPoints[0].Stage | Should -Be 'Compute'
        ($info.EntryPoints[0].WorkgroupSize -join ',') | Should -Be '64,1,1'
        $info.Bindings[0].Name | Should -Be 'v'
        $info.Bindings[0].Kind | Should -Be 'StorageBuffer'
        $info.Bindings[0].ElementType | Should -Be 'Float32'
        $info.Bindings[0].WgslType | Should -Be 'array<f32>'
        $info.Bindings[0].Stride | Should -Be 4
    }

    It 'reflects every group, binding kind, struct size and override' {
        $info = Get-WgpuShaderInfo -Source $params
        ($info.EntryPoints[0].WorkgroupSize -join ',') | Should -Be '8,4,2'
        $info.Bindings.Count | Should -Be 3
        $uniform = $info.Bindings | Where-Object Name -EQ 'p'
        $uniform.Kind | Should -Be 'UniformBuffer'
        $uniform.WgslType | Should -Be 'Params'
        $uniform.Size | Should -Be 24
        $extra = $info.Bindings | Where-Object Name -EQ 'extra'
        $extra.Group | Should -Be 1
        $extra.Binding | Should -Be 3
        $extra.Kind | Should -Be 'ReadOnlyStorageBuffer'
        $extra.ElementType | Should -Be 'Int32'
        $info.Overrides[0].Name | Should -Be 'gain'
        $info.Overrides[0].Default | Should -Be 1.5
        $info.Overrides[0].WgslType | Should -Be 'f32'
    }

    It 'writes texture and storage texture types as WGSL does' {
        $info = Get-WgpuShaderInfo -Source @'
@group(0) @binding(0) var a: texture_storage_2d<rgba8unorm, write>;
@group(0) @binding(1) var b: texture_storage_2d_array<r32float, read_write>;
@group(0) @binding(2) var c: texture_storage_3d<rgba16float, read>;
@group(0) @binding(3) var d: texture_cube<f32>;
@group(0) @binding(4) var e: texture_depth_2d;
@compute @workgroup_size(1)
fn main() {
    textureStore(a, vec2<i32>(0, 0), vec4<f32>(0.0));
    textureStore(b, vec2<i32>(0, 0), 0, textureLoad(b, vec2<i32>(0, 0), 0));
    _ = textureLoad(c, vec3<i32>(0, 0, 0));
    _ = textureDimensions(d);
    _ = textureDimensions(e);
}
'@
        ($info.Bindings | ForEach-Object WgslType) -join '; ' | Should -Be 'texture_storage_2d<rgba8unorm, write>; texture_storage_2d_array<r32float, read_write>; texture_storage_3d<rgba16float, read>; texture_cube<f32>; texture_depth_2d'
    }

    It 'reflects GLSL' {
        $info = Get-WgpuShaderInfo -Source $glsl
        $info.Language | Should -Be 'Glsl'
        ($info.EntryPoints[0].WorkgroupSize -join ',') | Should -Be '32,1,1'
        $info.Bindings[0].ElementType | Should -Be 'Float32'
    }

    It 'reads a shader object by name, by position and from the pipeline' {
        $shader = New-WgpuShader -Source $double -Label doubler
        (Get-WgpuShaderInfo -Shader $shader).EntryPoints[0].Name | Should -Be 'main'
        (Get-WgpuShaderInfo $shader).Label | Should -Be 'doubler'
        ($shader | Get-WgpuShaderInfo).Bindings[0].Name | Should -Be 'v'
    }

    It 'reads the shader text piped to it' {
        ($double | Get-WgpuShaderInfo).Bindings[0].Name | Should -Be 'v'
    }
}

Describe 'ConvertTo-WgpuShader' {
    It 'writes <To>' -TestCases @(
        @{ To = 'Wgsl'; Expect = 'fn main' }
        @{ To = 'Hlsl'; Expect = 'numthreads\(64, 1, 1\)' }
        @{ To = 'Msl'; Expect = 'kernel' }
        @{ To = 'Glsl'; Expect = '#version 450' }
        @{ To = 'Dot'; Expect = 'digraph' }
    ) {
        param($To, $Expect)
        ConvertTo-WgpuShader -Source $double -To $To | Should -Match $Expect
    }

    It 'writes SPIR-V words that start with its magic number' {
        $words = ConvertTo-WgpuShader -Source $double -To SpirV
        $words.GetType().Name | Should -Be 'UInt32[]'
        $words[0] | Should -Be 0x07230203
    }

    It 'reads SPIR-V back' {
        $words = ConvertTo-WgpuShader -Source $double -To SpirV
        ConvertTo-WgpuShader -SpirV $words -To Wgsl | Should -Match 'fn main'
    }

    It 'reports SPIR-V that makes naga panic as a problem with no line' {
        $words = ConvertTo-WgpuShader -Source $double -To SpirV
        $at = 5
        while (($words[$at] -band 0xffff) -ne 22) { $at += $words[$at] -shr 16 }
        $words[$at + 2] = 7
        $found = @(Test-WgpuShader -SpirV $words -Diagnostics)
        $found.Count | Should -Be 1
        $found[0].Line | Should -Be 0
        $found[0].Message | Should -BeLike 'naga panicked while reading the SPIR-V: *NonPowerOfTwoWidth*'
        { New-WgpuShader -SpirV $words } | Should -Throw '*naga panicked while reading the SPIR-V*'
    }

    It 'writes the names of the bindings, structs and members into SPIR-V, as glslc does' {
        $source = @'
struct Params { scale: f32, count: u32 }
@group(0) @binding(0) var<storage, read_write> data: array<f32>;
@group(0) @binding(1) var<uniform> p: Params;
@compute @workgroup_size(64)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    if (id.x < p.count) { data[id.x] = data[id.x] * p.scale; }
}
'@
        $words = ConvertTo-WgpuShader -Source $source -To SpirV
        $info = Get-WgpuShaderInfo -SpirV $words
        $info.Bindings.Name -join ',' | Should -Be 'data,p'
        ($info.Bindings | Where-Object Name -EQ 'p').WgslType | Should -Be 'Params'
        ConvertTo-WgpuShader -SpirV $words -To Wgsl | Should -Match 'scale: f32'
    }

    It 'translates a shader object by position and from the pipeline' {
        $shader = New-WgpuShader -Source $double -Label doubler
        ConvertTo-WgpuShader $shader -To Hlsl | Should -Match 'numthreads\(64, 1, 1\)'
        ($shader | ConvertTo-WgpuShader -To Wgsl).Label | Should -Be 'doubler'
    }

    It 'translates the shader text piped to it' {
        $double | ConvertTo-WgpuShader -To Hlsl | Should -Match 'numthreads\(64, 1, 1\)'
    }

    It 'translates GLSL to WGSL' {
        ConvertTo-WgpuShader -Source $glsl -To Wgsl | Should -Match '@workgroup_size\(32'
    }

    It 'writes OpenGL ES GLSL' {
        ConvertTo-WgpuShader -Source $double -To Glsl -GlslEs | Should -Match '#version 310 es'
    }

    It 'writes HLSL for a shader model' {
        ConvertTo-WgpuShader -Source $double -To Hlsl -ShaderModel 6.6 | Should -Match 'numthreads'
    }

    It 'writes SPIR-V to a binary file' {
        $file = Join-Path ([IO.Path]::GetTempPath()) "wgpu-$PID.spv"
        try {
            ConvertTo-WgpuShader -Source $double -To SpirV -OutFile $file
            $bytes = [IO.File]::ReadAllBytes($file)
            $bytes.Length % 4 | Should -Be 0
            [BitConverter]::ToUInt32($bytes, 0) | Should -Be 0x07230203
            Test-WgpuShader -Path $file | Should -BeTrue
        } finally {
            Remove-Item $file -ErrorAction Ignore
        }
    }

    It 'refuses an unknown shader model' {
        { ConvertTo-WgpuShader -Source $double -To Hlsl -ShaderModel 9.9 } | Should -Throw '*shader model*'
    }
}

Describe 'New-WgpuShader' {
    It 'parses WGSL into a shader object' {
        $shader = New-WgpuShader -Source $double -Label doubler
        $shader.Label | Should -Be 'doubler'
        $shader.Language | Should -Be 'Wgsl'
        $shader.EntryPoints[0].Name | Should -Be 'main'
        $shader.Source | Should -Match 'arrayLength'
    }

    It 'parses the shader text piped to it' {
        $shader = $double | New-WgpuShader -Label doubler
        $shader.Label | Should -Be 'doubler'
        $shader.EntryPoints[0].Name | Should -Be 'main'
    }

    It 'refuses an invalid shader with the line it is on' {
        { New-WgpuShader -Source "fn main() {`n    let x: f32 = 1u;`n}" } | Should -Throw '*line 2*'
    }

    It 'reports a file that does not exist as WgpuShaderFile, with PowerShell''s reason' {
        $missing = Join-Path ([IO.Path]::GetTempPath()) "wgpu-missing-$PID.wgsl"
        New-WgpuShader -Path $missing -ErrorAction SilentlyContinue -ErrorVariable failed
        $failed[0].FullyQualifiedErrorId | Should -BeLike 'WgpuShaderFile,*'
        $failed[0].CategoryInfo.Category | Should -Be 'ObjectNotFound'
        $failed[0].Exception.Message | Should -BeLike "Cannot find path '*' because it does not exist."
    }

    It 'converts through the object' {
        $shader = [Wgpu.Shader]::new($double, $null, $null)
        $shader.ConvertTo('Hlsl', $null) | Should -Match 'numthreads'
        $shader.Info().Bindings[0].Name | Should -Be 'v'
    }

    It 'leaves Source $null for SPIR-V' {
        $shader = New-WgpuShader -SpirV (ConvertTo-WgpuShader -Source $double -To SpirV)
        $shader.Language | Should -Be 'SpirV'
        $shader.Source | Should -BeNullOrEmpty
    }
}

Describe 'Shader files' {
    BeforeAll {
        $script:folder = Join-Path ([IO.Path]::GetTempPath()) "wgpu-shaders-$PID"
        [void][IO.Directory]::CreateDirectory($folder)
        $script:broken = Join-Path $folder 'a-broken.wgsl'
        [IO.File]::WriteAllText($broken, "fn main() {`n    let x: f32 = 1u;`n}")
        $script:bracketed = Join-Path $folder 'b[1].wgsl'
        [IO.File]::WriteAllText($bracketed, $double)
        $script:first = Join-Path $folder 'm1.wgsl'
        [IO.File]::WriteAllText($first, $double)
        $script:second = Join-Path $folder 'm2.wgsl'
        [IO.File]::WriteAllText($second, $params)
    }

    AfterAll {
        foreach ($file in $broken, $bracketed, $first, $second) {
            Remove-Item -LiteralPath $file -ErrorAction Ignore
        }
        Remove-Item -LiteralPath $folder -ErrorAction Ignore
    }

    It 'reads a file named with brackets through -LiteralPath, and refuses it through -Path' {
        (New-WgpuShader -LiteralPath $bracketed).Label | Should -Be $bracketed
        New-WgpuShader -Path $bracketed -ErrorAction SilentlyContinue -ErrorVariable failed
        $failed[0].FullyQualifiedErrorId | Should -BeLike 'WgpuShaderFile,*'
    }

    It 'takes the files Get-ChildItem pipes, by their exact names' {
        $results = @(Get-ChildItem -LiteralPath $folder -Filter '*.wgsl' | Sort-Object Name | Test-WgpuShader)
        ($results -join ',') | Should -Be 'False,True,True,True'
    }

    It 'makes a shader of each file Get-ChildItem pipes, labeled with its path' {
        $made = @(Get-ChildItem -LiteralPath $folder -Filter 'm*.wgsl' | Sort-Object Name | New-WgpuShader)
        ($made.Label -join ',') | Should -Be "$first,$second"
    }

    It 'takes a .NET FileInfo through its FullName' {
        ([IO.FileInfo]::new($bracketed) | Get-WgpuShaderInfo).Label | Should -Be $bracketed
    }

    It 'reads every file a wildcard matches, each labeled with its path' {
        $infos = @(Get-WgpuShaderInfo -Path (Join-Path $folder 'm*.wgsl'))
        $infos.Count | Should -Be 2
        ($infos | Where-Object Label -EQ $first).EntryPoints[0].Name | Should -Be 'main'
        ($infos | Where-Object Label -EQ $second).Overrides[0].Name | Should -Be 'gain'
        (Get-WgpuShaderInfo -Source $double).Label | Should -Be 'Wgpu shader'
    }

    It 'writes an error for a file that fails and goes on with the others' {
        $made = @(New-WgpuShader -Path (Join-Path $folder '*.wgsl') -ErrorAction SilentlyContinue -ErrorVariable failed)
        $made.Count | Should -Be 3
        $failed.Count | Should -Be 1
        $failed[0].Exception.Message | Should -BeLike '*line 2*'
    }

    It 'names the file each problem is in with -Diagnostics' {
        $found = @(Test-WgpuShader -LiteralPath $broken -Diagnostics)
        $found[0].Label | Should -Be $broken
        $found[0].Line | Should -Be 2
        (Test-WgpuShader -Source 'fn main( {' -Diagnostics)[0].Label | Should -Be 'Wgpu shader'
    }

    It 'notes the file on each translation' {
        $out = @(Get-ChildItem -LiteralPath $folder -Filter 'm*.wgsl' | Sort-Object Name | ConvertTo-WgpuShader -To Wgsl)
        $out.Count | Should -Be 2
        $out[0].Label | Should -Be $first
        $out[1].Label | Should -Be $second
        (ConvertTo-WgpuShader -Source $double -To Hlsl).Label | Should -Be 'Wgpu shader'
        (ConvertTo-WgpuShader -LiteralPath $first -To SpirV).Label | Should -Be $first
    }

    It 'refuses -OutFile for a wildcard that matches several files, and writes nothing' {
        $file = Join-Path $folder 'several.hlsl'
        ConvertTo-WgpuShader -Path (Join-Path $folder 'm*.wgsl') -To Hlsl -OutFile $file -ErrorAction SilentlyContinue -ErrorVariable failed
        $failed[0].FullyQualifiedErrorId | Should -BeLike 'WgpuOutFile,*'
        $failed[0].Exception.Message | Should -BeLike '-OutFile holds one shader, and * matches 2 files: *'
        Test-Path -LiteralPath $file | Should -BeFalse
    }

    It 'refuses a second piped shader for one -OutFile, and keeps the first' {
        $file = Join-Path $folder 'piped.hlsl'
        try {
            Get-ChildItem -LiteralPath $folder -Filter 'm*.wgsl' | Sort-Object Name | ConvertTo-WgpuShader -To Hlsl -OutFile $file -ErrorAction SilentlyContinue -ErrorVariable failed
            $failed.Count | Should -Be 1
            $failed[0].FullyQualifiedErrorId | Should -BeLike 'WgpuOutFile,*'
            [IO.File]::ReadAllText($file) | Should -Be (ConvertTo-WgpuShader -LiteralPath $first -To Hlsl)
        } finally {
            Remove-Item -LiteralPath $file -ErrorAction Ignore
        }
    }

    It 'reports a -LiteralPath that names no file as WgpuShaderFile, with PowerShell''s reason' {
        $missing = Join-Path $folder 'missing[1].wgsl'
        Test-WgpuShader -LiteralPath $missing -ErrorAction SilentlyContinue -ErrorVariable failed
        $failed[0].FullyQualifiedErrorId | Should -BeLike 'WgpuShaderFile,*'
        $failed[0].CategoryInfo.Category | Should -Be 'ObjectNotFound'
        $failed[0].Exception.Message | Should -Be "Cannot find path '$missing' because it does not exist."
    }
}
