# Show-GuiWindow's pages and Send-GuiMessage, in every mode the platform
# has. Pages: what a page closes with, the standards mode a page given
# without a doctype is laid out in, what -OnMessage replies to its sends
# with, the values the session sends it, a folder's files, the page staying
# on its origin, Ctrl+C and -NoWait. Also a page in a process with web data
# of its own, and a page loaded again after its browser process is killed
# getting back what it saved. With no webview engine, a widget window is
# drawn with egui while a page fails naming what to install. Widget pages:
# with no fonts and under the C locale, their boxes against egui's, their
# context menus opened by a person's input as egui opens them, and the
# sizes of windows fitted to their widgets against egui's. The engine: its
# settings, its version against the floor, the WebView2 add-on's runtime
# and the WebKitGTK add-on preferred. See Common.ps1 for the deadlines.

BeforeDiscovery {
    $modes = if ($IsWindows -or $PSVersionTable.PSEdition -eq 'Desktop') { 'Helper', 'InProcess' } else { 'Helper' }
    $onWindows = $IsWindows -or $PSVersionTable.PSEdition -eq 'Desktop'
}

BeforeAll {
    . (Join-Path $PSScriptRoot 'Common.ps1')
    $script:Watchdog = Start-Watchdog
}

AfterAll {
    Stop-Watchdog $script:Watchdog
}

Describe 'Show-GuiWindow pages in <_> mode' -ForEach $modes {
    BeforeAll {
        Reset-GuiSession $_
    }

    It 'writes what the page closes with' {
        $job = Start-Background "Show-GuiWindow -Title 'PWRS-GUI test close' -Html '<script>window.pwrs.close({ answer: 42, list: [1, 2] });</script>'"
        $done = Complete-Background $job
        $done.Errors | Should -BeNullOrEmpty
        $done.Output.Count | Should -Be 1
        $done.Output[0].PSTypeNames | Should -Contain 'PwrsGui.WindowResult'
        $done.Output[0].Result.answer | Should -Be 42
        @($done.Output[0].Result.list) | Should -Be @(1, 2)
    }

    It 'lays a page given without a doctype out in standards mode, and loads a page with a doctype as written' {
        $job = Start-Background "Show-GuiWindow -Title 'PWRS-GUI test standards' -Html '<script>window.pwrs.close(document.compatMode);</script>'"
        $done = Complete-Background $job
        $done.Errors | Should -BeNullOrEmpty
        $done.Output[0].Result | Should -Be 'CSS1Compat' -Because 'a page without a doctype is given <!DOCTYPE html> before it'
        $old = '<!DOCTYPE HTML PUBLIC "-//W3C//DTD HTML 4.0 Transitional//EN"><script>window.pwrs.close(document.compatMode);</script>'
        $job = Start-Background "Show-GuiWindow -Title 'PWRS-GUI test quirks' -Html '$old'"
        $done = Complete-Background $job
        $done.Errors | Should -BeNullOrEmpty
        $done.Output[0].Result | Should -Be 'BackCompat' -Because 'a doctype that selects quirks mode is loaded as written'
    }

    It 'writes the properties of what the page sends in the order the page wrote them' {
        $job = Start-Background "Show-GuiWindow -Title 'PWRS-GUI test order' -Html '<script>window.pwrs.close({ zeta: 1, alpha: 2, mid: 3 });</script>'"
        $done = Complete-Background $job
        $done.Errors | Should -BeNullOrEmpty
        @($done.Output[0].Result.PSObject.Properties.Name) | Should -Be @('zeta', 'alpha', 'mid')
    }

    It 'replies to the page''s sends with what -OnMessage returns' {
        $page = '<script>window.pwrs.send(21).then(function (r) { window.pwrs.close(r); });</script>'
        $job = Start-Background "Show-GuiWindow -Title 'PWRS-GUI test reply' -Html '$page' -OnMessage { param(`$m) `$m * 2 }"
        $done = Complete-Background $job
        $done.Errors | Should -BeNullOrEmpty
        $done.Output[0].Result | Should -Be 42
    }

    It 'replies to a send with null when -OnMessage writes nothing' {
        $page = '<script>window.pwrs.send("anything").then(function (r) { window.pwrs.close(r === null ? "null" : "not null"); });</script>'
        $job = Start-Background "Show-GuiWindow -Title 'PWRS-GUI test silent' -Html '$page' -OnMessage { param(`$m) }"
        (Complete-Background $job).Output[0].Result | Should -Be 'null'
    }

    It 'replies with an array -OnMessage writes as one object, and null for $null, in every host' {
        $page = '<script>window.pwrs.send("list").then(function (r) { window.pwrs.send("none").then(function (n) { window.pwrs.close(JSON.stringify([r, n])); }); });</script>'
        $job = Start-Background "Show-GuiWindow -Title 'PWRS-GUI test array' -Html '$page' -OnMessage { param(`$m) if (`$m -eq 'list') { , @('a', 'b') } else { `$null } }"
        $done = Complete-Background $job
        $done.Errors | Should -BeNullOrEmpty
        $done.Output[0].Result | Should -Be '[["a","b"],null]'
    }

    It 'replies with file names as plain strings, and a date as ISO 8601, in every host' {
        $site = Join-Path ([IO.Path]::GetTempPath()) "pwrs-gui-test-names-$PID"
        New-Item -ItemType Directory -Path $site -Force | Out-Null
        Set-Content -LiteralPath (Join-Path $site 'one.txt'), (Join-Path $site 'two.txt') -Value 'x'
        try {
            $page = '<script>window.pwrs.send("files").then(function (names) { window.pwrs.send("date").then(function (d) { window.pwrs.close(JSON.stringify(names) + " " + new Date(d).toISOString()); }); });</script>'
            $handler = "{ param(`$m) if (`$m -eq 'files') { , @(Get-ChildItem -Name -LiteralPath '$site') } else { [datetime]::new(2001, 2, 3, 4, 5, 6, 789, [DateTimeKind]::Utc) } }"
            $job = Start-Background "Show-GuiWindow -Title 'PWRS-GUI test names' -Html '$page' -OnMessage $handler"
            $done = Complete-Background $job
            $done.Errors | Should -BeNullOrEmpty
            $done.Output[0].Result | Should -Be '["one.txt","two.txt"] 2001-02-03T04:05:06.789Z'
        } finally {
            Remove-Item -LiteralPath $site -Recurse -Force
        }
    }

    It 'delivers what Send-GuiMessage sends to the page''s handlers' {
        $page = '<script>window.pwrs.on(function (v) { window.pwrs.close(v.hello + v.n); });</script>'
        $job = Start-Background "Show-GuiWindow -Title 'PWRS-GUI test deliver' -Html '$page'"
        $w = Wait-GuiWindow 'PWRS-GUI test deliver' $job
        Send-GuiMessage -Window $w.Id -Message @{ hello = 'world'; n = 1 }
        $done = Complete-Background $job
        $done.Errors | Should -BeNullOrEmpty
        $done.Output[0].Result | Should -Be 'world1'
    }

    It 'refuses a value for a window that shows no page of its own' {
        $job = Start-Background "Read-GuiChoice -Choice 'one','two' -Title 'PWRS-GUI test no page'"
        $w = Wait-GuiWindow 'PWRS-GUI test no page' $job
        Send-GuiMessage -Window $w.Id -Message 'hello' -ErrorVariable refused -ErrorAction SilentlyContinue
        Invoke-GuiTestEvent -Window $w.Id -Name Cancel -Event Click
        $null = Complete-Background $job
        $refused.Count | Should -Be 1
        $refused[0].FullyQualifiedErrorId | Should -BeLike 'GuiRefused,*'
        "$($refused[0])" | Should -BeLike '*page*'
    }

    It 'loads a folder''s page and its other files' {
        $site = Join-Path ([IO.Path]::GetTempPath()) "pwrs-gui-test-site-$PID"
        New-Item -ItemType Directory -Path $site -Force | Out-Null
        Set-Content -LiteralPath (Join-Path $site 'index.html') -Value '<script src="app.js"></script>'
        Set-Content -LiteralPath (Join-Path $site 'app.js') -Value 'window.pwrs.close("from app.js");'
        try {
            $job = Start-Background "Show-GuiWindow -Title 'PWRS-GUI test folder' -Path '$site'"
            $done = Complete-Background $job
            $done.Errors | Should -BeNullOrEmpty
            $done.Output[0].Result | Should -Be 'from app.js'
        } finally {
            Remove-Item -LiteralPath $site -Recurse -Force
        }
    }

    It 'refuses a -Path that names nothing with GuiNoPage alone' {
        $missing = Join-Path ([IO.Path]::GetTempPath()) "pwrs-gui-test-no-site-$PID"
        $caught = $null
        try { Show-GuiWindow -Path $missing -ErrorVariable seen } catch { $caught = $_ }
        $caught.FullyQualifiedErrorId | Should -BeLike 'GuiNoPage,*'
        @($seen).Count | Should -Be 1 -Because (@($seen) -join "`n")
    }

    It 'stops the page leaving its own origin without -AllowNavigation' {
        $page = '<script>location.href = "https://example.com/"; setTimeout(function () { window.pwrs.close(location.origin); }, 1000);</script>'
        $job = Start-Background "Show-GuiWindow -Title 'PWRS-GUI test origin' -Html '$page'"
        $done = Complete-Background $job
        $done.Errors | Should -BeNullOrEmpty
        $done.Output[0].Result | Should -Match '^(https?://pwrs\.localhost|pwrs://localhost)$'
    }

    It 'writes nothing and closes the window when stopped as Ctrl+C stops it' {
        $job = Start-Background "Show-GuiWindow -Title 'PWRS-GUI test page stop' -Html '<p>waiting</p>'"
        $null = Wait-GuiWindow 'PWRS-GUI test page stop' $job
        $done = Stop-Background $job
        $done.Output | Should -BeNullOrEmpty
        Wait-Until { -not (Get-GuiWindow -Title 'PWRS-GUI test page stop') } 'the stopped command''s window closing'
    }

    It 'with -NoWait, returns the window once Get-GuiWindow lists it, and Send-GuiMessage reaches it' {
        $page = '<script>window.pwrs.on(function (v) { window.pwrs.close(v); });</script>'
        $w = Show-GuiWindow -Title 'PWRS-GUI test nowait' -Html $page -NoWait
        $w.PSTypeNames | Should -Contain 'PwrsGui.PageWindow'
        @(Get-GuiWindow -Title 'PWRS-GUI test nowait').Id | Should -Be $w.Id -Because 'the command returns once the page reports its first frame, which is when a window is listed'
        Send-GuiMessage -Window $w.Id -Message 'bye'
        Wait-Until { -not (Get-GuiWindow -Title 'PWRS-GUI test nowait') } 'the page closing on the message'
    }

    It 'takes the page window Send-GuiMessage sends to from the pipeline' {
        $page = '<script>window.pwrs.on(function (v) { window.pwrs.close(v); });</script>'
        $w = Show-GuiWindow -Title 'PWRS-GUI test piped page' -Html $page -NoWait
        $null = Wait-GuiWindow 'PWRS-GUI test piped page'
        $w | Send-GuiMessage -Message 'bye'
        Wait-Until { -not (Get-GuiWindow -Title 'PWRS-GUI test piped page') } 'the page closing on the piped message'
    }

    It 'shows a page in a process whose web data is its own, with -WebView PerProcess' {
        # A renderer takes its web data mode as it starts, so a child host
        # starts one of its own.
        $mode = $_
        $child = @"
Import-Module '$script:GuiModule'
Set-GuiOption -Mode $mode -WebView PerProcess
try { `$r = Show-GuiWindow -Html '<script>window.pwrs.close(7);</script>' -ErrorAction Stop; 'PAGE result ' + `$r.Result } catch { 'PAGE ' + `$_.FullyQualifiedErrorId + ' ' + (`$_.Exception.Message -replace '\r?\n', ' ') }
"@
        $read = @(& (Get-Process -Id $PID).Path -NoProfile -NonInteractive -Command $child)
        ($read | Where-Object { $_ -like 'PAGE *' }) | Should -Be 'PAGE result 7' -Because ($read -join "`n")
    }

    It 'brings a page whose browser process ended back with what it saved' -Skip:(-not $onWindows) {
        $marker = Join-Path ([IO.Path]::GetTempPath()) "pwrs-gui-test-saved-$PID"
        if (Test-Path -LiteralPath $marker) { Remove-Item -LiteralPath $marker }
        $page = '<script>window.pwrs.restore().then(function (v) { if (v === null) { window.pwrs.save("kept"); window.pwrs.send("saved"); } else { window.pwrs.close(v); } });</script>'
        $job = Start-Background "Show-GuiWindow -Title 'PWRS-GUI test restore' -Html '$page' -OnMessage { param(`$m) if (`$m -eq 'saved') { New-Item -ItemType File -Path '$marker' -Force | Out-Null } }"
        Wait-Until { Test-Path -LiteralPath $marker } 'the page saving its value'
        $browsers = @(Get-CimInstance Win32_Process -Filter "Name = 'msedgewebview2.exe'" | Where-Object { $_.CommandLine -like '*PWRS-GUI*' -and $_.CommandLine -notlike '*--type=*' })
        # A browser process already ending, which Windows still lists and
        # Get-Process no longer finds, runs no page and cannot be stopped.
        $running = @($browsers | ForEach-Object { Get-Process -Id $_.ProcessId -ErrorAction Ignore })
        $running.Count | Should -BeGreaterThan 0 -Because 'a WebView2 browser process runs on PWRS-GUI''s data folder'
        $running | Stop-Process -Force
        $done = Complete-Background $job
        Remove-Item -LiteralPath $marker
        $done.Errors | Should -BeNullOrEmpty
        $done.Output[0].Result | Should -Be 'kept'
    }

    # A page requesting other browser arguments than the shared folder's
    # running browser process has is refused by some WebView2 runtimes
    # (154.0.4258.53) and joined to that browser process without its
    # arguments by others (154.0.4258.62); either is accepted, and a refusal
    # must name how to give a process its own browser process.
    It 'fails a page WebView2 refuses for other browser arguments, naming how to give a process its own browser process, or opens it where the runtime joins it' -Skip:(-not $onWindows) {
        $mode = $_
        $job = Start-Background "Show-GuiWindow -Title 'PWRS-GUI test holder' -Html '<p>holding the shared browser process</p>'"
        $null = Wait-GuiWindow 'PWRS-GUI test holder' $job
        $child = @"
Import-Module '$script:GuiModule'
`$env:WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS = '--disable-gpu-vsync'
Set-GuiOption -Mode $mode
try { `$r = Show-GuiWindow -Html '<script>window.pwrs.close(1);</script>' -ErrorAction Stop; 'PAGE result ' + `$r.Result } catch { 'PAGE ' + `$_.FullyQualifiedErrorId + ' ' + `$_.Exception.Message }
"@
        try {
            $read = @(& (Get-Process -Id $PID).Path -NoProfile -NonInteractive -Command $child)
        } finally {
            $null = Stop-Background $job
        }
        $page = $read | Where-Object { $_ -like 'PAGE *' }
        if ($page -like 'PAGE GuiPageFailed*') {
            $page | Should -BeLike '*-WebView PerProcess*' -Because ($read -join "`n")
        } else {
            $page | Should -Be 'PAGE result 1' -Because ($read -join "`n")
        }
    }
}

Describe 'With no webview engine, in <_> mode' -ForEach $modes {
    # A child host runs with the engine made unfindable: on Windows the
    # WebView2 loader is pointed at a folder that does not exist, elsewhere an
    # empty libwebkit2gtk-4.1.so.0 comes first on the library path and
    # PWRS_GUI_WEBKIT_ADDON says no add-on is to be used. The child writes
    # each record on one line, since a message quotes the helper's log. The
    # grid returns at the end of its input, which can be before its window's
    # first frame, and a window is listed once that frame is drawn, so the
    # child waits up to 20 s for the window to be listed.
    It 'draws a widget window requested in HTML with egui, saying why, and refuses a page naming what to install' -Skip:($IsMacOS) {
        $mode = $_
        $missing = Join-Path ([IO.Path]::GetTempPath()) "pwrs-gui-test-no-engine-$PID"
        $saved = @{}
        if ($IsWindows -or $PSVersionTable.PSEdition -eq 'Desktop') {
            $saved.WEBVIEW2_BROWSER_EXECUTABLE_FOLDER = $env:WEBVIEW2_BROWSER_EXECUTABLE_FOLDER
            $env:WEBVIEW2_BROWSER_EXECUTABLE_FOLDER = $missing
        } else {
            New-Item -ItemType Directory -Path $missing -Force | Out-Null
            New-Item -ItemType File -Path (Join-Path $missing 'libwebkit2gtk-4.1.so.0') -Force | Out-Null
            $saved.LD_LIBRARY_PATH = $env:LD_LIBRARY_PATH
            $env:LD_LIBRARY_PATH = if ($env:LD_LIBRARY_PATH) { "${missing}:$env:LD_LIBRARY_PATH" } else { $missing }
            $saved.PWRS_GUI_WEBKIT_ADDON = $env:PWRS_GUI_WEBKIT_ADDON
            $env:PWRS_GUI_WEBKIT_ADDON = 'none'
        }
        $child = @"
Import-Module '$script:GuiModule'
Set-GuiOption -Mode $mode -Renderer Html
`$said = 1..2 | Out-GuiGridView -Title 'PWRS-GUI test fallback' -Verbose 4>&1 | Where-Object { `$_ -is [System.Management.Automation.VerboseRecord] } | ForEach-Object Message
'VERBOSE ' + ((`$said -join ' | ') -replace '\r?\n', ' ')
`$deadline = [DateTime]::UtcNow.AddSeconds(20)
do {
    `$listed = @(Get-GuiWindow -Title 'PWRS-GUI test fallback').Count
    if (`$listed) { break }
    Start-Sleep -Milliseconds 50
} while ([DateTime]::UtcNow -lt `$deadline)
'WINDOWS ' + `$listed
try { Show-GuiWindow -Html '<p>no engine</p>' -ErrorAction Stop } catch { 'PAGE ' + `$_.FullyQualifiedErrorId + ' ' + (`$_.Exception.Message -replace '\r?\n', ' ') }
"@
        try {
            $read = @(& (Get-Process -Id $PID).Path -NoProfile -NonInteractive -Command $child)
        } finally {
            foreach ($name in $saved.Keys) {
                if ($saved[$name]) { Set-Item -Path "Env:$name" -Value $saved[$name] } else { Remove-Item -Path "Env:$name" }
            }
            if (Test-Path -LiteralPath $missing) { Remove-Item -LiteralPath $missing -Recurse -Force }
        }
        $text = $read -join "`n"
        ($read | Where-Object { $_ -like 'VERBOSE *' }) | Should -BeLike '*could not start, so this window is drawn with egui*' -Because $text
        ($read | Where-Object { $_ -like 'WINDOWS *' }) | Should -Be 'WINDOWS 1' -Because $text
        ($read | Where-Object { $_ -like 'PAGE *' }) | Should -BeLike 'PAGE GuiNoWebview*' -Because $text
        if ($IsWindows -or $PSVersionTable.PSEdition -eq 'Desktop') {
            ($read | Where-Object { $_ -like 'PAGE *' }) | Should -BeLike '*WebView2 Runtime*' -Because $text
        } else {
            ($read | Where-Object { $_ -like 'PAGE *' }) | Should -BeLike '*Install-Module PWRS-GUI.WebKit.*' -Because $text
        }
    }
}

Describe 'A widget page where the system has no fonts' {
    # A child host's web helper, and the engine it starts, read an empty
    # fontconfig file, so the system names no font; the page draws in egui's
    # own fonts, which the helper gives it. The child writes one line.
    It "draws its text in egui's own fonts, warning of nothing" -Skip:($onWindows -or $IsMacOS) {
        $empty = Join-Path ([IO.Path]::GetTempPath()) "pwrs-gui-test-no-fonts-$PID.conf"
        Set-Content -LiteralPath $empty -Value '<?xml version="1.0"?><fontconfig></fontconfig>'
        $saved = $env:FONTCONFIG_FILE
        $env:FONTCONFIG_FILE = $empty
        $child = @"
. '$PSScriptRoot/Common.ps1'
Reset-GuiSession Helper Html
`$job = Start-Background "Show-GuiWindow -Title 'PWRS-GUI test no fonts' { Text 'seen' -Name Seen; Button 'OK' -Name Ok -Result Ok }"
`$w = Wait-GuiWindow 'PWRS-GUI test no fonts' `$job
`$seen = (Get-GuiSnapshot -Window `$w.Id).Bounds.Seen
Invoke-GuiTestEvent -Window `$w.Id -Name Ok -Event Click
`$done = Complete-Background `$job
'WINDOW ' + `$done.Output[-1].Result + ' WIDTH ' + `$seen.Width + ' WARNINGS ' + ((`$done.Warnings -join ' | ') -replace '\r?\n', ' ')
"@
        try {
            $read = @(& (Get-Process -Id $PID).Path -NoProfile -NonInteractive -Command $child)
        } finally {
            $env:FONTCONFIG_FILE = $saved
            Remove-Item -LiteralPath $empty
        }
        $text = $read -join "`n"
        $line = $read | Where-Object { $_ -like 'WINDOW *' }
        $written = '^WINDOW Ok WIDTH (\d+(\.\d+)?) WARNINGS $'
        $line | Should -Match $written -Because $text
        [double][regex]::Match("$line", $written).Groups[1].Value | Should -BeGreaterThan 0 -Because "a text measures nothing across only where no font draws it: $text"
    }
}

Describe 'Widget boxes drawn by HTML and by egui in <_> mode' -ForEach $modes {
    # The same window drawn by each renderer, every named widget's box read
    # back once two reads 300 ms apart agree, each within 2 points of
    # egui's.
    BeforeAll {
        Reset-GuiSession $_
        function Read-Boxes([string] $Title, [string] $Renderer, [scriptblock] $Content, [string[]] $Tables) {
            $w = Show-GuiWindow -NoWait -Title $Title -Renderer $Renderer -Width 720 -Height 640 -Content $Content
            try {
                $before = $null
                $last = $null
                $deadline = [DateTime]::UtcNow.AddSeconds(20)
                while ([DateTime]::UtcNow -lt $deadline) {
                    Start-Sleep -Milliseconds 300
                    $shot = Get-GuiSnapshot -Window $w.Id
                    $now = $shot.Bounds | ConvertTo-Json -Compress -Depth 4
                    $full = -not @($Tables | Where-Object { $shot.State.$_.rows -lt 3 })
                    if ($full -and $shot.Bounds.Count -gt 0 -and $now -eq $last) { return $shot.Bounds }
                    $before = $last
                    $last = $now
                }
                throw "the boxes of '$Title' drawn by $Renderer did not settle within 20 s; the read before the last $before; the last $last"
            } finally {
                Close-GuiWindow -Window $w.Id
            }
        }
    }

    It 'places and sizes every widget of <Name> within 2 points of egui' -ForEach @(
        @{ Name = 'the inputs'; Tables = @(); Content = {
                CheckBox 'Dry run' -Name DryRun -Checked
                TextBox 'web01' -Name Server
                TextBox -Name Notes -Multiline
                NumberBox 3 -Minimum 1 -Maximum 10 -Name Copies
                Slider 20 -Name Volume
                ComboBox 'staging', 'production' -Name Target -Selected staging
                ComboBox 'web01', 'web02' -Editable -Selected 'web03' -Name Host
                RadioGroup 'Small', 'Large' -Name Size -Selected Small
                ColorPicker '#336699' -Name Accent
                DatePicker ([datetime]'2026-10-08T14:30') -Name When
                FilePicker -Name File
                Button 'OK' -Name Ok
            }
        }
        @{ Name = 'the display and layout widgets'; Tables = @(); Content = {
                Text 'Body text that is plain' -Name Body
                Text 'A heading' -Style Heading -Name Head
                Text 'mono text 0123' -Style Mono -Name Mono
                Text 'a link' -Style Link -Name Link
                ProgressBar 30 -Name Copy -Text 'Copying'
                Spinner -Name Busy
                Separator
                Spacer 12 -Name Gap
                Grid -Columns 2 -Name Form { Text 'Server' -Name Label1; TextBox 'web01' -Name Field1 }
                Group -Title 'Options' -Name Options { CheckBox 'Verbose' -Name Verbose }
                Tabs -Name Pages { Tab 'General' { Text 'first page' -Name First }; Tab 'More' { Text 'second page' } }
                Row -Name Bar { Button 'A' -Name A; Button 'Bee' -Name Bee; Text 'see' -Name See }
            }
        }
        @{ Name = 'the lists'; Tables = @('Files'); Content = {
                ListBox 'alpha', 'beta', 'gamma' -Name List
                ListBox 'x', 'y' -Multiple -Name Many
                Table (1..3 | ForEach-Object { [pscustomobject]@{ Name = "n$_"; Size = $_ * 10 } }) -Selection Single -Name Files
                TreeView -Name Tree -Node @{ Text = 'Fruit'; Children = 'Apple', 'Pear' }, 'Leek'
                LogView 'one', 'two' -Name Log -Height 60
            }
        }
        @{ Name = 'the dialog widgets'; Tables = @(); Content = {
                Text 'Deploy the build' -Style Instruction -Name Ask
                Grid -Columns 2 -Align Stretch -Name Form { Text 'Server' -Name ServerLabel; TextBox 'web01' -Name Server; Text 'Copies' -Name CopiesLabel; NumberBox 2 -Name Copies }
                Row -Align End -Name Tools { Button 'Refresh' -Name Refresh }
                Row -Name Steps { Text 'Step 1 of 3' -Name Step; Spacer -Fill; Button 'Next' -Name Next }
                ButtonBar { Button 'Deploy' -Name Deploy -Default -Result Ok; Button 'Cancel' -Name Cancel -Cancel -Result Cancel }
            }
        }
        @{ Name = 'the Markdown text blocks'; Tables = @(); Content = {
                Markdown -Text "First paragraph.`n`nSecond paragraph." -Name Paragraphs
                Markdown -Text "# One`n`n## Two`n`n### Three`n`nAfter them." -Name Headings
                Markdown -Text "- one`n- two`n  - nested`n`n1. first`n2. second" -Name Lists
                Markdown -Text "> a quote`n>`n> of two paragraphs" -Name Quote
                Markdown -Text "> [!NOTE]`n> Read this." -Name Alert
            }
        }
        @{ Name = 'the Markdown framed blocks'; Tables = @(); Content = {
                Markdown -Text "``````rust`nlet x = 1;`n```````n`nAfter it." -Name Code
                Markdown -Text "| a | b |`n|---|---|`n| 1 | 2 |" -Name Table
                Markdown -Text "Before.`n`n---`n`nAfter." -Name Rule
                Markdown -Text "Text with a note.[^1]`n`n[^1]: The note." -Name Footnote
            }
        }
    ) {
        $egui = Read-Boxes "PWRS-GUI test boxes egui" Egui $Content $Tables
        $html = Read-Boxes "PWRS-GUI test boxes html" Html $Content $Tables
        @($html.Keys) | Sort-Object | Should -Be (@($egui.Keys) | Sort-Object) -Because 'both renderers draw the same widgets'
        $off = foreach ($name in $egui.Keys) {
            $e = $egui[$name]
            $h = $html[$name]
            foreach ($side in 'X', 'Y', 'Width', 'Height') {
                $d = [math]::Abs($h.$side - $e.$side)
                if ($d -gt 2) { '{0} {1}: HTML {2}, egui {3}' -f $name, $side, $h.$side, $e.$side }
            }
        }
        # Every widget's box in both, so a drift that adds up down a window
        # shows where it starts.
        $boxes = foreach ($name in @($egui.Keys) | Sort-Object) {
            $e = $egui[$name]
            $h = $html[$name]
            '{0}: egui {1}, {2}, {3} x {4}; HTML {5}, {6}, {7} x {8}' -f $name, $e.X, $e.Y, $e.Width, $e.Height, $h.X, $h.Y, $h.Width, $h.Height
        }
        $off | Should -BeNullOrEmpty -Because ("every widget is placed and sized within 2 points of egui's; the boxes:`n" + ($boxes -join "`n"))
    }
}

Describe 'Context menus opened by a person''s input, drawn by HTML and by egui in <_> mode' -ForEach $modes {
    # The same window drawn by each renderer and given a person's input
    # through Invoke-GuiTestEvent -Input: its menu opens at a right click by
    # egui's rule, the right button let go within 6 points and 0.8 seconds
    # of its press, and at nothing else.
    BeforeAll {
        Reset-GuiSession $_
    }

    It 'opens a menu drawn with <Renderer> at a right click, and none at a right drag, a long right press, the Menu key or Shift+F10' -ForEach @(@{ Renderer = 'Egui' }, @{ Renderer = 'Html' }) {
        # A button, and a chart, whose right drag draws a box to zoom to.
        $w = Show-GuiWindow -NoWait -Title "PWRS-GUI test right click $Renderer" -Renderer $Renderer -Width 400 -Height 320 -Content {
            Button 'Target' -Name Target
            ContextMenu -For Target -Name Menu { MenuItem 'Copy' -Name Copy }
            Chart -Name Load -Height 160 -Series @{ Name = 'cpu'; Y = 3, 5, 4, 6 }
            ContextMenu -For Load -Name LoadMenu { MenuItem 'Reset' -Name Reset }
        }
        try {
            $bounds = (Get-GuiSnapshot -Window $w.Id).Bounds
            foreach ($pair in @('Target', 'Menu'), @('Load', 'LoadMenu')) {
                $b = $bounds[$pair[0]]
                $x = $b.X + $b.Width / 2
                $y = $b.Y + $b.Height / 2
                $menu = $pair[1]
                $open = { $null -ne (Get-GuiSnapshot -Window $w.Id).Bounds[$menu] }
                Invoke-GuiTestEvent -Window $w.Id -Input @{ Press = 'Right'; X = $x; Y = $y }, @{ Move = $true; X = $x + 20; Y = $y + 10 }, @{ Release = 'Right'; X = $x + 20; Y = $y + 10 }
                & $open | Should -BeFalse -Because "a right drag on $($pair[0]) is no click"
                Invoke-GuiTestEvent -Window $w.Id -Input @{ Press = 'Right'; X = $x; Y = $y }, @{ Wait = 0.9 }, @{ Release = 'Right'; X = $x; Y = $y }
                & $open | Should -BeFalse -Because "a right press on $($pair[0]) held past 0.8 seconds is no click"
                Invoke-GuiTestEvent -Window $w.Id -Input @{ Move = $true; X = $x; Y = $y }, @{ Key = 'Menu' }, @{ Key = 'Shift+F10' }
                & $open | Should -BeFalse -Because "the Menu key and Shift+F10 open no menu over $($pair[0])"
                Invoke-GuiTestEvent -Window $w.Id -Input @{ Press = 'Right'; X = $x; Y = $y }, @{ Release = 'Right'; X = $x + 3; Y = $y }
                Wait-Until { & $open } "the menu of $($pair[0]) opening at the right click"
                Invoke-GuiTestEvent -Window $w.Id -Input @{ Key = 'Escape' }
                Wait-Until { -not (& $open) } "Escape closing the menu of $($pair[0])"
            }
        } finally {
            Close-GuiWindow -Window $w.Id
        }
    }
}

Describe 'Windows fitted to their widgets by HTML and by egui in <_> mode' -ForEach $modes {
    # Each window opened without a size by each renderer in turn, its
    # capture's size compared, within 2 points at the display's scale.
    BeforeAll {
        Reset-GuiSession $_
        $w = Show-GuiWindow -NoWait -Title 'PWRS-GUI test fit scale' -Width 400 -Height 200 { Text 'measures the scale' -Name Scale }
        try {
            $script:Scale = (Get-GuiSnapshot -Window $w.Id -Pixels).Width / 400
        } finally {
            Close-GuiWindow -Window $w.Id
        }
        # The size in pixels of the window $Script opens, drawn by $Renderer,
        # once it has opened; then the button $Close ends the command.
        function Read-Size([string] $Renderer, [string] $Title, [string] $Script, [string] $Close) {
            Set-GuiOption -Renderer $Renderer
            try {
                $job = Start-Background ($Script.Replace('<title>', $Title))
                $w = Wait-GuiWindow $Title $job
                $shot = Get-GuiSnapshot -Window $w.Id -Pixels
                Invoke-GuiTestEvent -Window $w.Id -Name $Close -Event Click
                $done = Complete-Background $job
                $done.Errors | Should -BeNullOrEmpty
                @($shot.Width, $shot.Height)
            } finally {
                Set-GuiOption -Renderer Egui
            }
        }
        # The boxes of the named widgets of the window $Content describes,
        # drawn by $Renderer and fitted to them, read once it has opened.
        function Read-Fitted([string] $Renderer, [string] $Title, [scriptblock] $Content) {
            $w = Show-GuiWindow -NoWait -Title $Title -Renderer $Renderer -Content $Content
            try {
                $null = Wait-GuiWindow $Title
                (Get-GuiSnapshot -Window $w.Id).Bounds
            } finally {
                Close-GuiWindow -Window $w.Id
            }
        }
        # The pictures the window of pictures shows: a document's own beside
        # it, one from an https address with no server, and an Image's.
        $script:Pictures = Join-Path ([IO.Path]::GetTempPath()) "pwrs-gui-test-fit-$PID"
        $null = [IO.Directory]::CreateDirectory($script:Pictures)
        [IO.File]::WriteAllText((Join-Path $script:Pictures 'box.svg'), '<svg xmlns="http://www.w3.org/2000/svg" width="120" height="80"><rect width="120" height="80" fill="#ff0000"/></svg>')
        [IO.File]::WriteAllText((Join-Path $script:Pictures 'bar.svg'), '<svg xmlns="http://www.w3.org/2000/svg" width="200" height="30"><rect width="200" height="30" fill="#0000ff"/></svg>')
        [IO.File]::WriteAllText((Join-Path $script:Pictures 'doc.md'), "![a red box](box.svg)`n`nUnder the box ![a picture from nowhere](https://127.0.0.1:9/gone.png) a line.`n")
    }

    AfterAll {
        foreach ($f in 'box.svg', 'bar.svg', 'doc.md') { Remove-Item -LiteralPath (Join-Path $script:Pictures $f) }
        Remove-Item -LiteralPath $script:Pictures
    }

    It 'opens <Name> at the same size in both' -ForEach @(
        @{ Name = 'a choice'; Close = 'Cancel'; Script = "Read-GuiChoice -Prompt 'Pick one' -Choice 'alpha', 'beta', 'gamma' -Title '<title>'" }
        @{ Name = 'a long choice'; Close = 'Cancel'; Script = "Read-GuiChoice -Prompt 'Pick one of many' -Choice (1..25 | ForEach-Object { ""choice number `$_"" }) -Title '<title>'" }
        @{ Name = 'a form'; Close = 'Cancel'; Script = "Read-GuiInput -Prompt 'New user' -Field @{ Name = 'Login' }, @{ Name = 'Password'; Type = 'Password' }, @{ Name = 'Team'; Type = 'Choice'; Choice = 'build', 'ops' } -Title '<title>'" }
        @{ Name = 'a credential form'; Close = 'Cancel'; Script = "Read-GuiCredential -Message 'Sign in to the build server' -Title '<title>'" }
        @{ Name = 'a grid'; Close = 'Cancel'; Script = "1..6 | ForEach-Object { [pscustomobject]@{ Name = ""service`$_""; Status = 'Running'; Port = 8000 + `$_ } } | Out-GuiGridView -PassThru -Title '<title>'" }
        @{ Name = 'a window of widgets'; Close = 'Ok'; Script = "Show-GuiWindow -Title '<title>' { Text 'Deploy the build' -Style Instruction; Grid -Columns 2 -Align Stretch { Text 'Server'; TextBox 'web01' -Name Server }; ButtonBar { Button 'OK' -Name Ok -Default -Result Ok } }" }
        @{ Name = 'a window of pictures'; Close = 'Ok'; Script = "Show-GuiWindow -Title '<title>' { Markdown -Path '<pictures>/doc.md' -Name Doc; Image -Path '<pictures>/bar.svg' -Name Bar; ButtonBar { Button 'OK' -Name Ok -Default -Result Ok } }" }
        @{ Name = 'a grid of pictures'; Close = 'Ok'; Script = "Show-GuiWindow -Title '<title>' { Grid -Columns 2 { Image -Path '<pictures>/box.svg'; Image -Path '<pictures>/bar.svg' }; ButtonBar { Button 'OK' -Name Ok -Default -Result Ok } }" }
    ) {
        $shown = $Script.Replace('<pictures>', $script:Pictures)
        $egui = Read-Size Egui "PWRS-GUI test fit egui $Name" $shown $Close
        $html = Read-Size Html "PWRS-GUI test fit html $Name" $shown $Close
        $within = 2 * $script:Scale + 1
        [math]::Abs($html[0] - $egui[0]) | Should -BeLessOrEqual $within -Because "egui opened it $($egui[0]) by $($egui[1]) pixels and HTML $($html[0]) by $($html[1])"
        [math]::Abs($html[1] - $egui[1]) | Should -BeLessOrEqual $within -Because "egui opened it $($egui[0]) by $($egui[1]) pixels and HTML $($html[0]) by $($html[1])"
    }

    # A window fitted to a line exactly gives no scroll bar room it does not
    # need: one would narrow the line until it wrapped.
    It 'shows the line that sets the width of <Name> on one line in both' -ForEach @(
        @{ Name = 'a window of a line, a list and a button'; Lines = @('Line'); Content = {
                Text 'PWRS-GUI test: this line sets the width of the window it opens in' -Name Line
                ListBox 'alpha', 'beta', 'gamma'
                Row { Button 'OK' }
            }
        }
        @{ Name = 'a split of two lines over lists'; Lines = @('Top', 'Bottom'); Content = {
                Split -Orientation Vertical {
                    Column { Text 'PWRS-GUI test: this line sets the width of the window it opens in' -Name Top; ListBox 'alpha', 'beta', 'gamma' }
                    Column { Text 'PWRS-GUI test: this line sets the width of the window it opens in' -Name Bottom; ListBox 'delta', 'epsilon', 'zeta' }
                }
            }
        }
        @{ Name = 'a tab of a line over a list'; Lines = @('Line'); Content = {
                Tabs { Tab 'One' { Text 'PWRS-GUI test: this line sets the width of the window it opens in' -Name Line; ListBox 'alpha', 'beta', 'gamma' } }
            }
        }
    ) {
        $off = foreach ($renderer in 'Egui', 'Html') {
            $alone = (Read-Fitted $renderer "PWRS-GUI test fit line $renderer" { Text 'PWRS-GUI test: this line sets the width of the window it opens in' -Name Line }).Line
            $shown = Read-Fitted $renderer "PWRS-GUI test fit lines $renderer $Name" $Content
            foreach ($name in $Lines) {
                $box = $shown[$name]
                if ([math]::Abs($box.Height - $alone.Height) -gt 0.5) {
                    '{0} {1}: {2} by {3} points in the window, {4} by {5} alone' -f $renderer, $name, $box.Width, $box.Height, $alone.Width, $alone.Height
                }
            }
        }
        $off | Should -BeNullOrEmpty -Because 'each line stays on the one row it takes alone'
    }

    It 'replies to a snapshot requested before a fitted window is shown with the window as shown, in both' {
        $off = foreach ($renderer in 'Egui', 'Html') {
            $title = "PWRS-GUI test fit early snapshot $renderer"
            $w = Show-GuiWindow -NoWait -Title $title -Renderer $renderer -Content {
                Text 'PWRS-GUI test: this line sets the width of the window it opens in' -Name Line
                ListBox 'alpha', 'beta', 'gamma' -Name Choice
            }
            try {
                $early = (Get-GuiSnapshot -Window $w.Id).Bounds
                $null = Wait-GuiWindow $title
                $shown = (Get-GuiSnapshot -Window $w.Id).Bounds
                foreach ($name in 'Line', 'Choice') {
                    $a = $early[$name]
                    $b = $shown[$name]
                    if (-not $a -or [math]::Abs($a.Width - $b.Width) -gt 0.5 -or [math]::Abs($a.Height - $b.Height) -gt 0.5) {
                        '{0} {1}: {2} by {3} points requested at once, {4} by {5} once listed' -f $renderer, $name, $a.Width, $a.Height, $b.Width, $b.Height
                    }
                }
            } finally {
                Close-GuiWindow -Window $w.Id
            }
        }
        $off | Should -BeNullOrEmpty -Because 'a snapshot requested before the window is shown reads the window as shown'
    }
}

Describe 'The engine a page is drawn in, in <_> mode' -ForEach $modes {
    BeforeAll {
        Reset-GuiSession $_
    }

    It 'is named with its version and folder, with the browser''s keys, zoom, gestures and menus off' {
        $w = Show-GuiWindow -NoWait -Title 'PWRS-GUI test engine' -Renderer Html { Button 'OK' -Name Ok }
        try {
            $engine = (Get-GuiSnapshot -Window $w.Id).Engine
        } finally {
            Close-GuiWindow -Window $w.Id
        }
        $engine.Name | Should -Not -BeNullOrEmpty
        $engine.Version | Should -Match '^\d+\.'
        Test-Path -LiteralPath $engine.Folder | Should -BeTrue -Because "the engine's folder is $($engine.Folder)"
        $s = $engine.Settings
        if ($IsWindows -or $PSVersionTable.PSEdition -eq 'Desktop') {
            foreach ($key in 'AreBrowserAcceleratorKeysEnabled', 'IsZoomControlEnabled', 'IsPinchZoomEnabled', 'IsSwipeNavigationEnabled', 'IsStatusBarEnabled', 'AreDefaultContextMenusEnabled', 'AreDevToolsEnabled') {
                $s.$key | Should -BeFalse -Because "WebView2's $key is off for a widget page"
            }
        } elseif ($IsMacOS) {
            $s.allowsMagnification | Should -BeFalse
            $s.allowsBackForwardNavigationGestures | Should -BeFalse
        } else {
            $s.'enable-back-forward-navigation-gestures' | Should -BeFalse
            $s.'enable-developer-extras' | Should -BeFalse
            $s.'default-context-menu' | Should -BeFalse
        }
    }

    # WebKit on macOS reads back no developer setting of a webview.
    It 'leaves the developer keys and the default menus on for a page shown with -DevTools' -Skip:$IsMacOS {
        $w = Show-GuiWindow -NoWait -Title 'PWRS-GUI test engine tools' -DevTools -Html '<p>tools</p>'
        try {
            $s = (Get-GuiSnapshot -Window $w.Id).Engine.Settings
        } finally {
            Close-GuiWindow -Window $w.Id
        }
        if ($IsWindows -or $PSVersionTable.PSEdition -eq 'Desktop') {
            $s.AreBrowserAcceleratorKeysEnabled | Should -BeTrue
            $s.AreDefaultContextMenusEnabled | Should -BeTrue
            $s.AreDevToolsEnabled | Should -BeTrue
            $s.IsZoomControlEnabled | Should -BeFalse -Because 'everything but the developer keys and menus stays as for any page'
        } else {
            $s.'enable-developer-extras' | Should -BeTrue
            $s.'default-context-menu' | Should -BeTrue
            $s.'enable-back-forward-navigation-gestures' | Should -BeFalse
        }
    }
}

Describe 'A widget page under the C locale' {
    # A child host's web helper starts with LANG=C.UTF-8 and no other locale
    # variable, where WebKitGTK would give pages the language "C", which no
    # Intl constructor takes. The child writes each record on one line.
    It 'gives its pages the language en-US, so a chart draws' -Skip:($onWindows -or $IsMacOS) {
        $names = 'LANG', 'LC_ALL', 'LC_MESSAGES', 'LANGUAGE'
        $saved = @{}
        foreach ($name in $names) {
            $saved[$name] = [Environment]::GetEnvironmentVariable($name)
            if (Test-Path -LiteralPath "Env:$name") { Remove-Item -LiteralPath "Env:$name" }
        }
        $env:LANG = 'C.UTF-8'
        $child = @"
. '$PSScriptRoot/Common.ps1'
Reset-GuiSession Helper Html
'LANGUAGE ' + (Show-GuiWindow -Title 'PWRS-GUI test C locale page' -Html '<script>window.pwrs.close(navigator.language);</script>').Result
`$job = Start-Background "Show-GuiWindow -Title 'PWRS-GUI test C locale chart' { Chart -Name Load -Height 160 -Series @{ Name = 'cpu'; Y = 3, 5, 4 }; Button 'OK' -Name Ok -Result Ok }"
`$w = Wait-GuiWindow 'PWRS-GUI test C locale chart' `$job
Invoke-GuiTestEvent -Window `$w.Id -Name Ok -Event Click
`$done = Complete-Background `$job
'CHART ' + `$done.Output[-1].Result + ' WARNINGS ' + ((`$done.Warnings -join ' | ') -replace '\r?\n', ' ')
"@
        try {
            $read = @(& (Get-Process -Id $PID).Path -NoProfile -NonInteractive -Command $child)
        } finally {
            foreach ($name in $names) {
                if ($null -ne $saved[$name]) { Set-Item -LiteralPath "Env:$name" -Value $saved[$name] } elseif (Test-Path -LiteralPath "Env:$name") { Remove-Item -LiteralPath "Env:$name" }
            }
        }
        $text = $read -join "`n"
        ($read | Where-Object { $_ -like 'LANGUAGE *' }) | Should -Be 'LANGUAGE en-US' -Because $text
        ($read | Where-Object { $_ -like 'CHART *' }) | Should -Be 'CHART Ok WARNINGS ' -Because $text
    }
}

Describe 'An engine the test matrix runs' {
    # A child host starts an HTML session of its own, so the warnings its
    # first window raises include the hello's notes. The child writes one
    # line. Its script has no double quote, which Windows PowerShell 5.1
    # drops from a native command's argument.
    It 'opens a page with no warning that the engine is older than the oldest the tests run' {
        $child = @"
. '$PSScriptRoot/Common.ps1'
Reset-GuiSession Helper Html
`$job = Start-Background 'Show-GuiWindow -Title ''PWRS-GUI test floor'' { Button ''OK'' -Name Ok -Result Ok }'
`$w = Wait-GuiWindow 'PWRS-GUI test floor' `$job
Invoke-GuiTestEvent -Window `$w.Id -Name Ok -Event Click
`$done = Complete-Background `$job
'WINDOW ' + `$done.Output[-1].Result + ' WARNINGS ' + ((`$done.Warnings -join ' | ') -replace '\r?\n', ' ')
"@
        $read = @(& (Get-Process -Id $PID).Path -NoProfile -NonInteractive -Command $child)
        $text = $read -join "`n"
        $line = $read | Where-Object { $_ -like 'WINDOW *' }
        $line | Should -BeLike 'WINDOW Ok WARNINGS *' -Because $text
        $line | Should -Not -BeLike "*the oldest PWRS-GUI's tests run*" -Because $text
    }
}

Describe 'A page drawn on the WebView2 add-on''s runtime, in <_> mode' -ForEach $modes {
    # PWRS_GUI_TEST_WEBVIEW2_ADDON names a version folder of
    # PWRS-GUI.WebView2.win-x64 whose Install-GuiWebView2Runtime has run,
    # which the Windows gate gives. A child host names it in
    # PWRS_GUI_WEBVIEW2_ADDON and writes one line.
    It 'runs on the add-on''s Fixed Version runtime, named so, with its version and folder' -Skip:(-not $onWindows -or -not $env:PWRS_GUI_TEST_WEBVIEW2_ADDON) {
        $mode = $_
        $addon = $env:PWRS_GUI_TEST_WEBVIEW2_ADDON
        $version = (Import-PowerShellDataFile -Path (Join-Path $addon 'PWRS-GUI.WebView2.win-x64.psd1')).ModuleVersion
        $child = @"
`$env:PWRS_GUI_WEBVIEW2_ADDON = '$addon'
. '$PSScriptRoot/Common.ps1'
Reset-GuiSession $mode Html
`$w = Show-GuiWindow -NoWait -Title 'PWRS-GUI test fixed runtime' { Button 'OK' -Name Ok }
try { `$e = (Get-GuiSnapshot -Window `$w.Id).Engine; `$o = Get-GuiOption } finally { Close-GuiWindow -Window `$w.Id }
'ENGINE ' + `$o.Backend + ' | ' + `$e.Version + ' | ' + `$e.Folder
"@
        $read = @(& (Get-Process -Id $PID).Path -NoProfile -NonInteractive -Command $child)
        $text = $read -join "`n"
        ($read | Where-Object { $_ -like 'ENGINE *' }) | Should -Be ('ENGINE WebView2 Fixed Version | {0} | {1}' -f $version, (Join-Path $addon 'runtime')) -Because $text
    }
}

Describe 'A page drawn where the WebKitGTK add-on is installed' {
    # PWRS_GUI_TEST_WEBKIT_ADDON names a version folder of the platform's
    # PWRS-GUI.WebKit add-on, which the guests' gates give. A child host finds
    # the add-on on PSModulePath, as Install-Module leaves it, with
    # PWRS_GUI_WEBKIT_ADDON unset, and writes one line.
    It 'draws with the add-on''s WebKitGTK rather than the system''s' -Skip:($onWindows -or $IsMacOS -or -not $env:PWRS_GUI_TEST_WEBKIT_ADDON) {
        $addon = $env:PWRS_GUI_TEST_WEBKIT_ADDON
        $modules = Split-Path -Parent (Split-Path -Parent $addon)
        $child = @"
if (Test-Path Env:PWRS_GUI_WEBKIT_ADDON) { Remove-Item Env:PWRS_GUI_WEBKIT_ADDON }
`$env:PSModulePath = '$modules' + [IO.Path]::PathSeparator + `$env:PSModulePath
. '$PSScriptRoot/Common.ps1'
Reset-GuiSession Helper Html
`$w = Show-GuiWindow -NoWait -Title 'PWRS-GUI test WebKitGTK add-on' { Button 'OK' -Name Ok }
try { `$e = (Get-GuiSnapshot -Window `$w.Id).Engine } finally { Close-GuiWindow -Window `$w.Id }
'ENGINE ' + `$e.Version + ' | ' + `$e.Folder
"@
        $read = @(& (Get-Process -Id $PID).Path -NoProfile -NonInteractive -Command $child)
        $text = $read -join "`n"
        ($read | Where-Object { $_ -like 'ENGINE *' }) | Should -BeLike "ENGINE * | $addon/*" -Because "the add-on in $addon is preferred over the system's WebKitGTK: $text"
    }
}
