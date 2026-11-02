---
title: Invoke-GuiMedia
weight: 9
---

Plays a video or a sound clip in a window of its own. Source: `crates/pwrs-gui/src/media.rs`, `InvokeGuiMedia`, `picture` and `unplayable`; the window's player is the one [New-GuiMediaPlayer](New-GuiMediaPlayer.md) describes.

The examples ran in order, in PowerShell 7.6.9 and Windows PowerShell 5.1, after those of [New-GuiMediaPlayer](New-GuiMediaPlayer.md) in the same process. The output shown is PowerShell 7.6's; Windows PowerShell 5.1 printed the same. Each example closes its window at its end.

## Syntax

```powershell
Get-Command Invoke-GuiMedia -Syntax
```

```text
Invoke-GuiMedia [-Path] <string> [-AudioOnly] [-Wait] [-Loop] [-Muted] [-Volume <double>] [-Rate <double>] [-Subtitles <string>] [-EngineFetch] [-Title <string>] [-Renderer <Renderer>] [<CommonParameters>]
```

## Description

`Invoke-GuiMedia` opens a window titled with the clip's file name, a [MediaPlayer](New-GuiMediaPlayer.md) named `Player` filling it and playing at once. The window opens at the picture's size in pixels as points, which the clip's own headers give, an https clip's read through the module's loader first; where the headers give none it opens at 640 by 360 points. With `-AudioOnly` the clip's sound plays with no window shown, in the egui renderer whatever `-Renderer` says; the window written still takes [Get-GuiMediaPlayer](Get-GuiMediaPlayer.md), [Set-GuiMediaPlayer](Set-GuiMediaPlayer.md) and [Close-GuiWindow](Close-GuiWindow.md).

Without `-Wait` the command writes the window once the clip is open, and the window stays open on the clip's last frame when it ends, until it is closed. With `-Wait` it returns when the clip ends, closing its window, or when the window is closed or Ctrl+C is pressed. A clip that cannot be played ends it with an error whose message names the clip, why it cannot be played, the format its headers name and the remedy: `GuiMediaUnplayable` for a format the player cannot play, `GuiMediaFailed` for any other cause.

## Parameters

### -Path

| | |
|---|---|
| Type | `System.String` |
| Parameter sets | all: mandatory, position 0 |
| Pipeline input | no |
| Refused with | `GuiNoFile` for a path that names no file |

The clip's file, resolved from the current location as PowerShell resolves a path, with wildcards taken literally; or the clip's https address, which the module's loader fetches by byte ranges.

### -AudioOnly

| | |
|---|---|
| Type | `System.Management.Automation.SwitchParameter` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | off |

Plays the clip's sound alone, with no window shown.

### -Wait

| | |
|---|---|
| Type | `System.Management.Automation.SwitchParameter` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | off |

Returns when the clip ends, its window closing, rather than once the window has opened.

### -Loop

| | |
|---|---|
| Type | `System.Management.Automation.SwitchParameter` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | off |

Plays the clip again from the start each time it ends; with `-Wait`, until the window is closed or Ctrl+C is pressed.

### -Muted

| | |
|---|---|
| Type | `System.Management.Automation.SwitchParameter` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | off |

Silences the clip's sound.

### -Volume

| | |
|---|---|
| Type | `System.Double` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | 1 |
| Refused with | `GuiBadNumber` outside 0 to 1 |

The volume, from 0 for silence to 1 for the clip's own level.

### -Rate

| | |
|---|---|
| Type | `System.Double` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | 1 |
| Refused with | `GuiBadNumber` outside 0.25 to 4 |

How many times the clip's own speed it plays at, from 0.25 to 4, the sound's pitch kept.

### -Subtitles

| | |
|---|---|
| Type | `System.String` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | none |
| Refused with | `GuiBadProperty` for a file that cannot be read as SubRip or WebVTT cues |

A SubRip (`.srt`) or WebVTT (`.vtt`) file of subtitles, resolved from the current location, whose cues show over the picture at their times.

### -EngineFetch

| | |
|---|---|
| Type | `System.Management.Automation.SwitchParameter` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | off: the module's loader fetches an https clip |

Has the player's own engine fetch an https clip on the system's network stack, in place of the module's loader; the clip's server must allow it under CORS, in either renderer, as [New-GuiMediaPlayer](New-GuiMediaPlayer.md) describes.

### -Title

| | |
|---|---|
| Type | `System.String` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | the clip's file name |

The window's title.

### -Renderer

| | |
|---|---|
| Type | `PwrsGui.Renderer` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | the renderer [Set-GuiOption](Set-GuiOption.md) `-Renderer` chose |

Draws the window with this renderer: `Egui` or `Html`. With `-AudioOnly` the egui renderer plays the sound whatever this says.

## Inputs

None.

## Outputs

Without `-Wait`, a `PwrsGui.Window`, the window opened, as [Get-GuiWindow](Get-GuiWindow.md) writes it. With `-Wait`, nothing.

## Examples

The tone the [New-GuiMediaPlayer](New-GuiMediaPlayer.md) page writes to `$wav`, played to its end with no window:

```powershell
$took = Measure-Command { Invoke-GuiMedia $wav -AudioOnly -Wait }
'{0:N0} s' -f $took.TotalSeconds
```

```text
1 s
```

The tone playing over and over until its window is closed:

```powershell
$w = Invoke-GuiMedia $wav -AudioOnly -Loop
$w.Title
(Get-GuiMediaPlayer Player -Window $w.Id).Phase
Close-GuiWindow -Window $w.Id
```

```text
pwrs-gui-tone.wav
Playing
```

A file that is not there, and a file that holds no clip:

```powershell
try { Invoke-GuiMedia (Join-Path ([IO.Path]::GetTempPath()) 'no-such-clip.mp4') } catch { $_.FullyQualifiedErrorId }
$text = Join-Path ([IO.Path]::GetTempPath()) 'not-a-clip.mp4'
Set-Content -LiteralPath $text -Value 'Text, not a clip.'
try { Invoke-GuiMedia $text -AudioOnly -Wait } catch { $_.FullyQualifiedErrorId }
Remove-Item -LiteralPath $text
```

```text
GuiNoFile,Pwrs.Modules.PWRS_GUI.InvokeGuiMediaCommand
GuiMediaUnplayable,Pwrs.Modules.PWRS_GUI.InvokeGuiMediaCommand
```

## Errors

| Error ID | When |
|---|---|
| `GuiNoFile` | `-Path` names no file |
| `GuiBadNumber` | `-Volume` is outside 0 to 1, or `-Rate` outside 0.25 to 4 |
| `GuiBadProperty` | `-Subtitles` names a file that cannot be read as SubRip or WebVTT cues; the message says why |
| `GuiMediaUnplayable` | the player cannot play the clip's format, or the system has no player; the message names the clip, the format its headers name and the remedy |
| `GuiMediaFailed` | the clip cannot be played for another cause: it cannot be read or fetched, or its data cannot be decoded; the message names the clip and why |

Each ends the command. The [Error Reference](Error-Reference.md) describes each, and the session's errors, such as `GuiRendererGone`.
