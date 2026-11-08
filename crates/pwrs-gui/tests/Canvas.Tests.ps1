# The Canvas widget, in every mode the platform has and in windows of both
# renderers: its items drawn and changed, by the commands, from a SubEtha
# ring and from shared memory, the events its items and the canvas raise,
# its view and the commands that set it, and the keys it takes. A test event
# puts the pointer where a person's would and leaves the view as it is. See
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

Describe 'A canvas in <Mode> mode drawn with <Renderer>' -ForEach $cases {
    BeforeAll {
        Reset-GuiSession $Mode $Renderer

        # The red, green and blue of the texel of a snapshot $Across of the
        # way along it and $Down of the way down.
        function Texel($shot, [double]$Across = 0.5, [double]$Down = 0.5) {
            if (-not $shot) { return @() }
            $at = ([int][math]::Floor(($shot.Height - 1) * $Down) * $shot.Width + [int][math]::Floor(($shot.Width - 1) * $Across)) * 4
            @($shot.Pixels[$at..($at + 2)])
        }
        function Near($rgb, $want) { $rgb.Count -eq 3 -and @(0..2 | Where-Object { [math]::Abs([int]$rgb[$_] - $want[$_]) -gt 8 }).Count -eq 0 }
        function Shot($Window) { Get-GuiSnapshot -Window $Window.Id -Name Board -Pixels -ErrorAction Ignore }

        # The window's next event for which $Wanted is true, passing over any
        # other, as a move the real pointer makes over the window; nothing
        # when none comes within $Seconds.
        function Next($Window, [scriptblock]$Wanted, [int]$Seconds = 20) {
            $deadline = [DateTime]::UtcNow.AddSeconds($Seconds)
            while ([DateTime]::UtcNow -lt $deadline) {
                $e = Wait-GuiEvent -Window $Window.Id -Timeout 1
                if ($e -and ($e | Where-Object $Wanted)) { return $e }
            }
        }

        # A window titled $Title with $Content, once its canvas, Board, has
        # drawn, so a test event has a place to put the pointer on.
        function Open([string]$Title, [scriptblock]$Content) {
            $w = Show-GuiWindow -NoWait -Title $Title -Renderer $Renderer $Content
            $null = Wait-GuiWindow $Title
            Wait-Until { Shot $w } 'the canvas drawn'
            $w
        }
        function Done($Window) {
            Close-GuiWindow -Window $Window.Id
            Next $Window { $_.Kind -eq 'Close' } | Should -Not -BeNullOrEmpty -Because 'the window closes'
        }
    }

    It 'draws its items, a translucent group as one layer and a clipped group through its clip' {
        $w = Open 'PWRS-GUI test canvas draws' {
            Canvas -Name Board -Width 200 -Height 100 @(
                @{ Kind = 'Rect'; X = 0; Y = 0; Width = 200; Height = 100; Fill = 'White' }
                @{ Kind = 'Rect'; Name = 'Left'; X = 0; Y = 0; Width = 60; Height = 100; Fill = 'Red' }
                @{ Kind = 'Group'; Opacity = 0.5; Children = @(
                        @{ Kind = 'Rect'; X = 70; Y = 0; Width = 40; Height = 100; Fill = 'Blue' }
                        @{ Kind = 'Rect'; X = 90; Y = 0; Width = 40; Height = 100; Fill = 'Blue' }
                    )
                }
                @{ Kind = 'Group'; X = 140; Y = 0; Clip = @{ Kind = 'Ellipse'; X = 0; Y = 0; Width = 60; Height = 100 }; Children = @{ Kind = 'Rect'; X = 0; Y = 0; Width = 60; Height = 100; Fill = 'Lime' } }
            )
        }
        Wait-Until { Near (Texel (Shot $w) 0.15) @(255, 0, 0) } 'the left box drawn red'
        $once = Texel (Shot $w) 0.4
        $twice = Texel (Shot $w) 0.5
        Near $once @(128, 128, 255) | Should -BeTrue -Because "half-clear blue over white is drawn where one of the group's boxes lies: $once"
        Near $twice $once | Should -BeTrue -Because "a translucent group fades as one layer, so where both its boxes lie looks as where one does: $twice against $once"
        Near (Texel (Shot $w) 0.85) @(0, 255, 0) | Should -BeTrue -Because 'the clipped group shows inside its ellipse'
        Near (Texel (Shot $w) 0.71 0.04) @(255, 255, 255) | Should -BeTrue -Because 'and nothing of it outside, where the white beneath shows'
        Done $w
    }

    It 'changes an item, adds items to the canvas and to a group, and takes them away by name' {
        $w = Open 'PWRS-GUI test canvas changes' {
            Canvas -Name Board -Width 200 -Height 100 @(
                @{ Kind = 'Rect'; X = 0; Y = 0; Width = 200; Height = 100; Fill = 'White' }
                @{ Kind = 'Rect'; Name = 'Box'; X = 0; Y = 0; Width = 100; Height = 100; Fill = 'Red' }
                @{ Kind = 'Group'; Name = 'Layer'; Children = @{ Kind = 'Rect'; Name = 'Seed'; X = 0; Y = 0; Width = 1; Height = 1; Visible = $false } }
            )
        }
        Wait-Until { Near (Texel (Shot $w) 0.25) @(255, 0, 0) } 'the box drawn red'
        Update-GuiWidget -Window $w.Id -Name Board -Item Box -Property @{ Fill = 'Blue'; X = 100 }
        Wait-Until { Near (Texel (Shot $w) 0.75) @(0, 0, 255) } 'the box moved right and drawn blue'
        Near (Texel (Shot $w) 0.25) @(255, 255, 255) | Should -BeTrue -Because 'the box left the left half'
        Add-GuiItem -Window $w.Id -Name Board -Item @{ Kind = 'Rect'; Name = 'Patch'; X = 0; Y = 0; Width = 50; Height = 100; Fill = 'Lime' } -Parent Layer
        Wait-Until { Near (Texel (Shot $w) 0.1) @(0, 255, 0) } 'the patch added into the group'
        Remove-GuiItem -Window $w.Id -Name Board -Item Patch
        Wait-Until { Near (Texel (Shot $w) 0.1) @(255, 255, 255) } 'the patch taken away'
        { Update-GuiWidget -Window $w.Id -Name Board -Item Nothing -Property @{ X = 1 } } | Should -Throw -ErrorId 'GuiNoItem*'
        { Add-GuiItem -Window $w.Id -Name Board -Item @{ Kind = 'Rect'; Name = 'Box'; X = 0; Y = 0; Width = 1; Height = 1 } } | Should -Throw -ErrorId 'GuiBadItem*'
        Done $w
    }

    It 'raises its items'' events on the topmost named item at the pointer, inside an outline filled or not, and nothing where no item lies' {
        $w = Open 'PWRS-GUI test canvas item events' {
            Canvas -Name Board -Width 200 -Height 100 -OnClick {} -OnDoubleClick {} -OnDragStart {} @(
                @{ Kind = 'Rect'; Name = 'Under'; X = 0; Y = 10; Width = 80; Height = 80; Fill = 'Red' }
                @{ Kind = 'Rect'; Name = 'Over'; X = 60; Y = 10; Width = 80; Height = 80; Fill = 'Blue' }
                @{ Kind = 'Rect'; Name = 'Frame'; X = 150; Y = 20; Width = 40; Height = 60; Stroke = 'Black' }
            )
        }
        Invoke-GuiTestEvent -Window $w.Id -Name Board -Event Click -Value @{ Item = 'Under' }
        $click = Next $w { $_.Kind -eq 'Click' }
        ($click.Value.Item, $click.Value.Button) | Should -Be @('Under', 'Left')
        [math]::Abs($click.Value.SceneX - 40) + [math]::Abs($click.Value.SceneY - 50) | Should -BeLessThan 1.5 -Because "the pointer goes to the middle of where Under was drawn: $($click.Value.SceneX), $($click.Value.SceneY)"
        Invoke-GuiTestEvent -Window $w.Id -Name Board -Event Click -Value @{ U = 0.35; V = 0.5 }
        (Next $w { $_.Kind -eq 'Click' }).Value.Item | Should -Be 'Over' -Because 'where both lie the later one is on top'
        Invoke-GuiTestEvent -Window $w.Id -Name Board -Event Click -Value @{ U = 0.85; V = 0.5 }
        (Next $w { $_.Kind -eq 'Click' }).Value.Item | Should -Be 'Frame' -Because 'an outlined box takes events inside it, filled or not'
        Invoke-GuiTestEvent -Window $w.Id -Name Board -Event Click -Value @{ U = 0.5; V = 0.97 } -PassThru | Should -BeNullOrEmpty
        Next $w { $_.Kind -eq 'Click' } 2 | Should -BeNullOrEmpty -Because 'an item''s event where no named item lies raises nothing'
        Invoke-GuiTestEvent -Window $w.Id -Name Board -Event DragStart -Value @{ Item = 'Over' }
        $start = Next $w { $_.Kind -eq 'DragStart' }
        ($start.Value.Item, $start.Value.Button) | Should -Be @('Over', 'Left')
        Invoke-GuiTestEvent -Window $w.Id -Name Board -Event DoubleClick -Value @{ Item = 'Frame'; Shift = $true }
        $double = Next $w { $_.Kind -eq 'DoubleClick' }
        ($double.Value.Item, $double.Value.Shift) | Should -Be @('Frame', $true)
        Invoke-GuiTestEvent -Window $w.Id -Name Board -Event PointerDown -Value @{ U = 0.5; V = 0.5 } -ErrorVariable refused -ErrorAction SilentlyContinue
        $refused.Count | Should -Be 1
        $refused[0].FullyQualifiedErrorId | Should -BeLike 'GuiRefused,*'
        "$($refused[0])" | Should -BeLike '*-OnPointerDown*'
        Invoke-GuiTestEvent -Window $w.Id -Name Board -Event Click -Value @{ Item = 'Nothing' } -ErrorVariable missing -ErrorAction SilentlyContinue
        "$($missing[0])" | Should -BeLike '*drew no item named Nothing*'
        Done $w
    }

    It 'raises its pointer events naming the topmost item there or none, with where in pixels, fractions and the items'' coordinates' {
        $w = Open 'PWRS-GUI test canvas pointer' {
            Canvas -Name Board -Width 200 -Height 100 -OnPointerDown {} -OnPointerMove {} -OnWheel {} @{ Kind = 'Ellipse'; Name = 'Dot'; X = 20; Y = 20; Width = 60; Height = 60; Fill = 'Navy' }
        }
        Invoke-GuiTestEvent -Window $w.Id -Name Board -Event PointerDown -Value @{ U = 0.25; V = 0.5; Button = 'Right' }
        $down = Next $w { $_.Kind -eq 'PointerDown' }
        ($down.Value.Item, $down.Value.Button, $down.Value.U, $down.Value.V) | Should -Be @('Dot', 'Right', 0.25, 0.5)
        $down.Value.PixelsPerPoint | Should -BeGreaterThan 0
        [math]::Abs($down.Value.X - 50 * $down.Value.PixelsPerPoint) | Should -BeLessThan 1 -Because 'a quarter of 200 points across, in physical pixels'
        [math]::Abs($down.Value.SceneX - 50) + [math]::Abs($down.Value.SceneY - 50) | Should -BeLessThan 1 -Because 'without a view the items'' coordinates are the canvas''s points'
        Invoke-GuiTestEvent -Window $w.Id -Name Board -Event PointerMove -Value @{ U = 0.75; V = 0.5 }
        $move = Next $w { $_.Kind -eq 'PointerMove' -and $_.Value.U -eq 0.75 }
        $move.Value.Item | Should -BeNullOrEmpty -Because 'no named item lies there'
        $move.Value.Button | Should -BeNullOrEmpty
        Invoke-GuiTestEvent -Window $w.Id -Name Board -Event Wheel -Value @{ Item = 'Dot'; Notches = -2 }
        $turn = Next $w { $_.Kind -eq 'Wheel' }
        ($turn.Value.Item, $turn.Value.Notches) | Should -Be @('Dot', -2)
        Done $w
    }

    It 'lays a Text out in a row for each line of its text, taking events across every row' {
        $w = Open 'PWRS-GUI test canvas text rows' {
            Canvas -Name Board -Width 200 -Height 100 -OnPointerDown {} @(
                @{ Kind = 'Text'; Name = 'Two'; X = 10; Y = 10; Text = "Upper`nLower"; Size = 20 }
                @{ Kind = 'Text'; Name = 'One'; X = 110; Y = 10; Text = 'Upper'; Size = 20 }
            )
        }
        Invoke-GuiTestEvent -Window $w.Id -Name Board -Event PointerDown -Value @{ U = 0.1; V = 0.5 }
        (Next $w { $_.Kind -eq 'PointerDown' }).Value.Item | Should -Be 'Two' -Because 'the second line''s row lies 40 to 60 points down, inside the Text''s box'
        Invoke-GuiTestEvent -Window $w.Id -Name Board -Event PointerDown -Value @{ U = 0.6; V = 0.5 }
        (Next $w { $_.Kind -eq 'PointerDown' }).Value.Item | Should -BeNullOrEmpty -Because 'a Text of one line takes one row'
        Done $w
    }

    It 'takes its view from the commands, which places its items, their hits and the events'' scene points, and lets it go' {
        $w = Open 'PWRS-GUI test canvas view' {
            Canvas -Name Board -Width 200 -Height 100 -OnClick {} -OnPointerDown {} @(
                @{ Kind = 'Rect'; X = 0; Y = 0; Width = 200; Height = 100; Fill = 'White' }
                @{ Kind = 'Rect'; Name = 'Box'; X = 90; Y = 40; Width = 20; Height = 20; Fill = 'Red' }
            )
        }
        Get-GuiCanvasView Board -Window $w.Id | Should -BeNullOrEmpty -Because 'a canvas has no view until one is set'
        Near (Texel (Shot $w) 0.35) @(255, 255, 255) | Should -BeTrue -Because 'the box lies at the middle'
        # The view commands take the window from the pipeline as well as
        # from -Window.
        $w | Set-GuiCanvasView Board -Center 100, 50 -Zoom 4
        $view = $w | Get-GuiCanvasView Board
        ($view.Center[0], $view.Center[1], $view.Zoom, "$($view.Controls)", $view.Moving) | Should -Be @(100, 50, 4, 'Map', $false)
        Wait-Until { Near (Texel (Shot $w) 0.35) @(255, 0, 0) } 'the box, four times as near, reaching 35 % across'
        Invoke-GuiTestEvent -Window $w.Id -Name Board -Event Click -Value @{ U = 0.6; V = 0.5 }
        $click = Next $w { $_.Kind -eq 'Click' }
        $click.Value.Item | Should -Be 'Box' -Because 'hits follow the view'
        [math]::Abs($click.Value.SceneX - 105) | Should -BeLessThan 0.5 -Because "120 points across is 105 in the items' coordinates at zoom 4 about 100: $($click.Value.SceneX)"
        Invoke-GuiTestEvent -Window $w.Id -Name Board -Event PointerDown -Value @{ U = 0.75; V = 0.5 }
        $down = Next $w { $_.Kind -eq 'PointerDown' }
        $down.Value.Item | Should -BeNullOrEmpty
        [math]::Abs($down.Value.SceneX - 112.5) | Should -BeLessThan 0.5
        (Get-GuiCanvasView Board -Window $w.Id).Zoom | Should -Be 4 -Because 'a test event leaves the view'
        Set-GuiCanvasView Board -Window $w.Id -FitItem box
        [math]::Abs((Get-GuiCanvasView Board -Window $w.Id).Zoom - 5) | Should -BeLessThan 0.01 -Because 'the box, 20 points high, fits the canvas''s 100'
        Set-GuiCanvasView Board -Window $w.Id -Fit
        $all = Get-GuiCanvasView Board -Window $w.Id
        [math]::Abs($all.Zoom - 1) + [math]::Abs($all.Center[0] - 100) + [math]::Abs($all.Center[1] - 50) | Should -BeLessThan 0.01 -Because 'every item together is the white box, the canvas''s own size'
        $moving = Set-GuiCanvasView Board -Window $w.Id -Zoom 2 -Duration 1 -PassThru
        $moving.Moving | Should -BeTrue -Because 'the view eases to the change over a second'
        Wait-Until { -not (Get-GuiCanvasView Board -Window $w.Id).Moving } 'the eased change ending' 10
        (Get-GuiCanvasView Board -Window $w.Id).Zoom | Should -Be 2
        Set-GuiCanvasView Board -Window $w.Id -Controls Design
        $design = Get-GuiCanvasView Board -Window $w.Id
        "$($design.Controls)" | Should -Be 'Design'
        $design.Bindings['Shift+Wheel'] | Should -Be 'ScrollAcross'
        $design.Zoom | Should -Be 2 -Because 'switching the controls leaves the view where it is'
        $w | Remove-GuiCanvasView Board -Confirm:$false
        Get-GuiCanvasView Board -Window $w.Id | Should -BeNullOrEmpty
        # At zoom 2 the box covers 40 to 60 % across and in its own
        # coordinates 45 to 55 %, so 42 % turns white once the frame drawn
        # without the view is up, which a test event acts on.
        Wait-Until { Near (Texel (Shot $w) 0.42) @(255, 255, 255) } 'the items drawn in their own coordinates again'
        Invoke-GuiTestEvent -Window $w.Id -Name Board -Event PointerDown -Value @{ U = 0.75; V = 0.5 }
        [math]::Abs((Next $w { $_.Kind -eq 'PointerDown' }).Value.SceneX - 150) | Should -BeLessThan 0.5
        Done $w
    }

    It 'raises the keys its view''s controls leave, and refuses a test key they take' {
        $w = Open 'PWRS-GUI test canvas keys' {
            Canvas -Name Board -Width 100 -Height 60 -OnKey {} @{ Kind = 'Rect'; X = 0; Y = 0; Width = 10; Height = 10 }
        }
        Invoke-GuiTestEvent -Window $w.Id -Name Board -Event Key -Value 'ArrowLeft'
        (Next $w { $_.Kind -eq 'Key' }).Value | Should -Be 'ArrowLeft' -Because 'without a view every key is the canvas''s'
        Set-GuiCanvasView Board -Window $w.Id -Controls Map
        Invoke-GuiTestEvent -Window $w.Id -Name Board -Event Key -Value 'ArrowLeft' -ErrorVariable refused -ErrorAction SilentlyContinue
        $refused.Count | Should -Be 1
        $refused[0].FullyQualifiedErrorId | Should -BeLike 'GuiRefused,*'
        "$($refused[0])" | Should -BeLike '*view takes ArrowLeft itself*'
        Invoke-GuiTestEvent -Window $w.Id -Name Board -Event Key -Value 'Delete'
        (Next $w { $_.Kind -eq 'Key' }).Value | Should -Be 'Delete' -Because 'Map takes no Delete'
        Set-GuiCanvasView Board -Window $w.Id -Controls None
        Invoke-GuiTestEvent -Window $w.Id -Name Board -Event Key -Value 'ArrowLeft'
        (Next $w { $_.Kind -eq 'Key' }).Value | Should -Be 'ArrowLeft' -Because 'controls of None take no key'
        Done $w
    }

    It 'changes its items from a SubEtha ring another runspace writes, an Image''s picture by its file''s full path or its bytes in base64, and refuses an item with a mistake whole' {
        Import-Module (Join-Path $env:SUBETHA_MODULE 'SubEtha.psd1') -ErrorAction Stop
        $base = Join-Path ([IO.Path]::GetTempPath()) "pwrs-gui-test-canvas-ring-$PID-$Mode-$Renderer"
        $null = New-Item -ItemType Directory -Path $base -Force
        $red = 'iVBORw0KGgoAAAANSUhEUgAAAAgAAAAECAIAAAA8r+mnAAAAEklEQVR4nGP4z8CAFWEXJUsCAFpeH+EeQoQoAAAAAElFTkSuQmCC'
        $green = 'iVBORw0KGgoAAAANSUhEUgAAAAgAAAAECAIAAAA8r+mnAAAAEElEQVR4nGNg+M+AHVFPAgA6fh/hYeiK3wAAAABJRU5ErkJggg=='
        $file = Join-Path $base 'red.png'
        [IO.File]::WriteAllBytes($file, [Convert]::FromBase64String($red))
        $at = Join-Path $base 'feed'
        $feed = New-SubEthaRing -Path $at -Capacity 64
        try {
            $job = Start-Background @"
Import-Module (Join-Path '$env:SUBETHA_MODULE' 'SubEtha.psd1')
`$ring = New-SubEthaRing -Path '$at' -Capacity 64
try {
    Show-GuiWindow -Title 'PWRS-GUI test canvas ring' {
        Canvas -Name Board -Width 200 -Height 100 -Ring `$ring @(
            @{ Kind = 'Rect'; Name = 'Box'; X = 0; Y = 0; Width = 100; Height = 100; Fill = 'Red' }
            @{ Kind = 'Image'; Name = 'Pic'; X = 100; Y = 0; Width = 100; Height = 100; Bytes = [Convert]::FromBase64String('$green') }
        )
        Button 'OK' -Name Ok -Result Ok
    }
} finally {
    `$ring.Dispose()
}
"@
            $w = Wait-GuiWindow 'PWRS-GUI test canvas ring' $job
            Wait-Until { Near (Texel (Shot $w) 0.75) @(0, 255, 0) } 'the green picture drawn'
            $p = $feed.RegisterProducer()
            $feed.SendFrame($p, '{"item": "box", "fill": "Blue"}') | Should -BeTrue
            Wait-Until { Near (Texel (Shot $w) 0.25) @(0, 0, 255) } 'the ring turning the box blue, its keys and the item''s name in any case'
            $feed.SendFrame($p, (ConvertTo-Json -Compress -InputObject @{ Item = 'Pic'; Path = $file })) | Should -BeTrue
            Wait-Until { Near (Texel (Shot $w) 0.75) @(255, 0, 0) } 'the picture read from the full path of its file'
            $feed.SendFrame($p, (ConvertTo-Json -Compress -InputObject @(@{ Item = 'Pic'; Bytes = $green }, @{ Item = 'Box'; Fill = 'White' }))) | Should -BeTrue
            Wait-Until { (Near (Texel (Shot $w) 0.75) @(0, 255, 0)) -and (Near (Texel (Shot $w) 0.25) @(255, 255, 255)) } 'one item of the ring changing two of the canvas''s, a picture from its bytes in base64'
            $feed.SendFrame($p, '[{"Item": "Box", "Fill": "Black"}, {"Item": "Box", "Fill": "Plaid"}]') | Should -BeTrue
            Wait-Until { @($job.PS.Streams.Warning | Where-Object { $_.Message -like '*took no item from its ring*' }).Count -ge 1 } 'the item with a mistake being reported'
            Near (Texel (Shot $w) 0.25) @(255, 255, 255) | Should -BeTrue -Because 'an item with a mistake makes none of its changes, the one before the mistake included'
            Invoke-GuiTestEvent -Window $w.Id -Name Ok -Event Click
            $done = Complete-Background $job
            $done.Errors | Should -BeNullOrEmpty
            $done.Output[-1].Result | Should -Be 'Ok'
            @($done.Warnings | Where-Object { $_ -like '*Canvas Board took no item from its ring: Rect Box: Fill*' }).Count | Should -Be 1
        } finally {
            $feed.Dispose()
            [GC]::Collect()
            [GC]::WaitForPendingFinalizers()
            Wait-Until { Remove-Item -LiteralPath $base -Recurse -Force -ErrorAction Ignore; -not (Test-Path -LiteralPath $base) } 'the ring''s folder being removed once the window let go of it'
        }
    }

    It 'sets its items'' numbers from a SubEtha slab another process writes, row by row, woken by the notifier set it names, the slab opened with the layout it was made with' {
        Import-Module (Join-Path $env:SUBETHA_MODULE 'SubEtha.psd1') -ErrorAction Stop
        $base = Join-Path ([IO.Path]::GetTempPath()) "pwrs-gui-test-canvas-slab-$PID-$Mode-$Renderer"
        $null = New-Item -ItemType Directory -Path $base -Force
        $slab = New-SubEthaSlab -Path (Join-Path $base 'rows') -Capacity 2 -ElementSize 16 -Alignment 16 -Tag 7
        $set = New-SubEthaNotifierSet -Path (Join-Path $base 'notify')
        $row = { param([double]$x, [double]$width) [byte[]]([BitConverter]::GetBytes($x) + [BitConverter]::GetBytes($width)) }
        try {
            { New-GuiCanvas -SharedMemory @{ Source = $slab; Type = 'Float32'; Item = '*'; Property = 'X', 'Width'; Notifier = $set } } | Should -Throw '*each slot of Source has 16 bytes, where 2 values of Float32*'
            { New-GuiCanvas -SharedMemory @{ Source = $slab; Type = 'Float64'; Item = '*'; Property = 'X', 'Width' } } | Should -Throw '*names no Notifier and no Interval*'
            { New-GuiCanvas -SharedMemory @{ Source = $slab; Type = 'Float64'; Item = '*'; Property = 'X', 'Fill'; Notifier = $set } } | Should -Throw '*Property names Fill, which takes no single number*'
            { New-GuiCanvas -SharedMemory @{ Source = $slab; Item = '*'; Property = 'X', 'Width'; Notifier = $set } } | Should -Throw '*names no Type*'
            $w = Open 'PWRS-GUI test canvas slab' {
                Canvas -Name Board -Width 200 -Height 100 -SharedMemory @{ Source = $slab; Type = 'Float64'; Item = 'Left', 'Right'; Property = 'X', 'Width'; Notifier = $set } @(
                    @{ Kind = 'Rect'; X = 0; Y = 0; Width = 200; Height = 100; Fill = 'White' }
                    @{ Kind = 'Rect'; Name = 'Left'; X = 0; Y = 0; Width = 50; Height = 100; Fill = 'Red' }
                    @{ Kind = 'Rect'; Name = 'Right'; X = 150; Y = 0; Width = 50; Height = 100; Fill = 'Blue' }
                )
            }
            Wait-Until { Near (Texel (Shot $w) 0.1) @(255, 0, 0) } 'the left box drawn red'
            $shared = (Get-GuiSnapshot -Window $w.Id).Tree.Properties.shared
            "$($shared.alignment) $($shared.tag)" | Should -Be '16 7' -Because 'the canvas opens the slab with the Alignment and Tag its object has'
            $slab.Set(0, (& $row 50 100))
            $null = $set.Signal()
            Wait-Until { Near (Texel (Shot $w) 0.6) @(255, 0, 0) } 'row 0 moving Left to X 50, 100 wide'
            Near (Texel (Shot $w) 0.1) @(255, 255, 255) | Should -BeTrue -Because 'Left left the left edge'
            $slab.Set(1, (& $row 0 40))
            $null = $set.Signal()
            Wait-Until { Near (Texel (Shot $w) 0.1) @(0, 0, 255) } 'row 1 moving Right to X 0, 40 wide'
            Done $w
        } finally {
            $slab.Dispose()
            [GC]::Collect()
            [GC]::WaitForPendingFinalizers()
            Wait-Until { Remove-Item -LiteralPath $base -Recurse -Force -ErrorAction Ignore; -not (Test-Path -LiteralPath $base) } 'the slab''s folder being removed once the window let go of it'
        }
    }

    It 'opens a SubEtha slab whose object has no Alignment or Tag with alignment 1 and tag 0' {
        Import-Module (Join-Path $env:SUBETHA_MODULE 'SubEtha.psd1') -ErrorAction Stop
        $base = Join-Path ([IO.Path]::GetTempPath()) "pwrs-gui-test-canvas-layout-$PID-$Mode-$Renderer"
        $null = New-Item -ItemType Directory -Path $base -Force
        $path = Join-Path $base 'rows'
        $slab = New-SubEthaSlab -Path $path -Capacity 1 -ElementSize 16
        $slab.Set(0, [byte[]]([BitConverter]::GetBytes([double]50) + [BitConverter]::GetBytes([double]60)))
        # The child makes an object of the shape a SubEtha module writes whose
        # slabs have no Alignment or Tag, in a process with no other type of
        # that name.
        $child = @"
Import-Module '$script:GuiModule'
Set-GuiOption -Mode $Mode
Add-Type -TypeDefinition 'namespace SubEtha { public class Slab { public string Path { get; set; } public ulong Capacity { get; set; } public ulong ElementSize { get; set; } public bool Writable { get; set; } } }'
`$old = New-Object SubEtha.Slab -Property @{ Path = '$path'; Capacity = 1; ElementSize = 16 }
`$w = Show-GuiWindow -NoWait -Renderer $Renderer -Title 'PWRS-GUI test canvas layout' { Canvas -Name Board -Width 100 -Height 100 -SharedMemory @{ Source = `$old; Type = 'Float64'; Item = 'Box'; Property = 'X', 'Y'; Interval = 0.05 } @{ Kind = 'Rect'; Name = 'Box'; X = 0; Y = 0; Width = 10; Height = 10 } }
`$deadline = [DateTime]::UtcNow.AddSeconds(30)
do { `$tree = (Get-GuiSnapshot -Window `$w.Id).Tree; `$box = @(`$tree.Properties.items)[0] } until (`$box.x -eq 50 -or [DateTime]::UtcNow -gt `$deadline)
'LAYOUT ' + `$tree.Properties.shared.alignment + ' ' + `$tree.Properties.shared.tag
'ROW ' + `$box.x + ' ' + `$box.y
Close-GuiWindow -Window `$w.Id
"@
        try {
            $read = @(& (Get-Process -Id $PID).Path -NoProfile -NonInteractive -Command $child)
            $text = $read -join "`n"
            ($read | Where-Object { $_ -like 'LAYOUT *' }) | Should -Be 'LAYOUT 1 0' -Because $text
            ($read | Where-Object { $_ -like 'ROW *' }) | Should -Be 'ROW 50 60' -Because "the slab, made with New-SubEthaSlab's own layout, is read: $text"
        } finally {
            $slab.Dispose()
            [GC]::Collect()
            [GC]::WaitForPendingFinalizers()
            Wait-Until { Remove-Item -LiteralPath $base -Recurse -Force -ErrorAction Ignore; -not (Test-Path -LiteralPath $base) } 'the slab''s folder being removed once the child let go of it'
        }
    }

    It 'sets its items'' numbers from a file another process writes under a seqlock counter, read every interval, and refuses a file too short' {
        $base = Join-Path ([IO.Path]::GetTempPath()) "pwrs-gui-test-canvas-file-$PID-$Mode-$Renderer"
        $null = New-Item -ItemType Directory -Path $base -Force
        $file = Join-Path $base 'rows.bin'
        [IO.File]::WriteAllBytes($file, [byte[]]::new(8 + 16))
        $values = { param([double[]]$v) [byte[]]($v | ForEach-Object { [BitConverter]::GetBytes($_) }) }
        $write = {
            param([uint64]$counter, [byte[]]$bytes)
            $fs = [IO.File]::Open($file, 'Open', 'ReadWrite', 'ReadWrite')
            try {
                $fs.Position = 8
                $fs.Write($bytes, 0, $bytes.Length)
                $fs.Position = 0
                $fs.Write([BitConverter]::GetBytes($counter), 0, 8)
            } finally {
                $fs.Dispose()
            }
        }
        try {
            $source = @{ Path = $file; Offset = 8; Length = 2; Sequence = 0; Type = 'Float64'; Item = 'Box'; Property = 'X', 'Width'; Interval = 0.05 }
            $with = { param([hashtable]$changes) $s = $source.Clone(); foreach ($k in $changes.Keys) { $s[$k] = $changes[$k] }; $s }
            { New-GuiCanvas -SharedMemory (& $with @{ Length = 4 }) } | Should -Throw '*shorter than its values and counter, which end at byte 40*'
            { New-GuiCanvas -SharedMemory (& $with @{ Sequence = 8 }) } | Should -Throw '*among the values*'
            { New-GuiCanvas -SharedMemory (& $with @{ Shape = 2, 1 }) } | Should -Throw '*Shape is 2,1*'
            $w = Open 'PWRS-GUI test canvas file' {
                Canvas -Name Board -Width 200 -Height 100 -SharedMemory $source @(
                    @{ Kind = 'Rect'; X = 0; Y = 0; Width = 200; Height = 100; Fill = 'White' }
                    @{ Kind = 'Rect'; Name = 'Box'; X = 0; Y = 0; Width = 50; Height = 100; Fill = 'Red' }
                )
            }
            Wait-Until { Near (Texel (Shot $w) 0.1) @(255, 0, 0) } 'the box drawn red'
            & $write 3 (& $values 150, 50)
            Start-Sleep -Milliseconds 300
            Near (Texel (Shot $w) 0.1) @(255, 0, 0) | Should -BeTrue -Because 'an odd counter is a write under way, read at none of the intervals'
            & $write 4 (& $values 150, 50)
            Wait-Until { Near (Texel (Shot $w) 0.85) @(255, 0, 0) } 'the even counter moving the box to X 150'
            Near (Texel (Shot $w) 0.1) @(255, 255, 255) | Should -BeTrue
            Done $w
        } finally {
            [GC]::Collect()
            [GC]::WaitForPendingFinalizers()
            Wait-Until { Remove-Item -LiteralPath $base -Recurse -Force -ErrorAction Ignore; -not (Test-Path -LiteralPath $base) } 'the file''s folder being removed once the window let go of it'
        }
    }
}
