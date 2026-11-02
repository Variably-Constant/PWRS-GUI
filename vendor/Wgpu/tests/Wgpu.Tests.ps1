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
}

Describe 'Aliases' {
    It 'gives <Name> its Gpu alias' -TestCases @(
        @{ Name = 'Get-WgpuAdapter' }, @{ Name = 'Open-WgpuDevice' }, @{ Name = 'New-WgpuBuffer' }, @{ Name = 'Read-WgpuBuffer' },
        @{ Name = 'Invoke-WgpuCompute' }, @{ Name = 'Invoke-WgpuMap' }, @{ Name = 'Measure-WgpuArray' }, @{ Name = 'New-WgpuTexture' },
        @{ Name = 'Import-WgpuImage' }, @{ Name = 'ConvertTo-WgpuShader' }, @{ Name = 'Submit-WgpuCommand' }
    ) {
        param($Name)
        $alias = $Name -replace '-Wgpu', '-Gpu'
        (Get-Alias $alias).Definition | Should -Be $Name
    }
}

Describe 'Get-WgpuAdapter' {
    It 'describes each adapter with its features and limits' -Skip:$noGpu {
        $adapter = Get-WgpuAdapter | Select-Object -First 1
        $adapter.Name | Should -Not -BeNullOrEmpty
        $adapter.Backend | Should -BeIn @('Vulkan', 'Metal', 'Dx12', 'Gl')
        $adapter.Limits.MaxBufferSize | Should -BeGreaterThan 0
        $adapter.Limits.MaxComputeWorkgroupSizeX | Should -BeGreaterThan 0
        $adapter.DownlevelFlags.Count | Should -BeGreaterThan 0
    }

    It 'keeps to the backends asked for' -Skip:$noGpu {
        foreach ($backend in 'Vulkan', 'Dx12', 'Gl', 'Metal') {
            Get-WgpuAdapter -Backend $backend | ForEach-Object { $_.Backend | Should -Be $backend }
        }
    }

    It 'lists wgpu''s noop adapter when asked for it' {
        (Get-WgpuAdapter -Backend Noop).Backend | Should -Contain 'Noop'
    }
}

Describe 'Open-WgpuDevice' {
    It 'opens a device with every feature and limit of its adapter' -Skip:$noGpu {
        $gpu = Open-WgpuDevice
        $adapter = Get-WgpuAdapter | Where-Object { $_.Name -eq $gpu.Adapter -and $_.Backend -eq $gpu.Backend } | Select-Object -First 1
        $gpu.Limits.MaxBufferSize | Should -Be $adapter.Limits.MaxBufferSize
        $missing = $adapter.Features | Where-Object { $_ -notlike 'Experimental*' -and $gpu.Features -notcontains $_ }
        $missing | Should -BeNullOrEmpty
    }

    It 'opens only the features named' -Skip:$noGpu {
        $adapter = Get-WgpuAdapter | Where-Object { $_.Features.Count -gt 0 } | Select-Object -First 1
        if (-not $adapter) { Set-ItResult -Skipped -Because 'no adapter has an optional feature'; return }
        $feature = $adapter.Features | Where-Object { $_ -notlike 'Experimental*' } | Select-Object -First 1
        $gpu = Open-WgpuDevice -Adapter $adapter.Name -Backend $adapter.Backend -Feature $feature
        $gpu.Features | Should -Be @($feature)
    }

    It 'refuses a feature wgpu does not have' -Skip:$noGpu {
        { Open-WgpuDevice -Feature NoSuchFeature } | Should -Throw '*no feature named NoSuchFeature*'
    }

    It 'opens on the noop backend' {
        $device = Open-WgpuDevice -Backend Noop
        $device.Backend | Should -Be 'Noop'
    }

    It 'traces into the folder -TracePath names, and refuses one that does not exist' -Skip:$noGpu {
        $folder = Join-Path ([IO.Path]::GetTempPath()) "wgpu-trace-$PID-$(Get-Random)"
        Open-WgpuDevice -TracePath $folder -ErrorAction SilentlyContinue -ErrorVariable failed
        $failed[0].FullyQualifiedErrorId | Should -BeLike 'WgpuTracePath,*'
        $null = New-Item -ItemType Directory -Path $folder
        try {
            $gpu = Open-WgpuDevice -TracePath $folder
            Join-Path $folder 'trace.ron' | Should -Exist
            $gpu | Close-WgpuDevice
        } finally {
            Remove-Item $folder -Recurse -Force -ErrorAction Ignore
        }
    }

    It 'reports its status and counters' -Skip:$noGpu {
        $gpu = Open-WgpuDevice
        $status = $gpu.Status()
        $status.IsLost | Should -BeFalse
        $status.LostReason | Should -BeNullOrEmpty
        $null = $gpu.Upload([float[]](1, 2, 3))
        $gpu.Counters().Buffers | Should -BeGreaterOrEqual 0
        $gpu.TimestampPeriod() | Should -BeGreaterThan 0
    }

    It 'becomes the default device with -Default' -Skip:$noGpu {
        $gpu = Open-WgpuDevice -Default -Label chosen
        (Get-WgpuDevice).Label | Should -Be 'chosen'
        $gpu.Status().IsDefault | Should -BeTrue
    }

    It 'waits for submitted work with Wait-WgpuDevice' -Skip:$noGpu {
        $gpu = Get-WgpuDevice
        $null = New-WgpuBuffer ([float[]](1, 2, 3)) -Device $gpu
        $gpu | Wait-WgpuDevice
        Wait-WgpuDevice
        $gpu.Status().IsLost | Should -BeFalse
    }

    It 'stops working once Close-WgpuDevice destroys it' -Skip:$noGpu {
        $gpu = Open-WgpuDevice
        $gpu | Close-WgpuDevice
        { $gpu.Upload([float[]](1)) } | Should -Throw
    }

    It 'leaves the device open under Close-WgpuDevice -WhatIf' -Skip:$noGpu {
        $gpu = Open-WgpuDevice
        $gpu | Close-WgpuDevice -WhatIf
        $gpu.Status().IsLost | Should -BeFalse
        $gpu.Download($gpu.Upload([float[]](4))) | Should -Be 4
        $gpu | Close-WgpuDevice
    }

    It 'makes Test-WgpuShader -Device refuse a shader that needs a feature the device was opened without' -Skip:$noGpu {
        $f16 = "enable f16;`n@group(0) @binding(0) var<storage, read_write> v: array<f16>;`n@compute @workgroup_size(1) fn main() { v[0] = v[0] * 2.0h; }"
        $plain = "@group(0) @binding(0) var<storage, read_write> v: array<f32>;`n@compute @workgroup_size(1) fn main() { v[0] = v[0] * 2.0; }"
        Test-WgpuShader -Source $f16 | Should -BeTrue
        $gpu = Open-WgpuDevice -Feature @() -Label featureless
        Test-WgpuShader -Source $f16 -Device $gpu | Should -BeFalse
        @(Test-WgpuShader -Source $f16 -Device $gpu -Diagnostics)[0].Message | Should -BeLike '*f16*'
        @(Test-WgpuShader -Source $plain -Device $gpu -Diagnostics).Count | Should -Be 0
        $gpu | Close-WgpuDevice
    }
}

Describe 'New-WgpuBuffer and Read-WgpuBuffer' {
    It 'round-trips <Type> as its own element type' -Skip:$noGpu -TestCases @(
        @{ Type = 'float'; Element = 'Float32'; Values = '1,2,3,4,5,6,7' }
        @{ Type = 'double'; Element = 'Float64'; Values = '1,2,3,4,5,6,7' }
        @{ Type = 'byte'; Element = 'UInt8'; Values = '1,2,3,4,5,6,7' }
        @{ Type = 'sbyte'; Element = 'Int8'; Values = '-3,-2,-1,0,1,2,3' }
        @{ Type = 'int16'; Element = 'Int16'; Values = '-3,-2,-1,0,1,2,3' }
        @{ Type = 'uint16'; Element = 'UInt16'; Values = '1,2,3,4,5,6,7' }
        @{ Type = 'int'; Element = 'Int32'; Values = '-3,-2,-1,0,1,2,3' }
        @{ Type = 'uint32'; Element = 'UInt32'; Values = '1,2,3,4,5,6,7' }
        @{ Type = 'long'; Element = 'Int64'; Values = '-3,-2,-1,0,1,2,3' }
        @{ Type = 'uint64'; Element = 'UInt64'; Values = '1,2,3,4,5,6,7' }
    ) {
        param($Type, $Element, $Values)
        $data = ($Values -split ',') -as "$Type[]"
        $buffer = New-WgpuBuffer $data
        $buffer.ElementType | Should -Be $Element
        $buffer.Length | Should -Be 7
        $back = Read-WgpuBuffer $buffer
        $back.GetType() | Should -Be $data.GetType()
        ($back -join ',') | Should -Be $Values
    }

    It 'stores Float16 and reads it back as float' -Skip:$noGpu {
        $buffer = New-WgpuBuffer ([float[]](0.5, 1.5, -2)) -ElementType Float16
        $buffer.Size | Should -Be 8
        (Read-WgpuBuffer $buffer) -join ',' | Should -Be '0.5,1.5,-2'
    }

    It 'converts to the -Type given' -Skip:$noGpu {
        $buffer = New-WgpuBuffer ([int[]](1, 2, 3)) -Type Float64
        (Read-WgpuBuffer $buffer).GetType().Name | Should -Be 'Double[]'
    }

    It 'reads the buffer piped to it' -Skip:$noGpu {
        $buffer = New-WgpuBuffer ([int[]](4, 5, 6))
        ($buffer | Read-WgpuBuffer) -join ',' | Should -Be '4,5,6'
    }

    It 'takes numbers from the pipeline' -Skip:$noGpu {
        $buffer = 1..5 | New-WgpuBuffer -Type Int32
        $buffer.Length | Should -Be 5
        (Read-WgpuBuffer $buffer) -join ',' | Should -Be '1,2,3,4,5'
    }

    It 'makes zeroed buffers of a length' -Skip:$noGpu {
        $buffer = New-WgpuBuffer -Length 10 -Type UInt32
        (Read-WgpuBuffer $buffer) -join ',' | Should -Be '0,0,0,0,0,0,0,0,0,0'
    }

    It 'reads a range, as another type, and element by element' -Skip:$noGpu {
        $buffer = New-WgpuBuffer ([float[]](1..10))
        (Read-WgpuBuffer $buffer -Offset 3 -Count 2) -join ',' | Should -Be '4,5'
        (Read-WgpuBuffer $buffer -Count 1 -Type UInt32)[0] | Should -Be 1065353216
        @(Read-WgpuBuffer $buffer -Enumerate).Count | Should -Be 10
    }

    It 'reads into Memory<T> on PowerShell 7' -Skip:$noGpu {
        $buffer = New-WgpuBuffer ([int[]](7, 8, 9))
        $values = Read-WgpuBuffer $buffer -AsMemory
        if ($PSVersionTable.PSEdition -eq 'Core') {
            $values.GetType().Name | Should -BeLike '*Memory*'
            $values.ToArray() -join ',' | Should -Be '7,8,9'
        } else {
            $values -join ',' | Should -Be '7,8,9'
        }
    }

    It 'reads into an array it is given from index 0, writing it only with -PassThru' -Skip:$noGpu {
        $buffer = New-WgpuBuffer ([int[]](1..4))
        $values = [int[]](9, 9, 9, 9, 9, 9)
        @(Read-WgpuBuffer $buffer -Destination $values).Count | Should -Be 0
        $values -join ',' | Should -Be '1,2,3,4,9,9'
        Write-WgpuBuffer $buffer ([int[]](5, 6))
        $passed = Read-WgpuBuffer $buffer -Offset 1 -Count 2 -Destination $values -PassThru
        $values -join ',' | Should -Be '6,3,3,4,9,9'
        $passed.GetType().Name | Should -Be 'Int32[]'
        $passed[5] = 0
        $values[5] | Should -Be 0
        $buffer.ReadInto($values)
        $values -join ',' | Should -Be '5,6,3,4,9,0'
    }

    It 'reads a byte buffer from every offset within a word, into a new array and into one it is given' -Skip:$noGpu {
        $data = [byte[]](1..40)
        $buffer = New-WgpuBuffer $data
        $kept = [byte[]]::new(16)
        foreach ($offset in 0..7) {
            foreach ($count in 1..9) {
                $want = $data[$offset..($offset + $count - 1)] -join ','
                (Read-WgpuBuffer $buffer -Offset $offset -Count $count) -join ',' | Should -Be $want -Because "-Offset $offset -Count $count"
                $buffer.ReadRange($offset, $count) -join ',' | Should -Be $want -Because "ReadRange($offset, $count)"
                Read-WgpuBuffer $buffer -Offset $offset -Count $count -Destination $kept
                $kept[0..($count - 1)] -join ',' | Should -Be $want -Because "-Destination -Offset $offset -Count $count"
            }
        }
    }

    It 'reads 1 MiB and more from every offset within a word, every way a read returns' -Skip:$noGpu {
        $count = (1 -shl 20) + 64
        $data = [byte[]]::new($count + 8)
        [System.Random]::new(7).NextBytes($data)
        $buffer = New-WgpuBuffer $data
        $kept = [byte[]]::new($count)
        foreach ($offset in 0..7) {
            $want = [byte[]]::new($count)
            [Array]::Copy($data, $offset, $want, 0, $count)
            $memory = Read-WgpuBuffer $buffer -Offset $offset -Count $count -AsMemory
            $fromMemory = if ($PSVersionTable.PSEdition -eq 'Core') { $memory.ToArray() } else { $memory }
            foreach ($case in @(
                @{ Name = 'a new array'; Values = (Read-WgpuBuffer $buffer -Offset $offset -Count $count) }
                @{ Name = 'ReadRangeAsync'; Values = $buffer.ReadRangeAsync($offset, $count).GetAwaiter().GetResult() }
                @{ Name = 'Memory<T>'; Values = $fromMemory }
            )) {
                [System.Linq.Enumerable]::SequenceEqual([byte[]]$case.Values, $want) | Should -BeTrue -Because "$($case.Name) from -Offset $offset"
            }
            Read-WgpuBuffer $buffer -Offset $offset -Count $count -Destination $kept
            [System.Linq.Enumerable]::SequenceEqual($kept, $want) | Should -BeTrue -Because "-Destination from -Offset $offset"
            [Array]::Clear($kept, 0, $count)
            $buffer.Slice($offset, $count).ReadInto($kept)
            [System.Linq.Enumerable]::SequenceEqual($kept, $want) | Should -BeTrue -Because "a slice's ReadInto from element $offset"
        }
        $halves = [float[]]::new((1 -shl 19) + 8)
        for ($i = 0; $i -lt $halves.Length; $i++) { $halves[$i] = $i % 2048 }
        $half = New-WgpuBuffer $halves -ElementType Float16
        $wide = Read-WgpuBuffer $half -Offset 1 -Count ((1 -shl 19) + 4)
        $wide[0], $wide[2047], $wide[2048], $wide[-1] -join ',' | Should -Be "1,0,1,$(((1 -shl 19) + 4) % 2048)"
    }

    It 'reads as -ElementType into an array of that type, and Float16 into a float array' -Skip:$noGpu {
        $bytes = [byte[]]::new(8)
        Read-WgpuBuffer (New-WgpuBuffer ([float[]](1, 2))) -ElementType UInt8 -Destination $bytes
        $bytes -join ',' | Should -Be '0,0,128,63,0,0,0,64'
        $floats = [float[]]::new(2)
        Read-WgpuBuffer (New-WgpuBuffer ([float[]](0.5, -2)) -ElementType Float16) -Destination $floats
        $floats -join ',' | Should -Be '0.5,-2'
    }

    It 'refuses a destination of another type or shorter than the read, naming both' -Skip:$noGpu {
        $buffer = New-WgpuBuffer ([float[]](1..4))
        Read-WgpuBuffer $buffer -Destination ([double[]]::new(4)) -ErrorAction SilentlyContinue -ErrorVariable failed
        $failed[0].FullyQualifiedErrorId | Should -BeLike 'WgpuDestination,*'
        $failed[0].CategoryInfo.Category | Should -Be 'InvalidArgument'
        $failed[0].Exception.Message | Should -Be 'a read of Float32 elements fills a System.Single[], and the destination is a System.Double[]'
        Read-WgpuBuffer $buffer -Destination ([float[]]::new(3)) -ErrorAction SilentlyContinue -ErrorVariable failed
        $failed[0].Exception.Message | Should -Be 'the read gives 4 elements, and the destination holds 3'
        { $buffer.ReadInto([int[]]::new(4)) } | Should -Throw '*WgpuDestination*fills a System.Single*and the destination is a System.Int32*'
        { $buffer.ReadInto($null) } | Should -Throw '*WgpuDestination*no array was given to read into*'
        $refused = try { Read-WgpuBuffer $buffer -Destination ([float[]]::new(4)) -AsMemory } catch { $_ }
        $refused.FullyQualifiedErrorId | Should -BeLike 'AmbiguousParameterSet,*'
    }

    It 'refuses a buffer larger than the device creates, as an error' -Skip:$noGpu {
        $gpu = Get-WgpuDevice
        $elements = [uint64]($gpu.MaxBufferSize / 8) + 1
        { New-WgpuBuffer -Length $elements -Type UInt64 -Device $gpu } | Should -Throw '*more than the largest buffer*'
    }
}

Describe 'Writing, copying and clearing buffers' {
    It 'overwrites from an element offset' -Skip:$noGpu {
        $buffer = New-WgpuBuffer ([float[]](1..6))
        Write-WgpuBuffer $buffer ([float[]](10, 20)) -Offset 2
        (Read-WgpuBuffer $buffer) -join ',' | Should -Be '1,2,10,20,5,6'
    }

    It 'writes single bytes without touching their neighbors' -Skip:$noGpu {
        $buffer = New-WgpuBuffer ([byte[]](1..9))
        Write-WgpuBuffer $buffer ([byte[]](77)) -Offset 5
        (Read-WgpuBuffer $buffer) -join ',' | Should -Be '1,2,3,4,5,77,7,8,9'
    }

    It 'copies elements between buffers' -Skip:$noGpu {
        $a = New-WgpuBuffer ([int[]](1..8))
        $b = New-WgpuBuffer -Length 8 -Type Int32
        Copy-WgpuBuffer $a $b -SourceOffset 4 -Count 4
        (Read-WgpuBuffer $b) -join ',' | Should -Be '5,6,7,8,0,0,0,0'
    }

    It 'fills with a value and clears to zero' -Skip:$noGpu {
        $buffer = New-WgpuBuffer -Length 4 -Type Float32
        Clear-WgpuBuffer $buffer -Value 2.5
        (Read-WgpuBuffer $buffer) -join ',' | Should -Be '2.5,2.5,2.5,2.5'
        $buffer | Clear-WgpuBuffer
        (Read-WgpuBuffer $buffer) -join ',' | Should -Be '0,0,0,0'
    }

    It 'reads a slice and reinterprets the bytes' -Skip:$noGpu {
        $buffer = New-WgpuBuffer ([float[]](1..8))
        $buffer.Slice(2, 3).Read() -join ',' | Should -Be '3,4,5'
        $buffer.Reinterpret('UInt32').Read()[0] | Should -Be 1065353216
    }

    It 'frees a buffer with Remove-WgpuBuffer' -Skip:$noGpu {
        $buffer = New-WgpuBuffer ([float[]](1, 2))
        $buffer | Remove-WgpuBuffer
        { Read-WgpuBuffer $buffer } | Should -Throw
    }

    It 'leaves the buffer as it was under -WhatIf on Clear-WgpuBuffer and Remove-WgpuBuffer' -Skip:$noGpu {
        $buffer = New-WgpuBuffer ([float[]](1, 2))
        $buffer | Clear-WgpuBuffer -Value 9 -WhatIf
        $buffer | Remove-WgpuBuffer -WhatIf
        (Read-WgpuBuffer $buffer) -join ',' | Should -Be '1,2'
    }
}

Describe 'Untyped numbers in integer elements' {
    It 'refuses a number outside the element type''s range, whichever way it comes' -Skip:$noGpu {
        New-WgpuBuffer -Data @(300) -Type UInt8 -ErrorAction SilentlyContinue -ErrorVariable failed
        $failed[0].FullyQualifiedErrorId | Should -BeLike 'WgpuOutOfRange,*'
        $failed[0].CategoryInfo.Category | Should -Be 'InvalidArgument'
        $failed[0].Exception.Message | Should -Be '300 does not fit in UInt8, which holds 0 to 255'
        New-WgpuBuffer -Data @(-1) -Type UInt32 -ErrorAction SilentlyContinue -ErrorVariable failed
        $failed[0].Exception.Message | Should -Be '-1 does not fit in UInt32, which holds 0 to 4294967295'
        New-WgpuBuffer -Data @(1e30) -Type Int64 -ErrorAction SilentlyContinue -ErrorVariable failed
        $failed[0].Exception.Message | Should -BeLike '* does not fit in Int64, which holds -9223372036854775808 to 9223372036854775807'
        70000 | New-WgpuBuffer -Type Int16 -ErrorAction SilentlyContinue -ErrorVariable failed
        $failed[0].FullyQualifiedErrorId | Should -BeLike 'WgpuOutOfRange,*'
        $bytes = New-WgpuBuffer -Length 2 -Type UInt8
        Write-WgpuBuffer $bytes @(1, 256) -ErrorAction SilentlyContinue -ErrorVariable failed
        $failed[0].FullyQualifiedErrorId | Should -BeLike 'WgpuOutOfRange,*'
        Clear-WgpuBuffer $bytes -Value 300 -ErrorAction SilentlyContinue -ErrorVariable failed
        $failed[0].FullyQualifiedErrorId | Should -BeLike 'WgpuOutOfRange,*'
        { $bytes.Fill(-1) } | Should -Throw '*WgpuOutOfRange*-1 does not fit in UInt8*'
        (Read-WgpuBuffer $bytes) -join ',' | Should -Be '0,0'
        New-WgpuTexture 1 1 -Data @(300, 0, 0, 255) -ErrorAction SilentlyContinue -ErrorVariable failed
        $failed[0].FullyQualifiedErrorId | Should -BeLike 'WgpuOutOfRange,*'
        (Read-WgpuBuffer (255 | New-WgpuBuffer -Type UInt8))[0] | Should -Be 255
    }

    It 'rounds a number with a fraction half to even, as PowerShell does, whichever way it comes' -Skip:$noGpu {
        (Read-WgpuBuffer (New-WgpuBuffer -Data @(1.5, 2.5, -1.5) -Type Int32)) -join ',' | Should -Be '2,2,-2'
        $words = New-WgpuBuffer -Length 2 -Type UInt32
        Clear-WgpuBuffer $words -Value 1.5
        (Read-WgpuBuffer $words) -join ',' | Should -Be '2,2'
        $words.Fill(2.5)
        (Read-WgpuBuffer $words) -join ',' | Should -Be '2,2'
        $words.FillAsync(3.5).GetAwaiter().GetResult()
        (Read-WgpuBuffer $words) -join ',' | Should -Be '4,4'
    }

    It 'fills 64-bit elements exactly' -Skip:$noGpu {
        $long = New-WgpuBuffer -Length 1 -Type Int64
        Clear-WgpuBuffer $long -Value ([long]::MaxValue)
        (Read-WgpuBuffer $long)[0] | Should -Be ([long]::MaxValue)
        $long.Fill([long]9007199254740993)
        (Read-WgpuBuffer $long)[0] | Should -Be ([long]9007199254740993)
        $unsigned = New-WgpuBuffer -Length 1 -Type UInt64
        $unsigned.Fill([uint64]::MaxValue)
        (Read-WgpuBuffer $unsigned)[0] | Should -Be ([uint64]::MaxValue)
    }
}

Describe 'Methods that return tasks' {
    It 'reads through ReadAsync, ReadRangeAsync and ReadMemoryAsync' -Skip:$noGpu {
        $buffer = New-WgpuBuffer ([int[]](1..6))
        $read = $buffer.ReadAsync($null)
        $read | Should -BeOfType ([Threading.Tasks.Task])
        $read.GetAwaiter().GetResult() -join ',' | Should -Be '1,2,3,4,5,6'
        $buffer.ReadRangeAsync(2, 3).GetAwaiter().GetResult() -join ',' | Should -Be '3,4,5'
        $memory = $buffer.ReadMemoryAsync().GetAwaiter().GetResult()
        $values = if ($PSVersionTable.PSEdition -eq 'Core') { $memory.ToArray() } else { $memory }
        $values -join ',' | Should -Be '1,2,3,4,5,6'
        (New-WgpuBuffer ([float[]](1, 2))).ReadAsync('UInt32').GetAwaiter().GetResult()[0] | Should -Be 1065353216
    }

    It 'writes, fills, copies and clears through tasks' -Skip:$noGpu {
        $buffer = New-WgpuBuffer ([int[]](1..6))
        $buffer.WriteAsync([int[]](10, 20), 1).GetAwaiter().GetResult()
        (Read-WgpuBuffer $buffer) -join ',' | Should -Be '1,10,20,4,5,6'
        $buffer.FillAsync(7).GetAwaiter().GetResult()
        (Read-WgpuBuffer $buffer) -join ',' | Should -Be '7,7,7,7,7,7'
        $other = New-WgpuBuffer -Length 6 -Type Int32
        $buffer.CopyToAsync($other, 2, 0, 2).GetAwaiter().GetResult()
        (Read-WgpuBuffer $other) -join ',' | Should -Be '7,7,0,0,0,0'
        $buffer.ClearAsync().GetAwaiter().GetResult()
        (Read-WgpuBuffer $buffer) -join ',' | Should -Be '0,0,0,0,0,0'
    }

    It 'reads into an array through ReadIntoAsync, checking it before the read' -Skip:$noGpu {
        $buffer = New-WgpuBuffer ([int[]](1..3))
        $values = [int[]]::new(4)
        $read = $buffer.ReadIntoAsync($values)
        $read | Should -BeOfType ([Threading.Tasks.Task])
        $read.GetAwaiter().GetResult()
        $values -join ',' | Should -Be '1,2,3,0'
        { $buffer.ReadIntoAsync([int[]]::new(2)) } | Should -Throw '*WgpuDestination*the read gives 3 elements, and the destination holds 2*'
    }

    It 'reads a slice through ReadAsync' -Skip:$noGpu {
        $buffer = New-WgpuBuffer ([float[]](1..8))
        $buffer.Slice(2, 3).ReadAsync().GetAwaiter().GetResult() -join ',' | Should -Be '3,4,5'
    }

    It 'reads a slice into an array through ReadInto and ReadIntoAsync' -Skip:$noGpu {
        $buffer = New-WgpuBuffer ([float[]](1..8))
        $slice = $buffer.Slice(2, 3)
        $values = [float[]](9, 9, 9, 9)
        $slice.ReadInto($values)
        $values -join ',' | Should -Be '3,4,5,9'
        Write-WgpuBuffer $buffer ([float[]](30, 40)) -Offset 2
        $slice.ReadIntoAsync($values).GetAwaiter().GetResult()
        $values -join ',' | Should -Be '30,40,5,9'
        { $slice.ReadInto([float[]]::new(2)) } | Should -Throw '*WgpuDestination*the read gives 3 elements, and the destination holds 2*'
    }

    It 'waits through WaitAsync and reads through DownloadAsync' -Skip:$noGpu {
        $gpu = Get-WgpuDevice
        $buffer = $gpu.Upload([float[]](1, 2, 3))
        $gpu.WaitAsync().Wait(30000) | Should -BeTrue
        $gpu.DownloadAsync($buffer).GetAwaiter().GetResult() -join ',' | Should -Be '1,2,3'
    }

    It 'faults the task with the error the method would throw' -Skip:$noGpu {
        $gpu = Open-WgpuDevice
        $buffer = New-WgpuBuffer ([float[]](1, 2)) -Device $gpu
        $gpu | Close-WgpuDevice
        $read = $buffer.ReadAsync($null)
        { $read.GetAwaiter().GetResult() } | Should -Throw '*lost*'
        $read.IsFaulted | Should -BeTrue
    }
}

Describe 'Device events and stopping' {
    BeforeAll {
        $script:slow = @'
@group(0) @binding(0) var<storage, read_write> data: array<f32>;
override rounds: u32 = 1u;
@compute @workgroup_size(64)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    var x = data[id.x];
    for (var i = 0u; i < rounds; i++) {
        x = fma(x, 0.9999999, 0.0000001);
    }
    data[id.x] = x;
}
'@
        # Command buffers that keep the default device busy for $Seconds or
        # longer: ten for each second, each one dispatch of 80 ms or more.
        # The dispatch is timed as ten of them queued in one submission, as
        # the tests queue them, so what a submission and its wait cost once,
        # large on a software adapter, is spread over the ten rather than
        # counted as GPU work in each. DispatchMs is that measured time.
        function Build-SlowWork([double]$Seconds) {
            $gpu = Get-WgpuDevice
            $buffer = New-WgpuBuffer -Length 64 -Type Float32 -Device $gpu
            $record = {
                param($pipeline, $group, [int]$count)
                foreach ($i in 1..$count) {
                    $encoder = New-WgpuCommandEncoder -Device $gpu
                    $pass = $encoder.BeginComputePass($null, $null, $null, $null)
                    $pass.SetPipeline($pipeline)
                    $pass.SetBindGroup(0, $group, $null)
                    $pass.Dispatch(1, $null, $null)
                    $pass.End()
                    $encoder.Finish()
                }
            }
            $rounds = 4096
            while ($true) {
                $pipeline = New-WgpuComputePipeline $slow -Constant @{ rounds = $rounds } -Device $gpu
                $group = $pipeline.BindGroup(0, $buffer)
                $gpu.Submit(@(& $record $pipeline $group 1))
                $batch = @(& $record $pipeline $group 10)
                $clock = [Diagnostics.Stopwatch]::StartNew()
                $gpu.Submit($batch)
                $ms = $clock.Elapsed.TotalMilliseconds / 10
                if ($ms -ge 80 -or $rounds -ge [uint32]::MaxValue) { break }
                $rounds = [uint32][math]::Min([uint32]::MaxValue, [math]::Ceiling($rounds * [math]::Min(100 / [math]::Max($ms, 0.001), 64)))
            }
            $commands = @(& $record $pipeline $group ([math]::Ceiling($Seconds * 10)))
            @{ Device = $gpu; Commands = $commands; Rounds = $rounds; DispatchMs = [math]::Round($ms, 1) }
        }

        # Whether a submission's task is still running, then the status it
        # ends with and when, after waiting for it, so a test can say whether
        # the task ended before the work it waits for could have.
        function Get-SubmittedState($Submitted, $Clock) {
            $status = $Submitted.Status
            $now = "was $status at $([int]$Clock.Elapsed.TotalMilliseconds) ms"
            [void][Threading.Tasks.Task]::WhenAny($Submitted, [Threading.Tasks.Task]::Delay(120000)).GetAwaiter().GetResult()
            $ended = "ended $($Submitted.Status) at $([int]$Clock.Elapsed.TotalMilliseconds) ms"
            if ($Submitted.Exception) { $ended += ': ' + $Submitted.Exception.GetBaseException().Message }
            @{ Running = $status -notin 'RanToCompletion', 'Faulted', 'Canceled'; Text = "$now, and $ended" }
        }

        function Clear-DeviceLost {
            Get-Event | Where-Object SourceIdentifier -EQ 'Wgpu.DeviceLost' | Remove-Event
        }
    }

    It 'raises Wgpu.DeviceLost with the device''s label when Close-WgpuDevice destroys it' -Skip:$noGpu {
        $label = "closed-$PID-$(Get-Random)"
        Clear-DeviceLost
        try {
            $gpu = Open-WgpuDevice -Label $label
            $gpu | Close-WgpuDevice
            $lost = Wait-Event -SourceIdentifier Wgpu.DeviceLost -Timeout 10
            $lost.MessageData.Device | Should -Be $label
            $lost.MessageData.Kind | Should -Be 'Destroyed'
            $lost.MessageData.GetType().FullName | Should -Be 'Wgpu.DeviceEvent'
            $gpu.Status().LostReason | Should -Be 'Destroyed'
        } finally {
            Clear-DeviceLost
        }
    }

    It 'raises Wgpu.DeviceLost when the Destroy method destroys a device' -Skip:$noGpu {
        $label = "destroyed-$PID-$(Get-Random)"
        Clear-DeviceLost
        try {
            $gpu = Open-WgpuDevice -Label $label
            $gpu.Destroy()
            (Wait-Event -SourceIdentifier Wgpu.DeviceLost -Timeout 10).MessageData.Device | Should -Be $label
        } finally {
            Clear-DeviceLost
        }
    }

    It 'ends Wait-WgpuDevice when its pipeline stops, while the GPU works on' -Skip:$noGpu {
        $work = Build-SlowWork -Seconds 3
        $go = [Threading.ManualResetEventSlim]::new($false)
        $ps = [powershell]::Create()
        try {
            # The child's wait for $go is bounded and `finally` sets $go, so a
            # failed assertion cannot leave the child waiting. A running
            # child's collections are read by Count only: enumerating one
            # waits for the child to finish.
            $null = $ps.AddScript({ param($module, $go) Import-Module $module; 'ready'; if ($go.Wait(60000)) { Wait-WgpuDevice } }).AddArgument((Join-Path $env:PWRS_MODULE 'Wgpu.psd1')).AddArgument($go)
            $feed = [Management.Automation.PSDataCollection[psobject]]::new()
            $feed.Complete()
            $output = [Management.Automation.PSDataCollection[psobject]]::new()
            $handle = $ps.BeginInvoke($feed, $output)
            $deadline = [DateTime]::UtcNow.AddSeconds(60)
            while ($output.Count -eq 0 -and -not $handle.IsCompleted -and [DateTime]::UtcNow -lt $deadline) { Start-Sleep -Milliseconds 20 }
            $output.Count | Should -Be 1
            $clock = [Diagnostics.Stopwatch]::StartNew()
            $submitted = $work.Device.SubmitAsync($work.Commands)
            $go.Set()
            Start-Sleep -Milliseconds 300
            $ps.Stop()
            $ps.InvocationStateInfo.State | Should -Be 'Stopped'
            $state = Get-SubmittedState $submitted $clock
            $state.Running | Should -BeTrue -Because "the work submitted, $($work.Commands.Count) dispatches measured at $($work.DispatchMs) ms each ($($work.Rounds) rounds), outlasts the stopped wait; its task $($state.Text)"
            $submitted.Wait(120000) | Should -BeTrue
        } finally {
            $go.Set()
            $ps.Dispose()
            $go.Dispose()
        }
    }

    It 'cancels a task whose token is canceled while the GPU works on' -Skip:$noGpu {
        $work = Build-SlowWork -Seconds 3
        $buffer = New-WgpuBuffer ([float[]](1, 2, 3)) -Device $work.Device
        $clock = [Diagnostics.Stopwatch]::StartNew()
        $submitted = $work.Device.SubmitAsync($work.Commands)
        $source = [Threading.CancellationTokenSource]::new()
        try {
            $read = $buffer.ReadAsync($null, $source.Token)
            Start-Sleep -Milliseconds 300
            $source.Cancel()
            { $read.Wait(30000) } | Should -Throw
            $read.IsCanceled | Should -BeTrue
            $state = Get-SubmittedState $submitted $clock
            $state.Running | Should -BeTrue -Because "the work submitted, $($work.Commands.Count) dispatches measured at $($work.DispatchMs) ms each ($($work.Rounds) rounds), outlasts the canceled read; its task $($state.Text)"
            $submitted.Wait(120000) | Should -BeTrue
        } finally {
            $source.Dispose()
        }
    }
}

Describe 'Runspaces' {
    BeforeAll {
        $script:manifest = Join-Path $env:PWRS_MODULE 'Wgpu.psd1'
        $script:double = @'
@group(0) @binding(0) var<storage, read_write> values: array<f32>;
@compute @workgroup_size(64)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    if (id.x < arrayLength(&values)) { values[id.x] = values[id.x] * 2.0; }
}
'@
    }

    It 'keeps the default device when another runspace that imported the module is disposed' -Skip:$noGpu {
        $main = Get-WgpuDevice
        $ps = [powershell]::Create()
        try {
            $null = $ps.AddScript({ param($module) Import-Module $module; (Get-WgpuDevice).Label }).AddArgument($manifest)
            $ps.Invoke() | Should -Be $main.Label
        } finally {
            $ps.Dispose()
        }
        $main.Status().IsDefault | Should -BeTrue
        (Get-WgpuDevice).Label | Should -Be $main.Label
    }

    It 'keeps the default device when the runspaces of a parallel run close' -Skip:$noGpu {
        # pwsh runs ForEach-Object -Parallel, and Windows PowerShell, which
        # has no -Parallel, a runspace pool, as How To Use Threads And
        # Runspaces does in each.
        $main = Get-WgpuDevice
        if ($PSVersionTable.PSVersion.Major -ge 7) {
            $labels = 1..2 | ForEach-Object -Parallel { Import-Module $using:manifest; (Get-WgpuDevice).Label } -ThrottleLimit 2
        } else {
            $pool = [runspacefactory]::CreateRunspacePool(1, 2)
            $pool.Open()
            try {
                $labels = foreach ($i in 1..2) {
                    $shell = [powershell]::Create()
                    $shell.RunspacePool = $pool
                    $null = $shell.AddScript({ param($module) Import-Module $module; (Get-WgpuDevice).Label }).AddArgument($manifest)
                    $shell.Invoke()
                    $shell.Dispose()
                }
            } finally {
                $pool.Dispose()
            }
        }
        $labels | Should -Be @($main.Label, $main.Label)
        $main.Status().IsDefault | Should -BeTrue
        (Get-WgpuDevice).Label | Should -Be $main.Label
    }

    It 'reads a buffer made before another runspace closed through a pipeline made after it' -Skip:$noGpu {
        $buffer = New-WgpuBuffer ([float[]](1..4))
        $ps = [powershell]::Create()
        try {
            $null = $ps.AddScript({ param($module) Import-Module $module; $null = Get-WgpuDevice }).AddArgument($manifest)
            $null = $ps.Invoke()
        } finally {
            $ps.Dispose()
        }
        $pipeline = New-WgpuComputePipeline $double
        Invoke-WgpuCompute $pipeline @{ values = $buffer } -Invocations 4
        (Read-WgpuBuffer $buffer) -join ',' | Should -Be '2,4,6,8'
    }
}

Describe 'Wildcard paths' {
    It 'marks the -Path of each cmdlet that expands wildcards as taking them, and Export-WgpuImage''s as not' {
        foreach ($name in 'Import-WgpuImage', 'New-WgpuShader', 'Test-WgpuShader', 'Get-WgpuShaderInfo', 'ConvertTo-WgpuShader') {
            $path = (Get-Command $name).Parameters['Path']
            @($path.Attributes | Where-Object { $_ -is [System.Management.Automation.SupportsWildcardsAttribute] }).Count | Should -Be 1 -Because "$name expands wildcards in -Path"
            (Get-Help $name -Parameter Path).globbing | Should -Be 'true' -Because "$name's help says -Path takes wildcards"
        }
        $written = (Get-Command Export-WgpuImage).Parameters['Path']
        @($written.Attributes | Where-Object { $_ -is [System.Management.Automation.SupportsWildcardsAttribute] }).Count | Should -Be 0 -Because 'Export-WgpuImage names the file it writes as written'
    }
}
