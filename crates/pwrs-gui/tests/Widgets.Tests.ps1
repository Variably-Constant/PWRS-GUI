# Windows described in PowerShell: Show-GuiWindow -Content, its handlers
# and result, and the update commands, in every mode the platform has and
# with each renderer. A window's command runs in a background runspace,
# which the test drives as a person would. See Common.ps1 for the
# deadlines.

BeforeDiscovery {
    $onWindows = $IsWindows -or $PSVersionTable.PSEdition -eq 'Desktop'
    $modes = if ($onWindows) { 'Helper', 'InProcess' } else { 'Helper' }
    $cases = foreach ($m in $modes) { foreach ($r in 'Egui', 'Html') { @{ Mode = $m; Renderer = $r } } }
}

BeforeAll {
    . (Join-Path $PSScriptRoot 'Common.ps1')
    $script:Watchdog = Start-Watchdog
}

AfterAll {
    Stop-Watchdog $script:Watchdog
}

Describe 'A window described in PowerShell, in <Mode> mode drawn with <Renderer>' -ForEach $cases {
    BeforeAll {
        Reset-GuiSession $Mode $Renderer
        $script:Renderer = $Renderer
    }

    It 'writes the result of the button pressed, with every named input''s state' {
        $job = Start-Background @"
Show-GuiWindow -Title 'PWRS-GUI test widgets result' -Renderer $script:Renderer {
    Text 'Deploy to' -Style Heading
    ListBox 'staging', 'production' -Name Target
    Row { Button 'OK' -Name Ok -Result Ok; Button 'Cancel' -Name Cancel -Result Cancel }
}
"@
        $w = Wait-GuiWindow 'PWRS-GUI test widgets result' $job
        Invoke-GuiTestEvent -Window $w.Id -Name Target -Event Select -Value 1
        Invoke-GuiTestEvent -Window $w.Id -Name Ok -Event Click
        $done = Complete-Background $job
        $done.Errors | Should -BeNullOrEmpty
        $done.Output.Count | Should -Be 1
        $done.Output[0].PSObject.TypeNames[0] | Should -Be 'PwrsGui.WindowResult'
        $done.Output[0].Result | Should -Be 'Ok'
        $done.Output[0].State.Target | Should -Be 'production'
    }

    It 'runs a handler with its event, and the handler''s update reaches the window' {
        $job = Start-Background @"
Show-GuiWindow -Title 'PWRS-GUI test widgets handler' -Renderer $script:Renderer {
    Text 'idle' -Name Status
    Button 'Count' -Name Count -OnClick { param(`$e) Update-GuiWidget Status @{ Text = "clicked `$(`$e.Name) `$(`$e.Kind)" } }
}
"@
        $w = Wait-GuiWindow 'PWRS-GUI test widgets handler' $job
        Invoke-GuiTestEvent -Window $w.Id -Name Count -Event Click
        Wait-Until { (Find-GuiNode (Get-GuiSnapshot -Window $w.Id).Tree 'Status').Properties.text -eq 'clicked Count Click' } 'the handler''s update reaching the window'
        Close-GuiWindow -Window $w.Id -Result Done
        $done = Complete-Background $job
        $done.Errors | Should -BeNullOrEmpty
        $done.Output[-1].Result | Should -Be 'Done'
    }

    It 'closes from inside a handler with the result Close-GuiWindow gives' {
        $job = Start-Background @"
Show-GuiWindow -Title 'PWRS-GUI test widgets close' -Renderer $script:Renderer { Button 'Finish' -Name Finish -OnClick { Close-GuiWindow -Result 42 } }
"@
        $w = Wait-GuiWindow 'PWRS-GUI test widgets close' $job
        Invoke-GuiTestEvent -Window $w.Id -Name Finish -Event Click
        $done = Complete-Background $job
        $done.Errors | Should -BeNullOrEmpty
        $done.Output[-1].Result | Should -Be 42
    }

    It 'runs -OnFileChange when a watched file changes, its Name the file and its Value the length' {
        $log = Join-Path $TestDrive 'waiting.log'
        Set-Content -LiteralPath $log -Value 'first'
        $before = (Get-Item -LiteralPath $log).Length
        $job = Start-Background @"
Show-GuiWindow -Title 'PWRS-GUI test widgets watch' -Renderer $script:Renderer -HandlerOutput Pipeline -WatchPath '$log' -OnFileChange { param(`$e) `$e; Close-GuiWindow -Result Changed } { Text 'Body' }
"@
        $null = Wait-GuiWindow 'PWRS-GUI test widgets watch' $job
        # The window shows no sign of when its watch began, so a line is
        # added until the handler has closed it.
        Wait-Until { Add-Content -LiteralPath $log -Value 'more'; $job.Async.IsCompleted } 'the window closing from -OnFileChange'
        $done = Complete-Background $job
        $done.Errors | Should -BeNullOrEmpty
        $done.Output[0].Kind | Should -Be 'FileChange'
        $done.Output[0].Name | Should -Be (Resolve-Path -LiteralPath $log).ProviderPath
        $done.Output[0].Value | Should -BeGreaterThan $before
        $done.Output[-1].Result | Should -Be 'Changed'
    }

    It 'gives each event its value and writes handler output only with -HandlerOutput Pipeline' {
        $job = Start-Background @"
Show-GuiWindow -Title 'PWRS-GUI test widgets values' -Renderer $script:Renderer -HandlerOutput Pipeline -OnClose { param(`$e) "closed `$(`$e.Kind) `$(`$e.Value) `$(`$e.State.One)" } {
    ListBox 'a', 'b', 'c' -Multiple -Name Many -OnSelect { param(`$e) 'many ' + (`$e.Value -join ',') }
    ListBox 'x', 'y' -Name One -OnActivate { param(`$e) "one `$(`$e.Value) `$(`$_.Name)" }
    Button 'OK' -Name Ok -Result Ok
}
"@
        $w = Wait-GuiWindow 'PWRS-GUI test widgets values' $job
        Invoke-GuiTestEvent -Window $w.Id -Name Many -Event Select -Value 0, 2
        Invoke-GuiTestEvent -Window $w.Id -Name One -Event Activate -Value 1
        Invoke-GuiTestEvent -Window $w.Id -Name Ok -Event Click
        $done = Complete-Background $job
        $done.Errors | Should -BeNullOrEmpty
        $done.Output[0] | Should -Be 'many a,c'
        $done.Output[1] | Should -Be 'one y One'
        $done.Output[2] | Should -Be 'closed Close Ok y'
        $done.Output[3].Result | Should -Be 'Ok'
        @($done.Output[3].State.Many) | Should -Be @('a', 'c')
    }

    It 'gives a table''s selection as the objects its rows were made from' {
        $job = Start-Background @"
`$rows = 1..3 | ForEach-Object { [pscustomobject]@{ Name = "n`$_"; Size = `$_ * 10 } }
Show-GuiWindow -Title 'PWRS-GUI test widgets table' -Renderer $script:Renderer -HandlerOutput Pipeline {
    Table `$rows -Selection Multiple -Name Files -OnActivate { param(`$e) 'activated ' + `$e.Value.Name }
    Button 'OK' -Name Ok -Result Ok
}
"@
        $w = Wait-GuiWindow 'PWRS-GUI test widgets table' $job
        Wait-Until { (Get-GuiSnapshot -Window $w.Id).State.Files.rows -eq 3 } 'the table''s three rows arriving'
        @((Find-GuiNode (Get-GuiSnapshot -Window $w.Id).Tree 'Files').Properties.columns.label) | Should -Be @('Name', 'Size')
        Invoke-GuiTestEvent -Window $w.Id -Name Files -Event Activate -Value 2
        Invoke-GuiTestEvent -Window $w.Id -Name Files -Event Select -Value 0, 2
        Invoke-GuiTestEvent -Window $w.Id -Name Ok -Event Click
        $done = Complete-Background $job
        $done.Errors | Should -BeNullOrEmpty
        $done.Output[0] | Should -Be 'activated n3'
        $picked = @($done.Output[-1].State.Files)
        @($picked.Name) | Should -Be @('n1', 'n3')
        $picked[1].Size | Should -Be 30
    }

    It 'adds a row to a table from a handler, taking the columns of a table that opened empty from its first row' {
        $job = Start-Background @"
Show-GuiWindow -Title 'PWRS-GUI test widgets table grows' -Renderer $script:Renderer {
    Table -Name Jobs -Selection Single
    Button 'Add' -Name Add -OnClick { Add-GuiItem Jobs ([pscustomobject]@{ Job = 'build'; Minutes = 3 }) }
    Button 'OK' -Name Ok -Result Ok
}
"@
        $w = Wait-GuiWindow 'PWRS-GUI test widgets table grows' $job
        Invoke-GuiTestEvent -Window $w.Id -Name Add -Event Click
        Wait-Until { (Get-GuiSnapshot -Window $w.Id).State.Jobs.rows -eq 1 } 'the added row arriving'
        @((Find-GuiNode (Get-GuiSnapshot -Window $w.Id).Tree 'Jobs').Properties.columns.label) | Should -Be @('Job', 'Minutes')
        Invoke-GuiTestEvent -Window $w.Id -Name Jobs -Event Select -Value 0
        Invoke-GuiTestEvent -Window $w.Id -Name Ok -Event Click
        $done = Complete-Background $job
        $done.Errors | Should -BeNullOrEmpty
        $done.Output[-1].State.Jobs.Job | Should -Be 'build'
    }

    It 'gives a tree''s selection as the objects its nodes were made from, and fills a node through -OnExpand' {
        $job = Start-Background @"
`$share = [pscustomobject]@{ Name = 'share'; Kids = 2 }
Show-GuiWindow -Title 'PWRS-GUI test widgets tree' -Renderer $script:Renderer -HandlerOutput Pipeline {
    TreeView -Name Food -Node @{ Text = 'Fruit'; Children = 'Apple', 'Pear' }, @{ Text = 'Veg'; Children = @('Leek') } -OnActivate { param(`$e) 'activated ' + `$e.Value }
    TreeView -Name Lazy -Node `$share -Text Name -HasChildren { `$_.Kids -gt 0 } -OnExpand { param(`$e) 1..`$e.Value.Kids | ForEach-Object { [pscustomobject]@{ Name = "kid`$_"; Kids = 0 } } } -OnSelect { param(`$e) 'picked ' + `$e.Value.Name }
    Button 'OK' -Name Ok -Result Ok
}
"@
        $w = Wait-GuiWindow 'PWRS-GUI test widgets tree' $job
        @((Find-GuiNode (Get-GuiSnapshot -Window $w.Id).Tree 'Food').Children.Properties.text) | Should -Be @('Fruit', 'Veg')
        Invoke-GuiTestEvent -Window $w.Id -Name Food -Event Expand -Value 0
        @((Get-GuiSnapshot -Window $w.Id).State.Food.shown).Count | Should -Be 4
        Invoke-GuiTestEvent -Window $w.Id -Name Food -Event Activate -Value 2
        Invoke-GuiTestEvent -Window $w.Id -Name Lazy -Event Expand -Value 0
        Wait-Until { @((Get-GuiSnapshot -Window $w.Id).State.Lazy.shown).Count -eq 3 } 'the node''s two children arriving from -OnExpand'
        @((Find-GuiNode (Get-GuiSnapshot -Window $w.Id).Tree 'Lazy').Children[0].Children.Properties.text) | Should -Be @('kid1', 'kid2')
        Invoke-GuiTestEvent -Window $w.Id -Name Lazy -Event Select -Value 2
        { Invoke-GuiTestEvent -Window $w.Id -Name Lazy -Event Expand -Value 1 -ErrorAction Stop } | Should -Throw '*leaf*'
        Invoke-GuiTestEvent -Window $w.Id -Name Ok -Event Click
        $done = Complete-Background $job
        $done.Errors | Should -BeNullOrEmpty
        $done.Output[0] | Should -Be 'activated Pear'
        $done.Output[1] | Should -Be 'picked kid2'
        $done.Output[-1].State.Food | Should -Be 'Pear'
        $done.Output[-1].State.Lazy.Name | Should -Be 'kid2'
    }

    It 'closes a node whose -OnExpand fails, so opening it again runs -OnExpand again' {
        $job = Start-Background @"
`$global:PwrsGuiTries = 0
Show-GuiWindow -Title 'PWRS-GUI test widgets tree retry' -Renderer $script:Renderer {
    TreeView -Name Lazy -Node ([pscustomobject]@{ Name = 'share' }) -Text Name -HasChildren { `$true } -OnExpand { `$global:PwrsGuiTries++; if (`$global:PwrsGuiTries -eq 1) { throw 'the share is offline' }; 'found' }
    Button 'OK' -Name Ok -Result Ok
}
"@
        $w = Wait-GuiWindow 'PWRS-GUI test widgets tree retry' $job
        Invoke-GuiTestEvent -Window $w.Id -Name Lazy -Event Expand -Value 0
        Wait-Until { @((Get-GuiSnapshot -Window $w.Id).State.Lazy.open).Count -eq 0 } 'the node closing again after -OnExpand failed'
        @((Find-GuiNode (Get-GuiSnapshot -Window $w.Id).Tree 'Lazy').Children[0].Children).Count | Should -Be 0
        Invoke-GuiTestEvent -Window $w.Id -Name Lazy -Event Expand -Value 0
        Wait-Until { @((Get-GuiSnapshot -Window $w.Id).State.Lazy.shown).Count -eq 2 } 'the second opening giving the node its child'
        (Find-GuiNode (Get-GuiSnapshot -Window $w.Id).Tree 'Lazy').Children[0].Children[0].Properties.text | Should -Be 'found'
        Invoke-GuiTestEvent -Window $w.Id -Name Ok -Event Click
        $done = Complete-Background $job
        @($done.Errors).Count | Should -Be 1
        "$($done.Errors[0])" | Should -BeLike '*the share is offline*'
        $done.Output[-1].Result | Should -Be 'Ok'
    }

    It 'writes what a handler throws and the errors it writes as the command''s own, which -ErrorVariable and -ErrorAction apply to' {
        $job = Start-Background @"
`$result = Show-GuiWindow -Title 'PWRS-GUI test widgets handler errors' -Renderer $script:Renderer -ErrorVariable failed -ErrorAction SilentlyContinue {
    Button 'Throw' -Name Throw -OnClick { throw 'the share is offline' }
    Button 'Write' -Name Write -OnClick { Write-Error 'a file was skipped' }
    Button 'OK' -Name Ok -Result Ok
}
`$result.Result
@(`$failed).ForEach({ `$_.Exception.Message })
"@
        $w = Wait-GuiWindow 'PWRS-GUI test widgets handler errors' $job
        Invoke-GuiTestEvent -Window $w.Id -Name Throw -Event Click
        Invoke-GuiTestEvent -Window $w.Id -Name Write -Event Click
        Invoke-GuiTestEvent -Window $w.Id -Name Ok -Event Click
        $done = Complete-Background $job
        $done.Errors | Should -BeNullOrEmpty -Because '-ErrorAction SilentlyContinue leaves a handler''s errors off the error stream, as it does the command''s own'
        $done.Output[0] | Should -Be 'Ok'
        $done.Output[1..2] | Should -Be 'the share is offline', 'a file was skipped'
    }

    It 'gives each input''s value in State and in its Change event, a password field''s as a SecureString' {
        $job = Start-Background @"
Show-GuiWindow -Title 'PWRS-GUI test widgets inputs' -Renderer $script:Renderer -HandlerOutput Pipeline {
    CheckBox 'Dry run' -Name DryRun -Checked -OnChange { param(`$e) "check `$(`$e.Value)" }
    TextBox 'web01' -Name Server -OnChange { param(`$e) "text `$(`$e.Value)" }
    TextBox -Name Secret -Password -OnChange { param(`$e) "secret `$(`$e.Value.GetType().Name)" }
    NumberBox 3 -Minimum 1 -Maximum 10 -Name Copies -OnChange { param(`$e) "number `$(`$e.Value) `$(`$e.Value.GetType().Name)" }
    Slider 20 -Name Volume -OnChange { param(`$e) "slider `$(`$e.Value)" }
    ComboBox 'staging', 'production' -Name Target -OnChange { param(`$e) "combo `$(`$e.Value)" }
    RadioGroup 'Small', 'Large' -Name Size -Selected Small -OnChange { param(`$e) "radio `$(`$e.Value)" }
    Button 'OK' -Name Ok -Result Ok
}
"@
        $w = Wait-GuiWindow 'PWRS-GUI test widgets inputs' $job
        $state = (Get-GuiSnapshot -Window $w.Id).State
        $state.DryRun | Should -BeTrue
        $state.Server | Should -Be 'web01'
        $state.Copies | Should -Be 3
        $state.Volume | Should -Be 20
        $state.Target | Should -BeNullOrEmpty
        $state.Size | Should -Be 0
        Invoke-GuiTestEvent -Window $w.Id -Name DryRun -Event Change -Value $false
        Invoke-GuiTestEvent -Window $w.Id -Name Server -Event Change -Value 'web02'
        Invoke-GuiTestEvent -Window $w.Id -Name Secret -Event Change -Value 'hunter2'
        Invoke-GuiTestEvent -Window $w.Id -Name Copies -Event Change -Value 7
        Invoke-GuiTestEvent -Window $w.Id -Name Volume -Event Change -Value 55
        Invoke-GuiTestEvent -Window $w.Id -Name Target -Event Change -Value 1
        Invoke-GuiTestEvent -Window $w.Id -Name Size -Event Change -Value 1
        Invoke-GuiTestEvent -Window $w.Id -Name Ok -Event Click
        $done = Complete-Background $job
        $done.Errors | Should -BeNullOrEmpty
        $done.Output[0..6] | Should -Be @('check False', 'text web02', 'secret SecureString', 'number 7 Double', 'slider 55', 'combo production', 'radio Large')
        $state = $done.Output[7].State
        $state.DryRun | Should -BeFalse
        $state.Server | Should -Be 'web02'
        $state.Secret | Should -BeOfType [securestring]
        [System.Net.NetworkCredential]::new('', $state.Secret).Password | Should -Be 'hunter2'
        $state.Copies | Should -BeOfType [double]
        $state.Copies | Should -Be 7
        $state.Volume | Should -Be 55
        $state.Target | Should -Be 'production'
        $state.Size | Should -Be 'Large'
    }

    It 'offers a password field''s peek button once it has the focus and is typed into, shows the password while it is held, and withholds it once the focus leaves until the field is cleared' {
        $title = 'PWRS-GUI test widgets peek'
        $w = Show-GuiWindow -NoWait -Title $title -Renderer $Renderer {
            TextBox -Name Secret -Password
            TextBox 'given' -Name Given -Password
            TextBox -Name Plain
        }
        $null = Wait-GuiWindow $title
        $peek = { param($name) (Find-GuiNode (Get-GuiSnapshot -Window $w.Id).Tree $name).Properties['peek'] }
        # The field's picture once two captures in a row agree.
        $still = {
            $seen = @{ Last = '' }
            Wait-Until {
                $shot = [Convert]::ToBase64String((Get-GuiSnapshot -Window $w.Id -Name Secret -Pixels).Pixels)
                $same = $shot -eq $seen.Last
                $seen.Last = $shot
                $same
            } 'the field''s picture staying still'
            $seen.Last
        }
        try {
            & $peek Secret | Should -BeFalse -Because 'an empty field offers no peek button'
            (Find-GuiNode (Get-GuiSnapshot -Window $w.Id).Tree Plain).Properties.Contains('peek') | Should -BeFalse -Because 'a field that is no password field has none'
            Invoke-GuiTestEvent -Window $w.Id -Name Plain -Event Peek -Value $true -ErrorVariable refused -ErrorAction SilentlyContinue
            "$refused" | Should -BeLike '*is no password field*'
            Invoke-GuiTestEvent -Window $w.Id -Name Secret -Event Peek -Value $true -ErrorVariable refused -ErrorAction SilentlyContinue
            "$refused" | Should -BeLike '*offers no peek button now*'
            Set-GuiFocus -Window $w.Id -Name Secret
            Invoke-GuiTestEvent -Window $w.Id -Name Secret -Event Change -Value 'hunter2'
            Wait-Until { & $peek Secret } 'the peek button offered once the focused field is typed into'
            $hidden = & $still
            Invoke-GuiTestEvent -Window $w.Id -Name Secret -Event Peek -Value $true
            & $still | Should -Not -Be $hidden -Because 'the password shows while the button is held'
            Invoke-GuiTestEvent -Window $w.Id -Name Secret -Event Peek -Value $false
            # A window with the system's focus blinks the field's caret, so this
            # waits for the picture taken before the button was held, which a
            # shown password never matches.
            Wait-Until { [Convert]::ToBase64String((Get-GuiSnapshot -Window $w.Id -Name Secret -Pixels).Pixels) -eq $hidden } 'the password hidden again once the button is let go'
            Set-GuiFocus -Window $w.Id -Name Plain
            Wait-Until { -not (& $peek Secret) } 'the peek button withdrawn once the focus leaves the field'
            Set-GuiFocus -Window $w.Id -Name Secret
            & $peek Secret | Should -BeFalse -Because 'the focus coming back does not offer it again'
            Invoke-GuiTestEvent -Window $w.Id -Name Secret -Event Change -Value 'hunter3'
            & $peek Secret | Should -BeFalse -Because 'typing on does not offer it again either'
            Invoke-GuiTestEvent -Window $w.Id -Name Secret -Event Change -Value ''
            Invoke-GuiTestEvent -Window $w.Id -Name Secret -Event Change -Value 'x'
            Wait-Until { & $peek Secret } 'the peek button offered again once the field is cleared and typed into'
            Set-GuiFocus -Window $w.Id -Name Given
            Invoke-GuiTestEvent -Window $w.Id -Name Given -Event Change -Value 'given2'
            & $peek Given | Should -BeFalse -Because 'text a script set withholds the button until the field is cleared'
        } finally {
            Close-GuiWindow -Window $w.Id
        }
    }

    It 'leaves what a person entered as it is when a handler sets other properties, and moves a number into a new range' {
        $job = Start-Background @"
Show-GuiWindow -Title 'PWRS-GUI test widgets update inputs' -Renderer $script:Renderer {
    TextBox -Name Note -Placeholder 'say something'
    Slider 80 -Name Level
    ComboBox 'a', 'b', 'c' -Name Pick
    Button 'Change' -Name Change -OnClick { Update-GuiWidget Note @{ Placeholder = 'changed' }; Update-GuiWidget Level @{ Maximum = 50 }; Update-GuiWidget Pick @{ Choice = 'c', 'd' } }
    Button 'OK' -Name Ok -Result Ok
}
"@
        $w = Wait-GuiWindow 'PWRS-GUI test widgets update inputs' $job
        Invoke-GuiTestEvent -Window $w.Id -Name Note -Event Change -Value 'typed'
        Invoke-GuiTestEvent -Window $w.Id -Name Pick -Event Change -Value 2
        Invoke-GuiTestEvent -Window $w.Id -Name Change -Event Click
        Wait-Until { (@((Find-GuiNode (Get-GuiSnapshot -Window $w.Id).Tree 'Pick').Properties.items) -join ',') -eq 'c,d' } 'the handler''s updates reaching the window'
        $state = (Get-GuiSnapshot -Window $w.Id).State
        $state.Note | Should -Be 'typed'
        $state.Level | Should -Be 50
        $state.Pick | Should -Be 0
        Invoke-GuiTestEvent -Window $w.Id -Name Level -Event Change -Value 60 -ErrorVariable refused -ErrorAction SilentlyContinue
        $refused.Count | Should -Be 1
        $refused[0].FullyQualifiedErrorId | Should -BeLike 'GuiRefused,*'
        "$($refused[0])" | Should -BeLike '*outside the range*'
        Invoke-GuiTestEvent -Window $w.Id -Name Level -Event Change -Value 30 -PassThru | Should -BeOfType [double]
        { Update-GuiWidget -Window $w.Id -Name Level -Property @{ Value = 70 } } | Should -Throw -ErrorId 'GuiBadNumber*'
        { Update-GuiWidget -Window $w.Id -Name Note -Property @{ Password = $true } } | Should -Throw -ErrorId 'GuiOpeningOnly*'
        Invoke-GuiTestEvent -Window $w.Id -Name Ok -Event Click
        $done = Complete-Background $job
        $done.Errors | Should -BeNullOrEmpty
        $done.Output[-1].State.Note | Should -Be 'typed'
        $done.Output[-1].State.Level | Should -Be 30
        $done.Output[-1].State.Pick | Should -Be 'c'
    }

    It 'draws the layout widgets, and shows the tab chosen, whose title State gives' {
        $job = Start-Background @"
Show-GuiWindow -Title 'PWRS-GUI test widgets layout' -Renderer $script:Renderer -HandlerOutput Pipeline {
    Grid -Columns 2 -Name Form { Text 'Server'; TextBox 'web01' -Name Server; Text 'Copies'; NumberBox 2 -Name Copies }
    Group -Title 'Options' -Name Options { CheckBox 'Verbose' -Name Verbose }
    Tabs -Name Pages -OnChange { param(`$e) "tab `$(`$e.Value)" } { Tab 'General' { Text 'first page' -Name First }; Tab 'More' { TextBox 'later' -Name Later } }
    Split -Name Panes -Position 0.25 { Text 'left' -Name Left; Text 'right' -Name Right }
    Scroll -Height 60 -Name Lines { 1..20 | ForEach-Object { Text "line `$_" } }
    Button 'OK' -Name Ok -Result Ok
}
"@
        $w = Wait-GuiWindow 'PWRS-GUI test widgets layout' $job
        foreach ($name in 'Form', 'Server', 'Options', 'Pages', 'First', 'Panes', 'Left', 'Right', 'Lines') {
            (Get-GuiSnapshot -Window $w.Id -Name $name -Pixels).Width | Should -BeGreaterThan 0 -Because "$name is drawn"
        }
        (Get-GuiSnapshot -Window $w.Id).State.Pages | Should -Be 0
        Get-GuiSnapshot -Window $w.Id -Name Later -Pixels -ErrorVariable hidden -ErrorAction SilentlyContinue
        $hidden.Count | Should -Be 1
        "$($hidden[0])" | Should -BeLike '*not drawn*'
        Invoke-GuiTestEvent -Window $w.Id -Name Pages -Event Change -Value 1
        Wait-Until { (Get-GuiSnapshot -Window $w.Id -Name Later -Pixels -ErrorAction Ignore).Width -gt 0 } 'the second tab being drawn'
        { Update-GuiWidget -Window $w.Id -Name Lines -Property @{ Height = -1 } } | Should -Throw -ErrorId 'GuiBadSize*'
        Invoke-GuiTestEvent -Window $w.Id -Name Ok -Event Click
        $done = Complete-Background $job
        $done.Errors | Should -BeNullOrEmpty
        $done.Output[0] | Should -Be 'tab More'
        $done.Output[1].State.Pages | Should -Be 'More'
        $done.Output[1].State.Later | Should -Be 'later'
    }

    It 'gives a widget the keyboard focus, from a handler too, and refuses one that cannot take it' {
        $job = Start-Background @"
Show-GuiWindow -Title 'PWRS-GUI test widgets focus' -Renderer $script:Renderer {
    Text 'idle' -Name Status
    TextBox -Name Filter
    Button 'Off' -Name Off -Enabled:`$false
    Tabs -Name Pages { Tab 'One' { Button 'Shown' -Name Shown }; Tab 'Two' { Button 'Hidden' -Name Hidden } }
    Button 'Find' -Name Find -OnClick { Set-GuiFocus Filter }
    Button 'OK' -Name Ok -Result Ok
}
"@
        $w = Wait-GuiWindow 'PWRS-GUI test widgets focus' $job
        Set-GuiFocus -Window $w.Id -Name Shown
        (Get-GuiSnapshot -Window $w.Id).Focused | Should -Be 'Shown'
        Invoke-GuiTestEvent -Window $w.Id -Name Find -Event Click
        Wait-Until { (Get-GuiSnapshot -Window $w.Id).Focused -eq 'Filter' } 'the handler''s Set-GuiFocus reaching the window'
        { Set-GuiFocus -Window $w.Id -Name Status } | Should -Throw -ErrorId 'GuiNotFocusable*'
        Set-GuiFocus -Window $w.Id -Name Off -ErrorVariable off -ErrorAction SilentlyContinue
        $off[0].FullyQualifiedErrorId | Should -BeLike 'GuiRefused,*'
        $off[0].Exception.Message | Should -BeLike '*disabled*'
        Set-GuiFocus -Window $w.Id -Name Hidden -ErrorVariable hidden -ErrorAction SilentlyContinue
        $hidden[0].FullyQualifiedErrorId | Should -BeLike 'GuiRefused,*'
        $hidden[0].Exception.Message | Should -BeLike '*not shown*'
        (Get-GuiSnapshot -Window $w.Id).Focused | Should -Be 'Filter' -Because 'a refused focus leaves it where it was'
        Invoke-GuiTestEvent -Window $w.Id -Name Ok -Event Click
        $done = Complete-Background $job
        $done.Errors | Should -BeNullOrEmpty
    }

    It 'gives a spacer its size along the row or column it is in' {
        $job = Start-Background @"
Show-GuiWindow -Title 'PWRS-GUI test widgets spacer' -Renderer $script:Renderer {
    Row {
        Column -Name Tall { Text 'a'; Spacer 120; Text 'b' }
        Column -Name Short { Text 'a'; Text 'b' }
        Row -Name Wide { Text 'a'; Spacer 120; Text 'b' }
        Row -Name Narrow { Text 'a'; Text 'b' }
    }
    Button 'OK' -Name Ok -Result Ok
}
"@
        $w = Wait-GuiWindow 'PWRS-GUI test widgets spacer' $job
        $shot = @{}
        foreach ($name in 'Tall', 'Short', 'Wide', 'Narrow') {
            $shot[$name] = Get-GuiSnapshot -Window $w.Id -Name $name -Pixels
        }
        ([int]$shot.Tall.Height - [int]$shot.Short.Height) | Should -BeGreaterThan 60 -Because 'a spacer in a column is as high as its size'
        ([int]$shot.Wide.Width - [int]$shot.Narrow.Width) | Should -BeGreaterThan 60 -Because 'a spacer in a row is as wide as its size'
        ([int]$shot.Wide.Height - [int]$shot.Narrow.Height) | Should -BeLessThan 30 -Because 'a spacer in a row adds no height'
        Invoke-GuiTestEvent -Window $w.Id -Name Ok -Event Click
        $done = Complete-Background $job
        $done.Errors | Should -BeNullOrEmpty
    }

    It 'draws the display widgets, runs a link''s handler, and adds a log''s lines' {
        $job = Start-Background @"
Show-GuiWindow -Title 'PWRS-GUI test widgets display' -Renderer $script:Renderer -HandlerOutput Pipeline {
    Text 'Read the docs' -Style Link -Name Docs -OnClick { param(`$e) "link `$(`$e.Name) `$(`$e.Kind)"; Update-GuiWidget Copy @{ Value = 60; Text = 'more' }; Add-GuiItem Log 'three' }
    ProgressBar 30 -Name Copy -Text 'Copying'
    Spinner -Name Busy
    LogView 'one', 'two' -Name Log -Height 80
    Button 'OK' -Name Ok -Result Ok
}
"@
        $w = Wait-GuiWindow 'PWRS-GUI test widgets display' $job
        foreach ($name in 'Docs', 'Copy', 'Busy', 'Log') {
            $shot = Get-GuiSnapshot -Window $w.Id -Name $name -Pixels -ErrorVariable refused -ErrorAction SilentlyContinue
            "$refused" | Should -BeNullOrEmpty -Because "$name is drawn"
            $shot.Width | Should -BeGreaterThan 0 -Because "$name is drawn"
        }
        Invoke-GuiTestEvent -Window $w.Id -Name Docs -Event Click
        Wait-Until { @((Find-GuiNode (Get-GuiSnapshot -Window $w.Id).Tree 'Log').Properties.lines).Count -eq 3 } 'the handler''s new line reaching the log'
        $tree = (Get-GuiSnapshot -Window $w.Id).Tree
        (Find-GuiNode $tree 'Copy').Properties.value | Should -Be 60
        (Find-GuiNode $tree 'Copy').Properties.text | Should -Be 'more'
        @((Find-GuiNode $tree 'Log').Properties.lines) | Should -Be @('one', 'two', 'three')
        { Update-GuiWidget -Window $w.Id -Name Log -Property @{ Height = -1 } } | Should -Throw -ErrorId 'GuiBadSize*'
        Invoke-GuiTestEvent -Window $w.Id -Name Ok -Event Click
        $done = Complete-Background $job
        $done.Errors | Should -BeNullOrEmpty
        $done.Output[0] | Should -Be 'link Docs Click'
    }

    It 'shows a picture at its own size, one side given with the other in proportion, and a box its fit fills' {
        $red = 'iVBORw0KGgoAAAANSUhEUgAAAAgAAAAECAIAAAA8r+mnAAAAEklEQVR4nGP4z8CAFWEXJUsCAFpeH+EeQoQoAAAAAElFTkSuQmCC'
        $file = Join-Path ([IO.Path]::GetTempPath()) "pwrs-gui-test-picture-$PID-$script:Renderer.png"
        [IO.File]::WriteAllBytes($file, [Convert]::FromBase64String($red))
        function Middle($shot) {
            if (-not $shot) { return @() }
            $at = ([int][math]::Floor($shot.Height / 2) * $shot.Width + [int][math]::Floor($shot.Width / 2)) * 4
            @($shot.Pixels[$at..($at + 2)])
        }
        function Near($rgb, $want) { $rgb.Count -eq 3 -and @(0..2 | Where-Object { [math]::Abs([int]$rgb[$_] - $want[$_]) -gt 8 }).Count -eq 0 }
        $job = Start-Background @"
Show-GuiWindow -Title 'PWRS-GUI test widgets images' -Renderer $script:Renderer {
    Image '$file' -Name Own
    Image -Bytes ([Convert]::FromBase64String('$red')) -Width 40 -Name Wide
    Image -Bytes ([Convert]::FromBase64String('$red')) -Width 40 -Height 40 -Fit Fill -Name Filled
    Image -Bytes ([Text.Encoding]::UTF8.GetBytes('<svg xmlns="http://www.w3.org/2000/svg" width="20" height="10"><rect width="20" height="10" fill="#0000ff"/></svg>')) -Height 30 -AltText 'a blue bar' -Name Vector
    Button 'OK' -Name Ok -Result Ok
}
"@
        $w = Wait-GuiWindow 'PWRS-GUI test widgets images' $job
        $wanted = @{ Own = @(255, 0, 0); Wide = @(255, 0, 0); Filled = @(255, 0, 0); Vector = @(0, 0, 255) }
        $shots = @{}
        foreach ($name in 'Own', 'Wide', 'Filled', 'Vector') {
            Wait-Until { Near (Middle (Get-GuiSnapshot -Window $w.Id -Name $name -Pixels -ErrorAction Ignore)) $wanted[$name] } "$name's picture being drawn"
            $shots[$name] = Get-GuiSnapshot -Window $w.Id -Name $name -Pixels
        }
        $ratio = { param($shot) $shot.Width / $shot.Height }
        [math]::Abs((& $ratio $shots.Own) - 2) | Should -BeLessThan 0.25 -Because 'a picture at its own size is not stretched'
        [math]::Abs((& $ratio $shots.Wide) - 2) | Should -BeLessThan 0.25 -Because 'a width given, the height follows'
        [math]::Abs($shots.Wide.Width / $shots.Own.Width - 5) | Should -BeLessThan 0.5 -Because 'a width of 40 points is five times the picture''s own 8'
        [math]::Abs((& $ratio $shots.Filled) - 1) | Should -BeLessThan 0.25 -Because 'Fill stretches the picture to its box'
        [math]::Abs((& $ratio $shots.Vector) - 2) | Should -BeLessThan 0.25 -Because 'an SVG is not stretched either'
        Invoke-GuiTestEvent -Window $w.Id -Name Ok -Event Click
        $done = Complete-Background $job
        Remove-Item -LiteralPath $file
        $done.Errors | Should -BeNullOrEmpty
    }

    It 'shows a document''s picture from beside it, raises Click with a link''s address, scrolls to a heading''s instead, and reports a picture it cannot fetch' {
        $base = Join-Path ([IO.Path]::GetTempPath()) "pwrs-gui-test-markdown-$PID-$script:Renderer"
        [IO.File]::WriteAllText("$base.svg", '<svg xmlns="http://www.w3.org/2000/svg" width="40" height="40"><rect width="40" height="40" fill="#ff0000"/></svg>')
        $box = Split-Path -Leaf "$base.svg"
        [IO.File]::WriteAllText("$base.md", "![a red box]($box)`n`n# Notes {#notes}`n`nSee [the site](https://example.com/) or [the notes](#notes), and ![a picture from nowhere](https://127.0.0.1:9/gone.png).`n`n``````rust`nlet x = 1;`n```````n`n> [!NOTE]`n> Read this.`n")
        function Corner($shot) {
            if (-not $shot) { return @() }
            $at = (2 * $shot.Width + 2) * 4
            @($shot.Pixels[$at..($at + 2)])
        }
        function Near($rgb, $want) { $rgb.Count -eq 3 -and @(0..2 | Where-Object { [math]::Abs([int]$rgb[$_] - $want[$_]) -gt 8 }).Count -eq 0 }
        $job = Start-Background @"
Show-GuiWindow -Title 'PWRS-GUI test widgets markdown' -Renderer $script:Renderer -HandlerOutput Pipeline {
    Markdown -Path '$base.md' -Name Doc -OnClick { param(`$e) "link `$(`$e.Value)" }
    Button 'OK' -Name Ok -Result Ok
}
"@
        $w = Wait-GuiWindow 'PWRS-GUI test widgets markdown' $job
        Wait-Until { Near (Corner (Get-GuiSnapshot -Window $w.Id -Name Doc -Pixels -ErrorAction Ignore)) @(255, 0, 0) } 'the document''s picture being drawn at its top left'
        Wait-Until { @($job.PS.Streams.Warning | Where-Object { $_.Message -like '*https://127.0.0.1:9/gone.png shows its alt text*' }).Count -ge 1 } 'the picture it cannot fetch being reported'
        Invoke-GuiTestEvent -Window $w.Id -Name Doc -Event Click -Value 'https://example.com/'
        Invoke-GuiTestEvent -Window $w.Id -Name Doc -Event Click -Value '#notes'
        { Invoke-GuiTestEvent -Window $w.Id -Name Doc -Event Click -Value 'https://elsewhere.example/' -ErrorAction Stop } | Should -Throw '*no link to https://elsewhere.example/; its links are https://example.com/, #notes*'
        Invoke-GuiTestEvent -Window $w.Id -Name Ok -Event Click
        $done = Complete-Background $job
        Remove-Item -LiteralPath "$base.md", "$base.svg"
        $done.Errors | Should -BeNullOrEmpty
        @($done.Output | Where-Object { $_ -is [string] }) | Should -Be @('link https://example.com/') -Because 'a link to a heading scrolls there and raises nothing'
        @($done.Warnings | Where-Object { $_ -like '*https://127.0.0.1:9/gone.png shows its alt text*' }).Count | Should -Be 1 -Because 'a picture is reported once'
    }

    It 'raises Submit on Enter in a field of one line, and gives a color and an editable drop-down''s text' {
        $job = Start-Background @"
Show-GuiWindow -Title 'PWRS-GUI test widgets submit' -Renderer $script:Renderer -HandlerOutput Pipeline {
    TextBox -Name Query -OnSubmit { param(`$e) "submit `$(`$e.Value)" }
    TextBox -Name Key -Password -OnSubmit { param(`$e) "key `$(`$e.Value.GetType().Name)" }
    TextBox -Name Notes -Multiline
    ColorPicker '#336699' -Name Accent -OnChange { param(`$e) "color `$(`$e.Value)" }
    ComboBox 'web01', 'web02' -Editable -Selected 'web03' -Name Server -OnChange { param(`$e) "server `$(`$e.Value)" }
    Button 'OK' -Name Ok -Result Ok
}
"@
        $w = Wait-GuiWindow 'PWRS-GUI test widgets submit' $job
        $state = (Get-GuiSnapshot -Window $w.Id).State
        $state.Accent | Should -Be '#336699'
        $state.Server | Should -Be 'web03'
        Invoke-GuiTestEvent -Window $w.Id -Name Query -Event Submit -Value 'find this'
        Invoke-GuiTestEvent -Window $w.Id -Name Key -Event Submit -Value 'k'
        Invoke-GuiTestEvent -Window $w.Id -Name Accent -Event Change -Value '#FF0000'
        Invoke-GuiTestEvent -Window $w.Id -Name Server -Event Change -Value 'web02'
        Invoke-GuiTestEvent -Window $w.Id -Name Notes -Event Submit -Value 'x' -ErrorVariable refused -ErrorAction SilentlyContinue
        $refused.Count | Should -Be 1
        foreach ($name in 'Accent', 'Server') {
            (Get-GuiSnapshot -Window $w.Id -Name $name -Pixels).Width | Should -BeGreaterThan 0 -Because "$name is drawn"
        }
        { Update-GuiWidget -Window $w.Id -Name Server -Property @{ Selected = 'web01' } } | Should -Throw -ErrorId 'GuiBadProperty*'
        { Update-GuiWidget -Window $w.Id -Name Server -Property @{ Editable = $false } } | Should -Throw -ErrorId 'GuiOpeningOnly*'
        Invoke-GuiTestEvent -Window $w.Id -Name Ok -Event Click
        $done = Complete-Background $job
        $done.Errors | Should -BeNullOrEmpty
        $done.Output[0..3] | Should -Be @('submit find this', 'key SecureString', 'color #ff0000', 'server web02')
        $state = $done.Output[4].State
        $state.Query | Should -Be 'find this'
        $state.Accent | Should -Be '#ff0000'
        $state.Server | Should -Be 'web02'
    }

    It 'gives a date chooser''s minute as a [datetime] and a file chooser''s paths, in State and in their Change events' {
        $job = Start-Background @"
Show-GuiWindow -Title 'PWRS-GUI test widgets choosers' -Renderer $script:Renderer -HandlerOutput Pipeline {
    DatePicker ([datetime]'2026-10-08T14:30') -Minimum ([datetime]'2026-10-01T00:00') -Maximum ([datetime]'2026-10-31T23:59') -Name When -OnChange { param(`$e) "when `$(`$e.Value.GetType().Name) `$(`$e.Value.ToString('yyyy-MM-ddTHH:mm'))" }
    DatePicker -Name Unset
    FilePicker -Name Log -Filter 'Logs|*.log', 'All files|*' -OnChange { param(`$e) "log `$(`$e.Value)" }
    FilePicker -Name Many -Multiple -OnChange { param(`$e) "many `$(@(`$e.Value).Count)" }
    Button 'OK' -Name Ok -Result Ok
}
"@
        $w = Wait-GuiWindow 'PWRS-GUI test widgets choosers' $job
        $state = (Get-GuiSnapshot -Window $w.Id).State
        $state.When | Should -Be '2026-10-08T14:30'
        $state.Unset | Should -BeNullOrEmpty
        @($state.Log).Count | Should -Be 0
        Invoke-GuiTestEvent -Window $w.Id -Name When -Event Change -Value ([datetime]'2026-10-09T08:05:30')
        Invoke-GuiTestEvent -Window $w.Id -Name When -Event Change -Value ([datetime]'2026-11-01T00:00') -ErrorVariable outside -ErrorAction SilentlyContinue
        $outside.Count | Should -Be 1
        $path = Join-Path ([IO.Path]::GetTempPath()) 'pwrs-gui-test-chosen.log'
        Invoke-GuiTestEvent -Window $w.Id -Name Log -Event Change -Value $path
        Invoke-GuiTestEvent -Window $w.Id -Name Log -Event Change -Value @('a', 'b') -ErrorVariable one -ErrorAction SilentlyContinue
        $one.Count | Should -Be 1
        Invoke-GuiTestEvent -Window $w.Id -Name Many -Event Change -Value @('a', 'b')
        foreach ($name in 'When', 'Unset', 'Log') {
            (Get-GuiSnapshot -Window $w.Id -Name $name -Pixels).Width | Should -BeGreaterThan 0 -Because "$name is drawn"
        }
        Invoke-GuiTestEvent -Window $w.Id -Name Ok -Event Click
        $done = Complete-Background $job
        $done.Errors | Should -BeNullOrEmpty
        $done.Output[0..2] | Should -Be @('when DateTime 2026-10-09T08:05', "log $path", 'many 2')
        $result = $done.Output[3].State
        $result.When | Should -BeOfType [datetime]
        $result.When | Should -Be ([datetime]'2026-10-09T08:05')
        $result.Unset | Should -BeNullOrEmpty
        $result.Log | Should -Be $path
        @($result.Many) | Should -Be @('a', 'b')
    }

    It 'draws a chart of numbers and one of dates, and raises Select with a point''s Series, X and Y' {
        $job = Start-Background @"
Show-GuiWindow -Title 'PWRS-GUI test widgets chart' -Renderer $script:Renderer -HandlerOutput Pipeline {
    Chart -Name Load -Height 160 -XLabel 'sample' -YLabel 'percent' -Series @{ Name = 'cpu'; Y = 3, 5, 4 }, @{ Name = 'disk'; X = 0, 2; Y = 1, 2; Type = 'Bar' }, @{ Name = 'peak'; X = 1; Y = 9; Type = 'Points' } -OnSelect { param(`$e) "load `$(`$e.Value.Series) `$(`$e.Value.X) `$(`$e.Value.Y)" }
    Chart -Name Times -Height 160 -Type Points -Series @{ Name = 'ping'; X = [datetime]'2026-10-08T14:30', [datetime]'2026-10-08T14:31:30'; Y = 20, 35 }, @{ Name = 'once'; X = [datetime]'2026-10-08T14:32'; Y = 50 } -OnSelect { param(`$e) "times `$(`$e.Value.X.GetType().Name) `$(`$e.Value.X.ToString('HH:mm:ss')) `$(`$e.Value.y)" }
    Button 'OK' -Name Ok -Result Ok
}
"@
        $w = Wait-GuiWindow 'PWRS-GUI test widgets chart' $job
        Invoke-GuiTestEvent -Window $w.Id -Name Load -Event Select -Value @{ Series = 'cpu'; X = 1 }
        Invoke-GuiTestEvent -Window $w.Id -Name Load -Event Select -Value @{ Series = 'disk'; X = 2; Y = 2 }
        Invoke-GuiTestEvent -Window $w.Id -Name Times -Event Select -Value @{ Series = 'ping'; X = [datetime]'2026-10-08T14:31:30' }
        Invoke-GuiTestEvent -Window $w.Id -Name Load -Event Select -Value @{ Series = 'peak'; X = 1 }
        Invoke-GuiTestEvent -Window $w.Id -Name Times -Event Select -Value @{ Series = 'once'; X = [datetime]'2026-10-08T14:32' }
        Invoke-GuiTestEvent -Window $w.Id -Name Load -Event Select -Value @{ Series = 'cpu'; X = 7 } -ErrorVariable missing -ErrorAction SilentlyContinue
        $missing.Count | Should -Be 1
        foreach ($name in 'Load', 'Times') {
            (Get-GuiSnapshot -Window $w.Id -Name $name -Pixels).Width | Should -BeGreaterThan 0 -Because "$name is drawn"
        }
        Invoke-GuiTestEvent -Window $w.Id -Name Ok -Event Click
        $done = Complete-Background $job
        $done.Errors | Should -BeNullOrEmpty
        $done.Output[0..4] | Should -Be @('load cpu 1 5', 'load disk 2 2', 'times DateTime 14:31:30 35', 'load peak 1 9', 'times DateTime 14:32:00 50') -Because 'a series given one X and one Y is one point'
    }

    It 'raises Drop with what is dropped on a widget or the window, and Key for a key the focused widget does not use itself' {
        $job = Start-Background @"
Show-GuiWindow -Title 'PWRS-GUI test widgets drop' -Renderer $script:Renderer -HandlerOutput Pipeline -OnDrop { param(`$e) "window `$(@(`$e.Value).Count)" } {
    ListBox 'a', 'b' -Name Pick -OnDrop { param(`$e) "pick `$(`$e.Value -join ',')" } -OnKey { param(`$e) "key `$(`$e.Value)" }
    TextBox -Name Notes -OnKey { param(`$e) "notes `$(`$e.Value)" }
    Button 'OK' -Name Ok -Result Ok
}
"@
        $w = Wait-GuiWindow 'PWRS-GUI test widgets drop' $job
        Invoke-GuiTestEvent -Window $w.Id -Name Pick -Event Drop -Value @('C:\a.txt', 'C:\b.txt')
        Invoke-GuiTestEvent -Window $w.Id -Drop @('C:\c.txt')
        $onWindows = $IsWindows -or $PSVersionTable.PSEdition -eq 'Desktop'
        $takesText = $script:Renderer -eq 'Html' -and -not $onWindows
        Invoke-GuiTestEvent -Window $w.Id -Name Pick -Event Drop -Value 'dragged words' -ErrorVariable text -ErrorAction SilentlyContinue
        $text.Count | Should -Be ([int](-not $takesText)) -Because 'only a page outside Windows takes dropped text'
        Invoke-GuiTestEvent -Window $w.Id -Name Pick -Event Key -Value 'ctrl+k'
        Invoke-GuiTestEvent -Window $w.Id -Name Pick -Event Key -Value 'ArrowDown' -ErrorVariable kept -ErrorAction SilentlyContinue
        $kept.Count | Should -Be 1 -Because 'a list uses the arrows itself'
        Invoke-GuiTestEvent -Window $w.Id -Name Notes -Event Key -Value 'F5'
        Invoke-GuiTestEvent -Window $w.Id -Name Notes -Event Key -Value 'A' -ErrorVariable typed -ErrorAction SilentlyContinue
        $typed.Count | Should -Be 1 -Because 'a text field keeps what it types'
        Invoke-GuiTestEvent -Window $w.Id -Name Ok -Event Drop -Value @('C:\d.txt') -ErrorVariable none -ErrorAction SilentlyContinue
        $none.Count | Should -Be 1 -Because 'a button without -OnDrop takes no drop'
        Invoke-GuiTestEvent -Window $w.Id -Name Ok -Event Click
        $done = Complete-Background $job
        $done.Errors | Should -BeNullOrEmpty
        $expected = @('pick C:\a.txt,C:\b.txt', 'window 1') + @(if ($takesText) { 'pick dragged words' }) + @('key Ctrl+K', 'notes F5')
        $done.Output[0..($expected.Count - 1)] | Should -Be $expected
    }

    It 'shows a screen reader its widgets by name and role through UI Automation' -Skip:(-not ($IsWindows -or $PSVersionTable.PSEdition -eq 'Desktop')) {
        Add-Type -AssemblyName UIAutomationClient, UIAutomationTypes
        $job = Start-Background @"
Show-GuiWindow -Title 'PWRS-GUI test widgets access' -Renderer $script:Renderer {
    Button 'Deploy now' -Name Go -Result Ok
    CheckBox 'Dry run' -Name Dry
}
"@
        $w = Wait-GuiWindow 'PWRS-GUI test widgets access' $job
        $named = { param($value) New-Object System.Windows.Automation.PropertyCondition ([System.Windows.Automation.AutomationElement]::NameProperty), $value }
        $found = @{}
        $deadline = [DateTime]::UtcNow.AddSeconds($script:WindowSeconds)
        while ($found.Count -lt 2 -and [DateTime]::UtcNow -lt $deadline) {
            $window = [System.Windows.Automation.AutomationElement]::RootElement.FindFirst([System.Windows.Automation.TreeScope]::Children, (& $named 'PWRS-GUI test widgets access'))
            foreach ($name in 'Deploy now', 'Dry run') {
                if ($window -and -not $found[$name]) {
                    $element = $window.FindFirst([System.Windows.Automation.TreeScope]::Descendants, (& $named $name))
                    if ($element) { $found[$name] = $element.Current.ControlType.ProgrammaticName }
                }
            }
            if ($found.Count -lt 2) { Start-Sleep -Milliseconds 200 }
        }
        $found['Deploy now'] | Should -Be 'ControlType.Button'
        $found['Dry run'] | Should -Be 'ControlType.CheckBox'
        Invoke-GuiTestEvent -Window $w.Id -Name Go -Event Click
        $done = Complete-Background $job
        $done.Errors | Should -BeNullOrEmpty
        $done.Output[-1].Result | Should -Be 'Ok'
    }

    It 'draws a menu bar and a status bar, clicks a menu item by name and by its shortcut, and runs the window''s own shortcuts' {
        $job = Start-Background @"
Show-GuiWindow -Title 'PWRS-GUI test widgets menus' -Renderer $script:Renderer -HandlerOutput Pipeline -OnShortcut @{ 'shift+ctrl+s' = { param(`$e) "own `$(`$e.Kind) `$(`$e.Value)" }; 'F5' = { param(`$e) "refresh `$(`$e.Value)" } } {
    Text 'Body' -Name Body
    MenuBar -Name Bar {
        Menu 'File' -Name FileMenu {
            MenuItem 'Save' -Name Save -Shortcut 'ctrl+s' -OnClick { param(`$e) "save `$(`$e.Name) `$(`$e.Kind)" }
            MenuItem 'Locked' -Name Locked -Shortcut 'Ctrl+L' -Enabled:`$false -OnClick { 'locked' }
            Separator
            Menu 'Recent' { MenuItem 'One' -Name One -OnClick { 'one' } }
            MenuItem 'Exit' -Name Exit -Result Exit
        }
    }
    StatusBar -Name Status { Text 'Ready' -Name Ready }
}
"@
        $w = Wait-GuiWindow 'PWRS-GUI test widgets menus' $job
        foreach ($name in 'Body', 'Bar', 'FileMenu', 'Status', 'Ready') {
            (Get-GuiSnapshot -Window $w.Id -Name $name -Pixels).Width | Should -BeGreaterThan 0 -Because "$name is drawn"
        }
        Invoke-GuiTestEvent -Window $w.Id -Name Save -Event Click
        Invoke-GuiTestEvent -Window $w.Id -Shortcut 'Ctrl+S'
        Invoke-GuiTestEvent -Window $w.Id -Name One -Event Click
        Invoke-GuiTestEvent -Window $w.Id -Shortcut 'Ctrl+Shift+S'
        Invoke-GuiTestEvent -Window $w.Id -Shortcut 'f5'
        Invoke-GuiTestEvent -Window $w.Id -Name Locked -Event Click -ErrorVariable locked -ErrorAction SilentlyContinue
        "$locked" | Should -BeLike '*disabled*'
        Invoke-GuiTestEvent -Window $w.Id -Shortcut 'Ctrl+L' -ErrorVariable unbound -ErrorAction SilentlyContinue
        "$unbound" | Should -BeLike '*Ctrl+L*'
        { Invoke-GuiTestEvent -Window $w.Id -Shortcut 'Ctrl+Meta+X' } | Should -Throw -ErrorId 'GuiBadShortcut*'
        Invoke-GuiTestEvent -Window $w.Id -Name Exit -Event Click
        $done = Complete-Background $job
        $done.Errors | Should -BeNullOrEmpty
        $done.Output[0..4] | Should -Be @('save Save Click', 'save Save Click', 'one', 'own Shortcut Ctrl+Shift+S', 'refresh F5')
        $done.Output[5].Result | Should -Be 'Exit'
    }

    It 'acts on a disabled widget no more than a person can, until it is enabled' {
        $job = Start-Background @"
Show-GuiWindow -Title 'PWRS-GUI test widgets disabled' -Renderer $script:Renderer -HandlerOutput Pipeline {
    Button 'Deploy' -Name Deploy -Enabled:`$false -OnClick { 'deployed' }
    CheckBox 'Dry run' -Name DryRun -Enabled:`$false -OnChange { 'changed' }
    TextBox 'web01' -Name Server -Enabled:`$false -OnSubmit { 'submitted' }
    Button 'OK' -Name Ok -Result Ok
}
"@
        $w = Wait-GuiWindow 'PWRS-GUI test widgets disabled' $job
        foreach ($try in @(@('Deploy', 'Click', $null), @('DryRun', 'Change', $true), @('Server', 'Submit', 'web02'))) {
            Invoke-GuiTestEvent -Window $w.Id -Name $try[0] -Event $try[1] -Value $try[2] -ErrorVariable refused -ErrorAction SilentlyContinue
            "$refused" | Should -BeLike '*is disabled*' -Because "$($try[0]) is disabled"
        }
        Update-GuiWidget -Window $w.Id -Name Deploy -Property @{ Enabled = $true }
        Invoke-GuiTestEvent -Window $w.Id -Name Deploy -Event Click
        Invoke-GuiTestEvent -Window $w.Id -Name Ok -Event Click
        $done = Complete-Background $job
        $done.Errors | Should -BeNullOrEmpty
        $done.Output[0] | Should -Be 'deployed'
        $done.Output[1].State.DryRun | Should -BeFalse
        $done.Output[1].State.Server | Should -Be 'web01'
    }

    It 'clicks a context menu''s item, which is drawn only while it is open' {
        $job = Start-Background @"
Show-GuiWindow -Title 'PWRS-GUI test widgets context' -Renderer $script:Renderer -HandlerOutput Pipeline {
    ListBox 'web01', 'web02' -Name Servers -Selected 'web02'
    ContextMenu -For Servers -Name ServerMenu { MenuItem 'Copy' -Name CopyServer -OnClick { param(`$e) "copy `$(`$e.State.Servers)" } }
    Button 'OK' -Name Ok -Result Ok
}
"@
        $w = Wait-GuiWindow 'PWRS-GUI test widgets context' $job
        Get-GuiSnapshot -Window $w.Id -Name ServerMenu -Pixels -ErrorVariable closed -ErrorAction SilentlyContinue
        "$closed" | Should -BeLike '*not drawn*'
        Invoke-GuiTestEvent -Window $w.Id -Name CopyServer -Event Click
        Invoke-GuiTestEvent -Window $w.Id -Name Ok -Event Click
        $done = Complete-Background $job
        $done.Errors | Should -BeNullOrEmpty
        $done.Output[0] | Should -Be 'copy web02'
        $done.Output[1].Result | Should -Be 'Ok'
    }

    It 'drops handler output by default' {
        $job = Start-Background @"
Show-GuiWindow -Title 'PWRS-GUI test widgets dropped' -Renderer $script:Renderer { Button 'Say' -Name Say -OnClick { 'said' }; Button 'OK' -Name Ok -Result Ok }
"@
        $w = Wait-GuiWindow 'PWRS-GUI test widgets dropped' $job
        Invoke-GuiTestEvent -Window $w.Id -Name Say -Event Click
        Invoke-GuiTestEvent -Window $w.Id -Name Ok -Event Click
        $done = Complete-Background $job
        $done.Errors | Should -BeNullOrEmpty
        $done.Output.Count | Should -Be 1
        $done.Output[0].Result | Should -Be 'Ok'
    }
}

Describe 'A window described in PowerShell and opened with -NoWait, in <Mode> mode drawn with <Renderer>' -ForEach $cases {
    BeforeAll {
        Reset-GuiSession $Mode $Renderer
    }

    It 'returns the window once Get-GuiWindow lists it, and queues its events for Wait-GuiEvent, its Close last' {
        $w = Show-GuiWindow -NoWait -Title 'PWRS-GUI test widgets nowait' -Renderer $Renderer { ListBox 'a', 'b' -Name Pick; Button 'OK' -Name Ok -Result Ok }
        $w.PSObject.TypeNames[0] | Should -Be 'PwrsGui.Window'
        $w.SourceIdentifier | Should -BeNullOrEmpty
        @(Get-GuiWindow -Title 'PWRS-GUI test widgets nowait').Id | Should -Be $w.Id -Because 'the command returns once the first frame is drawn, which is when a window is listed'
        Invoke-GuiTestEvent -Window $w.Id -Name Pick -Event Select -Value 1
        $picked = Wait-GuiEvent -Window $w.Id -Timeout 20
        $picked.Kind | Should -Be 'Select'
        $picked.Name | Should -Be 'Pick'
        $picked.Value | Should -Be 'b'
        $picked.State.Pick | Should -Be 'b'
        Invoke-GuiTestEvent -Window $w.Id -Name Ok -Event Click
        (Wait-GuiEvent -Window $w.Id -Timeout 20).Kind | Should -Be 'Click'
        $closed = Wait-GuiEvent -Window $w.Id -Timeout 20
        $closed.Kind | Should -Be 'Close'
        $closed.Value | Should -Be 'Ok'
        { Wait-GuiEvent -Window $w.Id -Timeout 1 } | Should -Throw -ErrorId 'GuiNoWindow*'
    }

    It 'takes the window it acts on from the pipeline, as Show-GuiWindow -NoWait writes it' {
        $title = 'PWRS-GUI test widgets piped'
        $w = Show-GuiWindow -NoWait -Title $title -Renderer $Renderer { Text 'start' -Name Status; TextBox -Name Note; ListBox 'a', 'b' -Name Pick; Button 'Go' -Name Go; Button 'OK' -Name Ok -Result Ok }
        $null = Wait-GuiWindow $title
        $w | Update-GuiWidget -Name Status -Property @{ Text = 'piped' }
        $w | Add-GuiItem -Name Pick -Item 'c'
        $w | Remove-GuiItem -Name Pick -Item 'a'
        Wait-Until {
            $tree = ($w | Get-GuiSnapshot).Tree
            (Find-GuiNode $tree Status).Properties.text -eq 'piped' -and (@((Find-GuiNode $tree Pick).Properties.items) -join ',') -eq 'b,c'
        } 'the piped update and items reaching the window'
        $w | Set-GuiFocus -Name Note
        ($w | Get-GuiSnapshot).Focused | Should -Be 'Note'
        $w | Invoke-GuiTestEvent -Name Go -Event Click
        ($w | Wait-GuiEvent -Name Go -Timeout 20).Kind | Should -Be 'Click'
        $w | Invoke-GuiTestEvent -Name Ok -Event Click
        ($w | Wait-GuiEvent -Name Ok -Timeout 20).Kind | Should -Be 'Click'
    }

    It 'writes nothing when no event comes within -Timeout, and takes one widget''s events with -Name' {
        $w = Show-GuiWindow -NoWait -Title 'PWRS-GUI test widgets timeout' -Renderer $Renderer { Button 'A' -Name A; Button 'B' -Name B }
        $null = Wait-GuiWindow 'PWRS-GUI test widgets timeout'
        @(Wait-GuiEvent -Window $w.Id -Timeout 0.3) | Should -BeNullOrEmpty
        Invoke-GuiTestEvent -Window $w.Id -Name A -Event Click
        Invoke-GuiTestEvent -Window $w.Id -Name B -Event Click
        (Wait-GuiEvent -Window $w.Id -Name B -Timeout 20).Name | Should -Be 'B'
        (Wait-GuiEvent -Window $w.Id -Timeout 20).Name | Should -Be 'A'
        Close-GuiWindow -Window $w.Id -Result Done
        (Wait-GuiEvent -Window $w.Id -Timeout 20).Value | Should -Be 'Done'
    }

    It 'gives a press 3 points outside a widget to it, as egui''s hit test does, and one 6 points outside to none' {
        # Each widget stands alone, so the point beside it is near no other.
        $title = 'PWRS-GUI test widgets reach'
        $w = Show-GuiWindow -NoWait -Title $title -Renderer $Renderer -Width 480 -Height 640 {
            Button 'Press' -Name Press
            Spacer 24
            CheckBox 'Check' -Name Check
            Spacer 24
            Text 'a link' -Style Link -Name Link
            Spacer 24
            TextBox 'text' -Name Field
            Spacer 24
            Slider 50 -Name Level
            Spacer 24
            NumberBox 3 -Minimum 1 -Maximum 10 -Name Copies
        }
        $null = Wait-GuiWindow $title
        { Invoke-GuiTestEvent -Window $w.Id -Input @{ Press = 'Left'; X = 10 } } | Should -Throw -ErrorId 'GuiBadStep*'
        { Invoke-GuiTestEvent -Window $w.Id -Input @{ Move = $true; X = 10; Y = 10 }, @{ Move = $true; X = 10; Y = 5000 } -ErrorAction Stop } | Should -Throw -ErrorId 'GuiRefused*' -ExpectedMessage '*step 2 puts the pointer at 10, 5000*'
        { Invoke-GuiTestEvent -Window $w.Id -Input @{ Release = 'Right'; X = 10; Y = 10 } -ErrorAction Stop } | Should -Throw -ErrorId 'GuiRefused*' -ExpectedMessage '*does not hold*'
        { Invoke-GuiTestEvent -Window $w.Id -Input @{ Move = $true; X = 10; Y = 10 } -Timeout -1 } | Should -Throw -ErrorId 'GuiBadTimeout*'
        # No step is taken and seen in no time, so the first late one is
        # named; moves leave no button held for the presses after.
        { Invoke-GuiTestEvent -Window $w.Id -Input @{ Move = $true; X = 10; Y = 10 }, @{ Move = $true; X = 12; Y = 10 } -Timeout 0 -ErrorAction Stop } |
            Should -Throw -ErrorId 'GuiTimeout*' -ExpectedMessage '*step ? of 2, a move to 1?, 10, was not taken and seen within 0 seconds; -Timeout gives each step more time*'
        $bounds = (Get-GuiSnapshot -Window $w.Id).Bounds
        $click = { param([double] $X, [double] $Y) Invoke-GuiTestEvent -Window $w.Id -Input @{ Press = 'Left'; X = $X; Y = $Y }, @{ Release = 'Left'; X = $X; Y = $Y } }
        foreach ($name in 'Press', 'Check', 'Link') {
            $b = $bounds[$name]
            & $click ($b.X + $b.Width + 6) ($b.Y + $b.Height / 2)
            @(Wait-GuiEvent -Window $w.Id -Timeout 0.5) | Should -BeNullOrEmpty -Because "a press 6 points right of $name is beyond egui's reach"
            & $click ($b.X + $b.Width + 3) ($b.Y + $b.Height / 2)
            (Wait-GuiEvent -Window $w.Id -Timeout 20).Name | Should -Be $name -Because "a press 3 points right of $name is within egui's reach"
        }
        $top = $bounds.Press
        & $click ($top.X + $top.Width / 2) ($top.Y - 3)
        (Wait-GuiEvent -Window $w.Id -Timeout 20).Name | Should -Be 'Press' -Because 'a press 3 points above the button reaches it too'
        $f = $bounds.Field
        & $click ($f.X + $f.Width + 6) ($f.Y + $f.Height / 2)
        (Get-GuiSnapshot -Window $w.Id).Focused | Should -Not -Be 'Field' -Because 'a press 6 points right of the field is beyond egui''s reach'
        & $click ($f.X + $f.Width + 3) ($f.Y + $f.Height / 2)
        Wait-Until { (Get-GuiSnapshot -Window $w.Id).Focused -eq 'Field' } 'the field taking the keyboard at a press 3 points right of it'
        $n = $bounds.Copies
        & $click ($n.X + $n.Width + 6) ($n.Y + $n.Height / 2)
        (Get-GuiSnapshot -Window $w.Id).Focused | Should -Not -Be 'Copies' -Because 'a press 6 points right of the number is beyond egui''s reach'
        & $click ($n.X + $n.Width + 3) ($n.Y + $n.Height / 2)
        Wait-Until { (Get-GuiSnapshot -Window $w.Id).Focused -eq 'Copies' } 'the number taking the keyboard at a click 3 points right of it'
        $s = $bounds.Level
        foreach ($above in 6, 3) {
            $y = $s.Y - $above
            Invoke-GuiTestEvent -Window $w.Id -Input @{ Press = 'Left'; X = $s.X + 4; Y = $y }, @{ Move = $true; X = $s.X + 40; Y = $y }, @{ Release = 'Left'; X = $s.X + 40; Y = $y }
            $moved = Wait-GuiEvent -Window $w.Id -Name Level -Timeout $(if ($above -eq 6) { 0.5 } else { 20 })
            if ($above -eq 6) { @($moved) | Should -BeNullOrEmpty -Because 'a drag from 6 points above the slider is beyond egui''s reach' }
            else { $moved.Kind | Should -Be 'Change' -Because 'a drag from 3 points above the slider moves it' }
        }
        Close-GuiWindow -Window $w.Id
    }

    It 'gives a press 3 points outside a <Kind> to it, as egui''s hit test does, and one 6 points outside to none' -ForEach @(
        @{ Kind = 'drop-down' }, @{ Kind = 'color chooser' }, @{ Kind = 'date chooser' }, @{ Kind = 'tab' }, @{ Kind = 'split''s divider' },
        @{ Kind = 'table column''s edge' }, @{ Kind = 'Canvas' }, @{ Kind = 'GpuSurface' }
    ) {
        $title = "PWRS-GUI test widgets reach $Kind"
        $content = switch ($Kind) {
            'drop-down' { { Spacer 24; ComboBox 'one', 'two' -Name Target; ButtonBar { Button 'Cancel' -Name Cancel -Cancel } } }
            'color chooser' { { Spacer 24; ColorPicker '#336699' -Name Target; ButtonBar { Button 'Cancel' -Name Cancel -Cancel } } }
            'date chooser' { { Spacer 24; DatePicker ([datetime]'2026-10-08') -Name Target; ButtonBar { Button 'Cancel' -Name Cancel -Cancel } } }
            'tab' { { Spacer 24; Tabs -Name Target -Selected 'Two' { Tab 'One' { Text 'first' }; Tab 'Two' { Text 'second' } } } }
            'split''s divider' { { Split -Name Target -Position 0.5 { Column { Text 'left' -Name Left; Spacer 80 }; Text 'right' -Name Right } } }
            'table column''s edge' { { Table @([pscustomobject]@{ Name = 'alpha'; Size = 10 }, [pscustomobject]@{ Name = 'beta'; Size = 20 }) -Name Target } }
            'Canvas' { { Spacer 24; Row { Spacer 24; Canvas -Name Target -Width 200 -Height 100 @{ Kind = 'Rect'; X = 90; Y = 40; Width = 20; Height = 20; Fill = 'Navy' } } } }
            'GpuSurface' { { Spacer 24; Row { Spacer 24; GpuSurface Target -Width 120 -Height 80 -OnKey {} } } }
        }
        $w = Show-GuiWindow -NoWait -Title $title -Renderer $Renderer -Width 420 -Height 360 -Content $content
        $null = Wait-GuiWindow $title
        $t = (Get-GuiSnapshot -Window $w.Id).Bounds.Target
        $press = { param([double] $X, [double] $Y, [double] $DX = 0) Invoke-GuiTestEvent -Window $w.Id -Input @{ Press = 'Left'; X = $X; Y = $Y }, @{ Move = $true; X = $X + $DX; Y = $Y }, @{ Release = 'Left'; X = $X + $DX; Y = $Y } }
        $away = { Invoke-GuiTestEvent -Window $w.Id -Input @{ Move = $true; X = 2; Y = 2 } }
        foreach ($beyond in 6, 3) {
            $reaches = $beyond -eq 3
            $because = "a press $beyond points outside the $Kind is $(if ($reaches) { 'within' } else { 'beyond' }) egui's reach"
            switch ($Kind) {
                { $_ -in 'drop-down', 'color chooser', 'date chooser' } {
                    # The press is above the button that opens the popup, at
                    # its left, since a date chooser's hour and minute stand
                    # at its right. A popup open takes Escape, which otherwise
                    # presses Cancel.
                    & $press ($t.X + 10) ($t.Y - $beyond)
                    Invoke-GuiTestEvent -Window $w.Id -Input @{ Key = 'Escape' }
                    $cancel = Wait-GuiEvent -Window $w.Id -Name Cancel -Timeout $(if ($reaches) { 0.5 } else { 20 })
                    if ($reaches) { @($cancel) | Should -BeNullOrEmpty -Because "$because, so its popup opens and takes Escape" } else { $cancel.Kind | Should -Be 'Click' -Because $because }
                }
                'tab' {
                    & $press ($t.X + 10) ($t.Y - $beyond)
                    $changed = Wait-GuiEvent -Window $w.Id -Name Target -Timeout $(if ($reaches) { 20 } else { 0.5 })
                    if ($reaches) { $changed.Kind | Should -Be 'Change' -Because $because } else { @($changed) | Should -BeNullOrEmpty -Because $because }
                }
                'split''s divider' {
                    # The divider is 6 points wide in the middle of the split,
                    # and in egui as high as the taller pane's widgets. The
                    # left pane's spacer gives it room below the right pane's
                    # text, which would take a press beside the divider.
                    $before = (Get-GuiSnapshot -Window $w.Id).Bounds.Right.X
                    & $press ($t.X + $t.Width / 2 + 3 + $beyond) ($t.Y + $t.Height * 0.75) 40
                    $after = (Get-GuiSnapshot -Window $w.Id).Bounds.Right.X
                    if ($reaches) { $after - $before | Should -BeGreaterThan 20 -Because $because } else { $after | Should -Be $before -Because $because }
                }
                'table column''s edge' {
                    # The line between the columns, found in the table's
                    # empty room below its rows, where nothing else is drawn
                    # alike all the way down; its edge takes 3 points each
                    # side of it. The column widens when the line moves.
                    $lineOf = {
                        & $away
                        $shot = Get-GuiSnapshot -Window $w.Id -Name Target -Pixels
                        $scale = $shot.Width / $t.Width
                        $ys = 1..8 | ForEach-Object { [int](($t.Height - 70 + $_ * 7) * $scale) }
                        $at = { param([int] $x, [int] $y) [BitConverter]::ToInt32($shot.Pixels, (($y * $shot.Width) + $x) * 4) }
                        $found = 3..($shot.Width - 4) | Where-Object { $x = $_; $c = & $at $x $ys[0]; -not ($ys | Where-Object { (& $at $x $_) -ne $c }) -and $c -ne (& $at ($x - 3) $ys[0]) } | Select-Object -First 1
                        $found | Should -Not -BeNullOrEmpty -Because 'a line stands between the columns'
                        $found / $scale + $t.X
                    }
                    $line = & $lineOf
                    & $press ($line + 3 + $beyond) ($t.Y + $t.Height - 20) 60
                    $moved = (& $lineOf) - $line
                    if ($reaches) { $moved | Should -BeGreaterThan 20 -Because "$because, so the column widens" } else { $moved | Should -Be 0 -Because $because }
                }
                'Canvas' {
                    if (-not $reaches) { Set-GuiCanvasView Target -Window $w.Id -Controls Map }
                    $before = (Get-GuiCanvasView Target -Window $w.Id).Center
                    & $press ($t.X - $beyond) ($t.Y + $t.Height / 2) 40
                    if ($reaches) { Wait-Until { (Get-GuiCanvasView Target -Window $w.Id).Center[0] -ne $before[0] } "the canvas's view panning: $because" }
                    else { (Get-GuiCanvasView Target -Window $w.Id).Center[0] | Should -Be $before[0] -Because $because }
                }
                'GpuSurface' {
                    & $press ($t.X + $t.Width + $beyond) ($t.Y + $t.Height / 2)
                    if ($reaches) { Wait-Until { (Get-GuiSnapshot -Window $w.Id).Focused -eq 'Target' } "the surface taking the keyboard: $because" }
                    else { (Get-GuiSnapshot -Window $w.Id).Focused | Should -Not -Be 'Target' -Because $because }
                }
            }
        }
        Close-GuiWindow -Window $w.Id
    }

    It 'zooms a chart to the box a secondary drag draws, and draws none for a press that is no drag' {
        $title = 'PWRS-GUI test widgets chart box'
        $w = Show-GuiWindow -NoWait -Title $title -Renderer $Renderer -Width 420 -Height 300 { Chart -Name Load -Height 200 -Series @{ Name = 'cpu'; Y = 3, 5, 4, 6 } }
        $null = Wait-GuiWindow $title
        $c = (Get-GuiSnapshot -Window $w.Id).Bounds.Load
        $away = { Invoke-GuiTestEvent -Window $w.Id -Input @{ Move = $true; X = 2; Y = 2 } }
        $look = { & $away; [BitConverter]::ToString([Security.Cryptography.SHA256]::Create().ComputeHash((Get-GuiSnapshot -Window $w.Id -Name Load -Pixels).Pixels)) }
        # The box runs from where the press became a drag, as egui_plot's
        # does, so the pointer moves halfway first, then the rest.
        $right = { param([double] $DX, [double] $DY) $x = $c.X + $c.Width * 0.4; $y = $c.Y + $c.Height * 0.4; Invoke-GuiTestEvent -Window $w.Id -Input @{ Press = 'Right'; X = $x; Y = $y }, @{ Move = $true; X = $x + $DX / 2; Y = $y + $DY / 2 }, @{ Move = $true; X = $x + $DX; Y = $y + $DY }, @{ Release = 'Right'; X = $x + $DX; Y = $y + $DY } }
        $first = & $look
        & $right 4 3
        & $look | Should -Be $first -Because 'a press that moves 5 points is no drag, so it draws no box'
        & $right 60 40
        & $look | Should -Not -Be $first -Because 'a secondary drag zooms the chart to its box'
        Close-GuiWindow -Window $w.Id
    }

    It 'selects a table''s row at a click on its text, and copies the rows selected at Ctrl+C as Out-GridView copies them' {
        $title = 'PWRS-GUI test widgets table copy'
        $rows = 1..3 | ForEach-Object { [pscustomobject]@{ Name = "name$_"; Size = $_ * 10 } }
        $w = Show-GuiWindow -NoWait -Title $title -Renderer $Renderer -Width 420 -Height 300 { Table $rows -Selection Multiple -Name Files }
        $null = Wait-GuiWindow $title
        $opened = Get-GuiSnapshot -Window $w.Id
        $opened.Copied | Should -BeNullOrEmpty -Because 'a window has copied nothing until a copy'
        $b = $opened.Bounds.Files
        # The first column's text in the first and the third rows, under the
        # heading, the rows 21 points apart.
        $x = $b.X + 14
        $first = $b.Y + 30
        $third = $first + 42
        Invoke-GuiTestEvent -Window $w.Id -Input @{ Press = 'Left'; X = $x; Y = $first }, @{ Release = 'Left'; X = $x; Y = $first }
        @((Wait-GuiEvent -Window $w.Id -Name Files -Timeout 20).Value).Name | Should -Be 'name1' -Because 'a click on a cell''s text selects its row'
        Invoke-GuiTestEvent -Window $w.Id -Input @{ Press = 'Left'; X = $x; Y = $third; Ctrl = $true }, @{ Release = 'Left'; X = $x; Y = $third; Ctrl = $true }
        @((Wait-GuiEvent -Window $w.Id -Name Files -Timeout 20).Value).Name | Should -Be 'name1', 'name3'
        Invoke-GuiTestEvent -Window $w.Id -Input @{ Key = 'Ctrl+C' }
        $line = [Environment]::NewLine
        $copied = "name1`t10`t${line}name3`t30`t$line"
        Wait-Until { (Get-GuiSnapshot -Window $w.Id).Copied -ceq $copied } 'the window reporting the rows it copied'
        Get-SystemClipboard | Should -BeExactly $copied -Because 'the rows copied are on the system''s clipboard'
        Close-GuiWindow -Window $w.Id
    }

    It 'gives a table the keyboard focus at a press below its last row, and selects no row' {
        $title = 'PWRS-GUI test widgets table press'
        $rows = 1..2 | ForEach-Object { [pscustomobject]@{ Name = "name$_"; Size = $_ * 10 } }
        $w = Show-GuiWindow -NoWait -Title $title -Renderer $Renderer -Width 420 -Height 300 { TextBox 'note' -Name Note; Table $rows -Selection Multiple -Name Files }
        $null = Wait-GuiWindow $title
        $shot = Get-GuiSnapshot -Window $w.Id
        $n = $shot.Bounds.Note
        Invoke-GuiTestEvent -Window $w.Id -Input @{ Press = 'Left'; X = $n.X + 10; Y = $n.Y + $n.Height / 2 }, @{ Release = 'Left'; X = $n.X + 10; Y = $n.Y + $n.Height / 2 }
        Wait-Until { (Get-GuiSnapshot -Window $w.Id).Focused -eq 'Note' } 'the field taking the keyboard first'
        # The table takes the window's room below the field, so its two rows
        # leave room below them.
        $b = $shot.Bounds.Files
        $x = $b.X + 30
        $y = $b.Y + $b.Height - 20
        Invoke-GuiTestEvent -Window $w.Id -Input @{ Press = 'Left'; X = $x; Y = $y }, @{ Release = 'Left'; X = $x; Y = $y }
        Wait-Until { (Get-GuiSnapshot -Window $w.Id).Focused -eq 'Files' } 'the table taking the keyboard at a press below its last row'
        (Get-GuiSnapshot -Window $w.Id).State.Files.selected | Should -BeNullOrEmpty -Because 'a press below the last row selects no row'
        @(Wait-GuiEvent -Window $w.Id -Name Files -Timeout 0.5) | Should -BeNullOrEmpty -Because 'a press below the last row raises no Select'
        Close-GuiWindow -Window $w.Id
    }

    It 'pastes the system''s clipboard into a field at Ctrl+V' -Skip:$onWindows {
        # A paste test puts its text on the clipboard first, so it runs only
        # where the clipboard is no person's.
        $title = 'PWRS-GUI test widgets paste'
        $w = Show-GuiWindow -NoWait -Title $title -Renderer $Renderer -Width 320 -Height 160 { TextBox -Name Note }
        $null = Wait-GuiWindow $title
        Set-SystemClipboard 'pasted text'
        $b = (Get-GuiSnapshot -Window $w.Id).Bounds.Note
        $x = $b.X + $b.Width / 2
        $y = $b.Y + $b.Height / 2
        Invoke-GuiTestEvent -Window $w.Id -Input @{ Press = 'Left'; X = $x; Y = $y }, @{ Release = 'Left'; X = $x; Y = $y }, @{ Key = 'Ctrl+V' }
        Wait-Until { (Get-GuiSnapshot -Window $w.Id).State.Note -eq 'pasted text' } 'the clipboard''s text reaching the field'
        Close-GuiWindow -Window $w.Id
    }

    It 'runs a handler as an engine-event action, whose update reaches its window' {
        $w = Show-GuiWindow -NoWait -Title 'PWRS-GUI test widgets nowait handler' -Renderer $Renderer {
            Text 'idle' -Name Status
            Button 'Go' -Name Go -OnClick { param($e) Update-GuiWidget Status @{ Text = "went $($e.Name) $($e.Kind)" } }
        }
        $w.SourceIdentifier | Should -Not -BeNullOrEmpty
        $null = Wait-GuiWindow 'PWRS-GUI test widgets nowait handler'
        Invoke-GuiTestEvent -Window $w.Id -Name Go -Event Click
        # An action runs when the engine is idle, which Wait-Event gives it.
        Wait-Until { $null = Wait-Event -Timeout 0.1; (Find-GuiNode (Get-GuiSnapshot -Window $w.Id).Tree 'Status').Properties.text -eq 'went Go Click' } 'the action''s update reaching the window'
        Close-GuiWindow -Window $w.Id
        (Wait-GuiEvent -Window $w.Id -Timeout 20).Kind | Should -Be 'Click'
        (Wait-GuiEvent -Window $w.Id -Timeout 20).Kind | Should -Be 'Close'
        Unregister-Event -SourceIdentifier $w.SourceIdentifier
    }

    It 'gives a table''s selection as its objects to Wait-GuiEvent and to a handler''s action' {
        $global:PwrsGuiTablePicked = $null
        $items = 1..2 | ForEach-Object { [pscustomobject]@{ Name = "t$_"; Extra = $_ } }
        $w = Show-GuiWindow -NoWait -Title 'PWRS-GUI test widgets nowait table' -Renderer $Renderer { Table $items -Property Name -Name Things -OnSelect { param($e) $global:PwrsGuiTablePicked = $e.Value.Name } }
        $null = Wait-GuiWindow 'PWRS-GUI test widgets nowait table'
        Wait-Until { (Get-GuiSnapshot -Window $w.Id).State.Things.rows -eq 2 } 'the table''s two rows arriving'
        @((Find-GuiNode (Get-GuiSnapshot -Window $w.Id).Tree 'Things').Properties.columns.label) | Should -Be @('Name')
        Invoke-GuiTestEvent -Window $w.Id -Name Things -Event Select -Value 1
        $picked = Wait-GuiEvent -Window $w.Id -Timeout 20
        $picked.Value.Name | Should -Be 't2'
        $picked.Value.Extra | Should -Be 2
        $picked.State.Things.Name | Should -Be 't2'
        Wait-Until { $null = Wait-Event -Timeout 0.1; $global:PwrsGuiTablePicked -eq 't2' } 'the action getting the selected object'
        Close-GuiWindow -Window $w.Id
        (Wait-GuiEvent -Window $w.Id -Timeout 20).Kind | Should -Be 'Close'
        Unregister-Event -SourceIdentifier $w.SourceIdentifier
        Remove-Variable -Name PwrsGuiTablePicked -Scope Global
    }

    It 'adds and removes a list''s items, leaving the selection of those that stay' {
        $w = Show-GuiWindow -NoWait -Title 'PWRS-GUI test widgets items' -Renderer $Renderer { ListBox 'a', 'b', 'c' -Name Letters -Multiple -Selected 'b', 'c' }
        $null = Wait-GuiWindow 'PWRS-GUI test widgets items'
        Add-GuiItem -Window $w.Id -Name Letters -Item 'd'
        Remove-GuiItem -Window $w.Id -Name Letters -Item 'B'
        $node = Find-GuiNode (Get-GuiSnapshot -Window $w.Id).Tree 'Letters'
        @($node.Properties.items) | Should -Be @('a', 'c', 'd')
        @($node.Properties.selected) | Should -Be @(1)
        { Remove-GuiItem -Window $w.Id -Name Letters -Item 'zz' } | Should -Throw -ErrorId 'GuiNoItem*'
        Close-GuiWindow -Window $w.Id
        (Wait-GuiEvent -Window $w.Id -Timeout 20).Kind | Should -Be 'Close'
    }

    It 'adds a log''s lines, takes one line given as a string, and refuses to take any away' {
        $w = Show-GuiWindow -NoWait -Title 'PWRS-GUI test widgets log' -Renderer $Renderer { LogView 'first' -Name Log }
        $null = Wait-GuiWindow 'PWRS-GUI test widgets log'
        Add-GuiItem -Window $w.Id -Name Log -Item 'second', 'third'
        Wait-Until { @((Find-GuiNode (Get-GuiSnapshot -Window $w.Id).Tree 'Log').Properties.lines).Count -eq 3 } 'the two new lines reaching the log'
        @((Find-GuiNode (Get-GuiSnapshot -Window $w.Id).Tree 'Log').Properties.lines) | Should -Be @('first', 'second', 'third')
        { Remove-GuiItem -Window $w.Id -Name Log -Item 'first' } | Should -Throw -ErrorId 'GuiNoItems*'
        Update-GuiWidget -Window $w.Id -Name Log -Property @{ Line = @() }
        Wait-Until { @((Find-GuiNode (Get-GuiSnapshot -Window $w.Id).Tree 'Log').Properties.lines).Count -eq 0 } 'the log being cleared'
        Update-GuiWidget -Window $w.Id -Name Log -Property @{ Line = 'only line' }
        Wait-Until { @((Find-GuiNode (Get-GuiSnapshot -Window $w.Id).Tree 'Log').Properties.lines).Count -gt 0 } 'the line given as a string reaching the log'
        @((Find-GuiNode (Get-GuiSnapshot -Window $w.Id).Tree 'Log').Properties.lines) | Should -Be @('only line')
        Close-GuiWindow -Window $w.Id
        (Wait-GuiEvent -Window $w.Id -Timeout 20).Kind | Should -Be 'Close'
    }

    It 'adds a chart''s points, each joining its series or starting one, and replaces them with new series' {
        $w = Show-GuiWindow -NoWait -Title 'PWRS-GUI test widgets chart points' -Renderer $Renderer { Chart -Name Load -Height 160 -Series @{ Name = 'cpu'; Y = 1, 2 } }
        $null = Wait-GuiWindow 'PWRS-GUI test widgets chart points'
        Add-GuiItem -Window $w.Id -Name Load -Item @{ Series = 'cpu'; Y = 3 }, @{ Series = 'mem'; X = 5; Y = 9 }
        Wait-Until { @((Find-GuiNode (Get-GuiSnapshot -Window $w.Id).Tree 'Load').Properties.points).Count -eq 2 } 'the two points reaching the chart'
        Invoke-GuiTestEvent -Window $w.Id -Name Load -Event Select -Value @{ Series = 'cpu'; X = 2 }
        (Wait-GuiEvent -Window $w.Id -Timeout 20).Value.Y | Should -Be 3 -Because 'a point without X takes its series'' count of points'
        Invoke-GuiTestEvent -Window $w.Id -Name Load -Event Select -Value @{ Series = 'mem'; X = 5 }
        (Wait-GuiEvent -Window $w.Id -Timeout 20).Value.Y | Should -Be 9
        { Add-GuiItem -Window $w.Id -Name Load -Item @{ Series = 'cpu'; X = (Get-Date); Y = 1 } } | Should -Throw -ErrorId 'GuiBadPoint*'
        { Add-GuiItem -Window $w.Id -Name Load -Item @{ Y = 1 } } | Should -Throw -ErrorId 'GuiBadPoint*'
        { Remove-GuiItem -Window $w.Id -Name Load -Item 'cpu' } | Should -Throw -ErrorId 'GuiNoItems*'
        Update-GuiWidget -Window $w.Id -Name Load -Property @{ Series = @() }
        Wait-Until { @((Find-GuiNode (Get-GuiSnapshot -Window $w.Id).Tree 'Load').Properties.points).Count -eq 0 } 'the new series replacing the points'
        Update-GuiWidget -Window $w.Id -Name Load -Property @{ Series = @{ Name = 'one'; X = 4; Y = 8 } }
        Wait-Until { @((Find-GuiNode (Get-GuiSnapshot -Window $w.Id).Tree 'Load').Properties.series).Count -eq 1 } 'one series given alone replacing none'
        Invoke-GuiTestEvent -Window $w.Id -Name Load -Event Select -Value @{ Series = 'one'; X = 4 }
        (Wait-GuiEvent -Window $w.Id -Timeout 20).Value.Y | Should -Be 8 -Because 'a hashtable given alone is one series, and one number for X or Y one point'
        Close-GuiWindow -Window $w.Id
        (Wait-GuiEvent -Window $w.Id -Timeout 20).Kind | Should -Be 'Close'
    }

    It 'moves a progress bar and adds a log''s lines and a chart''s points from SubEtha rings another runspace writes, warning of an item a widget cannot take' {
        Import-Module (Join-Path $env:SUBETHA_MODULE 'SubEtha.psd1') -ErrorAction Stop
        $base = Join-Path ([IO.Path]::GetTempPath()) "pwrs-gui-test-rings-$PID-$Mode-$Renderer"
        $null = New-Item -ItemType Directory -Path $base -Force
        $at = @{ Copy = Join-Path $base 'copy'; Log = Join-Path $base 'log'; Load = Join-Path $base 'load' }
        $copy = New-SubEthaRing -Path $at.Copy -Capacity 64
        $log = New-SubEthaBroadcastRing -Path $at.Log -Capacity 64
        $load = New-SubEthaLocaleRing -Path $at.Load -Capacity 64
        try {
            $load.MigrateTo('File')
            $job = Start-Background @"
Import-Module (Join-Path '$env:SUBETHA_MODULE' 'SubEtha.psd1')
`$rings = @((New-SubEthaRing -Path '$($at.Copy)' -Capacity 64), (New-SubEthaBroadcastRing -Path '$($at.Log)' -Capacity 64), (New-SubEthaLocaleRing -Path '$($at.Load)' -Capacity 64))
try {
    Show-GuiWindow -Title 'PWRS-GUI test widgets rings' {
        ProgressBar -Name Copy -Ring `$rings[0]
        LogView -Name Log -Height 80 -Ring `$rings[1]
        Chart -Name Load -Height 120 -Ring `$rings[2]
        Button 'OK' -Name Ok -Result Ok
    }
} finally {
    foreach (`$r in `$rings) { `$r.Dispose() }
}
"@
            $w = Wait-GuiWindow 'PWRS-GUI test widgets rings' $job
            $node = { param($name) Find-GuiNode (Get-GuiSnapshot -Window $w.Id).Tree $name }
            $p = $copy.RegisterProducer()
            $copy.SendFrame($p, '{"Value": 40, "Text": "copying"}') | Should -BeTrue
            Wait-Until { (& $node 'Copy').Properties.value -eq 40 } 'the bar moving to 40'
            (& $node 'Copy').Properties.text | Should -Be 'copying'
            $log.WaitForConsumers(1, 20) | Should -Be 1 -Because 'the window reads the broadcast ring'
            $log.Push('copied a.txt') | Should -BeTrue
            Wait-Until { @((& $node 'Log').Properties.lines) -contains 'copied a.txt' } 'the line reaching the log'
            $q = $load.RegisterProducer()
            $load.Send($q, '{"Series": "cpu", "Y": 5}') | Should -BeTrue
            Wait-Until { @((& $node 'Load').Properties.points).Count -eq 1 } 'the point reaching the chart'
            $copy.SendFrame($p, '{"Value": "half"}') | Should -BeTrue
            Wait-Until { @($job.PS.Streams.Warning | Where-Object { $_.Message -like '*took no item from its ring*' }).Count -ge 1 } 'the item the bar cannot take being reported'
            Invoke-GuiTestEvent -Window $w.Id -Name Ok -Event Click
            $done = Complete-Background $job
            $done.Errors | Should -BeNullOrEmpty
            $done.Output[-1].Result | Should -Be 'Ok'
            @($done.Warnings | Where-Object { $_ -like '*ProgressBar Copy took no item from its ring: Value is "half"; it must be a finite number*' }).Count | Should -Be 1
        } finally {
            foreach ($r in $copy, $log, $load) { $r.Dispose() }
            [GC]::Collect()
            [GC]::WaitForPendingFinalizers()
            Wait-Until { Remove-Item -LiteralPath $base -Recurse -Force -ErrorAction Ignore; -not (Test-Path -LiteralPath $base) } 'the rings'' folder being removed once the window let go of them'
        }
    }

    It 'adds and removes a drop-down''s choices, the one chosen staying chosen' {
        $w = Show-GuiWindow -NoWait -Title 'PWRS-GUI test widgets choices' -Renderer $Renderer { ComboBox 'a', 'b' -Name Pick -Selected b }
        $null = Wait-GuiWindow 'PWRS-GUI test widgets choices'
        Add-GuiItem -Window $w.Id -Name Pick -Item 'c'
        Remove-GuiItem -Window $w.Id -Name Pick -Item 'a'
        $shot = Get-GuiSnapshot -Window $w.Id
        $node = Find-GuiNode $shot.Tree 'Pick'
        @($node.Properties.items) | Should -Be @('b', 'c')
        $node.Properties.selected | Should -Be 0
        $shot.State.Pick | Should -Be 0
        Close-GuiWindow -Window $w.Id
        (Wait-GuiEvent -Window $w.Id -Timeout 20).State.Pick | Should -Be 'b'
    }

    It 'gives an Image a new picture from bytes or from a file, and refuses both at once or bytes of no picture' {
        $red = [Convert]::FromBase64String('iVBORw0KGgoAAAANSUhEUgAAAAgAAAAECAIAAAA8r+mnAAAAEklEQVR4nGP4z8CAFWEXJUsCAFpeH+EeQoQoAAAAAElFTkSuQmCC')
        $green = [Convert]::FromBase64String('iVBORw0KGgoAAAANSUhEUgAAAAgAAAAECAIAAAA8r+mnAAAAEElEQVR4nGNg+M+AHVFPAgA6fh/hYeiK3wAAAABJRU5ErkJggg==')
        $file = Join-Path ([IO.Path]::GetTempPath()) "pwrs-gui-test-picture-update-$PID-$Renderer.png"
        [IO.File]::WriteAllBytes($file, $red)
        function Middle($shot) {
            if (-not $shot) { return @() }
            $at = ([int][math]::Floor($shot.Height / 2) * $shot.Width + [int][math]::Floor($shot.Width / 2)) * 4
            @($shot.Pixels[$at..($at + 2)])
        }
        function Near($rgb, $want) { $rgb.Count -eq 3 -and @(0..2 | Where-Object { [math]::Abs([int]$rgb[$_] - $want[$_]) -gt 8 }).Count -eq 0 }
        $w = Show-GuiWindow -NoWait -Title 'PWRS-GUI test widgets picture update' -Renderer $Renderer { Image -Bytes $red -Width 40 -Name Pic }
        $null = Wait-GuiWindow 'PWRS-GUI test widgets picture update'
        Wait-Until { Near (Middle (Get-GuiSnapshot -Window $w.Id -Name Pic -Pixels -ErrorAction Ignore)) @(255, 0, 0) } 'the red picture being drawn'
        Update-GuiWidget -Window $w.Id -Name Pic -Property @{ Bytes = $green }
        Wait-Until { Near (Middle (Get-GuiSnapshot -Window $w.Id -Name Pic -Pixels -ErrorAction Ignore)) @(0, 255, 0) } 'the green picture taking its place'
        Update-GuiWidget -Window $w.Id -Name Pic -Property @{ Path = $file }
        Wait-Until { Near (Middle (Get-GuiSnapshot -Window $w.Id -Name Pic -Pixels -ErrorAction Ignore)) @(255, 0, 0) } 'the file''s picture taking its place'
        $both = { Update-GuiWidget -Window $w.Id -Name Pic -Property @{ Path = $file; Bytes = $green } } | Should -Throw -ErrorId 'GuiBadProperty*' -PassThru
        $both.Exception.Message | Should -BeLike '*not both*'
        $none = { Update-GuiWidget -Window $w.Id -Name Pic -Property @{ Bytes = [byte[]](0x49, 0x49, 0x2A, 0) } } | Should -Throw -ErrorId 'GuiBadProperty*' -PassThru
        $none.Exception.Message | Should -BeLike '*a TIFF image*'
        Close-GuiWindow -Window $w.Id
        (Wait-GuiEvent -Window $w.Id -Timeout 20).Kind | Should -Be 'Close'
        Remove-Item -LiteralPath $file
    }

    It 'adds rows to a table and takes rows away by the objects they were made from, leaving the selection among the rows that stay' {
        $rows = 1..3 | ForEach-Object { [pscustomobject]@{ Name = "n$_"; Size = $_ * 10 } }
        $w = Show-GuiWindow -NoWait -Title 'PWRS-GUI test widgets table rows' -Renderer $Renderer { Table $rows -Selection Multiple -Name Files }
        $null = Wait-GuiWindow 'PWRS-GUI test widgets table rows'
        Wait-Until { (Get-GuiSnapshot -Window $w.Id).State.Files.rows -eq 3 } 'the table''s three rows arriving'
        Add-GuiItem -Window $w.Id -Name Files -Item ([pscustomobject]@{ Name = 'n4'; Size = 40 })
        Wait-Until { (Get-GuiSnapshot -Window $w.Id).State.Files.rows -eq 4 } 'the added row arriving'
        Invoke-GuiTestEvent -Window $w.Id -Name Files -Event Select -Value 1, 3
        @((Wait-GuiEvent -Window $w.Id -Name Files -Timeout 20).Value.Name) | Should -Be @('n2', 'n4')
        Remove-GuiItem -Window $w.Id -Name Files -Item $rows[0]
        Wait-Until { (Get-GuiSnapshot -Window $w.Id).State.Files.rows -eq 3 } 'the row taken away'
        @((Get-GuiSnapshot -Window $w.Id).State.Files.selected) | Should -Be @(0, 2)
        { Remove-GuiItem -Window $w.Id -Name Files -Item $rows[0] } | Should -Throw -ErrorId 'GuiNoRow*'
        { Remove-GuiItem -Window $w.Id -Name Files -Item ([pscustomobject]@{ Name = 'n2' }) } | Should -Throw -ErrorId 'GuiNoRow*'
        Invoke-GuiTestEvent -Window $w.Id -Name Files -Event Activate -Value 2
        (Wait-GuiEvent -Window $w.Id -Name Files -Timeout 20).Value.Name | Should -Be 'n4'
        Close-GuiWindow -Window $w.Id
        (Wait-GuiEvent -Window $w.Id -Timeout 20).Kind | Should -Be 'Close'
    }

    It 'scrolls a table showing its last row to the new last row as Add-GuiItem adds rows, and leaves one showing its first rows where it is' {
        # Selecting the last row changes the table's picture only where that row is in sight.
        $named = { param($from, $to) $from..$to | ForEach-Object { [pscustomobject]@{ Name = "n$_" } } }
        # The table's picture once two captures in a row agree, so a scroll bar still
        # fading in as its window opens is not taken for the rows moving.
        $still = {
            param($window)
            $seen = @{ Last = '' }
            Wait-Until {
                $shot = [Convert]::ToBase64String((Get-GuiSnapshot -Window $window -Name Lines -Pixels).Pixels)
                $same = $shot -eq $seen.Last
                $seen.Last = $shot
                $same
            } 'the table''s picture staying still'
            $seen.Last
        }
        foreach ($case in @(@{ Opened = 3; Follows = $true }, @{ Opened = 40; Follows = $false })) {
            $title = "PWRS-GUI test widgets table end $($case.Opened)"
            $opened = & $named 1 $case.Opened
            $w = Show-GuiWindow -NoWait -Title $title -Renderer $Renderer -Height 260 { Table $opened -Selection Single -Name Lines }
            $null = Wait-GuiWindow $title
            $total = $case.Opened + 20
            Add-GuiItem -Window $w.Id -Name Lines -Item (& $named ($case.Opened + 1) $total)
            Wait-Until { (Get-GuiSnapshot -Window $w.Id).State.Lines.rows -eq $total } 'the added rows arriving'
            $before = & $still $w.Id
            Invoke-GuiTestEvent -Window $w.Id -Name Lines -Event Select -Value ($total - 1)
            (Wait-GuiEvent -Window $w.Id -Name Lines -Timeout 20).Value.Name | Should -Be "n$total"
            $after = & $still $w.Id
            $inSight = $before -ne $after
            $inSight | Should -Be $case.Follows -Because "a table that opened with $($case.Opened) rows in room for about seven $(if ($case.Follows) { 'follows' } else { 'stays at its first rows' })"
            Close-GuiWindow -Window $w.Id
            (Wait-GuiEvent -Window $w.Id -Timeout 20).Kind | Should -Be 'Close'
        }
    }

    It 'takes a file chooser''s new path from the current location, as its command does' {
        $w = Show-GuiWindow -NoWait -Title 'PWRS-GUI test widgets chooser path' -Renderer $Renderer { FilePicker -Name Log }
        $null = Wait-GuiWindow 'PWRS-GUI test widgets chooser path'
        Update-GuiWidget -Window $w.Id -Name Log -Property @{ Path = 'relative.log' }
        @((Find-GuiNode (Get-GuiSnapshot -Window $w.Id).Tree 'Log').Properties.paths) | Should -Be @((Join-Path (Get-Location).ProviderPath 'relative.log'))
        Close-GuiWindow -Window $w.Id
        (Wait-GuiEvent -Window $w.Id -Timeout 20).Kind | Should -Be 'Close'
    }

    It 'gives a Markdown widget a new document from text or a file, refuses both at once, and warns of an image it cannot or must not read' {
        $file = Join-Path ([IO.Path]::GetTempPath()) "pwrs-gui-test-markdown-update-$PID-$Renderer.md"
        [IO.File]::WriteAllText($file, '[from the file](https://example.com/file)')
        $w = Show-GuiWindow -NoWait -Title 'PWRS-GUI test widgets markdown update' -Renderer $Renderer { Markdown "[first](https://example.com/first) ![missing](no-such-picture-$PID.png) ![shared](//server/share/x.png)" -Name Doc } -WarningVariable said -WarningAction SilentlyContinue
        $null = Wait-GuiWindow 'PWRS-GUI test widgets markdown update'
        @($said | Where-Object { "$_" -like "*no-such-picture-$PID.png shows its alt text*" }).Count | Should -Be 1
        @($said | Where-Object { "$_" -like '*//server/share/x.png shows its alt text*credentials*' }).Count | Should -Be 1
        Update-GuiWidget -Window $w.Id -Name Doc -Property @{ Text = '[second](https://example.com/second)' }
        Invoke-GuiTestEvent -Window $w.Id -Name Doc -Event Click -Value 'https://example.com/second'
        (Wait-GuiEvent -Window $w.Id -Name Doc -Timeout 20).Value | Should -Be 'https://example.com/second'
        Update-GuiWidget -Window $w.Id -Name Doc -Property @{ Path = $file }
        Invoke-GuiTestEvent -Window $w.Id -Name Doc -Event Click -Value 'https://example.com/file'
        (Wait-GuiEvent -Window $w.Id -Name Doc -Timeout 20).Value | Should -Be 'https://example.com/file'
        $both = { Update-GuiWidget -Window $w.Id -Name Doc -Property @{ Text = 'a'; Path = $file } } | Should -Throw -ErrorId 'GuiBadProperty*' -PassThru
        $both.Exception.Message | Should -BeLike '*not both*'
        Close-GuiWindow -Window $w.Id
        (Wait-GuiEvent -Window $w.Id -Timeout 20).Kind | Should -Be 'Close'
        Remove-Item -LiteralPath $file
    }

    It 'adds nodes under the node an object names, and removes a node with the nodes under it' {
        $fruit = @{ Text = 'Fruit'; Children = 'Apple', 'Pear' }
        $w = Show-GuiWindow -NoWait -Title 'PWRS-GUI test widgets tree items' -Renderer $Renderer { TreeView -Name Food -Node $fruit, 'Leek' -Multiple }
        $null = Wait-GuiWindow 'PWRS-GUI test widgets tree items'
        Add-GuiItem -Window $w.Id -Name Food -Item 'Kiwi' -Parent $fruit
        Add-GuiItem -Window $w.Id -Name Food -Item @{ Text = 'Nuts'; Children = 'Pecan' }
        Add-GuiItem -Window $w.Id -Name Food -Item 'Pecan' -Parent 'LEEK'
        $tree = Find-GuiNode (Get-GuiSnapshot -Window $w.Id).Tree 'Food'
        @($tree.Children.Properties.text) | Should -Be @('Fruit', 'Leek', 'Nuts')
        @($tree.Children[0].Children.Properties.text) | Should -Be @('Apple', 'Pear', 'Kiwi')
        $tree.Children[1].Children[0].Properties.text | Should -Be 'Pecan'
        $twice = { Remove-GuiItem -Window $w.Id -Name Food -Item 'pecan' } | Should -Throw -ErrorId 'GuiTwoNodes*' -PassThru
        $twice.Exception.Message | Should -BeLike '*Pecan under Nuts*Pecan under Leek*'
        Invoke-GuiTestEvent -Window $w.Id -Name Food -Event Expand -Value 0
        Invoke-GuiTestEvent -Window $w.Id -Name Food -Event Select -Value 1, 4
        @((Wait-GuiEvent -Window $w.Id -Name Food -Timeout 20).Value) | Should -Be @('Apple', 'Leek')
        Remove-GuiItem -Window $w.Id -Name Food -Item $fruit
        $shot = Get-GuiSnapshot -Window $w.Id
        @((Find-GuiNode $shot.Tree 'Food').Children.Properties.text) | Should -Be @('Leek', 'Nuts')
        @($shot.State.Food.selected).Count | Should -Be 1
        { Remove-GuiItem -Window $w.Id -Name Food -Item 'Apple' } | Should -Throw -ErrorId 'GuiNoNode*'
        { Add-GuiItem -Window $w.Id -Name Food -Item 'x' -Parent $fruit } | Should -Throw -ErrorId 'GuiNoNode*'
        Close-GuiWindow -Window $w.Id
        @((Wait-GuiEvent -Window $w.Id -Timeout 20).State.Food) | Should -Be @('Leek')
    }

    It 'fills a node through -OnExpand as an engine-event action, and gives Wait-GuiEvent the node''s object' {
        $w = Show-GuiWindow -NoWait -Title 'PWRS-GUI test widgets nowait tree' -Renderer $Renderer {
            TreeView -Name Lazy -Node ([pscustomobject]@{ Name = 'root' }) -Text Name -HasChildren { $_.Name -eq 'root' } -OnExpand { param($e) [pscustomobject]@{ Name = "under $($e.Value.Name)" } }
        }
        $null = Wait-GuiWindow 'PWRS-GUI test widgets nowait tree'
        Invoke-GuiTestEvent -Window $w.Id -Name Lazy -Event Expand -Value 0
        $opened = Wait-GuiEvent -Window $w.Id -Timeout 20
        $opened.Kind | Should -Be 'Expand'
        $opened.Value.Name | Should -Be 'root'
        Wait-Until { $null = Wait-Event -Timeout 0.1; @((Get-GuiSnapshot -Window $w.Id).State.Lazy.shown).Count -eq 2 } 'the action giving the node its child'
        (Find-GuiNode (Get-GuiSnapshot -Window $w.Id).Tree 'Lazy').Children[0].Children[0].Properties.text | Should -Be 'under root'
        Invoke-GuiTestEvent -Window $w.Id -Name Lazy -Event Select -Value 1
        (Wait-GuiEvent -Window $w.Id -Timeout 20).Value.Name | Should -Be 'under root'
        Close-GuiWindow -Window $w.Id
        $closed = Wait-GuiEvent -Window $w.Id -Timeout 20
        $closed.Kind | Should -Be 'Close'
        $closed.State.Lazy.Name | Should -Be 'under root'
        Unregister-Event -SourceIdentifier $w.SourceIdentifier
    }

    It 'runs -Text and -HasChildren, both script blocks, with each node as $_' {
        $w = Show-GuiWindow -NoWait -Title 'PWRS-GUI test widgets nowait tree blocks' -Renderer $Renderer {
            TreeView -Name Lazy -Node ([pscustomobject]@{ Name = 'root' }) -Text { $_.Name.ToUpper() } -HasChildren { -not $_.Leaf } -OnExpand {
                [pscustomobject]@{ Name = 'leaf'; Leaf = $true }
                [pscustomobject]@{ Name = 'branch'; Leaf = $false }
            }
        }
        $null = Wait-GuiWindow 'PWRS-GUI test widgets nowait tree blocks'
        $root = (Find-GuiNode (Get-GuiSnapshot -Window $w.Id).Tree 'Lazy').Children[0]
        $root.Properties.text | Should -Be 'ROOT'
        $root.Properties.more | Should -BeTrue
        Invoke-GuiTestEvent -Window $w.Id -Name Lazy -Event Expand -Value 0
        (Wait-GuiEvent -Window $w.Id -Timeout 20).Kind | Should -Be 'Expand'
        Wait-Until { $null = Wait-Event -Timeout 0.1; @((Get-GuiSnapshot -Window $w.Id).State.Lazy.shown).Count -eq 3 } 'the action giving the node its children'
        $under = @((Find-GuiNode (Get-GuiSnapshot -Window $w.Id).Tree 'Lazy').Children[0].Children)
        @($under.Properties.text) | Should -Be @('LEAF', 'BRANCH')
        $under[0].Properties.more | Should -Not -BeTrue
        $under[1].Properties.more | Should -BeTrue
        Close-GuiWindow -Window $w.Id
        (Wait-GuiEvent -Window $w.Id -Timeout 20).Kind | Should -Be 'Close'
        Unregister-Event -SourceIdentifier $w.SourceIdentifier
    }

    It 'gives an input''s value to Wait-GuiEvent and to a handler''s action as State gives it' {
        $global:PwrsGuiInputSeen = $null
        $w = Show-GuiWindow -NoWait -Title 'PWRS-GUI test widgets nowait inputs' -Renderer $Renderer {
            NumberBox 1 -Name Count -OnChange { param($e) $global:PwrsGuiInputSeen = "$($e.Value.GetType().Name) $($e.Value) $($e.State.Key.GetType().Name)" }
            TextBox -Name Key -Password
        }
        $null = Wait-GuiWindow 'PWRS-GUI test widgets nowait inputs'
        Invoke-GuiTestEvent -Window $w.Id -Name Key -Event Change -Value 'k'
        Invoke-GuiTestEvent -Window $w.Id -Name Count -Event Change -Value 4
        (Wait-GuiEvent -Window $w.Id -Name Key -Timeout 20).Value | Should -BeOfType [securestring]
        $counted = Wait-GuiEvent -Window $w.Id -Name Count -Timeout 20
        $counted.Value | Should -BeOfType [double]
        $counted.Value | Should -Be 4
        Wait-Until { $null = Wait-Event -Timeout 0.1; $global:PwrsGuiInputSeen -eq 'Double 4 SecureString' } 'the action getting the number and the password'
        Close-GuiWindow -Window $w.Id
        (Wait-GuiEvent -Window $w.Id -Timeout 20).Kind | Should -Be 'Close'
        Unregister-Event -SourceIdentifier $w.SourceIdentifier
        Remove-Variable -Name PwrsGuiInputSeen -Scope Global
    }

    It 'queues a menu item''s click by its shortcut and the window''s own shortcut for Wait-GuiEvent' {
        $w = Show-GuiWindow -NoWait -Title 'PWRS-GUI test widgets nowait menus' -Renderer $Renderer -OnShortcut @{ 'Ctrl+R' = { 'reloaded' } } {
            MenuBar { Menu 'File' { MenuItem 'Open' -Name Open -Shortcut Ctrl+O } }
            Text 'Body'
        }
        $null = Wait-GuiWindow 'PWRS-GUI test widgets nowait menus'
        Invoke-GuiTestEvent -Window $w.Id -Shortcut Ctrl+O
        $opened = Wait-GuiEvent -Window $w.Id -Timeout 20
        $opened.Kind | Should -Be 'Click'
        $opened.Name | Should -Be 'Open'
        Invoke-GuiTestEvent -Window $w.Id -Shortcut ctrl+r
        $own = Wait-GuiEvent -Window $w.Id -Timeout 20
        $own.Kind | Should -Be 'Shortcut'
        $own.Value | Should -Be 'Ctrl+R'
        Close-GuiWindow -Window $w.Id
        (Wait-GuiEvent -Window $w.Id -Timeout 20).Kind | Should -Be 'Close'
        Unregister-Event -SourceIdentifier $w.SourceIdentifier
    }

    It 'queues a watched file''s change for Wait-GuiEvent, one at a time, and runs -OnFileChange with it' {
        $global:PwrsGuiFileSeen = $null
        $log = Join-Path $TestDrive 'nowait.log'
        Set-Content -LiteralPath $log -Value 'first'
        $before = (Get-Item -LiteralPath $log).Length
        $w = Show-GuiWindow -NoWait -Title 'PWRS-GUI test widgets nowait watch' -Renderer $Renderer -WatchPath $log -OnFileChange { param($e) $global:PwrsGuiFileSeen = $e.Kind } { Text 'Body' }
        $null = Wait-GuiWindow 'PWRS-GUI test widgets nowait watch'
        Add-Content -LiteralPath $log -Value 'second'
        $changed = Wait-GuiEvent -Window $w.Id -Timeout 20
        $changed.Kind | Should -Be 'FileChange'
        $changed.Name | Should -Be (Resolve-Path -LiteralPath $log).ProviderPath
        $changed.Value | Should -BeGreaterThan $before
        Wait-Until { $null = Wait-Event -Timeout 0.1; $global:PwrsGuiFileSeen -eq 'FileChange' } 'the action getting the change'
        Add-Content -LiteralPath $log -Value 'third'
        $length = (Get-Item -LiteralPath $log).Length
        Wait-Until { $next = Wait-GuiEvent -Window $w.Id -Name $changed.Name -Timeout 1; $next.Kind -eq 'FileChange' -and $next.Value -eq $length } 'a change at the file''s new length'
        Close-GuiWindow -Window $w.Id
        (Wait-GuiEvent -Window $w.Id -Timeout 20).Kind | Should -Be 'Close'
        Unregister-Event -SourceIdentifier $w.SourceIdentifier
        Remove-Variable -Name PwrsGuiFileSeen -Scope Global
    }
}

Describe 'Show-GuiWindow -Content outside a window' {
    BeforeAll {
        Reset-GuiSession Helper
    }

    It 'defines the short names only while the block runs' {
        { Show-GuiWindow { 'not a widget' } } | Should -Throw -ErrorId 'GuiNotAWidget*'
        Get-Alias -Name Column, Text, Button -ErrorAction Ignore | Should -BeNullOrEmpty
        (Get-Alias -Name group).Definition | Should -Be 'Group-Object'
    }

    It 'refuses -WatchInterval and -OnFileChange without -WatchPath, a folder that is not there, and an interval of no length' {
        { Show-GuiWindow -OnFileChange { 'changed' } { Text 'x' } } | Should -Throw -ErrorId 'GuiBadWatch*'
        { Show-GuiWindow -WatchInterval 1 { Text 'x' } } | Should -Throw -ErrorId 'GuiBadWatch*'
        { Show-GuiWindow -WatchPath (Join-Path $TestDrive 'nowhere\app.log') { Text 'x' } } | Should -Throw -ErrorId 'GuiNoWatch*'
        { Show-GuiWindow -WatchPath (Join-Path $TestDrive 'app.log') -WatchInterval 0 { Text 'x' } } | Should -Throw -ErrorId 'GuiBadWatch*'
        { Show-GuiWindow -WatchPath (Join-Path $TestDrive 'app.log'), (Join-Path $TestDrive 'app.log') { Text 'x' } } | Should -Throw -ErrorId 'GuiBadWatch*'
    }

    It 'refuses two widgets of one name' {
        { Show-GuiWindow { Button 'A' -Name Same; ListBox 'x' -Name same } } | Should -Throw -ErrorId 'GuiNameTwice*'
    }

    It 'refuses a block that writes no widget' {
        { Show-GuiWindow { } } | Should -Throw -ErrorId 'GuiNoWidgets*'
    }

    It 'names the window an update command needs outside a handler' {
        { Update-GuiWidget Status @{ Text = 'x' } } | Should -Throw -ErrorId 'GuiNoWindow*'
        { Set-GuiFocus Status } | Should -Throw -ErrorId 'GuiNoWindow*'
        { Close-GuiWindow } | Should -Throw -ErrorId 'GuiNoWindow*'
    }

    It 'writes a non-terminating GuiNoWindow for a number no open window has, whether or not its renderer runs' {
        Reset-GuiSession Helper
        Close-GuiWindow -Window 4242 -ErrorVariable idle -ErrorAction SilentlyContinue
        @($idle).Count | Should -Be 1
        $idle[0].FullyQualifiedErrorId | Should -BeLike 'GuiNoWindow,*'
        $w = Show-GuiWindow -NoWait -Title 'PWRS-GUI test close unknown' { Text 'x' }
        $null = Wait-GuiWindow 'PWRS-GUI test close unknown'
        Close-GuiWindow -Window 4242 -ErrorVariable running -ErrorAction SilentlyContinue
        $running[0].FullyQualifiedErrorId | Should -BeLike 'GuiNoWindow,*'
        $running[0].Exception.Message | Should -Be 'no window is open with the number 4242'
        Close-GuiWindow -Window $w.Id
        Close-GuiWindow -Window $w.Id -ErrorVariable early -ErrorAction SilentlyContinue
        @($early).Count | Should -Be 0 -Because 'a -NoWait window counts as open until its Close is taken'
        (Wait-GuiEvent -Window $w.Id -Timeout 20).Kind | Should -Be 'Close'
        Close-GuiWindow -Window $w.Id -ErrorVariable late -ErrorAction SilentlyContinue
        $late[0].FullyQualifiedErrorId | Should -BeLike 'GuiNoWindow,*'
        @(4242, 4243) | ForEach-Object { [pscustomobject]@{ Id = $_ } } | Close-GuiWindow -ErrorVariable piped -ErrorAction SilentlyContinue
        @($piped).Count | Should -Be 2 -Because 'a piped list goes on past a number no window has'
    }

    It 'describes widgets with the New-Gui commands as with the short names' {
        $w = New-GuiColumn -Name Outer { New-GuiText 'Name' -Style Mono; New-GuiButton 'OK' -Result Ok -Name Ok }
        $w.PSObject.TypeNames[0] | Should -Be 'PwrsGui.Widget'
        $w.Kind | Should -Be 'Column'
        $w.Name | Should -Be 'Outer'
        $w.Children.Count | Should -Be 2
        $w.Children[0].Properties.Style | Should -Be 'Mono'
        $w.Children[1].Result | Should -Be 'Ok'
    }

    It 'refuses numbers a number field or slider cannot take, a password field of several lines, and a choice it does not have' {
        { New-GuiNumberBox 5 -Minimum 10 -Maximum 1 } | Should -Throw -ErrorId 'GuiBadNumber*'
        { New-GuiSlider 150 } | Should -Throw -ErrorId 'GuiBadNumber*'
        { New-GuiSlider -Step 0 } | Should -Throw -ErrorId 'GuiBadNumber*'
        { New-GuiTextBox -Password -Multiline } | Should -Throw -ErrorId 'GuiBadProperty*'
        { New-GuiComboBox 'a', 'b' -Selected 'z' } | Should -Throw -ErrorId 'GuiBadProperty*'
        (New-GuiNumberBox -Minimum 5).Properties.Value | Should -Be 5
        (New-GuiSlider).Properties.Maximum | Should -Be 100
    }

    It 'refuses a Tab outside a Tabs, anything but tabs in one, and a Split of other than two widgets' {
        { Show-GuiWindow { Tab 'Lost' { Text 'x' } } } | Should -Throw -ErrorId 'GuiTabOutsideTabs*'
        { New-GuiTabs { New-GuiText 'not a tab' } } | Should -Throw -ErrorId 'GuiNotATab*'
        { New-GuiTabs { New-GuiTab 'A' } -Selected 'B' } | Should -Throw -ErrorId 'GuiBadProperty*'
        { New-GuiSplit { New-GuiText 'one' } } | Should -Throw -ErrorId 'GuiNotTwoWidgets*'
        { New-GuiSplit -Position 2 { New-GuiText 'a'; New-GuiText 'b' } } | Should -Throw -ErrorId 'GuiBadNumber*'
        { New-GuiGrid -Columns 0 } | Should -Throw -ErrorId 'GuiBadProperty*'
        { New-GuiScroll -Height -1 } | Should -Throw -ErrorId 'GuiBadSize*'
    }

    It 'refuses a picture of a format it does not take, a file it cannot read, a size of 0, and a fit with no box' {
        $red = [Convert]::FromBase64String('iVBORw0KGgoAAAANSUhEUgAAAAgAAAAECAIAAAA8r+mnAAAAEklEQVR4nGP4z8CAFWEXJUsCAFpeH+EeQoQoAAAAAElFTkSuQmCC')
        $tiff = { New-GuiImage -Bytes ([byte[]](0x49, 0x49, 0x2A, 0, 8, 0, 0, 0)) } | Should -Throw -ErrorId 'GuiBadImage*' -PassThru
        $tiff.Exception.Message | Should -BeLike '*a TIFF image*'
        { New-GuiImage -Path (Join-Path ([IO.Path]::GetTempPath()) "pwrs-gui-no-such-picture-$PID.png") } | Should -Throw -ErrorId 'GuiNoFile*'
        { New-GuiImage -Bytes $red -Width 0 } | Should -Throw -ErrorId 'GuiBadSize*'
        { New-GuiImage -Bytes $red -Width 10 -Fit Cover } | Should -Throw -ErrorId 'GuiBadProperty*'
        $image = New-GuiImage -Bytes $red -Width 10 -Height 5 -Fit Cover -AltText 'red' -Name Pic
        $image.Properties.Fit | Should -Be 'Cover'
        $image.Properties.Bytes.Length | Should -Be $red.Length
    }

    It 'takes a document''s text or the full path of its file, and refuses a file it cannot read' {
        { New-GuiMarkdown -Path (Join-Path ([IO.Path]::GetTempPath()) "pwrs-gui-no-such-document-$PID.md") } | Should -Throw -ErrorId 'GuiNoFile*'
        $file = Join-Path ([IO.Path]::GetTempPath()) "pwrs-gui-test-markdown-path-$PID.md"
        [IO.File]::WriteAllText($file, '# Notes')
        Push-Location -LiteralPath ([IO.Path]::GetTempPath())
        try { $fromFile = New-GuiMarkdown -Path (Split-Path -Leaf $file) -Name Doc } finally { Pop-Location }
        $fromFile.Properties.Path | Should -Be (Resolve-Path -LiteralPath $file).ProviderPath
        (New-GuiMarkdown '' -Name Empty).Properties.Text | Should -Be ''
        Remove-Item -LiteralPath $file
    }

    It 'gives a spacer the size requested, leaves the renderer its own when none is, and refuses one below zero' {
        (New-GuiSpacer 24 -Name Gap).Properties.Size | Should -Be 24
        (New-GuiSpacer).Properties.Size | Should -BeNullOrEmpty
        { New-GuiSpacer -1 } | Should -Throw -ErrorId 'GuiBadSize*'
    }

    It 'refuses a click handler on text that is no link, and a progress outside its range' {
        { New-GuiText 'x' -OnClick { } } | Should -Throw -ErrorId 'GuiBadProperty*'
        (New-GuiText 'x' -Style Link -OnClick { 'clicked' }).Handlers.Click | Should -Not -BeNullOrEmpty
        { New-GuiProgressBar 150 } | Should -Throw -ErrorId 'GuiBadNumber*'
        { New-GuiProgressBar 5 -Maximum 0 } | Should -Throw -ErrorId 'GuiBadNumber*'
        { New-GuiLogView -Height -1 } | Should -Throw -ErrorId 'GuiBadSize*'
    }

    It 'refuses a color not written #rrggbb and Submit on a field of several lines' {
        { New-GuiColorPicker 'red' } | Should -Throw -ErrorId 'GuiBadProperty*'
        (New-GuiColorPicker '#AABBCC').Properties.Color | Should -Be '#aabbcc'
        { New-GuiTextBox -Multiline -OnSubmit { } } | Should -Throw -ErrorId 'GuiBadProperty*'
        (New-GuiComboBox 'a' -Editable -Selected 'anything').Properties.Text | Should -Be 'anything'
    }

    It 'takes a tree''s top nodes from -Node and the pipeline, and refuses -HasChildren and -OnExpand alone, a -Text of neither form, and a hashtable node with no Text' {
        @(('a', 'b' | New-GuiTreeView).Properties.Node) | Should -Be @('a', 'b')
        @((New-GuiTreeView 'a', 'b').Properties.Node) | Should -Be @('a', 'b')
        { New-GuiTreeView 'a' -HasChildren { $true } } | Should -Throw -ErrorId 'GuiBadProperty*'
        { New-GuiTreeView 'a' -OnExpand { 'x' } } | Should -Throw -ErrorId 'GuiBadProperty*'
        { New-GuiTreeView 'a' -Text 5 } | Should -Throw -ErrorId 'GuiBadProperty*'
        $thrown = { Show-GuiWindow { TreeView -Node @{ Children = 'x' } } } | Should -Throw -ErrorId 'GuiBadTreeNode*' -PassThru
        $thrown.Exception.Message | Should -Be "a tree's hashtable node gives its Text, and its keys are Children"
    }

    It 'takes a table''s rows from the pipeline as it takes them as its first parameter' {
        $rows = [pscustomobject]@{ Name = 'a'; Size = 1 }, [pscustomobject]@{ Name = 'b'; Size = 2 }
        @(($rows | New-GuiTable).Properties.Item).Name | Should -Be @('a', 'b')
        @((New-GuiTable $rows).Properties.Item).Name | Should -Be @('a', 'b')
    }

    It 'refuses a property a widget does not take, naming those it does' {
        $text = New-GuiText 'x'
        $text.Properties['Color'] = 'red'
        $thrown = { Show-GuiWindow { $text } } | Should -Throw -ErrorId 'GuiUnknownProperty*' -PassThru
        $thrown.Exception.Message | Should -BeLike '*a Text takes Text, Style; it was given Color*'
    }

    It 'refuses a menu or its item outside a menu, a bar inside another widget, a context menu for no widget, and a shortcut it cannot read' {
        { Show-GuiWindow { MenuItem 'Lost' } } | Should -Throw -ErrorId 'GuiNotInMenu*'
        { Show-GuiWindow { Menu 'Lost' { MenuItem 'x' } } } | Should -Throw -ErrorId 'GuiNotInMenu*'
        { Show-GuiWindow { Text 'x'; Row { MenuBar { Menu 'File' { MenuItem 'x' } } } } } | Should -Throw -ErrorId 'GuiNotAtTop*'
        { Show-GuiWindow { Text 'x'; Row { StatusBar { Text 'y' } } } } | Should -Throw -ErrorId 'GuiNotAtTop*'
        { Show-GuiWindow { Row { MenuBar { Menu 'File' { MenuItem 'x' } } } } } | Should -Throw -ErrorId 'GuiNotAtTop*' -Because 'a block of one widget makes it the root, and its children are not the window''s own'
        { Show-GuiWindow { Row { StatusBar { Text 'y' } } } } | Should -Throw -ErrorId 'GuiNotAtTop*'
        { Show-GuiWindow { Text 'x' -Name Here; ContextMenu -For Elsewhere { MenuItem 'x' } } } | Should -Throw -ErrorId 'GuiNoWidget*'
        { New-GuiMenuItem 'x' -Shortcut 'Ctrl+Meta+S' } | Should -Throw -ErrorId 'GuiBadShortcut*'
        { New-GuiMenuItem 'x' -Shortcut 'F01' } | Should -Throw -ErrorId 'GuiBadShortcut*'
        (New-GuiMenuItem 'x' -Shortcut 'shift+ctrl+f12').Properties.Shortcut | Should -Be 'Ctrl+Shift+F12'
        { Show-GuiWindow -OnShortcut @{ 'Ctrl+S' = 'not a block' } { Text 'x' } } | Should -Throw -ErrorId 'GuiBadShortcut*'
        { Show-GuiWindow -OnShortcut @{ 'Ctrl+Shift+S' = { }; 'Shift+Ctrl+S' = { } } { Text 'x' } } | Should -Throw -ErrorId 'GuiBadShortcut*'
    }

    It 'refuses a series without a name or Y, X and Y of different counts, numbers beside dates, and a key a series does not take' {
        { New-GuiChart -Series @{ Y = @(1) } } | Should -Throw -ErrorId 'GuiBadProperty*'
        { New-GuiChart -Series @{ Name = 'a' } } | Should -Throw -ErrorId 'GuiBadProperty*'
        { New-GuiChart -Series @{ Name = 'a'; X = 1, 2; Y = @(1) } } | Should -Throw -ErrorId 'GuiBadProperty*'
        { New-GuiChart -Series @{ Name = 'a'; X = 1, 2; Y = 1, 2 }, @{ Name = 'b'; X = (Get-Date), (Get-Date); Y = 1, 2 } } | Should -Throw -ErrorId 'GuiBadProperty*'
        { New-GuiChart -Series @{ Name = 'a'; Y = @(1); Color = 'red' } } | Should -Throw -ErrorId 'GuiBadProperty*'
        { New-GuiChart -Series @{ Name = 'a'; Y = @(1); Type = 'Pie' } } | Should -Throw -ErrorId 'GuiBadProperty*'
        { New-GuiChart -Height 0 } | Should -Throw -ErrorId 'GuiBadSize*'
        (New-GuiChart -Series @{ Name = 'a'; Y = 1, 2 } -Type Bar).Properties.Type | Should -Be 'Bar'
    }

    It 'takes one number or date given for X or Y as one point, and counts it against the other' {
        { New-GuiChart -Series @{ Name = 'one'; X = 2; Y = 5 } } | Should -Not -Throw
        { New-GuiChart -Series @{ Name = 'when'; X = [datetime]'2026-10-08T14:30'; Y = 20 } } | Should -Not -Throw
        { New-GuiChart -Series @{ Name = 'a'; X = 1; Y = 3, 5 } } | Should -Throw '*X has 1 value and Y 2*'
    }

    It 'takes a file chooser''s filter as strings or an ordered dictionary, refuses a plain hashtable, and refuses a save of a folder or of several paths' {
        { New-GuiFilePicker -Filter @{ Text = 'txt' } } | Should -Throw -ErrorId 'GuiBadProperty*'
        { New-GuiFilePicker -Filter 'no bar here' } | Should -Throw -ErrorId 'GuiBadProperty*'
        { New-GuiFilePicker -Filter 'Odd|a*b' } | Should -Throw -ErrorId 'GuiBadProperty*'
        (New-GuiFilePicker -Filter ([ordered]@{ 'Text files' = 'txt', '*.md'; 'All' = @('*') })).Kind | Should -Be 'FilePicker'
        (New-GuiFilePicker -Filter ([ordered]@{ 'Markdown' = '.md' })).Kind | Should -Be 'FilePicker' -Because 'one extension given as a string is one, not its characters'
        (New-GuiFilePicker -Filter 'Logs|*.log;.txt').Kind | Should -Be 'FilePicker'
        { New-GuiFilePicker -Save -Folder } | Should -Throw -ErrorId 'GuiBadProperty*'
        { New-GuiFilePicker -Save -Multiple } | Should -Throw -ErrorId 'GuiBadProperty*'
        { New-GuiFilePicker -Folder -Filter 'Logs|*.log' } | Should -Throw -ErrorId 'GuiBadProperty*'
        { New-GuiFilePicker -Path 'a', 'b' } | Should -Throw -ErrorId 'GuiBadProperty*'
        (New-GuiFilePicker -Path 'relative.txt').Properties.Path | Should -Be (Join-Path (Get-Location).ProviderPath 'relative.txt')
    }

    It 'takes a SubEtha ring a widget can read, and refuses a stamped ring, a capacity ring and anything else' {
        Import-Module (Join-Path $env:SUBETHA_MODULE 'SubEtha.psd1') -ErrorAction Stop
        $base = Join-Path ([IO.Path]::GetTempPath()) "pwrs-gui-test-ring-kinds-$PID"
        $null = New-Item -ItemType Directory -Path $base -Force
        $rings = [System.Collections.Generic.List[object]]::new()
        try {
            $rings.Add((New-SubEthaRing -Path (Join-Path $base 'plain') -Capacity 64))
            (New-GuiProgressBar -Ring $rings[0]).Kind | Should -Be 'ProgressBar'
            $rings.Add((New-SubEthaRing -Path (Join-Path $base 'stamped') -Capacity 64 -Stamps Counter))
            { New-GuiLogView -Ring $rings[1] } | Should -Throw '*made with -Stamps*'
            $rings.Add((New-SubEthaCapacityRing -Path (Join-Path $base 'resizable') -Capacity 64))
            { New-GuiChart -Ring $rings[2] } | Should -Throw '*resizes reach only the process*'
            { New-GuiProgressBar -Ring 'not a ring' } | Should -Throw -ErrorId 'GuiBadProperty*'
        } finally {
            foreach ($r in $rings) { $r.Dispose() }
            [GC]::Collect()
            [GC]::WaitForPendingFinalizers()
            Wait-Until { Remove-Item -LiteralPath $base -Recurse -Force -ErrorAction Ignore; -not (Test-Path -LiteralPath $base) } 'the rings'' folder being removed'
        }
    }

    It 'refuses a date outside its chooser''s range and a range that ends before it starts, and -OnKey on a widget that takes no focus' {
        { New-GuiDatePicker ([datetime]'2026-01-01') -Minimum ([datetime]'2026-02-01') } | Should -Throw -ErrorId 'GuiBadDate*'
        { New-GuiDatePicker -Minimum ([datetime]'2026-03-01') -Maximum ([datetime]'2026-02-01') } | Should -Throw -ErrorId 'GuiBadDate*'
        (New-GuiDatePicker ([datetime]'2026-10-08T14:30:59')).Properties.Value | Should -Be ([datetime]'2026-10-08T14:30:59')
        { New-GuiText 'x' -OnKey { } } | Should -Throw -ErrorId 'NamedParameterNotFound*'
        (New-GuiText 'x' -OnDrop { }).Handlers.Keys | Should -Be @('Drop')
    }
}
