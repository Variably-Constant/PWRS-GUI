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
    $script:double = @'
@group(0) @binding(0) var<storage, read_write> v: array<f32>;
@compute @workgroup_size(64)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    if (id.x < arrayLength(&v)) { v[id.x] = v[id.x] * 2.0; }
}
'@
    $script:add = @'
@group(0) @binding(0) var<storage, read_write> a: array<f32>;
@group(0) @binding(1) var<storage, read> b: array<f32>;
@compute @workgroup_size(64)
fn sum(@builtin(global_invocation_id) id: vec3<u32>) {
    if (id.x < arrayLength(&a)) { a[id.x] = a[id.x] + b[id.x]; }
}
'@
    $script:affine = @'
struct Params { scale: f32, offset: f32 }
@group(0) @binding(0) var<storage, read_write> v: array<f32>;
@group(0) @binding(1) var<uniform> p: Params;
@compute @workgroup_size(64)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    if (id.x < arrayLength(&v)) { v[id.x] = v[id.x] * p.scale + p.offset; }
}
'@
    $script:override = @'
override factor: f32 = 2.0;
@group(0) @binding(0) var<storage, read_write> v: array<f32>;
@compute @workgroup_size(64)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    if (id.x < arrayLength(&v)) { v[id.x] = v[id.x] * factor; }
}
'@
    $script:glsl = @'
#version 450
layout(local_size_x = 32) in;
layout(set = 0, binding = 0) buffer Data { float values[]; };
void main() {
    uint i = gl_GlobalInvocationID.x;
    if (i < values.length()) { values[i] += 1.0; }
}
'@
}

Describe 'Invoke-WgpuCompute' {
    It 'runs a shader over an array and writes the array back' -Skip:$noGpu {
        (Invoke-WgpuCompute $double ([float[]](1..4))) -join ',' | Should -Be '2,4,6,8'
    }

    It 'sizes the dispatch from the array when no size is given' -Skip:$noGpu {
        $out = Invoke-WgpuCompute $double ([float[]](1..1000))
        $out.Count | Should -Be 1000
        $out[999] | Should -Be 2000
    }

    It 'takes -Invocations and -Workgroups' -Skip:$noGpu {
        (Invoke-WgpuCompute $double ([float[]](1..100)) -Invocations 100)[99] | Should -Be 200
        $out = Invoke-WgpuCompute $double ([float[]](1..100)) -Workgroups 1
        $out[63] | Should -Be 128
        $out[64] | Should -Be 65
    }

    It 'converts an array to the element type the shader declares' -Skip:$noGpu {
        $out = Invoke-WgpuCompute $double ([int[]](1, 2, 3))
        $out.GetType().Name | Should -Be 'Single[]'
        $out -join ',' | Should -Be '2,4,6'
    }

    It 'binds by position, by name, and by group:binding' -Skip:$noGpu {
        (Invoke-WgpuCompute $add @([float[]](1, 2), [float[]](10, 20))) -join ',' | Should -Be '11,22'
        (Invoke-WgpuCompute $add @{ b = [float[]](10, 20); a = [float[]](1, 2) }) -join ',' | Should -Be '11,22'
        (Invoke-WgpuCompute $add @{ '0:0' = [float[]](1, 2); '0:1' = [float[]](5, 5) }) -join ',' | Should -Be '6,7'
    }

    It 'writes back only the bindings the shader may write' -Skip:$noGpu {
        @(Invoke-WgpuCompute $add @([float[]](1, 2), [float[]](10, 20))).Count | Should -Be 1
    }

    It 'packs a struct uniform from a hashtable of its members' -Skip:$noGpu {
        $out = Invoke-WgpuCompute $affine @{ v = [float[]](1, 2, 3); p = @{ scale = 10; offset = 0.5 } }
        $out -join ',' | Should -Be '10.5,20.5,30.5'
    }

    It 'refuses a member the struct does not have' -Skip:$noGpu {
        { Invoke-WgpuCompute $affine @{ v = [float[]](1); p = @{ sacle = 1 } } } | Should -Throw '*no member sacle*'
    }

    It 'refuses a resource that is $null, naming its binding' -Skip:$noGpu {
        { Invoke-WgpuCompute $affine @{ v = $null; p = @{ scale = 1 } } } | Should -Throw '*no resource was given for v (0:0): its value is $null*'
    }

    It 'sets override constants' -Skip:$noGpu {
        (Invoke-WgpuCompute $override ([float[]](1, 2)) -Constant @{ factor = 5 }) -join ',' | Should -Be '5,10'
        (Invoke-WgpuCompute $override ([float[]](1, 2))) -join ',' | Should -Be '2,4'
    }

    It 'runs GLSL compute shaders' -Skip:$noGpu {
        (Invoke-WgpuCompute $glsl ([float[]](1, 2, 3))) -join ',' | Should -Be '2,3,4'
    }

    It 'runs SPIR-V shaders' -Skip:$noGpu {
        $shader = New-WgpuShader -SpirV (ConvertTo-WgpuShader -Source $double -To SpirV)
        (Invoke-WgpuCompute $shader ([float[]](3, 4))) -join ',' | Should -Be '6,8'
    }

    It 'binds SPIR-V written here by the names of the shader it came from' -Skip:$noGpu {
        $shader = New-WgpuShader -SpirV (ConvertTo-WgpuShader -Source $affine -To SpirV)
        $out = Invoke-WgpuCompute $shader @{ v = [float[]](1, 2, 3); p = @{ scale = 10; offset = 0.5 } }
        $out -join ',' | Should -Be '10.5,20.5,30.5'
    }

    It 'leaves a Wgpu.Buffer on the GPU' -Skip:$noGpu {
        $buffer = New-WgpuBuffer ([float[]](1..4))
        @(Invoke-WgpuCompute $double $buffer).Count | Should -Be 0
        Invoke-WgpuCompute $double $buffer
        (Read-WgpuBuffer $buffer) -join ',' | Should -Be '4,8,12,16'
    }

    It 'dispatches the counts a buffer holds with -Indirect' -Skip:$noGpu {
        $counts = New-WgpuBuffer ([uint32[]](1, 1, 1))
        $out = Invoke-WgpuCompute $double ([float[]](1..100)) -Indirect $counts
        $out[63] | Should -Be 128
        $out[64] | Should -Be 65
    }

    It 'writes a result with every output and the time taken with -PassThru' -Skip:$noGpu {
        $result = Invoke-WgpuCompute $add @{ a = [float[]](1, 2); b = [float[]](3, 4) } -Entry sum -PassThru
        $result.Outputs['a'] -join ',' | Should -Be '4,6'
        $result.CpuTimeMs | Should -BeGreaterThan 0
        if ((Get-WgpuDevice).Features -contains 'TimestampQuery') {
            $result.GpuTimeNs | Should -BeGreaterOrEqual 0
        }
    }

    It 'reports a missing binding by name' -Skip:$noGpu {
        { Invoke-WgpuCompute $add @{ a = [float[]](1) } } | Should -Throw '*no resource was given for b*'
    }

    It 'reports an invalid shader with its line' -Skip:$noGpu {
        { Invoke-WgpuCompute "fn main() {`n  let x: f32 = 1u;`n}" ([float[]](1)) } | Should -Throw '*line 2*'
    }

    It 'runs a pipeline built once, many times' -Skip:$noGpu {
        $pipeline = New-WgpuComputePipeline $double
        $pipeline.Bindings[0].Name | Should -Be 'v'
        $buffer = New-WgpuBuffer ([float[]](1, 1))
        1..3 | ForEach-Object { Invoke-WgpuCompute $pipeline $buffer }
        (Read-WgpuBuffer $buffer) -join ',' | Should -Be '8,8'
    }

    It 'builds a pipeline from the shader piped to it, as text or as a shader object' -Skip:$noGpu {
        ($double | New-WgpuComputePipeline).Bindings[0].Name | Should -Be 'v'
        (New-WgpuShader -Source $double | New-WgpuComputePipeline).Bindings[0].Name | Should -Be 'v'
    }

    It 'runs through the device''s Run method' -Skip:$noGpu {
        $gpu = Get-WgpuDevice
        $buffer = $gpu.Upload([float[]](1..8))
        $gpu.Run($double, $buffer, 1, $null, $null, $null)
        $gpu.Download($buffer) -join ',' | Should -Be '2,4,6,8,10,12,14,16'
    }

    It 'runs through RunAsync and DispatchAsync, which return tasks' -Skip:$noGpu {
        $gpu = Get-WgpuDevice
        $buffer = $gpu.Upload([float[]](1..4))
        $run = $gpu.RunAsync($double, $buffer, 1, $null, $null, $null)
        $run | Should -BeOfType ([Threading.Tasks.Task])
        $run.GetAwaiter().GetResult()
        $run.Status | Should -Be 'RanToCompletion'
        $pipeline = New-WgpuComputePipeline $double
        $pipeline.DispatchAsync($buffer, 1, $null, $null).GetAwaiter().GetResult()
        $gpu.DownloadAsync($buffer).GetAwaiter().GetResult() -join ',' | Should -Be '4,8,12,16'
    }
}

Describe 'Encoders, passes and queries' {
    It 'records a pass, finishes it and submits it' -Skip:$noGpu {
        $pipeline = New-WgpuComputePipeline $add
        $a = New-WgpuBuffer ([float[]](1, 2, 3))
        $b = New-WgpuBuffer ([float[]](4, 5, 6))
        $group = $pipeline | New-WgpuBindGroup -Resource @{ a = $a; b = $b }
        $encoder = New-WgpuCommandEncoder
        $pass = $encoder.BeginComputePass($null, $null, $null, $null)
        $pass.SetPipeline($pipeline)
        $pass.SetBindGroup(0, $group, $null)
        $pass.Dispatch(1, $null, $null)
        $pass.End()
        $encoder.Finish() | Submit-WgpuCommand
        (Read-WgpuBuffer $a) -join ',' | Should -Be '5,7,9'
        $encoder.IsFinished | Should -BeTrue
    }

    It 'submits through SubmitAsync, which returns a task' -Skip:$noGpu {
        $buffer = New-WgpuBuffer ([float[]](1, 2, 3, 4))
        $encoder = New-WgpuCommandEncoder
        $encoder.ClearBuffer($buffer, $null, $null)
        (Get-WgpuDevice).SubmitAsync(@($encoder.Finish())).GetAwaiter().GetResult()
        (Read-WgpuBuffer $buffer) -join ',' | Should -Be '0,0,0,0'
    }

    It 'refuses to submit a command buffer twice, naming its encoder' -Skip:$noGpu {
        $encoder = New-WgpuCommandEncoder 'twice'
        $commands = $encoder.Finish()
        Submit-WgpuCommand $commands
        { Submit-WgpuCommand $commands } | Should -Throw "the command buffer of encoder 'twice' was already submitted; record the work again"
    }

    It 'makes a query set of the kind and count given' -Skip:$noGpu {
        $set = New-WgpuQuerySet Occlusion 4 -Label 'seen'
        $set.Kind | Should -Be 'Occlusion'
        $set.Count | Should -Be 4
        $set.Label | Should -Be 'seen'
    }

    It 'times a pass with timestamp queries' -Skip:$noGpu {
        if ((Get-WgpuDevice).Features -notcontains 'TimestampQuery') { Set-ItResult -Skipped -Because 'the device has no TimestampQuery'; return }
        $pipeline = New-WgpuComputePipeline $double
        $group = New-WgpuBindGroup $pipeline (New-WgpuBuffer ([float[]](1..1024)))
        $times = New-WgpuQuerySet Timestamp 2
        $encoder = New-WgpuCommandEncoder
        $pass = $encoder.BeginComputePass('timed', $times, 0, 1)
        $pass.SetPipeline($pipeline)
        $pass.SetBindGroup(0, $group, $null)
        $pass.Dispatch(16, $null, $null)
        $pass.End()
        $encoder.Finish() | Submit-WgpuCommand
        $ns = $times.ReadNanoseconds($null, $null)
        $ns.Count | Should -Be 2
        ($ns[1] - $ns[0]) | Should -BeGreaterOrEqual 0
        $times.ReadAsync($null, $null).GetAwaiter().GetResult().Count | Should -Be 2
        ($times.ReadNanosecondsAsync($null, $null).GetAwaiter().GetResult() -join ',') | Should -Be ($ns -join ',')
    }
}

Describe 'Invoke-WgpuMap' {
    It 'runs an expression over an array' -Skip:$noGpu {
        (Invoke-WgpuMap 'x * x' ([float[]](1..5))) -join ',' | Should -Be '1,4,9,16,25'
    }

    It 'takes numbers from the pipeline' -Skip:$noGpu {
        (1..5 | Invoke-WgpuMap 'x + 1') -join ',' | Should -Be '2,3,4,5,6'
    }

    It 'reads array and number variables' -Skip:$noGpu {
        $out = Invoke-WgpuMap 'a * x + b' ([float[]](1, 2, 3)) -Variable @{ a = 2.5; b = [float[]](10, 20, 30) }
        $out -join ',' | Should -Be '12.5,25,37.5'
    }

    It 'reads the index and writes the -ElementType given' -Skip:$noGpu {
        $out = Invoke-WgpuMap 'u32(i) * 3u' ([float[]](0, 0, 0, 0)) -ElementType UInt32
        $out.GetType().Name | Should -Be 'UInt32[]'
        $out -join ',' | Should -Be '0,3,6,9'
    }

    It 'leaves the result on the GPU with -AsBuffer' -Skip:$noGpu {
        $buffer = Invoke-WgpuMap 'x - 1.0' ([float[]](1, 2)) -AsBuffer
        $buffer.GetType().FullName | Should -Be 'Wgpu.Buffer'
        (Read-WgpuBuffer $buffer) -join ',' | Should -Be '0,1'
    }

    It 'refuses a variable name WGSL cannot use' -Skip:$noGpu {
        { Invoke-WgpuMap 'x' ([float[]](1)) -Variable @{ 'not a name' = 1 } } | Should -Throw '*cannot name a variable*'
    }
}

Describe 'Measure-WgpuArray' {
    It 'sums, averages and finds the extremes of floats' -Skip:$noGpu {
        $m = Measure-WgpuArray ([float[]](1..1000))
        $m.Count | Should -Be 1000
        $m.Sum | Should -Be 500500
        $m.Average | Should -Be 500.5
        $m.Minimum | Should -Be 1
        $m.Maximum | Should -Be 1000
    }

    It 'sums integers exactly past 32 bits' -Skip:$noGpu {
        $big = [int[]]::new(100000)
        for ($i = 0; $i -lt $big.Length; $i++) { $big[$i] = 2000000000 }
        (Measure-WgpuArray $big).Sum | Should -Be 200000000000000
        (Measure-WgpuArray ([uint32[]](4000000000, 4000000000))).Sum | Should -Be 8000000000
    }

    It 'handles negative integers' -Skip:$noGpu {
        $m = Measure-WgpuArray ([int[]](-5..5))
        $m.Sum | Should -Be 0
        $m.Minimum | Should -Be -5
        $m.Maximum | Should -Be 5
    }

    It 'measures a buffer in place on the GPU' -Skip:$noGpu {
        $buffer = New-WgpuBuffer ([float[]](3, 1, 2))
        ($buffer | Measure-WgpuArray).Maximum | Should -Be 3
    }

    It 'takes numbers from the pipeline' -Skip:$noGpu {
        (1..100 | Measure-WgpuArray).Sum | Should -Be 5050
    }
}
