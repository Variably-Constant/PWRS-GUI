# Measure-GuiLatency and PwrsGui.Clock, with clicks a test sends, in every
# mode the platform has and with each renderer. Times are asserted as
# orderings, which are true on any machine, never as absolute figures. See
# Common.ps1 for the deadlines.

BeforeDiscovery {
    $modes = if ($IsWindows -or $PSVersionTable.PSEdition -eq 'Desktop') { 'Helper', 'InProcess' } else { 'Helper' }
    $cases = foreach ($m in $modes) { foreach ($r in 'Egui', 'Html') { @{ Mode = $m; Renderer = $r } } }
}

BeforeAll {
    . (Join-Path $PSScriptRoot 'Common.ps1')
    $script:Watchdog = Start-Watchdog
}

AfterAll {
    Stop-Watchdog $script:Watchdog
}

Describe 'Measure-GuiLatency in <Mode> mode drawn with <Renderer>' -ForEach $cases {
    BeforeAll {
        Reset-GuiSession $Mode $Renderer
    }

    It 'writes one record per click, each place reached after the one before' {
        $job = Start-Background 'Measure-GuiLatency -Count 2'
        $w = Wait-GuiWindow 'PWRS-GUI latency' $job
        Invoke-GuiTestEvent -Window $w.Id -Name Click -Event Click
        Invoke-GuiTestEvent -Window $w.Id -Name Click -Event Click
        $done = Complete-Background $job
        $done.Errors | Should -BeNullOrEmpty
        $done.Output.Count | Should -Be 2
        $done.Output.Click | Should -Be @(1, 2)
        foreach ($r in $done.Output) {
            $r.GetType().FullName | Should -Be 'PwrsGui.Latency'
            $r.ToModuleUs | Should -BeGreaterThan 0
            $r.ToHandlerUs | Should -BeGreaterThan $r.ToModuleUs
            $r.ToPipelineUs | Should -BeGreaterThan $r.ToHandlerUs
            $r.ToEventUs | Should -BeNullOrEmpty
        }
    }

    It 'with -NoWait, returns once its window has opened and records each click''s engine-event action' {
        $run = Measure-GuiLatency -NoWait -Count 2
        $run.GetType().FullName | Should -Be 'PwrsGui.LatencyRun'
        Get-EventSubscriber -SourceIdentifier $run.SourceIdentifier | Should -Not -BeNullOrEmpty
        $w = Get-GuiWindow -Title 'PWRS-GUI latency'
        $w.Id | Should -Be $run.Window -Because 'the command returns once its window has opened, which is when a window is listed'
        Invoke-GuiTestEvent -Window $w.Id -Name Click -Event Click
        Invoke-GuiTestEvent -Window $w.Id -Name Click -Event Click
        Wait-Until { [void](Wait-Event -Timeout 1); $run.Samples().Count -ge 2 } 'both clicks'' actions running'
        $samples = $run.Samples()
        $samples.Click | Should -Be @(1, 2)
        foreach ($s in $samples) {
            $s.ToEventUs | Should -BeGreaterThan $s.ToModuleUs
            $s.ToHandlerUs | Should -BeNullOrEmpty
        }
        Unregister-Event -SourceIdentifier $run.SourceIdentifier
        Wait-Until { -not (Get-GuiWindow -Title 'PWRS-GUI latency') } 'the window closing after its last click'
    }
}

Describe 'PwrsGui.Clock' {
    BeforeAll {
        if (-not (Get-Module PWRS-GUI)) { Import-Module $script:GuiModule }
    }

    It 'reads forward in nanoseconds, on the clock events are stamped with' {
        $first = [PwrsGui.Clock]::Now()
        Start-Sleep -Milliseconds 20
        $second = [PwrsGui.Clock]::Now()
        $second | Should -BeGreaterThan $first
        ($second - $first) | Should -BeGreaterOrEqual 20000000
    }
}
