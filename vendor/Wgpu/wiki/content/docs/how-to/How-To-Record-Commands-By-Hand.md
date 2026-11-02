---
title: How To Record Commands By Hand
weight: 15
---

Run 64 generations of Conway's Game of Life on the GPU in one submission: two buffers of cells that take turns, the rule given as immediate data and the workgroup counts read from a buffer, a compute pass of dispatches between copies that keep a snapshot of every 16th generation, and the snapshots read back once at the end and drawn side by side. `Invoke-WgpuCompute` does every step of one dispatch, from building the pipeline to reading the result; recording by hand does each step once and keeps the cells on the GPU throughout. [Recording commands](Recording.md) has each part on its own.

## One generation

This shader works out one generation of a 64 by 64 grid whose edges wrap around, reading the cells from one buffer and writing the next generation into another. The rule comes in as immediate data, a few bytes a pass sets without a buffer: a bit for each number of live neighbors that brings a dead cell to life, and one for each that keeps a live cell alive. Conway's rule, life with exactly three and survival with two or three, is 8 and 12:

```powershell
PS> Get-Content ./life.wgsl
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
PS> $life = New-WgpuComputePipeline (Get-Content ./life.wgsl -Raw)
PS> $life.ImmediateSize
8
```

The rule is a struct, since naga's HLSL writer, which wgpu's Direct3D 12 backend uses, takes immediate data only as a struct.

## Two buffers that take turns

The grid starts with an R-pentomino, five cells that grow into a crowd. One buffer holds it and another takes the next generation; a bind group binds them one way round and a second the other, so the generations go from one buffer to the other and back with no copy between them:

```powershell
PS> $side = 64
PS> $cells = [uint32[]]::new($side * $side)
PS> foreach ($cell in @(32, 31), @(33, 31), @(31, 32), @(32, 32), @(32, 33)) { $cells[$cell[1] * $side + $cell[0]] = 1 }
PS> $a = New-WgpuBuffer $cells
PS> $b = New-WgpuBuffer -Length $cells.Length -ElementType UInt32
PS> $aToB = New-WgpuBindGroup $life @{ current = $a; next = $b }
PS> $bToA = New-WgpuBindGroup $life @{ current = $b; next = $a }
```

## Record the generations

A command encoder records, and a compute pass inside it takes the pipeline and the rule once, then a bind group and a dispatch for each generation, each dispatch seeing what the one before wrote. The dispatches read their workgroup counts, 8 by 8 workgroups of 8 by 8 cells, from a buffer on the GPU, where a shader could also have written them. After 16 generations, an even number, the newest is in the first buffer again, and a copy outside the pass keeps it in a history buffer, one grid after another. Nothing runs until the recording is submitted:

```powershell
PS> $history = New-WgpuBuffer -Length ($cells.Length * 5) -ElementType UInt32
PS> $counts = New-WgpuBuffer ([uint32[]](8, 8, 1))
PS> $encoder = New-WgpuCommandEncoder -Label life
PS> $encoder.CopyBufferToBuffer($a, 0, $history, 0)
PS> for ($snapshot = 1; $snapshot -le 4; $snapshot++) {
>>     $pass = $encoder.BeginComputePass()
>>     $pass.SetPipeline($life)
>>     $pass.SetImmediates(0, [uint32[]](8, 12))
>>     for ($generation = 0; $generation -lt 16; $generation++) {
>>         $pass.SetBindGroup(0, $(if ($generation % 2 -eq 0) { $aToB } else { $bToA }))
>>         $pass.DispatchIndirect($counts, 0)
>>     }
>>     $pass.End()
>>     $encoder.CopyBufferToBuffer($a, 0, $history, $snapshot * $cells.Length * 4)
>> }
PS> $commands = $encoder.Finish()
PS> Submit-WgpuCommand $commands
```

`Finish` turns the recording into a command buffer, and `Submit-WgpuCommand` runs it and waits. 64 dispatches and 5 copies went to the GPU in one submission.

## Read the snapshots

The history comes back once. The population of each snapshot, generations 0, 16, 32, 48 and 64:

```powershell
PS> $all = [uint32[]](Read-WgpuBuffer $history)
PS> 0..4 | ForEach-Object { ($all[($_ * 4096)..($_ * 4096 + 4095)] | Measure-Object -Sum).Sum }
5
23
32
45
69
```

Each snapshot drawn as a 64 by 64 picture, the five side by side and doubled in size:

```powershell
PS> $pixels = [byte[]]::new($all.Length * 4)
PS> for ($i = 0; $i -lt $all.Length; $i++) {
>>     $grid = [math]::Floor($i / 4096); $cell = $i % 4096
>>     $at = ([math]::Floor($cell / $side) * 5 * $side + $grid * $side + $cell % $side) * 4
>>     $shade = if ($all[$i]) { 30 } else { 240 }
>>     $pixels[$at] = $shade; $pixels[$at + 1] = $shade; $pixels[$at + 2] = $shade; $pixels[$at + 3] = 255
>> }
PS> New-WgpuTexture (5 * $side) $side -Data $pixels | Resize-WgpuTexture -Scale 2 -Filter Nearest | Export-WgpuImage -Path ./life.png
```

![Five snapshots of the Game of Life side by side: the R-pentomino's five cells, then the crowd it grows into at generations 16, 32, 48 and 64](/images/life.png)

## A command buffer runs once

A command buffer is spent once it runs; to run the work again, record it again:

```powershell
PS> Submit-WgpuCommand $commands
Submit-WgpuCommand: the command buffer of encoder 'life' was already submitted; record the work again
```

## Where a mistake is reported

A pass and an encoder refuse a command once they have ended or finished. Anything else is checked by wgpu when the encoder finishes, so a mistake made in a pass surfaces at `Finish`. A dispatch with no pipeline set:

```powershell
PS> $encoder = New-WgpuCommandEncoder
PS> $pass = $encoder.BeginComputePass()
PS> $pass.Dispatch(1)
PS> $pass.End()
PS> $encoder.Finish()
MethodInvocationException: Exception calling "Finish" with "0" argument(s): "[WgpuFinish] the recorded commands are invalid: Validation Error

Caused by:
  In a CommandEncoder, label = 'Wgpu encoder'
    In a dispatch command, indirect:false
      Compute pipeline must be set
"
```

[How To Time And Count GPU Work](How-To-Time-And-Count-GPU-Work.md) times a pass with timestamp queries, [How To Draw In A Render Pass Of Your Own](How-To-Draw-In-A-Render-Pass-Of-Your-Own.md) records a render pass, and [How To Wait Without Blocking](How-To-Wait-Without-Blocking.md) submits without waiting.
