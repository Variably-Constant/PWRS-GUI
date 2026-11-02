---
title: New-GuiMediaPlayer
weight: 33
---

Describes a player of a video or a sound clip. Source: `crates/pwrs-gui/src/widgets.rs`, `NewGuiMediaPlayer`.

The examples ran in order, in PowerShell 7.6.9 and Windows PowerShell 5.1, the examples of this page and of [Invoke-GuiMedia](Invoke-GuiMedia.md), [Set-GuiMediaPlayer](Set-GuiMediaPlayer.md) and [Get-GuiMediaPlayer](Get-GuiMediaPlayer.md) one after another in one process. The output shown is PowerShell 7.6's; Windows PowerShell 5.1 printed the same. Each example closes its window at its end.

## Syntax

```powershell
Get-Command New-GuiMediaPlayer -Syntax
```

```text
New-GuiMediaPlayer [-Path] <string> [-AudioOnly] [-AutoPlay] [-Loop] [-Muted] [-NoControls] [-Subtitles <string>] [-EngineFetch] [-Volume <double>] [-Rate <double>] [-PositionInterval <Object>] [-Width <double>] [-Height <double>] [-OnLoaded <scriptblock>] [-OnPlaying <scriptblock>] [-OnPaused <scriptblock>] [-OnEnded <scriptblock>] [-OnPosition <scriptblock>] [-OnError <scriptblock>] [-OnDrop <scriptblock>] [-Name <string>] [<CommonParameters>]
```

## Description

`New-GuiMediaPlayer` writes a `PwrsGui.Widget` that [Show-GuiWindow](Show-GuiWindow.md) draws as a player of the clip `-Path` names: a file, or an https address. The system's own player plays it: Media Foundation in egui windows on Windows, AVFoundation on macOS and GStreamer on Linux and FreeBSD; HTML windows play it in the webview's own `video` or `audio` element. In a `-Content` block `MediaPlayer` stands for `New-GuiMediaPlayer`.

The picture shows at the clip's size in pixels as points, shrunk to fit the room its place gives it, its proportions kept and the rest of the player black, or in a box of `-Width` and `-Height`. With `-AudioOnly` the clip's sound plays with no picture, under a bar of its controls. The controls, drawn alike by both renderers, play and pause, show the position and the length, seek, mute, set the volume and go to full screen; `-NoControls` takes them away. A double click also goes to full screen, where the player shows alone and its controls hide while the pointer rests; Escape comes back. A player with the keyboard focus takes a browser's player's keys: Space and Enter play or pause, Left and Right move 5 seconds, Up and Down change the volume by a tenth, M mutes and F goes to full screen.

`-Subtitles` shows the cues of a SubRip or WebVTT file over the picture at their times, beside those of the clip's own track of subtitles where the player reports one: Media Foundation's timed text, AVFoundation's legible output, and GStreamer's and WebKit's text tracks report them, and WebView2 reports none.

In egui windows each frame reaches the window's GPU without a copy through the CPU on Windows, where the renderer draws with Direct3D 12 or Vulkan on a hardware adapter, and on macOS, where it draws with Metal. With OpenGL or WARP, and on Linux and FreeBSD, it copies each frame through the CPU. In HTML windows the engine draws its own video element.

The module's loader fetches an https clip, by byte ranges, in both renderers: Mozilla's and the system's roots verify the server, the environment's proxy is gone through, and no Windows logon is offered. `-EngineFetch` has the player's own engine fetch it instead, on the system's network stack, proxy settings and cache; the clip's server must then let pages of the module's origin read it under CORS, in either renderer, and a server that does not fails the clip, saying so.

[Update-GuiWidget](Update-GuiWidget.md) changes `Loop`, `Muted`, `Volume`, `Rate`, `PositionInterval`, `NoControls`, `Subtitles` and `EngineFetch` while the window is open, and `Path` to play another clip. [Set-GuiMediaPlayer](Set-GuiMediaPlayer.md) plays, pauses and seeks it, and [Get-GuiMediaPlayer](Get-GuiMediaPlayer.md) reads where it is. [Get-GuiSnapshot](Get-GuiSnapshot.md) gives the sound it handed the system.

A clip the player cannot play raises `Error`. Its value is worded alike in both renderers: `Message`, which names the clip and why; `Address`, the clip's path or address; `Cause`, one of `Aborted`, `Network`, `Decode` and `FormatNotSupported`; `Format`, what the clip's headers say it holds, where they say; `Remedy`, the FFmpeg add-on or `-Renderer Egui` for a format the player cannot play, or what to install where the system has no player; and `Detail`, the player's own words.

## Parameters

### -Path

| | |
|---|---|
| Type | `System.String` |
| Parameter sets | all: position 0, mandatory |
| Pipeline input | no |
| Default | none |

The clip's file, resolved from the current location as PowerShell resolves a path, with wildcards taken literally; or the clip's https address.

### -AudioOnly

| | |
|---|---|
| Type | `System.Management.Automation.SwitchParameter` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | off |

Plays the clip's sound alone, with no picture.

### -AutoPlay

| | |
|---|---|
| Type | `System.Management.Automation.SwitchParameter` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | off |

Starts playing as soon as the clip is open.

### -Loop

| | |
|---|---|
| Type | `System.Management.Automation.SwitchParameter` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | off |

Plays the clip again from the start each time it ends.

### -Muted

| | |
|---|---|
| Type | `System.Management.Automation.SwitchParameter` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | off |

Silences the clip's sound.

### -NoControls

| | |
|---|---|
| Type | `System.Management.Automation.SwitchParameter` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | off |

Draws no controls, which a script then works itself.

### -Subtitles

| | |
|---|---|
| Type | `System.String` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | none |

A SubRip (`.srt`) or WebVTT (`.vtt`) file of subtitles, resolved from the current location, whose cues show over the picture at their times.

### -EngineFetch

| | |
|---|---|
| Type | `System.Management.Automation.SwitchParameter` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | off: the module's loader fetches an https clip |

Has the player's own engine fetch an https clip on the system's network stack, proxy settings and cache. The clip's server must allow it under CORS, in either renderer.

### -Volume

| | |
|---|---|
| Type | `System.Double` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | 1 |

The volume, from 0 for silence to 1 for the clip's own level.

### -Rate

| | |
|---|---|
| Type | `System.Double` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | 1 |

How many times the clip's own speed it plays at, from 0.25 to 4, the sound's pitch kept.

### -PositionInterval

| | |
|---|---|
| Type | `System.Object` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | 0.25 seconds |

The seconds, or a `[TimeSpan]`, between `Position` events while the clip plays, above 0.

### -Width

| | |
|---|---|
| Type | `System.Double` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | none: the room its place leaves it |

Its width in points, above 0.

### -Height

| | |
|---|---|
| Type | `System.Double` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | none: the height the window leaves it |

Its height in points, above 0.

### -OnLoaded

| | |
|---|---|
| Type | `System.Management.Automation.ScriptBlock` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | none |

Runs once the clip is open, with the event as its argument; its `Value` gives the clip's `Duration` in seconds, its `Width` and `Height` in pixels, and `HasVideo` and `HasAudio`.

### -OnPlaying

| | |
|---|---|
| Type | `System.Management.Automation.ScriptBlock` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | none |

Runs when the clip starts or goes on playing, with the event as its argument; its `Value` is the position in seconds.

### -OnPaused

| | |
|---|---|
| Type | `System.Management.Automation.ScriptBlock` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | none |

Runs when the clip pauses, with the event as its argument; its `Value` is the position in seconds.

### -OnEnded

| | |
|---|---|
| Type | `System.Management.Automation.ScriptBlock` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | none |

Runs when the clip plays to its end, with the event as its argument; its `Value` is the position in seconds.

### -OnPosition

| | |
|---|---|
| Type | `System.Management.Automation.ScriptBlock` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | none |

Runs every `-PositionInterval` while the clip plays, after each seek and at the clip's end, with the event as its argument; its `Value` is the position in seconds. Without it no `Position` event is raised.

### -OnError

| | |
|---|---|
| Type | `System.Management.Automation.ScriptBlock` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | none |

Runs when the clip cannot be played, with the event as its argument; its `Value` is described above.

### -OnDrop

| | |
|---|---|
| Type | `System.Management.Automation.ScriptBlock` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | none |

Runs when files or text are dropped on the player, with the event as its argument; its `Value` is the paths dropped, or the text.

### -Name

| | |
|---|---|
| Type | `System.String` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | none |

The name it goes by in the handlers' events and the update commands.

## Inputs

None.

## Outputs

`PwrsGui.Widget`, its `Kind` `MediaPlayer` and its `Properties` with `Path`, the clip's full path or address, and the others that are given.

## Examples

The examples play a second of a 1 kHz tone, which the first writes as a WAV file, a format every player here plays:

```powershell
$rate = 16000
$wav = Join-Path ([IO.Path]::GetTempPath()) 'pwrs-gui-tone.wav'
$out = [IO.BinaryWriter]::new([IO.File]::Create($wav))
$out.Write([Text.Encoding]::ASCII.GetBytes('RIFF')); $out.Write([int](36 + 2 * $rate)); $out.Write([Text.Encoding]::ASCII.GetBytes('WAVEfmt '))
$out.Write([int]16); $out.Write([int16]1); $out.Write([int16]1); $out.Write([int]$rate); $out.Write([int](2 * $rate)); $out.Write([int16]2); $out.Write([int16]16)
$out.Write([Text.Encoding]::ASCII.GetBytes('data')); $out.Write([int](2 * $rate))
for ($i = 0; $i -lt $rate; $i++) { $out.Write([int16](8192 * [Math]::Sin(2 * [Math]::PI * 1000 * $i / $rate))) }
$out.Close()
(Get-Item $wav).Length
```

```text
32044
```

The tone played to its end:

```powershell
$w = Show-GuiWindow -NoWait -Title 'Tone' { MediaPlayer $wav -AudioOnly -AutoPlay -OnEnded {} -Name Tone }
do { $e = Wait-GuiEvent -Window $w.Id -Timeout 10 } until ($null -eq $e -or $e.Kind -eq 'Ended')
'{0} at {1:N1} s' -f $e.Kind, $e.Value
Close-GuiWindow -Window $w.Id
```

```text
Ended at 1.0 s
```

What the player says of the clip once it is open:

```powershell
$w = Show-GuiWindow -NoWait -Title 'Tone' { MediaPlayer $wav -AudioOnly -OnLoaded {} -Name Tone }
$loaded = Wait-GuiEvent -Window $w.Id -Timeout 10
'{0}: {1:N1} s, sound {2}, picture {3}' -f $loaded.Kind, $loaded.Value.Duration, $loaded.Value.HasAudio, $loaded.Value.HasVideo
Close-GuiWindow -Window $w.Id
```

```text
Loaded: 1.0 s, sound True, picture False
```

A file that is not there, and a rate out of range:

```powershell
try { New-GuiMediaPlayer -Path (Join-Path ([IO.Path]::GetTempPath()) 'no-such-clip.mp4') } catch { $_.FullyQualifiedErrorId }
try { New-GuiMediaPlayer -Path $wav -Rate 8 } catch { $_.FullyQualifiedErrorId; $_.Exception.Message }
```

```text
GuiNoFile,Pwrs.Modules.PWRS_GUI.NewGuiMediaPlayerCommand
GuiBadNumber,Pwrs.Modules.PWRS_GUI.NewGuiMediaPlayerCommand
a MediaPlayer's rate is 8; a clip plays at 0.25 to 4 times its own speed
```

## Errors

| Error ID | When |
|---|---|
| `GuiNoFile` | `-Path` names no file |
| `GuiBadNumber` | `-Volume` is outside 0 to 1, or `-Rate` outside 0.25 to 4 |
| `GuiBadProperty` | `-PositionInterval` is not above 0, or `-Subtitles` names a file that cannot be read as SubRip or WebVTT cues; the message says why |
| `GuiBadSize` | `-Width` or `-Height` is not above 0 |

Each ends the command. A clip that cannot be played raises the player's `Error` event, not an error of the command, and [Invoke-GuiMedia](Invoke-GuiMedia.md) ends with it. Each is described in the [Error Reference](Error-Reference.md).
