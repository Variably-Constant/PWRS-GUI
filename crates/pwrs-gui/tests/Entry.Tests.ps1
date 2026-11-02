# Another module drawing through PWRS-GUI's session: the stand-in module,
# built beside this one, takes the session of either renderer through
# Get-GuiNativeEntry, in every mode the platform has, and the process
# has one renderer of each. See Common.ps1 for the deadlines.

BeforeDiscovery {
    $modes = if ($IsWindows -or $PSVersionTable.PSEdition -eq 'Desktop') { 'Helper', 'InProcess' } else { 'Helper' }
    $cases = foreach ($m in $modes) { foreach ($r in 'Egui', 'Html') { @{ Mode = $m; Renderer = $r } } }
}

BeforeAll {
    . (Join-Path $PSScriptRoot 'Common.ps1')
    if (-not (Test-Path -LiteralPath $script:StandinModule)) {
        throw "The stand-in module is not built at $($script:StandinModule); build it with cargo pwrs build --manifest-dir crates/pwrs-gui-standin and the same profile as this module"
    }
    $script:Watchdog = Start-Watchdog
}

AfterAll {
    if (Get-Module PwrsGuiStandin) { Remove-Module PwrsGuiStandin }
    Stop-Watchdog $script:Watchdog
}

Describe 'Get-GuiNativeEntry' {
    BeforeAll {
        Reset-GuiSession Helper
    }

    It 'returns the addresses, the ABI version and the protocol, without starting a session' {
        $entry = Get-GuiNativeEntry
        $entry.GetType().FullName | Should -Be 'PwrsGui.NativeEntry'
        $entry.AbiVersion | Should -Be 1
        # The protocol is the number the protocol crate declares, which the
        # module, its helpers and the entry all report.
        $declared = Select-String -LiteralPath (Join-Path $PSScriptRoot '../../pwrs-gui-proto/src/lib.rs') -Pattern '^pub const PROTOCOL: u32 = (\d+);' | Select-Object -First 1
        $declared | Should -Not -BeNullOrEmpty -Because 'the protocol crate declares its PROTOCOL'
        $entry.Protocol | Should -Be ([uint32]$declared.Matches[0].Groups[1].Value)
        foreach ($address in $entry.StartV1, $entry.WatchV1, $entry.UnwatchV1, $entry.SendV1) { $address | Should -BeGreaterThan 0 }
        $entry.SessionRunning | Should -BeFalse
        (Get-GuiNativeEntry).StartV1 | Should -Be $entry.StartV1
    }
}

Describe 'A module taking the <Renderer> session in <Mode> mode' -ForEach $cases {
    BeforeAll {
        $script:Mode = $Mode
        $script:Html = if ($Renderer -eq 'Html') { ' -Html' } else { '' }
        Reset-GuiSession $Mode $Renderer
        Import-Module $script:StandinModule
    }

    It 'starts the session when none runs, and it is the module''s own' {
        $taken = Get-StandinSession -Html:($script:Html -ne '')
        $taken | Should -BeLike "* $script:Mode"
        (Get-GuiNativeEntry).SessionRunning | Should -BeTrue
        (Get-GuiOption).Running | Should -Be $script:Mode
    }

    It 'opens a window the module''s commands list and drive, and gets its answer' {
        $job = Start-Background "Read-StandinChoice -Choice 'red','green','blue'$($script:Html)"
        $w = Wait-GuiWindow 'PWRS-GUI stand-in' $job
        $w.Names | Should -Be @('Choice', 'Ok')
        Invoke-GuiTestEvent -Window $w.Id -Name Choice -Event Select -Value 1
        Invoke-GuiTestEvent -Window $w.Id -Name Ok -Event Click
        $done = Complete-Background $job
        $done.Errors | Should -BeNullOrEmpty
        $done.Output | Should -Be @('green')
    }

    It 'draws beside the module''s own windows in one renderer' {
        $job = Start-Background "Read-StandinChoice -Choice 'one','two'$($script:Html)"
        Wait-GuiWindow 'PWRS-GUI stand-in' $job | Out-Null
        1..2 | Out-GuiGridView -Title 'PWRS-GUI test beside'
        Wait-GuiWindow 'PWRS-GUI test beside' | Out-Null
        $expected = if ($script:Mode -eq 'Helper') { 1 } else { 0 }
        @(Get-OwnHelpers).Count | Should -Be $expected
        $done = Stop-Background $job
        $done.Output | Should -BeNullOrEmpty
    }
}
