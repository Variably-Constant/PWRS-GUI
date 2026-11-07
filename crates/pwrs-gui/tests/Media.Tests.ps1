# The media player, in each renderer and mode the platform has. Each
# renderer plays the clips in media/ that its player claims to their end,
# with their picture and their sound, and raises the designed error for its
# player's most common gap: Media Foundation and WebView2 on Windows,
# AVFoundation and WKWebView on macOS, GStreamer and WebKitGTK elsewhere.
# Then the commands that drive a player. See Common.ps1 for the deadlines.

BeforeDiscovery {
    $windows = $IsWindows -or $PSVersionTable.PSEdition -eq 'Desktop'
    $modes = if ($windows) { 'Helper', 'InProcess' } else { 'Helper' }
    $cases = foreach ($m in $modes) { foreach ($r in 'Egui', 'Html') { @{ Mode = $m; Renderer = $r } } }

    # The clips each player claims, and its most common gap. On Windows the
    # HEVC clip plays where the HEVC Video Extensions are installed and is
    # the gap where they are not; AIFF, which no Windows player reads, is
    # the gap on every Windows host. On Linux and the BSDs both renderers'
    # gap is a ProRes clip, which only GStreamer's libav plugin decodes;
    # H.264 is in no list there, since OpenH264 and faad play it where
    # installed.
    $hevc = $windows -and [bool](Get-ChildItem -Path 'HKLM:\SOFTWARE\Classes\Local Settings\Software\Microsoft\Windows\CurrentVersion\AppModel\PackageRepository\Packages' -ErrorAction SilentlyContinue | Where-Object PSChildName -like 'Microsoft.HEVCVideoExtension*')
    $plan = if ($windows) {
        @{
            Egui = @{
                Plays = @('h264-aac.mp4', 'h264-aac.mov', 'h264-aac.mkv', 'h264-aac.ts', 'mpeg4-aac.3gp', 'wmv2-wmav2.wmv', 'tone.mp3', 'tone.m4a', 'tone.aac', 'tone.wav', 'tone.flac', 'tone.wma', 'tone.amr') + @(if ($hevc) { 'hevc-aac.mp4' })
                Gaps = @('tone.aiff') + @(if (-not $hevc) { 'hevc-aac.mp4' })
            }
            Html = @{
                Plays = @('h264-aac.mp4', 'h264-aac.mov', 'h264-aac.mkv', 'vp8-vorbis.webm', 'vp9-opus.webm', 'av1-opus.webm', 'tone.mp3', 'tone.m4a', 'tone.aac', 'tone.wav', 'tone.flac', 'tone.ogg', 'tone.opus')
                Gaps = @('tone.aiff')
            }
        }
    } elseif ($IsMacOS) {
        @{
            Egui = @{ Plays = @('h264-aac.mp4', 'h264-aac.mov', 'hevc-aac.mp4', 'mpeg4-aac.3gp', 'tone.m4a', 'tone.mp3', 'tone.wav', 'tone.aiff', 'tone.caf', 'tone.aac', 'tone.flac', 'tone.ac3'); Gaps = @('vp9-opus.webm') }
            Html = @{ Plays = @('h264-aac.mp4', 'h264-aac.mov', 'hevc-aac.mp4', 'tone.m4a', 'tone.mp3', 'tone.wav', 'tone.aac'); Gaps = @('theora-vorbis.ogv') }
        }
    } else {
        @{
            Egui = @{ Plays = @('vp8-vorbis.webm', 'vp9-opus.webm', 'theora-vorbis.ogv', 'mjpeg-pcm.avi', 'tone.wav', 'tone.flac', 'tone.ogg', 'tone.opus', 'tone.mp3'); Gaps = @('prores.mov') }
            Html = @{ Plays = @('vp8-vorbis.webm', 'vp9-opus.webm', 'theora-vorbis.ogv', 'tone.wav', 'tone.flac', 'tone.ogg', 'tone.opus', 'tone.mp3'); Gaps = @('prores.mov') }
        }
    }
    # The clips whose own text track each renderer's player reports, and the
    # same clip without one, whose picture is the same: Media Foundation
    # reports a SubRip track in Matroska, where the VP9 Video Extensions
    # play it; GStreamer and WebKitGTK report it too; WebKit reports MP4's
    # timed text, and AVFoundation's legible output does in egui windows.
    # WebView2 reports none.
    $vp9 = $windows -and [bool](Get-ChildItem -Path 'HKLM:\SOFTWARE\Classes\Local Settings\Software\Microsoft\Windows\CurrentVersion\AppModel\PackageRepository\Packages' -ErrorAction SilentlyContinue | Where-Object PSChildName -like 'Microsoft.VP9VideoExtensions*')
    $inBand = if ($windows) {
        @{ Egui = @(if ($vp9) { 'vp9-opus-subs.mkv' }); Html = @() }
    } elseif ($IsMacOS) {
        @{ Egui = @('h264-aac-subs.mp4'); Html = @('h264-aac-subs.mp4') }
    } else {
        @{ Egui = @('vp9-opus-subs.mkv'); Html = @('vp9-opus-subs.mkv') }
    }
    $plain = @{ 'vp9-opus-subs.mkv' = 'vp9-opus.webm'; 'h264-aac-subs.mp4' = 'h264-aac.mp4' }
    foreach ($c in $cases) {
        $c.Plays = @($plan[$c.Renderer].Plays | ForEach-Object { @{ Clip = $_ } })
        $c.Gaps = @($plan[$c.Renderer].Gaps | ForEach-Object { @{ Clip = $_ } })
        # Whether the renderer measures the sound a player sends the
        # system: egui windows through each backend's own tap, on every
        # platform here, and HTML windows through Web Audio on WebView2 and
        # WebKit. On WebKitGTK an HTML window measures none, for the reason
        # `Unmeasured` gives, so each player's sound reads null there.
        $c.Measured = $c.Renderer -eq 'Egui' -or $windows -or $IsMacOS
        $c.Unmeasured = if (-not $c.Measured) { 'an HTML window on WebKitGTK measures no player''s sound, since a player routed through Web Audio there plays ahead of real time' }
        $c.InBand = @($inBand[$c.Renderer] | ForEach-Object { @{ Clip = $_; Plain = $plain[$_] } })
    }
}

BeforeAll {
    . (Join-Path $PSScriptRoot 'Common.ps1')
    $script:Clips = Join-Path $PSScriptRoot 'media'
    $script:Watchdog = Start-Watchdog 2400

    # The window's events until one of the kinds `$Kinds` names, which is
    # returned; the test fails when none comes in `$Seconds`.
    function Wait-Media($Window, [string[]] $Kinds, [int] $Seconds = 20) {
        $deadline = [DateTime]::UtcNow.AddSeconds($Seconds)
        $seen = [System.Collections.Generic.List[string]]::new()
        while ([DateTime]::UtcNow -lt $deadline) {
            $e = Wait-GuiEvent -Window $Window.Id -Timeout 1
            if (-not $e) { continue }
            $seen.Add([string]$e.Kind)
            if ($Kinds -contains [string]$e.Kind) { return $e }
        }
        throw "no $($Kinds -join ' or ') came within $Seconds s; the window raised $($seen -join ', ')"
    }

    # How many of a snapshot's pixels are not black.
    function Get-Lit($Shot) {
        $lit = 0
        for ($i = 0; $i -lt $Shot.Pixels.Length; $i += 4) { if ($Shot.Pixels[$i] -gt 16 -or $Shot.Pixels[$i + 1] -gt 16 -or $Shot.Pixels[$i + 2] -gt 16) { $lit++ } }
        $lit
    }

    # How many pixels in the lower half of two snapshots of one widget
    # differ by more than a sixth of the range in a channel.
    function Get-Changed($Before, $After) {
        $changed = 0
        $end = [Math]::Min($Before.Pixels.Length, $After.Pixels.Length)
        for ($i = [int]($After.Height / 2) * $After.Width * 4; $i -lt $end; $i += 4) {
            for ($c = 0; $c -lt 3; $c++) {
                if ([Math]::Abs([int]$Before.Pixels[$i + $c] - [int]$After.Pixels[$i + $c]) -gt 42) { $changed++; break }
            }
        }
        $changed
    }

    # Seeks player P of window `$Window` to `$Seconds` and gives a snapshot
    # of it once its picture there is drawn. A seek's end comes before its
    # picture, and a picture from before the seek is as lit as the new one,
    # so the time goes first. A renderer may draw no picture for longer, so
    # the snapshot then waits for the test picture to fill the player.
    function Get-Frame($Window, [double] $Seconds) {
        Set-GuiMediaPlayer P -Window $Window.Id -Position $Seconds
        Wait-Media $Window 'Position' | Out-Null
        Start-Sleep -Milliseconds 500
        $frame = @{}
        Wait-Until { $frame.Shot = Get-GuiSnapshot -Window $Window.Id -Name P -Pixels; (Get-Lit $frame.Shot) -gt ($frame.Shot.Width * $frame.Shot.Height / 2) } 'the test picture filling the player' | Out-Null
        $frame.Shot
    }

    # Whether player P of window `$Window` shows a cue over the test picture:
    # a snapshot that differs from `$Bare` with the picture filling the
    # player. A snapshot that came back with no picture differs from any
    # picture, so a change counts only with the picture there.
    function Test-CueShown($Window, $Bare) {
        $shot = Get-GuiSnapshot -Window $Window.Id -Name P -Pixels
        (Get-Lit $shot) -gt ($shot.Width * $shot.Height / 2) -and (Get-Changed $Bare $shot) -gt 150
    }

    # Watches player P of window `$Window` start its clip over twice, a
    # Position below the last being a start over, and gives how many times it
    # raised Playing. Two starts over are the fewest that show a repeat late
    # in a pass. A looping player's clip never ends for whoever follows it.
    function Get-LoopPlaying($Window) {
        $seen = @{ Playing = 0; Over = 0; Position = 0.0 }
        Wait-Until {
            $e = Wait-GuiEvent -Window $Window.Id -Timeout 1
            if ($e.Kind -eq 'Ended') { throw 'a looping player raised Ended' }
            if ($e.Kind -eq 'Playing') { $seen.Playing++ }
            if ($e.Kind -eq 'Position') {
                if ($e.Value -lt $seen.Position) { $seen.Over++ }
                $seen.Position = $e.Value
            }
            $seen.Over -ge 2
        } 'the clip starting over twice' | Out-Null
        $seen.Playing
    }
}

AfterAll {
    Stop-Watchdog $script:Watchdog
}

Describe 'A MediaPlayer in <Renderer> windows in <Mode> mode' -ForEach $cases {
    BeforeAll {
        Reset-GuiSession $Mode $Renderer
        $script:Renderer = $Renderer
    }

    It 'plays <Clip> to its end with its picture and its sound' -ForEach $Plays {
        $path = Join-Path $script:Clips $Clip
        $sound = $Clip -like 'tone.*'
        $w = Show-GuiWindow -NoWait -Title 'PWRS-GUI media test' -Width 320 -Height 240 { MediaPlayer -Path $path -Name P -AutoPlay -AudioOnly:$sound -OnPosition {} -PositionInterval 0.25 }
        try {
            $loaded = Wait-Media $w 'Loaded', 'Error'
            $loaded.Kind | Should -Be 'Loaded' -Because ($loaded.Value | ConvertTo-Json -Compress)
            $loaded.Value.Duration | Should -BeGreaterThan 1.5
            $loaded.Value.HasAudio | Should -BeTrue
            $loaded.Value.HasVideo | Should -Be (-not $sound)
            if (-not $sound) {
                $loaded.Value.Width | Should -Be 160
                $loaded.Value.Height | Should -Be 120
            }
            Wait-Media $w 'Playing' | Out-Null
            # The phase is read as the clip starts, since a clip of two
            # seconds may end during the waits below.
            (Get-GuiMediaPlayer P -Window $w.Id).Phase | Should -Be 'Playing'
            if ($Measured) {
                # A renderer's measure may hear a clip's sound a while after
                # the clip plays, so the case waits for the tone, which plays
                # at an eighth of full scale, and for the seconds it counts.
                # The peak looks back half a second and a clip's last picture
                # stays, so the tone comes first.
                Wait-Until { $heard = (Get-GuiSnapshot -Window $w.Id).Sound.P; $heard.Peak -gt 0.05 -and $heard.Seconds -gt 0.2 } 'the tone heard above 0.05 for more than 0.2 s' | Out-Null
            } else {
                $heard = (Get-GuiSnapshot -Window $w.Id).Sound
                $heard.Contains('P') | Should -BeTrue -Because 'a snapshot gives each media player''s sound, measured or not'
                $heard.P | Should -BeNullOrEmpty -Because $Unmeasured
            }
            if (-not $sound) {
                # A renderer may draw a clip's first picture a while after the
                # clip plays, so the case waits for the test picture to fill
                # the player.
                Wait-Until { $shot = Get-GuiSnapshot -Window $w.Id -Name P -Pixels; (Get-Lit $shot) -gt ($shot.Width * $shot.Height / 2) } 'the test picture filling the player' | Out-Null
            }
            $ended = Wait-Media $w 'Ended' -Seconds 15
            $ended.Value | Should -BeGreaterThan 1.5
        } finally {
            Close-GuiWindow -Window $w.Id
        }
    }

    It 'raises the designed error for <Clip>, naming its format' -ForEach $Gaps {
        $path = Join-Path $script:Clips $Clip
        $w = Show-GuiWindow -NoWait -Title 'PWRS-GUI media test' -Width 320 -Height 240 { MediaPlayer -Path $path -Name P -AutoPlay }
        try {
            $e = Wait-Media $w 'Error', 'Ended'
            $e.Kind | Should -Be 'Error'
            $e.Value.Cause | Should -Be 'FormatNotSupported'
            $e.Value.Address | Should -Be $path
            $e.Value.Format | Should -Not -BeNullOrEmpty
            $e.Value.Message | Should -Be "$path could not be played: its format is not supported here; it holds $($e.Value.Format)"
            $remedy = if ($script:Renderer -eq 'Html') { '-Renderer Egui' } else { 'Install-Module PWRS-GUI.FFmpeg.*' }
            $e.Value.Remedy | Should -BeLike $remedy
            $e.Value.Detail | Should -Not -BeNullOrEmpty
            (Get-GuiMediaPlayer P -Window $w.Id).Phase | Should -Be 'Failed'
        } finally {
            Close-GuiWindow -Window $w.Id
        }
    }

    It 'raises Error for an https clip it cannot fetch, for the network''s cause' {
        $address = 'https://127.0.0.1:9/gone.mp4'
        $w = Show-GuiWindow -NoWait -Title 'PWRS-GUI media test' -Width 320 -Height 240 { MediaPlayer -Path $address -Name P -AutoPlay }
        try {
            $e = Wait-Media $w 'Error', 'Loaded'
            $e.Kind | Should -Be 'Error'
            $e.Value.Cause | Should -Be 'Network' -Because ($e.Value | ConvertTo-Json -Compress)
            $e.Value.Address | Should -Be $address
            $e.Value.Format | Should -BeNullOrEmpty
            $e.Value.Detail | Should -Not -BeNullOrEmpty
            (Get-GuiMediaPlayer P -Window $w.Id).Phase | Should -Be 'Failed'
        } finally {
            Close-GuiWindow -Window $w.Id
        }
        { Invoke-GuiMedia $address -Wait -Renderer $script:Renderer } | Should -Throw -ErrorId 'GuiMediaFailed*'
    }

    It 'is moved, slowed, quietened and read by Set-GuiMediaPlayer and Get-GuiMediaPlayer' {
        $w = Show-GuiWindow -NoWait -Title 'PWRS-GUI media test' -Width 320 -Height 240 { MediaPlayer -Path (Join-Path $script:Clips 'tone.wav') -Name P -AudioOnly -OnPosition {} }
        try {
            Wait-Media $w 'Loaded' | Out-Null
            $state = Set-GuiMediaPlayer P -Window $w.Id -Position 1 -Rate 1.5 -Volume 0.5 -Muted -Loop:$false -PassThru
            $state.GetType().FullName | Should -Be 'PwrsGui.MediaPlayer'
            $state.Position | Should -BeGreaterOrEqual 0.9
            $state.Rate | Should -Be 1.5
            $state.Volume | Should -Be 0.5
            $state.Muted | Should -BeTrue
            (Wait-Media $w 'Position').Value | Should -BeGreaterOrEqual 0.9 -Because 'a seek raises Position where it went'
            Set-GuiMediaPlayer P -Window $w.Id -Play
            Start-Sleep -Milliseconds 400
            if ($Measured) {
                (Get-GuiSnapshot -Window $w.Id).Sound.P.Peak | Should -Be 0 -Because 'a muted player sends the system silence'
            } else {
                (Get-GuiSnapshot -Window $w.Id).Sound.P | Should -BeNullOrEmpty -Because $Unmeasured
            }
            Wait-Media $w 'Ended' | Out-Null
            (Get-GuiMediaPlayer P -Window $w.Id).Phase | Should -Be 'Ended'
            { Set-GuiMediaPlayer P -Window $w.Id -Rate 5 } | Should -Throw -ErrorId 'GuiBadMedia*'
            { Set-GuiMediaPlayer P -Window $w.Id -Play -Pause } | Should -Throw -ErrorId 'GuiBadMedia*'
        } finally {
            Close-GuiWindow -Window $w.Id
        }
    }

    It 'raises Position every -PositionInterval while it plays' {
        $w = Show-GuiWindow -NoWait -Title 'PWRS-GUI media test' -Width 320 -Height 240 { MediaPlayer -Path (Join-Path $script:Clips 'tone.wav') -Name P -AudioOnly -AutoPlay -OnPosition {} -PositionInterval 0.5 }
        try {
            $positions = 0
            $deadline = [DateTime]::UtcNow.AddSeconds(15)
            while ([DateTime]::UtcNow -lt $deadline) {
                $e = Wait-GuiEvent -Window $w.Id -Timeout 1
                if ($e.Kind -eq 'Position') { $positions++ }
                if ($e.Kind -eq 'Ended') { break }
            }
            $positions | Should -BeGreaterOrEqual 3 -Because 'two seconds at a half-second interval, and the end'
            $positions | Should -BeLessOrEqual 7
        } finally {
            Close-GuiWindow -Window $w.Id
        }
    }

    # On WebKitGTK the page restarts every looping player itself. The next
    # two cases loop a clip alone and with cues.
    It 'plays its clip again at each end with -Loop' {
        $clip = Join-Path $script:Clips ($Plays.Clip | Where-Object { $_ -notlike 'tone.*' } | Select-Object -First 1)
        $w = Show-GuiWindow -NoWait -Title 'PWRS-GUI media test' -Width 320 -Height 240 { MediaPlayer -Path $clip -Name P -NoControls -AutoPlay -Loop -Muted -OnPosition {} -PositionInterval 0.25 }
        try {
            Get-LoopPlaying $w | Should -Be 1
            $state = Get-GuiMediaPlayer P -Window $w.Id
            $state.Phase | Should -Be 'Playing'
            $state.Loop | Should -BeTrue
        } finally {
            Close-GuiWindow -Window $w.Id
        }
    }

    It 'plays its clip again at each end with -Loop and -Subtitles' {
        $clip = Join-Path $script:Clips ($Plays.Clip | Where-Object { $_ -notlike 'tone.*' } | Select-Object -First 1)
        $cues = Join-Path $script:Clips 'cues.srt'
        $w = Show-GuiWindow -NoWait -Title 'PWRS-GUI media test' -Width 320 -Height 240 { MediaPlayer -Path $clip -Name P -NoControls -AutoPlay -Loop -Muted -Subtitles $cues -OnPosition {} -PositionInterval 0.25 }
        try {
            Get-LoopPlaying $w | Should -Be 1
            (Get-GuiMediaPlayer P -Window $w.Id).Loop | Should -BeTrue
        } finally {
            Close-GuiWindow -Window $w.Id
        }
    }

    It 'shows the cues of -Subtitles <Sidecar> over the picture at their times' -ForEach @(@{ Sidecar = 'cues.srt' }, @{ Sidecar = 'cues.vtt' }) {
        $clip = Join-Path $script:Clips ($Plays.Clip | Where-Object { $_ -notlike 'tone.*' } | Select-Object -First 1)
        $w = Show-GuiWindow -NoWait -Title 'PWRS-GUI media test' -Width 320 -Height 240 { MediaPlayer -Path $clip -Name P -NoControls -OnPosition {} }
        try {
            Wait-Media $w 'Loaded' | Out-Null
            $early = Get-Frame $w 0.2
            $bare = Get-Frame $w 1
            Update-GuiWidget -Window $w.Id -Name P -Property @{ Subtitles = (Join-Path $script:Clips $Sidecar) }
            Wait-Until { Test-CueShown $w $bare } 'the cue showing over the picture at 1 s'
            Get-Changed $early (Get-Frame $w 0.2) | Should -BeLessThan 50 -Because 'no cue shows before 0.5 s'
        } finally {
            Close-GuiWindow -Window $w.Id
        }
        { New-GuiMediaPlayer -Path $clip -Subtitles (Join-Path $script:Clips 'tone.wav') } | Should -Throw -ErrorId 'GuiBadProperty*'
    }

    # A renderer whose player reports no clip's own text track here has no
    # case, which Pester refuses as an empty list. A clip's own cue comes
    # from its engine some time after the seek, so the case waits for it.
    if (@($InBand).Count -gt 0) {
        It 'shows the cue of <Clip>''s own text track over the picture at 1 s' -ForEach $InBand {
            $w = Show-GuiWindow -NoWait -Title 'PWRS-GUI media test' -Width 320 -Height 240 { MediaPlayer -Path (Join-Path $script:Clips $Plain) -Name P -NoControls -OnPosition {} }
            try {
                Wait-Media $w 'Loaded' | Out-Null
                $bare = Get-Frame $w 1
            } finally {
                Close-GuiWindow -Window $w.Id
            }
            $w = Show-GuiWindow -NoWait -Title 'PWRS-GUI media test' -Width 320 -Height 240 { MediaPlayer -Path (Join-Path $script:Clips $Clip) -Name P -NoControls -OnPosition {} }
            try {
                Wait-Media $w 'Loaded' | Out-Null
                Get-Frame $w 1 | Out-Null
                Wait-Until { Test-CueShown $w $bare } "the cue of $Clip's own track over the picture $Plain shares with it"
            } finally {
                Close-GuiWindow -Window $w.Id
            }
        }

        # On WebKitGTK an HTML window plays a muted player at volume 0 with its
        # element unmuted, and the player reads as muted at its own volume.
        It 'plays <Clip> with -Muted from its start, muted' -ForEach $InBand {
            $path = Join-Path $script:Clips $Clip
            $w = Show-GuiWindow -NoWait -Title 'PWRS-GUI media test' -Width 320 -Height 240 { MediaPlayer -Path $path -Name P -NoControls -AutoPlay -Muted -OnPosition {} }
            try {
                Wait-Media $w 'Playing' | Out-Null
                $state = Get-GuiMediaPlayer P -Window $w.Id
                $state.Phase | Should -Be 'Playing'
                $state.Muted | Should -BeTrue
                $state.Volume | Should -Be 1
            } finally {
                Close-GuiWindow -Window $w.Id
            }
        }

        # On WebKitGTK an HTML window plays such a clip briefly and silently
        # before its first paused seek, and the player says nothing of it.
        It 'seeks <Clip> paused raising Position alone, its volume and mute as set' -ForEach $InBand {
            $path = Join-Path $script:Clips $Clip
            $w = Show-GuiWindow -NoWait -Title 'PWRS-GUI media test' -Width 320 -Height 240 { MediaPlayer -Path $path -Name P -NoControls -Volume 0.5 -OnPosition {} }
            try {
                Wait-Media $w 'Loaded' | Out-Null
                $state = Set-GuiMediaPlayer P -Window $w.Id -Position 1 -PassThru
                $state.Position | Should -BeGreaterOrEqual 0.9 -Because 'the reply gives the place asked for'
                $state.Phase | Should -Be 'Paused'
                $kinds = [System.Collections.Generic.List[string]]::new()
                Wait-Until { $e = Wait-GuiEvent -Window $w.Id -Timeout 1; if ($e) { $kinds.Add([string]$e.Kind) }; $kinds -contains 'Position' } 'Position after the seek' | Out-Null
                $kinds | Should -Not -Contain 'Playing'
                $kinds | Should -Not -Contain 'Paused'
                $kinds | Should -Not -Contain 'Ended'
                $after = Get-GuiMediaPlayer P -Window $w.Id
                $after.Phase | Should -Be 'Paused'
                $after.Volume | Should -Be 0.5
                $after.Muted | Should -BeFalse
            } finally {
                Close-GuiWindow -Window $w.Id
            }
        }

        It 'plays <Clip> again at each end with -Loop, its own text track followed' -ForEach $InBand {
            $path = Join-Path $script:Clips $Clip
            $w = Show-GuiWindow -NoWait -Title 'PWRS-GUI media test' -Width 320 -Height 240 { MediaPlayer -Path $path -Name P -NoControls -AutoPlay -Loop -Muted -OnPosition {} -PositionInterval 0.25 }
            try {
                Get-LoopPlaying $w | Should -Be 1
                (Get-GuiMediaPlayer P -Window $w.Id).Loop | Should -BeTrue
            } finally {
                Close-GuiWindow -Window $w.Id
            }
        }
    }

    It 'is played by Invoke-GuiMedia in a window of its own' {
        $clip = if ($Plays.Clip -contains 'h264-aac.mp4') { 'h264-aac.mp4' } else { 'vp9-opus.webm' }
        $w = Invoke-GuiMedia (Join-Path $script:Clips $clip) -Renderer $script:Renderer
        try {
            $w.Title | Should -Be $clip
            (Get-GuiMediaPlayer Player -Window $w.Id).Phase | Should -BeIn 'Paused', 'Playing'
        } finally {
            Close-GuiWindow -Window $w.Id
        }
        $took = Measure-Command { Invoke-GuiMedia (Join-Path $script:Clips $clip) -Wait -Renderer $script:Renderer }
        $took.TotalSeconds | Should -BeGreaterThan 1.5
        $took.TotalSeconds | Should -BeLessThan 15
        { Invoke-GuiMedia (Join-Path $script:Clips $Gaps[0].Clip) -Wait -Renderer $script:Renderer } | Should -Throw -ErrorId 'GuiMediaUnplayable*'
    }

    It 'plays a clip''s sound with Invoke-GuiMedia -AudioOnly and no window shown' {
        $w = Invoke-GuiMedia (Join-Path $script:Clips 'tone.wav') -AudioOnly -Renderer $script:Renderer
        try {
            $w.Title | Should -Be 'tone.wav'
            Wait-Until { (Get-GuiMediaPlayer Player -Window $w.Id).Phase -eq 'Playing' } 'the sound playing' | Out-Null
            (Get-GuiMediaPlayer Player -Window $w.Id).HasAudio | Should -BeTrue
        } finally {
            Close-GuiWindow -Window $w.Id
        }
        $took = Measure-Command { Invoke-GuiMedia (Join-Path $script:Clips 'tone.wav') -AudioOnly -Wait -Renderer $script:Renderer }
        $took.TotalSeconds | Should -BeGreaterThan 1.5
        $took.TotalSeconds | Should -BeLessThan 15
    }

    It 'refuses Get-GuiMediaPlayer on a widget of another kind' {
        $w = Show-GuiWindow -NoWait -Title 'PWRS-GUI media test' { Text 'Hi' -Name T }
        try {
            { Get-GuiMediaPlayer T -Window $w.Id } | Should -Throw -ErrorId 'GuiNotAMediaPlayer*'
        } finally {
            Close-GuiWindow -Window $w.Id
        }
    }
}
