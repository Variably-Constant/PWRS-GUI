# The single values and plain containers: Region, Cell, Vec, SharedArc,
# LazyValue and BitVec.
BeforeAll {
    . (Join-Path $PSScriptRoot 'Common.ps1')
    Import-SubEthaModule
    $script:dir = New-SubEthaScratch 'primitives'
}

AfterAll {
    Remove-SubEthaScratch $script:dir
}

Describe 'SubEtha.Region' {
    It 'allocates slots and reads them back' {
        $r = New-SubEthaRegion -Path (Join-Path $script:dir 'region') -Capacity 8 -SlotSize 5
        $r.GetType().FullName | Should -Be 'SubEtha.Region'
        $r.Capacity | Should -Be 8
        $r.SlotSize | Should -Be 5
        $r.Count() | Should -Be 0
        $i = $r.Allocate((ConvertTo-SEBytes 'hello'))
        $i | Should -Be 0
        $r.Count() | Should -Be 1
        Get-SEText $r.Get($i) | Should -Be 'hello'
        $r.Set($i, 'world')
        Get-SEText $r.Get($i) | Should -Be 'world'
        $r.Dispose()
    }

    It 'reports the layout it was made with, which another handle opens with' {
        $r = New-SubEthaRegion -Path (Join-Path $script:dir 'region-layout') -Capacity 4 -SlotSize 8 -Alignment 16 -Tag 0x52454731
        $r.SlotSize | Should -Be 8
        $r.Alignment | Should -Be 16
        $r.Tag | Should -Be 0x52454731
        $again = Open-SubEthaRegion -Path $r.Path -Capacity $r.Capacity -SlotSize $r.SlotSize -Alignment $r.Alignment -Tag $r.Tag
        $again.Alignment | Should -Be 16
        $again.Tag | Should -Be 0x52454731
        { Open-SubEthaRegion -Path $r.Path -Capacity $r.Capacity -SlotSize $r.SlotSize -Alignment $r.Alignment -ErrorAction Stop } |
            Should -Throw
        $plain = New-SubEthaRegion -Path (Join-Path $script:dir 'region-plain') -Capacity 4 -SlotSize 8
        $plain.Alignment | Should -Be 1
        $plain.Tag | Should -Be 0
        $plain.Dispose()
        $again.Dispose()
        $r.Dispose()
    }

    It 'snapshots the whole span in one array' {
        $r = New-SubEthaRegion -Path (Join-Path $script:dir 'snap') -Capacity 4 -SlotSize 8
        $null = $r.Allocate('abcdefgh')
        $null = $r.Allocate('12345678')
        $all = $r.Snapshot()
        $all.Length | Should -Be 32
        ConvertFrom-SEBytes $all[0..15] | Should -Be 'abcdefgh12345678'
        $r.Dispose()
    }

    It 'refuses an item that is not exactly a slot' {
        $r = New-SubEthaRegion -Path (Join-Path $script:dir 'long') -Capacity 2 -SlotSize 4
        { $r.Allocate('too long for four') } | Should -Throw
        { $r.Allocate('abc') } | Should -Throw
        $r.Dispose()
    }

    It 'answers to the SE alias' {
        $r = New-SERegion -Path (Join-Path $script:dir 'alias') -Capacity 2 -SlotSize 4
        $r.Capacity | Should -Be 2
        $r.Dispose()
    }
}

Describe 'SubEtha.Cell' {
    It 'steps its version on every write' {
        $path = Join-Path $script:dir 'cell'
        $c = New-SubEthaCell -Path $path -ValueSize 8
        $c.ValueSize | Should -Be 8
        $v0 = $c.Version()
        $c.Set('abcdefgh')
        $c.Version() | Should -BeGreaterThan $v0
        Get-SEText $c.Get() | Should -Be 'abcdefgh'
        $other = Open-SubEthaCell -Path $path -ValueSize 8
        Get-SEText $other.Get() | Should -Be 'abcdefgh'
        $c.Flush()
        $c.Dispose()
        $other.Dispose()
    }

    It 'counts each write once, and Load answers the version with the value' {
        $path = Join-Path $script:dir 'cell-load'
        $c = New-SubEthaCell -Path $path -ValueSize 4
        $first = $c.Load()
        $first.GetType().FullName | Should -Be 'SubEtha.CellReading'
        $first.Version | Should -Be 0
        $c.Set('abcd') | Should -Be 1
        $read = $c.Load()
        $read.Version | Should -Be 1
        Get-SEText $read.Value | Should -Be 'abcd'
        $c.Dispose()
    }

    It 'lands CompareAndSet only on the version it saw' {
        $path = Join-Path $script:dir 'cell-cas'
        $a = New-SubEthaCell -Path $path -ValueSize 4
        $b = Open-SubEthaCell -Path $path -ValueSize 4
        $seen = $a.Load().Version
        $won = $b.CompareAndSet($seen, 'won!')
        $won.GetType().FullName | Should -Be 'SubEtha.CellWrite'
        $won.Landed | Should -BeTrue
        $won.Version | Should -Be ($seen + 1)
        $lost = $a.CompareAndSet($seen, 'lost')
        $lost.Landed | Should -BeFalse
        $lost.Version | Should -Be ($seen + 1)
        Get-SEText $a.Get() | Should -Be 'won!'
        $a.Dispose()
        $b.Dispose()
    }
}

Describe 'SubEtha.Vec' {
    It 'pushes, pops, gets and sets' {
        $v = New-SubEthaVec -Path (Join-Path $script:dir 'vec') -Capacity 4 -ElementSize 4 -Writers 2
        $v.Capacity | Should -Be 4
        $v.Writers | Should -Be 2
        $v.ElementSize | Should -Be 4
        $v.Writable | Should -BeTrue
        $v.Push('aaaa') | Should -Be 0
        $v.Push('bbbb') | Should -Be 1
        $v.Count() | Should -Be 2
        Get-SEText $v.Get(1) | Should -Be 'bbbb'
        $null -eq $v.Get(7) | Should -BeTrue
        $v.Set(0, 'cccc')
        Get-SEText $v.Pop() | Should -Be 'bbbb'
        Get-SEText $v.Pop() | Should -Be 'cccc'
        $null -eq $v.Pop() | Should -BeTrue
        $v.Dispose()
    }

    It 'answers null when full and counts a batch' {
        $v = New-SubEthaVec -Path (Join-Path $script:dir 'full') -Capacity 2 -ElementSize 2 -Writers 1
        $v.PushMany(@([byte[]](1, 2), [byte[]](3, 4), [byte[]](5, 6))) | Should -Be 2
        $null -eq $v.Push('xx') | Should -BeTrue
        $v.Clear()
        $v.Count() | Should -Be 0
        $v.Dispose()
    }

    It 'moves ranges packed end to end' {
        $v = New-SubEthaVec -Path (Join-Path $script:dir 'range') -Capacity 8 -ElementSize 2 -Writers 1
        $null = $v.PushMany(@('ab', 'cd', 'ef', 'gh'))
        $packed = $v.ReadRange(1, 2)
        ConvertFrom-SEBytes $packed | Should -Be 'cdef'
        $v.WriteRange(0, (ConvertTo-SEBytes 'wxyz')) | Should -Be 2
        ConvertFrom-SEBytes $v.ReadRange(0, 2) | Should -Be 'wxyz'
        { $v.WriteRange(0, (ConvertTo-SEBytes 'abc')) } | Should -Throw
        $v.Dispose()
    }

    It 'reports the layout it was made with, which another handle opens with' {
        $v = New-SubEthaVec -Path (Join-Path $script:dir 'vec-layout') -Capacity 4 -ElementSize 8 -Writers 3 -Alignment 16 -Tag 0x56454354
        $v.ElementSize | Should -Be 8
        $v.Alignment | Should -Be 16
        $v.Tag | Should -Be 0x56454354
        $again = Open-SubEthaVec -Path $v.Path -Capacity $v.Capacity -ElementSize $v.ElementSize -Writers $v.Writers -Alignment $v.Alignment -Tag $v.Tag
        $again.Alignment | Should -Be 16
        $again.Tag | Should -Be 0x56454354
        { Open-SubEthaVec -Path $v.Path -Capacity $v.Capacity -ElementSize $v.ElementSize -Writers $v.Writers -Alignment $v.Alignment -ErrorAction Stop } |
            Should -Throw
        { Open-SubEthaVec -Path $v.Path -Capacity $v.Capacity -ElementSize $v.ElementSize -Writers 4 -Alignment $v.Alignment -Tag $v.Tag -ErrorAction Stop } |
            Should -Throw -ExpectedMessage "*writer slots*"
        $plain = New-SubEthaVec -Path (Join-Path $script:dir 'vec-plain') -Capacity 4 -ElementSize 8 -Writers 1
        $plain.Alignment | Should -Be 1
        $plain.Tag | Should -Be 0
        $plain.Dispose()
        $again.Dispose()
        $v.Dispose()
    }
}

Describe 'SubEtha.SharedArc' {
    It 'shares a value between holders and counts them' {
        $path = Join-Path $script:dir 'arc'
        $a = New-SubEthaSharedArc -Path $path -Value 'shared!!' -KeepOnLast
        $a.ValueSize | Should -Be 8
        $a.Holders() | Should -Be 1
        $b = Open-SubEthaSharedArc -Path $path -ValueSize 8 -KeepOnLast
        $a.Holders() | Should -Be 2
        Get-SEText $b.Get() | Should -Be 'shared!!'
        $b.WriteAt(0, 'SH')
        Get-SEText $a.ReadAt(0, 2) | Should -Be 'SH'
        $b.Dispose()
        $a.Holders() | Should -Be 1
        $a.Dispose()
    }

    It 'refuses a range past the end of the value, naming it' {
        $a = New-SubEthaSharedArc -Path (Join-Path $script:dir 'arc-range') -Value 'four'
        { $a.ReadAt(3, 2) } | Should -Throw '*2 bytes from offset 3 run past the 4-byte value*'
        { $a.WriteAt(3, 'xy') } | Should -Throw '*run past the 4-byte value*'
        Get-SEText $a.ReadAt(2, 2) | Should -Be 'ur'
        $a.Dispose()
    }
}

Describe 'SubEtha.LazyValue' {
    It 'is claimed, published and read' {
        $l = New-SubEthaLazyValue -Path (Join-Path $script:dir 'lazy') -ValueSize 4
        $l.Ready() | Should -BeFalse
        $null -eq $l.Get() | Should -BeTrue
        $l.Claim() | Should -BeTrue
        $l.Claim() | Should -BeFalse
        { $l.Publish('toolong') } | Should -Throw
        $l.Publish('done') | Should -BeTrue
        $l.Ready() | Should -BeTrue
        Get-SEText $l.Get() | Should -Be 'done'
        $l.Claim() | Should -BeFalse
        $l.Dispose()
    }

    It 'takes over a claim whose process is gone' {
        $l = New-SubEthaLazyValue -Path (Join-Path $script:dir 'lazy-gone') -ValueSize 4
        # A pid this high is not a live process on any of the gate hosts.
        $l.Claim(2147483632) | Should -BeTrue
        $null -eq $l.Get() | Should -BeTrue
        $l.Claim() | Should -BeTrue
        $l.Publish('done') | Should -BeTrue
        Get-SEText $l.Get() | Should -Be 'done'
        $l.Dispose()
    }
}

Describe 'SubEtha.BitVec' {
    It 'sets, clears, toggles and reads bits' {
        $b = New-SubEthaBitVec -Path (Join-Path $script:dir 'bits') -CapacityBits 128
        $b.CapacityBits | Should -Be 128
        $b.Get(5) | Should -BeFalse
        $b.Set(5) | Should -BeFalse
        $b.Get(5) | Should -BeTrue
        $b.Set(5) | Should -BeTrue
        $b.Clear(5) | Should -BeTrue
        $b.Toggle(70) | Should -BeTrue
        $b.Toggle(70) | Should -BeFalse
        $b.SetRange(10, 14)
        $b.Get(13) | Should -BeTrue
        $b.Get(14) | Should -BeFalse
        { $b.Get(1000) } | Should -Throw
        $b.Dispose()
    }
}
