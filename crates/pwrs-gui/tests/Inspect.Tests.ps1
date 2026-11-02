# The commands that look at windows and drive them, the options, and the
# session's end, in every mode the platform has and with each renderer.
# See Common.ps1 for the deadlines.

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

Describe 'Inspecting and driving windows in <Mode> mode drawn with <Renderer>' -ForEach $cases {
    BeforeAll {
        $script:Mode = $Mode
        $script:Renderer = $Renderer
        Reset-GuiSession $Mode $Renderer
    }

    It 'lists no window before any opens, without starting a renderer' {
        @(Get-GuiWindow) | Should -BeNullOrEmpty
        (Get-GuiOption).Running | Should -BeNullOrEmpty
    }

    It 'lists a window opened in another runspace, by title wildcard' {
        1..2 | Out-GuiGridView -Title 'PWRS-GUI test listed'
        $w = Wait-GuiWindow 'PWRS-GUI test listed'
        $w.GetType().FullName | Should -Be 'PwrsGui.Window'
        $w.Names | Should -Be @('Grid')
        @(Get-GuiWindow -Title 'pwrs-gui TEST list*').Id | Should -Contain $w.Id
        @(Get-GuiWindow -Title 'no such window*') | Should -BeNullOrEmpty
    }

    It 'refuses an event on a widget the window lacks, naming the widgets it has' {
        $w = Wait-GuiWindow 'PWRS-GUI test listed'
        { Invoke-GuiTestEvent -Window $w.Id -Name Missing -Event Click -ErrorAction Stop } | Should -Throw '*Missing*'
    }

    It 'takes a window''s pixels and a widget''s, the widget''s smaller' {
        $w = Wait-GuiWindow 'PWRS-GUI test listed'
        $whole = Get-GuiSnapshot -Window $w.Id -Pixels
        $whole.Width | Should -BeGreaterThan 0
        $whole.Height | Should -BeGreaterThan 0
        $whole.Pixels.Length | Should -Be ($whole.Width * $whole.Height * 4)
        $grid = Get-GuiSnapshot -Window $w.Id -Name Grid
        $grid.Pixels.Length | Should -Be ($grid.Width * $grid.Height * 4)
        ($grid.Width * $grid.Height) | Should -BeLessThan ($whole.Width * $whole.Height)
    }

    It 'gives the state without pixels unless -Pixels is given' {
        $w = Wait-GuiWindow 'PWRS-GUI test listed'
        $shot = Get-GuiSnapshot -Window $w.Id
        $shot.Pixels | Should -BeNullOrEmpty
        $shot.State.Grid.rows | Should -Be 2
        $shot.Tree.Kind | Should -Be 'Column'
        @($shot.Tree.Keys) | Should -Be @('Id', 'Kind', 'Name', 'Properties', 'Children')
        $grid = Find-GuiNode $shot.Tree 'Grid'
        $names = @($grid.Properties.Keys)
        $names | Should -Be @($names | Sort-Object)
        @($shot.State.Grid.Keys) | Should -Be @('filter', 'rows', 'selected', 'shown')
    }

    It 'lists the adapters, one of them in use' {
        $adapters = @(Get-GuiGpuAdapter)
        $adapters.Count | Should -BeGreaterThan 0
        @($adapters | Where-Object InUse).Count | Should -Be 1
    }

    It 'reports the running renderer in Get-GuiOption' {
        $o = Get-GuiOption
        $o.Mode | Should -Be $script:Mode
        $o.Running | Should -Be $script:Mode
        $o.Backend | Should -Not -BeNullOrEmpty
        $o.AdapterInUse | Should -Not -BeNullOrEmpty
    }

    It 'leaves every window open when another runspace that imported the module closes' {
        $w = Wait-GuiWindow 'PWRS-GUI test listed'
        $other = Complete-Background (Start-Background "@(Get-GuiWindow -Title 'PWRS-GUI test listed').Id")
        $other.Output | Should -Be $w.Id -Because 'the other runspace draws in the same session'
        @(Get-GuiWindow -Title 'PWRS-GUI test listed').Id | Should -Be $w.Id -Because 'a runspace that closes leaves the sessions every other runspace shares'
        (Get-GuiOption).Running | Should -Be $script:Mode
        (Get-GuiSnapshot -Window $w.Id).State.Grid.rows | Should -Be 2
    }

    It 'leaves every window open when another runspace removes the module' {
        $w = Wait-GuiWindow 'PWRS-GUI test listed'
        $other = Complete-Background (Start-Background "Remove-Module PWRS-GUI; [bool](Get-Module PWRS-GUI)")
        $other.Output | Should -Be $false
        @(Get-GuiWindow -Title 'PWRS-GUI test listed').Id | Should -Be $w.Id -Because 'a runspace that removes the module leaves the sessions every other runspace shares'
        (Get-GuiOption).Running | Should -Be $script:Mode
    }

    It 'ends the session when the last runspace that has the module removes it' {
        $helpers = @(Get-OwnHelpers)
        if ($script:Mode -eq 'Helper') {
            # Get-GuiGpuAdapter requests the adapters from egui's helper, so
            # an HTML run has both.
            $expected = if ($script:Renderer -eq 'Html') { @('pwrs-gui-egui', 'pwrs-gui-web') } else { @('pwrs-gui-egui') }
            @($helpers.ProcessName | Sort-Object) | Should -Be $expected -Because 'one helper per renderer draws every window of its session'
        } else {
            $helpers.Count | Should -Be 0 -Because 'an in-process session starts no helper'
        }
        Remove-Module PWRS-GUI
        foreach ($h in $helpers) { $h.WaitForExit(20000) | Should -BeTrue -Because 'the helper ends with its session' }
        Import-Module $script:GuiModule
        @(Get-GuiWindow) | Should -BeNullOrEmpty
    }
}

Describe 'Options' {
    BeforeAll {
        Reset-GuiSession Helper
    }

    It 'store the mode, adapter and low-power choice for the next renderer' {
        Set-GuiOption -Mode Helper -Adapter 'no such adapter' -LowPower:$true
        $o = Get-GuiOption
        $o.Mode | Should -Be 'Helper'
        $o.Adapter | Should -Be 'no such adapter'
        $o.LowPower | Should -BeTrue
        Set-GuiOption -Adapter '' -LowPower:$false
        $o = Get-GuiOption
        $o.Adapter | Should -BeNullOrEmpty
        $o.LowPower | Should -BeFalse
    }

    It 'refuses a mode that is not one of the three' {
        { Set-GuiOption -Mode Window } | Should -Throw
    }

    It 'store the renderer and the web data choice for what starts next' {
        Set-GuiOption -Renderer Html -WebView PerProcess
        $o = Get-GuiOption
        $o.Renderer | Should -Be 'Html'
        $o.WebView | Should -Be 'PerProcess'
        Set-GuiOption -Renderer Egui -WebView Shared
        $o = Get-GuiOption
        $o.Renderer | Should -Be 'Egui'
        $o.WebView | Should -Be 'Shared'
    }

    It 'take the shared web data folder from the current location, and an empty string restores the default' {
        $temp = [IO.Path]::GetTempPath()
        Push-Location $temp
        try { Set-GuiOption -WebViewDataFolder 'pwrs-gui-test-data' } finally { Pop-Location }
        (Get-GuiOption).WebViewDataFolder | Should -Be (Join-Path $temp 'pwrs-gui-test-data')
        Set-GuiOption -WebViewDataFolder ''
        (Get-GuiOption).WebViewDataFolder | Should -BeNullOrEmpty
    }

    It 'warn that a shared data folder is not used with PerProcess' {
        Set-GuiOption -WebView PerProcess -WebViewDataFolder (Join-Path ([IO.Path]::GetTempPath()) 'pwrs-gui-test-data') -WarningVariable warned -WarningAction SilentlyContinue
        Set-GuiOption -WebView Shared -WebViewDataFolder ''
        "$warned" | Should -BeLike '*used with -WebView Shared*'
    }

    It 'refuse a web data choice that is not one of the two' {
        { Set-GuiOption -WebView Sometimes } | Should -Throw
    }

    It 'take the web data choice from PWRS_GUI_WEBVIEW in a process where Set-GuiOption has not set one' {
        $env:PWRS_GUI_WEBVIEW = 'PerProcess'
        try {
            $read = & (Get-Process -Id $PID).Path -NoProfile -NonInteractive -Command "Import-Module '$script:GuiModule'; (Get-GuiOption).WebView"
        } finally {
            Remove-Item Env:PWRS_GUI_WEBVIEW
        }
        "$read".Trim() | Should -Be 'PerProcess'
    }

    # On Linux, FreeBSD and macOS what $env: sets stays in PowerShell's own
    # copy of the environment, out of the process's, which native code and
    # the processes it starts read. Each child host below sets its variables
    # with $env: in its own session, after it started, and writes each
    # result on one line.
    It 'take PWRS_GUI_MODE and PWRS_GUI_WEBVIEW set with $env: in the session itself' {
        $child = @"
Import-Module '$script:GuiModule'
`$env:PWRS_GUI_MODE = 'Helper'
`$env:PWRS_GUI_WEBVIEW = 'PerProcess'
`$o = Get-GuiOption
'OPTIONS ' + `$o.Mode + ' ' + `$o.WebView
`$env:PWRS_GUI_MODE = 'Window'
try { `$null = Get-GuiOption -ErrorAction Stop; 'REFUSED no' } catch { 'REFUSED ' + `$_.FullyQualifiedErrorId }
"@
        $read = @(& (Get-Process -Id $PID).Path -NoProfile -NonInteractive -Command $child)
        $text = $read -join "`n"
        ($read | Where-Object { $_ -like 'OPTIONS *' }) | Should -Be 'OPTIONS Helper PerProcess' -Because $text
        ($read | Where-Object { $_ -like 'REFUSED *' }) | Should -BeLike 'REFUSED GuiModeUnknown*' -Because $text
    }

    It 'start a helper with the session''s environment, $env: changes included' {
        $child = @"
Import-Module '$script:GuiModule'
Set-GuiOption -Mode Helper
`$env:PWRS_GUI_BACKEND = 'bogus'
try { `$null = Get-GuiGpuAdapter -ErrorAction Stop; 'STARTED yes' } catch { 'STARTED ' + `$_.FullyQualifiedErrorId + ' ' + (`$_.Exception.Message -replace '\r?\n', ' ') }
"@
        $read = @(& (Get-Process -Id $PID).Path -NoProfile -NonInteractive -Command $child)
        $text = $read -join "`n"
        ($read | Where-Object { $_ -like 'STARTED *' }) | Should -BeLike 'STARTED GuiStartFailed*PWRS_GUI_BACKEND names bogus*' -Because $text
    }

    It 'look for the display in the session''s environment, $env: changes included' -Skip:($IsWindows -or $IsMacOS -or $PSVersionTable.PSEdition -eq 'Desktop') {
        # The first child starts with no display in its process's environment
        # and sets DISPLAY in its session; the second starts with it and
        # removes it from its session.
        $saved = @{ DISPLAY = $env:DISPLAY; WAYLAND_DISPLAY = $env:WAYLAND_DISPLAY }
        $given = @"
`$env:DISPLAY = '$($saved.DISPLAY)'
Import-Module '$script:GuiModule'
Set-GuiOption -Mode Helper
try { 'ADAPTERS ' + @(Get-GuiGpuAdapter -ErrorAction Stop).Count } catch { 'ADAPTERS ' + `$_.FullyQualifiedErrorId + ' ' + (`$_.Exception.Message -replace '\r?\n', ' ') }
"@
        $taken = @"
Remove-Item Env:DISPLAY, Env:WAYLAND_DISPLAY -ErrorAction Ignore
Import-Module '$script:GuiModule'
Set-GuiOption -Mode Helper
try { `$null = Get-GuiGpuAdapter -ErrorAction Stop; 'NODISPLAY no' } catch { 'NODISPLAY ' + `$_.FullyQualifiedErrorId }
"@
        Remove-Item Env:DISPLAY, Env:WAYLAND_DISPLAY -ErrorAction Ignore
        try {
            $first = @(& (Get-Process -Id $PID).Path -NoProfile -NonInteractive -Command $given)
        } finally {
            foreach ($name in $saved.Keys) {
                if ($saved[$name]) { Set-Item -Path "Env:$name" -Value $saved[$name] }
            }
        }
        $second = @(& (Get-Process -Id $PID).Path -NoProfile -NonInteractive -Command $taken)
        $text = ($first + $second) -join "`n"
        $count = ($first | Where-Object { $_ -like 'ADAPTERS *' }) -replace '^ADAPTERS ', ''
        $count | Should -Match '^[1-9][0-9]*$' -Because $text
        ($second | Where-Object { $_ -like 'NODISPLAY *' }) | Should -BeLike 'NODISPLAY GuiNoDisplay*' -Because $text
    }
}

Describe 'Capturing a window the moment it opens in <Mode> mode drawn with <Renderer>' -ForEach $cases {
    # A capture requested the moment a new renderer's window is listed, before
    # it may have drawn, is of that window. The window opens at 420 by 178
    # points, so its capture has that shape at any display scale.
    It 'captures the window itself, on a renderer just started' {
        foreach ($attempt in 1..3) {
            Reset-GuiSession $Mode $Renderer
            $job = Start-Background "Show-GuiWindow -Title 'PWRS-GUI test fresh capture' -Width 420 -Height 178 { ListBox 'alpha', 'beta' -Name Choice; ButtonBar { Button 'Cancel' -Name Cancel -Cancel -Result Cancel } }"
            $w = Wait-GuiWindow 'PWRS-GUI test fresh capture' $job
            $shot = Get-GuiSnapshot -Window $w.Id -Pixels
            ($shot.Width / $shot.Height) | Should -BeGreaterThan (420 / 178 - 0.03)
            ($shot.Width / $shot.Height) | Should -BeLessThan (420 / 178 + 0.03)
            Invoke-GuiTestEvent -Window $w.Id -Name Cancel -Event Click
            (Complete-Background $job).Output.Result | Should -Be 'Cancel'
        }
    }
}

Describe 'Windows opened without the focus, drawn with <_>' -ForEach 'Egui', 'Html' {
    # PWRS_GUI_NO_FOCUS is set in a child host before its session starts, and
    # the child reads who has the focus once its window is open. A window
    # opened with the variable 0 takes the focus, and three times the time
    # that took is how long a window opened with it 1 is checked, which
    # shows the reading can detect the focus being taken. macOS reads the
    # front application from NSWorkspace through osascript, Finder brought to
    # the front before each window; X11 reads the active window of a display
    # of the test's own, Xvfb with Openbox, which gives each new window the
    # focus unless the window opts out, a window of this host's own given the
    # focus before each, as Finder is on macOS. Windows gives a process the
    # foreground by rules of its own, which the front probes measure, so the
    # test does not run there.
    BeforeAll {
        $script:Renderer = $_
        $script:Display = $null
        $script:SavedDisplay = $env:DISPLAY
        $script:Started = @()
        $script:Logs = @()
        if ($IsMacOS) {
            $script:Front = @'
function Get-FrontPid {
    $said = "$(& osascript -l JavaScript -e 'ObjC.import("AppKit"); var a = $.NSWorkspace.sharedWorkspace.frontmostApplication; a.isNil() ? 0 : a.processIdentifier' 2>&1)".Trim()
    if ($said -match '^\d+$') { return [int]$said }
    throw "NSWorkspace said '$said' of the front application"
}
'@
            . ([scriptblock]::Create($script:Front))
        } elseif (-not ($IsWindows -or $PSVersionTable.PSEdition -eq 'Desktop')) {
            $null = Get-Command -Name 'Xvfb', 'openbox', 'xprop' -CommandType Application -ErrorAction SilentlyContinue -ErrorVariable absent
            if ($absent) { throw "the focus test needs Xvfb, Openbox and xprop: $($absent -join '; ')" }
            # A display is free when no server uses it: neither its lock file
            # nor its socket is there, or its lock file names a process that
            # has ended and both are this user's to remove, which they are
            # then, so the socket Xvfb is waited for is its own. Xvfb writes
            # its lock file read-only, which Remove-Item removes only with
            # -Force.
            $free = {
                param([int] $Number)
                $lock = "/tmp/.X$Number-lock"
                $socket = "/tmp/.X11-unix/X$Number"
                if (-not (Test-Path -LiteralPath $lock)) { return -not (Test-Path -LiteralPath $socket) }
                $holder = "$(Get-Content -LiteralPath $lock -Raw)".Trim()
                if ($holder -notmatch '^\d+$') { return $false }
                $null = & $script:PsPath -p $holder
                if ($LASTEXITCODE -eq 0) { return $false }
                try {
                    Remove-Item -LiteralPath (@($lock, $socket) | Where-Object { Test-Path -LiteralPath $_ }) -Force -ErrorAction Stop
                    $true
                } catch {
                    Write-Warning "display :$Number has a stale lock of an ended process that this user cannot remove: $_"
                    $false
                }
            }
            $n = 60..69 | Where-Object { & $free $_ } | Select-Object -First 1
            if ($null -eq $n) { throw 'no display from :60 to :69 is free for the focus test' }
            $script:Display = ":$n"
            $log = Join-Path ([IO.Path]::GetTempPath()) "pwrs-gui-test-focus-$PID-$n"
            $script:Logs = "$log-xvfb.out", "$log-xvfb.err", "$log-wm.out", "$log-wm.err"
            $script:Started += Start-Process -FilePath Xvfb -ArgumentList $script:Display, '-screen', '0', '1280x1024x24', '-nolisten', 'tcp' -PassThru -RedirectStandardOutput "$log-xvfb.out" -RedirectStandardError "$log-xvfb.err"
            Wait-Until { Test-Path -LiteralPath "/tmp/.X11-unix/X$n" } "Xvfb to listen on $($script:Display)" 10
            $saved = $env:DISPLAY
            $env:DISPLAY = $script:Display
            try {
                $script:Started += Start-Process -FilePath openbox -ArgumentList '--sm-disable' -PassThru -RedirectStandardOutput "$log-wm.out" -RedirectStandardError "$log-wm.err"
            } finally {
                $env:DISPLAY = $saved
            }
            Wait-Until { "$(& xprop -display $script:Display -root _NET_SUPPORTING_WM_CHECK)" -match '# 0x' } "Openbox to manage $($script:Display)" 10
            $script:Front = @'
function Get-FrontPid {
    $said = "$(& xprop -root _NET_ACTIVE_WINDOW)"
    if ($said -match 'not found') { return 0 }
    if ($said -notmatch '# (0x[0-9a-fA-F]+)') { throw "xprop said '$said' of the active window" }
    $window = $Matches[1]
    if ([Convert]::ToInt64($window, 16) -eq 0) { return 0 }
    $owner = "$(& xprop -id $window _NET_WM_PID)"
    if ($owner -match '= (\d+)') { [int]$Matches[1] } else { 0 }
}
'@
            . ([scriptblock]::Create($script:Front))
            # This host's own session draws on the test's display, for the
            # window that has the focus before each child's opens.
            $env:DISPLAY = $script:Display
            Reset-GuiSession Helper
        }

        # Opens a window in a child host with PWRS_GUI_NO_FOCUS $Value and
        # checks for up to $Seconds whether one of its helpers takes the
        # focus: when it did, in seconds, or NEVER, and what the child wrote.
        function Watch-Focus([string] $Value, [double] $Seconds) {
            $display = if ($script:Display) { "`$env:DISPLAY = '$($script:Display)'`nif (Test-Path Env:WAYLAND_DISPLAY) { Remove-Item Env:WAYLAND_DISPLAY }" } else { '' }
            $child = @"
$display
`$env:PWRS_GUI_NO_FOCUS = '$Value'
. '$PSScriptRoot/Common.ps1'
$($script:Front)
Reset-GuiSession Helper $($script:Renderer)
`$w = Show-GuiWindow -NoWait -Title 'PWRS-GUI test focus' { Button 'OK' -Name Ok }
`$helpers = @(Get-OwnHelpers | ForEach-Object Id)
`$watch = [Diagnostics.Stopwatch]::StartNew()
`$took = 'NEVER'
do {
    `$front = Get-FrontPid
    if (`$helpers -contains `$front) { `$took = [math]::Round(`$watch.Elapsed.TotalSeconds, 2); break }
    Start-Sleep -Milliseconds 100
} while (`$watch.Elapsed.TotalSeconds -lt $Seconds)
Close-GuiWindow -Window `$w.Id
'FOCUS ' + `$took + ' FRONT ' + `$front + ' HELPERS ' + (`$helpers -join ',')
"@
            $holder = $null
            if ($IsMacOS) {
                & open -a Finder
                $finder = (Get-Process -Name Finder | Select-Object -First 1).Id
                Wait-Until { (Get-FrontPid) -eq $finder } 'Finder to come to the front' 10
            } elseif ($script:Display) {
                $holder = Show-GuiWindow -NoWait -Title 'PWRS-GUI test focus holder' { Button 'OK' -Name Ok }
                $own = @(Get-OwnHelpers | ForEach-Object Id)
                Wait-Until { $own -contains (Get-FrontPid) } "the test's own window to take the focus" 10
            }
            try {
                $read = @(& (Get-Process -Id $PID).Path -NoProfile -NonInteractive -Command $child)
            } finally {
                if ($holder) { Close-GuiWindow -Window $holder.Id }
            }
            $line = $read | Where-Object { $_ -like 'FOCUS *' }
            if ("$line" -notmatch '^FOCUS (\S+) ') { throw "the child wrote no FOCUS line: $($read -join "`n")" }
            [pscustomobject]@{ Took = $Matches[1]; Text = ($read -join "`n") }
        }
    }

    AfterAll {
        if ($script:Display) {
            $env:DISPLAY = $script:SavedDisplay
            Reset-GuiSession Helper
        }
        foreach ($p in $script:Started) {
            if (-not $p.HasExited) { Stop-Process -Id $p.Id }
        }
        # Stop-Process ends Xvfb with SIGKILL, which leaves its lock file and
        # socket in place; once the server has exited they are this test's to
        # remove, so the display is free for the next run.
        if ($script:Display) {
            foreach ($p in $script:Started) {
                if (-not $p.WaitForExit(10000)) { throw "process $($p.Id) of the focus test's display did not exit within 10 s" }
            }
            $n = $script:Display.TrimStart(':')
            foreach ($left in "/tmp/.X$n-lock", "/tmp/.X11-unix/X$n") {
                if (Test-Path -LiteralPath $left) {
                    try { Remove-Item -LiteralPath $left -Force -ErrorAction Stop } catch { Write-Warning "the focus test could not remove $left after its display ended: $_" }
                }
            }
        }
        foreach ($f in $script:Logs) {
            if (Test-Path -LiteralPath $f) { Remove-Item -LiteralPath $f }
        }
    }

    It 'leaves the focus where it was when PWRS_GUI_NO_FOCUS is 1, where a window opened with it 0 takes it' -Skip:($IsWindows -or $PSVersionTable.PSEdition -eq 'Desktop') {
        $taken = Watch-Focus '0' 20
        $taken.Took | Should -Not -Be 'NEVER' -Because "a window opened with PWRS_GUI_NO_FOCUS 0 takes the focus, which shows the reading detects it: $($taken.Text)"
        $seconds = [math]::Max(2, 3 * [double]$taken.Took)
        $left = Watch-Focus '1' $seconds
        $left.Took | Should -Be 'NEVER' -Because "a window opened with PWRS_GUI_NO_FOCUS 1 leaves the focus where it was for the $seconds s it was checked: $($left.Text)"
    }
}
