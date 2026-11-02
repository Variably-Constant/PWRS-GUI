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
    $script:fill = @'
@group(0) @binding(0) var target_texture: texture_storage_2d<rgba8unorm, write>;
@compute @workgroup_size(8, 8)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    let size = textureDimensions(target_texture);
    if (id.x < size.x && id.y < size.y) {
        textureStore(target_texture, vec2<i32>(id.xy), vec4<f32>(f32(id.x) / 255.0, f32(id.y) / 255.0, 0.0, 1.0));
    }
}
'@
    $script:sample = @'
@group(0) @binding(0) var source: texture_2d<f32>;
@group(0) @binding(1) var smp: sampler;
@group(0) @binding(2) var<storage, read_write> red: array<f32>;
@compute @workgroup_size(1)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    red[id.x] = textureSampleLevel(source, smp, vec2<f32>((f32(id.x) + 0.5) / 4.0, 0.5), 0.0).r;
}
'@
}

Describe 'New-WgpuTexture, Write-WgpuTexture and Read-WgpuTexture' {
    It 'round-trips Rgba8Unorm texels as bytes' -Skip:$noGpu {
        $pixels = [byte[]](0..63)
        $texture = New-WgpuTexture 4 4 -Data $pixels
        $texture.Format | Should -Be 'Rgba8Unorm'
        $texture.Width | Should -Be 4
        (Read-WgpuTexture $texture) -join ',' | Should -Be ($pixels -join ',')
    }

    It 'reads the texture piped to it' -Skip:$noGpu {
        $pixels = [byte[]](0..15)
        $texture = New-WgpuTexture 2 2 -Data $pixels
        ($texture | Read-WgpuTexture) -join ',' | Should -Be ($pixels -join ',')
    }

    It 'round-trips R32Float texels as floats' -Skip:$noGpu {
        $texture = New-WgpuTexture 3 2 -Format R32Float
        Write-WgpuTexture $texture ([float[]](1.5, 2, 3, 4, 5, 6.25))
        $back = Read-WgpuTexture $texture
        $back.GetType().Name | Should -Be 'Single[]'
        $back -join ',' | Should -Be '1.5,2,3,4,5,6.25'
    }

    It 'reads texels through ReadAsync, which returns a task' -Skip:$noGpu {
        $pixels = [byte[]](0..63)
        $texture = New-WgpuTexture 4 4 -Data $pixels
        $texture.ReadAsync($null, $null, $null).GetAwaiter().GetResult() -join ',' | Should -Be ($pixels -join ',')
    }

    It 'refuses texels that do not fill the level' -Skip:$noGpu {
        $texture = New-WgpuTexture 4 4
        { Write-WgpuTexture $texture ([byte[]](1, 2, 3)) } | Should -Throw '*holds 64 bytes*'
    }

    It 'round-trips the depth of Depth16Unorm and the stencil of Depth24PlusStencil8' -Skip:$noGpu {
        $depth = New-WgpuTexture 2 2 -Format Depth16Unorm
        Write-WgpuTexture $depth ([uint16[]](0, 16384, 32768, 65535)) -Aspect DepthOnly
        (Read-WgpuTexture $depth -Aspect DepthOnly) -join ',' | Should -Be '0,16384,32768,65535'
        $stencil = New-WgpuTexture 2 2 -Format Depth24PlusStencil8
        Write-WgpuTexture $stencil ([byte[]](1, 2, 3, 4)) -Aspect StencilOnly
        (Read-WgpuTexture $stencil -Aspect StencilOnly) -join ',' | Should -Be '1,2,3,4'
    }

    It 'gives a depth format no storage usage on any backend, and refuses one' -Skip:$noGpu {
        foreach ($backend in @(Get-WgpuAdapter | ForEach-Object Backend | Select-Object -Unique)) {
            $gpu = Open-WgpuDevice -Backend $backend
            try {
                foreach ($format in 'Depth16Unorm', 'Depth32Float') {
                    (Get-WgpuTextureFormat $format -Device $gpu | Where-Object Name -EQ $format).AllowedUsages | Should -Not -Contain 'StorageBinding' -Because "$format on $backend"
                    (New-WgpuTexture 2 2 -Format $format -Device $gpu).Usage | Should -Not -Contain 'StorageBinding' -Because "$format on $backend"
                    $gpu.Status().IsLost | Should -BeFalse -Because "a $format texture was created on $backend"
                    { New-WgpuTexture 2 2 -Format $format -Usage StorageBinding -Device $gpu } | Should -Throw "$format is a depth or stencil format, which cannot be a storage binding"
                }
            } finally {
                $gpu | Close-WgpuDevice
            }
        }
    }

    It 'refuses the depth of Depth24PlusStencil8 and names the aspect that can be read' -Skip:$noGpu {
        $stencil = New-WgpuTexture 2 2 -Format Depth24PlusStencil8
        Read-WgpuTexture $stencil -Aspect DepthOnly -ErrorAction SilentlyContinue -ErrorVariable failed
        $failed[0].FullyQualifiedErrorId | Should -BeLike 'WgpuAspect,*'
        $failed[0].Exception.Message | Should -Be 'only the stencil of Depth24PlusStencil8 can be read or written, with -Aspect StencilOnly: its depth has no layout outside the GPU'
        { Write-WgpuTexture (New-WgpuTexture 2 2 -Format Depth24Plus) ([float[]](0, 0, 0, 0)) -Aspect DepthOnly } | Should -Throw 'the depth of Depth24Plus has no layout outside the GPU*'
    }

    It 'views a texture as another format its -ViewFormat allows' -Skip:$noGpu {
        $texture = New-WgpuTexture 2 2 -ViewFormat Rgba8UnormSrgb
        $texture.Usage | Should -Contain 'StorageBinding'
        $texture.CreateView('Rgba8UnormSrgb').Format | Should -Be 'Rgba8UnormSrgb'
    }

    It 'refuses a format wgpu does not have' -Skip:$noGpu {
        { New-WgpuTexture 4 4 -Format Rgba9Unorm } | Should -Throw '*no texture format named*'
    }
}

Describe 'NV12 and P010 planes' {
    It 'writes and reads each plane of NV12 at its own size, layers too, and views a plane in its format' -Skip:$noGpu {
        if ((Get-WgpuDevice).Features -notcontains 'TextureFormatNv12') { Set-ItResult -Skipped -Because 'the device has no TextureFormatNv12'; return }
        $nv12 = New-WgpuTexture 4 4 -Depth 2 -Format NV12
        $luma = [byte[]](0..15 | ForEach-Object { $_ * 10 })
        $chroma = [byte[]](0..7 | ForEach-Object { 200 - $_ })
        Write-WgpuTexture $nv12 $luma -Aspect Plane0 -Layer 1
        Write-WgpuTexture $nv12 $chroma -Aspect Plane1 -Layer 1
        (Read-WgpuTexture $nv12 -Aspect Plane0 -Layer 1) -join ',' | Should -Be ($luma -join ',')
        (Read-WgpuTexture $nv12 -Aspect Plane1 -Layer 1) -join ',' | Should -Be ($chroma -join ',')
        { Write-WgpuTexture $nv12 $luma -Aspect Plane1 } | Should -Throw '*holds 8 bytes*'
        $nv12.CreateView($null, $null, $null, $null, $null, $null, 'Plane1').Format | Should -Be 'Rg8Unorm'
    }

    It 'writes and reads each plane of P010 in 16-bit values' -Skip:$noGpu {
        if ((Get-WgpuDevice).Features -notcontains 'TextureFormatP010') { Set-ItResult -Skipped -Because 'the device has no TextureFormatP010'; return }
        $p010 = New-WgpuTexture 4 4 -Format P010
        $luma = [uint16[]](0..15 | ForEach-Object { $_ * 1000 })
        $chroma = [uint16[]](0..7 | ForEach-Object { 60000 - $_ * 7 })
        Write-WgpuTexture $p010 $luma -Aspect Plane0
        Write-WgpuTexture $p010 $chroma -Aspect Plane1
        (Read-WgpuTexture $p010 -Aspect Plane0) -join ',' | Should -Be ($luma -join ',')
        (Read-WgpuTexture $p010 -Aspect Plane1) -join ',' | Should -Be ($chroma -join ',')
        (Measure-WgpuTexture $p010 | ForEach-Object Channel) -join ',' | Should -Be 'Red,Green,Blue'
    }

    It 'refuses a plane left out or named where there is none, and points the commands that read whole textures to the planes' -Skip:$noGpu {
        if ((Get-WgpuDevice).Features -notcontains 'TextureFormatNv12') { Set-ItResult -Skipped -Because 'the device has no TextureFormatNv12'; return }
        $nv12 = New-WgpuTexture 4 4 -Format NV12
        Read-WgpuTexture $nv12 -ErrorAction SilentlyContinue -ErrorVariable failed
        $failed[0].FullyQualifiedErrorId | Should -BeLike 'WgpuAspect,*'
        $failed[0].Exception.Message | Should -Be 'NV12 keeps its texels in planes: give -Aspect Plane0 for its luma or Plane1 for its chroma'
        { Read-WgpuTexture (New-WgpuTexture 2 2) -Aspect Plane0 } | Should -Throw 'Rgba8Unorm has no planes: -Aspect Plane0 and Plane1 name the planes of NV12 and P010'
        { Export-WgpuImage $nv12 (Join-Path $TestDrive 'nv12.png') } | Should -Throw 'NV12 keeps its texels in planes, which are read one at a time: Read-WgpuTexture reads its luma with -Aspect Plane0 and its chroma with -Aspect Plane1'
    }

    It 'refuses an NV12 texture of more than one mip level or of three dimensions, and keeps the device' -Skip:$noGpu {
        if ((Get-WgpuDevice).Features -notcontains 'TextureFormatNv12') { Set-ItResult -Skipped -Because 'the device has no TextureFormatNv12'; return }
        { New-WgpuTexture 16 16 -Format NV12 -MipLevels 2 } | Should -Throw '*Multi-planar texture (NV12) mip level count (2) must be 1*'
        { New-WgpuTexture 16 16 -Depth 2 -Dimension D3 -Format NV12 } | Should -Throw '*Multi-planar texture (NV12) can''t be created as D3*'
        (Get-WgpuDevice).Status().IsLost | Should -BeFalse
    }

    It 'compares and measures NV12 planes as channels, and draws a chroma error over the 2 by 2 texels that share it' -Skip:$noGpu {
        if ((Get-WgpuDevice).Features -notcontains 'TextureFormatNv12') { Set-ItResult -Skipped -Because 'the device has no TextureFormatNv12'; return }
        $chroma = [byte[]](@(50, 200) * 64)
        $reference = New-WgpuTexture 16 16 -Format NV12
        Write-WgpuTexture $reference ([byte[]](@(100) * 256)) -Aspect Plane0
        Write-WgpuTexture $reference $chroma -Aspect Plane1
        # The texture's luma is 1 higher, and the Blue of chroma texel (3, 2)
        # is 2 higher: byte 39 is that texel's second value.
        $changed = [byte[]]$chroma.Clone()
        $changed[39] = 202
        $texture = New-WgpuTexture 16 16 -Format NV12
        Write-WgpuTexture $texture ([byte[]](@(101) * 256)) -Aspect Plane0
        Write-WgpuTexture $texture $changed -Aspect Plane1
        $rows = @(Compare-WgpuTexture $reference $texture)
        ($rows | ForEach-Object { '{0} {1}' -f $_.Channel, ($null -ne $_.SSIM) }) -join ', ' | Should -Be 'Red True, Green False, Blue False'
        $rows[1].PSNR | Should -Be ([double]::PositiveInfinity)
        (Measure-WgpuTexture $reference | ForEach-Object { '{0} {1}' -f $_.Channel, ($_.Histogram | Measure-Object -Sum).Sum }) -join ', ' | Should -Be 'Red 256, Green 64, Blue 64'
        $picture = Compare-WgpuTexture $reference $texture -Difference
        $picture.Notes | Should -Contain "each chroma texel's error fills the 2 by 2 texels that share it"
        $texels = Read-WgpuTexture $picture
        @(0..255 | Where-Object { $texels[$_ * 4 + 2] -gt 0 }) -join ',' | Should -Be '70,71,86,87'
        { Compare-WgpuTexture $reference (New-WgpuTexture 16 16) } | Should -Throw 'the reference is NV12 and the texture Rgba8Unorm: textures compare only when both keep their texels in planes or neither does'
    }
}

Describe 'Get-WgpuTextureFormat' {
    It 'lists every format wgpu has, ASTC included' -Skip:$noGpu {
        $formats = Get-WgpuTextureFormat
        $formats.Count | Should -BeGreaterThan 100
        $formats.Name | Should -Contain 'Rgba8Unorm'
        $formats.Name | Should -Contain 'Astc4x4UnormSrgb'
    }

    It 'describes a format and what the adapter can do with it' -Skip:$noGpu {
        $rgba = Get-WgpuTextureFormat Rgba8Unorm | Where-Object Name -EQ 'Rgba8Unorm'
        $rgba.BlockSize | Should -Be 4
        $rgba.Components | Should -Be 4
        $rgba.ElementType | Should -Be 'UInt8'
        $rgba.AllowedUsages | Should -Contain 'TextureBinding'
        $rgba.Capabilities | Should -Contain 'Filterable'
    }
}

Describe 'Samplers' {
    It 'makes a sampler with the filters and address modes given' -Skip:$noGpu {
        $sampler = New-WgpuSampler -Filter Linear -AddressMode Repeat -Anisotropy 8
        $sampler.MagFilter | Should -Be 'Linear'
        $sampler.AddressModeU | Should -Be 'Repeat'
        $sampler.Anisotropy | Should -Be 8
        $sampler.Compare | Should -BeNullOrEmpty
    }

    It 'makes a comparison sampler' -Skip:$noGpu {
        (New-WgpuSampler -Compare LessEqual).Compare | Should -Be 'LessEqual'
    }
}

Describe 'Image files' {
    It 'saves a texture as PNG and loads it back' -Skip:$noGpu {
        $file = Join-Path ([IO.Path]::GetTempPath()) "wgpu-$PID.png"
        try {
            $pixels = [byte[]](0..63)
            $texture = New-WgpuTexture 4 4 -Data $pixels
            $texture | Export-WgpuImage -Path $file
            (Get-Item $file).Length | Should -BeGreaterThan 0
            $loaded = Import-WgpuImage $file
            $loaded.Format | Should -Be 'Rgba8Unorm'
            (Read-WgpuTexture $loaded) -join ',' | Should -Be ($pixels -join ',')
        } finally {
            Remove-Item $file -ErrorAction Ignore
        }
    }

    It 'writes an .exr file with ZIP, in half floats for 16-bit float texels and full floats for others' -Skip:$noGpu {
        # Values half floats hold exactly, 65504 and 2^-14 among them.
        $values = [float[]](0.5, 1.25, 1000, 1, 0.125, 0.25, 0.375, 1, 3, 2, 1, 1, 65504, 0, 0.00006103515625, 1)
        foreach ($case in @(@('Rgba16Float', 1), @('Rgba32Float', 2))) {
            $file = Join-Path ([IO.Path]::GetTempPath()) "wgpu-$PID-$($case[0]).exr"
            try {
                New-WgpuTexture 2 2 -Format $case[0] -Data $values | Export-WgpuImage -Path $file
                $bytes = [IO.File]::ReadAllBytes($file)
                $text = [Text.Encoding]::ASCII.GetString($bytes)
                # The compression attribute's value follows its name, its
                # type and its size: 3 is ZIP in blocks of 16 scan lines.
                $bytes[$text.IndexOf("compression`0compression`0") + 28] | Should -Be 3 -Because $case[0]
                # The first channel, A, then its pixel type: 1 half, 2 float.
                [BitConverter]::ToInt32($bytes, $text.IndexOf("channels`0chlist`0") + 22) | Should -Be $case[1] -Because $case[0]
                (Read-WgpuTexture (Import-WgpuImage $file)) -join ',' | Should -Be ($values -join ',') -Because $case[0]
            } finally {
                Remove-Item $file -ErrorAction Ignore
            }
        }
    }

    It 'saves and loads two channels as red and green, and a gray-and-alpha image as gray and alpha' -Skip:$noGpu {
        $file = Join-Path ([IO.Path]::GetTempPath()) "wgpu-$PID-pair.png"
        $grayFile = Join-Path ([IO.Path]::GetTempPath()) "wgpu-$PID-gray-alpha.png"
        try {
            New-WgpuTexture 2 1 -Format Rg8Unorm -Data ([byte[]](200, 100, 50, 150)) | Export-WgpuImage -Path $file
            (Read-WgpuTexture (Import-WgpuImage $file)) -join ',' | Should -Be '200,100,0,255,50,150,0,255'
            (Read-WgpuTexture (Import-WgpuImage $file -Format Rg8Unorm)) -join ',' | Should -Be '200,100,50,150'
            (Read-WgpuTexture (Import-WgpuImage $file -Format Rg16Unorm)) -join ',' | Should -Be ((200, 100, 50, 150 | ForEach-Object { $_ * 257 }) -join ',')
            # A 2x1 PNG of gray and alpha: gray 200 with alpha 100, then gray
            # 50 with alpha 150.
            [IO.File]::WriteAllBytes($grayFile, [Convert]::FromBase64String('iVBORw0KGgoAAAANSUhEUgAAAAIAAAABCAQAAABeK7cBAAAADUlEQVR4nGM4kWI0DQAFSwH1SYjangAAAABJRU5ErkJggg=='))
            (Read-WgpuTexture (Import-WgpuImage $grayFile -Format Rg8Unorm)) -join ',' | Should -Be '200,100,50,150'
        } finally {
            Remove-Item $file, $grayFile -ErrorAction Ignore
        }
    }

    It 'saves float two-channel and packed textures, the BRDF table among them' -Skip:$noGpu {
        $file = Join-Path ([IO.Path]::GetTempPath()) "wgpu-$PID-saved.exr"
        try {
            $table = New-WgpuBrdfTable -Size 4 -Format Rg32Float
            $table | Export-WgpuImage -Path $file
            $pairs = Read-WgpuTexture $table
            (Read-WgpuTexture (Import-WgpuImage $file))[0..3] -join ',' | Should -Be ((@($pairs[0], $pairs[1], 0, 1) | ForEach-Object { [float]$_ }) -join ',')
            # Values an 11-bit and a 10-bit float hold exactly.
            $packed = New-WgpuTexture 2 1 -Format Rgba32Float -Data ([float[]](0.5, 2, 8, 1, 0.25, 1, 4, 1)) | Convert-WgpuTexture -Format Rg11b10Ufloat
            $packed | Export-WgpuImage -Path $file
            (Read-WgpuTexture (Import-WgpuImage $file)) -join ',' | Should -Be '0.5,2,8,1,0.25,1,4,1'
        } finally {
            Remove-Item $file -ErrorAction Ignore
        }
    }

    It 'loads an .avif file it saved, told by its content, color and alpha within a few steps' -Skip:$noGpu {
        $file = Join-Path ([IO.Path]::GetTempPath()) "wgpu-$PID.avif"
        $renamed = Join-Path ([IO.Path]::GetTempPath()) "wgpu-$PID-avif.bin"
        try {
            # A gradient across and down, opaque on the left half and alpha
            # 100 on the right.
            $texels = [byte[]]@(foreach ($y in 0..7) { foreach ($x in 0..15) { $x * 15; $y * 30; 120; if ($x -lt 8) { 255 } else { 100 } } })
            $source = New-WgpuTexture 16 8 -Data $texels
            $source | Export-WgpuImage -Path $file
            Copy-Item $file $renamed
            foreach ($path in $file, $renamed) {
                $loaded = Import-WgpuImage $path
                '{0} {1}x{2}' -f $loaded.Format, $loaded.Width, $loaded.Height | Should -Be 'Rgba8Unorm 16x8' -Because $path
                foreach ($row in Compare-WgpuTexture $loaded $source) {
                    $row.Psnr | Should -BeGreaterThan 30 -Because "$path $($row.Channel)"
                }
            }
        } finally {
            Remove-Item $file, $renamed -ErrorAction Ignore
        }
    }

    It 'loads an .avif file whose last box runs to the end of the file, as Photoshop writes them' -Skip:$noGpu {
        $file = Join-Path ([IO.Path]::GetTempPath()) "wgpu-$PID-sized.avif"
        $toEnd = Join-Path ([IO.Path]::GetTempPath()) "wgpu-$PID-to-end.avif"
        try {
            New-WgpuTexture 8 8 -Data ([byte[]](0..255)) | Export-WgpuImage -Path $file
            $bytes = [IO.File]::ReadAllBytes($file)
            # Each top-level box starts with its size, a big-endian 32-bit
            # number; the last one's becomes 0, "to the end of the file".
            $at = 0
            $last = 0
            while ($at -lt $bytes.Length) {
                $size = ([int]$bytes[$at] -shl 24) -bor ([int]$bytes[$at + 1] -shl 16) -bor ([int]$bytes[$at + 2] -shl 8) -bor [int]$bytes[$at + 3]
                if ($size -lt 8) { throw "the box at $at states size $size" }
                $last = $at
                $at += $size
            }
            $at | Should -Be $bytes.Length
            [Array]::Clear($bytes, $last, 4)
            [IO.File]::WriteAllBytes($toEnd, $bytes)
            (Read-WgpuTexture (Import-WgpuImage $toEnd)) -join ',' | Should -Be ((Read-WgpuTexture (Import-WgpuImage $file)) -join ',')
        } finally {
            Remove-Item $file, $toEnd -ErrorAction Ignore
        }
    }

    It 'saves through SaveAsync, which returns a task' -Skip:$noGpu {
        $file = Join-Path ([IO.Path]::GetTempPath()) "wgpu-async-$PID.png"
        try {
            $pixels = [byte[]](0..63)
            $texture = New-WgpuTexture 4 4 -Data $pixels
            $texture.SaveAsync($file).GetAwaiter().GetResult()
            (Read-WgpuTexture (Import-WgpuImage $file)) -join ',' | Should -Be ($pixels -join ',')
        } finally {
            Remove-Item $file -ErrorAction Ignore
        }
    }

    It 'reports a file that does not exist as WgpuImage, with PowerShell''s reason' -Skip:$noGpu {
        $missing = Join-Path ([IO.Path]::GetTempPath()) "wgpu-missing-$PID.png"
        Import-WgpuImage $missing -ErrorAction SilentlyContinue -ErrorVariable failed
        $failed[0].FullyQualifiedErrorId | Should -BeLike 'WgpuImage,*'
        $failed[0].CategoryInfo.Category | Should -Be 'ObjectNotFound'
        $failed[0].Exception.Message | Should -BeLike "Cannot find path '*' because it does not exist."
        $literal = Join-Path ([IO.Path]::GetTempPath()) "wgpu-missing[1]-$PID.png"
        Import-WgpuImage -LiteralPath $literal -ErrorAction SilentlyContinue -ErrorVariable failed
        $failed[0].FullyQualifiedErrorId | Should -BeLike 'WgpuImage,*'
        $failed[0].CategoryInfo.Category | Should -Be 'ObjectNotFound'
        $failed[0].Exception.Message | Should -Be "Cannot find path '$literal' because it does not exist."
    }

    It 'loads a file named with brackets through -LiteralPath and from Get-ChildItem, and refuses it through -Path' -Skip:$noGpu {
        $folder = Join-Path ([IO.Path]::GetTempPath()) "wgpu-images-$PID"
        [void][IO.Directory]::CreateDirectory($folder)
        $bracketed = Join-Path $folder 'p[1].png'
        $plain = Join-Path $folder 'p2.png'
        try {
            $pixels = [byte[]](0..63)
            New-WgpuTexture 4 4 -Data $pixels | Export-WgpuImage -Path $bracketed
            New-WgpuTexture 4 4 -Data $pixels | Export-WgpuImage -Path $plain
            (Read-WgpuTexture (Import-WgpuImage -LiteralPath $bracketed)) -join ',' | Should -Be ($pixels -join ',')
            Import-WgpuImage -Path $bracketed -ErrorAction SilentlyContinue -ErrorVariable failed
            $failed[0].FullyQualifiedErrorId | Should -BeLike 'WgpuImage,*'
            $piped = @(Get-ChildItem -LiteralPath $folder -Filter '*.png' | Sort-Object Name | Import-WgpuImage)
            ($piped.Label -join ',') | Should -Be (($bracketed, $plain) -join ',')
            ([IO.FileInfo]::new($bracketed) | Import-WgpuImage).Label | Should -Be $bracketed
            (Resolve-Path -LiteralPath $plain | Import-WgpuImage).Label | Should -Be $plain
        } finally {
            Remove-Item -LiteralPath $bracketed -ErrorAction Ignore
            Remove-Item -LiteralPath $plain -ErrorAction Ignore
            Remove-Item -LiteralPath $folder -ErrorAction Ignore
        }
    }

    It 'loads every file a wildcard matches, and writes an error for one that fails without stopping' -Skip:$noGpu {
        $folder = Join-Path ([IO.Path]::GetTempPath()) "wgpu-wildcard-$PID"
        [void][IO.Directory]::CreateDirectory($folder)
        $broken = Join-Path $folder 'a-broken.png'
        $first = Join-Path $folder 'b1.png'
        $second = Join-Path $folder 'b2.png'
        try {
            [IO.File]::WriteAllBytes($broken, [byte[]](1, 2, 3))
            New-WgpuTexture 4 4 -Data ([byte[]](0..63)) | Export-WgpuImage -Path $first
            New-WgpuTexture 2 2 -Data ([byte[]](0..15)) | Export-WgpuImage -Path $second
            $loaded = @(Import-WgpuImage -Path (Join-Path $folder '*.png') -ErrorAction SilentlyContinue -ErrorVariable failed)
            ($loaded | Sort-Object Label).Width -join ',' | Should -Be '4,2'
            $failed.Count | Should -Be 1
            $failed[0].FullyQualifiedErrorId | Should -BeLike 'WgpuImage,*'
        } finally {
            Remove-Item -LiteralPath $broken -ErrorAction Ignore
            Remove-Item -LiteralPath $first -ErrorAction Ignore
            Remove-Item -LiteralPath $second -ErrorAction Ignore
            Remove-Item -LiteralPath $folder -ErrorAction Ignore
        }
    }

    It 'loads as the format asked for' -Skip:$noGpu {
        $file = Join-Path ([IO.Path]::GetTempPath()) "wgpu-gray-$PID.png"
        try {
            New-WgpuTexture 2 2 -Data ([byte[]](255, 255, 255, 255, 0, 0, 0, 255, 255, 255, 255, 255, 0, 0, 0, 255)) | Export-WgpuImage -Path $file
            $gray = Import-WgpuImage $file -Format R8Unorm
            (Read-WgpuTexture $gray) -join ',' | Should -Be '255,0,255,0'
        } finally {
            Remove-Item $file -ErrorAction Ignore
        }
    }

    It 'completes -Format with the texture formats and the Basis Universal formats' {
        $line = 'Import-WgpuImage photo.png -Format BasisUa'
        $names = (TabExpansion2 -inputScript $line -cursorColumn $line.Length).CompletionMatches.CompletionText
        $names | Should -Contain 'BasisUastc4x4'
        $names | Should -Contain 'BasisUastcHdr6x6'
        $line = 'Export-WgpuImage $texture photo.dds -Format Bc7'
        (TabExpansion2 -inputScript $line -cursorColumn $line.Length).CompletionMatches.CompletionText | Should -Contain 'Bc7RgbaUnormSrgb'
    }
}

Describe 'Orientation and color on import' {
    BeforeAll {
        $script:temp = [IO.Path]::GetTempPath()
        # Eight texels of different colors, 4x2, so a turn or a mirror shows.
        $script:eight = [byte[]]@(foreach ($i in 0..7) { $i * 30; 255 - $i * 30; ($i * 70) % 256; 255 })

        # The big-endian number of `count` bytes at `at` in `bytes`.
        function script:Read-BigEndian([byte[]] $bytes, [int] $at, [int] $count) {
            $value = [long]0
            for ($i = 0; $i -lt $count; $i++) { $value = ($value -shl 8) -bor $bytes[$at + $i] }
            $value
        }

        # `value` as `count` big-endian bytes.
        function script:ConvertTo-BigEndian([long] $value, [int] $count) {
            [byte[]]@(for ($i = $count - 1; $i -ge 0; $i--) { ($value -shr (8 * $i)) -band 255 })
        }

        # A JPEG's bytes with `segment` after its start-of-image marker.
        function script:Add-JpegSegment([byte[]] $jpeg, [byte[]] $segment) {
            [byte[]]($jpeg[0..1] + $segment + $jpeg[2..($jpeg.Length - 1)])
        }

        # EXIF whose one tag is the orientation given: "Exif" and two zero
        # bytes, then a little-endian TIFF block of one IFD entry.
        function script:New-Exif([int] $orientation) {
            $tiff = [byte[]](0x49, 0x49, 42, 0, 8, 0, 0, 0, 1, 0, 0x12, 0x01, 3, 0, 1, 0, 0, 0, $orientation, 0, 0, 0, 0, 0, 0, 0)
            [byte[]]([Text.Encoding]::ASCII.GetBytes("Exif`0`0") + $tiff)
        }

        # A JPEG segment of the marker given, its length before `body`.
        function script:New-JpegSegment([byte] $marker, [byte[]] $body) {
            [byte[]]((0xFF, $marker) + (ConvertTo-BigEndian ($body.Length + 2) 2) + $body)
        }

        # A version 2 display profile of sRGB's colorants and a curve of no
        # entries, the identity: its colors are linear sRGB.
        function script:New-LinearProfile {
            $s15 = { param([double] $v) ConvertTo-BigEndian ([long][math]::Round($v * 65536)) 4 }
            $xyz = { param($x, $y, $z) [byte[]]([Text.Encoding]::ASCII.GetBytes('XYZ ') + [byte[]]::new(4) + (& $s15 $x) + (& $s15 $y) + (& $s15 $z)) }
            $curve = [byte[]]([Text.Encoding]::ASCII.GetBytes('curv') + [byte[]]::new(8))
            # A textDescriptionType: its ASCII, then no Unicode or ScriptCode
            # text.
            $text = [Text.Encoding]::ASCII.GetBytes("Linear sRGB`0")
            $desc = [byte[]]([Text.Encoding]::ASCII.GetBytes('desc') + [byte[]]::new(4) + (ConvertTo-BigEndian $text.Length 4) + $text + [byte[]]::new(78))
            $tags = [ordered]@{
                desc = $desc
                wtpt = & $xyz 0.9642 1.0 0.8249
                rXYZ = & $xyz 0.4360747 0.2225045 0.0139322
                gXYZ = & $xyz 0.3850649 0.7168786 0.0971045
                bXYZ = & $xyz 0.1430804 0.0606169 0.7141733
                rTRC = $curve
                gTRC = $curve
                bTRC = $curve
            }
            $table = [Collections.Generic.List[byte]]::new()
            $body = [Collections.Generic.List[byte]]::new()
            $first = 128 + 4 + 12 * $tags.Count
            # A function's array comes back as objects, which AddRange takes
            # only cast back to bytes.
            $table.AddRange([byte[]](ConvertTo-BigEndian $tags.Count 4))
            foreach ($name in $tags.Keys) {
                $data = [byte[]]$tags[$name]
                $table.AddRange([Text.Encoding]::ASCII.GetBytes($name))
                $table.AddRange([byte[]](ConvertTo-BigEndian ($first + $body.Count) 4))
                $table.AddRange([byte[]](ConvertTo-BigEndian $data.Length 4))
                $body.AddRange($data)
                while ($body.Count % 4) { $body.Add(0) }
            }
            # The header: size, CMM, version 2.1, class, space and PCS, date,
            # signature, platform to intent, D50 and the rest.
            $header = [Collections.Generic.List[byte]]::new()
            $header.AddRange([byte[]](ConvertTo-BigEndian (128 + $table.Count + $body.Count) 4))
            $header.AddRange([byte[]](0, 0, 0, 0, 2, 0x10, 0, 0))
            $header.AddRange([Text.Encoding]::ASCII.GetBytes('mntrRGB XYZ '))
            $header.AddRange([byte[]]::new(12))
            $header.AddRange([Text.Encoding]::ASCII.GetBytes('acsp'))
            $header.AddRange([byte[]]::new(28))
            foreach ($v in 0.9642, 1.0, 0.8249) { $header.AddRange([byte[]](& $s15 $v)) }
            $header.AddRange([byte[]]::new(48))
            [byte[]]($header.ToArray() + $table.ToArray() + $body.ToArray())
        }

        # The CRC-32 a PNG chunk ends with, of `bytes`.
        function script:Get-Crc32([byte[]] $bytes) {
            $crc = [long]4294967295
            foreach ($b in $bytes) {
                $crc = $crc -bxor $b
                for ($k = 0; $k -lt 8; $k++) {
                    if ($crc -band 1) { $crc = ($crc -shr 1) -bxor 3988292384 } else { $crc = $crc -shr 1 }
                }
            }
            $crc -bxor 4294967295
        }

        # A PNG's bytes with a tEXt chunk of `keyword` and `text` after its
        # header chunk: the signature and IHDR take its first 33 bytes.
        function script:Add-PngText([byte[]] $png, [string] $keyword, [string] $text) {
            $typed = [byte[]]([Text.Encoding]::ASCII.GetBytes('tEXt') + [Text.Encoding]::ASCII.GetBytes($keyword) + [byte[]](0) + [Text.Encoding]::ASCII.GetBytes($text))
            $chunk = [byte[]]((ConvertTo-BigEndian ($typed.Length - 4) 4) + $typed + (ConvertTo-BigEndian (Get-Crc32 $typed) 4))
            [byte[]]($png[0..32] + $chunk + $png[33..($png.Length - 1)])
        }

        # An .avif's bytes with a colr box of the CICP primaries, transfer
        # and matrix given at the end of ipco and associated with the primary
        # item in ipma, which must be of version 0 with 1-byte indexes; meta,
        # iprp, ipco and ipma grow by the bytes added, and the extents of an
        # iloc of version 0 before iprp move past them.
        function script:Add-AvifColor([byte[]] $avif, [int] $primaries, [int] $transfer, [int] $matrix) {
            # The boxes laid end to end from `from` to `to`.
            $boxes = {
                param($from, $to)
                $at = $from
                while ($at -lt $to) {
                    $size = Read-BigEndian $avif $at 4
                    [pscustomobject]@{ Kind = [Text.Encoding]::ASCII.GetString($avif, $at + 4, 4); At = $at; Size = $size }
                    $at += $size
                }
            }
            # meta, iloc and pitm are full boxes: version and flags after the
            # 8-byte header.
            $meta = & $boxes 0 $avif.Length | Where-Object Kind -eq 'meta'
            $inMeta = @(& $boxes ($meta.At + 12) ($meta.At + $meta.Size))
            $pitm = $inMeta | Where-Object Kind -eq 'pitm'
            $iloc = $inMeta | Where-Object Kind -eq 'iloc'
            $iprp = $inMeta | Where-Object Kind -eq 'iprp'
            $inIprp = @(& $boxes ($iprp.At + 8) ($iprp.At + $iprp.Size))
            $ipco = $inIprp | Where-Object Kind -eq 'ipco'
            $ipma = $inIprp | Where-Object Kind -eq 'ipma'
            if ($avif[$pitm.At + 8] -ne 0 -or $avif[$ipma.At + 8] -ne 0 -or ($avif[$ipma.At + 11] -band 1) -or $avif[$iloc.At + 8] -ne 0 -or ($avif[$iloc.At + 13] -shr 4) -ne 0 -or $iloc.At -gt $iprp.At) {
                throw 'the .avif is not of the box versions and order Add-AvifColor reads'
            }
            $primary = Read-BigEndian $avif ($pitm.At + 12) 2
            $index = @(& $boxes ($ipco.At + 8) ($ipco.At + $ipco.Size)).Count + 1
            $colr = [byte[]]((ConvertTo-BigEndian 19 4) + [Text.Encoding]::ASCII.GetBytes('colrnclx') + (ConvertTo-BigEndian $primaries 2) + (ConvertTo-BigEndian $transfer 2) + (ConvertTo-BigEndian $matrix 2) + [byte[]](0x80))
            $added = $colr.Length + 1
            $bytes = [Collections.Generic.List[byte]]::new($avif)
            # Each ipma entry: a 2-byte item id, a count, and that many
            # 1-byte associations.
            $at = $ipma.At + 16
            foreach ($entry in 1..(Read-BigEndian $avif ($ipma.At + 12) 4)) {
                $count = $avif[$at + 2]
                if ((Read-BigEndian $avif $at 2) -eq $primary) {
                    $bytes.Insert($at + 3 + $count, [byte]$index)
                    $bytes[$at + 2] = [byte]($count + 1)
                }
                $at += 3 + $count
            }
            $bytes.InsertRange($ipco.At + $ipco.Size, $colr)
            foreach ($grown in @(@($meta, $added, 0), @($iprp, $added, 0), @($ipco, $colr.Length, 0), @($ipma, 1, $colr.Length))) {
                $box, $more, $moved = $grown
                $size = ConvertTo-BigEndian ($box.Size + $more) 4
                for ($i = 0; $i -lt 4; $i++) { $bytes[$box.At + $moved + $i] = $size[$i] }
            }
            # iloc: offset and length sizes, an item count, and each item's
            # id, data reference, extent count and extents.
            $offsetSize = $avif[$iloc.At + 12] -shr 4
            $lengthSize = $avif[$iloc.At + 12] -band 15
            $at = $iloc.At + 16
            foreach ($item in 1..(Read-BigEndian $avif ($iloc.At + 14) 2)) {
                $at += 4
                $extents = Read-BigEndian $avif $at 2
                $at += 2
                foreach ($extent in 1..$extents) {
                    $offset = ConvertTo-BigEndian ((Read-BigEndian $avif $at $offsetSize) + $added) $offsetSize
                    for ($i = 0; $i -lt $offsetSize; $i++) { $bytes[$at + $i] = $offset[$i] }
                    $at += $offsetSize + $lengthSize
                }
            }
            $bytes.ToArray()
        }
    }

    It 'turns a JPEG by its EXIF orientation as Convert-WgpuTexture turns, and says so' -Skip:$noGpu {
        $plain = Join-Path $temp "wgpu-$PID-plain.jpg"
        $turned = Join-Path $temp "wgpu-$PID-turned.jpg"
        try {
            New-WgpuTexture 4 2 -Data $eight | Export-WgpuImage -Path $plain
            $jpeg = [IO.File]::ReadAllBytes($plain)
            $stored = Import-WgpuImage $plain
            # EXIF's orientations 2 to 8 as Convert-WgpuTexture's turns, which
            # it makes before it flips, and how the import names them.
            $orientations = @{
                2 = @{ Flip = 'Horizontal' }, 'mirrored left to right'
                3 = @{ Rotate = 180 }, 'turned 180 degrees'
                4 = @{ Flip = 'Vertical' }, 'mirrored top to bottom'
                5 = @{ Rotate = 90; Flip = 'Horizontal' }, 'turned 90 degrees clockwise and mirrored left to right'
                6 = @{ Rotate = 90 }, 'turned 90 degrees clockwise'
                7 = @{ Rotate = 270; Flip = 'Horizontal' }, 'turned 90 degrees counterclockwise and mirrored left to right'
                8 = @{ Rotate = 270 }, 'turned 90 degrees counterclockwise'
            }
            foreach ($orientation in 2..8) {
                $turn, $words = $orientations[$orientation]
                [IO.File]::WriteAllBytes($turned, (Add-JpegSegment $jpeg (New-JpegSegment 0xE1 (New-Exif $orientation))))
                $records = Import-WgpuImage $turned -Verbose 4>&1
                $loaded = $records | Where-Object { $_ -isnot [System.Management.Automation.VerboseRecord] }
                $said = $records | Where-Object { $_ -is [System.Management.Automation.VerboseRecord] } | ForEach-Object Message
                $want = $stored | Convert-WgpuTexture @turn
                '{0}x{1}' -f $loaded.Width, $loaded.Height | Should -Be ('{0}x{1}' -f $want.Width, $want.Height) -Because "orientation $orientation"
                (Read-WgpuTexture $loaded) -join ',' | Should -Be ((Read-WgpuTexture $want) -join ',') -Because "orientation $orientation"
                $said | Should -Contain "the pixels were $words, as the file's orientation says" -Because "orientation $orientation"
            }
        } finally {
            Remove-Item $plain, $turned -ErrorAction Ignore
        }
    }

    It 'converts colors from a JPEG''s color profile to sRGB, and says so' -Skip:$noGpu {
        $plain = Join-Path $temp "wgpu-$PID-unprofiled.jpg"
        $linear = Join-Path $temp "wgpu-$PID-linear.jpg"
        try {
            # Grays from black to white.
            New-WgpuTexture 4 1 -Data ([byte[]](0, 0, 0, 255, 64, 64, 64, 255, 160, 160, 160, 255, 255, 255, 255, 255)) | Export-WgpuImage -Path $plain
            $icc = [byte[]]([Text.Encoding]::ASCII.GetBytes("ICC_PROFILE`0") + [byte[]](1, 1) + (New-LinearProfile))
            [IO.File]::WriteAllBytes($linear, (Add-JpegSegment ([IO.File]::ReadAllBytes($plain)) (New-JpegSegment 0xE2 $icc)))
            $stored = Read-WgpuTexture (Import-WgpuImage $plain)
            $records = Import-WgpuImage $linear -Verbose 4>&1
            $converted = Read-WgpuTexture ($records | Where-Object { $_ -isnot [System.Management.Automation.VerboseRecord] })
            $records | Where-Object { $_ -is [System.Management.Automation.VerboseRecord] } | ForEach-Object Message |
                Should -Contain 'the colors were converted from the file''s color profile, Linear sRGB, to sRGB, perceptually'
            # A sample of 0 or 255 stays as it is, and linear light between
            # them encodes brighter in sRGB, whose curve rises above the
            # line from 0 to 1.
            foreach ($at in 0, 1, 2, 4, 5, 6, 8, 9, 10, 12, 13, 14) {
                if ($stored[$at] -in 0, 255) {
                    $converted[$at] | Should -Be $stored[$at] -Because "sample $at"
                } else {
                    $converted[$at] | Should -BeGreaterThan $stored[$at] -Because "sample $at, $($stored[$at]) as stored"
                }
            }
        } finally {
            Remove-Item $plain, $linear -ErrorAction Ignore
        }
    }

    It 'loads the colors as stored and warns where the color profile cannot be read' -Skip:$noGpu {
        $plain = Join-Path $temp "wgpu-$PID-plain-profile.jpg"
        $broken = Join-Path $temp "wgpu-$PID-broken-profile.jpg"
        try {
            New-WgpuTexture 4 2 -Data $eight | Export-WgpuImage -Path $plain
            $icc = [byte[]]([Text.Encoding]::ASCII.GetBytes("ICC_PROFILE`0") + [byte[]](1, 1) + [Text.Encoding]::ASCII.GetBytes('not a color profile'))
            [IO.File]::WriteAllBytes($broken, (Add-JpegSegment ([IO.File]::ReadAllBytes($plain)) (New-JpegSegment 0xE2 $icc)))
            $loaded = Import-WgpuImage $broken -WarningVariable said -WarningAction SilentlyContinue
            (Read-WgpuTexture $loaded) -join ',' | Should -Be ((Read-WgpuTexture (Import-WgpuImage $plain)) -join ',')
            $said.Count | Should -Be 1
            $said[0].Message | Should -BeLike "the color profile of $broken was not used, as it cannot be read: *, so its colors loaded as stored"
        } finally {
            Remove-Item $plain, $broken -ErrorAction Ignore
        }
    }

    It 'loads as stored with -IgnoreOrientation and -IgnoreProfile, converts with -RenderingIntent, and says so' -Skip:$noGpu {
        $plain = Join-Path $temp "wgpu-$PID-asked-plain.jpg"
        $marked = Join-Path $temp "wgpu-$PID-asked-marked.jpg"
        try {
            # Grays from black to white: a file that records nothing, and one
            # with a linear profile and orientation 6.
            New-WgpuTexture 4 1 -Data ([byte[]](0, 0, 0, 255, 64, 64, 64, 255, 160, 160, 160, 255, 255, 255, 255, 255)) | Export-WgpuImage -Path $plain
            $icc = [byte[]]([Text.Encoding]::ASCII.GetBytes("ICC_PROFILE`0") + [byte[]](1, 1) + (New-LinearProfile))
            $profiled = Add-JpegSegment ([IO.File]::ReadAllBytes($plain)) (New-JpegSegment 0xE2 $icc)
            [IO.File]::WriteAllBytes($marked, (Add-JpegSegment $profiled (New-JpegSegment 0xE1 (New-Exif 6))))
            $stored = Read-WgpuTexture (Import-WgpuImage $plain)
            $records = Import-WgpuImage $marked -IgnoreOrientation -IgnoreProfile -Verbose 4>&1
            $loaded = $records | Where-Object { $_ -isnot [System.Management.Automation.VerboseRecord] }
            '{0}x{1}' -f $loaded.Width, $loaded.Height | Should -Be '4x1'
            (Read-WgpuTexture $loaded) -join ',' | Should -Be ($stored -join ',')
            $said = $records | Where-Object { $_ -is [System.Management.Automation.VerboseRecord] } | ForEach-Object Message
            $said | Should -Contain 'the colors loaded as stored, as -IgnoreProfile asks'
            $said | Should -Contain 'the pixels loaded as stored, as -IgnoreOrientation asks, where the file''s orientation says they are turned 90 degrees clockwise'
            $records = Import-WgpuImage $marked -RenderingIntent RelativeColorimetric -Verbose 4>&1
            $records | Where-Object { $_ -is [System.Management.Automation.VerboseRecord] } | ForEach-Object Message |
                Should -Contain 'the colors were converted from the file''s color profile, Linear sRGB, to sRGB, with the relative colorimetric intent'
            $null = Import-WgpuImage $plain -IgnoreOrientation -IgnoreProfile -WarningVariable warned -WarningAction SilentlyContinue
            ($warned | ForEach-Object Message) -join '|' | Should -Be 'the file carries no color profile, so -IgnoreProfile changed nothing|the file records no orientation, so -IgnoreOrientation changed nothing'
            { Import-WgpuImage $marked -IgnoreProfile -RenderingIntent Saturation } | Should -Throw '-RenderingIntent chooses how colors convert from a file''s color profile, and -IgnoreProfile converts none: give one or the other'
        } finally {
            Remove-Item $plain, $marked -ErrorAction Ignore
        }
    }

    It 'turns a PNG by the EXIF ImageMagick writes in a text chunk' -Skip:$noGpu {
        $plain = Join-Path $temp "wgpu-$PID-plain.png"
        $tagged = Join-Path $temp "wgpu-$PID-text-exif.png"
        try {
            New-WgpuTexture 4 2 -Data $eight | Export-WgpuImage -Path $plain
            $exif = New-Exif 6
            $hex = -join ($exif | ForEach-Object { $_.ToString('x2') })
            # ImageMagick's form: a blank line, the profile's name, its length,
            # then the hexadecimal.
            [IO.File]::WriteAllBytes($tagged, (Add-PngText ([IO.File]::ReadAllBytes($plain)) 'Raw profile type exif' ("`nexif`n{0,8}`n{1}`n" -f $exif.Length, $hex)))
            $records = Import-WgpuImage $tagged -Verbose 4>&1
            $loaded = $records | Where-Object { $_ -isnot [System.Management.Automation.VerboseRecord] }
            '{0}x{1}' -f $loaded.Width, $loaded.Height | Should -Be '2x4'
            (Read-WgpuTexture $loaded) -join ',' | Should -Be ((Read-WgpuTexture (New-WgpuTexture 4 2 -Data $eight | Convert-WgpuTexture -Rotate 90)) -join ',')
            $records | Where-Object { $_ -is [System.Management.Automation.VerboseRecord] } | ForEach-Object Message |
                Should -Contain 'the pixels were turned 90 degrees clockwise, as the file''s orientation says'
        } finally {
            Remove-Item $plain, $tagged -ErrorAction Ignore
        }
    }

    It 'loads a PQ .avif file as Rgba16Float linear light, its reference white at 1, and says so' -Skip:$noGpu {
        $sdr = Join-Path $temp "wgpu-$PID-sdr.avif"
        $pq = Join-Path $temp "wgpu-$PID-pq.avif"
        try {
            # White then black.
            New-WgpuTexture 2 1 -Data ([byte[]](255, 255, 255, 255, 0, 0, 0, 255)) | Export-WgpuImage -Path $sdr
            # BT.2020 primaries and PQ's transfer, beside BT.601's matrix,
            # which the file's samples were written in.
            [IO.File]::WriteAllBytes($pq, (Add-AvifColor ([IO.File]::ReadAllBytes($sdr)) 9 16 6))
            $records = Import-WgpuImage $pq -Verbose 4>&1
            $loaded = $records | Where-Object { $_ -isnot [System.Management.Automation.VerboseRecord] }
            $loaded.Format | Should -Be 'Rgba16Float'
            $records | Where-Object { $_ -is [System.Management.Automation.VerboseRecord] } | ForEach-Object Message |
                Should -Contain 'the colors were converted from the file''s PQ light in Bt2020 primaries to linear light in sRGB''s, BT.2408''s HDR reference white of 203 nits at 1'
            $texels = Read-WgpuTexture $loaded
            # PQ's full signal is 10,000 nits, far above the reference white,
            # and black far below it.
            foreach ($at in 0..2) {
                $texels[$at] | Should -BeGreaterThan 1 -Because "sample $at"
                $texels[$at + 4] | Should -BeLessThan 1 -Because "sample $($at + 4)"
            }
            $texels[3] | Should -Be 1
        } finally {
            Remove-Item $sdr, $pq -ErrorAction Ignore
        }
    }

    It 'loads a JPEG XL file, told by its content, sample for sample' -Skip:$noGpu {
        $file = Join-Path $temp "wgpu-$PID.jxl"
        $renamed = Join-Path $temp "wgpu-$PID-jxl.bin"
        try {
            # A 4 by 3 RGB image in sRGB that libjxl encoded losslessly as a
            # bare codestream, and its samples (tools/jxl_reference.py).
            $jxl = [byte[]](255, 10, 16, 48, 16, 9, 8, 0, 1, 0, 180, 0, 75, 24, 147, 142, 131, 131, 132, 19, 24, 219, 196, 164,
                254, 2, 193, 0, 60, 129, 142, 212, 145, 250, 53, 210, 183, 148, 54, 0, 240, 189, 165, 75, 97, 3, 0, 192,
                6, 136, 99, 113, 11, 113, 26, 91, 136)
            $rgb = [byte[]](13, 252, 235, 218, 201, 184, 167, 150, 133, 116, 99, 82, 65, 48, 31, 14, 253, 236, 219, 202, 185, 168, 151, 134,
                117, 100, 83, 66, 49, 32, 15, 254, 237, 220, 203, 186)
            $expected = foreach ($at in 0..11) { $rgb[(3 * $at)..(3 * $at + 2)]; 255 }
            foreach ($path in $file, $renamed) {
                [IO.File]::WriteAllBytes($path, $jxl)
                (Read-WgpuTexture (Import-WgpuImage $path)) -join ',' | Should -Be ($expected -join ',') -Because $path
            }
        } finally {
            Remove-Item $file, $renamed -ErrorAction Ignore
        }
    }

    It 'loads a PQ JPEG XL file as Rgba16Float linear light, its reference white at 1, and says so' -Skip:$noGpu {
        $pq = Join-Path $temp "wgpu-$PID-pq.jxl"
        try {
            # A 4 by 3 16-bit RGB image of PQ signals in BT.2020's primaries,
            # tagged so, that libjxl encoded losslessly in a container
            # (tools/jxl_reference.py).
            [IO.File]::WriteAllBytes($pq, [byte[]](0, 0, 0, 12, 74, 88, 76, 32, 13, 10, 135, 10, 0, 0, 0, 20, 102, 116, 121, 112, 106, 120, 108, 32,
                0, 0, 0, 0, 106, 120, 108, 32, 0, 0, 0, 9, 106, 120, 108, 108, 10, 0, 0, 0, 124, 106, 120, 108,
                99, 255, 10, 16, 48, 1, 63, 0, 121, 244, 136, 195, 1, 0, 0, 0, 32, 8, 0, 1, 0, 120, 1, 75,
                24, 147, 142, 131, 131, 132, 20, 132, 94, 88, 107, 10, 112, 251, 92, 32, 24, 128, 83, 206, 140, 51, 43, 205,
                92, 203, 140, 74, 51, 215, 1, 32, 206, 140, 51, 227, 204, 126, 255, 79, 51, 227, 221, 57, 51, 206, 172, 33,
                194, 79, 51, 215, 16, 225, 199, 255, 227, 204, 56, 51, 206, 140, 119, 31, 222, 157, 51, 227, 204, 134, 119, 31,
                222, 157, 51, 227, 204, 56, 179, 134, 8, 87, 102, 122, 194, 232, 38, 94, 47, 226, 253, 27, 239))
            $records = Import-WgpuImage $pq -Verbose 4>&1
            $loaded = $records | Where-Object { $_ -isnot [System.Management.Automation.VerboseRecord] }
            $loaded.Format | Should -Be 'Rgba16Float'
            $records | Where-Object { $_ -is [System.Management.Automation.VerboseRecord] } | ForEach-Object Message |
                Should -Contain 'the colors were converted from the file''s PQ light in Bt2020 primaries to linear light in sRGB''s, BT.2408''s HDR reference white of 203 nits at 1'
        } finally {
            Remove-Item $pq -ErrorAction Ignore
        }
    }
}

Describe 'SVG and PDF files' {
    BeforeAll {
        $script:temp = [IO.Path]::GetTempPath()
        # A 12 by 8 drawing of rectangles on whole pixels: red at the top
        # left, a half transparent blue over it, and a green bar.
        $script:blocks = @'
<svg xmlns="http://www.w3.org/2000/svg" width="12" height="8" viewBox="0 0 12 8">
  <rect x="0" y="0" width="5" height="3" fill="#cc3300"/>
  <rect x="3" y="2" width="6" height="4" fill="#3366cc" fill-opacity="0.5"/>
  <rect x="9" y="1" width="2" height="7" fill="#009933"/>
</svg>
'@
        # A 40 by 30 point page of four rectangles, the first red from (2, 2),
        # and the same page under the user password wgpu, written by
        # tools/pdf_reference.py.
        $script:rects = [Convert]::FromBase64String('JVBERi0xLjcKMSAwIG9iago8PCAvVHlwZSAvQ2F0YWxvZyAvUGFnZXMgMiAwIFIgPj4KZW5kb2JqCjIgMCBvYmoKPDwgL1R5cGUgL1BhZ2VzIC9LaWRzIFszIDAgUl0gL0NvdW50IDEgPj4KZW5kb2JqCjMgMCBvYmoKPDwgL1R5cGUgL1BhZ2UgL1BhcmVudCAyIDAgUiAvTWVkaWFCb3ggWzAgMCA0MCAzMF0gL1Jlc291cmNlcyA8PCA+PiAvQ29udGVudHMgNCAwIFIgPj4KZW5kb2JqCjQgMCBvYmoKPDwgL0xlbmd0aCAxMDggPj4Kc3RyZWFtCjAuOCAwLjIgMCByZyAyIDE2IDE2IDEyIHJlIGYKMC4yIDAuNCAwLjggcmcgMTAgOCAyMCAxNCByZSBmCjAgMC42IDAuMiByZyAyNCA0IDEyIDIyIHJlIGYKMC42IGcgNCA0IDEwIDggcmUgZgplbmRzdHJlYW0KZW5kb2JqCnhyZWYKMCA1CjAwMDAwMDAwMDAgNjU1MzUgZiAKMDAwMDAwMDAwOSAwMDAwMCBuIAowMDAwMDAwMDU4IDAwMDAwIG4gCjAwMDAwMDAxMTUgMDAwMDAgbiAKMDAwMDAwMDIxNyAwMDAwMCBuIAp0cmFpbGVyCjw8IC9TaXplIDUgL1Jvb3QgMSAwIFIgPj4Kc3RhcnR4cmVmCjM3NgolJUVPRgo=')
        $script:protected = [Convert]::FromBase64String('JVBERi0xLjcKJb/3ov4KMSAwIG9iago8PCAvRXh0ZW5zaW9ucyA8PCAvQURCRSA8PCAvQmFzZVZlcnNpb24gLzEuNyAvRXh0ZW5zaW9uTGV2ZWwgOCA+PiA+PiAvUGFnZXMgMiAwIFIgL1R5cGUgL0NhdGFsb2cgPj4KZW5kb2JqCjIgMCBvYmoKPDwgL0NvdW50IDEgL0tpZHMgWyAzIDAgUiBdIC9UeXBlIC9QYWdlcyA+PgplbmRvYmoKMyAwIG9iago8PCAvQ29udGVudHMgNCAwIFIgL01lZGlhQm94IFsgMCAwIDQwIDMwIF0gL1BhcmVudCAyIDAgUiAvUmVzb3VyY2VzIDw8ID4+IC9UeXBlIC9QYWdlID4+CmVuZG9iago0IDAgb2JqCjw8IC9MZW5ndGggOTYgL0ZpbHRlciAvRmxhdGVEZWNvZGUgPj4Kc3RyZWFtCpr5ksT610byQ22tVnS/HjWX7S9zOXBA9Tuefm6J3zDxh8Dtt01HCkDu9cqQnHIQ5y7w5IzBTeYZ19yNOjJQv7lpDK+XuLZpzOjKGd5Sbgat4aIiNGm42WrVwOawBfCNjAplbmRzdHJlYW0KZW5kb2JqCjUgMCBvYmoKPDwgL0NGIDw8IC9TdGRDRiA8PCAvQXV0aEV2ZW50IC9Eb2NPcGVuIC9DRk0gL0FFU1YzIC9MZW5ndGggMzIgPj4gPj4gL0ZpbHRlciAvU3RhbmRhcmQgL0xlbmd0aCAyNTYgL08gPDgyNGVmZGE5MjBkODg2ZDQ3ZmNhN2RmOWEwOTkxMTUzMzg3MzcyYzE3YzJmMzkzNjhiZWNjOTY0MTE2M2RiNjBjOGJjYWFjYTI0ZGU0OWExOWQ2NTVhODlhODkyOTY3Yj4gL09FIDxjMmQ4NGY3ZGM4OTk5ODAyMzUzZWM3NzdiMmNhMGIzMmRhNzVlOTQ3MDRlZjMzYzkxNjg4MjdiZTZjOTljOTljPiAvUCAtMTAyOCAvUGVybXMgPDFlMmIxMTA3YTc4YmRmYWY4Mjg2MmIxNzIxYTM4YTk3PiAvUiA2IC9TdG1GIC9TdGRDRiAvU3RyRiAvU3RkQ0YgL1UgPGRkYzUxNGQ1ZjgxOWU1NWNlNDgyMzcyNGQ1MzAxZTUxYTBiOWMwOGRlYWQ0NTMzMjhkOTAxNjE5YmMyNTNlYzJlM2ViNzIwZGJhMWI1NWZhM2JjYWI0MjE3ODE0NzBmNj4gL1VFIDxmMGM4ZGU1ZWU3NTA4ODk5Njk1MzMwMzUxNWM4OWJhYzY1ODc1Y2EzNDNkMTg0YjE1NTc4M2Y0OTlmNmJiZGVkPiAvViA1ID4+CmVuZG9iagp4cmVmCjAgNgowMDAwMDAwMDAwIDY1NTM1IGYgCjAwMDAwMDAwMTUgMDAwMDAgbiAKMDAwMDAwMDEzMCAwMDAwMCBuIAowMDAwMDAwMTg5IDAwMDAwIG4gCjAwMDAwMDAyOTMgMDAwMDAgbiAKMDAwMDAwMDQ2MCAwMDAwMCBuIAp0cmFpbGVyIDw8IC9Sb290IDEgMCBSIC9TaXplIDYgL0lEIFs8MTA4ODU0YzdhNjc5MTk2OGFjYzM4MDM0ZTkxM2E5OWQ+PDEwODg1NGM3YTY3OTE5NjhhY2MzODAzNGU5MTNhOTlkPl0gL0VuY3J5cHQgNSAwIFIgPj4Kc3RhcnR4cmVmCjEwMTAKJSVFT0YK')
        # The texel at (x, y) of 8-bit RGBA texels `width` texels wide.
        function Get-Texel($texels, $width, $x, $y) {
            $at = 4 * ($y * $width + $x)
            $texels[$at..($at + 3)] -join ','
        }
    }

    It 'draws an SVG file at the size it gives, and at the size -Width asks for, drawn there and not resampled' -Skip:$noGpu {
        $file = Join-Path $temp "wgpu-$PID-blocks.svg"
        try {
            [IO.File]::WriteAllText($file, $blocks)
            $records = Import-WgpuImage $file -Verbose 4>&1
            $own = $records | Where-Object { $_ -isnot [System.Management.Automation.VerboseRecord] }
            '{0}x{1}' -f $own.Width, $own.Height | Should -Be '12x8'
            $records | Where-Object { $_ -is [System.Management.Automation.VerboseRecord] } | ForEach-Object Message |
                Should -Contain 'the SVG file, 12x8 at its own size, was drawn at 12x8'
            $small = Read-WgpuTexture $own
            Get-Texel $small 12 0 0 | Should -Be '204,51,0,255'
            Get-Texel $small 12 11 0 | Should -Be '0,0,0,0'
            $double = Import-WgpuImage $file -Width 24 -Filter Lanczos -WarningVariable warned -WarningAction SilentlyContinue
            '{0}x{1}' -f $double.Width, $double.Height | Should -Be '24x16'
            $large = Read-WgpuTexture $double
            # Twice the size, each texel shows what the texel at half its
            # place shows at the file's own size: no filter blends edges.
            foreach ($at in @(0, 0), @(9, 5), @(19, 3), @(23, 0)) {
                Get-Texel $large 24 $at[0] $at[1] | Should -Be (Get-Texel $small 12 ([math]::Floor($at[0] / 2)) ([math]::Floor($at[1] / 2))) -Because "texel $at"
            }
            ($warned | ForEach-Object Message) -join '|' | Should -Be 'the file was drawn at its new size, so -Filter changed nothing'
        } finally {
            Remove-Item $file -ErrorAction Ignore
        }
    }

    It 'draws a PDF''s first page at 72 dots per inch on white, at -Dpi, and with -NoBackground' -Skip:$noGpu {
        $file = Join-Path $temp "wgpu-$PID-rects.pdf"
        try {
            [IO.File]::WriteAllBytes($file, $rects)
            $records = Import-WgpuImage $file -Verbose 4>&1
            $page = $records | Where-Object { $_ -isnot [System.Management.Automation.VerboseRecord] }
            '{0}x{1}' -f $page.Width, $page.Height | Should -Be '40x30'
            $records | Where-Object { $_ -is [System.Management.Automation.VerboseRecord] } | ForEach-Object Message |
                Should -Contain 'page 1 of the PDF''s 1 was drawn at 72 dots per inch, 40x30 at that resolution, at 40x30'
            $texels = Read-WgpuTexture $page
            Get-Texel $texels 40 0 0 | Should -Be '255,255,255,255'
            Get-Texel $texels 40 2 2 | Should -Be '204,51,0,255'
            $fine = Import-WgpuImage $file -Dpi 144
            '{0}x{1}' -f $fine.Width, $fine.Height | Should -Be '80x60'
            Get-Texel (Read-WgpuTexture $fine) 80 4 4 | Should -Be '204,51,0,255'
            Get-Texel (Read-WgpuTexture (Import-WgpuImage $file -NoBackground)) 40 0 0 | Should -Be '0,0,0,0'
        } finally {
            Remove-Item $file -ErrorAction Ignore
        }
    }

    It 'opens a protected PDF with -Password as a SecureString, and says why it cannot without one' -Skip:$noGpu {
        $file = Join-Path $temp "wgpu-$PID-protected.pdf"
        try {
            [IO.File]::WriteAllBytes($file, $protected)
            $password = ConvertTo-SecureString 'wgpu' -AsPlainText -Force
            Get-Texel (Read-WgpuTexture (Import-WgpuImage $file -Password $password)) 40 2 2 | Should -Be '204,51,0,255'
            { Import-WgpuImage $file -ErrorAction Stop } | Should -Throw "$file cannot be read as an image: it is protected by a password: give it as a SecureString with -Password"
        } finally {
            Remove-Item $file -ErrorAction Ignore
        }
    }

    It 'warns that -Dpi, -NoBackground and -Password change nothing for a file that is not a PDF, and refuses a -Dpi of 0' -Skip:$noGpu {
        $file = Join-Path $temp "wgpu-$PID-not-a-pdf.png"
        try {
            New-WgpuTexture 2 2 -Data ([byte[]](0..15)) | Export-WgpuImage -Path $file
            $password = ConvertTo-SecureString 'wgpu' -AsPlainText -Force
            $null = Import-WgpuImage $file -Dpi 300 -NoBackground -Password $password -WarningVariable warned -WarningAction SilentlyContinue
            ($warned | ForEach-Object Message) -join '|' |
                Should -Be 'the file is not a PDF, so -Dpi changed nothing|the file is not a PDF, so -NoBackground changed nothing|the file is not a PDF, so -Password changed nothing'
            { Import-WgpuImage $file -Dpi 0 -ErrorAction Stop } | Should -Throw '-Dpi is a number of dots per inch above 0, not 0'
        } finally {
            Remove-Item $file -ErrorAction Ignore
        }
    }
}

Describe 'Photoshop documents' {
    BeforeAll {
        $script:temp = [IO.Path]::GetTempPath()

        # `value` as `count` big-endian bytes.
        function script:ConvertTo-BigEndian([long] $value, [int] $count) {
            [byte[]]@(for ($i = $count - 1; $i -ge 0; $i--) { ($value -shr (8 * $i)) -band 255 })
        }

        # A document of the mode given, 1 for gray, 3 for RGB and 7 for
        # multichannel, of `depth` bits a sample, its channels' planes
        # uncompressed, with the image resources and the layer and mask
        # section given.
        function script:New-PsdDocument([int] $mode, [int] $width, [int] $height, [byte[][]] $planes, [byte[]] $resources = @(), [byte[]] $layers = @(), [int] $depth = 8) {
            $out = [Collections.Generic.List[byte]]::new()
            $out.AddRange([byte[]][Text.Encoding]::ASCII.GetBytes('8BPS'))
            $out.AddRange([byte[]](0, 1, 0, 0, 0, 0, 0, 0))
            $out.AddRange([byte[]](ConvertTo-BigEndian $planes.Count 2))
            $out.AddRange([byte[]](ConvertTo-BigEndian $height 4))
            $out.AddRange([byte[]](ConvertTo-BigEndian $width 4))
            $out.AddRange([byte[]](ConvertTo-BigEndian $depth 2))
            $out.AddRange([byte[]](ConvertTo-BigEndian $mode 2))
            $out.AddRange([byte[]](ConvertTo-BigEndian 0 4))
            foreach ($section in @(, $resources) + @(, $layers)) {
                $out.AddRange([byte[]](ConvertTo-BigEndian $section.Length 4))
                $out.AddRange([byte[]]$section)
            }
            $out.AddRange([byte[]](0, 0))
            foreach ($plane in $planes) { $out.AddRange([byte[]]$plane) }
            $out.ToArray()
        }

        # An image resource block of the id given holding `data`.
        function script:New-PsdResource([int] $id, [byte[]] $data) {
            $block = [byte[]]([Text.Encoding]::ASCII.GetBytes('8BIM') + (ConvertTo-BigEndian $id 2) + [byte[]](0, 0) + (ConvertTo-BigEndian $data.Length 4) + $data)
            if ($data.Length % 2) { $block = [byte[]]($block + [byte[]]0) }
            $block
        }

        # A layer and mask section of one layer named L over the whole
        # canvas, its red, green, blue and transparency planes uncompressed.
        function script:New-PsdLayer([int] $width, [int] $height, [byte[][]] $planes) {
            $info = [Collections.Generic.List[byte]]::new()
            $info.AddRange([byte[]](ConvertTo-BigEndian 1 2))
            foreach ($edge in 0, 0, $height, $width) { $info.AddRange([byte[]](ConvertTo-BigEndian $edge 4)) }
            $info.AddRange([byte[]](ConvertTo-BigEndian 4 2))
            foreach ($id in 0, 1, 2, 0xFFFF) {
                $info.AddRange([byte[]](ConvertTo-BigEndian $id 2))
                $info.AddRange([byte[]](ConvertTo-BigEndian (2 + $planes[0].Length) 4))
            }
            $info.AddRange([byte[]][Text.Encoding]::ASCII.GetBytes('8BIMnorm'))
            $info.AddRange([byte[]](255, 0, 0, 0))
            $info.AddRange([byte[]](ConvertTo-BigEndian 12 4))
            $info.AddRange([byte[]](0, 0, 0, 0, 0, 0, 0, 0, 1, 76, 0, 0))
            foreach ($plane in $planes) {
                $info.AddRange([byte[]](0, 0))
                $info.AddRange([byte[]]$plane)
            }
            [byte[]]((ConvertTo-BigEndian $info.Count 4) + $info.ToArray())
        }

        # `values` as big-endian 32-bit floats, a 32-bit document's samples.
        function script:ConvertTo-BigEndianSingle([float[]] $values) {
            [byte[]]@(foreach ($v in $values) {
                    $bytes = [BitConverter]::GetBytes($v)
                    if ([BitConverter]::IsLittleEndian) { [array]::Reverse($bytes) }
                    $bytes
                })
        }

        # A version 2 gray display profile whose curve of no entries is the
        # identity, so its gray is linear light, as the profile Photoshop
        # embeds in a 32-bit gray document is.
        function script:New-PsdLinearGrayProfile {
            $s15 = { param([double] $v) ConvertTo-BigEndian ([long][math]::Round($v * 65536)) 4 }
            $text = [Text.Encoding]::ASCII.GetBytes("Linear Gray`0")
            $tags = [ordered]@{
                desc = [byte[]]([Text.Encoding]::ASCII.GetBytes('desc') + [byte[]]::new(4) + (ConvertTo-BigEndian $text.Length 4) + $text + [byte[]]::new(78))
                wtpt = [byte[]]([Text.Encoding]::ASCII.GetBytes('XYZ ') + [byte[]]::new(4) + (& $s15 0.9642) + (& $s15 1.0) + (& $s15 0.8249))
                kTRC = [byte[]]([Text.Encoding]::ASCII.GetBytes('curv') + [byte[]]::new(8))
            }
            $table = [Collections.Generic.List[byte]]::new()
            $body = [Collections.Generic.List[byte]]::new()
            $first = 128 + 4 + 12 * $tags.Count
            $table.AddRange([byte[]](ConvertTo-BigEndian $tags.Count 4))
            foreach ($name in $tags.Keys) {
                $data = [byte[]]$tags[$name]
                $table.AddRange([Text.Encoding]::ASCII.GetBytes($name))
                $table.AddRange([byte[]](ConvertTo-BigEndian ($first + $body.Count) 4))
                $table.AddRange([byte[]](ConvertTo-BigEndian $data.Length 4))
                $body.AddRange($data)
                while ($body.Count % 4) { $body.Add(0) }
            }
            # The header: size, CMM, version 2.1, class, space and PCS, date,
            # signature, platform to intent, D50 and the rest.
            $header = [Collections.Generic.List[byte]]::new()
            $header.AddRange([byte[]](ConvertTo-BigEndian (128 + $table.Count + $body.Count) 4))
            $header.AddRange([byte[]](0, 0, 0, 0, 2, 0x10, 0, 0))
            $header.AddRange([Text.Encoding]::ASCII.GetBytes('mntrGRAYXYZ '))
            $header.AddRange([byte[]]::new(12))
            $header.AddRange([Text.Encoding]::ASCII.GetBytes('acsp'))
            $header.AddRange([byte[]]::new(28))
            foreach ($v in 0.9642, 1.0, 0.8249) { $header.AddRange([byte[]](& $s15 $v)) }
            $header.AddRange([byte[]]::new(48))
            [byte[]]($header.ToArray() + $table.ToArray() + $body.ToArray())
        }
    }

    It 'loads a document''s composite with the transparency the file marks, taking out the white it is stored blended with' -Skip:$noGpu {
        $path = Join-Path $temp "wgpu-$PID-marked.psd"
        try {
            # Alpha identifier 0 marks the fourth channel as the composite's
            # transparency; a pixel of alpha 128 is stored over white.
            $planes = [byte[]](255, 191), [byte[]](255, 127), [byte[]](255, 64), [byte[]](0, 128)
            [IO.File]::WriteAllBytes($path, (New-PsdDocument 3 2 1 $planes (New-PsdResource 1053 ([byte[]](0, 0, 0, 0)))))
            (Read-WgpuTexture (Import-WgpuImage $path)) -join ',' | Should -Be '255,255,255,0,128,0,0,128'
        } finally {
            Remove-Item $path -ErrorAction Ignore
        }
    }

    It 'refuses a layered document whose version info says the composite is not the real merge, and says why' -Skip:$noGpu {
        $path = Join-Path $temp "wgpu-$PID-merged.psd"
        try {
            $white = [byte[]](255, 255), [byte[]](255, 255), [byte[]](255, 255)
            $layer = New-PsdLayer 2 1 ([byte[]](10, 20), [byte[]](30, 40), [byte[]](50, 60), [byte[]](255, 128))
            # Version info: a version, then hasRealMergedData 0.
            $resources = New-PsdResource 1057 ([byte[]](0, 0, 0, 1, 0))
            [IO.File]::WriteAllBytes($path, (New-PsdDocument 3 2 1 $white $resources $layer))
            { Import-WgpuImage $path } | Should -Throw '*not the real merge, and the module does not merge a document''s layers*'
        } finally {
            Remove-Item $path -ErrorAction Ignore
        }
    }

    It 'loads a 32-bit gray document as linear light in sRGB''s primaries through its gray profile, light above 1 kept, and says so' -Skip:$noGpu {
        $path = Join-Path $temp "wgpu-$PID-float-gray.psd"
        try {
            $gray = ConvertTo-BigEndianSingle 0.25, 4.0
            [IO.File]::WriteAllBytes($path, (New-PsdDocument 1 2 1 (, $gray) (New-PsdResource 1039 (New-PsdLinearGrayProfile)) -depth 32))
            $records = Import-WgpuImage $path -Verbose -WarningVariable warned -WarningAction SilentlyContinue 4>&1
            $loaded = $records | Where-Object { $_ -isnot [System.Management.Automation.VerboseRecord] }
            $warned | Should -BeNullOrEmpty
            $loaded.Format | Should -Be 'Rgba32Float'
            $texels = Read-WgpuTexture $loaded
            $texels[4..6] | ForEach-Object { $_ | Should -BeGreaterThan 1 }
            $records | Where-Object { $_ -is [System.Management.Automation.VerboseRecord] } | ForEach-Object Message |
                Should -Contain "the colors were converted from the file's color profile, Linear Gray, to linear light in sRGB's primaries, perceptually, through its gray curve"
        } finally {
            Remove-Item $path -ErrorAction Ignore
        }
    }

    It 'refuses a multichannel document and says why' {
        $path = Join-Path $temp "wgpu-$PID-multichannel.psd"
        try {
            [IO.File]::WriteAllBytes($path, (New-PsdDocument 7 1 1 ([byte[]]7, [byte[]]9)))
            { Import-WgpuImage $path } | Should -Throw '*its color mode is multichannel, whose channels are inks with no color of their own*'
        } finally {
            Remove-Item $path -ErrorAction Ignore
        }
    }
}

Describe 'DDS and KTX2 files and conversions' {
    BeforeAll {
        # A 4x4 image whose left half is 0 and right half 200 in red, and 50
        # in green: every mip level below it averages to 100 and 50.
        $script:halves = [byte[]](0..15 | ForEach-Object { if ($_ % 4 -lt 2) { 0, 50, 0, 255 } else { 200, 50, 0, 255 } })
        $script:temp = [IO.Path]::GetTempPath()
    }

    It 'saves a cube map as a DDS file and loads it back whole' -Skip:$noGpu {
        $file = Join-Path $temp "wgpu-cube-$PID.dds"
        try {
            $cube = New-WgpuTexture 4 4 -Depth 6 -MipLevels 3 -Cube -Data $halves
            $cube.IsCube | Should -BeTrue
            $cube | Export-WgpuImage -Path $file
            $loaded = Import-WgpuImage $file
            $loaded.IsCube | Should -BeTrue
            $loaded.DepthOrArrayLayers | Should -Be 6
            $loaded.MipLevelCount | Should -Be 3
            (Read-WgpuTexture $loaded) -join ',' | Should -Be ($halves -join ',')
        } finally {
            Remove-Item $file -ErrorAction Ignore
        }
    }

    It 'saves an array texture as DDS and KTX2 files and loads every layer and level back' -Skip:$noGpu {
        $layers = New-WgpuTexture 2 2 -Layers 3 -MipLevels 2 -Data ([byte[]](1..16))
        Write-WgpuTexture $layers ([byte[]](17..32)) -Layer 1
        Write-WgpuTexture $layers ([byte[]](33..48)) -Layer 2
        Write-WgpuTexture $layers ([byte[]](49..52)) -Layer 2 -MipLevel 1
        foreach ($extension in 'dds', 'ktx2') {
            $file = Join-Path $temp "wgpu-array-$PID.$extension"
            try {
                $layers | Export-WgpuImage -Path $file
                $loaded = Import-WgpuImage $file
                $loaded.Dimension | Should -Be 'D2' -Because ".$extension"
                $loaded.IsCube | Should -BeFalse -Because ".$extension"
                $loaded.DepthOrArrayLayers | Should -Be 3 -Because ".$extension"
                $loaded.MipLevelCount | Should -Be 2 -Because ".$extension"
                (Read-WgpuTexture $loaded -Layer 1) -join ',' | Should -Be ((17..32) -join ',') -Because ".$extension"
                (Read-WgpuTexture $loaded -Layer 2 -MipLevel 1) -join ',' | Should -Be '49,50,51,52' -Because ".$extension"
            } finally {
                Remove-Item $file -ErrorAction Ignore
            }
        }
    }

    It 'writes a KTX2 file with Zstandard and reads it back' -Skip:$noGpu {
        $file = Join-Path $temp "wgpu-zstd-$PID.ktx2"
        try {
            New-WgpuTexture 4 4 -Data $halves | Export-WgpuImage -Path $file -Zstd
            (Read-WgpuTexture (Import-WgpuImage $file)) -join ',' | Should -Be ($halves -join ',')
        } finally {
            Remove-Item $file -ErrorAction Ignore
        }
    }

    It 'writes layers as cube faces with -Cube, and refuses layers that are not' -Skip:$noGpu {
        $file = Join-Path $temp "wgpu-faces-$PID.ktx2"
        try {
            New-WgpuTexture 2 2 -Depth 6 | Export-WgpuImage -Path $file -Cube
            (Import-WgpuImage $file).IsCube | Should -BeTrue
            { New-WgpuTexture 2 2 -Depth 4 | Export-WgpuImage -Path $file -Cube } | Should -Throw 'cube faces are square 2D layers, six to a cube; a 2x2 texture with 4 layers is not that'
            { New-WgpuTexture 4 4 -Cube } | Should -Throw 'cube faces are square 2D layers, six to a cube; a 4x4 texture with 1 layer is not that'
        } finally {
            Remove-Item $file -ErrorAction Ignore
        }
    }

    It 'gives an image mip levels as it loads, and says so' -Skip:$noGpu {
        $file = Join-Path $temp "wgpu-mips-$PID.png"
        try {
            New-WgpuTexture 4 4 -Data $halves | Export-WgpuImage -Path $file
            $records = Import-WgpuImage $file -MipLevels 0 -Verbose 4>&1
            $texture = $records | Where-Object { $_ -isnot [System.Management.Automation.VerboseRecord] }
            $said = $records | Where-Object { $_ -is [System.Management.Automation.VerboseRecord] } | ForEach-Object Message
            $texture.MipLevelCount | Should -Be 3
            (Read-WgpuTexture $texture -MipLevel 0) -join ',' | Should -Be ($halves -join ',')
            (Read-WgpuTexture $texture -MipLevel 2) -join ',' | Should -Be '100,50,0,255'
            $said | Should -Contain 'mip levels 1 to 2 were rendered on the GPU in Rgba16Float, each from the level above it'
            $texture.Notes | Should -Contain 'mip levels 1 to 2 were packed into Rgba8Unorm on the CPU'
        } finally {
            Remove-Item $file -ErrorAction Ignore
        }
    }

    It 'converts an image into a mipmapped BC7 texture on the GPU, and writes it to a DDS file and back block for block' -Skip:$noGpu {
        if ((Get-WgpuDevice).Features -notcontains 'TextureCompressionBc') { Set-ItResult -Skipped -Because 'the device has no TextureCompressionBc'; return }
        $png = Join-Path $temp "wgpu-bc7-$PID.png"
        $dds = Join-Path $temp "wgpu-bc7-$PID.dds"
        try {
            New-WgpuTexture 4 4 -Data $halves | Export-WgpuImage -Path $png
            $bc = Import-WgpuImage $png -Format Bc7RgbaUnormSrgb -MipLevels 0
            $bc.Format | Should -Be 'Bc7RgbaUnormSrgb'
            $bc.MipLevelCount | Should -Be 3
            $bc.Notes | Should -Contain "mip levels 0 to 2 were encoded into Bc7RgbaUnormSrgb on the GPU by block_compression's BC7 opaque basic settings"
            $bc | Export-WgpuImage -Path $dds
            $loaded = Import-WgpuImage $dds
            $loaded.Format | Should -Be 'Bc7RgbaUnormSrgb'
            $loaded.MipLevelCount | Should -Be 3
            $loaded.Notes | Should -BeNullOrEmpty
            foreach ($level in 0..2) {
                (Read-WgpuTexture $loaded -MipLevel $level) -join ',' | Should -Be ((Read-WgpuTexture $bc -MipLevel $level) -join ',') -Because "mip level $level"
            }
        } finally {
            Remove-Item $png, $dds -ErrorAction Ignore
        }
    }

    It 'saves a BC7 texture as a PNG file, decoded on the CPU or on the GPU' -Skip:$noGpu {
        if ((Get-WgpuDevice).Features -notcontains 'TextureCompressionBc') { Set-ItResult -Skipped -Because 'the device has no TextureCompressionBc'; return }
        $cpu = Join-Path $temp "wgpu-decoded-cpu-$PID.png"
        $gpu = Join-Path $temp "wgpu-decoded-gpu-$PID.png"
        $method = Join-Path $temp "wgpu-decoded-method-$PID.png"
        $dds = Join-Path $temp "wgpu-decoded-$PID.dds"
        try {
            $bc = (New-WgpuTexture 4 4 -Data $halves).Compress('Bc7RgbaUnorm', $null, $null)
            ($bc | Export-WgpuImage -Path $cpu -Verbose 4>&1).Message | Should -Be 'the Bc7RgbaUnorm texels were decoded on the CPU into Rgba8Unorm'
            ($bc | Export-WgpuImage -Path $gpu -Decoder Gpu -Verbose 4>&1).Message | Should -Be 'the Bc7RgbaUnorm texels were decoded on the GPU into Rgba16Float and packed into Rgba8Unorm on the CPU'
            $bc.SaveAsync($method, 'Gpu', [Threading.CancellationToken]::None).GetAwaiter().GetResult()
            $decoded = Read-WgpuTexture (Import-WgpuImage $cpu)
            (Read-WgpuTexture (Import-WgpuImage $gpu)) -join ',' | Should -Be ($decoded -join ',')
            (Read-WgpuTexture (Import-WgpuImage $method)) -join ',' | Should -Be ($decoded -join ',')
            # Each row stays darker in red on the left than on the right, and opaque.
            (0..3 | ForEach-Object { $decoded[$_ * 16] -lt $decoded[$_ * 16 + 12] }) | Should -Not -Contain $false
            (0..15 | ForEach-Object { $decoded[$_ * 4 + 3] } | Sort-Object -Unique) | Should -Be 255
            { $bc | Export-WgpuImage -Path $dds -Decoder Gpu } | Should -Throw "-Decoder applies to an image file; a DDS or KTX2 file holds a block-compressed texture's blocks as they are"
            { $bc.Save($dds, 'Cpu') } | Should -Throw "*-Decoder applies to an image file*"
            { New-WgpuTexture 4 4 | Export-WgpuImage -Path $cpu -Decoder Cpu } | Should -Throw '-Decoder applies to a block-compressed texture, and Rgba8Unorm texels need no decoding'
        } finally {
            Remove-Item $cpu, $gpu, $method, $dds -ErrorAction Ignore
        }
    }

    It 'encodes an image in a Basis Universal format and transcodes it for the device' -Skip:$noGpu {
        $file = Join-Path $temp "wgpu-basis-$PID.png"
        try {
            New-WgpuTexture 4 4 -Data $halves | Export-WgpuImage -Path $file
            $basis = Import-WgpuImage $file -Format BasisUastc4x4 -MipLevels 0 -Quality 50
            $basis.MipLevelCount | Should -Be 3
            $basis.Notes | Should -Contain 'the texture was encoded into BasisUastc4x4 by Basis Universal'
            ($basis.Notes -join "`n") | Should -BeLike "*the KTX2 file's Basis Universal texels were transcoded into*"
        } finally {
            Remove-Item $file -ErrorAction Ignore
        }
    }

    It 'writes Basis Universal KTX2 files in ETC1S and UASTC, and loads each into a format the device holds' -Skip:$noGpu {
        $bare = Open-WgpuDevice -Feature @()
        try {
            foreach ($format in 'BasisEtc1s', 'BasisUastc4x4') {
                $file = Join-Path $temp "wgpu-$format-$PID.ktx2"
                try {
                    New-WgpuTexture 4 4 -Data $halves | Export-WgpuImage -Path $file -Format $format -MipLevels 0
                    $loaded = Import-WgpuImage $file
                    $loaded.MipLevelCount | Should -Be 3 -Because $format
                    ($loaded.Notes -join "`n") | Should -BeLike "*the KTX2 file's Basis Universal texels were transcoded into*" -Because $format
                    $plain = Import-WgpuImage $file -Device $bare
                    $plain.Format | Should -Be 'Rgba8Unorm' -Because "$format on a device with no compression features"
                    $plain.MipLevelCount | Should -Be 3 -Because $format
                    $plain.Notes | Should -Contain "the KTX2 file's Basis Universal texels were transcoded into Rgba8Unorm" -Because $format
                } finally {
                    Remove-Item $file -ErrorAction Ignore
                }
            }
        } finally {
            $bare | Close-WgpuDevice
        }
    }

    It 'writes a DDS file in a BC format with mip levels from an uncompressed texture' -Skip:$noGpu {
        $file = Join-Path $temp "wgpu-bc1-$PID.dds"
        try {
            New-WgpuTexture 8 8 -Data ([byte[]](@(255, 0, 0, 255) * 64)) | Export-WgpuImage -Path $file -Format Bc1RgbaUnorm -MipLevels 0
            $loaded = Import-WgpuImage $file -WarningAction SilentlyContinue
            $loaded.MipLevelCount | Should -Be 4
            # BC1 where the device holds it, and decoded on the CPU where not.
            $loaded.Format | Should -BeIn 'Bc1RgbaUnorm', 'Rgba8Unorm'
        } finally {
            Remove-Item $file -ErrorAction Ignore
        }
    }

    It 'decodes a BC file on the CPU for a device without TextureCompressionBc, and warns' -Skip:$noGpu {
        $file = Join-Path $temp "wgpu-decode-$PID.dds"
        $bare = Open-WgpuDevice -Feature @()
        try {
            New-WgpuTexture 8 8 -Data ([byte[]](@(255, 0, 0, 255) * 64)) | Export-WgpuImage -Path $file -Format Bc1RgbaUnorm -MipLevels 0
            $loaded = Import-WgpuImage $file -Device $bare -WarningAction SilentlyContinue -WarningVariable warned
            $loaded.Format | Should -Be 'Rgba8Unorm'
            $loaded.MipLevelCount | Should -Be 4
            (Read-WgpuTexture $loaded -MipLevel 3) -join ',' | Should -Be '255,0,0,255'
            $warned.Count | Should -Be 1
            $warned[0].Message | Should -Be 'the device has no TextureCompressionBc, so the Bc1RgbaUnorm texels were decoded on the CPU into Rgba8Unorm'
            $loaded.Notes | Should -Contain 'the device has no TextureCompressionBc, so the Bc1RgbaUnorm texels were decoded on the CPU into Rgba8Unorm'
        } finally {
            $bare | Close-WgpuDevice
            Remove-Item $file -ErrorAction Ignore
        }
    }

    It 'warns that -Srgb did nothing for a file whose format has no sRGB form' -Skip:$noGpu {
        $file = Join-Path $temp "wgpu-r8-$PID.dds"
        try {
            New-WgpuTexture 2 2 -Format R8Unorm | Export-WgpuImage -Path $file
            $loaded = Import-WgpuImage $file -Srgb -WarningAction SilentlyContinue -WarningVariable warned
            $loaded.Format | Should -Be 'R8Unorm'
            $warned[0].Message | Should -Be '-Srgb did nothing for the file: its format, R8Unorm, has no sRGB form'
            $loaded.Notes | Should -Contain '-Srgb did nothing for the file: its format, R8Unorm, has no sRGB form'
        } finally {
            Remove-Item $file -ErrorAction Ignore
        }
    }

    It 'renders levels with -MipFilter and -MipEdge, a sampler''s address mode among the edges' -Skip:$noGpu {
        $png = Join-Path $temp "wgpu-mipfilter-$PID.png"
        $dds = Join-Path $temp "wgpu-mipfilter-$PID.dds"
        try {
            New-WgpuTexture 4 4 -Data $halves | Export-WgpuImage -Path $png
            # Level 1's first red value by the Triangle filter: 29 when the
            # texel past the left edge is left out, 50 when the right edge's
            # 200 wraps in, and 25 when the edge's 0 is mirrored.
            (Read-WgpuTexture (Import-WgpuImage $png -MipLevels 0 -MipFilter Triangle) -MipLevel 1)[0] | Should -Be 29
            $wrapped = Import-WgpuImage $png -MipLevels 0 -MipFilter Triangle -MipEdge Repeat
            (Read-WgpuTexture $wrapped -MipLevel 1)[0] | Should -Be 50
            $wrapped.Notes | Should -Contain 'mip levels 1 to 2 were rendered on the GPU in Rgba16Float, each from level 0 by the Triangle filter, its edges wrapped'
            (Read-WgpuTexture (Import-WgpuImage $png -MipLevels 0 -MipFilter Triangle -MipEdge ([Wgpu.AddressMode]::MirrorRepeat)) -MipLevel 1)[0] | Should -Be 25
            New-WgpuTexture 4 4 -Data $halves | Export-WgpuImage -Path $dds -MipLevels 0 -MipFilter Triangle -MipEdge Wrap
            (Read-WgpuTexture (Import-WgpuImage $dds) -MipLevel 1)[0] | Should -Be 50
        } finally {
            Remove-Item $png, $dds -ErrorAction Ignore
        }
    }

    It 'refuses a mip filter or edge that changes nothing for any file, and warns when the file has the levels' -Skip:$noGpu {
        $png = Join-Path $temp "wgpu-mipnothing-$PID.png"
        $dds = Join-Path $temp "wgpu-mipnothing-$PID.dds"
        try {
            New-WgpuTexture 4 4 -Data $halves | Export-WgpuImage -Path $png
            { Import-WgpuImage $png -MipFilter Kaiser } | Should -Throw '-MipFilter, -MipEdge, -KeepCoverage and -NormalMips choose how the mip levels -MipLevels adds and a transform renders again are made, and the call gives neither: give -MipLevels'
            { Import-WgpuImage $png -KeepCoverage 0.5 } | Should -Throw '-MipFilter, -MipEdge, -KeepCoverage and -NormalMips choose how the mip levels*'
            { Import-WgpuImage $png -NormalMips Renormalize } | Should -Throw '-MipFilter, -MipEdge, -KeepCoverage and -NormalMips choose how the mip levels*'
            { Import-WgpuImage $png -MipLevels 1 -MipFilter Kaiser -Width 2 } | Should -Throw '-MipFilter, -MipEdge, -KeepCoverage and -NormalMips choose how the levels below level 0 are made, and -MipLevels 1 keeps level 0 alone'
            { Import-WgpuImage $png -MipLevels 0 -MipEdge Wrap } | Should -Throw 'Box, the filter when none is given, reads only the texels each new texel covers and never past an edge, so the Wrap edge changes nothing; Triangle, Kaiser, Mitchell and Lanczos read past one'
            { Import-WgpuImage $png -MipLevels 0 -MipFilter Max -MipEdge Mirror } | Should -Throw 'Max reads only the texels each new texel covers*'
            { Import-WgpuImage $png -MipLevels 0 -MipFilter Kaiser -MipEdge ClampToBorder } | Should -Throw '*a mip level has no border color*'
            { New-WgpuTexture 4 4 | Export-WgpuImage -Path $png -MipFilter Kaiser } | Should -Throw '-Format, -MipLevels, -MipFilter, -MipEdge, -KeepCoverage, -NormalMips, -TwoChannel, -Quality, -Effort, -Zstd and -Cube apply to .dds and .ktx2 files'
            New-WgpuTexture 4 4 -MipLevels 3 -Data $halves | Export-WgpuImage -Path $dds
            $loaded = Import-WgpuImage $dds -MipLevels 0 -MipFilter Kaiser -WarningAction SilentlyContinue -WarningVariable warned
            $loaded.MipLevelCount | Should -Be 3
            $warned[0].Message | Should -Be 'no mip level was rendered, as the file has every level the call asks for, so the mip filter changed nothing'
        } finally {
            Remove-Item $png, $dds -ErrorAction Ignore
        }
    }

    It 'refuses a setting that does nothing for the format it goes with' -Skip:$noGpu {
        $png = Join-Path $temp "wgpu-refuse-$PID.png"
        $dds = Join-Path $temp "wgpu-refuse-$PID.dds"
        try {
            $texture = New-WgpuTexture 4 4 -Data $halves
            $texture | Export-WgpuImage -Path $png
            { Import-WgpuImage $png -Format Bc7RgbaUnorm -Quality 50 } | Should -Throw '-Quality applies to a Basis Universal -Format with a quality setting'
            { Import-WgpuImage $png -Format BasisAstc4x4 -Quality 50 } | Should -Throw 'BasisAstc4x4 has no quality setting*'
            { Import-WgpuImage $png -Format Bc1RgbaUnorm -Effort 5 } | Should -Throw '-Effort applies to Bc7RgbaUnorm, Bc7RgbaUnormSrgb and Bc6hRgbUfloat, and Bc1RgbaUnorm has a single setting'
            { Import-WgpuImage $png -Format Bc7RgbaUnorm -Srgb } | Should -Throw '-Srgb applies to an image file loaded without -Format or with a Basis Universal format*'
            { $texture | Export-WgpuImage -Path $dds -Zstd } | Should -Throw '-Zstd applies to a .ktx2 file in a GPU format*'
            { $texture | Export-WgpuImage -Path $png -MipLevels 0 } | Should -Throw '-Format, -MipLevels, -MipFilter, -MipEdge, -KeepCoverage, -NormalMips, -TwoChannel, -Quality, -Effort, -Zstd and -Cube apply to .dds and .ktx2 files'
            { $texture | Export-WgpuImage -Path $png -TwoChannel } | Should -Throw '-Format, -MipLevels, -MipFilter, -MipEdge, -KeepCoverage, -NormalMips, -TwoChannel*'
            { $texture | Export-WgpuImage -Path $dds -Format Bc6hRgbFloat } | Should -Throw 'the module encodes BC6H unsigned only*'
            { $texture | Export-WgpuImage -Path $dds -Format BasisEtc1s } | Should -Throw 'a DDS file cannot hold BasisEtc1s: give a .ktx2 path'
        } finally {
            Remove-Item $png, $dds -ErrorAction Ignore
        }
    }
}

Describe 'GenerateMips and Compress' {
    BeforeAll {
        $script:halves = [byte[]](0..15 | ForEach-Object { if ($_ % 4 -lt 2) { 0, 50, 0, 255 } else { 200, 50, 0, 255 } })
        $script:red = @'
@vertex
fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    var corners = array<vec2<f32>, 3>(vec2<f32>(-1.0, -1.0), vec2<f32>(3.0, -1.0), vec2<f32>(-1.0, 3.0));
    return vec4<f32>(corners[i], 0.0, 1.0);
}
@fragment
fn fs() -> @location(0) vec4<f32> { return vec4<f32>(1.0, 0.0, 0.0, 1.0); }
'@
    }

    It 'fills a texture''s own mip levels, and refuses one with a single level' -Skip:$noGpu {
        $texture = New-WgpuTexture 4 4 -MipLevels 3 -Data $halves
        $texture.GenerateMips() | Should -BeNullOrEmpty
        (Read-WgpuTexture $texture -MipLevel 2) -join ',' | Should -Be '100,50,0,255'
        { (New-WgpuTexture 4 4).GenerateMips() } | Should -Throw '*GenerateMips(0) returns a copy with the full chain*'
    }

    It 'returns a copy with the levels asked for, whose Notes say how' -Skip:$noGpu {
        $texture = New-WgpuTexture 4 4 -Data $halves -Label photo
        $copy = $texture.GenerateMips(0)
        $copy.MipLevelCount | Should -Be 3
        $copy.Label | Should -Be 'photo'
        (Read-WgpuTexture $copy -MipLevel 2) -join ',' | Should -Be '100,50,0,255'
        $copy.Notes | Should -Contain 'mip levels 1 to 2 were rendered on the GPU in Rgba16Float, each from the level above it'
        $texture.GenerateMipsAsync(2).GetAwaiter().GetResult().MipLevelCount | Should -Be 2
    }

    It 'renders levels with the filter and edge given, a sampler''s address mode among the edges' -Skip:$noGpu {
        $texture = New-WgpuTexture 4 4 -Data $halves
        # Level 1's first red value by the Triangle filter: 29 with the
        # texel past the left edge left out, 50 wrapped, 25 mirrored.
        (Read-WgpuTexture ($texture.GenerateMips(0, 'Triangle', $null)) -MipLevel 1)[0] | Should -Be 29
        $wrapped = $texture.GenerateMips(0, 'Triangle', 'Repeat')
        (Read-WgpuTexture $wrapped -MipLevel 1)[0] | Should -Be 50
        $wrapped.Notes | Should -Contain 'mip levels 1 to 2 were rendered on the GPU in Rgba16Float, each from level 0 by the Triangle filter, its edges wrapped'
        $mirrored = $texture.GenerateMipsAsync(0, 'Triangle', [Wgpu.AddressMode]::MirrorRepeat).GetAwaiter().GetResult()
        (Read-WgpuTexture $mirrored -MipLevel 1)[0] | Should -Be 25
        $own = New-WgpuTexture 4 4 -MipLevels 3 -Data $halves
        $own.GenerateMips($null, 'Max', $null) | Should -BeNullOrEmpty
        (Read-WgpuTexture $own -MipLevel 2) -join ',' | Should -Be '200,50,0,255'
        $own.GenerateMips(2, 'Kaiser', $null).Notes | Should -Be 'no mip level was rendered, as the texture has every level the call asks for, so the mip filter changed nothing'
        { $own.GenerateMips(0, $null, 'Wrap') } | Should -Throw '*Box, the filter when none is given, reads only the texels each new texel covers*'
        { $own.GenerateMips(0, 'Triangle', 'Sideways') } | Should -Throw '*Sideways is not a mip edge*'
    }

    It 'compresses a copy and says how in its Notes' -Skip:$noGpu {
        if ((Get-WgpuDevice).Features -notcontains 'TextureCompressionBc') { Set-ItResult -Skipped -Because 'the device has no TextureCompressionBc'; return }
        $texture = New-WgpuTexture 4 4 -Data $halves
        $bc = $texture.Compress('Bc7RgbaUnorm', $null, 0)
        $bc.Format | Should -Be 'Bc7RgbaUnorm'
        $bc.Notes | Should -Contain "mip level 0 was encoded into Bc7RgbaUnorm on the GPU by block_compression's BC7 opaque ultra fast settings"
        $basis = $texture.CompressAsync('BasisEtc1s', 80, $null).GetAwaiter().GetResult()
        $basis.Notes | Should -Contain 'the texture was encoded into BasisEtc1s by Basis Universal'
        { $texture.Compress('Rgba16Float', $null, $null) } | Should -Throw '*Compress takes a BC format or a Basis Universal format, not Rgba16Float*'
    }

    It 'gives a rendered texture mip levels' -Skip:$noGpu {
        $rendered = Invoke-WgpuRender $red 8 8 -VertexCount 3
        $mipped = $rendered.GenerateMips(0)
        $mipped.MipLevelCount | Should -Be 4
        $mipped.Label | Should -Be 'Invoke-WgpuRender'
        $mipped.Usage | Should -Contain 'RenderAttachment'
        (Read-WgpuTexture $mipped -MipLevel 3) -join ',' | Should -Be '255,0,0,255'
        $mipped.Notes | Should -Contain 'mip levels 1 to 3 were rendered on the GPU in Rgba16Float, each from the level above it'
    }

    It 'compresses a rendered texture, leaving out the usages the BC format does not allow' -Skip:$noGpu {
        if ((Get-WgpuDevice).Features -notcontains 'TextureCompressionBc') { Set-ItResult -Skipped -Because 'the device has no TextureCompressionBc'; return }
        $rendered = Invoke-WgpuRender $red 8 8 -VertexCount 3
        $bc = $rendered.Compress('Bc1RgbaUnorm', $null, $null)
        $bc.Format | Should -Be 'Bc1RgbaUnorm'
        $bc.Usage | Should -Not -Contain 'RenderAttachment'
        $bc.Notes | Should -Contain 'mip level 0 was encoded into Bc1RgbaUnorm on the GPU by block_compression'
        # The copy leaves out StorageBinding only where the device allows it
        # for the source's format and not for Bc1RgbaUnorm: Metal allows
        # read-only storage of every format.
        $allowed = (Get-WgpuTextureFormat Bc1RgbaUnorm | Where-Object Name -EQ Bc1RgbaUnorm).AllowedUsages
        $leftOut = if ($rendered.Usage -contains 'StorageBinding' -and $allowed -notcontains 'StorageBinding') { 'StorageBinding and RenderAttachment usages' } else { 'RenderAttachment usage' }
        $bc.Notes | Should -Contain "the new texture leaves out the source's $leftOut, which Bc1RgbaUnorm does not allow"
    }
}

Describe 'Size and orientation' {
    BeforeAll {
        # Sixteen texels whose bytes count up from 0, so a texel that lands
        # in the wrong place shows; texel (x, y) of the 4x4 texture is bytes
        # (y * 4 + x) * 4 and the three after.
        $script:counting = [byte[]](0..63)
        $script:halves = [byte[]](0..15 | ForEach-Object { if ($_ % 4 -lt 2) { 0, 50, 0, 255 } else { 200, 50, 0, 255 } })
        $script:temp = [IO.Path]::GetTempPath()
    }

    It 'resizes to a width, keeping the aspect ratio, and says how' -Skip:$noGpu {
        $texture = New-WgpuTexture 4 4 -Data $halves
        $records = $texture | Resize-WgpuTexture -Width 2 -Verbose 4>&1
        $resized = $records | Where-Object { $_ -isnot [System.Management.Automation.VerboseRecord] }
        $said = $records | Where-Object { $_ -is [System.Management.Automation.VerboseRecord] } | ForEach-Object Message
        $resized.Width, $resized.Height -join 'x' | Should -Be '2x2'
        (Read-WgpuTexture $resized) -join ',' | Should -Be ((0, 50, 0, 255, 200, 50, 0, 255) * 2 -join ',')
        $said | Should -Contain 'the texture was resized to 2x2 with the Box filter on the GPU in Rgba16Float'
        $resized.Notes | Should -Contain 'the texture was resized to 2x2 with the Box filter on the GPU in Rgba16Float'
        $texture.Width | Should -Be 4 -Because 'the texture is left as it is'
    }

    It 'resizes into the -Format named' -Skip:$noGpu {
        $resized = New-WgpuTexture 4 4 -Data $halves | Resize-WgpuTexture -Width 2 -Format Rgba32Float
        $resized.Format | Should -Be 'Rgba32Float'
        $resized.Width, $resized.Height -join 'x' | Should -Be '2x2'
        # Resized on the GPU in Rgba16Float, so within its precision.
        $values = Read-WgpuTexture $resized
        $expected = 0, (50 / 255), 0, 1, (200 / 255), (50 / 255), 0, 1
        (0..7 | ForEach-Object { [math]::Abs($values[$_] - $expected[$_]) } | Measure-Object -Maximum).Maximum | Should -BeLessThan 0.001
    }

    It 'scales with a chosen filter, and rounds to powers of two after a size limit' -Skip:$noGpu {
        $nearest = New-WgpuTexture 4 4 -Data $counting | Resize-WgpuTexture -Scale 0.5 -Filter Nearest
        # Nearest takes the texel each new texel's center lands in: (1, 1), (3, 1), (1, 3) and (3, 3).
        (Read-WgpuTexture $nearest) -join ',' | Should -Be ((20..23 + 28..31 + 52..55 + 60..63) -join ',')
        $rounded = New-WgpuTexture 6 3 | Resize-WgpuTexture -PowerOfTwo Nearest
        $rounded.Width, $rounded.Height -join 'x' | Should -Be '8x4'
        $limited = New-WgpuTexture 16 8 | Resize-WgpuTexture -MaxSize 6 -PowerOfTwo Down
        $limited.Width, $limited.Height -join 'x' | Should -Be '4x2'
    }

    It 'crops a region, a size by its anchor, and margins' -Skip:$noGpu {
        $texture = New-WgpuTexture 4 4 -Data $counting
        (Read-WgpuTexture ($texture | Resize-WgpuTexture -Crop 1, 1, 2, 2)) -join ',' | Should -Be ((20..27 + 36..43) -join ',')
        (Read-WgpuTexture ($texture | Resize-WgpuTexture -Crop 2, 2 -Anchor BottomRight)) -join ',' | Should -Be ((40..47 + 56..63) -join ',')
        $margins = $texture | Resize-WgpuTexture -CropMargin 1, 0, 1, 2
        $margins.Width, $margins.Height -join 'x' | Should -Be '2x2'
        (Read-WgpuTexture $margins) -join ',' | Should -Be ((4..11 + 20..27) -join ',')
    }

    It 'pads onto a larger canvas, and crops, resizes and pads in that order whatever order they are given' -Skip:$noGpu {
        $gray = New-WgpuTexture 2 2 -Data ([byte[]](@(9, 9, 9, 255) * 4))
        $padded = $gray | Resize-WgpuTexture -Pad 3, 2 -Anchor TopLeft -PadColor 1, 0, 0, 1
        (Read-WgpuTexture $padded) -join ',' | Should -Be ((9, 9, 9, 255, 9, 9, 9, 255, 255, 0, 0, 255) * 2 -join ',')
        # Cropped to the top 4x2, resized to 2x1, and padded to 3x3 with
        # transparent black, the image in the middle row.
        $all = New-WgpuTexture 4 4 -Data $halves | Resize-WgpuTexture -Pad 3, 3 -Width 2 -Crop 0, 0, 4, 2
        $all.Width, $all.Height -join 'x' | Should -Be '3x3'
        $clear = (0, 0, 0, 0) * 3
        (Read-WgpuTexture $all) -join ',' | Should -Be (($clear + (0, 50, 0, 255, 200, 50, 0, 255, 0, 0, 0, 0) + $clear) -join ',')
    }

    It 'keeps as many mip levels as the new size has, rendered again from level 0' -Skip:$noGpu {
        $smaller = New-WgpuTexture 4 4 -MipLevels 3 -Data $halves | Resize-WgpuTexture -Width 2
        $smaller.MipLevelCount | Should -Be 2
        (Read-WgpuTexture $smaller -MipLevel 1) -join ',' | Should -Be '100,50,0,255'
    }

    It 'refuses what it cannot do, and says why' -Skip:$noGpu {
        $texture = New-WgpuTexture 4 4
        { $texture | Resize-WgpuTexture } | Should -Throw 'give a new size, a crop or padding*'
        { $texture | Resize-WgpuTexture -Crop 1, 1, 2, 2 -CropMargin 1, 1, 1, 1 } | Should -Throw 'give -Crop or -CropMargin, not both'
        { $texture | Resize-WgpuTexture -Pad 2, 2 } | Should -Throw '-Pad grows the canvas, and 2x2 is smaller than the 4x4 texture it pads'
        { $texture | Resize-WgpuTexture -Filter Lanczos } | Should -Throw '-Filter applies with a new size*'
        { $texture | Resize-WgpuTexture -Crop 3, 3, 2, 2 } | Should -Throw 'the region at 3,3 of 2x2 texels is not inside the 4x4 texture'
        $texture | Resize-WgpuTexture -Scale 0 -ErrorAction SilentlyContinue -ErrorVariable failed
        $failed[0].FullyQualifiedErrorId | Should -BeLike 'WgpuResize,*'
    }

    It 'flips and rotates, turning before it flips' -Skip:$noGpu {
        $row = New-WgpuTexture 2 1 -Data ([byte[]](1..8))
        (Read-WgpuTexture ($row | Convert-WgpuTexture -Flip Horizontal)) -join ',' | Should -Be '5,6,7,8,1,2,3,4'
        $column = $row | Convert-WgpuTexture -Rotate 90
        $column.Width, $column.Height -join 'x' | Should -Be '1x2'
        (Read-WgpuTexture $column) -join ',' | Should -Be '1,2,3,4,5,6,7,8'
        (Read-WgpuTexture ($row | Convert-WgpuTexture -Flip Vertical -Rotate 90)) -join ',' | Should -Be '5,6,7,8,1,2,3,4'
    }

    It 'refuses to flip or rotate a cube map, and a turn that is not a quarter' -Skip:$noGpu {
        { New-WgpuTexture 2 2 -Depth 6 -Cube | Convert-WgpuTexture -Flip Vertical } | Should -Throw 'a cube map is not flipped or rotated, as its faces would no longer meet at their edges'
        { New-WgpuTexture 2 2 | Convert-WgpuTexture -Rotate 45 } | Should -Throw 'a rotation is 90, 180 or 270 degrees clockwise, not 45'
        { New-WgpuTexture 2 2 | Convert-WgpuTexture } | Should -Throw 'give -Flip, -Rotate, -Format, a filter, -Blur, -Sharpen or -Edges, or a change to the channels or colors*'
    }

    It 'resizes, crops, pads, flips and rotates through methods and their Async forms' -Skip:$noGpu {
        $texture = New-WgpuTexture 4 4 -Data $counting -Label photo
        $half = $texture.Resize(2, 0, 'Nearest')
        $half.Width, $half.Height -join 'x' | Should -Be '2x2'
        $half.Label | Should -Be 'photo'
        (Read-WgpuTexture $half) -join ',' | Should -Be ((20..23 + 28..31 + 52..55 + 60..63) -join ',')
        (Read-WgpuTexture $texture.Crop(1, 1, 2, 2)) -join ',' | Should -Be ((20..27 + 36..43) -join ',')
        $texture.Pad(6, 4, 'Left', $null).Width | Should -Be 6
        (Read-WgpuTexture $texture.Flip('Vertical'))[0..3] -join ',' | Should -Be '48,49,50,51'
        (Read-WgpuTexture $texture.Rotate(180))[0..3] -join ',' | Should -Be '60,61,62,63'
        $texture.ResizeAsync(1, 1, 'Box').GetAwaiter().GetResult().Width | Should -Be 1
        $texture.CropAsync(0, 0, 3, 1).GetAwaiter().GetResult().Width | Should -Be 3
        $texture.PadAsync(5, 5, 'Center', @(1, 1, 1, 1)).GetAwaiter().GetResult().Height | Should -Be 5
        (Read-WgpuTexture $texture.FlipAsync('Horizontal').GetAwaiter().GetResult())[0..3] -join ',' | Should -Be '12,13,14,15'
        $texture.RotateAsync(270).GetAwaiter().GetResult().Width | Should -Be 4
        { $texture.Rotate(45) } | Should -Throw '*a rotation is 90, 180 or 270 degrees clockwise, not 45*'
    }

    It 'transforms on Import-WgpuImage and Export-WgpuImage, before -Format converts' -Skip:$noGpu {
        $png = Join-Path $temp "wgpu-transform-$PID.png"
        $dds = Join-Path $temp "wgpu-transform-$PID.dds"
        try {
            # Cropped to the top 4x2, then turned to 2x4.
            New-WgpuTexture 4 4 -Data $counting | Export-WgpuImage -Path $png -Rotate 90 -Crop 0, 0, 4, 2
            $saved = Import-WgpuImage $png
            $saved.Width, $saved.Height -join 'x' | Should -Be '2x4'
            $gray = Import-WgpuImage $png -Width 1 -Format R8Unorm
            $gray.Width, $gray.Height -join 'x' | Should -Be '1x2'
            $gray.Format | Should -Be 'R8Unorm'
            New-WgpuTexture 4 4 -MipLevels 3 -Data $halves | Export-WgpuImage -Path $dds -Pad 8, 8 -Anchor TopLeft
            $loaded = Import-WgpuImage $dds
            $loaded.Width | Should -Be 8
            $loaded.MipLevelCount | Should -Be 3
            (Read-WgpuTexture $loaded)[0..3] -join ',' | Should -Be '0,50,0,255'
        } finally {
            Remove-Item $png, $dds -ErrorAction Ignore
        }
    }

    It 'saves an image only from a layer the texture has, transformed or not' -Skip:$noGpu {
        $png = Join-Path $temp "wgpu-layer-$PID.png"
        try {
            { New-WgpuTexture 4 4 | Export-WgpuImage -Path $png -Layer 1 } | Should -Throw "layer 1 is past the texture's single layer"
            { New-WgpuTexture 4 4 -Depth 2 | Export-WgpuImage -Path $png -Layer 2 -Width 2 } | Should -Throw "layer 2 is past the texture's 2 layers"
            Test-Path $png | Should -BeFalse
        } finally {
            Remove-Item $png -ErrorAction Ignore
        }
    }

    It 'encodes a BC7 texture again in BC7 after resizing it' -Skip:$noGpu {
        if ((Get-WgpuDevice).Features -notcontains 'TextureCompressionBc') { Set-ItResult -Skipped -Because 'the device has no TextureCompressionBc'; return }
        $bc = (New-WgpuTexture 8 8 -Data ([byte[]](@(255, 0, 0, 255) * 64))).Compress('Bc7RgbaUnorm', $null, $null)
        $smaller = $bc | Resize-WgpuTexture -Width 4
        $smaller.Format | Should -Be 'Bc7RgbaUnorm'
        ($smaller.Notes -join "`n") | Should -BeLike '*the texels were encoded again into Bc7RgbaUnorm on the GPU by block_compression*'
    }
}

Describe 'Channels and color' {
    BeforeAll {
        # Four texels: red, green and blue 10, 20 and 30 with alpha 40, and
        # each texel after it 40 more in every channel.
        $script:four = [byte[]](10, 20, 30, 40, 50, 60, 70, 80, 90, 100, 110, 120, 130, 140, 150, 160)
        $script:temp = [IO.Path]::GetTempPath()
        # The largest difference between two lists of numbers.
        function script:Get-Gap($a, $b) {
            ($(for ($i = 0; $i -lt $a.Count; $i++) { [math]::Abs([double]$a[$i] - [double]$b[$i]) }) | Measure-Object -Maximum).Maximum
        }
    }

    It 'swizzles the values a texture stores, an sRGB texture''s alpha included' -Skip:$noGpu {
        $texture = New-WgpuTexture 2 2 -Data $four
        (Read-WgpuTexture ($texture | Convert-WgpuTexture -Swizzle bgra)) -join ',' | Should -Be '30,20,10,40,70,60,50,80,110,100,90,120,150,140,130,160'
        (Read-WgpuTexture ($texture | Convert-WgpuTexture -Swizzle rrr1)) -join ',' | Should -Be '10,10,10,255,50,50,50,255,90,90,90,255,130,130,130,255'
        $srgb = New-WgpuTexture 2 2 -Format Rgba8UnormSrgb -Data $four
        (Read-WgpuTexture ($srgb | Convert-WgpuTexture -Swizzle aaa1)) -join ',' | Should -Be '40,40,40,255,80,80,80,255,120,120,120,255,160,160,160,255'
    }

    It 'inverts, brightens and gamma corrects the stored values as ImageMagick does' -Skip:$noGpu {
        $texture = New-WgpuTexture 2 2 -Data $four
        (Read-WgpuTexture ($texture | Convert-WgpuTexture -Invert)) -join ',' | Should -Be '245,235,225,40,205,195,185,80,165,155,145,120,125,115,105,160'
        # -brightness-contrast 20x0 adds 0.2, 51 steps of 255.
        (Read-WgpuTexture ($texture | Convert-WgpuTexture -Brightness 20))[0..3] -join ',' | Should -Be '61,71,81,40'
        # -gamma 2 takes each value to the power 1/2.
        $root = foreach ($i in 0..15) { if ($i % 4 -eq 3) { $four[$i] } else { [math]::Round([math]::Sqrt($four[$i] / 255) * 255) } }
        Get-Gap (Read-WgpuTexture ($texture | Convert-WgpuTexture -Gamma 2)) $root | Should -BeLessOrEqual 1
        # -brightness-contrast 0x40: slope 100 / 60 about the middle.
        $steeper = foreach ($i in 0..15) { if ($i % 4 -eq 3) { $four[$i] } else { [math]::Round([math]::Min(255, [math]::Max(0, (($four[$i] / 255 - 0.5) * 100 / 60 + 0.5) * 255))) } }
        Get-Gap (Read-WgpuTexture ($texture | Convert-WgpuTexture -Contrast 40)) $steeper | Should -BeLessOrEqual 1
    }

    It 'makes grayscale with Rec. 709 in linear light, Rec. 601 on stored values, or weights given' -Skip:$noGpu {
        $texture = New-WgpuTexture 2 2 -Data $four
        # Rec. 709 on Rgba8Unorm values, which are linear: 0.2126 * 10 + 0.7152 * 20 + 0.0722 * 30 = 18.6.
        (Read-WgpuTexture ($texture | Convert-WgpuTexture -Grayscale))[0..3] -join ',' | Should -Be '19,19,19,40'
        # Pillow's convert('L'): (10 * 19595 + 20 * 38470 + 30 * 7471 + 32768) >> 16 = 18.
        (Read-WgpuTexture ($texture | Convert-WgpuTexture -Grayscale -GrayscaleWeights Rec601))[0..3] -join ',' | Should -Be '18,18,18,40'
        (Read-WgpuTexture ($texture | Convert-WgpuTexture -Grayscale -GrayscaleWeights 0, 0, 1))[0..3] -join ',' | Should -Be '30,30,30,40'
    }

    It 'converts and relabels color spaces, and says when there was nothing to do' -Skip:$noGpu {
        $texture = New-WgpuTexture 2 2 -Data $four
        $records = $texture | Convert-WgpuTexture -ColorSpace Srgb -Verbose 4>&1
        $converted = $records | Where-Object { $_ -is [Wgpu.Texture] }
        $converted.Format | Should -Be 'Rgba8UnormSrgb'
        $records | Where-Object { $_ -is [System.Management.Automation.VerboseRecord] } | ForEach-Object Message | Should -Contain 'the texels were converted to sRGB, which makes the texture Rgba8UnormSrgb'
        # sRGB's encoding of 10 / 255.
        (Read-WgpuTexture $converted)[0] | Should -Be ([math]::Round((1.055 * [math]::Pow(10 / 255, 1 / 2.4) - 0.055) * 255))
        $relabeled = $texture | Convert-WgpuTexture -AssignColorSpace Srgb
        $relabeled.Format | Should -Be 'Rgba8UnormSrgb'
        (Read-WgpuTexture $relabeled) -join ',' | Should -Be ($four -join ',')
        ($converted | Convert-WgpuTexture -ColorSpace Srgb).Notes | Should -Contain 'the texture was already sRGB, so -ColorSpace changed nothing'
        { New-WgpuTexture 2 2 -Format Rgba16Float | Convert-WgpuTexture -AssignColorSpace Srgb } | Should -Throw '-AssignColorSpace relabels a format that has an sRGB form*'
    }

    It 'writes a -Format after the changes, encoding sRGB once, and loads an image whole for them' -Skip:$noGpu {
        # Four HDR texels, the brightest well past 1.
        $bright = New-WgpuTexture 2 2 -Format Rgba32Float -Data ([float[]](0.1, 0.2, 0.3, 1, 0.5, 1, 2, 1, 4, 3, 2, 1, 16, 8, 4, 1))
        $float = $bright | Convert-WgpuTexture -ToneMap AgX -ColorSpace Srgb
        $float.Format | Should -Be 'Rgba32Float'
        $encoded = (Read-WgpuTexture $float) | ForEach-Object { $_ * 255 }
        foreach ($format in 'Rgba8Unorm', 'Rgba8UnormSrgb') {
            $eight = $bright | Convert-WgpuTexture -ToneMap AgX -ColorSpace Srgb -Format $format
            $eight.Format | Should -Be 'Rgba8UnormSrgb' -Because $format
            Get-Gap (Read-WgpuTexture $eight) $encoded | Should -BeLessOrEqual 1 -Because $format
        }
        # -Format alone converts each level, keeping the values.
        $plain = New-WgpuTexture 2 2 -Format Rgba32Float -Data ([float[]](0, 0.25, 0.5, 1, 0.1, 0.2, 0.3, 0.4, 0.6, 0.7, 0.8, 0.9, 1, 1, 1, 1))
        $bytes = $plain | Convert-WgpuTexture -Format Rgba8Unorm
        $bytes.Format | Should -Be 'Rgba8Unorm'
        Get-Gap (Read-WgpuTexture $bytes) ((Read-WgpuTexture $plain) | ForEach-Object { $_ * 255 }) | Should -BeLessOrEqual 1
        # -AssignColorSpace says how the texels read and leaves -Format as it
        # is: 50, read as sRGB, is 8 in linear light.
        $read = New-WgpuTexture 2 2 -Data $four | Convert-WgpuTexture -AssignColorSpace Srgb -Format Rgba8Unorm
        $read.Format | Should -Be 'Rgba8Unorm'
        (Read-WgpuTexture $read)[4] | Should -Be ([math]::Round([math]::Pow((50 / 255 + 0.055) / 1.055, 2.4) * 255))
        # Import-WgpuImage loads the image whole for a transform, so the
        # tone map reads 16 rather than 1.
        $file = Join-Path $temp "wgpu-$PID-bright.exr"
        try {
            $bright | Export-WgpuImage -Path $file
            $loaded = Import-WgpuImage $file -ToneMap AgX -ColorSpace Srgb -Format Rgba8Unorm
            $loaded.Format | Should -Be 'Rgba8UnormSrgb'
            Get-Gap (Read-WgpuTexture $loaded) $encoded | Should -BeLessOrEqual 1
        } finally {
            Remove-Item $file -ErrorAction Ignore
        }
        { $plain | Convert-WgpuTexture -Invert -Format R32Uint } | Should -Throw '-Format names R32Uint, which holds integers, depth or stencil, and a transform writes floating-point colors'
    }

    It 'premultiplies and unpremultiplies alpha' -Skip:$noGpu {
        $texture = New-WgpuTexture 1 1 -Data ([byte[]](200, 100, 50, 128))
        # 200 * 128 / 255 = 100.4, 100 * 128 / 255 = 50.2 and 50 * 128 / 255 = 25.1.
        $premultiplied = $texture | Convert-WgpuTexture -Premultiply
        (Read-WgpuTexture $premultiplied) -join ',' | Should -Be '100,50,25,128'
        Get-Gap (Read-WgpuTexture ($premultiplied | Convert-WgpuTexture -Unpremultiply)) @(200, 100, 50, 128) | Should -BeLessOrEqual 1
    }

    It 'tone maps HDR colors with each curve after an exposure, Reinhard from the brightest texel' -Skip:$noGpu {
        $hdr = New-WgpuTexture 2 1 -Format Rgba16Float -Data ([float[]](4, 2, 1, 1, 0.5, 0.25, 0.125, 1))
        $mapped = foreach ($curve in 'Reinhard', 'AcesFilmic', 'AgX', 'Neutral') { (Read-WgpuTexture ($hdr | Convert-WgpuTexture -Exposure 1 -ToneMap $curve)) -join ',' }
        ($mapped | Select-Object -Unique).Count | Should -Be 4
        ($hdr | Convert-WgpuTexture -ToneMap Reinhard).Notes -join "`n" | Should -Match "Reinhard's white point was the brightest texel's luminance, "
    }

    It 'changes channels and colors through methods and their Async forms' -Skip:$noGpu {
        $texture = New-WgpuTexture 2 2 -Data $four
        (Read-WgpuTexture $texture.Swizzle('bgra'))[0..3] -join ',' | Should -Be '30,20,10,40'
        (Read-WgpuTexture $texture.SwizzleAsync('rgba').GetAwaiter().GetResult()) -join ',' | Should -Be ($four -join ',')
        (Read-WgpuTexture $texture.Invert())[0] | Should -Be 245
        (Read-WgpuTexture $texture.InvertAsync().GetAwaiter().GetResult())[0] | Should -Be 245
        (Read-WgpuTexture $texture.Grayscale($null))[0..3] -join ',' | Should -Be '19,19,19,40'
        (Read-WgpuTexture $texture.GrayscaleAsync('Rec601').GetAwaiter().GetResult())[0..3] -join ',' | Should -Be '18,18,18,40'
        (Read-WgpuTexture $texture.Adjust(20, $null, $null, $null))[0..3] -join ',' | Should -Be '61,71,81,40'
        # One stop of exposure doubles the linear values.
        (Read-WgpuTexture $texture.AdjustAsync($null, $null, $null, 1).GetAwaiter().GetResult())[0..3] -join ',' | Should -Be '20,40,60,40'
        $texture.ConvertColorSpace('Srgb').Format | Should -Be 'Rgba8UnormSrgb'
        $texture.ConvertColorSpaceAsync('Srgb').GetAwaiter().GetResult().Format | Should -Be 'Rgba8UnormSrgb'
        $texture.AssignColorSpace('Srgb').Format | Should -Be 'Rgba8UnormSrgb'
        $texture.AssignColorSpaceAsync('Linear').GetAwaiter().GetResult().Format | Should -Be 'Rgba8Unorm'
        (Read-WgpuTexture $texture.Premultiply())[0..3] -join ',' | Should -Be '2,3,5,40'
        (Read-WgpuTexture $texture.PremultiplyAsync().GetAwaiter().GetResult())[0..3] -join ',' | Should -Be '2,3,5,40'
        (Read-WgpuTexture $texture.Unpremultiply())[3] | Should -Be 40
        (Read-WgpuTexture $texture.UnpremultiplyAsync().GetAwaiter().GetResult())[3] | Should -Be 40
        $texture.ToneMap('Neutral').Format | Should -Be 'Rgba8Unorm'
        $texture.ToneMapAsync('AgX').GetAwaiter().GetResult().Format | Should -Be 'Rgba8Unorm'
        { $texture.Adjust($null, $null, $null, $null) } | Should -Throw '*Adjust takes a brightness, a contrast, a gamma or an exposure*'
    }

    It 'changes channels and colors on Export-WgpuImage and Import-WgpuImage' -Skip:$noGpu {
        $png = Join-Path $temp "wgpu-color-$PID.png"
        try {
            # Swizzled to 30, 20, 10, then inverted.
            New-WgpuTexture 2 2 -Data $four | Export-WgpuImage -Path $png -Invert -Swizzle bgra
            (Read-WgpuTexture (Import-WgpuImage $png))[0..3] -join ',' | Should -Be '225,235,245,40'
            $gray = [math]::Floor((225 * 19595 + 235 * 38470 + 245 * 7471 + 32768) / 65536)
            (Read-WgpuTexture (Import-WgpuImage $png -Grayscale -GrayscaleWeights Rec601))[0..3] -join ',' | Should -Be "$gray,$gray,$gray,40"
        } finally {
            Remove-Item $png -ErrorAction Ignore
        }
    }

    It 'joins channels of several textures and numbers, resizing to the largest' -Skip:$noGpu {
        $red = New-WgpuTexture 2 2 -Data ([byte[]](@(10, 0, 0, 255) * 4))
        $green = New-WgpuTexture 4 4 -Data ([byte[]](@(20, 0, 0, 77) * 16))
        $records = Join-WgpuTexture -Red $red -Green $green -Blue 0.5 -Alpha ($green, 'a') -Verbose 4>&1
        $joined = $records | Where-Object { $_ -is [Wgpu.Texture] }
        $joined.Width, $joined.Height -join 'x' | Should -Be '4x4'
        $joined.Format | Should -Be 'Rgba8Unorm'
        (Read-WgpuTexture $joined)[0..3] -join ',' | Should -Be '10,20,128,77'
        $records | Where-Object { $_ -is [System.Management.Automation.VerboseRecord] } | ForEach-Object Message | Should -Contain '2x2 Rgba8Unorm was resized from 2x2 to 4x4 on the GPU'
        (Read-WgpuTexture ([Wgpu.Texture]::Join($green, $null, $null, $null, $null)))[0..3] -join ',' | Should -Be '20,0,0,255'
        if ((Get-WgpuDevice).Features -contains 'TextureCompressionBc') {
            (Join-WgpuTexture $red $green -Format Bc7RgbaUnorm).Format | Should -Be 'Bc7RgbaUnorm'
        }
        { Join-WgpuTexture -Red 0.5 } | Should -Throw 'Join-WgpuTexture takes at least one texture*'
        { Join-WgpuTexture -Red ($red, 'x') } | Should -Throw "-Red's channel letter is r, g, b or a, not 'x'"
    }

    It 'refuses what a channel or color parameter cannot mean, and says why' -Skip:$noGpu {
        $texture = New-WgpuTexture 2 2
        { $texture | Convert-WgpuTexture -Swizzle rgb } | Should -Throw '-Swizzle takes four letters of r, g, b, a, 0 and 1*'
        { $texture | Convert-WgpuTexture -GrayscaleWeights Rec601 } | Should -Throw '-GrayscaleWeights picks the weights of -Grayscale: give -Grayscale'
        { $texture | Convert-WgpuTexture -Grayscale -GrayscaleWeights 1, 2 } | Should -Throw '*2 numbers were given'
        { $texture | Convert-WgpuTexture -Brightness 150 } | Should -Throw '-Brightness runs from -100 to 100*'
        { $texture | Convert-WgpuTexture -Gamma 0 } | Should -Throw '-Gamma is a number above 0, not 0'
        $texture | Convert-WgpuTexture -Contrast -101 -ErrorAction SilentlyContinue -ErrorVariable failed
        $failed[0].FullyQualifiedErrorId | Should -BeLike 'WgpuAdjust,*'
    }
}

Describe 'Alpha for cutouts' {
    BeforeAll {
        # Magenta, a near magenta, and one color with alpha either side of a
        # half.
        $script:cut = [byte[]](255, 0, 255, 200, 250, 5, 250, 200, 10, 20, 30, 127, 10, 20, 30, 128)
        # A row of six: red, four transparent texels, blue.
        $script:row = [byte[]](200, 10, 20, 255) + [byte[]]::new(16) + [byte[]](30, 40, 220, 255)
        $script:temp = [IO.Path]::GetTempPath()
    }

    It 'clears the alpha of a key color, exactly or within a tolerance, and keeps other alpha' -Skip:$noGpu {
        $texture = New-WgpuTexture 2 2 -Data $cut
        (Read-WgpuTexture ($texture | Convert-WgpuTexture -ColorKey '#FF00FF')) -join ',' | Should -Be '255,0,255,0,250,5,250,200,10,20,30,127,10,20,30,128'
        (Read-WgpuTexture ($texture | Convert-WgpuTexture -ColorKey 1, 0, 1 -ColorKeyTolerance 0.05)) -join ',' | Should -Be '255,0,255,0,250,5,250,0,10,20,30,127,10,20,30,128'
        ($texture | Convert-WgpuTexture -ColorKey '#ff00ff').Notes -join "`n" | Should -Match 'made transparent where red, green and blue were 1, 0 and 1'
    }

    It 'cuts alpha at a threshold, before premultiplying' -Skip:$noGpu {
        $texture = New-WgpuTexture 2 2 -Data $cut
        (Read-WgpuTexture ($texture | Convert-WgpuTexture -AlphaThreshold 0.5)) -join ',' | Should -Be '255,0,255,255,250,5,250,255,10,20,30,0,10,20,30,255'
        (Read-WgpuTexture ($texture | Convert-WgpuTexture -AlphaThreshold 0.5 -Premultiply))[8..15] -join ',' | Should -Be '0,0,0,0,10,20,30,255'
        (Read-WgpuTexture ($texture | Convert-WgpuTexture -ColorKey '#FF00FF' -Premultiply))[0..3] -join ',' | Should -Be '0,0,0,0'
    }

    It 'keys and cuts through methods and their Async forms' -Skip:$noGpu {
        $texture = New-WgpuTexture 2 2 -Data $cut
        (Read-WgpuTexture $texture.ColorKey('#FF00FF', $null))[3] | Should -Be 0
        (Read-WgpuTexture $texture.ColorKeyAsync(@(1, 0, 1), 0.05).GetAwaiter().GetResult())[7] | Should -Be 0
        (Read-WgpuTexture $texture.AlphaThreshold(0.5))[11] | Should -Be 0
        (Read-WgpuTexture $texture.AlphaThresholdAsync(0.5).GetAwaiter().GetResult())[15] | Should -Be 255
        { $texture.ColorKey($null, $null) } | Should -Throw '*ColorKey takes a color*'
    }

    It 'keys and cuts on Export-WgpuImage and Import-WgpuImage' -Skip:$noGpu {
        $png = Join-Path $temp "wgpu-cutout-$PID.png"
        try {
            New-WgpuTexture 2 2 -Data $cut | Export-WgpuImage -Path $png -ColorKey '#FF00FF'
            (Read-WgpuTexture (Import-WgpuImage $png))[3] | Should -Be 0
            (Read-WgpuTexture (Import-WgpuImage $png -AlphaThreshold 0.5))[8..15] -join ',' | Should -Be '10,20,30,0,10,20,30,255'
        } finally {
            Remove-Item $png -ErrorAction Ignore
        }
    }

    It 'refuses a key or a threshold that cannot mean anything, and says why' -Skip:$noGpu {
        $texture = New-WgpuTexture 2 2 -Data $cut
        { $texture | Convert-WgpuTexture -AlphaThreshold 0 } | Should -Throw '-AlphaThreshold is above 0 and at most 1*'
        { $texture | Convert-WgpuTexture -ColorKey 'FF00FF' } | Should -Throw "-ColorKey is red, green and blue from 0 to 1, or '#RRGGBB'*"
        { $texture | Convert-WgpuTexture -ColorKey 1, 0 } | Should -Throw '*2 numbers were given*'
        { $texture | Convert-WgpuTexture -ColorKeyTolerance 0.1 } | Should -Throw '-ColorKeyTolerance widens -ColorKey: give -ColorKey'
        { New-WgpuTexture 2 2 -Format R8Unorm | Convert-WgpuTexture -AlphaThreshold 0.5 } | Should -Throw 'R8Unorm has no alpha channel, so -AlphaThreshold would change nothing'
    }

    It 'colors each transparent texel from the nearest texel with alpha, within a reach' -Skip:$noGpu {
        $texture = New-WgpuTexture 6 1 -Data $row
        $fixed = $texture | Convert-WgpuTexture -FixAlphaBorder
        (Read-WgpuTexture $fixed) -join ',' | Should -Be '200,10,20,255,200,10,20,0,200,10,20,0,30,40,220,0,30,40,220,0,30,40,220,255'
        $fixed.Notes -join "`n" | Should -Match 'every transparent texel took the color of the nearest texel with alpha above 0, found on the GPU'
        (Read-WgpuTexture ($texture | Convert-WgpuTexture -FixAlphaBorder -BorderReach 1)) -join ',' | Should -Be '200,10,20,255,200,10,20,0,0,0,0,0,0,0,0,0,30,40,220,0,30,40,220,255'
    }

    It 'fixes the border through methods, their Async forms and the image cmdlets' -Skip:$noGpu {
        $texture = New-WgpuTexture 6 1 -Data $row
        (Read-WgpuTexture $texture.FixAlphaBorder($null))[8..11] -join ',' | Should -Be '200,10,20,0'
        (Read-WgpuTexture $texture.FixAlphaBorderAsync(1).GetAwaiter().GetResult())[8..11] -join ',' | Should -Be '0,0,0,0'
        $png = Join-Path $temp "wgpu-border-$PID.png"
        try {
            $texture | Export-WgpuImage -Path $png -FixAlphaBorder
            (Read-WgpuTexture (Import-WgpuImage $png))[12..15] -join ',' | Should -Be '30,40,220,0'
            $texture | Export-WgpuImage -Path $png
            (Read-WgpuTexture (Import-WgpuImage $png -FixAlphaBorder -BorderReach 1))[4..7] -join ',' | Should -Be '200,10,20,0'
        } finally {
            Remove-Item $png -ErrorAction Ignore
        }
    }

    It 'keeps alpha coverage across mip levels, through methods and cmdlets' -Skip:$noGpu {
        # Thin strands of full alpha, which averaging thins below a half,
        # level by level.
        $bytes = [byte[]]::new(32 * 32 * 4)
        for ($y = 0; $y -lt 32; $y++) {
            for ($x = 0; $x -lt 32; $x++) {
                $at = ($y * 32 + $x) * 4
                $bytes[$at] = 200
                $bytes[$at + 1] = 100
                $bytes[$at + 2] = 50
                if (($x * 7 + $y * 3) % 11 -lt 2) { $bytes[$at + 3] = 255 }
            }
        }
        $texture = New-WgpuTexture 32 32 -Data $bytes
        # The share of a level's texels with alpha at or above a half.
        $share = {
            param($t, $level)
            $texels = Read-WgpuTexture $t -MipLevel $level
            $passing = 0
            for ($i = 3; $i -lt $texels.Count; $i += 4) { if ($texels[$i] -ge 128) { $passing++ } }
            $passing / ($texels.Count / 4)
        }
        $plain = $texture.GenerateMips(0)
        $kept = $texture.GenerateMips(0, $null, $null, 0.5)
        $level0 = & $share $kept 0
        foreach ($level in 1..3) {
            [math]::Abs((& $share $kept $level) - $level0) | Should -BeLessOrEqual ([math]::Abs((& $share $plain $level) - $level0))
        }
        [math]::Abs((& $share $kept 2) - $level0) | Should -BeLessThan ([math]::Abs((& $share $plain 2) - $level0))
        $kept.Notes -join "`n" | Should -Match "its alpha scaled to keep level 0's share of texels with alpha at or above 0.5"
        $async = $texture.GenerateMipsAsync(0, $null, $null, 0.5).GetAwaiter().GetResult()
        (Read-WgpuTexture $async -MipLevel 2) -join ',' | Should -Be ((Read-WgpuTexture $kept -MipLevel 2) -join ',')
        $resized = $kept | Resize-WgpuTexture -Width 16 -KeepCoverage 0.5
        $resized.Notes -join "`n" | Should -Match 'its alpha scaled to keep'
        $png = Join-Path $temp "wgpu-coverage-$PID.png"
        try {
            $texture | Export-WgpuImage -Path $png
            $imported = Import-WgpuImage $png -MipLevels 0 -KeepCoverage 0.5
            (Read-WgpuTexture $imported -MipLevel 2) -join ',' | Should -Be ((Read-WgpuTexture $kept -MipLevel 2) -join ',')
        } finally {
            Remove-Item $png -ErrorAction Ignore
        }
    }

    It 'refuses a coverage to keep that cannot mean anything, and says why' -Skip:$noGpu {
        $texture = New-WgpuTexture 4 4
        { $texture.GenerateMips(0, $null, $null, 0) } | Should -Throw '*-KeepCoverage is the alpha an alpha test passes at, above 0 and at most 1, not 0*'
        { $texture.GenerateMips(1, $null, $null, 0.5) } | Should -Throw '*GenerateMips(1) keeps level 0 alone*'
        { $texture | Convert-WgpuTexture -Invert -KeepCoverage 0.5 } | Should -Throw "*and this one has level 0 alone"
        { (New-WgpuTexture 4 4 -Format R8Unorm).GenerateMips(0, $null, $null, 0.5) } | Should -Throw '*R8Unorm has no alpha channel, so -KeepCoverage would change nothing*'
    }

    It 'refuses a border fix that cannot mean anything, and says why' -Skip:$noGpu {
        $texture = New-WgpuTexture 6 1 -Data $row
        { $texture | Convert-WgpuTexture -BorderReach 2 } | Should -Throw '-BorderReach limits -FixAlphaBorder: give -FixAlphaBorder'
        { $texture | Convert-WgpuTexture -FixAlphaBorder -BorderReach 0 } | Should -Throw '-BorderReach is at least 1 texel*'
        { $texture | Convert-WgpuTexture -FixAlphaBorder -Premultiply } | Should -Throw '*give -FixAlphaBorder or -Premultiply, not both'
        { New-WgpuTexture 2 2 -Format R8Unorm | Convert-WgpuTexture -FixAlphaBorder } | Should -Throw 'R8Unorm has no alpha channel, so -FixAlphaBorder would change nothing'
    }
}

Describe 'Filters' {
    BeforeAll {
        # A row of five grays, 0.2 but for 0.8 in the middle. A Gaussian of
        # standard deviation 1 weighs a texel 0.3989, its neighbors 0.2420
        # and those two away 0.0540, which give the blur, the unsharp mask
        # of amount 0.25 and Sobel's gradient below.
        $script:grays = [byte[]](51, 51, 51, 255, 51, 51, 51, 255, 204, 204, 204, 255, 51, 51, 51, 255, 51, 51, 51, 255)
        $script:blurred = '59,59,59,255,88,88,88,255,112,112,112,255,88,88,88,255,59,59,59,255'
        $script:sharpened = '49,49,49,255,42,42,42,255,227,227,227,255,42,42,42,255,49,49,49,255'
        $script:sloped = '0,0,0,255,153,153,153,255,0,0,0,255,153,153,153,255,0,0,0,255'
        # A white square of 6 by 6 texels on black, 16 by 16 in all.
        $script:square = [byte[]]::new(16 * 16 * 4)
        for ($y = 0; $y -lt 16; $y++) {
            for ($x = 0; $x -lt 16; $x++) {
                $at = ($y * 16 + $x) * 4
                $v = if ($x -ge 5 -and $x -le 10 -and $y -ge 5 -and $y -le 10) { 255 } else { 0 }
                $square[$at] = $v
                $square[$at + 1] = $v
                $square[$at + 2] = $v
                $square[$at + 3] = 255
            }
        }
        $script:temp = [IO.Path]::GetTempPath()
    }

    It 'blurs, sharpens and finds gradients with Convert-WgpuTexture, and says so in the notes' -Skip:$noGpu {
        $texture = New-WgpuTexture 5 1 -Data $grays
        $blur = $texture | Convert-WgpuTexture -Blur 1
        (Read-WgpuTexture $blur) -join ',' | Should -Be $blurred
        $blur.Notes -join "`n" | Should -Match 'blurred on the GPU by a Gaussian of a standard deviation of 1 texel, in linear light weighed by alpha, the edge texels repeated past the edges'
        $sharp = $texture | Convert-WgpuTexture -Sharpen 0.25
        (Read-WgpuTexture $sharp) -join ',' | Should -Be $sharpened
        $sharp.Notes -join "`n" | Should -Match 'sharpened on the GPU by an unsharp mask, 0.25 times the difference from a Gaussian of a standard deviation of 1 texel added where it is at least 0'
        $slope = $texture | Convert-WgpuTexture -Edges Sobel
        (Read-WgpuTexture $slope) -join ',' | Should -Be $sloped
        $slope.Notes -join "`n" | Should -Match 'the edges were found on the GPU as the size of the Sobel gradient of the stored values'
    }

    It "finds Canny's edges around a square, one texel or so wide" -Skip:$noGpu {
        $edges = New-WgpuTexture 16 16 -Data $square | Convert-WgpuTexture -Edges Canny
        $texels = Read-WgpuTexture $edges
        $on = 0
        for ($y = 0; $y -lt 16; $y++) {
            for ($x = 0; $x -lt 16; $x++) {
                $at = ($y * 16 + $x) * 4
                $texels[$at] | Should -BeIn @(0, 255)
                $texels[$at + 3] | Should -Be 255
                if ($texels[$at] -eq 255) {
                    $on++
                    $x | Should -BeIn (3..12)
                    $y | Should -BeIn (3..12)
                }
            }
        }
        $on | Should -BeGreaterOrEqual 20
        $edges.Notes -join "`n" | Should -Match "Canny's edges were found on the GPU in the luminance smoothed by a Gaussian of a standard deviation of 1 texel, kept above 0.1 where they link to one above 0.2"
        # A gradient's size is at most the square root of 2 here, so these
        # thresholds keep no texel.
        (Read-WgpuTexture (New-WgpuTexture 16 16 -Data $square | Convert-WgpuTexture -Edges Canny -EdgeThreshold 1.5, 2)) | Where-Object { $_ -eq 255 } | Measure-Object | Select-Object -ExpandProperty Count | Should -Be 256
    }

    It 'blurs, sharpens and finds edges through methods and their Async forms' -Skip:$noGpu {
        $texture = New-WgpuTexture 5 1 -Data $grays
        (Read-WgpuTexture $texture.Blur(1)) -join ',' | Should -Be $blurred
        (Read-WgpuTexture $texture.BlurAsync(@(1, 1), 'Wrap').GetAwaiter().GetResult()) -join ',' | Should -Be ((Read-WgpuTexture ($texture | Convert-WgpuTexture -Blur 1 -FilterEdge Wrap)) -join ',')
        (Read-WgpuTexture $texture.Sharpen(0.25)) -join ',' | Should -Be $sharpened
        (Read-WgpuTexture $texture.SharpenAsync(0.25, 1, 0, 'Clamp').GetAwaiter().GetResult()) -join ',' | Should -Be $sharpened
        (Read-WgpuTexture $texture.FindEdges('Sobel')) -join ',' | Should -Be $sloped
        $box = New-WgpuTexture 16 16 -Data $square
        (Read-WgpuTexture $box.FindEdgesAsync('Canny', @(0.1, 0.2), 1, 'Mirror').GetAwaiter().GetResult()) -join ',' | Should -Be ((Read-WgpuTexture ($box | Convert-WgpuTexture -Edges Canny -FilterEdge Mirror)) -join ',')
        { $texture.Blur(@(), $null) } | Should -Throw '*-Blur takes one standard deviation, or two, across and down, and was given 0*'
    }

    It 'blurs and sharpens on Export-WgpuImage and Import-WgpuImage' -Skip:$noGpu {
        $png = Join-Path $temp "wgpu-filters-$PID.png"
        try {
            New-WgpuTexture 5 1 -Data $grays | Export-WgpuImage -Path $png -Blur 1
            (Read-WgpuTexture (Import-WgpuImage $png)) -join ',' | Should -Be $blurred
            New-WgpuTexture 5 1 -Data $grays | Export-WgpuImage -Path $png
            (Read-WgpuTexture (Import-WgpuImage $png -Sharpen 0.25 -SharpenRadius 1 -SharpenThreshold 0 -FilterEdge Clamp)) -join ',' | Should -Be $sharpened
        } finally {
            Remove-Item $png -ErrorAction Ignore
        }
    }

    It 'refuses filter settings that cannot mean anything, and says why' -Skip:$noGpu {
        $texture = New-WgpuTexture 5 1 -Data $grays
        { $texture | Convert-WgpuTexture -Blur 0 } | Should -Throw '-Blur takes standard deviations above 0, and 0 is not'
        { $texture | Convert-WgpuTexture -Blur 1, 2, 3 } | Should -Throw '-Blur takes one standard deviation, or two, across and down, and was given 3'
        { $texture | Convert-WgpuTexture -Sharpen 0 } | Should -Throw '-Sharpen is how much of the detail to add, above 0, and 0 is not'
        { $texture | Convert-WgpuTexture -SharpenRadius 2 } | Should -Throw '-SharpenRadius and -SharpenThreshold say how -Sharpen sharpens: give -Sharpen'
        { $texture | Convert-WgpuTexture -Sharpen 1 -SharpenThreshold 2 } | Should -Throw '-SharpenThreshold is a difference from 0 to 1, and 2 is not'
        { $texture | Convert-WgpuTexture -Edges Sobel -EdgeSigma 2 } | Should -Throw '-EdgeThreshold and -EdgeSigma say how Canny finds edges: give -Edges Canny'
        { $texture | Convert-WgpuTexture -Edges Canny -EdgeThreshold 0.3, 0.2 } | Should -Throw "-EdgeThreshold is Canny's low threshold, 0 or above, and its high one, no lower, and 0.3 and 0.2 are not"
        { $texture | Convert-WgpuTexture -Edges Canny -EdgeThreshold 0.1 } | Should -Throw "-EdgeThreshold takes two numbers, Canny's low and high thresholds, and was given 1"
        { $texture | Convert-WgpuTexture -FilterEdge Wrap } | Should -Throw '-FilterEdge says what a filter reads past the edge of the texture, and no filter was given'
        { $texture | Export-WgpuImage -Path (Join-Path $temp "wgpu-refused-$PID.png") -SharpenThreshold 0.1 } | Should -Throw '-SharpenRadius and -SharpenThreshold say how -Sharpen sharpens: give -Sharpen'
    }
}

Describe 'Compare and measure' {
    BeforeAll {
        # A 32x32 gradient with a ripple in blue, which BC7 keeps close but
        # not exact, and opaque alpha.
        $script:gradient = [byte[]]::new(32 * 32 * 4)
        for ($y = 0; $y -lt 32; $y++) {
            for ($x = 0; $x -lt 32; $x++) {
                $at = ($y * 32 + $x) * 4
                $gradient[$at] = $x * 8
                $gradient[$at + 1] = $y * 8
                $gradient[$at + 2] = (($x * $y) % 61) * 4
                $gradient[$at + 3] = 255
            }
        }
    }

    It 'compares a BC7 copy with its original, a row for each level, layer and channel' -Skip:$noGpu {
        $original = New-WgpuTexture 32 32 -Data $gradient
        $bc7 = $original.Compress('Bc7RgbaUnorm', $null, $null)
        $rows = @($bc7 | Compare-WgpuTexture $original)
        $rows.Count | Should -Be 4
        $rows.Channel -join ',' | Should -Be 'Red,Green,Blue,Alpha'
        foreach ($row in $rows) {
            "$($row.Level),$($row.Layer)" | Should -Be '0,0'
            $row.PSNR | Should -BeGreaterThan 30
            $row.SSIM | Should -BeGreaterThan 0.9
        }
        $rows[3].MSE | Should -Be 0
        $rows[3].PSNR | Should -Be ([double]::PositiveInfinity)
    }

    It 'finds a texture identical to itself, and gives a level under 11 texels on a side no SSIM' -Skip:$noGpu {
        $texture = New-WgpuTexture 16 16 -Data ([byte[]]$gradient[0..1023])
        foreach ($row in Compare-WgpuTexture $texture $texture) {
            $row.MSE | Should -Be 0
            $row.PSNR | Should -Be ([double]::PositiveInfinity)
            [math]::Abs($row.SSIM - 1) | Should -BeLessThan 1e-6
        }
        $small = New-WgpuTexture 8 8
        (Compare-WgpuTexture $small $small)[0].SSIM | Should -BeNullOrEmpty
    }

    It 'draws the error on a log scale, red, green and blue in their own colors or one channel as gray' -Skip:$noGpu {
        $reference = New-WgpuTexture 2 1 -Data ([byte[]]::new(8))
        $texture = New-WgpuTexture 2 1 -Data ([byte[]](255, 1, 0, 255, 0, 0, 0, 0))
        $picture = $texture | Compare-WgpuTexture $reference -Difference
        $picture.Format | Should -Be 'Rgba8Unorm'
        (Read-WgpuTexture $picture) -join ',' | Should -Be '255,85,0,255,0,0,0,255'
        $picture.Notes -join "`n" | Should -Match 'on a log scale, black at 2\^-12 or less and full at 1 or more'
        (Read-WgpuTexture ($texture | Compare-WgpuTexture $reference -Difference -Channel Green)) -join ',' | Should -Be '85,85,85,255,0,0,0,255'
        (Read-WgpuTexture ($texture | Compare-WgpuTexture $reference -Difference -Channel Alpha)) -join ',' | Should -Be '255,255,255,255,0,0,0,255'
    }

    It 'measures each channel: minimum, maximum, mean, standard deviation and a histogram' -Skip:$noGpu {
        $texture = New-WgpuTexture 2 1 -Data ([byte[]](0, 255, 51, 255, 255, 255, 102, 255))
        $rows = @($texture | Measure-WgpuTexture -Bins 4)
        $rows.Count | Should -Be 4
        "$($rows[0].Minimum),$($rows[0].Maximum),$($rows[0].Average),$($rows[0].StandardDeviation)" | Should -Be '0,1,0.5,0.5'
        $rows[0].Histogram -join ',' | Should -Be '1,0,0,1'
        $rows[1].StandardDeviation | Should -Be 0
        [math]::Abs($rows[2].Average - 0.3) | Should -BeLessThan 1e-12
        $rows[2].Histogram -join ',' | Should -Be '1,1,0,0'
        (Measure-WgpuTexture $texture)[0].Histogram.Count | Should -Be 256
        # Values below the range count in the first bin and values above it in the last.
        ($texture | Measure-WgpuTexture -Range 0.25, 0.75 -Bins 2)[0].Histogram -join ',' | Should -Be '1,1'
    }

    It 'compares and measures through methods and their Async forms' -Skip:$noGpu {
        $reference = New-WgpuTexture 16 16 -Data ([byte[]]$gradient[0..1023])
        $texture = New-WgpuTexture 16 16 -Data ([byte[]]$gradient[1024..2047])
        $rows = $texture.Compare($reference)
        $rows.Count | Should -Be 4
        $texture.CompareAsync($reference).GetAwaiter().GetResult().MSE -join ',' | Should -Be ($rows.MSE -join ',')
        $piped = Read-WgpuTexture ($texture | Compare-WgpuTexture $reference -Difference)
        (Read-WgpuTexture $texture.Difference($reference, $null)) -join ',' | Should -Be ($piped -join ',')
        $blue = Read-WgpuTexture ($texture | Compare-WgpuTexture $reference -Difference -Channel Blue)
        (Read-WgpuTexture $texture.DifferenceAsync($reference, 'Blue').GetAwaiter().GetResult()) -join ',' | Should -Be ($blue -join ',')
        $texture.Measure(8, @(0, 1))[0].Histogram.Count | Should -Be 8
        $texture.MeasureAsync($null, $null).GetAwaiter().GetResult()[0].Histogram.Count | Should -Be 256
        $texture.Measure($null, $null).Average -join ',' | Should -Be ((Measure-WgpuTexture $texture).Average -join ',')
    }

    It 'reads a multisampled texture as its samples averaged, and says so' -Skip:$noGpu {
        $texture = New-WgpuTexture 16 16 -SampleCount 4 -Usage RenderAttachment, TextureBinding
        $records = Measure-WgpuTexture $texture -Verbose 4>&1
        @($records | Where-Object { $_ -is [System.Management.Automation.VerboseRecord] }).Message | Should -Be 'the 4 samples of each texel of the texture were averaged in a shader and rounded to Rgba8Unorm'
        $rows = @($records | Where-Object { $_ -isnot [System.Management.Automation.VerboseRecord] })
        $rows.Count | Should -Be 4
        $rows.Maximum -join ',' | Should -Be '0,0,0,0'
    }

    It 'reads an integer texture through the norm format with its bits' -Skip:$noGpu {
        $bytes = [byte[]]$gradient[0..1023]
        $unorm = New-WgpuTexture 16 16 -Data $bytes
        $uint = New-WgpuTexture 16 16 -Format Rgba8Uint -Data $bytes
        (Compare-WgpuTexture $unorm $uint).MSE -join ',' | Should -Be '0,0,0,0'
    }

    It 'refuses what it cannot compare or measure, and says why' -Skip:$noGpu {
        { Compare-WgpuTexture (New-WgpuTexture 4 4) (New-WgpuTexture 4 2) } | Should -Throw 'the reference is 4x4 and the texture 4x2: textures compare only at the same width and height'
        { Compare-WgpuTexture (New-WgpuTexture 4 4) (New-WgpuTexture 4 4 -Depth 2) } | Should -Throw 'the reference has 1 layer and the texture 2 layers*'
        { Compare-WgpuTexture (New-WgpuTexture 4 4 -Format R8Unorm) (New-WgpuTexture 4 4) -Difference -Channel Blue } | Should -Throw 'the textures do not both store blue: they share red'
        { Measure-WgpuTexture (New-WgpuTexture 4 4 -Usage TextureBinding) } | Should -Throw 'the texture needs CopySrc among its usages to be read back from the GPU'
        { Measure-WgpuTexture (New-WgpuTexture 4 4) -Bins 0 } | Should -Throw '-Bins is at least 1'
        { Measure-WgpuTexture (New-WgpuTexture 4 4) -Range 1, 0 } | Should -Throw '-Range takes two numbers*'
    }
}

Describe 'Normal maps' {
    BeforeAll {
        # A 4x4 height map rising 64 a texel to the right: at strength 4 each
        # normal is (-0.7085, 0, 0.7057), stored as 37, 128 and 217.
        $script:ramp = [byte[]](@(0, 64, 128, 192) * 4)
        # Two normals stored as n / 2 + 1 / 2 with a Z of -1.
        $script:normals = [byte[]](230, 200, 0, 255, 128, 64, 0, 255)
        $script:bumps = [byte[]]::new(64)
        for ($i = 0; $i -lt 64; $i++) { $bumps[$i] = ($i * 37 + ($i -shr 3) * 91) % 256 }
    }

    It 'makes a normal map from a height map, green up and alpha 1, and says how' -Skip:$noGpu {
        $height = New-WgpuTexture 4 4 -Format R8Unorm -Data $ramp
        $records = $height | ConvertTo-WgpuNormalMap -Strength 4 -Verbose 4>&1
        $made = @($records | Where-Object { $_ -isnot [System.Management.Automation.VerboseRecord] })[0]
        $made.Format | Should -Be 'Rgba8Unorm'
        (Read-WgpuTexture $made)[0..7] -join ',' | Should -Be '37,128,217,255,37,128,217,255'
        $verbose = @($records | Where-Object { $_ -is [System.Management.Automation.VerboseRecord] }).Message -join "`n"
        $verbose | Should -Match 'made into normals on the GPU by the Prewitt kernel at strength 4'
        $made.Notes -join "`n" | Should -Match 'green points up, as glTF reads it, and alpha holds 1'
    }

    It 'flips green, reads another channel, and puts the height or occlusion in alpha' -Skip:$noGpu {
        $data = [byte[]]::new(64)
        for ($y = 0; $y -lt 4; $y++) { for ($x = 0; $x -lt 4; $x++) { $data[($y * 4 + $x) * 4 + 3] = $y * 64 } }
        $height = New-WgpuTexture 4 4 -Data $data
        (Read-WgpuTexture ($height | ConvertTo-WgpuNormalMap -HeightChannel Alpha -Strength 4))[0..3] -join ',' | Should -Be '128,218,217,255'
        (Read-WgpuTexture ($height | ConvertTo-WgpuNormalMap -HeightChannel Alpha -Strength 4 -FlipGreen))[0..3] -join ',' | Should -Be '128,37,217,255'
        (Read-WgpuTexture ($height | ConvertTo-WgpuNormalMap -HeightChannel Alpha -Alpha Height))[19] | Should -Be 64
        (Read-WgpuTexture ($height | ConvertTo-WgpuNormalMap -HeightChannel Alpha -Strength 4 -Alpha Occlusion -Kernel Sobel -Edge Continue))[3] | Should -Be 165
    }

    It 'writes the format asked for, and X and Y alone with -TwoChannel' -Skip:$noGpu {
        $height = New-WgpuTexture 4 4 -Format R8Unorm -Data $ramp
        $rg = $height | ConvertTo-WgpuNormalMap -Strength 4 -Format Rg8Unorm -TwoChannel
        $rg.Format | Should -Be 'Rg8Unorm'
        (Read-WgpuTexture $rg)[0..1] -join ',' | Should -Be '37,128'
        $float = $height | ConvertTo-WgpuNormalMap -Strength 4 -Format Rgba32Float
        [math]::Abs((Read-WgpuTexture $float)[0] + 0.708489) | Should -BeLessThan 1e-5
    }

    It 'keeps the normals of each mip level unit length, as -NormalMips says' -Skip:$noGpu {
        $height = New-WgpuTexture 8 8 -Format R8Unorm -MipLevels 4 -Data $bumps
        $made = $height | ConvertTo-WgpuNormalMap -Strength 2
        $made.MipLevelCount | Should -Be 4
        $made.Notes -join "`n" | Should -Match "each texel's normal scaled back to length 1"
        for ($level = 1; $level -lt 4; $level++) {
            $texels = Read-WgpuTexture $made -MipLevel $level
            for ($at = 0; $at -lt $texels.Count; $at += 4) {
                $n = 0..2 | ForEach-Object { $texels[$at + $_] / 255 * 2 - 1 }
                [math]::Abs([math]::Sqrt($n[0] * $n[0] + $n[1] * $n[1] + $n[2] * $n[2]) - 1) | Should -BeLessThan 0.02
            }
        }
        ($height | ConvertTo-WgpuNormalMap -NormalMips SlopeSpace).Notes -join "`n" | Should -Match "the normals' slopes filtered and made unit normals again"
    }

    It 'changes the normals a texture holds with Convert-WgpuTexture' -Skip:$noGpu {
        $texture = New-WgpuTexture 2 1 -Data $normals
        (Read-WgpuTexture ($texture | Convert-WgpuTexture -ReconstructZ)) -join ',' | Should -Be '230,200,150,255,128,64,238,255'
        (Read-WgpuTexture ($texture | Convert-WgpuTexture -FlipGreen)) -join ',' | Should -Be '230,55,0,255,128,191,0,255'
        (Read-WgpuTexture ($texture | Convert-WgpuTexture -Normalize)) -join ',' | Should -Be '201,179,37,255,128,71,13,255'
        $mipped = New-WgpuTexture 8 8 -MipLevels 2 -Data ([byte[]]($normals * 32)) | Convert-WgpuTexture -FlipGreen -NormalMips Renormalize
        $mipped.Notes -join "`n" | Should -Match "each texel's normal scaled back to length 1"
        $resized = $mipped | Resize-WgpuTexture -Width 4 -NormalMips SlopeSpace
        $resized.Notes -join "`n" | Should -Match "slopes filtered"
    }

    It 'makes heights into normals and changes normals on Import-WgpuImage and Export-WgpuImage' -Skip:$noGpu {
        $height = New-WgpuTexture 4 4 -Format R8Unorm -Data $ramp
        $png = Join-Path $TestDrive 'height.png'
        Export-WgpuImage $height $png
        (Read-WgpuTexture (Import-WgpuImage $png -HeightToNormal 4))[0..3] -join ',' | Should -Be '37,128,217,255'
        $dds = Join-Path $TestDrive 'normal.dds'
        Export-WgpuImage $height $dds -HeightToNormal 4 -Format Rgba8Unorm
        (Read-WgpuTexture (Import-WgpuImage $dds -FlipGreen))[0..3] -join ',' | Should -Be '37,127,217,255'
        $ktx2 = Join-Path $TestDrive 'normal.ktx2'
        Export-WgpuImage $height $ktx2 -HeightToNormal 4 -Format BasisUastc4x4 -TwoChannel -MipLevels 0
        $texel = (Read-WgpuTexture (Import-WgpuImage $ktx2 -Format Rgba8Unorm))[0..3]
        foreach ($channel in 0..2) { [math]::Abs($texel[$channel] - 37) | Should -BeLessOrEqual 4 }
        [math]::Abs($texel[3] - 128) | Should -BeLessOrEqual 4
    }

    It 'makes and changes normal maps through methods and their Async forms' -Skip:$noGpu {
        $height = New-WgpuTexture 4 4 -Format R8Unorm -Data $ramp
        $piped = (Read-WgpuTexture ($height | ConvertTo-WgpuNormalMap -Strength 4)) -join ','
        (Read-WgpuTexture $height.ToNormalMap(4, $null, $null, $null, $null)) -join ',' | Should -Be $piped
        (Read-WgpuTexture $height.ToNormalMapAsync(4, $null, $null, $null, $null).GetAwaiter().GetResult()) -join ',' | Should -Be $piped
        $texture = New-WgpuTexture 2 1 -Data $normals
        (Read-WgpuTexture $texture.FlipGreen()) -join ',' | Should -Be '230,55,0,255,128,191,0,255'
        (Read-WgpuTexture $texture.FlipGreenAsync().GetAwaiter().GetResult()) -join ',' | Should -Be '230,55,0,255,128,191,0,255'
        (Read-WgpuTexture $texture.ReconstructZ()) -join ',' | Should -Be '230,200,150,255,128,64,238,255'
        (Read-WgpuTexture $texture.ReconstructZAsync().GetAwaiter().GetResult()) -join ',' | Should -Be '230,200,150,255,128,64,238,255'
        (Read-WgpuTexture $texture.Normalize()) -join ',' | Should -Be '201,179,37,255,128,71,13,255'
        (Read-WgpuTexture $texture.NormalizeAsync().GetAwaiter().GetResult()) -join ',' | Should -Be '201,179,37,255,128,71,13,255'
        $mipped = (New-WgpuTexture 8 8 -Data ([byte[]]($normals * 32))).GenerateMips(0, $null, $null, $null, 'Renormalize')
        $mipped.MipLevelCount | Should -Be 4
        $mipped.Notes -join "`n" | Should -Match "each texel's normal scaled back to length 1"
        $mipped = (New-WgpuTexture 8 8 -Data ([byte[]]($normals * 32))).GenerateMipsAsync(0, $null, $null, $null, 'SlopeSpace').GetAwaiter().GetResult()
        $mipped.Notes -join "`n" | Should -Match 'slopes filtered'
    }

    It 'says when a height map looks like it tiles' -Skip:$noGpu {
        $tiles = [byte[]]::new(64)
        for ($i = 0; $i -lt 64; $i++) { $tiles[$i] = 128 + [int](100 * [math]::Sin([math]::PI * ($i % 8) / 4) * [math]::Cos([math]::PI * ($i -shr 3) / 4)) }
        $height = New-WgpuTexture 8 8 -Format R8Unorm -Data $tiles
        $records = $height | ConvertTo-WgpuNormalMap -Verbose 4>&1
        @($records | Where-Object { $_ -is [System.Management.Automation.VerboseRecord] }).Message -join "`n" | Should -Match 'looks like it tiles'
        $wrapped = $height | ConvertTo-WgpuNormalMap -HeightEdge Wrap
        $wrapped.Notes -join "`n" | Should -Not -Match 'looks like it tiles'
    }

    It 'refuses what makes no normal map, and says why' -Skip:$noGpu {
        $height = New-WgpuTexture 4 4 -Format R8Unorm -Data $ramp
        { ConvertTo-WgpuNormalMap $height -Format Rgba8UnormSrgb } | Should -Throw 'a normal map stores directions, which Rgba8UnormSrgb would encode as sRGB colors*'
        { ConvertTo-WgpuNormalMap $height -TwoChannel } | Should -Throw '-TwoChannel keeps X and Y apart for a format with two channels*'
        { ConvertTo-WgpuNormalMap $height -MipFilter Min } | Should -Throw "Min takes each channel's smallest value on its own*"
        { ConvertTo-WgpuNormalMap $height -Strength ([double]::NaN) } | Should -Throw '-Strength is a finite number*'
        { ConvertTo-WgpuNormalMap (New-WgpuTexture 4 4 -Depth 6 -Cube) } | Should -Throw "*a cube map's faces meet at their edges*"
        { Convert-WgpuTexture (New-WgpuTexture 4 4 -Format Rg8Unorm) -ReconstructZ } | Should -Throw 'Rg8Unorm has no blue channel to hold the Z it rebuilds*'
        $png = Join-Path $TestDrive 'refused.png'
        Export-WgpuImage $height $png
        { Import-WgpuImage $png -HeightChannel Alpha } | Should -Throw '-HeightChannel, -NormalKernel, -HeightEdge and -NormalAlpha say how -HeightToNormal makes normals from heights: give -HeightToNormal'
        { Import-WgpuImage $png -HeightToNormal 1 -Normalize } | Should -Throw '-ReconstructZ and -Normalize change normals a texture holds*'
    }

    It 'writes normals again for a -Format that stores them the other way, with -NormalMips' -Skip:$noGpu {
        # (0.48, 0.6, 0.64) and a flat normal, stored as n / 2 + 1 / 2.
        $png = Join-Path $TestDrive 'stored.png'
        Export-WgpuImage (New-WgpuTexture 2 1 -Data ([byte[]](189, 204, 209, 255, 128, 128, 255, 255))) $png
        $signed = Import-WgpuImage $png -Format Rgba16Float -NormalMips Renormalize
        $texels = Read-WgpuTexture $signed
        [math]::Abs($texels[0] - 0.482) | Should -BeLessThan 0.003
        [math]::Abs($texels[1] - 0.6) | Should -BeLessThan 0.003
        [math]::Abs($texels[2] - 0.639) | Should -BeLessThan 0.003
        $signed.Notes -join "`n" | Should -Match 'written again on the CPU as Rgba16Float stores them'
        $dds = Join-Path $TestDrive 'stored.dds'
        Export-WgpuImage $signed $dds -Format Rgba8Unorm -NormalMips Renormalize
        (Read-WgpuTexture (Import-WgpuImage $dds))[0..3] -join ',' | Should -Be '189,204,209,255'
        $null = Import-WgpuImage $png -Format Rgba8Unorm -NormalMips Renormalize -WarningAction SilentlyContinue -WarningVariable warned
        $warned[0].Message | Should -Be 'no mip level was rendered, as the file has every level the call asks for, so -NormalMips changed nothing'
        { Import-WgpuImage $png -NormalMips Renormalize } | Should -Throw '-MipFilter, -MipEdge, -KeepCoverage and -NormalMips choose how the mip levels*'
    }
}

Describe 'Cube maps and lighting' {
    BeforeAll {
        # Six 4x4 faces, each texel different from every other face's.
        $script:faces = foreach ($face in 0..5) {
            New-WgpuTexture 4 4 -Data ([byte[]](0..63 | ForEach-Object { ($face * 40 + $_ * 3) % 256 }))
        }
        # A 32x16 panorama in Rgba32Float whose color follows the direction
        # each texel's center faces: 0.5 + 0.4 d in red, green and blue.
        $panoramaTexels = [float[]]::new(32 * 16 * 4)
        for ($y = 0; $y -lt 16; $y++) {
            for ($x = 0; $x -lt 32; $x++) {
                $phi = (($x + 0.5) / 32 - 0.5) * 2 * [math]::PI
                $theta = ($y + 0.5) / 16 * [math]::PI
                $at = ($y * 32 + $x) * 4
                $panoramaTexels[$at] = 0.5 + 0.4 * [math]::Sin($theta) * [math]::Cos($phi)
                $panoramaTexels[$at + 1] = 0.5 + 0.4 * [math]::Cos($theta)
                $panoramaTexels[$at + 2] = 0.5 + 0.4 * [math]::Sin($theta) * [math]::Sin($phi)
                $panoramaTexels[$at + 3] = 1
            }
        }
        $script:panorama = New-WgpuTexture 32 16 -Format Rgba32Float -Data $panoramaTexels
        # An 8x8 cube of one color, (0.25, 0.5, 0.75).
        $script:gray = New-WgpuTexture 8 8 -Depth 6 -Format Rgba32Float -Cube
        $grayTexels = [float[]]::new(8 * 8 * 4)
        for ($i = 0; $i -lt 64; $i++) { $grayTexels[$i * 4] = 0.25; $grayTexels[$i * 4 + 1] = 0.5; $grayTexels[$i * 4 + 2] = 0.75; $grayTexels[$i * 4 + 3] = 1 }
        foreach ($layer in 0..5) { $gray.Write($grayTexels, 0, $layer) }
        $script:layouts = 'HorizontalCross', 'VerticalCross', 'VerticalCrossTurned', 'HorizontalTee', 'HorizontalStrip', 'VerticalStrip', 'Grid2x3', 'Grid3x2'
    }

    It 'makes a cube of six faces, as an array or by name, and keeps them as they are' -Skip:$noGpu {
        $cube = ConvertTo-WgpuCubeMap $faces
        $cube.IsCube | Should -BeTrue
        $cube.DepthOrArrayLayers | Should -Be 6
        $cube.Format | Should -Be 'Rgba8Unorm'
        foreach ($layer in 0..5) { (Read-WgpuTexture $cube -Layer $layer) -join ',' | Should -Be ((Read-WgpuTexture $faces[$layer]) -join ',') }
        $named = ConvertTo-WgpuCubeMap -PositiveX $faces[0] -NegativeX $faces[1] -PositiveY $faces[2] -NegativeY $faces[3] -PositiveZ $faces[4] -NegativeZ $faces[5]
        (Read-WgpuTexture $named -Layer 3) -join ',' | Should -Be ((Read-WgpuTexture $faces[3]) -join ',')
        (Read-WgpuTexture ([Wgpu.Texture]::CubeFromFaces($faces, $null)) -Layer 5) -join ',' | Should -Be ((Read-WgpuTexture $faces[5]) -join ',')
        $async = [Wgpu.Texture]::CubeFromFacesAsync($faces, 'Rgba16Float').GetAwaiter().GetResult()
        $async.Format | Should -Be 'Rgba16Float'
        $async.IsCube | Should -BeTrue
    }

    It 'lays a cube out in every layout and cuts it back as it was' -Skip:$noGpu {
        $cube = ConvertTo-WgpuCubeMap $faces
        foreach ($layout in $layouts) {
            $image = ConvertFrom-WgpuCubeMap $cube -Layout $layout
            $image.IsCube | Should -BeFalse
            $back = ConvertTo-WgpuCubeMap $image -Layout $layout
            foreach ($layer in 0..5) { (Read-WgpuTexture $back -Layer $layer) -join ',' | Should -Be ((Read-WgpuTexture $cube -Layer $layer) -join ',') -Because "$layout face $layer" }
        }
        (ConvertFrom-WgpuCubeMap $cube).Width | Should -Be 16
        $records = ConvertFrom-WgpuCubeMap $cube -Layout Grid2x3 | ConvertTo-WgpuCubeMap -Verbose 4>&1
        @($records | Where-Object { $_ -is [System.Management.Automation.VerboseRecord] }).Message -join "`n" | Should -Match "the faces were cut from the Grid2x3's cells as they are, 4 texels across"
    }

    It 'reads a panorama, an angular map and an octahedral map, and writes each back' -Skip:$noGpu {
        $records = $panorama | ConvertTo-WgpuCubeMap -Verbose 4>&1
        $cube = @($records | Where-Object { $_ -isnot [System.Management.Automation.VerboseRecord] })[0]
        $cube.Width | Should -Be 8
        $cube.Format | Should -Be 'Rgba32Float'
        @($records | Where-Object { $_ -is [System.Management.Automation.VerboseRecord] }).Message -join "`n" | Should -Match 'the panorama was resampled on the GPU into faces 8 texels across'
        $back = $cube | ConvertFrom-WgpuCubeMap -Panorama
        $back.Width | Should -Be 32
        $back.Height | Should -Be 16
        $rows = Compare-WgpuTexture $back $panorama
        ($rows | Where-Object Channel -ne 'Alpha' | Measure-Object PSNR -Minimum).Minimum | Should -BeGreaterThan 40
        $angular = $cube | ConvertFrom-WgpuCubeMap -Layout AngularMap
        $angular.Width | Should -Be 32
        (ConvertTo-WgpuCubeMap $angular -Layout AngularMap).Width | Should -Be 8
        $octahedral = $cube | ConvertFrom-WgpuCubeMap -Layout Octahedral -FaceSize 16
        $octahedral.Width | Should -Be 32
        (ConvertTo-WgpuCubeMap $octahedral -Layout Octahedral -FaceSize 4 -Yaw 90).Width | Should -Be 4
    }

    It 'makes a cube on Import-WgpuImage and writes one out on Export-WgpuImage' -Skip:$noGpu {
        $exr = Join-Path $TestDrive 'sky.exr'
        Export-WgpuImage $panorama $exr
        $cube = Import-WgpuImage $exr -Cube -FaceSize 4 -Format Rgba16Float -MipLevels 0
        $cube.IsCube | Should -BeTrue
        $cube.Width | Should -Be 4
        $cube.Format | Should -Be 'Rgba16Float'
        $cube.MipLevelCount | Should -Be 3
        $cross = Join-Path $TestDrive 'cross.png'
        Export-WgpuImage (ConvertTo-WgpuCubeMap $faces) $cross -Layout HorizontalCross
        $read = Import-WgpuImage $cross -Cube
        (Read-WgpuTexture $read -Layer 2) -join ',' | Should -Be ((Read-WgpuTexture $faces[2]) -join ',')
        $sky = Join-Path $TestDrive 'back.hdr'
        Export-WgpuImage $cube $sky -Panorama -FaceSize 8
        (Import-WgpuImage $sky).Width | Should -Be 32
        $dds = Join-Path $TestDrive 'layers.dds'
        Export-WgpuImage (New-WgpuTexture 4 4 -Depth 6) $dds
        (Import-WgpuImage $dds -Cube).IsCube | Should -BeTrue
    }

    It 'makes the irradiance map of an environment, and its harmonics' -Skip:$noGpu {
        $records = $gray | ConvertTo-WgpuIrradianceMap -Harmonics -Verbose 4>&1
        $made = @($records | Where-Object { $_ -isnot [System.Management.Automation.VerboseRecord] })[0]
        $made.Width | Should -Be 32
        $made.Format | Should -Be 'Rgba16Float'
        $made.IsCube | Should -BeTrue
        $texels = Read-WgpuTexture $made -Layer 4
        [math]::Abs($texels[0] - 0.25) | Should -BeLessThan 1e-3
        [math]::Abs($texels[2] - 0.75) | Should -BeLessThan 1e-3
        $made.Harmonics.Red.Count | Should -Be 9
        [math]::Abs($made.Harmonics.Red[0] - 0.25 * [math]::PI / 0.282094791773878) | Should -BeLessThan 1e-4
        [math]::Abs($made.Harmonics.Blue[3]) | Should -BeLessThan 1e-6
        @($records | Where-Object { $_ -is [System.Management.Automation.VerboseRecord] }).Message -join "`n" | Should -Match 'the irradiance was summed on the GPU into faces 32 texels across'
        ($gray | ConvertTo-WgpuIrradianceMap -FaceSize 4).Harmonics | Should -BeNullOrEmpty
        ($panorama | ConvertTo-WgpuIrradianceMap -FaceSize 4).Width | Should -Be 4
    }

    It 'makes the specular map, Linear and Quadratic' -Skip:$noGpu {
        $cube = ConvertTo-WgpuCubeMap $panorama
        $linear = $cube | ConvertTo-WgpuSpecularMap -Samples 64
        $linear.MipLevelCount | Should -Be 3
        $linear.Format | Should -Be 'Rgba16Float'
        $base = Read-WgpuTexture $cube -Layer 1
        $level0 = Read-WgpuTexture $linear -Layer 1
        [math]::Abs($level0[0] - $base[0]) | Should -BeLessThan 1e-3
        $quadratic = $cube | ConvertTo-WgpuSpecularMap -RoughnessMapping Quadratic -Samples 64 -Format Rgba32Float
        $quadratic.MipLevelCount | Should -Be 2
        ($cube | ConvertTo-WgpuSpecularMap -Samples 16 -MipLevels 0).MipLevelCount | Should -Be 4
        $gray16 = Read-WgpuTexture ($gray | ConvertTo-WgpuSpecularMap -Samples 32 -Format Rgba32Float) -MipLevel 2 -Layer 3
        [math]::Abs($gray16[1] - 0.5) | Should -BeLessThan 1e-5
        ($panorama | ConvertTo-WgpuSpecularMap -Samples 16).Notes -join "`n" | Should -Match 'prefiltered on the GPU into 3 mip levels from 8 texels a face'
    }

    It 'makes the BRDF table' -Skip:$noGpu {
        $table = New-WgpuBrdfTable -Size 16 -Samples 64
        $table.Width | Should -Be 16
        $table.Format | Should -Be 'Rg16Float'
        $texels = Read-WgpuTexture $table
        # Seen head on, a smooth surface reflects F0: scale 1 and bias 0.
        [math]::Abs($texels[30] + $texels[31] - 1) | Should -BeLessThan 2e-2
        $texels[31] | Should -BeLessThan 1e-3
        $table.Notes -join "`n" | Should -Match "Smith's height-correlated GGX visibility"
        (Read-WgpuTexture ([Wgpu.Texture]::NewBrdfTable(16, 64, $null, $null))) -join ',' | Should -Be ($texels -join ',')
    }

    It 'converts cubes and makes lighting maps through methods and their Async forms' -Skip:$noGpu {
        $cube = $panorama.ToCubeMap($null, $null, $null, $null)
        $cube.IsCube | Should -BeTrue
        $panorama.ToCubeMapAsync($null, 45, 4, $null).GetAwaiter().GetResult().Width | Should -Be 4
        $cube.ToPanorama($null, $null, $null, $null).Width | Should -Be 32
        $cube.ToPanoramaAsync(10, 4, $null, 'Rgba16Float').GetAwaiter().GetResult().Format | Should -Be 'Rgba16Float'
        $cube.ToLayout('VerticalStrip', $null, $null, $null, $null).Height | Should -Be 48
        $cube.ToLayoutAsync('Octahedral', $null, $null, $null, $null).GetAwaiter().GetResult().Width | Should -Be 16
        $cube.ToIrradianceMap(4, $true, $null).Harmonics.Green.Count | Should -Be 9
        $cube.ToIrradianceMapAsync(4, $false, $null).GetAwaiter().GetResult().Width | Should -Be 4
        $cube.ToSpecularMap('Quadratic', 16, $null, $null).MipLevelCount | Should -Be 2
        $cube.ToSpecularMapAsync($null, 16, 2, $null).GetAwaiter().GetResult().MipLevelCount | Should -Be 2
        [Wgpu.Texture]::NewBrdfTableAsync(8, 16, 'Rg32Float', $null).GetAwaiter().GetResult().Format | Should -Be 'Rg32Float'
    }

    It 'refuses what makes no cube or lighting map, and says why' -Skip:$noGpu {
        { ConvertTo-WgpuCubeMap $faces[0..4] } | Should -Throw 'a cube is made from one image holding its faces or from six textures, its faces, and 5 were given'
        $odd = @($faces[0..2]) + (New-WgpuTexture 8 8) + @($faces[4..5])
        { ConvertTo-WgpuCubeMap $odd } | Should -Throw "the -Y face is 8x8 Rgba8Unorm where the +X face is 4x4 Rgba8Unorm: a cube's faces share one size and format*"
        { ConvertTo-WgpuCubeMap $faces -Layout HorizontalCross } | Should -Throw '-Layout, -Yaw and -FaceSize read a cube from one image; six textures are its faces as they are'
        { ConvertTo-WgpuCubeMap (New-WgpuTexture 8 8) } | Should -Throw 'a square image may be an angular map or an octahedral map: give -Layout AngularMap or -Layout Octahedral*'
        { ConvertTo-WgpuCubeMap (New-WgpuTexture 12 4) } | Should -Throw 'a 12x4 image has no shape a cube comes in*'
        { ConvertTo-WgpuCubeMap (New-WgpuTexture 16 12) -Layout Grid3x2 } | Should -Throw 'a Grid3x2 is 3 cells across and 2 down, which a 16x12 image is not'
        { ConvertTo-WgpuCubeMap (New-WgpuTexture 16 12) -Yaw 90 } | Should -Throw '-Yaw and -FaceSize apply to a panorama, an angular map or an octahedral map, which are resampled*'
        { ConvertTo-WgpuCubeMap $panorama -FaceSize 0 } | Should -Throw "-FaceSize is the faces' size in texels, 1 or more"
        { ConvertTo-WgpuCubeMap $panorama -Yaw ([double]::NaN) } | Should -Throw '-Yaw is a finite number of degrees, not NaN'
        $cube = ConvertTo-WgpuCubeMap $faces
        { ConvertFrom-WgpuCubeMap $cube -Layout HorizontalCross -Panorama } | Should -Throw '-Layout and -Panorama each name the image; give one'
        { ConvertFrom-WgpuCubeMap $faces[0] } | Should -Throw 'a cube map is six square layers, its faces, and this texture holds one 4x4 layer'
        { ConvertFrom-WgpuCubeMap $cube -MipLevel 1 } | Should -Throw 'the cube has mip levels 0 to 0, not 1'
        $png = Join-Path $TestDrive 'face.png'
        Export-WgpuImage $faces[0] $png
        { Import-WgpuImage $png -Layout AngularMap } | Should -Throw '-Layout, -Yaw and -FaceSize say how -Cube reads a cube from the file: give -Cube'
        { Export-WgpuImage $cube (Join-Path $TestDrive 'cube.dds') -Layout HorizontalCross } | Should -Throw '-Layout and -Panorama write a cube into an image file; a .dds or .ktx2 file holds its faces as they are'
        { Export-WgpuImage $cube (Join-Path $TestDrive 'cube.png') -Yaw 10 } | Should -Throw '-Yaw and -FaceSize say how -Layout or -Panorama writes a cube into an image file: give one of them'
        { ConvertTo-WgpuSpecularMap $cube -Samples 0 } | Should -Throw '-Samples is how many samples each texel takes, 1 or more'
        { ConvertTo-WgpuSpecularMap $cube -MipLevels 9 } | Should -Throw 'faces 4 texels across have 3 mip levels down to 1x1, not 9'
        { ConvertTo-WgpuIrradianceMap $cube -Layout HorizontalCross } | Should -Throw '-Layout and -Yaw read a cube from one image, and this texture is a cube''s six faces already'
        { New-WgpuBrdfTable -Size 0 } | Should -Throw "-Size is the table's side in texels, 1 or more"
    }
}

Describe 'Textures in compute shaders' {
    BeforeAll {
        # Reads each face of a cube at the middle of the face: +X, -X, +Y,
        # -Y, +Z and -Z in turn.
        $script:byFace = @'
@group(0) @binding(0) var faces: texture_cube<f32>;
@group(0) @binding(1) var smp: sampler;
@group(0) @binding(2) var<storage, read_write> red: array<f32>;
@compute @workgroup_size(1)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    var axes = array<vec3<f32>, 6>(vec3<f32>(1.0, 0.0, 0.0), vec3<f32>(-1.0, 0.0, 0.0), vec3<f32>(0.0, 1.0, 0.0), vec3<f32>(0.0, -1.0, 0.0), vec3<f32>(0.0, 0.0, 1.0), vec3<f32>(0.0, 0.0, -1.0));
    red[id.x] = textureSampleLevel(faces, smp, axes[id.x], 0.0).r;
}
'@
        $script:byLayer = @'
@group(0) @binding(0) var layers: texture_2d_array<f32>;
@group(0) @binding(1) var<storage, read_write> red: array<f32>;
@compute @workgroup_size(1)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    red[id.x] = textureLoad(layers, vec2<i32>(0, 0), i32(id.x), 0).r;
}
'@
        $script:stencilRead = @'
@group(0) @binding(0) var marks: texture_2d<u32>;
@group(0) @binding(1) var<storage, read_write> values: array<u32>;
@compute @workgroup_size(1)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    values[id.x] = textureLoad(marks, vec2<i32>(i32(id.x % 2u), i32(id.x / 2u)), 0).r;
}
'@
        $script:depthRead = @'
@group(0) @binding(0) var depth: texture_depth_2d;
@group(0) @binding(1) var<storage, read_write> values: array<f32>;
@compute @workgroup_size(1)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    values[id.x] = textureLoad(depth, vec2<i32>(i32(id.x % 2u), i32(id.x / 2u)), 0);
}
'@
        $script:nearest = New-WgpuSampler -Filter Nearest
        # Six 4x4 faces in Rgba8Unorm, face f all red f * 40.
        function script:New-Faces([int] $Layers, [switch] $Cube) {
            $texture = New-WgpuTexture 4 4 -Depth $Layers -Cube:$Cube
            foreach ($layer in 0..($Layers - 1)) { $texture.Write([byte[]](1..16 | ForEach-Object { (($layer % 6) * 40), 0, 0, 255 }), 0, $layer) }
            $texture
        }
        function script:Get-Red($Values) { ($Values | ForEach-Object { [math]::Round($_ * 255) }) -join ',' }
    }

    It 'binds a cube map whole as a cube or as six layers, and its view with no dimension as a cube' -Skip:$noGpu {
        $cube = New-Faces 6 -Cube
        Get-Red (Invoke-WgpuCompute $byFace @{ faces = $cube; smp = $nearest; red = [float[]]::new(6) } -Workgroups 6) | Should -Be '0,40,80,120,160,200'
        Get-Red (Invoke-WgpuCompute $byFace @{ faces = $cube.CreateView(); smp = $nearest; red = [float[]]::new(6) } -Workgroups 6) | Should -Be '0,40,80,120,160,200'
        Get-Red (Invoke-WgpuCompute $byLayer @{ layers = $cube; red = [float[]]::new(6) } -Workgroups 6) | Should -Be '0,40,80,120,160,200'
    }

    It 'binds six square layers as a cube whether or not the texture is a cube map' -Skip:$noGpu {
        $layers = New-Faces 6
        $layers.IsCube | Should -BeFalse
        Get-Red (Invoke-WgpuCompute $byFace @{ faces = $layers; smp = $nearest; red = [float[]]::new(6) } -Workgroups 6) | Should -Be '0,40,80,120,160,200'
    }

    It 'binds the depth or the stencil of a depth-and-stencil texture, as the binding reads it' -Skip:$noGpu {
        $both = New-WgpuTexture 2 2 -Format Depth24PlusStencil8
        Write-WgpuTexture $both ([byte[]](1, 2, 3, 4)) -Aspect StencilOnly
        (Invoke-WgpuCompute $stencilRead @{ marks = $both; values = [uint32[]]::new(4) } -Workgroups 4) -join ',' | Should -Be '1,2,3,4'
        (Invoke-WgpuCompute $depthRead @{ depth = $both; values = [float[]]::new(4) } -Workgroups 4) -join ',' | Should -Be '0,0,0,0'
    }

    It 'refuses a texture its binding cannot read whole, and binds the part a view picks' -Skip:$noGpu {
        $two = New-Faces 12 -Cube
        { Invoke-WgpuCompute $byFace @{ faces = $two; smp = $nearest; red = [float[]]::new(6) } -Workgroups 6 } | Should -Throw 'faces is a texture_cube<f32> binding, which reads six layers, and the texture has 12; bind six through CreateView'
        Get-Red (Invoke-WgpuCompute $byFace @{ faces = $two.CreateView($null, 'Cube', $null, $null, 6, 6); smp = $nearest; red = [float[]]::new(6) } -Workgroups 6) | Should -Be '0,40,80,120,160,200'
        { Invoke-WgpuCompute $byFace @{ faces = (New-WgpuTexture 8 4 -Depth 6); smp = $nearest; red = [float[]]::new(6) } -Workgroups 6 } | Should -Throw 'faces is a texture_cube<f32> binding, which reads square layers, and the texture is 8x4'
        { Invoke-WgpuCompute $sample @{ source = (New-WgpuTexture 4 1 -Depth 6 -Format R32Float); smp = $nearest; red = [float[]]::new(4) } -Workgroups 4 } | Should -Throw 'source is a texture_2d<f32> binding, which reads one layer, and the texture has 6; bind one through CreateView'
        $mipped = New-WgpuTexture 16 16 -MipLevels 5 -Usage StorageBinding, CopySrc
        { Invoke-WgpuCompute $fill $mipped -Workgroups 2, 2 } | Should -Throw 'target_texture is a texture_storage_2d<rgba8unorm, write> binding, which takes one mip level, and the texture has 5; bind one through CreateView'
        Invoke-WgpuCompute $fill $mipped.CreateView($null, $null, 0, 1) -Workgroups 2, 2
        (Read-WgpuTexture $mipped)[0..3] -join ',' | Should -Be '0,0,0,255'
    }

    It 'fills a storage texture' -Skip:$noGpu {
        $texture = New-WgpuTexture 16 16 -Usage StorageBinding, CopySrc
        Invoke-WgpuCompute $fill $texture -Workgroups 2, 2
        $texels = Read-WgpuTexture $texture
        $texels[0..3] -join ',' | Should -Be '0,0,0,255'
        $last = ((15 * 16) + 15) * 4
        $texels[$last..($last + 3)] -join ',' | Should -Be '15,15,0,255'
    }

    It 'samples a texture through a sampler' -Skip:$noGpu {
        $texture = New-WgpuTexture 4 1 -Format R32Float -Data ([float[]](1, 2, 3, 4))
        $nearest = New-WgpuSampler -Filter Nearest
        $out = Invoke-WgpuCompute $sample @{ source = $texture; smp = $nearest; red = [float[]]::new(4) } -Workgroups 4
        $out -join ',' | Should -Be '1,2,3,4'
    }
}
