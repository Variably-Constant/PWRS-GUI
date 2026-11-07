# The system's dialogs: Show-GuiMessage, Read-GuiFile, Read-GuiSavePath and
# Read-GuiFolder, found with Get-GuiDialog and answered with
# Invoke-GuiTestEvent -Dialog as a person would, in every mode the platform
# has and with each renderer; and the refusals of answers a dialog cannot
# take. See Common.ps1 for the deadlines.

BeforeDiscovery {
    $modes = if ($IsWindows -or $PSVersionTable.PSEdition -eq 'Desktop') { 'Helper', 'InProcess' } else { 'Helper' }
    $cases = foreach ($m in $modes) { foreach ($r in 'Egui', 'Html') { @{ Mode = $m; Renderer = $r } } }
}

BeforeAll {
    . (Join-Path $PSScriptRoot 'Common.ps1')
    $script:Watchdog = Start-Watchdog
    # Presses the Browse button of a FilePicker in a window opened with
    # -NoWait, answers the dialog it opens with $Path, and returns the
    # chooser's Change event.
    function Invoke-PickerBrowse([string]$Prefix, [string]$Path) {
        $w = Show-GuiWindow -NoWait -Title "$Prefix window" { FilePicker -Name Log -Title "$Prefix dialog" }
        $null = Wait-GuiWindow "$Prefix window"
        Invoke-GuiTestEvent -Window $w.Id -Name Log -Event Click
        $deadline = [DateTime]::UtcNow.AddSeconds($script:WindowSeconds)
        while (-not ($d = Get-GuiDialog -Title "$Prefix dialog") -and [DateTime]::UtcNow -lt $deadline) {
            Start-Sleep -Milliseconds 50
        }
        if (-not $d) { throw "pressing Browse opened no dialog titled '$Prefix dialog' within $($script:WindowSeconds) s" }
        $d.Kind | Should -Be 'Open'
        Invoke-GuiTestEvent -Dialog $d.Id -Value $Path
        $changed = Wait-GuiEvent -Window $w.Id -Name Log -Timeout $script:WindowSeconds
        Close-GuiWindow -Window $w.Id
        $changed
    }
    # The open dialog titled $Title; fails, naming what the background
    # command reported, when none opens in time.
    function Wait-GuiDialog([string]$Title, $Job) {
        $deadline = [DateTime]::UtcNow.AddSeconds($script:WindowSeconds)
        while ([DateTime]::UtcNow -lt $deadline) {
            $dialog = Get-GuiDialog -Title $Title
            if ($dialog) { return $dialog }
            if ($Job.Async.IsCompleted) { break }
            Start-Sleep -Milliseconds 50
        }
        $why = " The background command is $($Job.PS.InvocationStateInfo.State): $($Job.PS.InvocationStateInfo.Reason) $(@($Job.PS.Streams.Error) -join '; ')"
        if (-not $Job.Async.IsCompleted) { $Job.PS.Stop() }
        throw "No dialog titled '$Title' within $($script:WindowSeconds) s.$why"
    }
    $script:Here = [IO.Path]::GetTempPath()
}

AfterAll {
    Stop-Watchdog $script:Watchdog
}

Describe 'Invoke-GuiTestEvent -Dialog' {
    It 'refuses a dialog that is not open' {
        { Invoke-GuiTestEvent -Dialog 999999 -Value Ok } | Should -Throw -ErrorId 'GuiNoDialog*'
    }
}

Describe 'The system''s dialogs in <Mode> mode drawn with <Renderer>' -ForEach $cases {
    BeforeAll {
        Reset-GuiSession $Mode $Renderer
    }

    It 'writes the button a message is answered with' {
        $job = Start-Background "Show-GuiMessage 'Delete the old logs?' -Title 'PWRS-GUI test message' -Icon Question -Buttons YesNo"
        $d = Wait-GuiDialog 'PWRS-GUI test message' $job
        $d.Kind | Should -Be 'Message'
        Invoke-GuiTestEvent -Dialog $d.Id -Value Yes
        $done = Complete-Background $job
        $done.Errors | Should -BeNullOrEmpty
        "$($done.Output)" | Should -Be 'Yes'
    }

    It 'writes Cancel for a message canceled where it shows Cancel, and No where it shows Yes and No alone' {
        $job = Start-Background "Show-GuiMessage 'Copy again?' -Title 'PWRS-GUI test message cancel' -Icon Warning -Buttons OkCancel"
        $d = Wait-GuiDialog 'PWRS-GUI test message cancel' $job
        Invoke-GuiTestEvent -Dialog $d.Id
        "$((Complete-Background $job).Output)" | Should -Be 'Cancel'
        $job = Start-Background "Show-GuiMessage 'Keep the logs?' -Title 'PWRS-GUI test message no' -Buttons YesNo"
        $d = Wait-GuiDialog 'PWRS-GUI test message no' $job
        Invoke-GuiTestEvent -Dialog $d.Id
        "$((Complete-Background $job).Output)" | Should -Be 'No'
    }

    It 'refuses a button the message does not show, and paths for a message' {
        $job = Start-Background "Show-GuiMessage 'Go on?' -Title 'PWRS-GUI test message refuse' -Buttons OkCancel"
        $d = Wait-GuiDialog 'PWRS-GUI test message refuse' $job
        { Invoke-GuiTestEvent -Dialog $d.Id -Value Yes -ErrorAction Stop } | Should -Throw -ExpectedMessage '*shows*'
        { Invoke-GuiTestEvent -Dialog $d.Id -Value Maybe } | Should -Throw -ErrorId 'GuiBadAnswer*'
        Invoke-GuiTestEvent -Dialog $d.Id -Value Ok
        "$((Complete-Background $job).Output)" | Should -Be 'Ok'
    }

    It 'writes the paths a file dialog is answered with, in the order given' {
        $first = Join-Path $Here 'pwrs-gui-test-first.log'
        $second = Join-Path $Here 'pwrs-gui-test-second.log'
        $job = Start-Background "Read-GuiFile -Filter 'Logs|*.log', 'All files|*' -Multiple -Title 'PWRS-GUI test open'"
        $d = Wait-GuiDialog 'PWRS-GUI test open' $job
        $d.Kind | Should -Be 'Open'
        Invoke-GuiTestEvent -Dialog $d.Id -Value $first, $second
        $done = Complete-Background $job
        $done.Errors | Should -BeNullOrEmpty
        $done.Output | Should -Be @($first, $second)
    }

    It 'refuses two paths for a dialog of one file' {
        $job = Start-Background "Read-GuiFile -Title 'PWRS-GUI test open one'"
        $d = Wait-GuiDialog 'PWRS-GUI test open one' $job
        { Invoke-GuiTestEvent -Dialog $d.Id -Value 'a.txt', 'b.txt' -ErrorAction Stop } | Should -Throw -ExpectedMessage '*one path*'
        Invoke-GuiTestEvent -Dialog $d.Id
        (Complete-Background $job).Output | Should -BeNullOrEmpty
    }

    It 'writes the path a save dialog and a folder dialog are answered with' {
        $file = Join-Path $Here 'pwrs-gui-test-report.csv'
        $job = Start-Background "Read-GuiSavePath -Filter 'Spreadsheets|*.csv' -FileName 'report.csv' -Title 'PWRS-GUI test save'"
        $d = Wait-GuiDialog 'PWRS-GUI test save' $job
        $d.Kind | Should -Be 'Save'
        Invoke-GuiTestEvent -Dialog $d.Id -Value $file
        (Complete-Background $job).Output | Should -Be $file
        $job = Start-Background "Read-GuiFolder -InitialDirectory '$Here' -Title 'PWRS-GUI test folder'"
        $d = Wait-GuiDialog 'PWRS-GUI test folder' $job
        $d.Kind | Should -Be 'Folder'
        Invoke-GuiTestEvent -Dialog $d.Id -Value $Here
        (Complete-Background $job).Output | Should -Be $Here
    }

    It 'opens a FilePicker''s dialog from its Browse button, listed as any other, and the chooser takes its answer' {
        $file = Join-Path $Here 'pwrs-gui-test-picked.log'
        $changed = Invoke-PickerBrowse 'PWRS-GUI test picker' $file
        $changed.Kind | Should -Be 'Change'
        $changed.Value | Should -Be $file
    }

    It 'writes nothing for a file dialog that is canceled' {
        $job = Start-Background "Read-GuiFolder -Title 'PWRS-GUI test folder cancel'"
        $d = Wait-GuiDialog 'PWRS-GUI test folder cancel' $job
        Invoke-GuiTestEvent -Dialog $d.Id
        $done = Complete-Background $job
        $done.Errors | Should -BeNullOrEmpty
        $done.Output | Should -BeNullOrEmpty
    }

    It 'closes its dialog and writes nothing when stopped as Ctrl+C stops it' -ForEach @(
        @{ Command = "Show-GuiMessage 'Wait' -Title 'PWRS-GUI test stop message'"; Title = 'PWRS-GUI test stop message' }
        @{ Command = "Read-GuiFile -Title 'PWRS-GUI test stop open'"; Title = 'PWRS-GUI test stop open' }
    ) {
        $job = Start-Background $Command
        $null = Wait-GuiDialog $Title $job
        $done = Stop-Background $job
        $done.Output | Should -BeNullOrEmpty
        Wait-Until { -not (Get-GuiDialog -Title $Title) } 'the stopped command''s dialog closing'
    }

    It 'leaves no dialog showing when its command is stopped as the dialog opens' {
        $job = Start-Background "'asking'; Read-GuiFile -Title 'PWRS-GUI test stop early'"
        Wait-Until { $job.Output.Count -gt 0 } 'the background command reaching Read-GuiFile'
        $done = Stop-Background $job
        $done.Output | Should -Be 'asking'
        # A dialog requested after the stopped one shows after it would have.
        $later = Start-Background "Read-GuiFolder -Title 'PWRS-GUI test stop later'"
        $d = Wait-GuiDialog 'PWRS-GUI test stop later' $later
        Wait-Until { -not (Get-GuiDialog -Title 'PWRS-GUI test stop early') } 'the dialog of the command stopped as it opened closing'
        Invoke-GuiTestEvent -Dialog $d.Id
        $null = Complete-Background $later
    }
}

# With zenity off PATH, and no XDG portal on Xvfb, the system has no dialog
# of any kind, and the egui helper draws its own: in an HTML session too,
# which passes the dialog to its egui session.
Describe 'The renderer''s own dialogs where the system has none, in a session drawn with <Renderer>' -ForEach @(@{ Renderer = 'Egui' }, @{ Renderer = 'Html' }) -Skip:($IsWindows -or $IsMacOS -or $PSVersionTable.PSEdition -eq 'Desktop') {
    BeforeAll {
        $script:PathBefore = $env:PATH
        $separator = [IO.Path]::PathSeparator
        $env:PATH = ($env:PATH -split $separator | Where-Object { $_ -and -not (Test-Path -LiteralPath (Join-Path $_ 'zenity')) }) -join $separator
        Reset-GuiSession 'Helper' $Renderer
    }

    AfterAll {
        $env:PATH = $script:PathBefore
        Reset-GuiSession 'Helper' 'Egui'
    }

    It 'writes the button its message is answered with' {
        $job = Start-Background "Show-GuiMessage 'Drawn by the renderer' -Title 'PWRS-GUI test own message' -Icon Warning -Buttons YesNoCancel"
        $d = Wait-GuiDialog 'PWRS-GUI test own message' $job
        $d.Kind | Should -Be 'Message'
        Invoke-GuiTestEvent -Dialog $d.Id -Value No
        $done = Complete-Background $job
        $done.Errors | Should -BeNullOrEmpty
        "$($done.Output)" | Should -Be 'No'
    }

    It 'writes the paths its file dialogs are answered with' {
        $first = Join-Path $Here 'pwrs-gui-test-own.log'
        $job = Start-Background "Read-GuiFile -Filter 'Logs|*.log' -Multiple -Title 'PWRS-GUI test own open'"
        $d = Wait-GuiDialog 'PWRS-GUI test own open' $job
        $d.Kind | Should -Be 'Open'
        Invoke-GuiTestEvent -Dialog $d.Id -Value $first
        $done = Complete-Background $job
        $done.Errors | Should -BeNullOrEmpty
        $done.Output | Should -Be $first
        $job = Start-Background "Read-GuiFolder -InitialDirectory '$Here' -Title 'PWRS-GUI test own folder'"
        $d = Wait-GuiDialog 'PWRS-GUI test own folder' $job
        Invoke-GuiTestEvent -Dialog $d.Id -Value $Here
        (Complete-Background $job).Output | Should -Be $Here
    }

    It 'draws a FilePicker''s dialog, an HTML window''s through its command, and the chooser takes its answer' {
        $file = Join-Path $Here 'pwrs-gui-test-own-picked.log'
        $changed = Invoke-PickerBrowse 'PWRS-GUI test own picker' $file
        $changed.Kind | Should -Be 'Change'
        $changed.Value | Should -Be $file
    }

    It 'closes the dialog it draws and writes nothing when stopped as Ctrl+C stops it' {
        $job = Start-Background "Read-GuiSavePath -Title 'PWRS-GUI test own stop'"
        $null = Wait-GuiDialog 'PWRS-GUI test own stop' $job
        $done = Stop-Background $job
        $done.Output | Should -BeNullOrEmpty
        Wait-Until { -not (Get-GuiDialog -Title 'PWRS-GUI test own stop') } 'the stopped command''s dialog closing'
    }
}
