# Show-GuiProgress and Set-GuiProgress in every mode the platform has and
# with each renderer: a window the script updates, one a SubEtha ring
# fills, and the checks of their arguments, which open no window. See
# Common.ps1 for the deadlines.

BeforeDiscovery {
    $modes = if ($IsWindows -or $PSVersionTable.PSEdition -eq 'Desktop') { 'Helper', 'InProcess' } else { 'Helper' }
    $cases = foreach ($m in $modes) { foreach ($r in 'Egui', 'Html') { @{ Mode = $m; Renderer = $r } } }
}

BeforeAll {
    . (Join-Path $PSScriptRoot 'Common.ps1')
    $script:Watchdog = Start-Watchdog
    # A property of the widget $Name in window $Id, as its renderer has it.
    function Get-Shown($Id, $Name, $Property) { (Find-GuiNode (Get-GuiSnapshot -Window $Id).Tree $Name).Properties.$Property }
}

AfterAll {
    Stop-Watchdog $script:Watchdog
}

Describe 'Show-GuiProgress and Set-GuiProgress arguments' {
    It 'refuses a -PercentComplete outside 0 to 100' {
        { Show-GuiProgress 'Copying' -PercentComplete 101 } | Should -Throw -ErrorId 'GuiBadPercent*'
        { Set-GuiProgress 1 -PercentComplete -1 } | Should -Throw -ErrorId 'GuiBadPercent*'
    }

    It 'refuses a -Window that is neither a progress window nor a number' {
        { Set-GuiProgress 'not a window' -Status 'x' } | Should -Throw -ErrorId 'GuiNoWindow*'
    }

    It 'does nothing for a window that is not open' {
        Set-GuiProgress 999999 -Status 'nobody sees this' | Should -BeNullOrEmpty
    }
}

Describe 'A progress window in <Mode> mode drawn with <Renderer>' -ForEach $cases {
    BeforeAll {
        Reset-GuiSession $Mode $Renderer
    }

    It 'shows the activity, and its status over the bar inside a dialog''s margin, takes each update, and closes when completed' {
        $p = Show-GuiProgress 'Copying files' -Status 'Starting' -PercentComplete 10 -Title 'PWRS-GUI test progress'
        $p.PSObject.TypeNames[0] | Should -Be 'PwrsGui.ProgressWindow'
        $w = Get-GuiWindow -Title 'PWRS-GUI test progress'
        $w.Id | Should -Be $p.Id -Because 'the command returns once its window has opened, which is when a window is listed'
        $p.State | Should -Be 'Open'
        Get-Shown $w.Id Activity text | Should -Be 'Copying files'
        Get-Shown $w.Id Progress text | Should -Be 'Starting' -Because 'the status is the bar''s text, over it with how much is done'
        Get-Shown $w.Id Progress value | Should -Be 10
        Wait-Until { (Get-GuiSnapshot -Window $w.Id).Bounds.Contains('Activity') } 'the activity being drawn'
        [math]::Abs((Get-GuiSnapshot -Window $w.Id).Bounds['Activity'].X - 12) | Should -BeLessOrEqual 1 -Because 'a progress window is drawn inside a dialog''s 12-point margin'
        Set-GuiProgress $p -Status 'Half way' -PercentComplete 50 -Activity 'Moving files'
        Wait-Until { (Get-Shown $w.Id Progress value) -eq 50 } 'the bar moving to 50'
        Get-Shown $w.Id Progress text | Should -Be 'Half way'
        Get-Shown $w.Id Activity text | Should -Be 'Moving files'
        $p | Set-GuiProgress -Completed
        $p.State | Should -Be 'Closed' -Because 'Set-GuiProgress -Completed returns once the window has closed'
        Wait-Until { -not (Get-GuiWindow -Title 'PWRS-GUI test progress') } 'the window closing'
    }

    It 'takes no update once closed, its State reading Closed, so the script runs on' {
        $p = Show-GuiProgress 'Waiting' -Title 'PWRS-GUI test progress closed'
        $w = Wait-GuiWindow 'PWRS-GUI test progress closed'
        Close-GuiWindow -Window $w.Id
        Wait-Until { $p.State -eq 'Closed' } 'State reading Closed'
        { Set-GuiProgress $p -Status 'after' -PercentComplete 90 } | Should -Not -Throw
        { Set-GuiProgress $p -Completed } | Should -Not -Throw
    }

    It 'fills the bar from a SubEtha ring another process writes' {
        Import-Module (Join-Path $env:SUBETHA_MODULE 'SubEtha.psd1') -ErrorAction Stop
        $base = Join-Path ([IO.Path]::GetTempPath()) "pwrs-gui-test-progress-ring-$PID-$Mode-$Renderer"
        $null = New-Item -ItemType Directory -Path $base -Force
        $ring = New-SubEthaRing -Path (Join-Path $base 'copy') -Capacity 64
        try {
            $p = Show-GuiProgress 'Copying from a ring' -Ring $ring -Title 'PWRS-GUI test progress ring'
            $w = Wait-GuiWindow 'PWRS-GUI test progress ring'
            $producer = $ring.RegisterProducer()
            $ring.SendFrame($producer, '{"Value": 42, "Text": "copying"}') | Should -BeTrue
            Wait-Until { (Get-Shown $w.Id Progress value) -eq 42 } 'the bar moving to 42'
            Get-Shown $w.Id Progress text | Should -Be 'copying'
            Set-GuiProgress $p -Completed
            $p.State | Should -Be 'Closed' -Because 'Set-GuiProgress -Completed returns once the window has closed'
        } finally {
            $ring.Dispose()
            [GC]::Collect()
            [GC]::WaitForPendingFinalizers()
            Wait-Until { Remove-Item -LiteralPath $base -Recurse -Force -ErrorAction Ignore; -not (Test-Path -LiteralPath $base) } 'the ring''s folder being removed once the window let go of it'
        }
    }
}
