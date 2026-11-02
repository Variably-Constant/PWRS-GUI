---
title: How To Use Threads And Runspaces
weight: 18
---

Make thumbnails of a folder of pictures with several runspaces at once, all on one GPU: resize them in parallel, brighten them with a compute pipeline built once and shared by every runspace, do the same in Windows PowerShell with a runspace pool, and know where a device's events arrive. The pictures are [scene.png](/images/scene.png), [badge.png](/images/badge.png) and [sprite.png](/images/sprite.png). [Tasks and threads](Tasks.md) has each part on its own.

## One device for every runspace

The module keeps its devices, and everything made on them, in the process rather than in a runspace. Every runspace of one process uses the same default device, and a buffer, texture or pipeline made in one works in all of them:

```powershell
PS> Get-ChildItem ./*.png | Select-Object Name, Length
Name       Length
----       ------
badge.png     966
scene.png    4063
sprite.png  10058

PS> $shared = New-WgpuBuffer ([float[]](1..8))
PS> $runspace = [powershell]::Create()
PS> $null = $runspace.AddScript({ param($buffer) "$((Get-WgpuDevice).Label): $((Read-WgpuBuffer $buffer) -join ',')" }).AddArgument($shared)
PS> $runspace.Invoke()
Wgpu device 1: 1,2,3,4,5,6,7,8

PS> $runspace.Dispose()
```

The runspace loads the module by name, as any session does, and reads the buffer the main runspace made, on the device it opened. Disposing it changes nothing for the main runspace: the module releases the default device and its instances only once no runspace of the process has the module, so a runspace that ends, a parallel one or a thread job's included, leaves them to the others.

## Resize them in parallel

In PowerShell 7, `ForEach-Object -Parallel` runs its script block for each input in runspaces of its own, at the same time, here three at a time. `$using:` hands each one a variable of the main runspace:

<!-- pwsh 7 -->
```powershell
PS> $null = New-Item -ItemType Directory -Path ./thumbs
PS> Get-ChildItem ./*.png | ForEach-Object -Parallel {
>>     Import-WgpuImage $_.FullName | Resize-WgpuTexture -MaxSize 64 | Export-WgpuImage -Path (Join-Path $using:PWD "thumbs/$($_.Name)")
>> } -ThrottleLimit 3
PS> Get-ChildItem ./thumbs | ForEach-Object { $thumb = Import-WgpuImage $_.FullName; '{0}: {1} x {2}' -f $_.Name, $thumb.Width, $thumb.Height }
badge.png: 64 x 64
scene.png: 64 x 40
sprite.png: 64 x 64
```

## Share a pipeline

A pipeline compiles its shader once, which can take longer than running it. Built in the main runspace, it runs in every parallel one through `$using:`. This one brightens a picture by a quarter:

<!-- pwsh 7 -->
```powershell
PS> Get-Content ./brighter.wgsl
@group(0) @binding(0) var source: texture_2d<f32>;
@group(0) @binding(1) var result: texture_storage_2d<rgba8unorm, write>;
@compute @workgroup_size(8, 8)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    let size = textureDimensions(source);
    if (id.x >= size.x || id.y >= size.y) { return; }
    let c = textureLoad(source, vec2<i32>(id.xy), 0);
    textureStore(result, vec2<i32>(id.xy), vec4<f32>(min(c.rgb * 1.25, vec3<f32>(1.0)), c.a));
}
PS> $brighter = New-WgpuComputePipeline (Get-Content ./brighter.wgsl -Raw)
PS> Get-ChildItem ./thumbs/*.png | ForEach-Object -Parallel {
>>     $picture = Import-WgpuImage $_.FullName
>>     $bright = New-WgpuTexture $picture.Width $picture.Height -Usage StorageBinding, CopySrc
>>     Invoke-WgpuCompute $using:brighter @{ source = $picture; result = $bright } -Invocations $picture.Width, $picture.Height
>>     Export-WgpuImage $bright (Join-Path $using:PWD "thumbs/bright-$($_.Name)")
>> } -ThrottleLimit 3
PS> (Get-ChildItem ./thumbs/bright-*.png).Name
bright-badge.png
bright-scene.png
bright-sprite.png
```

## In Windows PowerShell

Windows PowerShell has no `-Parallel`. A runspace pool runs scripts at the same time in either host, here at most three, making the same thumbnails:

```powershell
PS> $null = New-Item -ItemType Directory -Path ./pooled
PS> $pool = [runspacefactory]::CreateRunspacePool(1, 3)
PS> $pool.Open()
PS> $work = foreach ($file in Get-ChildItem ./*.png) {
>>     $shell = [powershell]::Create()
>>     $shell.RunspacePool = $pool
>>     $null = $shell.AddScript({ param($path, $folder) Import-WgpuImage $path | Resize-WgpuTexture -MaxSize 64 | Export-WgpuImage -Path (Join-Path $folder (Split-Path -Leaf $path)) }).AddArgument($file.FullName).AddArgument((Join-Path $PWD pooled))
>>     [pscustomobject]@{ Shell = $shell; Running = $shell.BeginInvoke() }
>> }
PS> $work | ForEach-Object { $_.Shell.EndInvoke($_.Running); $_.Shell.Dispose() }
PS> $pool.Close()
PS> (Get-ChildItem ./pooled).Name
badge.png
scene.png
sprite.png
```

Thread jobs, from `Start-ThreadJob`, work the same way: PowerShell 7 has it, and Windows PowerShell gets it from the ThreadJob module on the PowerShell Gallery.

## Where a device's events arrive

A device raises its engine events, `Wgpu.DeviceLost` and `Wgpu.UncapturedError`, in the runspace that opened it, and the default device in the runspace whose call opened it. Calling `Get-WgpuDevice` in the main runspace before starting any threads opens the default device there, so its events arrive there. A device opened with `[Wgpu.Device]::new` raises none. [How To Handle Errors](How-To-Handle-Errors.md) shows what the events carry.

## OpenGL on Windows

On Windows, NVIDIA's driver deadlocks the process when a thread that used OpenGL ends while another thread opens or closes a device, and wgpu uses OpenGL on every thread that looks for OpenGL adapters. So on Windows the module looks for adapters through Vulkan and Direct3D 12, and through OpenGL only when `-Backend Gl` asks for it. A script that asks for it there keeps OpenGL out of thread jobs and parallel runspaces, whose threads end with their work.
