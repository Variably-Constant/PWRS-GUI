# The structures that keep history or reach values by something other
# than a place.
BeforeAll {
    . (Join-Path $PSScriptRoot 'Common.ps1')
    Import-SubEthaModule
    $script:dir = New-SubEthaScratch 'versioned'
}

AfterAll {
    Remove-SubEthaScratch $script:dir
}

Describe 'SubEtha.Reservoir' {
    It 'keeps a bounded sample' {
        $r = New-SubEthaReservoir -Path (Join-Path $script:dir 'reservoir') -Capacity 4
        $r.MaxValueBytes | Should -Be 52
        $r.Record('a') | Should -Be 0
        $r.RecordMany(@('b', 'c', 'd')) | Should -Be 3
        $r.Count() | Should -Be 4
        $null = $r.RecordMany((1..50 | ForEach-Object { "x$_" }))
        $r.Count() | Should -Be 4
        $r.TotalSeen() | Should -Be 54
        $r.Snapshot().Count | Should -Be 4
        $r.Reset()
        $r.TotalSeen() | Should -Be 0
        $r.Flush()
        $r.FlushAsync()
        $r.Dispose()
    }
}

Describe 'SubEtha.HandleTable' {
    It 'names values by handles that do not survive removal' {
        $t = New-SubEthaHandleTable -Path (Join-Path $script:dir 'handles') -Capacity 2
        $t.MaxValueBytes | Should -Be 44
        $h = $t.Insert('first')
        $t.Contains($h) | Should -BeTrue
        Get-SEText $t.Get($h) | Should -Be 'first'
        Get-SEText $t.Remove($h) | Should -Be 'first'
        $t.Contains($h) | Should -BeFalse
        $null -eq $t.Get($h) | Should -BeTrue
        $handles = $t.InsertMany(@('a', 'b', 'c'))
        $handles.Count | Should -Be 2
        $many = $t.GetMany(@($handles[0], $h))
        Get-SEText $many[0] | Should -Be 'a'
        $null -eq $many[1] | Should -BeTrue
        $t.Count() | Should -Be 2
        { $t.Insert('d') } | Should -Throw
        $t.Dispose()
    }
}

Describe 'SubEtha.TimePointTile' {
    It 'shows a reader only what existed at its version' {
        $t = New-SubEthaTimePointTile -Path (Join-Path $script:dir 'tile')
        $t.Lanes | Should -Be 16
        $l1 = $t.Insert(10, 'ten')
        $l2 = $t.Insert(20, 'twenty')
        $t.Count() | Should -Be 2
        $t.VisibleCount(15) | Should -Be 1
        $t.VisibleCount(25) | Should -Be 2
        ($t.VisibleMask(15) -band (1 -shl $l1)) | Should -Not -Be 0
        $seen = $t.Visible(15)
        $seen.Count | Should -Be 1
        $seen[0].GetType().FullName | Should -Be 'SubEtha.TileEntry'
        Get-SEText $seen[0].Bytes | Should -Be 'ten'
        $seen[0].Version | Should -Be 10
        $at = $t.At($l2)
        $at.Version | Should -Be 20
        $t.Remove($l2)
        $null -eq $t.At($l2) | Should -BeTrue
        $t.IsFull() | Should -BeFalse
        { $t.At(99) } | Should -Throw
        $t.Dispose()
    }
}

Describe 'SubEtha.VersionChain' {
    It 'keeps every version and answers as of a version' {
        $c = New-SubEthaVersionChain -Path (Join-Path $script:dir 'chain') -Capacity 4
        $null -eq $c.Current() | Should -BeTrue
        $c.Push(1, 'one')
        $c.Push(5, 'five')
        $c.Count() | Should -Be 2
        Get-SEText $c.ReadAt(3) | Should -Be 'one'
        Get-SEText $c.ReadAt(9) | Should -Be 'five'
        $null -eq $c.ReadAt(0) | Should -BeTrue
        $cur = $c.Current()
        $cur.Version | Should -Be 5
        { $c.Push(2, 'late') } | Should -Throw
        $c.Clear()
        $c.Count() | Should -Be 0
        $c.Dispose()
    }
}

Describe 'SubEtha.VersionedSlab' {
    It 'keeps slot history and reads through a pin' {
        $slab = New-SubEthaVersionedSlab -Path (Join-Path $script:dir 'vslab') -Capacity 4 -EpochsPath (Join-Path $script:dir 'vslab-epochs')
        $slab.Depth | Should -Be 4
        $slab.Set(0, 'a')
        $pin = $slab.Pin()
        $pin.GetType().FullName | Should -Be 'SubEtha.SlabPin'
        $pin.Held() | Should -BeTrue
        $pin.Epoch() | Should -BeGreaterThan 0
        $slab.Set(0, 'b')
        Get-SEText $slab.Get(0) | Should -Be 'b'
        Get-SEText $pin.Get(0) | Should -Be 'a'
        $many = $pin.GetMany(@(0, 1))
        Get-SEText $many[0] | Should -Be 'a'
        $null -eq $many[1] | Should -BeTrue
        $history = $slab.History(0)
        $history.Count | Should -Be 2
        $history[0].GetType().FullName | Should -Be 'SubEtha.SlotVersion'
        $null -eq $history[0].Died | Should -BeTrue
        $history[1].Died | Should -Not -BeNullOrEmpty
        $pin.Release()
        $pin.Held() | Should -BeFalse
        { $pin.Get(0) } | Should -Throw
        Get-SEText $slab.Retire(0) | Should -Be 'b'
        $null -eq $slab.Get(0) | Should -BeTrue
        $slab.SweepSlot(0) | Should -BeGreaterOrEqual 0
        $slab.VoidEpoch(999999) | Should -Be 0
        $slab.Flush()
        $slab.Dispose()
    }

    It 'writes at named epochs and undoes one of them' {
        $slab = New-SubEthaVersionedSlab -Path (Join-Path $script:dir 'vslab-at') -Capacity 4 -EpochsPath (Join-Path $script:dir 'vslab-at-epochs')
        $slab.SetAt(0, 'old', 1)
        $slab.SetAt(1, 'old', 1)
        $slab.SetAt(0, 'new', 2)
        $slab.SetAt(1, 'new', 2)
        Get-SEText $slab.Get(0) | Should -Be 'new'
        $slab.VoidEpoch(2) | Should -BeGreaterOrEqual 4
        Get-SEText $slab.Get(0) | Should -Be 'old'
        Get-SEText $slab.Get(1) | Should -Be 'old'
        Get-SEText $slab.RetireAt(1, 7) | Should -Be 'old'
        $null -eq $slab.Get(1) | Should -BeTrue
        $slab.Dispose()
    }
}

Describe 'SubEtha.VersionedMap' {
    It 'scans one unchanging view through a pin' {
        $m = New-SubEthaVersionedMap -Path (Join-Path $script:dir 'vmap') -Capacity 64 -EpochsPath (Join-Path $script:dir 'vmap-epochs')
        $null -eq $m.Insert(1, 10) | Should -BeTrue
        $m.Insert(1, 11) | Should -Be 10
        $m.InsertMany(@([uint64] 2, [uint64] 3), @([uint64] 20, [uint64] 30)).Count | Should -Be 2
        $m.Get(2) | Should -Be 20
        $null -eq $m.Get(9) | Should -BeTrue
        $pin = $m.Pin()
        $pin.GetType().FullName | Should -Be 'SubEtha.MapPin'
        $m.Insert(4, 40) | Out-Null
        $m.Remove(1) | Should -Be 11
        $pin.Get(1) | Should -Be 11
        $null -eq $pin.Get(4) | Should -BeTrue
        $scan = $pin.Scan($null, $null, 100)
        $scan[0].GetType().FullName | Should -Be 'SubEtha.Entry'
        ($scan | ForEach-Object { $_.Key }) | Should -Be @(1, 2, 3)
        $from = $pin.ScanFrom(2, $null, 100)
        $from.GetType().FullName | Should -Be 'SubEtha.Scan'
        ($from.Entries | ForEach-Object { $_.Value }) | Should -Be @(20, 30)
        $pin.Release()
        $m.Sweep() | Should -BeGreaterOrEqual 0
        $m.Count() | Should -BeGreaterOrEqual 3
        $m.Flush()
        $m.Dispose()
    }

    It 'inserts at named epochs and undoes one of them' {
        $m = New-SubEthaVersionedMap -Path (Join-Path $script:dir 'vmap-at') -Capacity 64 -EpochsPath (Join-Path $script:dir 'vmap-at-epochs')
        $null -eq $m.InsertAt(1, 10, 1) | Should -BeTrue
        $null -eq $m.InsertAt(2, 20, 2) | Should -BeTrue
        $m.Get(2) | Should -Be 20
        $m.VoidEpoch(2) | Should -BeGreaterOrEqual 1
        $null -eq $m.Get(2) | Should -BeTrue
        $m.Get(1) | Should -Be 10
        $m.RemoveAt(1, 3) | Should -Be 10
        $m.Dispose()
    }
}

Describe 'SubEtha.LanedMap' {
    It 'writes through lanes and reads across them' {
        $m = New-SubEthaLanedMap -Directory (Join-Path $script:dir 'laned') -Lanes 2
        $m.Lanes | Should -Be 2
        $claim = $m.ClaimLane()
        $claim.GetType().FullName | Should -Be 'SubEtha.LaneClaim'
        $claim.Held() | Should -BeTrue
        $claim.Lanes() | Should -Be 2
        $null -eq $claim.Insert(7, 70) | Should -BeTrue
        $claim.InsertMany(@([uint64] 8), @([uint64] 80)).Count | Should -Be 1
        $m.HeldLanes() | Should -Be 1
        $m.Get(7) | Should -Be 70
        $m.LaneOf(7) | Should -Be $claim.Index()
        $other = $m.ClaimLane()
        { $other.Remove(7) } | Should -Throw
        $other.Release()
        $claim.Remove(8) | Should -Be 80
        $claim.Release()
        $m.HeldLanes() | Should -Be 0
        $again = $m.ClaimLaneFor(7)
        $again.Index() | Should -Be $m.LaneOf(7)
        $again.Release()
        { $m.ClaimLaneFor(12345) } | Should -Throw
        $pin = $m.Pin()
        $pin.GetType().FullName | Should -Be 'SubEtha.LanedPin'
        ($pin.Scan($null, $null, 100) | ForEach-Object { $_.Key }) | Should -Be @(7)
        $pin.Get(7) | Should -Be 70
        $pin.ScanFrom($null, $null, 100).Entries.Count | Should -Be 1
        $pin.Release()
        $m.ReapDeadClaims() | Should -BeGreaterOrEqual 0
        $m.Sweep() | Should -BeGreaterOrEqual 0
        $m.Count() | Should -BeGreaterOrEqual 1
        $m.Flush()
        $m.Dispose()
    }
}

Describe 'SubEtha.VersionedIndex' {
    It 'keeps every version a pin can still see' {
        $index = New-SubEthaVersionedIndex -Path (Join-Path $script:dir 'vindex') -EpochsPath (Join-Path $script:dir 'vindex-epochs') -ChunkBits 6
        $index.GetType().FullName | Should -Be 'SubEtha.VersionedIndex'
        $pin = $index.Pin()
        $pin.GetType().FullName | Should -Be 'SubEtha.IndexPin'
        $ticket = $index.Begin()
        $ticket.GetType().FullName | Should -Be 'SubEtha.IndexTicket'
        foreach ($k in 0..9) { $index.Write([uint64] $k, [uint64] (10 * $k), $ticket, $pin) }
        $ticket.Publish(); $pin.Release()
        $before = $index.Pin()
        $pin = $index.Pin(); $ticket = $index.Begin()
        $index.Write(1, 11, $ticket, $pin)
        $ticket.Publish(); $pin.Release()
        $index.GetAt(1, $before, $null) | Should -Be 10
        $before.Release()
        $after = $index.Pin()
        $index.GetAt(1, $after, $null) | Should -Be 11
        $null -eq $index.GetAt(99, $after, $null) | Should -BeTrue
        $after.Held() | Should -BeTrue
        $after.Release()
        $after.Held() | Should -BeFalse
        { New-SubEthaVersionedIndex -Path (Join-Path $script:dir 'vindex-bad') -EpochsPath (Join-Path $script:dir 'vindex-bad-epochs') -ChunkBits 3 -ErrorAction Stop } |
            Should -Throw
        $index.Dispose()
    }

    It 'refuses a write a later version beat, naming its epoch, and takes back what the statement wrote' {
        $index = New-SubEthaVersionedIndex -Path (Join-Path $script:dir 'vindex-c') -EpochsPath (Join-Path $script:dir 'vindex-c-epochs') -ChunkBits 6
        $pin = $index.Pin(); $ticket = $index.Begin()
        $index.Write(1, 10, $ticket, $pin)
        $index.Write(2, 20, $ticket, $pin)
        $ticket.Publish(); $pin.Release()
        $stale = $index.Pin()
        $pin = $index.Pin(); $ticket = $index.Begin()
        $won = $ticket.Epoch()
        $index.Write(1, 11, $ticket, $pin)
        $ticket.Publish(); $pin.Release()
        $late = $index.Begin()
        { $index.Write(1, 12, $late, $stale) } | Should -Throw -ExpectedMessage "*epoch $won, after the pin*"
        $index.Write(2, 22, $late, $stale)
        $index.GetAt(2, $stale, $late) | Should -Be 22
        $index.GetAt(2, $stale, $null) | Should -Be 20
        $index.Void(2, $late.Epoch()) | Should -BeTrue
        $index.Void(2, $late.Epoch()) | Should -BeFalse
        $index.Write(3, 30, $late, $stale)
        $index.Delete(2, $late, $stale)
        $null -eq $index.GetAt(2, $stale, $late) | Should -BeTrue
        $index.VoidEpoch($late.Epoch()) | Should -Be 2
        $late.Publish(); $stale.Release()
        $late.Held() | Should -BeFalse
        { $late.Epoch() } | Should -Throw
        $now = $index.Pin()
        $index.GetAt(1, $now, $null) | Should -Be 11
        $index.GetAt(2, $now, $null) | Should -Be 20
        $null -eq $index.GetAt(3, $now, $null) | Should -BeTrue
        $now.Release()
        $index.Dispose()
    }

    It 'walks a range a page at a time and reports what changed since an epoch' {
        $index = New-SubEthaVersionedIndex -Path (Join-Path $script:dir 'vindex-r') -EpochsPath (Join-Path $script:dir 'vindex-r-epochs') -ChunkBits 6
        $pin = $index.Pin(); $ticket = $index.Begin()
        foreach ($k in 0..9) { $index.Write([uint64] $k, [uint64] (100 + $k), $ticket, $pin) }
        $ticket.Held() | Should -BeTrue
        $ticket.Publish(); $pin.Release()
        $pin = $index.Pin()
        $keys = @(); $low = $null; $pages = 0
        do {
            $page = $index.RangeAt($pin, $low, $null, 3, $null)
            $page.GetType().FullName | Should -Be 'SubEtha.Scan'
            $keys += @($page.Entries | ForEach-Object { $_.Key })
            $pages++
            if ($null -ne $page.ResumeFrom) { $low = [uint64] ($page.ResumeFrom + 1) }
        } while ($null -ne $page.ResumeFrom -and $pages -lt 16)
        $keys | Should -Be (0..9)
        $pages | Should -Be 4
        $index.RangeAt($pin, 2, 4, $null, $null).Entries.Count | Should -Be 3
        $index.RangeAt($pin, 7, 3, $null, $null).Entries.Count | Should -Be 0
        $pin.Release()
        $since = $index.Now()
        $pin = $index.Pin(); $ticket = $index.Begin()
        $mid = $index.Now()
        $index.Write(7, 70, $ticket, $pin)
        $index.ChangedSince($pin, $mid, 7, 7, $ticket) | Should -BeFalse
        $ticket.Publish(); $pin.Release()
        $pin = $index.Pin()
        $index.ChangedSince($pin, $since, 0, 9, $null) | Should -BeTrue
        $index.ChangedSince($pin, $since, 8, 9, $null) | Should -BeFalse
        $pin.Release()
        $index.VersionCount() | Should -Be 11
        $index.Sweep() | Should -Be 1
        $index.VersionCount() | Should -Be 10
        $index.LiveCount() | Should -Be 10
        $index.Dispose()
    }

    It 'removes its files and leaves the epochs' {
        $p = Join-Path $script:dir 'vindex-remove'
        $e = Join-Path $script:dir 'vindex-remove-epochs'
        $index = New-SubEthaVersionedIndex -Path $p -EpochsPath $e -ChunkBits 6
        $pin = $index.Pin(); $ticket = $index.Begin()
        foreach ($k in 0..99) { $index.Write([uint64] $k, [uint64] ($k * 10), $ticket, $pin) }
        $ticket.Publish(); $pin.Release()
        $index.Dispose()
        $named = @(Get-ChildItem -LiteralPath $script:dir | Where-Object { $_.Name -eq 'vindex-remove' -or $_.Name.StartsWith('vindex-remove.') })
        $named.Count | Should -BeGreaterOrEqual 2 -Because 'an index is its main file and the chunk files beside it'
        Remove-SubEthaVersionedIndex -Path $p -WhatIf | Should -BeNullOrEmpty
        Test-Path -LiteralPath $p | Should -BeTrue -Because '-WhatIf removes nothing'
        $report = Remove-SubEthaVersionedIndex -Path $p
        $report.Removed | Should -Be $named.Count
        $report.Missing | Should -Be 0
        @(Get-ChildItem -LiteralPath $script:dir | Where-Object { $_.Name -eq 'vindex-remove' -or $_.Name.StartsWith('vindex-remove.') }).Count | Should -Be 0
        Test-Path -LiteralPath $e | Should -BeTrue -Because 'the epochs file stays for whoever else stamps through it'
        (Remove-SubEthaVersionedIndex -Path $p).Missing | Should -Be 1
    }
}

Describe 'SubEtha.TopologyMap' {
    It 'reads the shape off the traffic' {
        $t = New-SubEthaTopologyMap -Path (Join-Path $script:dir 'topology') -Participants 4
        $t.Participants | Should -Be 4
        $t.RecordSend(0, 1) | Should -Be 1
        $t.RecordMany(@([uint32] 0, [uint32] 0), @([uint32] 2, [uint32] 3)) | Should -Be 2
        $t.FanOut(0) | Should -Be 3
        $t.FanIn(1) | Should -Be 1
        $t.TotalSends() | Should -Be 3
        $busy = $t.BusiestSender()
        $busy.GetType().FullName | Should -Be 'SubEtha.FanCount'
        $busy.Participant | Should -Be 0
        $busy.Places | Should -Be 3
        $t.BusiestReceiver().Places | Should -Be 1
        $shape = $t.Recommend()
        $shape.GetType().FullName | Should -Be 'SubEtha.Topology'
        $t.PublishRecommendation() | Should -Be $shape
        $t.PublishedRecommendation() | Should -Be $shape
        $t.RecommendationEpoch() | Should -BeGreaterThan 0
        $t.BroadcastRoot() | Should -BeGreaterOrEqual 0
        $t.Dispose()
    }
}

Describe 'SubEtha.Graph' {
    It 'adds nodes and edges and walks them' {
        $g = New-SubEthaGraph -Path (Join-Path $script:dir 'graph') -MaxNodes 8 -MaxEdges 8
        $a = $g.AddNode(100)
        $b = $g.AddNode(200)
        $more = $g.AddNodes(@([uint64] 300, [uint64] 400))
        $more.Count | Should -Be 2
        $e = $g.AddEdge($a, $b, 5)
        $g.AddEdges(@([uint32] $a), @([uint32] $more[0]), @([uint64] 6)).Count | Should -Be 1
        $g.NodeCount() | Should -Be 4
        $g.EdgeCount() | Should -Be 2
        $g.NodeValue($b) | Should -Be 200
        $null -eq $g.NodeValue(99) | Should -BeTrue
        $g.OutDegree($a) | Should -Be 2
        $n = $g.Neighbors($a)
        $n.Count | Should -Be 2
        $n[0].GetType().FullName | Should -Be 'SubEtha.Neighbor'
        ($n | Where-Object { $_.Edge -eq $e }).Value | Should -Be 5
        $g.RemoveEdge($a, $e) | Should -Be 5
        $g.OutDegree($a) | Should -Be 1
        $g.Flush()
        $g.FlushAsync()
        $g.Dispose()
    }
}

Describe 'SubEtha.Universal' {
    It 'stores itself as a list and moves to a map' {
        $u = New-SubEthaUniversal -Path (Join-Path $script:dir 'universal') -Capacity 64
        $u.Strategy() | Should -Be ([SubEtha.SetStrategy]::List)
        $u.Insert(1) | Should -BeTrue
        $u.Insert(1) | Should -BeFalse -Because 'a set holds a value once'
        $u.InsertMany(@([uint64] 2, [uint64] 3)) | Should -Be 2
        $u.Contains(2) | Should -BeTrue
        $u.ContainsMany(@([uint64] 3, [uint64] 9)) | Should -Be @($true, $false)
        $u.Count() | Should -Be 3
        ($u.Snapshot() | Sort-Object) | Should -Be @(1, 2, 3)
        $v = $u.Version()
        $u.MigrateTo([SubEtha.SetStrategy]::Map)
        $u.Strategy() | Should -Be ([SubEtha.SetStrategy]::Map)
        $u.Version() | Should -BeGreaterThan $v
        $u.Contains(1) | Should -BeTrue
        $u.Insert(1) | Should -BeFalse -Because 'a value carried across is not new on the map'
        $counts = $u.OpCounts()
        $counts.GetType().FullName | Should -Be 'SubEtha.OpCounts'
        $counts.Inserts | Should -BeGreaterOrEqual 3
        $u.Generation() | Should -BeGreaterOrEqual 0
        $v = $u.Version()
        $u.Clear()
        $u.Count() | Should -Be 0
        $u.Version() | Should -BeGreaterThan $v -Because 'a clear steps the version as a migration does'
        $u.Strategy() | Should -Be ([SubEtha.SetStrategy]::Map)
        $u.Insert(1) | Should -BeTrue -Because 'a value cleared away is new again'
        $u.Dispose()
    }

    It 'counts only the values that were new, and removes its files' {
        $p = Join-Path $script:dir 'universal-remove'
        $u = New-SubEthaUniversal -Path $p -Capacity 64
        $u.InsertMany(@([uint64] 1, [uint64] 2)) | Should -Be 2
        $u.InsertMany(@([uint64] 2, [uint64] 3, [uint64] 3)) | Should -Be 1
        $u.MigrateTo([SubEtha.SetStrategy]::Map)
        $u.Dispose()
        Remove-SubEthaUniversal -Path $p -WhatIf | Should -BeNullOrEmpty
        Test-Path -LiteralPath "$p.state.bin" | Should -BeTrue -Because '-WhatIf removes nothing'
        $report = Remove-SubEthaUniversal -Path $p
        $report.GetType().FullName | Should -Be 'SubEtha.UnlinkReport'
        $report.Removed | Should -Be 3 -Because 'a set is its state file and one table for each way of storing it'
        $report.Missing | Should -Be 0
        Test-Path -LiteralPath "$p.state.bin" | Should -BeFalse
        $again = Remove-SubEthaUniversal -Path $p
        $again.Removed | Should -Be 0
        $again.Missing | Should -Be 1
    }
}

Describe 'SubEtha.Tower' {
    It 'reaches values by a path that checks itself' {
        $t = New-SubEthaTower -Path (Join-Path $script:dir 'tower-bottom') -Capacity 8 -ValueSize 4 -LevelPath @((Join-Path $script:dir 'tower-top')) -LevelCapacity @(8)
        $t.Depth | Should -Be 2
        $t.ValueSize | Should -Be 4
        $path = $t.Append('abcd')
        $path.Count | Should -Be 2
        Get-SEText $t.Get($path) | Should -Be 'abcd'
        $paths = $t.AppendMany(@('efgh', 'ijkl'))
        $paths.Count | Should -Be 2
        $got = $t.GetMany($paths)
        Get-SEText $got[1] | Should -Be 'ijkl'
        $top = $t.InsertAtTop(5, 'mnop')
        $top[0] | Should -Be 5
        Get-SEText $t.Get($top) | Should -Be 'mnop'
        $t.Count() | Should -Be 4
        $wrong = [uint32[]] @($path[0], $path[1] + 1)
        { $t.Get($wrong) } | Should -Throw
        $t.Flush()
        $t.Dispose()
    }
}
