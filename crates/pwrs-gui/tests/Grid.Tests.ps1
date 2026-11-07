# Out-GuiGridView, driven as a person would through Invoke-GuiTestEvent, in
# every mode the platform has and with each renderer. See Common.ps1 for
# the deadlines.

BeforeDiscovery {
    $modes = if ($IsWindows -or $PSVersionTable.PSEdition -eq 'Desktop') { 'Helper', 'InProcess' } else { 'Helper' }
    $cases = foreach ($m in $modes) { foreach ($r in 'Egui', 'Html') { @{ Mode = $m; Renderer = $r } } }
}

BeforeAll {
    . (Join-Path $PSScriptRoot 'Common.ps1')
    $script:Watchdog = Start-Watchdog

    # How many rows a grid window has, from its snapshot.
    function script:Get-GridRows([uint64]$Window) {
        (Get-GuiSnapshot -Window $Window).State.Grid.rows
    }

    # The grid's column labels, from its snapshot.
    function script:Get-GridColumns([uint64]$Window) {
        (Find-GuiNode (Get-GuiSnapshot -Window $Window).Tree 'Grid').Properties.columns | ForEach-Object { $_.label }
    }

    # The column labels of the first table view Get-FormatData gives
    # $TypeName: each header's label, or its entry's property or script
    # block where the header has none, as the console's table heads them.
    function script:Get-TableLabels([string]$TypeName) {
        $view = Get-FormatData -TypeName $TypeName -PowerShellVersion $PSVersionTable.PSVersion |
            ForEach-Object FormatViewDefinition | Where-Object { $_.Control -is [System.Management.Automation.TableControl] } | Select-Object -First 1
        if (-not $view) { throw "Get-FormatData gives $TypeName no table view" }
        $headers = $view.Control.Headers
        $entries = $view.Control.Rows[0].Columns
        for ($i = 0; $i -lt $entries.Count; $i++) {
            if ($i -lt $headers.Count -and $headers[$i].Label) { $headers[$i].Label } else { $entries[$i].DisplayEntry.Value }
        }
    }
}

AfterAll {
    Stop-Watchdog $script:Watchdog
}

Describe 'Out-GuiGridView in <Mode> mode drawn with <Renderer>' -ForEach $cases {
    BeforeAll {
        Reset-GuiSession $Mode $Renderer
        $script:Items = "1..5 | ForEach-Object { [pscustomobject]@{ Name = ""item`$_""; Size = `$_ * 10 } }"
    }

    It 'with -PassThru, writes the selected input objects in input order' {
        $job = Start-Background "$script:Items | Out-GuiGridView -Title 'PWRS-GUI test passthru' -PassThru"
        $w = Wait-GuiWindow 'PWRS-GUI test passthru' $job
        Wait-Until { (Get-GridRows $w.Id) -eq 5 } 'all five rows arriving'
        Invoke-GuiTestEvent -Window $w.Id -Name Grid -Event Select -Value 3, 0
        Wait-Until { (Find-GuiNode (Get-GuiSnapshot -Window $w.Id).Tree 'Ok').Properties.enabled } 'OK becoming enabled'
        Invoke-GuiTestEvent -Window $w.Id -Name Ok -Event Click
        $done = Complete-Background $job
        $done.Errors | Should -BeNullOrEmpty
        $done.Output.Name | Should -Be @('item1', 'item4')
        $done.Output[1].Size | Should -Be 40
    }

    It 'with -OutputMode Single, writes the row activated' {
        $job = Start-Background "$script:Items | Out-GuiGridView -Title 'PWRS-GUI test single' -OutputMode Single"
        $w = Wait-GuiWindow 'PWRS-GUI test single' $job
        Wait-Until { (Get-GridRows $w.Id) -eq 5 } 'all five rows arriving'
        Invoke-GuiTestEvent -Window $w.Id -Name Grid -Event Activate -Value 2
        (Complete-Background $job).Output.Name | Should -Be @('item3')
    }

    It 'writes nothing when Cancel is pressed' {
        $job = Start-Background "$script:Items | Out-GuiGridView -Title 'PWRS-GUI test grid cancel' -PassThru"
        $w = Wait-GuiWindow 'PWRS-GUI test grid cancel' $job
        Invoke-GuiTestEvent -Window $w.Id -Name Grid -Event Select -Value 1
        Invoke-GuiTestEvent -Window $w.Id -Name Cancel -Event Click
        $done = Complete-Background $job
        $done.Errors | Should -BeNullOrEmpty
        $done.Output | Should -BeNullOrEmpty
    }

    It 'without -PassThru, returns once its input has ended and its window has opened, and leaves the window open' {
        1..3 | Out-GuiGridView -Title 'PWRS-GUI test detached'
        $w = @(Get-GuiWindow -Title 'PWRS-GUI test detached')
        $w.Count | Should -Be 1 -Because 'the command returns once its window has opened, which is when a window is listed'
        Wait-Until { (Get-GridRows $w[0].Id) -eq 3 } 'all three rows arriving'
        @(Get-GuiWindow -Title 'PWRS-GUI test detached').Count | Should -Be 1
    }

    It 'takes its columns from the table view Get-FormatData gives Get-Process''s objects' {
        $expected = Get-TableLabels System.Diagnostics.Process
        Get-Process -Id $PID | Out-GuiGridView -Title 'PWRS-GUI test format view'
        $w = Wait-GuiWindow 'PWRS-GUI test format view'
        Get-GridColumns $w.Id | Should -Be @($expected)
    }

    It 'takes its columns from the table view Get-FormatData gives Get-ChildItem''s files' {
        $expected = Get-TableLabels System.IO.FileInfo
        Get-ChildItem -LiteralPath $PSHOME -File | Select-Object -First 5 | Out-GuiGridView -Title 'PWRS-GUI test file view'
        $w = Wait-GuiWindow 'PWRS-GUI test file view'
        Get-GridColumns $w.Id | Should -Be @($expected)
    }

    It 'takes the default display properties of a type with no table view' {
        Update-TypeData -TypeName PwrsGui.Test.Displayed -DefaultDisplayPropertySet Name, Size -Force
        [pscustomobject]@{ PSTypeName = 'PwrsGui.Test.Displayed'; Name = 'a'; Hidden = 1; Size = 2 } | Out-GuiGridView -Title 'PWRS-GUI test display set'
        $w = Wait-GuiWindow 'PWRS-GUI test display set'
        Get-GridColumns $w.Id | Should -Be @('Name', 'Size')
    }

    It 'takes every property, in order, from an object with no table view and no display set' {
        [pscustomobject]@{ Zeta = 1; Alpha = 2; Mid = 3 } | Out-GuiGridView -Title 'PWRS-GUI test properties'
        $w = Wait-GuiWindow 'PWRS-GUI test properties'
        Get-GridColumns $w.Id | Should -Be @('Zeta', 'Alpha', 'Mid')
    }

    It 'shows a value type as one column named for its type' {
        1..2 | Out-GuiGridView -Title 'PWRS-GUI test scalar'
        $w = Wait-GuiWindow 'PWRS-GUI test scalar'
        Get-GridColumns $w.Id | Should -Be @('Int32')
    }

    It 'takes a thousand rows, sent in batches while the pipeline runs' {
        1..1000 | ForEach-Object { [pscustomobject]@{ N = $_ } } | Out-GuiGridView -Title 'PWRS-GUI test many'
        $w = Wait-GuiWindow 'PWRS-GUI test many'
        Wait-Until { (Get-GridRows $w.Id) -eq 1000 } 'all thousand rows arriving'
    }
}
