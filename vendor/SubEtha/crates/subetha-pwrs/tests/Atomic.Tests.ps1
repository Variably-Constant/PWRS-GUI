# SubEtha.Atomic, SubEtha.AtomicU32 and SubEtha.AtomicBool: a 64-bit
# integer, a 32-bit integer and a flag in a file every process maps.
# PWRS_MODULE points at the built module folder.
BeforeAll {
    $module = $env:PWRS_MODULE
    if (-not $module) { throw 'PWRS_MODULE is not set' }
    Import-Module (Join-Path $module 'SubEtha.psd1') -Force -ErrorAction Stop
    $script:dir = Join-Path ([System.IO.Path]::GetTempPath()) ('subetha-ps-atomic-' + [guid]::NewGuid().ToString('n'))
    New-Item -ItemType Directory -Path $script:dir | Out-Null
}

AfterAll {
    [GC]::Collect()
    [GC]::WaitForPendingFinalizers()
    # A BeforeAll that failed before making the directory leaves it unset,
    # and Get-ChildItem of a null path lists the current directory.
    if ($script:dir -and (Test-Path -LiteralPath $script:dir)) {
        Get-ChildItem -LiteralPath $script:dir -File | Remove-Item -Force -ErrorAction SilentlyContinue
        Remove-Item -LiteralPath $script:dir -Force -ErrorAction SilentlyContinue
    }
}

Describe 'SubEtha.Atomic' {
    It 'creates the file holding Init and attaches to the live value' {
        $path = Join-Path $script:dir 'counter'
        $a = New-SubEthaAtomic -Path $path -Init 7
        $a.GetType().FullName | Should -Be 'SubEtha.Atomic'
        $a.Path | Should -Be $path
        $a.Load() | Should -Be 7
        $b = Open-SubEthaAtomic -Path $path
        $b.Load() | Should -Be 7
        $null = $a.FetchAdd(3)
        $b.Load() | Should -Be 10
        $a.Dispose()
        $b.Dispose()
    }

    It 'leaves the value alone when the file exists' {
        $path = Join-Path $script:dir 'existing'
        $a = New-SubEthaAtomic -Path $path -Init 5
        $b = New-SubEthaAtomic -Path $path -Init 99
        $b.Load() | Should -Be 5
        $a.Dispose()
        $b.Dispose()
    }

    It 'refuses to open a file that does not exist' {
        { Open-SubEthaAtomic -Path (Join-Path $script:dir 'missing') -ErrorAction Stop } | Should -Throw
    }

    It 'remakes the file under -Reset whatever it held, at the width asked for' {
        # Windows refuses to remake a file any handle still maps, so each
        # handle is disposed before the next reset.
        $path = Join-Path $script:dir 'remade'
        $a = New-SubEthaAtomic -Path $path -Init 41
        $null = $a.FetchAdd(1)
        $a.Dispose()
        $b = New-SubEthaAtomic -Path $path -Init 5 -Reset
        $b.Load() | Should -Be 5
        $b.Dispose()
        $c = New-SubEthaAtomicU32 -Path $path -Init 9 -Reset
        $c.Load() | Should -Be 9
        $c.Dispose()
        $d = New-SubEthaAtomicBool -Path $path -Init -Reset
        $d.Load() | Should -BeTrue
        $d.Dispose()
        $e = Open-SubEthaAtomicBool -Path $path
        $e.Load() | Should -BeTrue
        $e.Dispose()
    }

    It 'resolves a relative path against the current location' {
        Push-Location $script:dir
        try {
            $a = New-SubEthaAtomic -Path '.\relative' -Init 1
            $a.Path | Should -Be (Join-Path $script:dir 'relative')
            $a.Dispose()
        } finally {
            Pop-Location
        }
    }

    It 'returns the value before each read-modify-write' {
        $a = New-SubEthaAtomic -Path (Join-Path $script:dir 'rmw') -Init 10
        $a.FetchAdd() | Should -Be 10
        $a.FetchAdd(5) | Should -Be 11
        $a.FetchSub(1) | Should -Be 16
        $a.FetchOr(0xF0) | Should -Be 15
        $a.FetchAnd(0x0F) | Should -Be 255
        $a.FetchXor(0xFF) | Should -Be 15
        $a.Swap(42) | Should -Be 240
        $a.Load() | Should -Be 42
        $a.Load().GetType().Name | Should -Be 'UInt64'
        $a.Dispose()
    }

    It 'wraps round at the top and the bottom' {
        $a = New-SubEthaAtomic -Path (Join-Path $script:dir 'wrap') -Init ([uint64]::MaxValue)
        $null = $a.FetchAdd(1)
        $a.Load() | Should -Be 0
        $null = $a.FetchSub(1)
        $a.Load() | Should -Be ([uint64]::MaxValue)
        $a.Dispose()
    }

    It 'compares and exchanges, answering the value either way' {
        $a = New-SubEthaAtomic -Path (Join-Path $script:dir 'cas') -Init 1
        $a.CompareExchange(1, 2) | Should -Be 1
        $a.Load() | Should -Be 2
        $a.CompareExchange(1, 3) | Should -Be 2
        $a.Load() | Should -Be 2
        $a.Dispose()
    }

    It 'runs a batch of adds for one call' {
        $a = New-SubEthaAtomic -Path (Join-Path $script:dir 'batch') -Init 100
        $a.FetchAddMany(10) | Should -Be 100
        $a.Load() | Should -Be 110
        $a.FetchAddMany(3, 5) | Should -Be 110
        $a.Load() | Should -Be 125
        $a.Dispose()
    }

    It 'takes a memory order as the enum or as its name' {
        $a = New-SubEthaAtomic -Path (Join-Path $script:dir 'order') -Init 3
        $a.Load([SubEtha.MemoryOrder]::Relaxed) | Should -Be 3
        $a.Load('Acquire') | Should -Be 3
        $a.Store(4, 'Release')
        $a.Load() | Should -Be 4
        $a.FetchAdd(1, [SubEtha.MemoryOrder]::AcqRel) | Should -Be 4
        { $a.Load('Sideways') } | Should -Throw
        $a.Dispose()
    }

    It 'refuses an order a load or a store cannot take, naming the ones it can' {
        $a = New-SubEthaAtomic -Path (Join-Path $script:dir 'orders')
        { $a.Load('Release') } | Should -Throw -ExpectedMessage '*a load cannot be Release; use Relaxed, Acquire or SeqCst*'
        { $a.Load('AcqRel') } | Should -Throw -ExpectedMessage '*a load cannot be AcqRel*'
        { $a.Store(1, 'Acquire') } | Should -Throw -ExpectedMessage '*a store cannot be Acquire; use Relaxed, Release or SeqCst*'
        { $a.Store(1, 'AcqRel') } | Should -Throw -ExpectedMessage '*a store cannot be AcqRel*'
        $a.Load() | Should -Be 0 -Because 'a refused store writes nothing'
        $a.Dispose()
        $b = New-SubEthaAtomicU32 -Path (Join-Path $script:dir 'orders32')
        { $b.Load('Release') } | Should -Throw -ExpectedMessage '*a load cannot be Release*'
        { $b.Store(1, 'Acquire') } | Should -Throw -ExpectedMessage '*a store cannot be Acquire*'
        $b.Dispose()
        $c = New-SubEthaAtomicBool -Path (Join-Path $script:dir 'orders-flag')
        { $c.Load('AcqRel') } | Should -Throw -ExpectedMessage '*a load cannot be AcqRel*'
        { $c.Store($true, 'AcqRel') } | Should -Throw -ExpectedMessage '*a store cannot be AcqRel*'
        $c.Swap($true, 'AcqRel') | Should -BeFalse -Because 'a read-modify-write takes every order'
        $c.Dispose()
    }

    It 'answers to the SE alias' {
        $path = Join-Path $script:dir 'alias'
        $a = New-SEAtomic -Path $path -Init 8
        $b = Open-SEAtomic -Path $path
        $b.Load() | Should -Be 8
        $a.Dispose()
        $b.Dispose()
    }

    It 'refuses a call after Dispose' {
        $a = New-SubEthaAtomic -Path (Join-Path $script:dir 'disposed')
        $a.Dispose()
        $a.IsDisposed | Should -BeTrue
        { $a.Load() } | Should -Throw
    }

    It 'lists the operations through Get-Member' {
        $a = New-SubEthaAtomic -Path (Join-Path $script:dir 'members')
        $names = ($a | Get-Member -MemberType Method).Name
        foreach ($n in 'Load', 'Store', 'FetchAdd', 'FetchSub', 'FetchOr', 'FetchAnd', 'FetchXor', 'Swap', 'CompareExchange', 'FetchAddMany', 'Dispose') {
            $names | Should -Contain $n
        }
        $a.Dispose()
    }
}

Describe 'SubEtha.AtomicU32' {
    It 'creates the file holding Init and attaches to the live value' {
        $path = Join-Path $script:dir 'counter32'
        $a = New-SubEthaAtomicU32 -Path $path -Init 7
        $a.GetType().FullName | Should -Be 'SubEtha.AtomicU32'
        $a.Path | Should -Be $path
        $a.Load() | Should -Be 7
        $a.Load().GetType().Name | Should -Be 'UInt32'
        $b = Open-SubEthaAtomicU32 -Path $path
        $null = $a.FetchAdd(3)
        $b.Load() | Should -Be 10
        $a.Dispose()
        $b.Dispose()
    }

    It 'returns the value before each read-modify-write' {
        $a = New-SubEthaAtomicU32 -Path (Join-Path $script:dir 'rmw32') -Init 10
        $a.FetchAdd() | Should -Be 10
        $a.FetchAdd(5) | Should -Be 11
        $a.FetchSub(1) | Should -Be 16
        $a.FetchOr(0xF0) | Should -Be 15
        $a.FetchAnd(0x0F) | Should -Be 255
        $a.FetchXor(0xFF) | Should -Be 15
        $a.Swap(42) | Should -Be 240
        $a.CompareExchange(42, 43) | Should -Be 42
        $a.CompareExchange(42, 44) | Should -Be 43
        $a.Load() | Should -Be 43
        $a.Dispose()
    }

    It 'wraps round at thirty-two bits' {
        $a = New-SubEthaAtomicU32 -Path (Join-Path $script:dir 'wrap32') -Init ([uint32]::MaxValue)
        $a.FetchAdd(1) | Should -Be ([uint32]::MaxValue)
        $a.Load() | Should -Be 0
        $null = $a.FetchSub(1)
        $a.Load() | Should -Be ([uint32]::MaxValue)
        $a.Dispose()
    }

    It 'refuses a file a 64-bit atomic laid out, naming both widths' {
        $p = Join-Path $script:dir 'wide'
        $a = New-SubEthaAtomic -Path $p -Init 1
        { Open-SubEthaAtomicU32 -Path $p -ErrorAction Stop } |
            Should -Throw -ExpectedMessage "*$p*the file's width is 8 where 4 was asked for*"
        { New-SubEthaAtomicU32 -Path $p -ErrorAction Stop } |
            Should -Throw -ExpectedMessage "*$p*the file's width is 8 where 4 was asked for*"
        $a.Dispose()
    }

    It 'answers to the SE alias' {
        $path = Join-Path $script:dir 'alias32'
        $a = New-SEAtomicU32 -Path $path -Init 8
        $b = Open-SEAtomicU32 -Path $path
        $b.Load() | Should -Be 8
        $a.Dispose()
        $b.Dispose()
    }
}

Describe 'SubEtha.AtomicBool' {
    It 'starts clear and is seen set by every handle' {
        $path = Join-Path $script:dir 'flag'
        $a = New-SubEthaAtomicBool -Path $path
        $a.GetType().FullName | Should -Be 'SubEtha.AtomicBool'
        $a.Path | Should -Be $path
        $a.Load() | Should -BeFalse
        $b = Open-SubEthaAtomicBool -Path $path
        $a.Store($true)
        $b.Load() | Should -BeTrue
        $b.Load().GetType().Name | Should -Be 'Boolean'
        $a.Dispose()
        $b.Dispose()
    }

    It 'tells the one caller that set it by what Swap returns' {
        $a = New-SubEthaAtomicBool -Path (Join-Path $script:dir 'once')
        $a.Swap($true) | Should -BeFalse
        $a.Swap($true) | Should -BeTrue
        $a.Load('Acquire') | Should -BeTrue
        $a.Dispose()
    }

    It 'creates the flag set under Init and leaves a live flag alone' {
        $path = Join-Path $script:dir 'kept'
        $a = New-SubEthaAtomicBool -Path $path -Init
        $a.Load() | Should -BeTrue
        $b = New-SubEthaAtomicBool -Path $path
        $b.Load() | Should -BeTrue
        $a.Dispose()
        $b.Dispose()
    }

    It 'refuses to open a file that does not exist' {
        { Open-SubEthaAtomicBool -Path (Join-Path $script:dir 'no-flag') -ErrorAction Stop } |
            Should -Throw -ExpectedMessage '*cannot obtain the flag*'
    }

    It 'answers to the SE alias' {
        $path = Join-Path $script:dir 'alias-flag'
        $a = New-SEAtomicBool -Path $path
        $b = Open-SEAtomicBool -Path $path
        $b.Load() | Should -BeFalse
        $a.Dispose()
        $b.Dispose()
    }
}
