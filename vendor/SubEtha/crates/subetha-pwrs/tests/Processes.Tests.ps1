# A second process of the same host attaches to the files this one
# made, which is what the whole library is for.
BeforeAll {
    . (Join-Path $PSScriptRoot 'Common.ps1')
    Import-SubEthaModule
    $script:dir = New-SubEthaScratch 'processes'
    $script:exe = [System.Diagnostics.Process]::GetCurrentProcess().MainModule.FileName
    $script:manifest = Join-Path $env:PWRS_MODULE 'SubEtha.psd1'

    # Runs $Body in a fresh process of the host running this suite, with
    # the module imported first, and answers its output lines as one
    # array, so a single line still indexes as a line and not as its
    # characters. A child that fails, fails the test with its own output.
    function Invoke-SEChild {
        param([string] $Body)
        $file = Join-Path $script:dir ('child-' + [guid]::NewGuid().ToString('n') + '.ps1')
        $text = "`$ErrorActionPreference = 'Stop'`r`nImport-Module '$($script:manifest)'`r`n$Body"
        [System.IO.File]::WriteAllText($file, $text)
        $out = @(& $script:exe -NoProfile -NonInteractive -ExecutionPolicy Bypass -File $file 2>&1 | ForEach-Object { "$_" })
        if ($LASTEXITCODE -ne 0) { throw "the child process failed: $($out -join [Environment]::NewLine)" }
        Write-Output -NoEnumerate $out
    }
}

AfterAll {
    Remove-SubEthaScratch $script:dir
}

Describe 'two processes on one file' {
    It 'share a counter' {
        $p = Join-Path $script:dir 'hits'
        $a = New-SubEthaAtomic -Path $p -Init 10
        $out = Invoke-SEChild "`$c = Open-SubEthaAtomic -Path '$p'; `$null = `$c.FetchAdd(5); Write-Output `$c.Load(); `$c.Dispose()"
        [int] $out[-1] | Should -Be 15
        $a.Load() | Should -Be 15
        $a.Dispose()
    }

    It 'send items through a ring the other made' {
        $p = Join-Path $script:dir 'ring'
        $ring = New-SubEthaRing -Path $p -Capacity 16 -MaxProducers 2 -MaxConsumers 2
        $c = $ring.RegisterConsumer()
        $out = Invoke-SEChild "`$r = Open-SubEthaRing -Path '$p' -Capacity 16 -MaxProducers 2 -MaxConsumers 2; `$prod = `$r.RegisterProducer(); Write-Output `$r.SendMany(`$prod, @('one', 'two', 'three')); `$r.Dispose()"
        [int] $out[-1] | Should -Be 3
        ($ring.RecvMany($c, 10) | ForEach-Object { ConvertFrom-SEBytes $_ }) -join ',' | Should -Be 'one,two,three'
        $ring.Dispose()
    }

    It 'pass a channel item and read it back with a wait' {
        $p = Join-Path $script:dir 'channel'
        $ch = New-SubEthaChannel -Path $p -Capacity 8
        $null = Invoke-SEChild "`$c = Open-SubEthaChannel -Path '$p' -Capacity 8; if (-not `$c.Send('from the child')) { throw 'full' }; `$c.Dispose()"
        Get-SEText $ch.RecvFor(2) | Should -Be 'from the child'
        $ch.Dispose()
    }

    It 'see a role the other claims, by process id' {
        $p = Join-Path $script:dir 'leader'
        $election = New-SubEthaLeaderElection -Path $p
        $out = Invoke-SEChild "`$e = Open-SubEthaLeaderElection -Path '$p'; if (-not `$e.TryClaim()) { throw 'refused' }; Write-Output `$PID; `$e.Dispose()"
        $childPid = [int] $out[-1]
        $election.Leader() | Should -Be $childPid
        $election.IsLeader() | Should -BeFalse
        $election.StepDown($childPid) | Should -BeTrue
        $null -eq $election.Leader() | Should -BeTrue
        $election.Dispose()
    }

    It 'add to one set, each value new to exactly one of them' {
        $p = Join-Path $script:dir 'set'
        $set = New-SubEthaUniversal -Path $p -Capacity 256
        $set.InsertMany([uint64[]] (0..49)) | Should -Be 50
        $out = Invoke-SEChild "`$s = Open-SubEthaUniversal -Path '$p' -Capacity 256; Write-Output `$s.InsertMany([uint64[]] (25..99)); `$s.Dispose()"
        [int] $out[-1] | Should -Be 50 -Because 'the child added 75 values and 25 of them were already there'
        $set.Count() | Should -Be 100
        $set.Insert(99) | Should -BeFalse
        $set.Dispose()
    }

    It 'publish and subscribe across the boundary' {
        $p = Join-Path $script:dir 'pubsub'
        $pub = New-SubEthaPubSub -Path $p -Capacity 8
        $sub = $pub.Subscribe()
        $null = Invoke-SEChild "`$q = Open-SubEthaPubSub -Path '$p' -Capacity 8; `$null = `$q.PublishMany(@('a', 'b')); `$q.Dispose()"
        ($sub.NextMany(10) | ForEach-Object { Get-SEText $_ }) -join '' | Should -Be 'ab'
        $sub.Dispose()
        $pub.Dispose()
    }

    It 'take back the epoch of a versioned index writer that died mid-statement' {
        $p = Join-Path $script:dir 'vindex'
        $e = Join-Path $script:dir 'vindex-epochs'
        $index = New-SubEthaVersionedIndex -Path $p -EpochsPath $e -ChunkBits 6
        $pin = $index.Pin(); $ticket = $index.Begin()
        $index.Write(1, 10, $ticket, $pin)
        $ticket.Publish(); $pin.Release()
        # The child ends its own process with its ticket open, which is what
        # a writer that dies does: nothing is left to publish it. It runs
        # outside Invoke-SEChild, whose children must exit cleanly.
        $file = Join-Path $script:dir ('child-' + [guid]::NewGuid().ToString('n') + '.ps1')
        $body = @(
            "`$ErrorActionPreference = 'Stop'"
            "Import-Module '$($script:manifest)'"
            "`$index = Open-SubEthaVersionedIndex -Path '$p' -EpochsPath '$e'"
            "`$pin = `$index.Pin()"
            "`$ticket = `$index.Begin()"
            "foreach (`$k in 0..19) { `$index.Write([uint64] `$k, [uint64] (5000 + `$k), `$ticket, `$pin) }"
            "[Console]::Out.WriteLine('written'); [Console]::Out.Flush()"
            "[System.Diagnostics.Process]::GetCurrentProcess().Kill()"
        ) -join "`r`n"
        [System.IO.File]::WriteAllText($file, $body)
        $out = @(& $script:exe -NoProfile -NonInteractive -ExecutionPolicy Bypass -File $file 2>&1 | ForEach-Object { "$_" })
        $out -contains 'written' | Should -BeTrue -Because ($out -join [Environment]::NewLine)
        $dead = @($index.DeadTickets())
        $dead.Count | Should -Be 1
        $pin = $index.Pin()
        $pin.Epoch() | Should -BeLessThan $dead[0]
        $null -eq $index.GetAt(2, $pin, $null) | Should -BeTrue
        $index.GetAt(1, $pin, $null) | Should -Be 10
        $pin.Release()
        $index.VoidEpoch($dead[0]) | Should -Be 20
        $index.FreeDeadTicket($dead[0]) | Should -BeTrue
        $pin = $index.Pin(); $ticket = $index.Begin()
        $index.Write(50, 500, $ticket, $pin)
        $ticket.Publish(); $pin.Release()
        $pin = $index.Pin()
        $pin.Epoch() | Should -BeGreaterThan $dead[0]
        $null -eq $index.GetAt(2, $pin, $null) | Should -BeTrue
        $index.GetAt(1, $pin, $null) | Should -Be 10
        $pin.Release()
        $index.LiveCount() | Should -Be 2
        $index.Dispose()
    }
}
