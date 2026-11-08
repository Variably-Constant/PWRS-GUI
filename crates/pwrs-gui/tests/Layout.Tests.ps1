# The dialog layout the widget set gives, in every mode the platform has and
# with each renderer: a ButtonBar's buttons at the bottom right in the
# system's order and at least a dialog button's size, -Align, -Fill and the
# share of a Column's height its lists take, a window with a menu bar or a
# tree fitted to its widgets, a log at the size it is given, the theme
# Set-GuiOption gives, and every ready-made dialog opening at its size with
# no warning. Bounds are in points from the top left of a window's inside;
# a window's size in points is its capture's pixels over the display's
# scale, which a window of a given size measures. See Common.ps1 for the
# deadlines.

BeforeDiscovery {
    $modes = if ($IsWindows -or $PSVersionTable.PSEdition -eq 'Desktop') { 'Helper', 'InProcess' } else { 'Helper' }
    $cases = foreach ($m in $modes) { foreach ($r in 'Egui', 'Html') { @{ Mode = $m; Renderer = $r } } }
}

BeforeAll {
    . (Join-Path $PSScriptRoot 'Common.ps1')
    $script:Watchdog = Start-Watchdog
    # Windows puts a dialog's default button first; macOS, Linux and the
    # BSDs put it last.
    $script:WindowsOrder = $IsWindows -or $PSVersionTable.PSEdition -eq 'Desktop'

    # The bounds of the named widgets of window $Window once two reads
    # 300 ms apart agree, every name in $Names among them.
    function Read-Bounds($Window, [string[]] $Names) {
        $last = $null
        $deadline = [DateTime]::UtcNow.AddSeconds(20)
        while ([DateTime]::UtcNow -lt $deadline) {
            Start-Sleep -Milliseconds 300
            $bounds = (Get-GuiSnapshot -Window $Window.Id).Bounds
            $now = $bounds | ConvertTo-Json -Compress -Depth 4
            $all = -not @($Names | Where-Object { -not $bounds.Contains($_) })
            if ($all -and $now -eq $last) { return $bounds }
            $last = $now
        }
        throw "the bounds of window $($Window.Id) did not settle with $($Names -join ', ') within 20 s; last read $last"
    }

    # The red, green and blue of pixel ($X, $Y) of a capture.
    function Get-Pixel($Shot, [int] $X, [int] $Y) {
        $at = ($Y * $Shot.Width + $X) * 4
        @($Shot.Pixels[$at], $Shot.Pixels[$at + 1], $Shot.Pixels[$at + 2])
    }
}

AfterAll {
    Stop-Watchdog $script:Watchdog
}

Describe 'The widgets a dialog is built from, refused where they cannot be' {
    BeforeAll {
        Reset-GuiSession Helper
    }

    It 'refuses a button that is both the default and the cancel button' {
        { New-GuiButton 'OK' -Default -Cancel } | Should -Throw -ErrorId 'GuiButtonRole*'
    }

    It 'refuses a window with two button bars, two default buttons or two cancel buttons' {
        { Show-GuiWindow { Text 'two bars'; ButtonBar { Button 'A' }; ButtonBar { Button 'B' } } } | Should -Throw -ErrorId 'GuiButtonRole*'
        { Show-GuiWindow { Text 'two defaults'; ButtonBar { Button 'Save' -Default; Button 'Save All' -Default } } } | Should -Throw -ErrorId 'GuiButtonRole*'
        { Show-GuiWindow { Button 'Stop' -Cancel; ButtonBar { Button 'Close' -Cancel } } } | Should -Throw -ErrorId 'GuiButtonRole*'
    }

    It 'refuses a bar with a widget that is not a button, and a bar inside another widget' {
        { Show-GuiWindow { Text 'a text in the bar'; ButtonBar { Text 'not a button' } } } | Should -Throw -ErrorId 'GuiNotAButton*'
        { Show-GuiWindow { Column { Text 'inside'; ButtonBar { Button 'OK' } } } } | Should -Throw -ErrorId 'GuiNotAtTop*'
    }
}

Describe 'A dialog laid out in <Mode> mode drawn with <Renderer>' -ForEach $cases {
    BeforeAll {
        Reset-GuiSession $Mode $Renderer
        $script:Renderer = $Renderer
        $w = Show-GuiWindow -NoWait -Title 'PWRS-GUI test scale' -Renderer $Renderer -Width 400 -Height 200 { Text 'measures the scale' -Name Scale }
        try {
            $script:Scale = (Get-GuiSnapshot -Window $w.Id -Pixels).Width / 400
        } finally {
            Close-GuiWindow -Window $w.Id
        }
    }

    It 'puts the bar''s buttons at the bottom right in the system''s order, 8 points apart and each at least 80 by 24' {
        $w = Show-GuiWindow -NoWait -Title 'PWRS-GUI test bar' -Renderer $script:Renderer -Width 400 -Height 200 {
            Text 'Delete the old logs?' -Style Instruction -Name Ask
            ButtonBar { Button 'Delete' -Name Ok -Default -Result Ok; Button 'Cancel' -Name Cancel -Cancel -Result Cancel }
        }
        try {
            $b = Read-Bounds $w 'Ok', 'Cancel', 'Ask'
            $ok = $b['Ok']
            $cancel = $b['Cancel']
            foreach ($button in $ok, $cancel) {
                $button.Width | Should -BeGreaterOrEqual 79.5
                $button.Height | Should -BeGreaterOrEqual 23.5
                [math]::Abs($button.Y + $button.Height - 188) | Should -BeLessOrEqual 2 -Because 'the bar draws its buttons inside a 12-point margin at the window''s bottom'
            }
            $first, $last = if ($script:WindowsOrder) { $ok, $cancel } else { $cancel, $ok }
            $last.X | Should -BeGreaterThan $first.X -Because 'the system''s order puts the default button first on Windows and last elsewhere'
            [math]::Abs($last.X + $last.Width - 388) | Should -BeLessOrEqual 2 -Because 'the last button ends at the bar''s 12-point margin'
            [math]::Abs($last.X - ($first.X + $first.Width) - 8) | Should -BeLessOrEqual 1.5 -Because 'the buttons are 8 points apart'
            $b['Ask'].Y | Should -BeLessThan $first.Y
        } finally {
            Close-GuiWindow -Window $w.Id
        }
    }

    It 'places widgets across as -Align says, and a Spacer -Fill takes the room its Row leaves' {
        $w = Show-GuiWindow -NoWait -Title 'PWRS-GUI test align' -Renderer $script:Renderer -Width 400 -Height 240 {
            Row -Align End -Name Right { Button 'At the end' -Name A }
            Row -Align Center -Name Middle { Button 'In the middle' -Name B }
            Row -Name Left { Button 'At the start' -Name C }
            Column -Align Stretch -Name Wide { Button 'Stretched' -Name D }
            Row -Name Steps { Text 'Step 1 of 3' -Name Step; Spacer -Fill; Button 'Next' -Name Next }
        }
        try {
            $b = Read-Bounds $w 'A', 'B', 'C', 'D', 'Next', 'Step'
            [math]::Abs($b['A'].X + $b['A'].Width - 392) | Should -BeLessOrEqual 2 -Because '-Align End puts the row''s widgets at the right of the 8-point margin'
            [math]::Abs($b['B'].X + $b['B'].Width / 2 - 200) | Should -BeLessOrEqual 2 -Because '-Align Center puts them in the middle'
            [math]::Abs($b['C'].X - 8) | Should -BeLessOrEqual 2 -Because 'a row places its widgets at the left by default'
            [math]::Abs($b['D'].Width - 384) | Should -BeLessOrEqual 2 -Because '-Align Stretch makes a column''s widgets as wide as it'
            [math]::Abs($b['Next'].X + $b['Next'].Width - 392) | Should -BeLessOrEqual 2 -Because 'a Spacer -Fill takes the room the row''s other widgets leave'
            [math]::Abs($b['Step'].X - 8) | Should -BeLessOrEqual 2
        } finally {
            Close-GuiWindow -Window $w.Id
        }
    }

    It 'shares the height a Column leaves between the lists in it, the widgets after them still in the window' {
        $w = Show-GuiWindow -NoWait -Title 'PWRS-GUI test fill' -Renderer $script:Renderer -Width 300 -Height 300 {
            Text 'top' -Name Top
            ListBox 'a', 'b' -Name One
            ListBox 'c', 'd' -Name Two
            Text 'bottom' -Name Bottom
        }
        try {
            $b = Read-Bounds $w 'Top', 'One', 'Two', 'Bottom'
            [math]::Abs($b['One'].Height - $b['Two'].Height) | Should -BeLessOrEqual 2 -Because 'the two lists share the room equally'
            $b['One'].Height | Should -BeGreaterOrEqual 60 -Because 'a list that fills takes three rows of controls at least'
            [math]::Abs($b['Bottom'].Y + $b['Bottom'].Height - 292) | Should -BeLessOrEqual 2 -Because 'the lists leave the text after them in the window, above its 8-point margin'
        } finally {
            Close-GuiWindow -Window $w.Id
        }
    }

    It 'fits a window with <Name> to its widgets, not to the screen' -ForEach @(
        @{ Name = 'a menu bar'; Content = { MenuBar { Menu 'Edit' { MenuItem 'Find' -Name Find } }; TextBox -Name Filter } }
        @{ Name = 'a tree'; Content = { TreeView 'web01', 'web02', 'db01' -Name Sources } }
    ) {
        $w = Show-GuiWindow -NoWait -Title "PWRS-GUI test fit $Name" -Renderer $script:Renderer $Content
        try {
            $null = Read-Bounds $w @()
            $across = (Get-GuiSnapshot -Window $w.Id -Pixels).Width / $script:Scale
            $across | Should -BeLessOrEqual 321 -Because "$Name narrower than the least width opens its window at that width, 320 points"
        } finally {
            Close-GuiWindow -Window $w.Id
        }
    }

    It 'draws a table''s filter row only with -Filter, the window fitted to it' {
        $rows = 1..4 | ForEach-Object { [pscustomobject]@{ Name = "row$_" } }
        $plain = Show-GuiWindow -NoWait -Title 'PWRS-GUI test table plain' -Renderer $script:Renderer { Table $rows -Name Lines }
        $filtered = Show-GuiWindow -NoWait -Title 'PWRS-GUI test table filter' -Renderer $script:Renderer { Table $rows -Name Lines -Filter }
        try {
            $without = (Read-Bounds $plain 'Lines')['Lines'].Height
            $with = (Read-Bounds $filtered 'Lines')['Lines'].Height
            [math]::Abs($with - $without - 23) | Should -BeLessOrEqual 1 -Because "the filter row is an interactive widget's 20 points with a 3-point gap under it; the table measured $without without it and $with with it"
        } finally {
            Close-GuiWindow -Window $plain.Id
            Close-GuiWindow -Window $filtered.Id
        }
    }

    It 'gives a log the width and height it is given, however few or many lines it has' {
        $w = Show-GuiWindow -NoWait -Title 'PWRS-GUI test log size' -Renderer $script:Renderer { LogView -Name Log -Width 400 -Height 120 }
        try {
            $b = Read-Bounds $w 'Log'
            [math]::Abs($b['Log'].Width - 400) | Should -BeLessOrEqual 1 -Because 'an empty log is as wide as -Width'
            [math]::Abs($b['Log'].Height - 120) | Should -BeLessOrEqual 1 -Because 'an empty log is as high as -Height'
            Add-GuiItem -Window $w.Id -Name Log -Item (1..50 | ForEach-Object { "line $_ of a log that grows past its height" })
            Wait-Until {
                $nodes = [System.Collections.Generic.Stack[object]]::new()
                $nodes.Push((Get-GuiSnapshot -Window $w.Id).Tree)
                $count = 0
                while ($nodes.Count -gt 0) {
                    $node = $nodes.Pop()
                    if ($node.Name -eq 'Log') { $count = @($node.Properties.lines).Count }
                    foreach ($child in $node.Children) { $nodes.Push($child) }
                }
                $count -eq 50
            } 'the fifty lines reaching the log'
            $b = Read-Bounds $w 'Log'
            [math]::Abs($b['Log'].Height - 120) | Should -BeLessOrEqual 1 -Because 'lines past its height scroll in the room it has'
        } finally {
            Close-GuiWindow -Window $w.Id
        }
    }

    It 'opens in the theme Set-GuiOption -Theme names, whatever the system''s setting' -ForEach @(
        @{ Theme = 'Dark'; Panel = 27 }
        @{ Theme = 'Light'; Panel = 248 }
    ) {
        Set-GuiOption -Theme $Theme
        try {
            $w = Show-GuiWindow -NoWait -Title "PWRS-GUI test theme $Theme" -Renderer $script:Renderer -Width 200 -Height 100 { Text 'in a theme' -Name Themed }
            try {
                $null = Read-Bounds $w 'Themed'
                $shot = Get-GuiSnapshot -Window $w.Id -Pixels
                Get-Pixel $shot 2 2 | Should -Be @($Panel, $Panel, $Panel) -Because "the $Theme theme's window is drawn in gray $Panel"
            } finally {
                Close-GuiWindow -Window $w.Id
            }
        } finally {
            Set-GuiOption -Theme System
        }
    }

    It 'opens <Name> at its size, with no warning' -ForEach @(
        @{ Name = 'a choice'; Title = 'PWRS-GUI test fit choice'; Least = 320; Most = 548; Close = 'Cancel'
            Script = "Read-GuiChoice -Prompt 'Pick one' -Choice 'alpha', 'beta', 'gamma' -Title '<title>'" }
        @{ Name = 'a form'; Title = 'PWRS-GUI test fit input'; Least = 320; Most = 548; Close = 'Cancel'
            Script = "Read-GuiInput -Prompt 'New user' -Field @{ Name = 'Login' }, @{ Name = 'Count'; Type = 'Number'; Default = 3 }, @{ Name = 'Admin'; Label = 'Make an administrator'; Type = 'Check' } -Title '<title>'" }
        @{ Name = 'a credential form'; Title = 'PWRS-GUI test fit credential'; Least = 320; Most = 548; Close = 'Cancel'
            Script = "Read-GuiCredential -Message 'Sign in to the build server' -Title '<title>'" }
        @{ Name = 'a grid of objects to pick'; Title = 'PWRS-GUI test fit grid'; Least = 320; Most = 100000; Close = 'Cancel'
            Script = "1..3 | ForEach-Object { [pscustomobject]@{ Name = ""item`$_""; Size = `$_ * 10 } } | Out-GuiGridView -PassThru -Title '<title>'" }
        @{ Name = 'a progress window'; Title = 'PWRS-GUI test fit progress'; Least = 380; Most = 380; Close = 'Window'
            Script = "`$p = Show-GuiProgress 'Copying' -Status 'File 1 of 3' -PercentComplete 30 -Title '<title>'; while (`$p.State -ne 'Closed') { Start-Sleep -Milliseconds 50 }" }
    ) {
        $job = Start-Background ($Script.Replace('<title>', $Title))
        $w = Wait-GuiWindow $Title $job
        $shot = Get-GuiSnapshot -Window $w.Id -Pixels
        $across = $shot.Width / $script:Scale
        $down = $shot.Height / $script:Scale
        $across | Should -BeGreaterOrEqual ($Least - 1)
        $across | Should -BeLessOrEqual ($Most + 1)
        $down | Should -BeLessOrEqual 757 -Because 'no dialog is more than 756 points high'
        if ($Close -eq 'Window') { Close-GuiWindow -Window $w.Id } else { Invoke-GuiTestEvent -Window $w.Id -Name $Close -Event Click }
        $done = Complete-Background $job
        $done.Errors | Should -BeNullOrEmpty
        $done.Warnings | Should -BeNullOrEmpty -Because 'a ready-made dialog draws every widget it is built from'
    }

    It 'opens the latency window with no warning' {
        $job = Start-Background 'Measure-GuiLatency -Count 1'
        $w = Wait-GuiWindow 'PWRS-GUI latency' $job
        Invoke-GuiTestEvent -Window $w.Id -Name Click -Event Click
        $done = Complete-Background $job
        $done.Errors | Should -BeNullOrEmpty
        $done.Warnings | Should -BeNullOrEmpty
    }
}
